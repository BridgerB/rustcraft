//! Bot — connects, logs in, runs a 50 ms physics tick (movement sim + position
//! packets), tracks world/inventory/entities, and exposes high-level actions
//! (block queries, controls, look, dig, pathfinding `goto`). Faithful-in-spirit
//! port of typecraft's event-driven `bot`, adapted to a single-task Rust model:
//! every action drives [`Bot::drive_tick`], which races packet reads against the
//! 50 ms physics deadline, so keep-alive + physics keep running while waiting.

mod conversions;

pub use conversions::*;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::block::{state_id_to_block, BlockInfo};
use crate::chunk::{ChunkColumn, ChunkColumnOptions, GLOBAL_BITS_PER_BIOME, GLOBAL_BITS_PER_BLOCK};
use crate::entity::Entity;
use crate::item::{from_notch, Item};
use crate::path::{get_path_to, GoalNear, MovementsConfig, PathStatus};
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
    pub held_slot: i32,
    pub control_state: ControlState,
    pub physics_enabled: bool,

    physics: Option<PhysicsEngine>,
    should_physics: bool,
    last_tick: Instant,
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
        Ok(Bot {
            inventory: Window::new(0, "minecraft:inventory", "", 46, 9, 44, 0, true),
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
            physics: None,
            should_physics: false,
            last_tick: Instant::now(),
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
            "set_carried_item" => {
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

    fn handle_inventory_content(&mut self, params: &PValue) {
        if params.get("windowId").and_then(PValue::as_i32).unwrap_or(-1) != 0 {
            return;
        }
        if let Some(items) = params.get("items").and_then(PValue::as_list) {
            for (i, slot) in items.iter().enumerate() {
                if i < self.inventory.slots.len() {
                    self.inventory.slots[i] = from_notch(self.registry, slot);
                }
            }
        }
    }

    fn handle_inventory_slot(&mut self, params: &PValue) {
        if params.get("windowId").and_then(PValue::as_i32).unwrap_or(-1) != 0 {
            return;
        }
        if let (Some(slot), Some(item)) = (params.get("slot").and_then(PValue::as_i32), params.get("item")) {
            let i = slot as usize;
            if i < self.inventory.slots.len() {
                self.inventory.slots[i] = from_notch(self.registry, item);
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
        self.look_at(vec3(x as f64 + 0.5, y as f64 + 0.5, z as f64 + 0.5));
        let time = self.dig_time(x, y, z);
        self.sequence += 1;
        let seq = self.sequence;
        self.client
            .write(
                "player_action",
                PValue::compound(vec![
                    ("status", PValue::num(0.0)),
                    ("location", block_pos(x, y, z)),
                    ("face", PValue::num(1.0)),
                    ("sequence", PValue::num(seq as f64)),
                ]),
            )
            .await?;
        self.swing_arm().await?;

        let deadline = Instant::now() + time;
        while Instant::now() < deadline {
            self.swing_arm().await?;
            self.wait_ticks(7).await?;
            if self.block_state_at(x, y, z) == 0 {
                break;
            }
        }

        self.sequence += 1;
        let fseq = self.sequence;
        self.client
            .write(
                "player_action",
                PValue::compound(vec![
                    ("status", PValue::num(2.0)),
                    ("location", block_pos(x, y, z)),
                    ("face", PValue::num(1.0)),
                    ("sequence", PValue::num(fseq as f64)),
                ]),
            )
            .await
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
        let speed = tool_speed(self.held_item().map(|i| i.name.as_str()), self.block_at(x, y, z).map(|b| b.name));
        let can_harvest = speed > 1.0;
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
            .await
    }

    /// Walk to within ~1 block of (x,y,z) using A* + the physics engine.
    /// Returns `true` if it arrived.
    pub async fn goto(&mut self, x: i32, y: i32, z: i32) -> std::io::Result<bool> {
        let start = (
            self.entity.position.x.floor() as i32,
            self.entity.position.y.floor() as i32,
            self.entity.position.z.floor() as i32,
        );
        let goal = GoalNear::new(x as f64, y as f64, z as f64, 1.0);
        let result = get_path_to(&self.world, start, &goal, MovementsConfig::default(), 256.0, Duration::from_secs(5));
        if result.status != PathStatus::Success && result.path.is_empty() {
            return Ok(false);
        }

        self.set_control_state("sprint", true);
        for node in &result.path {
            let target = vec3(node.x as f64 + 0.5, node.y as f64, node.z as f64 + 0.5);
            let mut ticks_without_progress = 0u32;
            let mut best = self.entity.position.distance_xz(target);
            loop {
                self.look_at(vec3(target.x, self.entity.position.y, target.z));
                self.set_control_state("forward", true);
                let need_jump = node.y as f64 > self.entity.position.y + 0.4;
                self.set_control_state("jump", need_jump);

                match self.drive_tick().await? {
                    DriveStep::Disconnected => return Ok(false),
                    DriveStep::Tick => {
                        let dxz = self.entity.position.distance_xz(target);
                        let dy = (self.entity.position.y - node.y as f64).abs();
                        if dxz < 0.4 && dy < 1.2 {
                            break;
                        }
                        if dxz < best - 0.05 {
                            best = dxz;
                            ticks_without_progress = 0;
                        } else {
                            ticks_without_progress += 1;
                            // ~1.5 s of no progress toward this node — give up on it.
                            if ticks_without_progress > 30 {
                                break;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        self.clear_control_states();
        self.wait_ticks(2).await?;
        let arrived = self.entity.position.distance(vec3(x as f64, y as f64, z as f64)) < 2.5;
        Ok(arrived)
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
