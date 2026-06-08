//! Bot core — connects, logs in, tracks game state (position, health, time,
//! world chunks, entities), and exposes core actions. An idiomatic Rust
//! distillation of typecraft's event-driven `bot` module: instead of an
//! EventEmitter, the caller drives [`Bot::next_event`], which processes packets
//! (responding to keep-alive/teleport automatically) and surfaces high-level
//! [`BotEvent`]s.

mod conversions;

pub use conversions::*;

use std::collections::HashMap;

use crate::chunk::{ChunkColumn, ChunkColumnOptions};
use crate::entity::Entity;
use crate::protocol::{Client, ClientOptions, PValue};
use crate::registry::Registry;
use crate::varint::push_var_int;
use crate::vec3::{vec3, Vec3, ZERO};

/// Build a `position` bitfield value (x:26, z:26, y:12 signed).
fn block_pos(x: i32, y: i32, z: i32) -> PValue {
    PValue::compound(vec![
        ("x", PValue::num(x as f64)),
        ("y", PValue::num(y as f64)),
        ("z", PValue::num(z as f64)),
    ])
}

#[derive(Debug, Clone, Default)]
pub struct GameInfo {
    pub game_mode: String,
    pub dimension: String,
    pub difficulty: String,
    pub hardcore: bool,
    pub max_players: i32,
    pub server_brand: String,
    pub min_y: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Default)]
pub struct TimeInfo {
    pub age: i64,
    pub time_of_day: i64,
    pub day: i64,
    pub is_day: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Player {
    pub uuid: String,
    pub username: String,
    pub gamemode: i32,
    pub ping: i32,
    pub entity_id: Option<i32>,
}

/// Loaded chunk columns, keyed by chunk coordinate.
#[derive(Default)]
pub struct World {
    pub columns: HashMap<(i32, i32), ChunkColumn>,
}

impl World {
    pub fn block_state_at(&self, x: i32, y: i32, z: i32) -> u32 {
        let key = (x >> 4, z >> 4);
        match self.columns.get(&key) {
            Some(col) => col.get_block_state_id(x & 15, y, z & 15),
            None => 0,
        }
    }

    pub fn set_block_state_at(&mut self, x: i32, y: i32, z: i32, state: u32) {
        let key = (x >> 4, z >> 4);
        if let Some(col) = self.columns.get_mut(&key) {
            col.set_block_state_id(x & 15, y, z & 15, state);
        }
    }
}

/// High-level events surfaced from the packet stream.
#[derive(Debug, Clone)]
pub enum BotEvent {
    Login,
    Spawn,
    Death,
    Health,
    Time,
    Position,
    ChunkLoad(i32, i32),
    EntitySpawn(i32),
    Chat(String),
    /// The server set the player inventory contents (window id).
    Inventory(i32),
    Kicked(String),
    /// A packet not otherwise handled (its name).
    Packet(String),
}

pub struct Bot {
    pub client: Client,
    pub registry: Option<Registry>,
    pub username: String,
    pub entity_id: i32,
    pub game: GameInfo,
    pub position: Vec3,
    pub yaw: f64,
    pub pitch: f64,
    pub on_ground: bool,
    pub health: f64,
    pub food: f64,
    pub food_saturation: f64,
    pub spawn_point: Vec3,
    pub is_raining: bool,
    pub time: TimeInfo,
    pub world: World,
    pub entities: HashMap<i32, Entity>,
    pub players: HashMap<String, Player>,
    pub brand: String,
    pub held_slot: i32,
    sequence: i32,
    spawned: bool,
    alive: bool,
    settings_sent: bool,
}

/// A block face direction for digging/placing.
#[derive(Debug, Clone, Copy)]
pub enum Face {
    Bottom = 0,
    Top = 1,
    North = 2,
    South = 3,
    West = 4,
    East = 5,
}

impl Bot {
    /// Connect, log in, and advance to the PLAY state.
    pub async fn connect(
        options: ClientOptions,
        registry: Option<Registry>,
    ) -> std::io::Result<Bot> {
        let mut client = Client::connect(&options.host, options.port, &options.username).await?;
        client.login(&options).await?;
        Ok(Bot {
            username: client.username.clone(),
            client,
            registry,
            entity_id: 0,
            game: GameInfo {
                game_mode: "survival".into(),
                dimension: "overworld".into(),
                difficulty: "normal".into(),
                min_y: -64,
                height: 384,
                ..Default::default()
            },
            position: ZERO,
            yaw: 0.0,
            pitch: 0.0,
            on_ground: true,
            health: 20.0,
            food: 20.0,
            food_saturation: 5.0,
            spawn_point: ZERO,
            is_raining: false,
            time: TimeInfo::default(),
            world: World::default(),
            entities: HashMap::new(),
            players: HashMap::new(),
            brand: "vanilla".into(),
            held_slot: 0,
            sequence: 0,
            spawned: false,
            alive: true,
            settings_sent: false,
        })
    }

    /// Pull and process the next packet, returning a high-level event (or `None`
    /// on disconnect). Keep-alive, teleport confirmation, and login handshakes
    /// are handled internally.
    pub async fn next_event(&mut self) -> std::io::Result<Option<BotEvent>> {
        loop {
            let Some((name, params)) = self.client.next_packet().await? else {
                return Ok(None);
            };
            match name.as_str() {
                "login" => {
                    self.handle_login(&params).await?;
                    return Ok(Some(BotEvent::Login));
                }
                "keep_alive" => {
                    let id = params
                        .get("keepAliveId")
                        .cloned()
                        .unwrap_or(PValue::Long(0));
                    self.client
                        .write("keep_alive", PValue::compound(vec![("keepAliveId", id)]))
                        .await?;
                }
                "ping" => {
                    if let Some(id) = params.get("id").cloned() {
                        self.client
                            .write("pong", PValue::compound(vec![("id", id)]))
                            .await?;
                    }
                }
                "player_position" => {
                    self.handle_position(&params).await?;
                    return Ok(Some(BotEvent::Position));
                }
                "set_health" => {
                    if let Some(e) = self.handle_health(&params) {
                        return Ok(Some(e));
                    }
                    return Ok(Some(BotEvent::Health));
                }
                "set_time" => {
                    self.handle_time(&params);
                    return Ok(Some(BotEvent::Time));
                }
                "set_default_spawn_position" => {
                    if let Some(loc) = params.get("location") {
                        self.spawn_point = vec3(
                            loc.get("x").and_then(PValue::as_f64).unwrap_or(0.0),
                            loc.get("y").and_then(PValue::as_f64).unwrap_or(0.0),
                            loc.get("z").and_then(PValue::as_f64).unwrap_or(0.0),
                        );
                    }
                }
                "level_chunk_with_light" => {
                    if let Some((cx, cz)) = self.handle_chunk(&params) {
                        return Ok(Some(BotEvent::ChunkLoad(cx, cz)));
                    }
                }
                "add_entity" => {
                    let id = params.get("entityId").and_then(PValue::as_i32).unwrap_or(0);
                    let mut entity = Entity::new(id);
                    entity.position = vec3(
                        params.get("x").and_then(PValue::as_f64).unwrap_or(0.0),
                        params.get("y").and_then(PValue::as_f64).unwrap_or(0.0),
                        params.get("z").and_then(PValue::as_f64).unwrap_or(0.0),
                    );
                    self.entities.insert(id, entity);
                    return Ok(Some(BotEvent::EntitySpawn(id)));
                }
                "container_set_content" => {
                    let id = params.get("windowId").and_then(PValue::as_i32).unwrap_or(0);
                    return Ok(Some(BotEvent::Inventory(id)));
                }
                "set_carried_item" => {
                    if let Some(s) = params.get("slot").and_then(PValue::as_i32) {
                        self.held_slot = s;
                    }
                }
                "remove_entities" => {
                    if let Some(ids) = params.get("entityIds").and_then(PValue::as_list) {
                        for id in ids {
                            if let Some(id) = id.as_i32() {
                                self.entities.remove(&id);
                            }
                        }
                    }
                }
                "disconnect" => {
                    let reason = params
                        .get("reason")
                        .and_then(PValue::as_str)
                        .unwrap_or("disconnected")
                        .to_string();
                    return Ok(Some(BotEvent::Kicked(reason)));
                }
                _ => return Ok(Some(BotEvent::Packet(name))),
            }
        }
    }

    async fn handle_login(&mut self, params: &PValue) -> std::io::Result<()> {
        self.entity_id = params.get("entityId").and_then(PValue::as_i32).unwrap_or(0);
        self.game.max_players = params
            .get("maxPlayers")
            .and_then(PValue::as_i32)
            .unwrap_or(0);
        self.game.hardcore = params
            .get("isHardcore")
            .and_then(PValue::as_bool)
            .unwrap_or(false);

        // Brand via plugin channel (data = length-prefixed string).
        let mut brand_data = Vec::new();
        push_var_int(&mut brand_data, self.brand.len() as i32);
        brand_data.extend_from_slice(self.brand.as_bytes());
        self.client
            .write(
                "custom_payload",
                PValue::compound(vec![
                    ("channel", PValue::str("minecraft:brand")),
                    ("data", PValue::Bytes(brand_data)),
                ]),
            )
            .await?;

        self.send_settings().await?;

        if self.client.protocol_version >= 769 {
            let _ = self
                .client
                .write("player_loaded", PValue::compound(vec![]))
                .await;
        }
        Ok(())
    }

    async fn send_settings(&mut self) -> std::io::Result<()> {
        self.settings_sent = true;
        // Skin parts: show everything (bitmask 0x7f).
        self.client
            .write(
                "client_information",
                PValue::compound(vec![
                    ("locale", PValue::str("en_US")),
                    ("viewDistance", PValue::num(12.0)),
                    ("chatFlags", PValue::num(0.0)),
                    ("chatColors", PValue::Bool(true)),
                    ("skinParts", PValue::num(127.0)),
                    ("mainHand", PValue::num(1.0)),
                    ("enableTextFiltering", PValue::Bool(false)),
                    ("enableServerListing", PValue::Bool(true)),
                    ("particleStatus", PValue::str("all")),
                ]),
            )
            .await
    }

    async fn handle_position(&mut self, params: &PValue) -> std::io::Result<()> {
        let g = |k: &str| params.get(k).and_then(PValue::as_f64).unwrap_or(0.0);
        self.position = vec3(g("x"), g("y"), g("z"));
        self.yaw = g("yaw");
        self.pitch = g("pitch");
        if let Some(e) = self.entities.get_mut(&self.entity_id) {
            e.position = self.position;
        }
        // Confirm the teleport.
        if let Some(id) = params.get("teleportId").cloned() {
            self.client
                .write(
                    "accept_teleportation",
                    PValue::compound(vec![("teleportId", id)]),
                )
                .await?;
        }
        Ok(())
    }

    fn handle_health(&mut self, params: &PValue) -> Option<BotEvent> {
        self.health = params
            .get("health")
            .and_then(PValue::as_f64)
            .unwrap_or(self.health);
        self.food = params
            .get("food")
            .and_then(PValue::as_f64)
            .unwrap_or(self.food);
        self.food_saturation = params
            .get("foodSaturation")
            .and_then(PValue::as_f64)
            .unwrap_or(self.food_saturation);

        if !self.spawned && self.health > 0.0 {
            self.spawned = true;
            self.alive = true;
            return Some(BotEvent::Spawn);
        }
        if self.health <= 0.0 && self.alive {
            self.alive = false;
            return Some(BotEvent::Death);
        }
        if self.health > 0.0 && !self.alive {
            self.alive = true;
            return Some(BotEvent::Spawn);
        }
        None
    }

    fn handle_time(&mut self, params: &PValue) {
        let age = params
            .get("gameTime")
            .and_then(PValue::as_i64)
            .unwrap_or(self.time.age);
        // 26.1.2 nests day-time in a clocks array; older sends timeOfDay directly.
        let tod = params
            .get("timeOfDay")
            .and_then(PValue::as_i64)
            .or_else(|| {
                params
                    .get("clocks")
                    .and_then(PValue::as_list)
                    .and_then(|c| c.first())
                    .and_then(|c| c.get("time"))
                    .and_then(PValue::as_i64)
            })
            .map(|t| t.abs())
            .unwrap_or(self.time.time_of_day);
        self.time = TimeInfo {
            age,
            time_of_day: tod % 24000,
            day: tod / 24000,
            is_day: tod % 24000 < 13000,
        };
    }

    fn handle_chunk(&mut self, params: &PValue) -> Option<(i32, i32)> {
        let cx = params.get("x").and_then(PValue::as_i32)?;
        let cz = params.get("z").and_then(PValue::as_i32)?;
        let data = params.get("chunkData").and_then(PValue::as_bytes)?;
        let mut col = ChunkColumn::new(ChunkColumnOptions {
            min_y: Some(self.game.min_y),
            world_height: Some(self.game.height),
            max_bits_per_block: crate::chunk::GLOBAL_BITS_PER_BLOCK,
            max_bits_per_biome: crate::chunk::GLOBAL_BITS_PER_BIOME,
        });
        // Protocol 770+ omits the per-container data-array length (it's computed
        // from bits-per-value and capacity).
        let no_array_length = self.client.protocol_version >= 770;
        col.load(data, no_array_length);
        self.world.columns.insert((cx, cz), col);
        Some((cx, cz))
    }

    // ── World queries ──

    /// Block state id at a world position.
    pub fn block_state_at(&self, x: i32, y: i32, z: i32) -> u32 {
        self.world.block_state_at(x, y, z)
    }

    /// Resolve the block at a world position to a name + properties (requires a
    /// loaded registry).
    pub fn block_at(&self, x: i32, y: i32, z: i32) -> Option<crate::block::BlockInfo> {
        let registry = self.registry.as_ref()?;
        let state = self.world.block_state_at(x, y, z);
        if registry.blocks_by_state_id.contains_key(&state) {
            Some(crate::block::state_id_to_block(registry, state))
        } else {
            None
        }
    }

    // ── Actions ──

    /// Send a chat command (without the leading slash).
    pub async fn run_command(&mut self, command: &str) -> std::io::Result<()> {
        self.client
            .write(
                "chat_command",
                PValue::compound(vec![("command", PValue::str(command))]),
            )
            .await
    }

    /// Send a position update.
    pub async fn send_position(
        &mut self,
        x: f64,
        y: f64,
        z: f64,
        on_ground: bool,
    ) -> std::io::Result<()> {
        self.position = vec3(x, y, z);
        self.on_ground = on_ground;
        self.client
            .write(
                "move_player_pos",
                PValue::compound(vec![
                    ("x", PValue::num(x)),
                    ("y", PValue::num(y)),
                    ("z", PValue::num(z)),
                    ("onGround", PValue::Bool(on_ground)),
                    ("flags", PValue::num(if on_ground { 1.0 } else { 0.0 })),
                ]),
            )
            .await
    }

    /// Swing the main arm.
    pub async fn swing_arm(&mut self) -> std::io::Result<()> {
        self.client
            .write("swing", PValue::compound(vec![("hand", PValue::num(0.0))]))
            .await
    }

    /// Select a hotbar slot (0-8) as the held item.
    pub async fn set_held_slot(&mut self, slot: i32) -> std::io::Result<()> {
        self.held_slot = slot;
        self.client
            .write(
                "set_carried_item",
                PValue::compound(vec![("slotId", PValue::num(slot as f64))]),
            )
            .await
    }

    /// Look toward a yaw/pitch (degrees).
    pub async fn look(&mut self, yaw: f64, pitch: f64) -> std::io::Result<()> {
        self.yaw = yaw;
        self.pitch = pitch;
        self.client
            .write(
                "move_player_rot",
                PValue::compound(vec![
                    ("yaw", PValue::num(yaw)),
                    ("pitch", PValue::num(pitch)),
                    (
                        "flags",
                        PValue::compound(vec![("onGround", PValue::Bool(self.on_ground))]),
                    ),
                ]),
            )
            .await
    }

    /// Dig (break) the block at (x, y, z). Sends start + finish digging and a
    /// swing — instant on creative; survival servers honour the sequence.
    pub async fn dig(&mut self, x: i32, y: i32, z: i32, face: Face) -> std::io::Result<()> {
        self.sequence += 1;
        let start = PValue::compound(vec![
            ("status", PValue::num(0.0)), // START_DESTROY_BLOCK
            ("location", block_pos(x, y, z)),
            ("face", PValue::num(face as i32 as f64)),
            ("sequence", PValue::num(self.sequence as f64)),
        ]);
        self.client.write("player_action", start).await?;
        self.swing_arm().await?;
        self.sequence += 1;
        let finish = PValue::compound(vec![
            ("status", PValue::num(2.0)), // STOP_DESTROY_BLOCK (finish)
            ("location", block_pos(x, y, z)),
            ("face", PValue::num(face as i32 as f64)),
            ("sequence", PValue::num(self.sequence as f64)),
        ]);
        self.client.write("player_action", finish).await
    }

    /// Place the held item against the given block face.
    pub async fn place_block(&mut self, x: i32, y: i32, z: i32, face: Face) -> std::io::Result<()> {
        self.sequence += 1;
        let packet = PValue::compound(vec![
            ("hand", PValue::num(0.0)),
            ("location", block_pos(x, y, z)),
            ("direction", PValue::num(face as i32 as f64)),
            ("cursorX", PValue::num(0.5)),
            ("cursorY", PValue::num(0.5)),
            ("cursorZ", PValue::num(0.5)),
            ("insideBlock", PValue::Bool(false)),
            ("worldBorderHit", PValue::Bool(false)),
            ("sequence", PValue::num(self.sequence as f64)),
        ]);
        self.client.write("use_item_on", packet).await
    }

    /// Attack an entity by id.
    pub async fn attack(&mut self, entity_id: i32) -> std::io::Result<()> {
        let packet = PValue::compound(vec![
            ("target", PValue::num(entity_id as f64)),
            ("mouse", PValue::num(1.0)), // ATTACK
            ("sneaking", PValue::Bool(false)),
        ]);
        self.client.write("interact", packet).await?;
        self.swing_arm().await
    }

    /// Respawn after death.
    pub async fn respawn(&mut self) -> std::io::Result<()> {
        self.client
            .write(
                "client_command",
                PValue::compound(vec![
                    ("actionId", PValue::num(0.0)),
                    ("payload", PValue::num(0.0)),
                ]),
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_block_access() {
        let mut world = World::default();
        let mut col = ChunkColumn::new(ChunkColumnOptions {
            min_y: Some(-64),
            world_height: Some(384),
            max_bits_per_block: 15,
            max_bits_per_biome: 7,
        });
        col.set_block_state_id(3, 10, 5, 42);
        world.columns.insert((0, 0), col);
        assert_eq!(world.block_state_at(3, 10, 5), 42);
        assert_eq!(world.block_state_at(100, 10, 5), 0); // unloaded chunk
    }
}
