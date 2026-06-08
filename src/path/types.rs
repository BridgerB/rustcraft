//! Pathfinding types: moves, goals, configs, and block queries.
//! Port of typecraft's `path/types.ts`.

/// Collision-free position hash, unique for x,z ∈ [-30000,30000], y ∈ [-64,320].
pub fn pos_hash(x: i32, y: i32, z: i32) -> i64 {
    (x as i64 * 60001 + z as i64) * 385 + (y as i64 + 64)
}

/// A block placement action executed while following a path.
#[derive(Debug, Clone)]
pub struct PlaceAction {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub dx: i32,
    pub dy: i32,
    pub dz: i32,
    pub jump: bool,
    pub return_pos: Option<(i32, i32, i32)>,
}

/// A discrete position node in the A* search graph.
#[derive(Debug, Clone)]
pub struct Move {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub cost: f64,
    pub hash: i64,
    pub remaining_blocks: i32,
    pub to_break: Vec<(i32, i32, i32)>,
    pub to_place: Vec<PlaceAction>,
    pub parkour: bool,
}

impl Move {
    pub fn start(x: i32, y: i32, z: i32) -> Move {
        Move {
            x,
            y,
            z,
            cost: 0.0,
            hash: pos_hash(x, y, z),
            remaining_blocks: 0,
            to_break: Vec::new(),
            to_place: Vec::new(),
            parkour: false,
        }
    }
}

/// Drives A* toward a target.
pub trait Goal {
    fn heuristic(&self, x: i32, y: i32, z: i32) -> f64;
    fn is_end(&self, x: i32, y: i32, z: i32) -> bool;
    fn has_changed(&mut self) -> bool {
        false
    }
    fn is_valid(&self) -> bool {
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathStatus {
    Success,
    Timeout,
    NoPath,
    Partial,
}

#[derive(Debug, Clone)]
pub struct PathResult {
    pub status: PathStatus,
    pub path: Vec<Move>,
    pub cost: f64,
    pub visited_nodes: usize,
    pub generated_nodes: usize,
}

/// Configuration for movement generation.
#[derive(Debug, Clone)]
pub struct MovementsConfig {
    pub can_dig: bool,
    pub dig_cost: f64,
    pub place_cost: f64,
    pub liquid_cost: f64,
    pub entity_cost: f64,
    pub dont_create_flow: bool,
    pub dont_mine_under_falling_block: bool,
    pub allow_1by1_towers: bool,
    pub allow_parkour: bool,
    pub allow_sprinting: bool,
    pub max_drop_down: i32,
    pub infinite_liquid_dropdown_distance: bool,
    pub blocks_cant_break: std::collections::HashSet<i32>,
    pub blocks_to_avoid: std::collections::HashSet<i32>,
    pub scaffolding_blocks: Vec<i32>,
}

impl Default for MovementsConfig {
    fn default() -> Self {
        MovementsConfig {
            can_dig: true,
            dig_cost: 1.0,
            place_cost: 1.0,
            liquid_cost: 1.0,
            entity_cost: 1.0,
            dont_create_flow: true,
            dont_mine_under_falling_block: true,
            allow_1by1_towers: true,
            allow_parkour: true,
            allow_sprinting: true,
            max_drop_down: 4,
            infinite_liquid_dropdown_distance: true,
            blocks_cant_break: std::collections::HashSet::new(),
            blocks_to_avoid: std::collections::HashSet::new(),
            scaffolding_blocks: Vec::new(),
        }
    }
}

/// Simplified block classification for movement decisions.
#[derive(Debug, Clone)]
pub struct BlockQuery {
    pub safe: bool,
    pub physical: bool,
    pub liquid: bool,
    pub lava: bool,
    pub climbable: bool,
    pub height: f64,
    pub name: String,
    pub replaceable: bool,
    pub can_fall: bool,
    pub openable: bool,
    pub id: i32,
}

impl BlockQuery {
    /// Void/unloaded — impassable.
    pub fn void() -> BlockQuery {
        BlockQuery {
            safe: false,
            physical: false,
            liquid: false,
            lava: false,
            climbable: false,
            height: 0.0,
            name: "void".into(),
            replaceable: false,
            can_fall: false,
            openable: false,
            id: -1,
        }
    }
}
