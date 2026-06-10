//! Bot — connects, logs in, runs a 50 ms physics tick (movement sim + position
//! packets), tracks world/inventory/entities, and exposes high-level actions
//! (block queries, controls, look, dig, pathfinding `goto`). Faithful-in-spirit
//! port of typecraft's event-driven `bot`, adapted to a single-task Rust model:
//! every action drives [`Bot::drive_tick`], which races packet reads against the
//! 50 ms physics deadline, so keep-alive + physics keep running while waiting.

mod conversions;
mod crafting;
mod inventory;

pub use conversions::*;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::block::{state_id_to_block, BlockInfo};
use crate::chunk::{ChunkColumn, ChunkColumnOptions, GLOBAL_BITS_PER_BIOME, GLOBAL_BITS_PER_BLOCK};
use crate::entity::Entity;
use crate::item::{from_notch, Item};
use crate::path::{get_path_to, Goal, GoalNear, GoalNearXZ, Move, MovementsConfig, PathStatus};
use crate::physics::{
    apply_player_state, create_player_state, PhysicsEngine, PhysicsWorld, PlayerControls,
    WorldPhysics,
};
use crate::protocol::{Client, ClientOptions, PValue};
use crate::registry::Registry;
use crate::vec3::{vec3, Vec3};
use crate::window::Window;
use crate::world::{raycast, World, PLAYER_EYE_HEIGHT};

const TICK: Duration = Duration::from_millis(50);

/// A block face for digging/placing.
#[derive(Debug, Clone, Copy)]
pub enum Face {
    Bottom = 0,
    Top = 1,
    North = 2,
    South = 3,
    West = 4,
    East = 5,
}

#[derive(Debug, Clone, Default)]
pub struct GameInfo {
    pub game_mode: String,
    pub dimension: String,
    pub difficulty: String,
    pub hardcore: bool,
    pub max_players: i32,
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

#[derive(Debug, Clone, Copy, Default)]
pub struct ControlState {
    pub forward: bool,
    pub back: bool,
    pub left: bool,
    pub right: bool,
    pub jump: bool,
    pub sprint: bool,
    pub sneak: bool,
}

/// Result of following one computed path segment.
enum FollowOutcome {
    /// Reached the end of the path.
    Reached,
    /// Stuck or blocked — the caller should recompute a path.
    NeedRepath,
    Disconnected,
}

/// Outcome of one [`Bot::drive_tick`].
#[derive(Debug, Clone)]
pub enum DriveStep {
    /// The connection closed.
    Disconnected,
    /// A 50 ms physics tick ran.
    Tick,
    /// A packet was handled internally with no surfaced event.
    Handled,
    /// A high-level event occurred.
    Event(BotEvent),
}

/// High-level events surfaced from [`Bot::next_event`].
#[derive(Debug, Clone)]
pub enum BotEvent {
    /// A 50 ms physics tick elapsed.
    Tick,
    Login,
    Spawn,
    Death,
    Health,
    /// Server-forced teleport.
    ForcedMove,
    ChunkLoad(i32, i32),
    BlockUpdate(i32, i32, i32),
    EntitySpawn(i32),
    Chat(String),
    Inventory,
    Kicked(String),
    Packet(String),
}

pub struct Bot<'a> {
    pub client: Client,
    pub registry: &'a Registry,
    /// The bot's own entity (position/velocity/yaw/pitch/on_ground/effects).
    pub entity: Entity,
    pub game: GameInfo,
    pub health: f64,
    pub food: f64,
    pub food_saturation: f64,
    pub spawn_point: Vec3,
    pub time: TimeInfo,
    pub world: World<'a>,
    pub entities: HashMap<i32, Entity>,
    pub inventory: Window,
    /// Currently-open container window (chest/furnace/crafting table), if any.
    pub current_window: Option<Window>,
    pub held_slot: i32,
    pub control_state: ControlState,
    pub physics_enabled: bool,
    /// Movement rules for pathfinding. Defaults to `max_drop_down = 1` so the
    /// bot never takes a one-way drop it can't climb back up (surface-safe);
    /// raise it for tasks that deliberately descend (e.g. mining).
    pub movement: MovementsConfig,

    next_action_id: i32,
    /// Bumped on every inventory/window slot update from the server.
    inv_revision: u32,
    physics: Option<PhysicsEngine>,
    should_physics: bool,
    last_tick: Instant,
    /// When the server last teleported/corrected our position. Digs wait for this
    /// to be stale (position agreed) before mining, else they're out-of-reach.
    last_teleport: Instant,
    last_sent: Option<(Vec3, f64, f64)>,
    sequence: i32,
    spawned: bool,
    alive: bool,
    brand: String,
}

fn block_pos(x: i32, y: i32, z: i32) -> PValue {
    PValue::compound(vec![
        ("x", PValue::num(x as f64)),
        ("y", PValue::num(y as f64)),
        ("z", PValue::num(z as f64)),
    ])
}

impl<'a> Bot<'a> {
    /// Connect, log in, and advance to PLAY.
    pub async fn connect(options: ClientOptions, registry: &'a Registry) -> std::io::Result<Bot<'a>> {
        let mut client = Client::connect(&options.host, options.port, &options.username).await?;
        client.login(&options).await?;
        let mut entity = Entity::new(0);
        entity.health = 20.0;
        // Pickaxe-required blocks the bot can't break by hand. The pathfinder
        // must NOT route through these (it would tunnel "through" stone the bot
        // can never mine, and the follower would wedge digging it forever). The
        // mining task clears this once a pickaxe is in hand.
        let mut cant_break: std::collections::HashSet<i32> = std::collections::HashSet::new();
        for (name, def) in &registry.blocks_by_name {
            if name.contains("stone") || name.contains("ore") || name.contains("deepslate")
                || name.contains("obsidian") || name.contains("basalt") || name.contains("blackstone")
                || name.contains("granite") || name.contains("diorite") || name.contains("andesite")
                || name.contains("tuff") || name.contains("calcite") || name.contains("terracotta")
                || name.contains("brick") || name.contains("ancient_debris")
            {
                cant_break.insert(def.id);
            }
        }
        Ok(Bot {
            inventory: crate::window::create_window_from_type(registry, 0, -1, Some("minecraft:inventory"), "Inventory", None)
                .unwrap_or_else(|| Window::new(0, "minecraft:inventory", "", 46, 9, 44, 0, true)),
            current_window: None,
            next_action_id: 0,
            inv_revision: 0,
            world: World::new(registry),
            registry,
            entity,
            game: GameInfo {
                game_mode: "survival".into(),
                dimension: "overworld".into(),
                difficulty: "normal".into(),
                min_y: -64,
                height: 384,
                ..Default::default()
            },
            health: 20.0,
            food: 20.0,
            food_saturation: 5.0,
            spawn_point: Vec3::new(0.0, 0.0, 0.0),
            time: TimeInfo::default(),
            entities: HashMap::new(),
            held_slot: 0,
            control_state: ControlState::default(),
            physics_enabled: true,
            movement: MovementsConfig { liquid_cost: 100.0, max_drop_down: 1, blocks_cant_break: cant_break, ..MovementsConfig::default() }, // low drop + don't path through unbreakable stone
            physics: None,
            should_physics: false,
            last_tick: Instant::now(),
            last_teleport: Instant::now(),
            last_sent: None,
            sequence: 0,
            spawned: false,
            alive: true,
            brand: "vanilla".into(),
            client,
        })
    }

    pub fn username(&self) -> &str {
        &self.client.username
    }

    /// Held item (main hand).
    pub fn held_item(&self) -> Option<&Item> {
        self.inventory.slots.get(36 + self.held_slot as usize).and_then(|s| s.as_ref())
    }

    // ── Core drive loop ──

    /// Advance one step: handle a packet if one arrives before the 50 ms physics
    /// deadline, otherwise run a physics tick.
    pub async fn drive_tick(&mut self) -> std::io::Result<DriveStep> {
        let elapsed = self.last_tick.elapsed();
        if elapsed >= TICK {
            self.physics_tick().await?;
            self.last_tick = Instant::now();
            return Ok(DriveStep::Tick);
        }
        match tokio::time::timeout(TICK - elapsed, self.client.next_packet()).await {
            Ok(Ok(Some((name, params)))) => match self.handle_packet(&name, &params).await? {
                Some(ev) => Ok(DriveStep::Event(ev)),
                None => Ok(DriveStep::Handled),
            },
            Ok(Ok(None)) => Ok(DriveStep::Disconnected),
            Ok(Err(e)) => Err(e),
            Err(_) => {
                self.physics_tick().await?;
                self.last_tick = Instant::now();
                Ok(DriveStep::Tick)
            }
        }
    }

    /// Drive until a high-level event occurs. Returns `None` on disconnect.
    pub async fn next_event(&mut self) -> std::io::Result<Option<BotEvent>> {
        loop {
            match self.drive_tick().await? {
                DriveStep::Disconnected => return Ok(None),
                DriveStep::Event(ev) => return Ok(Some(ev)),
                DriveStep::Tick | DriveStep::Handled => continue,
            }
        }
    }

    /// Drive the loop for `n` physics ticks (stops early on disconnect).
    pub async fn wait_ticks(&mut self, n: u32) -> std::io::Result<()> {
        let mut ticks = 0;
        while ticks < n {
            match self.drive_tick().await? {
                DriveStep::Disconnected => return Ok(()),
                DriveStep::Tick => ticks += 1,
                _ => {}
            }
        }
        Ok(())
    }

    // ── Physics tick ──

    async fn physics_tick(&mut self) -> std::io::Result<()> {
        if !self.physics_enabled || !self.should_physics {
            return Ok(());
        }
        if self.physics.is_none() {
            self.physics = Some(PhysicsEngine::new(self.registry));
        }

        let controls = PlayerControls {
            forward: self.control_state.forward,
            back: self.control_state.back,
            left: self.control_state.left,
            right: self.control_state.right,
            jump: self.control_state.jump,
            sprint: self.control_state.sprint,
            sneak: self.control_state.sneak,
        };
        let mut state = create_player_state(self.registry, &self.entity, controls);
        {
            let pw = WorldPhysics::new(&self.world);
            let engine = self.physics.as_ref().unwrap();
            engine.simulate_player(&mut state, &pw as &dyn PhysicsWorld);
        }
        apply_player_state(&state, &mut self.entity);

        self.send_position().await
    }

    async fn send_position(&mut self) -> std::io::Result<()> {
        let pos = self.entity.position;
        let yaw = self.entity.yaw;
        let pitch = self.entity.pitch;
        let on_ground = self.entity.on_ground;

        let (pos_changed, look_changed) = match self.last_sent {
            Some((p, y, pi)) => (p != pos, y != yaw || pi != pitch),
            None => (true, true),
        };
        if !pos_changed && !look_changed {
            return Ok(());
        }
        let flags = PValue::compound(vec![("onGround", PValue::Bool(on_ground))]);

        if pos_changed && look_changed {
            self.client
                .write(
                    "move_player_pos_rot",
                    PValue::compound(vec![
                        ("x", PValue::num(pos.x)),
                        ("y", PValue::num(pos.y)),
                        ("z", PValue::num(pos.z)),
                        ("yaw", PValue::num(to_notchian_yaw(yaw))),
                        ("pitch", PValue::num(to_notchian_pitch(pitch))),
                        ("flags", flags),
                    ]),
                )
                .await?;
        } else if pos_changed {
            self.client
                .write(
                    "move_player_pos",
                    PValue::compound(vec![
                        ("x", PValue::num(pos.x)),
                        ("y", PValue::num(pos.y)),
                        ("z", PValue::num(pos.z)),
                        ("flags", flags),
                    ]),
                )
                .await?;
        } else {
            self.client
                .write(
                    "move_player_rot",
                    PValue::compound(vec![
                        ("yaw", PValue::num(to_notchian_yaw(yaw))),
                        ("pitch", PValue::num(to_notchian_pitch(pitch))),
                        ("flags", flags),
                    ]),
                )
                .await?;
        }
        self.last_sent = Some((pos, yaw, pitch));
        Ok(())
    }

    // ── Packet handling ──

    async fn handle_packet(&mut self, name: &str, params: &PValue) -> std::io::Result<Option<BotEvent>> {
        if std::env::var("PKT_DEBUG").is_ok()
            && (name.contains("block") || name.contains("ack") || name.contains("position") || name.contains("disconnect") || name == "system_chat")
        {
            eprintln!("PKT {name} {:?}", params);
        }
        match name {
            "login" => {
                self.handle_login(params).await?;
                return Ok(Some(BotEvent::Login));
            }
            "keep_alive" => {
                let id = params.get("keepAliveId").cloned().unwrap_or(PValue::Long(0));
                self.client.write("keep_alive", PValue::compound(vec![("keepAliveId", id)])).await?;
            }
            "ping" => {
                if let Some(id) = params.get("id").cloned() {
                    self.client.write("pong", PValue::compound(vec![("id", id)])).await?;
                }
            }
            "player_position" => {
                self.handle_position(params).await?;
                return Ok(Some(BotEvent::ForcedMove));
            }
            "set_health" => {
                if let Some(ev) = self.handle_health(params) {
                    return Ok(Some(ev));
                }
                return Ok(Some(BotEvent::Health));
            }
            "set_time" => self.handle_time(params),
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
                if let Some((cx, cz)) = self.handle_chunk(params) {
                    return Ok(Some(BotEvent::ChunkLoad(cx, cz)));
                }
            }
            "chunk_batch_finished" => {
                self.client
                    .write("chunk_batch_received", PValue::compound(vec![("chunksPerTick", PValue::num(20.0))]))
                    .await?;
            }
            "forget_level_chunk" => {
                let cx = params.get("chunkX").and_then(PValue::as_i32).unwrap_or(0);
                let cz = params.get("chunkZ").and_then(PValue::as_i32).unwrap_or(0);
                let _ = self.world.unload_column(cx, cz);
            }
            "block_update" => {
                if let Some(loc) = params.get("location") {
                    let (x, y, z) = loc_xyz(loc);
                    let state = params.get("type").and_then(PValue::as_i32).unwrap_or(0) as u32;
                    self.world.set_block_state_id(vec3(x as f64, y as f64, z as f64), state);
                    self.world.take_events();
                    return Ok(Some(BotEvent::BlockUpdate(x, y, z)));
                }
            }
            "set_held_slot" => {
                if let Some(s) = params.get("slot").and_then(PValue::as_i32) {
                    self.held_slot = s;
                }
            }
            "container_set_content" => {
                self.handle_inventory_content(params);
                return Ok(Some(BotEvent::Inventory));
            }
            "container_set_slot" => {
                self.handle_inventory_slot(params);
            }
            "set_player_inventory" => {
                if let (Some(slot), Some(item)) =
                    (params.get("slotId").and_then(PValue::as_i32), params.get("contents"))
                {
                    let i = slot as usize;
                    if i < self.inventory.slots.len() {
                        self.inventory.slots[i] = from_notch(self.registry, item);
                    }
                }
            }
            "open_screen" => {
                self.handle_open_screen(params);
            }
            "container_close" => {
                self.sync_window_to_inventory();
                self.current_window = None;
            }
            "add_entity" => {
                let id = params.get("entityId").and_then(PValue::as_i32).unwrap_or(0);
                let mut e = Entity::new(id);
                e.position = vec3(
                    params.get("x").and_then(PValue::as_f64).unwrap_or(0.0),
                    params.get("y").and_then(PValue::as_f64).unwrap_or(0.0),
                    params.get("z").and_then(PValue::as_f64).unwrap_or(0.0),
                );
                e.entity_type = params.get("type").and_then(PValue::as_i32);
                if let Some(reg_id) = e.entity_type {
                    e.init(self.registry, reg_id);
                }
                self.entities.insert(id, e);
                return Ok(Some(BotEvent::EntitySpawn(id)));
            }
            "move_entity_pos" | "move_entity_pos_rot" => {
                let id = params.get("entityId").and_then(PValue::as_i32).unwrap_or(0);
                if let Some(e) = self.entities.get_mut(&id) {
                    let g = |k: &str| params.get(k).and_then(PValue::as_f64).unwrap_or(0.0) / 4096.0;
                    e.position = e.position.offset(g("dx"), g("dy"), g("dz"));
                }
            }
            "entity_position_sync" | "teleport_entity" => {
                let id = params.get("entityId").and_then(PValue::as_i32).unwrap_or(0);
                if let Some(e) = self.entities.get_mut(&id) {
                    let g = |k: &str| params.get(k).and_then(PValue::as_f64);
                    if let (Some(x), Some(y), Some(z)) = (g("x"), g("y"), g("z")) {
                        e.position = vec3(x, y, z);
                    }
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
                let reason = params.get("reason").and_then(PValue::as_str).unwrap_or("disconnected").to_string();
                return Ok(Some(BotEvent::Kicked(reason)));
            }
            _ => return Ok(Some(BotEvent::Packet(name.to_string()))),
        }
        Ok(None)
    }

    async fn handle_login(&mut self, params: &PValue) -> std::io::Result<()> {
        self.entity.id = params.get("entityId").and_then(PValue::as_i32).unwrap_or(0);
        self.game.max_players = params.get("maxPlayers").and_then(PValue::as_i32).unwrap_or(0);
        self.game.hardcore = params.get("isHardcore").and_then(PValue::as_bool).unwrap_or(false);
        // Game mode lives in the per-world spawn info (1.20.5+); fall back to a
        // top-level field on older protocols. 0=survival 1=creative 2=adventure 3=spectator.
        let gm = params
            .get("worldState")
            .and_then(|w| w.get("gamemode"))
            .or_else(|| params.get("gamemode"))
            .or_else(|| params.get("gameMode"))
            .and_then(PValue::as_i32);
        if std::env::var("GM_DEBUG").is_ok() {
            eprintln!("LOGIN gamemode={gm:?} worldState={:?}", params.get("worldState"));
        }
        self.game.game_mode = match gm {
            Some(1) => "creative".into(),
            Some(2) => "adventure".into(),
            Some(3) => "spectator".into(),
            _ => "survival".into(),
        };

        let mut brand_data = Vec::new();
        crate::varint::push_var_int(&mut brand_data, self.brand.len() as i32);
        brand_data.extend_from_slice(self.brand.as_bytes());
        self.client
            .write(
                "custom_payload",
                PValue::compound(vec![("channel", PValue::str("minecraft:brand")), ("data", PValue::Bytes(brand_data))]),
            )
            .await?;
        self.send_settings().await?;
        if self.client.protocol_version >= 769 {
            let _ = self.client.write("player_loaded", PValue::compound(vec![])).await;
        }
        Ok(())
    }

    async fn send_settings(&mut self) -> std::io::Result<()> {
        self.client
            .write(
                "client_information",
                PValue::compound(vec![
                    ("locale", PValue::str("en_US")),
                    ("viewDistance", PValue::num(8.0)),
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
        let flags = params.get("flags");
        let flag = |k: &str| flags.and_then(|f| f.get(k)).and_then(PValue::as_bool).unwrap_or(false);
        let g = |k: &str| params.get(k).and_then(PValue::as_f64).unwrap_or(0.0);

        let p = self.entity.position;
        self.entity.position = vec3(
            if flag("x") { p.x + g("x") } else { g("x") },
            if flag("y") { p.y + g("y") } else { g("y") },
            if flag("z") { p.z + g("z") } else { g("z") },
        );
        let yaw = from_notchian_yaw(g("yaw"));
        let pitch = from_notchian_pitch(g("pitch"));
        self.entity.yaw = if flag("yaw") { self.entity.yaw + yaw } else { yaw };
        self.entity.pitch = if flag("pitch") { self.entity.pitch + pitch } else { pitch };
        self.entity.velocity = crate::vec3::ZERO;

        if let Some(id) = params.get("teleportId").cloned() {
            self.client.write("accept_teleportation", PValue::compound(vec![("teleportId", id)])).await?;
            self.last_teleport = Instant::now();
        }
        self.should_physics = true;
        self.last_sent = None;
        Ok(())
    }

    fn handle_health(&mut self, params: &PValue) -> Option<BotEvent> {
        self.health = params.get("health").and_then(PValue::as_f64).unwrap_or(self.health);
        self.food = params.get("food").and_then(PValue::as_f64).unwrap_or(self.food);
        self.food_saturation = params.get("foodSaturation").and_then(PValue::as_f64).unwrap_or(self.food_saturation);
        self.entity.health = self.health;
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
        let age = params.get("gameTime").and_then(PValue::as_i64).unwrap_or(self.time.age);
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
        self.time = TimeInfo { age, time_of_day: tod % 24000, day: tod / 24000, is_day: tod % 24000 < 13000 };
    }

    fn handle_chunk(&mut self, params: &PValue) -> Option<(i32, i32)> {
        let cx = params.get("x").and_then(PValue::as_i32)?;
        let cz = params.get("z").and_then(PValue::as_i32)?;
        let data = params.get("chunkData").and_then(PValue::as_bytes)?.to_vec();
        let min_y = self.game.min_y;
        let height = self.game.height;
        let no_array_length = self.client.protocol_version >= 770;
        let loaded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut col = ChunkColumn::new(ChunkColumnOptions {
                min_y: Some(min_y),
                world_height: Some(height),
                max_bits_per_block: GLOBAL_BITS_PER_BLOCK,
                max_bits_per_biome: GLOBAL_BITS_PER_BIOME,
            });
            col.load(&data, no_array_length);
            col
        }));
        match loaded {
            Ok(col) => {
                self.world.set_column(cx, cz, col);
                self.world.take_events();
                Some((cx, cz))
            }
            Err(_) => None,
        }
    }

    /// Choose which window a packet's `windowId` targets.
    fn window_for(&mut self, window_id: i32) -> Option<&mut Window> {
        if window_id == 0 || window_id == -1 {
            Some(&mut self.inventory)
        } else if self.current_window.as_ref().map(|w| w.id) == Some(window_id) {
            self.current_window.as_mut()
        } else {
            None
        }
    }

    fn handle_inventory_content(&mut self, params: &PValue) {
        self.inv_revision = self.inv_revision.wrapping_add(1);
        let window_id = params.get("windowId").and_then(PValue::as_i32).unwrap_or(-1);
        let state_id = params.get("stateId").and_then(PValue::as_i32);
        let registry = self.registry;
        let Some(window) = self.window_for(window_id) else {
            return;
        };
        if let Some(items) = params.get("items").and_then(PValue::as_list) {
            for (i, slot) in items.iter().enumerate() {
                if i < window.slots.len() {
                    window.slots[i] = from_notch(registry, slot);
                }
            }
        }
        if let Some(sid) = state_id {
            window.state_id = sid;
        }
        // Clear any items stuck in the 2x2 crafting grid after a resync.
        if window_id == 0 {
            for s in 1..=4 {
                if self.inventory.slots[s].is_some() {
                    for dest in 9..45 {
                        if self.inventory.slots[dest].is_none() {
                            self.inventory.slots[dest] = self.inventory.slots[s].take();
                            break;
                        }
                    }
                }
            }
        }
    }

    fn handle_inventory_slot(&mut self, params: &PValue) {
        self.inv_revision = self.inv_revision.wrapping_add(1);
        let window_id = params.get("windowId").and_then(PValue::as_i32).unwrap_or(-1);
        let state_id = params.get("stateId").and_then(PValue::as_i32);
        let registry = self.registry;
        let Some(slot) = params.get("slot").and_then(PValue::as_i32) else {
            return;
        };
        let item = params.get("item");
        let Some(window) = self.window_for(window_id) else {
            return;
        };
        let i = slot as usize;
        if i < window.slots.len() {
            window.slots[i] = item.and_then(|it| from_notch(registry, it));
        }
        if let Some(sid) = state_id {
            window.state_id = sid;
        }
    }

    fn handle_open_screen(&mut self, params: &PValue) {
        let window_id = params.get("windowId").and_then(PValue::as_i32).unwrap_or(0);
        let type_id = params.get("inventoryType").and_then(PValue::as_i64).unwrap_or(0);
        let title = params.get("windowTitle").and_then(PValue::as_str).unwrap_or("").to_string();
        let mut win =
            crate::window::create_window_from_type(self.registry, window_id, type_id, None, &title, None);
        // Seed the new window's inventory portion with our current inventory — the
        // client already knows its items; without this the container opens "empty",
        // crafts can't find ingredients, and closing wipes the real inventory.
        if let Some(w) = win.as_mut() {
            let inv_len = w.inventory_end - w.inventory_start;
            for i in 0..inv_len {
                let ws = w.inventory_start + i;
                let ps = self.inventory.inventory_start + i;
                if ws < w.slots.len() && ps < self.inventory.slots.len() {
                    w.slots[ws] = self.inventory.slots[ps].clone();
                }
            }
        }
        self.current_window = win;
    }

    /// Copy a closing container's inventory section back into the player inventory.
    fn sync_window_to_inventory(&mut self) {
        if let Some(w) = self.current_window.take() {
            let inv_len = w.inventory_end - w.inventory_start;
            for i in 0..inv_len {
                let cs = w.inventory_start + i;
                let ps = self.inventory.inventory_start + i;
                if cs < w.slots.len() && ps < self.inventory.slots.len() {
                    self.inventory.slots[ps] = w.slots[cs].clone();
                }
            }
        }
    }

    // ── World / block queries ──

    pub fn block_state_at(&self, x: i32, y: i32, z: i32) -> u32 {
        self.world.get_block_state_id(vec3(x as f64, y as f64, z as f64)).unwrap_or(0)
    }

    /// Block name + properties at a world coordinate (`None` if unloaded/air).
    pub fn block_at(&self, x: i32, y: i32, z: i32) -> Option<BlockInfo> {
        let state = self.world.get_block_state_id(vec3(x as f64, y as f64, z as f64))?;
        if state == 0 || !self.registry.blocks_by_state_id.contains_key(&state) {
            return None;
        }
        Some(state_id_to_block(self.registry, state))
    }

    /// Find up to `count` blocks matching `name` within `max_distance`,
    /// nearest first, requiring line-of-sight + an exposed face.
    pub fn find_blocks(&self, name: &str, max_distance: i32, count: usize) -> Vec<(i32, i32, i32)> {
        let Some(def) = self.registry.blocks_by_name.get(name) else {
            return vec![];
        };
        let target_id = def.id;
        let origin = self.entity.position;
        let (ox, oy, oz) = (origin.x.floor() as i32, origin.y.floor() as i32, origin.z.floor() as i32);
        let mut results = Vec::new();
        for dist in 0..=max_distance {
            for dx in -dist..=dist {
                for dy in -dist..=dist {
                    for dz in -dist..=dist {
                        if dx.abs() != dist && dy.abs() != dist && dz.abs() != dist {
                            continue;
                        }
                        if dx * dx + dy * dy + dz * dz > max_distance * max_distance {
                            continue;
                        }
                        let (x, y, z) = (ox + dx, oy + dy, oz + dz);
                        let Some(state) = self.world.get_block_state_id(vec3(x as f64, y as f64, z as f64)) else {
                            continue;
                        };
                        if state == 0 {
                            continue;
                        }
                        let matches = self.registry.blocks_by_state_id.get(&state).map(|d| d.id) == Some(target_id);
                        if matches && self.is_exposed(x, y, z) && self.can_see_block(x, y, z) {
                            results.push((x, y, z));
                            if results.len() >= count {
                                return results;
                            }
                        }
                    }
                }
            }
        }
        results
    }

    pub fn find_block(&self, name: &str, max_distance: i32) -> Option<(i32, i32, i32)> {
        self.find_blocks(name, max_distance, 1).into_iter().next()
    }

    fn is_exposed(&self, x: i32, y: i32, z: i32) -> bool {
        for (ox, oy, oz) in [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
            match self.world.get_block_state_id(vec3((x + ox) as f64, (y + oy) as f64, (z + oz) as f64)) {
                None | Some(0) => return true,
                Some(s) => {
                    if self.registry.blocks_by_state_id.get(&s).map(|d| d.transparent).unwrap_or(false) {
                        return true;
                    }
                }
            }
        }
        false
    }

    pub fn can_see_block(&self, x: i32, y: i32, z: i32) -> bool {
        let eye = vec3(self.entity.position.x, self.entity.position.y + PLAYER_EYE_HEIGHT, self.entity.position.z);
        let center = vec3(x as f64 + 0.5, y as f64 + 0.5, z as f64 + 0.5);
        let d = center.subtract(eye);
        let dist = d.length();
        if dist == 0.0 {
            return true;
        }
        let dir = d.scale(1.0 / dist);
        match raycast(&self.world, eye, dir, dist + 1.0, None) {
            None => true,
            Some(hit) => {
                hit.position.x.floor() as i32 == x
                    && hit.position.y.floor() as i32 == y
                    && hit.position.z.floor() as i32 == z
            }
        }
    }

    // ── Controls / look ──

    pub fn set_control_state(&mut self, control: &str, state: bool) {
        match control {
            "forward" => self.control_state.forward = state,
            "back" => self.control_state.back = state,
            "left" => self.control_state.left = state,
            "right" => self.control_state.right = state,
            "jump" => self.control_state.jump = state,
            "sprint" => self.control_state.sprint = state,
            "sneak" => self.control_state.sneak = state,
            _ => {}
        }
    }

    pub fn clear_control_states(&mut self) {
        self.control_state = ControlState::default();
    }

    /// Face a yaw/pitch (radians) immediately.
    pub fn look(&mut self, yaw: f64, pitch: f64) {
        self.entity.yaw = yaw;
        self.entity.pitch = pitch;
    }

    /// Face a world point immediately.
    pub fn look_at(&mut self, point: Vec3) {
        let eye = vec3(self.entity.position.x, self.entity.position.y + 1.62, self.entity.position.z);
        let delta = point.subtract(eye);
        let yaw = (-delta.x).atan2(-delta.z);
        let ground = (delta.x * delta.x + delta.z * delta.z).sqrt();
        let pitch = delta.y.atan2(ground);
        self.look(yaw, pitch);
    }

    // ── Actions ──

    /// Send a chat command (no leading slash).
    pub async fn run_command(&mut self, command: &str) -> std::io::Result<()> {
        self.client.write("chat_command", PValue::compound(vec![("command", PValue::str(command))])).await
    }

    pub async fn set_held_slot(&mut self, slot: i32) -> std::io::Result<()> {
        self.held_slot = slot;
        self.client.write("set_carried_item", PValue::compound(vec![("slotId", PValue::num(slot as f64))])).await
    }

    pub async fn swing_arm(&mut self) -> std::io::Result<()> {
        self.client.write("swing", PValue::compound(vec![("hand", PValue::num(0.0))])).await
    }

    /// Dig the block at (x,y,z): face it, send start, wait the break time while
    /// swinging, then send finish.
    pub async fn dig(&mut self, x: i32, y: i32, z: i32) -> std::io::Result<()> {
        let center = vec3(x as f64 + 0.5, y as f64 + 0.5, z as f64 + 0.5);
        let face = self.dig_face(x, y, z);
        // Stop moving and let the server agree on where we are before digging.
        // While walking/climbing the client position can run ahead of the
        // server's (which periodically teleports us back); a dig sent from the
        // client's position is then rejected as out-of-reach. Settling on the
        // ground with no controls lets the positions reconcile.
        self.clear_control_states();
        // Briefly stop and settle on the ground so client & server agree on our
        // position before mining — a dig sent mid-move is rejected as
        // out-of-reach (the server periodically teleports us back).
        for _ in 0..12 {
            if matches!(self.drive_tick().await?, DriveStep::Disconnected) {
                return Ok(());
            }
            if self.entity.on_ground && self.entity.velocity.length() < 0.05 {
                break;
            }
        }
        // Face the block and push that look to the server before starting — the
        // server validates the player is looking at the block.
        self.look_at(center);
        self.wait_ticks(2).await?;
        let time = self.dig_time(x, y, z);
        self.sequence += 1;
        let seq = self.sequence;
        self.client
            .write(
                "player_action",
                PValue::compound(vec![
                    ("status", PValue::num(0.0)),
                    ("location", block_pos(x, y, z)),
                    ("face", PValue::num(face as f64)),
                    ("sequence", PValue::num(seq as f64)),
                ]),
            )
            .await?;
        self.swing_arm().await?;

        // Mine for the computed break time (like the vanilla client), swinging
        // periodically and holding the look on the block, then send STOP. A small
        // margin covers rounding. Break early if the server turns it to air.
        let mine_for = if time.is_zero() { Duration::ZERO } else { time + Duration::from_millis(150) };
        let deadline = Instant::now() + mine_for;
        let start_state = self.block_state_at(x, y, z);
        while Instant::now() < deadline {
            self.look_at(center); // hold the look on the block while mining
            self.swing_arm().await?;
            self.wait_ticks(2).await?;
            if self.block_state_at(x, y, z) == 0 {
                break;
            }
        }
        let _ = start_state;

        self.sequence += 1;
        let fseq = self.sequence;
        self.client
            .write(
                "player_action",
                PValue::compound(vec![
                    ("status", PValue::num(2.0)),
                    ("location", block_pos(x, y, z)),
                    ("face", PValue::num(face as f64)),
                    ("sequence", PValue::num(fseq as f64)),
                ]),
            )
            .await?;

        // The server breaks the block in response to FINISH (status 2). With the
        // 1.19+ prediction model it does NOT echo a block_update back to the
        // breaker, so wait a few ticks (in case it does), then count nearby item
        // drops — a drop proves the break landed even with no block_update.
        let pre = self.block_state_at(x, y, z);
        for _ in 0..6 {
            if matches!(self.drive_tick().await?, DriveStep::Disconnected) {
                break;
            }
            if self.block_state_at(x, y, z) == 0 {
                break;
            }
        }
        // 1.19+ block prediction: the server breaks the block in response to
        // FINISH but does NOT echo a block_update back to the breaking player, so
        // our world would stay stale and we'd re-dig the same block forever.
        // Reflect the break locally for blocks that break by hand in this time.
        if self.block_state_at(x, y, z) != 0 && time < Duration::from_secs(4) {
            self.world.set_block_state_id(vec3(x as f64, y as f64, z as f64), 0);
        }
        let item_type = self.registry.entities_by_name.get("item").map(|d| d.id);
        let drops = self
            .entities
            .values()
            .filter(|e| item_type.is_none() || e.entity_type == item_type)
            .filter(|e| {
                let dx = e.position.x - (x as f64 + 0.5);
                let dy = e.position.y - (y as f64 + 0.5);
                let dz = e.position.z - (z as f64 + 0.5);
                dx * dx + dy * dy + dz * dz < 9.0
            })
            .count();
        if std::env::var("DIG_DEBUG").is_ok() {
            let p = self.entity.position;
            let dist = ((x as f64 + 0.5 - p.x).powi(2) + (y as f64 + 0.5 - p.y - 1.62).powi(2) + (z as f64 + 0.5 - p.z).powi(2)).sqrt();
            eprintln!(
                "DIG ({x},{y},{z}) bot=({:.1},{:.1},{:.1}) eyeDist={dist:.1} see={} ground={} sinceTp={}ms preFinish={pre} broke={} dropsNear={drops}",
                p.x, p.y, p.z, self.can_see_block(x, y, z), self.entity.on_ground, self.last_teleport.elapsed().as_millis(), self.block_state_at(x, y, z) == 0
            );
        }
        Ok(())
    }

    /// Dig toward (x,y,z): raycast from the eye and dig whatever solid block is
    /// in the way (e.g. a leaf occluding a trunk), or the target itself if the
    /// ray reaches it. Returns `true` if it dug the actual target. `false` if the
    /// target is out of reach. Call repeatedly to clear a path to a block.
    pub async fn dig_toward(&mut self, x: i32, y: i32, z: i32) -> std::io::Result<bool> {
        let eye = vec3(self.entity.position.x, self.entity.position.y + PLAYER_EYE_HEIGHT, self.entity.position.z);
        let center = vec3(x as f64 + 0.5, y as f64 + 0.5, z as f64 + 0.5);
        let d = center.subtract(eye);
        let dist = d.length();
        if dist > 5.5 {
            return Ok(false);
        }
        self.look_at(center);
        self.wait_ticks(2).await?;
        // The server validates line-of-sight, so an occluded block can't be dug —
        // we must clear the occluder first. Use the SAME raycast that
        // `can_see_block` uses and dig whatever it hits first: the target if the
        // line is clear, otherwise the occluding block (a leaf/trunk in the way).
        // Repeated calls tunnel a sight-line through to the trunk.
        let dir = d.scale(1.0 / dist.max(1e-6));
        match raycast(&self.world, eye, dir, dist + 1.0, None) {
            Some(hit) => {
                let (bx, by, bz) = (hit.position.x.floor() as i32, hit.position.y.floor() as i32, hit.position.z.floor() as i32);
                self.dig(bx, by, bz).await?;
                Ok((bx, by, bz) == (x, y, z))
            }
            None => {
                self.dig(x, y, z).await?;
                Ok(true)
            }
        }
    }

    /// The block face nearest the bot: 0 bottom, 1 top, 2 north(-z), 3 south(+z),
    /// 4 west(-x), 5 east(+x).
    fn dig_face(&self, x: i32, y: i32, z: i32) -> i32 {
        let eye = vec3(self.entity.position.x, self.entity.position.y + 1.62, self.entity.position.z);
        let d = eye.subtract(vec3(x as f64 + 0.5, y as f64 + 0.5, z as f64 + 0.5));
        let (ax, ay, az) = (d.x.abs(), d.y.abs(), d.z.abs());
        if ay >= ax && ay >= az {
            if d.y >= 0.0 { 1 } else { 0 }
        } else if ax >= az {
            if d.x >= 0.0 { 5 } else { 4 }
        } else if d.z >= 0.0 {
            3
        } else {
            2
        }
    }

    /// Estimated break time for the block at (x,y,z).
    pub fn dig_time(&self, x: i32, y: i32, z: i32) -> Duration {
        if self.game.game_mode == "creative" {
            return Duration::ZERO;
        }
        let state = self.block_state_at(x, y, z);
        let hardness = self.registry.blocks_by_state_id.get(&state).and_then(|d| d.hardness).unwrap_or(1.0);
        if hardness <= 0.0 {
            return Duration::ZERO;
        }
        let name = self.block_at(x, y, z).map(|b| b.name.clone()).unwrap_or_default();
        let speed = tool_speed(self.held_item().map(|i| i.name.as_str()), Some(name.clone()));
        // A block is "harvestable" (normal speed, /30) when it needs no specific
        // tool OR we hold the right one. Only tool-required blocks (stone/ores/
        // metal) are 5× slower by hand (/100). Wood/dirt/leaves/sand need no tool.
        let needs_tool = name.contains("stone") || name.contains("ore") || name.contains("deepslate")
            || name.contains("obsidian") || name.ends_with("_block") && (name.contains("iron") || name.contains("gold") || name.contains("diamond") || name.contains("copper") || name.contains("netherite"))
            || name.contains("anvil") || name.contains("furnace") || name.contains("brick");
        let can_harvest = !needs_tool || speed > 1.0;
        let damage = speed / hardness / if can_harvest { 30.0 } else { 100.0 };
        if damage >= 1.0 {
            return Duration::ZERO;
        }
        let ticks = (1.0 / damage).ceil() as u64;
        Duration::from_millis(ticks * 50)
    }

    /// Place the held item against a block face.
    pub async fn place_block(&mut self, x: i32, y: i32, z: i32, face: Face) -> std::io::Result<()> {
        self.sequence += 1;
        let seq = self.sequence;
        let held = self.held_item().map(|i| i.name.clone());
        self.client
            .write(
                "use_item_on",
                PValue::compound(vec![
                    ("hand", PValue::num(0.0)),
                    ("location", block_pos(x, y, z)),
                    ("direction", PValue::num(face as i32 as f64)),
                    ("cursorX", PValue::num(0.5)),
                    ("cursorY", PValue::num(0.5)),
                    ("cursorZ", PValue::num(0.5)),
                    ("insideBlock", PValue::Bool(false)),
                    ("worldBorderHit", PValue::Bool(false)),
                    ("sequence", PValue::num(seq as f64)),
                ]),
            )
            .await?;
        // Predict the placement: the server places the block but (1.19+) sends no
        // block_update back to the placer, so reflect it locally — the new block
        // appears one step out from the clicked face.
        let (ox, oy, oz) = match face {
            Face::Bottom => (0, -1, 0),
            Face::Top => (0, 1, 0),
            Face::North => (0, 0, -1),
            Face::South => (0, 0, 1),
            Face::West => (-1, 0, 0),
            Face::East => (1, 0, 0),
        };
        let (px, py, pz) = (x + ox, y + oy, z + oz);
        if self.block_state_at(px, py, pz) == 0 {
            let state = held.and_then(|n| {
                let key = n.strip_prefix("minecraft:").unwrap_or(&n).to_string();
                self.registry.blocks_by_name.get(&key).map(|b| b.default_state)
            });
            if let Some(state) = state {
                self.world.set_block_state_id(vec3(px as f64, py as f64, pz as f64), state);
            }
        }
        Ok(())
    }

    /// Walk to within ~2 blocks of (x,y,z). Returns `true` if the goal is met.
    pub async fn goto(&mut self, x: i32, y: i32, z: i32) -> std::io::Result<bool> {
        let goal = GoalNear::new(x as f64, y as f64, z as f64, 2.0);
        self.goto_goal(&goal, Duration::from_secs(18)).await
    }

    /// Walk to within `range` blocks of (x,y,z).
    pub async fn goto_near(&mut self, x: i32, y: i32, z: i32, range: f64) -> std::io::Result<bool> {
        let goal = GoalNear::new(x as f64, y as f64, z as f64, range);
        self.goto_goal(&goal, Duration::from_secs(18)).await
    }

    /// Walk to within `range` blocks horizontally of (x,z) — at any reachable Y.
    /// Use for descending to something (e.g. a tree column in a valley).
    pub async fn goto_xz(&mut self, x: i32, z: i32, range: f64) -> std::io::Result<bool> {
        let goal = GoalNearXZ::new(x as f64, z as f64, range);
        self.goto_goal(&goal, Duration::from_secs(18)).await
    }

    fn goal_reached(&self, goal: &dyn Goal) -> bool {
        let p = self.entity.position;
        goal.is_end(p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32)
    }

    /// Navigate to a goal: compute an A* path, follow it (digging obstacles,
    /// jumping, dropping), and re-path when stuck or the path runs out before the
    /// goal. Single-task port of typecraft's tick-driven pathfinder follower.
    pub async fn goto_goal(&mut self, goal: &dyn Goal, timeout: Duration) -> std::io::Result<bool> {
        let started = Instant::now();
        let mut retries = 0;
        loop {
            if self.goal_reached(goal) {
                self.clear_control_states();
                self.wait_ticks(2).await?;
                return Ok(true);
            }
            if started.elapsed() > timeout {
                self.clear_control_states();
                return Ok(self.goal_reached(goal));
            }
            let start = (
                self.entity.position.x.floor() as i32,
                self.entity.position.y.floor() as i32,
                self.entity.position.z.floor() as i32,
            );
            let result = get_path_to(
                &self.world,
                start,
                goal,
                self.movement.clone(),
                -1.0,
                Duration::from_millis(2000),
            );
            if std::env::var("GOTO_DEBUG").is_ok() && retries == 0 {
                let end = result.path.last().map(|m| (m.x, m.y, m.z));
                eprintln!("GOTO from {start:?} status={:?} len={} end={end:?}", result.status, result.path.len());
            }
            if result.path.is_empty() {
                if result.status == PathStatus::NoPath {
                    self.clear_control_states();
                    return Ok(false);
                }
                retries += 1;
                if retries > 6 {
                    self.clear_control_states();
                    return Ok(false);
                }
                self.wait_ticks(4).await?;
                continue;
            }
            match self.follow_path(&result.path).await? {
                FollowOutcome::Reached => {
                    // Reached the path's end; loop re-checks the goal / re-paths.
                    retries += 1;
                    if retries > 12 {
                        self.clear_control_states();
                        return Ok(self.goal_reached(goal));
                    }
                }
                FollowOutcome::NeedRepath => {
                    retries += 1;
                    if retries > 12 {
                        self.clear_control_states();
                        return Ok(self.goal_reached(goal));
                    }
                }
                FollowOutcome::Disconnected => return Ok(false),
            }
        }
    }

    /// Follow a fixed path until it ends, the bot gets stuck, or it needs to
    /// place a block (unsupported → re-path). Digs `to_break` blocks in the way.
    async fn follow_path(&mut self, path: &[Move]) -> std::io::Result<FollowOutcome> {
        // Start at the waypoint nearest the bot (skip already-passed nodes).
        let p = self.entity.position;
        let mut idx = 0;
        let mut best = f64::MAX;
        for (i, m) in path.iter().enumerate() {
            let dx = m.x as f64 + 0.5 - p.x;
            let dz = m.z as f64 + 0.5 - p.z;
            let d = dx * dx + dz * dz;
            if d < best {
                best = d;
                idx = i;
            }
        }

        let follow_start = Instant::now();
        let mut last_progress = Instant::now();
        let mut dig_progress = 0usize;
        let mut last_xz = (self.entity.position.x, self.entity.position.z);
        let mut stuck_ticks = 0u32;
        let debug = std::env::var("FOLLOW_DEBUG").is_ok();
        let mut dbgi = 0u32;
        while idx < path.len() {
            if debug && dbgi % 8 == 0 {
                let pp = self.entity.position;
                let n = &path[idx];
                eprintln!(
                    "FOLLOW idx={idx}/{} bot=({:.1},{:.1},{:.1}) wp=({},{},{}) dy={:.1} stuck={stuck_ticks} ground={} break={}",
                    path.len(), pp.x, pp.y, pp.z, n.x, n.y, n.z, n.y as f64 - pp.y, self.entity.on_ground, n.to_break.len()
                );
            }
            dbgi += 1;
            // Hard cap so one path segment can't exceed the overall goto budget.
            if follow_start.elapsed() > Duration::from_secs(12) {
                self.clear_control_states();
                return Ok(FollowOutcome::NeedRepath);
            }
            let next = &path[idx];
            let p = self.entity.position;
            let dx = next.x as f64 + 0.5 - p.x;
            let dz = next.z as f64 + 0.5 - p.z;
            let dy = next.y as f64 - p.y;

            // Reached the waypoint only when at/above its level (dy <= 0.6) —
            // for an upward step this forces the bot to actually CLIMB before
            // advancing (a loose dy tolerance let it skip climbs and stall at the
            // base of ledges); for a drop it's already above, so it advances.
            if dx * dx + dz * dz <= 0.49 && dy <= 0.6 {
                idx += 1;
                dig_progress = 0;
                last_progress = Instant::now();
                continue;
            }

            // Dig any blocks the move requires breaking, one at a time.
            if dig_progress < next.to_break.len() {
                let (bx, by, bz) = next.to_break[dig_progress];
                if self.block_state_at(bx, by, bz) != 0 {
                    self.clear_control_states();
                    self.dig(bx, by, bz).await?;
                }
                dig_progress += 1;
                last_progress = Instant::now();
                continue;
            }

            // Block placement (scaffolding) isn't supported yet — re-path around it.
            if !next.to_place.is_empty() {
                self.clear_control_states();
                return Ok(FollowOutcome::NeedRepath);
            }

            // Walk toward the waypoint; jump to step up, for parkour, or to clear
            // a lip when we've stopped making horizontal progress (anti-wedge).
            let mx = next.x as f64 + 0.5 - p.x;
            let mz = next.z as f64 + 0.5 - p.z;
            self.look((-mx).atan2(-mz), 0.0);
            self.set_control_state("forward", true);
            self.set_control_state("sprint", true);
            // Jump to climb only when CLOSE to the up-step (so we walk up to it with
            // ground momentum and step onto it), or for parkour. Jumping while far
            // from the step just bounces in open air with no forward progress.
            let near = dx * dx + dz * dz < 1.6;
            self.set_control_state("jump", next.parkour || (next.y as f64 > p.y + 0.5 && near));

            let step = self.drive_tick().await?;
            if matches!(step, DriveStep::Disconnected) {
                return Ok(FollowOutcome::Disconnected);
            }

            // Only judge "stuck" on actual PHYSICS ticks — the loop also spins on
            // packet I/O (no movement), and counting those as stuck makes the bot
            // re-path constantly and crawl. One physics tick = one chance to move.
            if matches!(step, DriveStep::Tick) {
                let np = self.entity.position;
                let moved = (np.x - last_xz.0).powi(2) + (np.z - last_xz.1).powi(2);
                if moved > 0.0009 {
                    last_xz = (np.x, np.z);
                    stuck_ticks = 0;
                } else {
                    stuck_ticks += 1;
                }
            }

            // Re-path when genuinely stuck: ~40 ticks with no horizontal movement
            // (tick-based — reliable even when the loop spins on packet I/O), or
            // 2.5 s wall-clock without reaching the next waypoint.
            if stuck_ticks >= 40 || last_progress.elapsed() > Duration::from_millis(2500) {
                self.clear_control_states();
                return Ok(FollowOutcome::NeedRepath);
            }
        }
        self.clear_control_states();
        Ok(FollowOutcome::Reached)
    }

    pub async fn respawn(&mut self) -> std::io::Result<()> {
        self.client
            .write("client_command", PValue::compound(vec![("actionId", PValue::num(0.0)), ("payload", PValue::num(0.0))]))
            .await
    }
}

fn loc_xyz(loc: &PValue) -> (i32, i32, i32) {
    (
        loc.get("x").and_then(PValue::as_i32).unwrap_or(0),
        loc.get("y").and_then(PValue::as_i32).unwrap_or(0),
        loc.get("z").and_then(PValue::as_i32).unwrap_or(0),
    )
}

/// Coarse tool-speed multiplier (typecraft's fallback when material data is absent).
fn tool_speed(tool: Option<&str>, block: Option<String>) -> f64 {
    let (Some(tool), Some(block)) = (tool, block) else {
        return 1.0;
    };
    let tier = match tool.split('_').next().unwrap_or("") {
        "wooden" => 2.0,
        "stone" => 4.0,
        "iron" => 6.0,
        "golden" => 12.0,
        "diamond" => 8.0,
        "netherite" => 9.0,
        _ => return 1.0,
    };
    let ttype = tool.rsplit('_').next().unwrap_or("");
    let pick = block.contains("stone") || block.contains("ore") || block.contains("brick") || block.contains("deepslate");
    let axe = block.contains("log") || block.contains("planks") || block.contains("wood");
    let shovel = block.contains("dirt") || block.contains("sand") || block.contains("gravel") || block == "grass_block";
    match ttype {
        "pickaxe" if pick => tier,
        "axe" if axe => tier,
        "shovel" if shovel => tier,
        _ => 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_speed_matches() {
        assert_eq!(tool_speed(Some("stone_pickaxe"), Some("stone".into())), 4.0);
        assert_eq!(tool_speed(Some("wooden_axe"), Some("oak_log".into())), 2.0);
        assert_eq!(tool_speed(Some("diamond_pickaxe"), Some("dirt".into())), 1.0);
        assert_eq!(tool_speed(None, Some("stone".into())), 1.0);
    }
}
