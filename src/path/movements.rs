//! Movement generation for A* — classifies blocks and yields valid neighboring
//! moves (walk, jump, drop, diagonal, parkour, tower, swim). Port of typecraft's
//! `path/movements.ts`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use crate::physics::{PhysicsWorld, WorldPhysics};
use crate::registry::Registry;
use crate::world::World;

use super::astar::NeighborGen;
use super::types::{pos_hash, BlockQuery, Move, MovementsConfig, PlaceAction};

const CARDINAL: [(i32, i32); 4] = [(-1, 0), (1, 0), (0, -1), (0, 1)];
const DIAGONAL: [(i32, i32); 4] = [(-1, -1), (-1, 1), (1, -1), (1, 1)];
const SQRT2: f64 = std::f64::consts::SQRT_2;

pub struct Movements<'a> {
    physics: WorldPhysics<'a>,
    pub config: MovementsConfig,
    liquid_ids: HashSet<i32>,
    lava_ids: HashSet<i32>,
    climbable_ids: HashSet<i32>,
    avoid_ids: HashSet<i32>,
    fence_ids: HashSet<i32>,
    gravity_ids: HashSet<i32>,
    gate_ids: HashSet<i32>,
    cache: RefCell<HashMap<i64, BlockQuery>>,
}

fn ids_for(registry: &Registry, names: &[&str]) -> HashSet<i32> {
    names
        .iter()
        .filter_map(|n| registry.blocks_by_name.get(*n).map(|b| b.id))
        .collect()
}

impl<'a> Movements<'a> {
    pub fn new(world: &'a World<'a>, config: MovementsConfig) -> Movements<'a> {
        let registry = world.registry;

        let liquid_ids = ids_for(
            registry,
            &["water", "flowing_water", "lava", "flowing_lava"],
        );
        let lava_ids = ids_for(registry, &["lava", "flowing_lava"]);
        let avoid_ids = ids_for(
            registry,
            &[
                "lava",
                "flowing_lava",
                "fire",
                "soul_fire",
                "cobweb",
                "web",
                "sweet_berry_bush",
                "wither_rose",
            ],
        );
        let climbable_ids = ids_for(registry, &["ladder", "vine"]);
        let gravity_ids = ids_for(registry, &["sand", "gravel", "red_sand"]);

        // Fences = blocks whose collision shape is taller than 1.0.
        let mut fence_ids = HashSet::new();
        let shapes = &registry.block_collision_shapes;
        for block in &registry.blocks_array {
            if let Some(shape_ref) = shapes.blocks.get(&block.name) {
                let shape_id = shape_ref
                    .as_u64()
                    .or_else(|| {
                        shape_ref
                            .as_array()
                            .and_then(|a| a.first())
                            .and_then(|v| v.as_u64())
                    })
                    .unwrap_or(0);
                if let Some(shape_list) = shapes.shapes.get(&shape_id.to_string()) {
                    if shape_list
                        .iter()
                        .any(|s| s.get(4).copied().unwrap_or(0.0) > 1.0)
                    {
                        fence_ids.insert(block.id);
                    }
                }
            }
        }

        let gate_ids = registry
            .blocks_array
            .iter()
            .filter(|b| b.name.contains("fence_gate"))
            .map(|b| b.id)
            .collect();

        Movements {
            physics: WorldPhysics::new(world),
            config,
            liquid_ids,
            lava_ids,
            climbable_ids,
            avoid_ids,
            fence_ids,
            gravity_ids,
            gate_ids,
            cache: RefCell::new(HashMap::new()),
        }
    }

    fn query(&self, x: i32, y: i32, z: i32) -> BlockQuery {
        let key = pos_hash(x, y, z);
        if let Some(cached) = self.cache.borrow().get(&key) {
            return cached.clone();
        }
        let block = self.physics.get_block(x as f64, y as f64, z as f64);
        let q = match block {
            None => BlockQuery::void(),
            Some(block) => {
                let is_liquid = self.liquid_ids.contains(&block.id);
                let is_lava = self.lava_ids.contains(&block.id);
                let is_climbable = self.climbable_ids.contains(&block.id);
                let is_avoid = self.avoid_ids.contains(&block.id);
                let is_fence = self.fence_ids.contains(&block.id);
                let is_empty = block.bounding_box == "empty";
                let is_physical = block.bounding_box == "block" && !is_fence;
                let height = block
                    .shapes
                    .iter()
                    .map(|s| s.get(4).copied().unwrap_or(0.0))
                    .fold(0.0, f64::max);
                let is_carpet = is_empty && height > 0.0 && height < 0.1;
                let is_safe = (is_empty || is_climbable || is_liquid || is_carpet) && !is_avoid;
                BlockQuery {
                    safe: is_safe,
                    physical: is_physical,
                    liquid: is_liquid,
                    lava: is_lava,
                    climbable: is_climbable,
                    height,
                    replaceable: is_empty && !is_climbable && !is_liquid,
                    can_fall: self.gravity_ids.contains(&block.id),
                    openable: self.gate_ids.contains(&block.id),
                    name: block.name,
                    id: block.id,
                }
            }
        };
        self.cache.borrow_mut().insert(key, q.clone());
        q
    }

    /// 0 if safe, dig_cost if breakable (records break), -1 if impassable.
    fn safe_or_break(&self, x: i32, y: i32, z: i32, to_break: &mut Vec<(i32, i32, i32)>) -> f64 {
        let block = self.query(x, y, z);
        // Lava is a liquid but is NEVER passable — treat it as a wall the path must
        // route around (water is fine to wade through). Also avoid stepping right
        // next to lava: a block with lava on any side is a death trap (drift/knock).
        if block.lava {
            return -1.0;
        }
        for (dx, dy, dz) in [(1, 0, 0), (-1, 0, 0), (0, 0, 1), (0, 0, -1), (0, -1, 0)] {
            if self.query(x + dx, y + dy, z + dz).lava {
                return -1.0;
            }
        }
        if block.safe || block.liquid {
            return 0.0;
        }
        if !self.config.can_dig || self.config.blocks_cant_break.contains(&block.id) {
            return -1.0;
        }
        if self.config.dont_create_flow {
            for (dx, dz) in [(0, 1), (0, -1), (1, 0), (-1, 0)] {
                if self.query(x + dx, y, z + dz).liquid {
                    return -1.0;
                }
            }
            if self.query(x, y + 1, z).liquid {
                return -1.0;
            }
        }
        if self.config.dont_mine_under_falling_block && self.query(x, y + 1, z).can_fall {
            return -1.0;
        }
        to_break.push((x, y, z));
        self.config.dig_cost
    }

    fn push(
        &self,
        out: &mut Vec<Move>,
        x: i32,
        y: i32,
        z: i32,
        cost: f64,
        to_break: Vec<(i32, i32, i32)>,
        to_place: Vec<PlaceAction>,
        parkour: bool,
    ) {
        out.push(Move {
            x,
            y,
            z,
            cost,
            hash: pos_hash(x, y, z),
            remaining_blocks: 0,
            to_break,
            to_place,
            parkour,
        });
    }

    fn move_forward(&self, node: &Move, dx: i32, dz: i32, out: &mut Vec<Move>) {
        let (nx, nz) = (node.x + dx, node.z + dz);
        let floor = self.query(nx, node.y - 1, nz);
        if !floor.physical && !floor.liquid {
            return;
        }
        let mut to_break = Vec::new();
        let mut cost = 1.0;
        if floor.liquid {
            cost *= self.config.liquid_cost;
        }
        let body = self.safe_or_break(nx, node.y, nz, &mut to_break);
        if body < 0.0 {
            return;
        }
        cost += body;
        let head = self.safe_or_break(nx, node.y + 1, nz, &mut to_break);
        if head < 0.0 {
            return;
        }
        cost += head;
        self.push(out, nx, node.y, nz, cost, to_break, vec![], false);
    }

    fn move_jump_up(&self, node: &Move, dx: i32, dz: i32, out: &mut Vec<Move>) {
        let (nx, nz, ny) = (node.x + dx, node.z + dz, node.y + 1);
        let mut to_break = Vec::new();
        let mut cost = 2.0;
        let above = self.safe_or_break(node.x, node.y + 2, node.z, &mut to_break);
        if above < 0.0 {
            return;
        }
        cost += above;
        let jump_block = self.query(nx, node.y, nz);
        if !jump_block.physical && !jump_block.liquid {
            return;
        }
        let body = self.safe_or_break(nx, ny, nz, &mut to_break);
        if body < 0.0 {
            return;
        }
        cost += body;
        let head = self.safe_or_break(nx, ny + 1, nz, &mut to_break);
        if head < 0.0 {
            return;
        }
        cost += head;
        self.push(out, nx, ny, nz, cost, to_break, vec![], false);
    }

    fn move_drop_down(&self, node: &Move, dx: i32, dz: i32, out: &mut Vec<Move>) {
        let (nx, nz) = (node.x + dx, node.z + dz);
        if !self.query(nx, node.y + 1, nz).safe {
            return;
        }
        if !self.query(nx, node.y, nz).safe {
            return;
        }
        if self.query(nx, node.y - 1, nz).physical {
            return;
        }
        let max_drop = if self.config.infinite_liquid_dropdown_distance {
            256
        } else {
            self.config.max_drop_down + 1
        };
        let mut dy = -2;
        while dy >= -max_drop {
            let landing = self.query(nx, node.y + dy, nz);
            if landing.physical {
                let land_y = node.y + dy + 1;
                if land_y != node.y && !self.query(nx, land_y, nz).safe {
                    return;
                }
                self.push(
                    out,
                    nx,
                    land_y,
                    nz,
                    1.0 + (node.y - land_y) as f64 * 0.5,
                    vec![],
                    vec![],
                    false,
                );
                return;
            }
            if landing.liquid {
                if landing.lava {
                    return;
                }
                self.push(
                    out,
                    nx,
                    node.y + dy,
                    nz,
                    1.0 + (node.y - (node.y + dy)) as f64 * 0.3,
                    vec![],
                    vec![],
                    false,
                );
                return;
            }
            if !landing.safe {
                return;
            }
            dy -= 1;
        }
    }

    fn move_diagonal(&self, node: &Move, dx: i32, dz: i32, out: &mut Vec<Move>) {
        let (nx, nz) = (node.x + dx, node.z + dz);
        let dest_floor = self.query(nx, node.y - 1, nz);
        if dest_floor.physical {
            if !self.query(nx, node.y, nz).safe || !self.query(nx, node.y + 1, nz).safe {
                return;
            }
            let p1 = self.query(node.x, node.y, node.z + dz).safe
                && self.query(node.x, node.y + 1, node.z + dz).safe;
            let p2 = self.query(node.x + dx, node.y, node.z).safe
                && self.query(node.x + dx, node.y + 1, node.z).safe;
            if !p1 && !p2 {
                return;
            }
            self.push(out, nx, node.y, nz, SQRT2, vec![], vec![], false);
            return;
        }
        if self.query(nx, node.y, nz).physical
            && self.query(nx, node.y + 1, nz).safe
            && self.query(nx, node.y + 2, nz).safe
            && self.query(node.x, node.y + 2, node.z).safe
        {
            self.push(out, nx, node.y + 1, nz, SQRT2 + 1.0, vec![], vec![], false);
        }
        if self.query(nx, node.y, nz).safe {
            let mut dy = -2;
            while dy >= -(self.config.max_drop_down + 1) {
                let landing = self.query(nx, node.y + dy, nz);
                if landing.physical {
                    let land_y = node.y + dy + 1;
                    self.push(
                        out,
                        nx,
                        land_y,
                        nz,
                        SQRT2 + (node.y - land_y) as f64 * 0.5,
                        vec![],
                        vec![],
                        false,
                    );
                    return;
                }
                if landing.liquid {
                    self.push(
                        out,
                        nx,
                        node.y + dy,
                        nz,
                        SQRT2 + (node.y - (node.y + dy)) as f64 * 0.3,
                        vec![],
                        vec![],
                        false,
                    );
                    return;
                }
                if !landing.safe {
                    return;
                }
                dy -= 1;
            }
        }
    }

    fn move_parkour(&self, node: &Move, dx: i32, dz: i32, out: &mut Vec<Move>) {
        if !self.config.allow_parkour {
            return;
        }
        let (nx1, nz1) = (node.x + dx, node.z + dz);
        if !self.query(node.x, node.y - 1, node.z).physical {
            return;
        }
        if self.query(nx1, node.y - 1, nz1).physical {
            return;
        }
        if !self.query(node.x, node.y + 2, node.z).safe {
            return;
        }
        let max_dist = if self.config.allow_sprinting { 4 } else { 2 };
        for dist in 2..=max_dist {
            let (nx, nz) = (node.x + dx * dist, node.z + dz * dist);
            if !self
                .query(
                    node.x + dx * (dist - 1),
                    node.y + 2,
                    node.z + dz * (dist - 1),
                )
                .safe
            {
                break;
            }
            if self.query(nx, node.y - 1, nz).physical
                && self.query(nx, node.y, nz).safe
                && self.query(nx, node.y + 1, nz).safe
            {
                self.push(out, nx, node.y, nz, dist as f64 + 1.0, vec![], vec![], true);
            }
            if dist <= 2
                && self.query(nx, node.y, nz).physical
                && self.query(nx, node.y + 1, nz).safe
                && self.query(nx, node.y + 2, nz).safe
            {
                self.push(
                    out,
                    nx,
                    node.y + 1,
                    nz,
                    dist as f64 + 2.0,
                    vec![],
                    vec![],
                    true,
                );
            }
            if self.query(nx, node.y - 2, nz).physical
                && self.query(nx, node.y - 1, nz).safe
                && self.query(nx, node.y, nz).safe
            {
                self.push(
                    out,
                    nx,
                    node.y - 1,
                    nz,
                    dist as f64 + 0.5,
                    vec![],
                    vec![],
                    true,
                );
            }
        }
    }

    fn move_up(&self, node: &Move, out: &mut Vec<Move>) {
        if !self.config.allow_1by1_towers || self.config.scaffolding_blocks.is_empty() {
            return;
        }
        if !self.query(node.x, node.y + 2, node.z).safe {
            return;
        }
        let current = self.query(node.x, node.y, node.z);
        if !current.safe && !current.replaceable {
            return;
        }
        self.push(
            out,
            node.x,
            node.y + 1,
            node.z,
            1.0 + self.config.place_cost,
            vec![],
            vec![PlaceAction {
                x: node.x,
                y: node.y,
                z: node.z,
                dx: 0,
                dy: -1,
                dz: 0,
                jump: true,
                return_pos: None,
            }],
            false,
        );
    }

    fn move_down(&self, node: &Move, out: &mut Vec<Move>) {
        if self.query(node.x, node.y - 1, node.z).physical {
            return;
        }
        let max_drop = if self.config.infinite_liquid_dropdown_distance {
            256
        } else {
            self.config.max_drop_down + 1
        };
        let mut dy = -1;
        while dy >= -max_drop {
            let landing = self.query(node.x, node.y + dy, node.z);
            if landing.physical {
                let land_y = node.y + dy + 1;
                self.push(
                    out,
                    node.x,
                    land_y,
                    node.z,
                    1.0 + (node.y - land_y) as f64 * 0.5,
                    vec![],
                    vec![],
                    false,
                );
                return;
            }
            if landing.liquid {
                if landing.lava {
                    return;
                }
                self.push(
                    out,
                    node.x,
                    node.y + dy,
                    node.z,
                    1.0 + (node.y - (node.y + dy)) as f64 * 0.3,
                    vec![],
                    vec![],
                    false,
                );
                return;
            }
            if !landing.safe {
                return;
            }
            dy -= 1;
        }
    }

    fn move_swim_up(&self, node: &Move, out: &mut Vec<Move>) {
        let current = self.query(node.x, node.y, node.z);
        if !current.liquid || current.lava {
            return;
        }
        let above = self.query(node.x, node.y + 1, node.z);
        if (above.safe || above.liquid) && !above.lava {
            self.push(out, node.x, node.y + 1, node.z, 1.0, vec![], vec![], false);
        }
    }
}

impl NeighborGen for Movements<'_> {
    fn neighbors(&self, node: &Move) -> Vec<Move> {
        let mut out = Vec::new();
        for (dx, dz) in CARDINAL {
            self.move_forward(node, dx, dz, &mut out);
            self.move_jump_up(node, dx, dz, &mut out);
            self.move_drop_down(node, dx, dz, &mut out);
            self.move_parkour(node, dx, dz, &mut out);
        }
        for (dx, dz) in DIAGONAL {
            self.move_diagonal(node, dx, dz, &mut out);
        }
        self.move_up(node, &mut out);
        self.move_down(node, &mut out);
        self.move_swim_up(node, &mut out);
        out
    }
}
