//! Minecraft player physics simulation. Port of typecraft's `physics/physics.ts`.

use std::collections::{HashMap, HashSet};

use crate::entity::Entity;
use crate::item::get_enchants;
use crate::registry::Registry;

use super::aabb::{compute_offset_x, compute_offset_y, compute_offset_z, Aabb};
use super::attribute::{AttributeModifier, AttributeValue};

/// A mutable 3-component vector used during simulation.
#[derive(Debug, Clone, Copy, Default)]
pub struct Mv3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PlayerControls {
    pub forward: bool,
    pub back: bool,
    pub left: bool,
    pub right: bool,
    pub jump: bool,
    pub sprint: bool,
    pub sneak: bool,
}

/// Block info as needed by the physics engine.
#[derive(Debug, Clone)]
pub struct PhysicsBlock {
    pub id: i32,
    pub name: String,
    pub state_id: u32,
    pub shapes: Vec<Vec<f64>>,
    /// "block" or "empty".
    pub bounding_box: String,
    pub properties: HashMap<String, String>,
}

/// World interface for physics — decoupled from the concrete `World`.
pub trait PhysicsWorld {
    /// Block at the given (floored) world coordinates.
    fn get_block(&self, x: f64, y: f64, z: f64) -> Option<PhysicsBlock>;
}

#[derive(Debug, Clone, Copy)]
pub struct BubbleDrag {
    pub down: f64,
    pub max_down: f64,
    pub up: f64,
    pub max_up: f64,
}

pub struct PhysicsConfig {
    pub gravity: f64,
    pub airdrag: f64,
    pub player_speed: f64,
    pub sprint_speed: f64,
    pub sneak_speed: f64,
    pub step_height: f64,
    pub negligeable_velocity: f64,
    pub soulsand_speed: f64,
    pub honeyblock_speed: f64,
    pub honeyblock_jump_speed: f64,
    pub ladder_max_speed: f64,
    pub ladder_climb_speed: f64,
    pub player_half_width: f64,
    pub player_height: f64,
    pub water_inertia: f64,
    pub lava_inertia: f64,
    pub liquid_acceleration: f64,
    pub airborne_inertia: f64,
    pub airborne_acceleration: f64,
    pub default_slipperiness: f64,
    pub out_of_liquid_impulse: f64,
    pub autojump_cooldown: i32,
    pub bubble_column_surface_drag: BubbleDrag,
    pub bubble_column_drag: BubbleDrag,
    pub slow_falling: f64,
    pub water_gravity: f64,
    pub lava_gravity: f64,
    pub movement_speed_attribute: String,
    pub sprinting_uuid: &'static str,
}

/// Mutable player state for one simulation tick.
pub struct PlayerState {
    pub pos: Mv3,
    pub vel: Mv3,
    pub on_ground: bool,
    pub is_in_water: bool,
    pub is_in_lava: bool,
    pub is_in_web: bool,
    pub is_collided_horizontally: bool,
    pub is_collided_vertically: bool,
    pub elytra_flying: bool,
    pub firework_rocket_duration: i32,
    pub riptide_ticks: i32,
    pub jump_ticks: i32,
    pub jump_queued: bool,
    pub yaw: f64,
    pub pitch: f64,
    pub control: PlayerControls,
    pub attributes: Option<HashMap<String, AttributeValue>>,
    pub jump_boost: i32,
    pub speed: i32,
    pub slowness: i32,
    pub dolphins_grace: i32,
    pub slow_falling: i32,
    pub levitation: i32,
    pub depth_strider: i32,
    pub soul_speed: i32,
    pub swift_sneak: i32,
    pub elytra_equipped: bool,
}

const SPRINTING_UUID: &str = "662a6b8d-da3e-4c1c-8813-96ea6097278d";

fn fround(x: f64) -> f64 {
    x as f32 as f64
}

fn clamp(min: f64, x: f64, max: f64) -> f64 {
    min.max(x.min(max))
}

fn normalize(v: &mut Mv3) {
    let len = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
    if len > 0.0 {
        v.x /= len;
        v.y /= len;
        v.z /= len;
    }
}

/// The physics engine for a specific Minecraft version.
pub struct PhysicsEngine {
    pub config: PhysicsConfig,
    slime_block_id: i32,
    soulsand_id: i32,
    honeyblock_id: i32,
    web_id: i32,
    ladder_id: i32,
    vine_id: i32,
    bubblecolumn_id: i32,
    powder_snow_id: i32,
    water_ids: HashSet<i32>,
    lava_ids: HashSet<i32>,
    water_like: HashSet<i32>,
    trapdoor_ids: HashSet<i32>,
    block_slipperiness: HashMap<i32, f64>,
    has_velocity_blocks_on_collision: bool,
    has_velocity_blocks_on_top: bool,
    has_climb_using_jump: bool,
    has_climbable_trapdoor: bool,
}

const FEATURE_VERSIONS: &[(&str, &[&str])] = &[
    (
        "independentLiquidGravity",
        &["1.8", "1.9", "1.10", "1.11", "1.12"],
    ),
    (
        "velocityBlocksOnCollision",
        &["1.8", "1.9", "1.10", "1.11", "1.12", "1.13", "1.14"],
    ),
    (
        "velocityBlocksOnTop",
        &["1.15", "1.16", "1.17", "1.18", "1.19", "1.20"],
    ),
    (
        "climbUsingJump",
        &["1.14", "1.15", "1.16", "1.17", "1.18", "1.19", "1.20"],
    ),
    (
        "climbableTrapdoor",
        &[
            "1.9", "1.10", "1.11", "1.12", "1.13", "1.14", "1.15", "1.16", "1.17", "1.18", "1.19",
            "1.20",
        ],
    ),
];

fn supports(feature: &str, major_version: &str) -> bool {
    FEATURE_VERSIONS
        .iter()
        .any(|(name, versions)| *name == feature && versions.contains(&major_version))
}

impl PhysicsEngine {
    pub fn new(registry: &Registry) -> PhysicsEngine {
        let major = registry.version.major_version.clone();
        let block_id = |name: &str| {
            registry
                .blocks_by_name
                .get(name)
                .map(|b| b.id)
                .unwrap_or(-1)
        };
        let ids_from = |names: &[&str]| -> HashSet<i32> {
            names
                .iter()
                .map(|n| block_id(n))
                .filter(|&id| id != -1)
                .collect()
        };

        let slime_block_id = if block_id("slime_block") != -1 {
            block_id("slime_block")
        } else {
            block_id("slime")
        };
        let web_id = if block_id("cobweb") != -1 {
            block_id("cobweb")
        } else {
            block_id("web")
        };
        let water_id = block_id("water");
        let flowing_water_id = block_id("flowing_water");
        let lava_id = block_id("lava");
        let flowing_lava_id = block_id("flowing_lava");

        let water_ids = [water_id, flowing_water_id]
            .into_iter()
            .filter(|&id| id != -1)
            .collect();
        let lava_ids = [lava_id, flowing_lava_id]
            .into_iter()
            .filter(|&id| id != -1)
            .collect();
        let water_like = ids_from(&[
            "seagrass",
            "tall_seagrass",
            "kelp",
            "kelp_plant",
            "bubble_column",
        ]);
        let trapdoor_ids = ids_from(&[
            "iron_trapdoor",
            "acacia_trapdoor",
            "birch_trapdoor",
            "jungle_trapdoor",
            "oak_trapdoor",
            "dark_oak_trapdoor",
            "spruce_trapdoor",
            "crimson_trapdoor",
            "warped_trapdoor",
            "mangrove_trapdoor",
            "cherry_trapdoor",
        ]);

        let mut block_slipperiness = HashMap::new();
        block_slipperiness.insert(slime_block_id, 0.8);
        block_slipperiness.insert(block_id("ice"), 0.98);
        block_slipperiness.insert(block_id("packed_ice"), 0.98);
        if block_id("frosted_ice") != -1 {
            block_slipperiness.insert(block_id("frosted_ice"), 0.98);
        }
        if block_id("blue_ice") != -1 {
            block_slipperiness.insert(block_id("blue_ice"), 0.989);
        }

        let independent_gravity = supports("independentLiquidGravity", &major);
        let gravity = 0.08;
        let movement_speed_attribute = registry
            .attributes_by_name
            .get("movementSpeed")
            .map(|a| a.resource.clone())
            .unwrap_or_else(|| "generic.movement_speed".to_string());

        let config = PhysicsConfig {
            gravity,
            airdrag: fround(1.0 - 0.02),
            player_speed: 0.1,
            sprint_speed: 0.3,
            sneak_speed: 0.3,
            step_height: 0.6,
            negligeable_velocity: 0.003,
            soulsand_speed: 0.4,
            honeyblock_speed: 0.4,
            honeyblock_jump_speed: 0.4,
            ladder_max_speed: 0.15,
            ladder_climb_speed: 0.2,
            player_half_width: 0.3,
            player_height: 1.8,
            water_inertia: 0.8,
            lava_inertia: 0.5,
            liquid_acceleration: 0.02,
            airborne_inertia: 0.91,
            airborne_acceleration: 0.02,
            default_slipperiness: 0.6,
            out_of_liquid_impulse: 0.3,
            autojump_cooldown: 10,
            bubble_column_surface_drag: BubbleDrag {
                down: 0.03,
                max_down: -0.9,
                up: 0.1,
                max_up: 1.8,
            },
            bubble_column_drag: BubbleDrag {
                down: 0.03,
                max_down: -0.3,
                up: 0.06,
                max_up: 0.7,
            },
            slow_falling: 0.125,
            water_gravity: if independent_gravity {
                0.02
            } else {
                gravity / 16.0
            },
            lava_gravity: if independent_gravity {
                0.02
            } else {
                gravity / 4.0
            },
            movement_speed_attribute,
            sprinting_uuid: SPRINTING_UUID,
        };

        PhysicsEngine {
            config,
            slime_block_id,
            soulsand_id: block_id("soul_sand"),
            honeyblock_id: block_id("honey_block"),
            web_id,
            ladder_id: block_id("ladder"),
            vine_id: block_id("vine"),
            bubblecolumn_id: block_id("bubble_column"),
            powder_snow_id: block_id("powder_snow"),
            water_ids,
            lava_ids,
            water_like,
            trapdoor_ids,
            block_slipperiness,
            has_velocity_blocks_on_collision: supports("velocityBlocksOnCollision", &major),
            has_velocity_blocks_on_top: supports("velocityBlocksOnTop", &major),
            has_climb_using_jump: supports("climbUsingJump", &major),
            has_climbable_trapdoor: supports("climbableTrapdoor", &major),
        }
    }

    fn player_bb(&self, pos: Mv3) -> Aabb {
        let w = self.config.player_half_width;
        Aabb::new(
            pos.x - w,
            pos.y,
            pos.z - w,
            pos.x + w,
            pos.y + self.config.player_height,
            pos.z + w,
        )
    }

    fn set_position_to_bb(&self, bb: Aabb, pos: &mut Mv3) {
        pos.x = bb.min_x + self.config.player_half_width;
        pos.y = bb.min_y;
        pos.z = bb.min_z + self.config.player_half_width;
    }

    fn surrounding_bbs(&self, world: &dyn PhysicsWorld, query: Aabb) -> Vec<Aabb> {
        let mut result = Vec::new();
        for y in (query.min_y.floor() as i32 - 1)..=(query.max_y.floor() as i32) {
            for z in (query.min_z.floor() as i32)..=(query.max_z.floor() as i32) {
                for x in (query.min_x.floor() as i32)..=(query.max_x.floor() as i32) {
                    if let Some(block) = world.get_block(x as f64, y as f64, z as f64) {
                        for shape in &block.shapes {
                            if shape.len() >= 6 {
                                let bb = Aabb::new(
                                    shape[0], shape[1], shape[2], shape[3], shape[4], shape[5],
                                )
                                .offset(x as f64, y as f64, z as f64);
                                result.push(bb);
                            }
                        }
                    }
                }
            }
        }
        result
    }

    fn move_entity(
        &self,
        entity: &mut PlayerState,
        world: &dyn PhysicsWorld,
        mut dx: f64,
        mut dy: f64,
        mut dz: f64,
    ) {
        if entity.is_in_web {
            dx *= 0.25;
            dy *= 0.05;
            dz *= 0.25;
            entity.vel.x = 0.0;
            entity.vel.y = 0.0;
            entity.vel.z = 0.0;
            entity.is_in_web = false;
        }

        let mut old_vel_x = dx;
        let old_vel_y = dy;
        let mut old_vel_z = dz;

        if entity.control.sneak && entity.on_ground {
            let step = 0.05;
            while dx != 0.0
                && self
                    .surrounding_bbs(world, self.player_bb(entity.pos).offset(dx, 0.0, 0.0))
                    .is_empty()
            {
                if (-step..step).contains(&dx) {
                    dx = 0.0;
                } else if dx > 0.0 {
                    dx -= step;
                } else {
                    dx += step;
                }
                old_vel_x = dx;
            }
            while dz != 0.0
                && self
                    .surrounding_bbs(world, self.player_bb(entity.pos).offset(0.0, 0.0, dz))
                    .is_empty()
            {
                if (-step..step).contains(&dz) {
                    dz = 0.0;
                } else if dz > 0.0 {
                    dz -= step;
                } else {
                    dz += step;
                }
                old_vel_z = dz;
            }
            while dx != 0.0
                && dz != 0.0
                && self
                    .surrounding_bbs(world, self.player_bb(entity.pos).offset(dx, 0.0, dz))
                    .is_empty()
            {
                if (-step..step).contains(&dx) {
                    dx = 0.0;
                } else if dx > 0.0 {
                    dx -= step;
                } else {
                    dx += step;
                }
                if (-step..step).contains(&dz) {
                    dz = 0.0;
                } else if dz > 0.0 {
                    dz -= step;
                } else {
                    dz += step;
                }
                old_vel_x = dx;
                old_vel_z = dz;
            }
        }

        let mut player_bb = self.player_bb(entity.pos);
        let query = player_bb.extend(dx, dy, dz);
        let surrounding = self.surrounding_bbs(world, query);
        let old_bb = player_bb;

        for &block_bb in &surrounding {
            dy = compute_offset_y(block_bb, player_bb, dy);
        }
        player_bb = player_bb.offset(0.0, dy, 0.0);
        for &block_bb in &surrounding {
            dx = compute_offset_x(block_bb, player_bb, dx);
        }
        player_bb = player_bb.offset(dx, 0.0, 0.0);
        for &block_bb in &surrounding {
            dz = compute_offset_z(block_bb, player_bb, dz);
        }
        player_bb = player_bb.offset(0.0, 0.0, dz);

        // Step assist.
        if self.config.step_height > 0.0
            && (entity.on_ground || (dy != old_vel_y && old_vel_y < 0.0))
            && (dx != old_vel_x || dz != old_vel_z)
        {
            let old_vel_x_col = dx;
            let old_vel_y_col = dy;
            let old_vel_z_col = dz;
            let old_bb_col = player_bb;

            dy = self.config.step_height;
            let step_query = old_bb.extend(old_vel_x, dy, old_vel_z);
            let step_bbs = self.surrounding_bbs(world, step_query);

            let mut bb1 = old_bb;
            let mut bb2 = old_bb;
            let bb_xz = bb1.extend(dx, 0.0, dz);

            let mut dy1 = dy;
            let mut dy2 = dy;
            for &block_bb in &step_bbs {
                dy1 = compute_offset_y(block_bb, bb_xz, dy1);
                dy2 = compute_offset_y(block_bb, bb2, dy2);
            }
            bb1 = bb1.offset(0.0, dy1, 0.0);
            bb2 = bb2.offset(0.0, dy2, 0.0);

            let mut dx1 = old_vel_x;
            let mut dx2 = old_vel_x;
            for &block_bb in &step_bbs {
                dx1 = compute_offset_x(block_bb, bb1, dx1);
                dx2 = compute_offset_x(block_bb, bb2, dx2);
            }
            bb1 = bb1.offset(dx1, 0.0, 0.0);
            bb2 = bb2.offset(dx2, 0.0, 0.0);

            let mut dz1 = old_vel_z;
            let mut dz2 = old_vel_z;
            for &block_bb in &step_bbs {
                dz1 = compute_offset_z(block_bb, bb1, dz1);
                dz2 = compute_offset_z(block_bb, bb2, dz2);
            }
            bb1 = bb1.offset(0.0, 0.0, dz1);
            bb2 = bb2.offset(0.0, 0.0, dz2);

            let norm1 = dx1 * dx1 + dz1 * dz1;
            let norm2 = dx2 * dx2 + dz2 * dz2;
            if norm1 > norm2 {
                dx = dx1;
                dy = -dy1;
                dz = dz1;
                player_bb = bb1;
            } else {
                dx = dx2;
                dy = -dy2;
                dz = dz2;
                player_bb = bb2;
            }
            for &block_bb in &step_bbs {
                dy = compute_offset_y(block_bb, player_bb, dy);
            }
            player_bb = player_bb.offset(0.0, dy, 0.0);

            if old_vel_x_col * old_vel_x_col + old_vel_z_col * old_vel_z_col >= dx * dx + dz * dz {
                dx = old_vel_x_col;
                dy = old_vel_y_col;
                dz = old_vel_z_col;
                player_bb = old_bb_col;
            }
        }

        self.set_position_to_bb(player_bb, &mut entity.pos);
        entity.is_collided_horizontally = dx != old_vel_x || dz != old_vel_z;
        entity.is_collided_vertically = dy != old_vel_y;
        entity.on_ground = entity.is_collided_vertically && old_vel_y < 0.0;

        let block_at_feet = world.get_block(entity.pos.x, entity.pos.y - 0.2, entity.pos.z);

        if dx != old_vel_x {
            entity.vel.x = 0.0;
        }
        if dz != old_vel_z {
            entity.vel.z = 0.0;
        }
        if dy != old_vel_y {
            if block_at_feet.as_ref().map(|b| b.id) == Some(self.slime_block_id)
                && !entity.control.sneak
            {
                entity.vel.y = -entity.vel.y;
            } else {
                entity.vel.y = 0.0;
            }
        }

        // Block-collision effects (web, soulsand, honey, bubble columns).
        let contracted = player_bb.contract(0.001, 0.001, 0.001);
        for cy in (contracted.min_y.floor() as i32)..=(contracted.max_y.floor() as i32) {
            for cz in (contracted.min_z.floor() as i32)..=(contracted.max_z.floor() as i32) {
                for cx in (contracted.min_x.floor() as i32)..=(contracted.max_x.floor() as i32) {
                    if let Some(block) = world.get_block(cx as f64, cy as f64, cz as f64) {
                        if self.has_velocity_blocks_on_collision {
                            if block.id == self.soulsand_id {
                                entity.vel.x *= self.config.soulsand_speed;
                                entity.vel.z *= self.config.soulsand_speed;
                            } else if block.id == self.honeyblock_id {
                                entity.vel.x *= self.config.honeyblock_speed;
                                entity.vel.z *= self.config.honeyblock_speed;
                            }
                        }
                        if block.id == self.web_id {
                            entity.is_in_web = true;
                        } else if block.id == self.bubblecolumn_id {
                            let down = block
                                .properties
                                .get("drag")
                                .map(|s| s == "true")
                                .unwrap_or(false);
                            let above = world.get_block(cx as f64, (cy + 1) as f64, cz as f64);
                            let drag = if above.as_ref().map(|b| b.name == "air").unwrap_or(false) {
                                self.config.bubble_column_surface_drag
                            } else {
                                self.config.bubble_column_drag
                            };
                            if down {
                                entity.vel.y = drag.max_down.max(entity.vel.y - drag.down);
                            } else {
                                entity.vel.y = drag.max_up.min(entity.vel.y + drag.up);
                            }
                        }
                    }
                }
            }
        }

        if self.has_velocity_blocks_on_top {
            let below = world.get_block(
                entity.pos.x.floor(),
                entity.pos.y.floor() - 1.0,
                entity.pos.z.floor(),
            );
            if let Some(below) = below {
                if below.id == self.soulsand_id {
                    if entity.soul_speed > 0 {
                        entity.vel.x *= 1.0 + entity.soul_speed as f64 * 0.105;
                        entity.vel.z *= 1.0 + entity.soul_speed as f64 * 0.105;
                    } else {
                        entity.vel.x *= self.config.soulsand_speed;
                        entity.vel.z *= self.config.soulsand_speed;
                    }
                } else if below.id == self.honeyblock_id {
                    entity.vel.x *= self.config.honeyblock_speed;
                    entity.vel.z *= self.config.honeyblock_speed;
                }
            }
        }

        if self.powder_snow_id != -1 {
            let feet = world.get_block(
                entity.pos.x.floor(),
                entity.pos.y.floor(),
                entity.pos.z.floor(),
            );
            if feet.map(|b| b.id) == Some(self.powder_snow_id) {
                entity.vel.x *= 0.9;
                entity.vel.z *= 0.9;
            }
        }
    }

    fn look_dir(&self, entity: &PlayerState) -> Mv3 {
        let sin_yaw = entity.yaw.sin();
        let cos_yaw = entity.yaw.cos();
        let sin_pitch = entity.pitch.sin();
        let cos_pitch = entity.pitch.cos();
        Mv3 {
            x: -sin_yaw * cos_pitch,
            y: sin_pitch,
            z: -cos_yaw * cos_pitch,
        }
    }

    fn apply_heading(
        &self,
        entity: &mut PlayerState,
        mut strafe: f64,
        mut forward: f64,
        multiplier: f64,
    ) {
        let mut speed = (strafe * strafe + forward * forward).sqrt();
        if speed < 0.01 {
            return;
        }
        speed = multiplier / speed.max(1.0);
        strafe *= speed;
        forward *= speed;
        let yaw = std::f64::consts::PI - entity.yaw;
        let sin = yaw.sin();
        let cos = yaw.cos();
        entity.vel.x -= strafe * cos + forward * sin;
        entity.vel.z += forward * cos - strafe * sin;
    }

    fn is_on_ladder(&self, world: &dyn PhysicsWorld, pos: Mv3) -> bool {
        let Some(block) = world.get_block(pos.x, pos.y, pos.z) else {
            return false;
        };
        if block.id == self.ladder_id || block.id == self.vine_id {
            return true;
        }
        if !self.has_climbable_trapdoor || !self.trapdoor_ids.contains(&block.id) {
            return false;
        }
        let below = world.get_block(pos.x, pos.y - 1.0, pos.z);
        below.as_ref().map(|b| b.id) == Some(self.ladder_id)
            && block
                .properties
                .get("open")
                .map(|s| s == "true")
                .unwrap_or(false)
            && block.properties.get("facing")
                == below.as_ref().and_then(|b| b.properties.get("facing"))
    }

    fn does_not_collide(&self, world: &dyn PhysicsWorld, pos: Mv3) -> bool {
        let pbb = self.player_bb(pos);
        !self
            .surrounding_bbs(world, pbb)
            .iter()
            .any(|&x| pbb.intersects(x))
            && self.water_in_bb(world, pbb).is_empty()
    }

    fn rendered_depth(&self, block: Option<&PhysicsBlock>) -> i32 {
        let Some(block) = block else { return -1 };
        if self.water_like.contains(&block.id) {
            return 0;
        }
        if block
            .properties
            .get("waterlogged")
            .map(|s| s == "true")
            .unwrap_or(false)
        {
            return 0;
        }
        if !self.water_ids.contains(&block.id) {
            return -1;
        }
        let level: i32 = block
            .properties
            .get("level")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if level >= 8 {
            0
        } else {
            level
        }
    }

    fn liquid_height_pcent(&self, block: &PhysicsBlock) -> f64 {
        (self.rendered_depth(Some(block)) + 1) as f64 / 9.0
    }

    fn water_in_bb(&self, world: &dyn PhysicsWorld, bb: Aabb) -> Vec<PhysicsBlock> {
        let mut blocks = Vec::new();
        for cy in (bb.min_y.floor() as i32)..=(bb.max_y.floor() as i32) {
            for cz in (bb.min_z.floor() as i32)..=(bb.max_z.floor() as i32) {
                for cx in (bb.min_x.floor() as i32)..=(bb.max_x.floor() as i32) {
                    if let Some(block) = world.get_block(cx as f64, cy as f64, cz as f64) {
                        let is_water = self.water_ids.contains(&block.id)
                            || self.water_like.contains(&block.id)
                            || block
                                .properties
                                .get("waterlogged")
                                .map(|s| s == "true")
                                .unwrap_or(false);
                        if is_water {
                            let water_level = cy as f64 + 1.0 - self.liquid_height_pcent(&block);
                            if bb.max_y.ceil() >= water_level {
                                blocks.push(block);
                            }
                        }
                    }
                }
            }
        }
        blocks
    }

    fn get_flow(
        &self,
        world: &dyn PhysicsWorld,
        block: &PhysicsBlock,
        bx: i32,
        by: i32,
        bz: i32,
    ) -> Mv3 {
        let curlevel = self.rendered_depth(Some(block));
        let mut flow = Mv3::default();
        const DIRS: [(i32, i32); 4] = [(0, 1), (-1, 0), (0, -1), (1, 0)];
        for (dx, dz) in DIRS {
            let adj = world.get_block((bx + dx) as f64, by as f64, (bz + dz) as f64);
            let adj_level = self.rendered_depth(adj.as_ref());
            if adj_level < 0 {
                if adj
                    .as_ref()
                    .map(|b| b.bounding_box != "empty")
                    .unwrap_or(false)
                {
                    let below =
                        world.get_block((bx + dx) as f64, (by - 1) as f64, (bz + dz) as f64);
                    let below_level = self.rendered_depth(below.as_ref());
                    if below_level >= 0 {
                        let f = (below_level - (curlevel - 8)) as f64;
                        flow.x += dx as f64 * f;
                        flow.z += dz as f64 * f;
                    }
                }
            } else {
                let f = (adj_level - curlevel) as f64;
                flow.x += dx as f64 * f;
                flow.z += dz as f64 * f;
            }
        }
        let level: i32 = block
            .properties
            .get("level")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if level >= 8 {
            for (dx, dz) in DIRS {
                let adj = world.get_block((bx + dx) as f64, by as f64, (bz + dz) as f64);
                let adj_up = world.get_block((bx + dx) as f64, (by + 1) as f64, (bz + dz) as f64);
                if adj.map(|b| b.bounding_box != "empty").unwrap_or(false)
                    || adj_up.map(|b| b.bounding_box != "empty").unwrap_or(false)
                {
                    normalize(&mut flow);
                    flow.y -= 6.0;
                    break;
                }
            }
        }
        normalize(&mut flow);
        flow
    }

    fn is_in_water_apply_current(&self, world: &dyn PhysicsWorld, bb: Aabb, vel: &mut Mv3) -> bool {
        let mut acceleration = Mv3::default();
        let water_blocks = self.water_in_bb(world, bb);
        let is_in_water = !water_blocks.is_empty();
        for block in &water_blocks {
            let flow = self.get_flow(
                world,
                block,
                bb.min_x.floor() as i32,
                bb.min_y.floor() as i32,
                bb.min_z.floor() as i32,
            );
            acceleration.x += flow.x;
            acceleration.y += flow.y;
            acceleration.z += flow.z;
        }
        normalize(&mut acceleration);
        vel.x += acceleration.x * 0.014;
        vel.y += acceleration.y * 0.014;
        vel.z += acceleration.z * 0.014;
        is_in_water
    }

    fn is_material_in_bb(
        &self,
        world: &dyn PhysicsWorld,
        query: Aabb,
        types: &HashSet<i32>,
    ) -> bool {
        for cy in (query.min_y.floor() as i32)..=(query.max_y.floor() as i32) {
            for cz in (query.min_z.floor() as i32)..=(query.max_z.floor() as i32) {
                for cx in (query.min_x.floor() as i32)..=(query.max_x.floor() as i32) {
                    if let Some(block) = world.get_block(cx as f64, cy as f64, cz as f64) {
                        if types.contains(&block.id) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    fn move_with_heading(
        &self,
        entity: &mut PlayerState,
        world: &dyn PhysicsWorld,
        strafe: f64,
        forward: f64,
    ) {
        let gravity_multiplier = if entity.vel.y <= 0.0 && entity.slow_falling > 0 {
            self.config.slow_falling
        } else {
            1.0
        };

        if entity.is_in_water || entity.is_in_lava {
            let last_y = entity.pos.y;
            let mut acceleration = self.config.liquid_acceleration;
            let inertia = if entity.is_in_water {
                self.config.water_inertia
            } else {
                self.config.lava_inertia
            };
            let mut horizontal_inertia = inertia;

            if entity.is_in_water {
                let mut strider = entity.depth_strider.min(3) as f64;
                if !entity.on_ground {
                    strider *= 0.5;
                }
                if strider > 0.0 {
                    horizontal_inertia += (0.546 - horizontal_inertia) * strider / 3.0;
                    acceleration += (0.7 - acceleration) * strider / 3.0;
                }
                if entity.dolphins_grace > 0 {
                    horizontal_inertia = 0.96;
                }
            }

            self.apply_heading(entity, strafe, forward, acceleration);
            self.move_entity(entity, world, entity.vel.x, entity.vel.y, entity.vel.z);
            entity.vel.y *= inertia;
            entity.vel.y -= (if entity.is_in_water {
                self.config.water_gravity
            } else {
                self.config.lava_gravity
            }) * gravity_multiplier;
            entity.vel.x *= horizontal_inertia;
            entity.vel.z *= horizontal_inertia;

            if entity.is_collided_horizontally
                && self.does_not_collide(
                    world,
                    Mv3 {
                        x: entity.vel.x,
                        y: entity.vel.y + 0.6 - entity.pos.y + last_y,
                        z: entity.vel.z,
                    },
                )
            {
                entity.vel.y = self.config.out_of_liquid_impulse;
            }
        } else if entity.elytra_flying {
            let look = self.look_dir(entity);
            let pitch = entity.pitch;
            let cos_pitch = entity.pitch.cos();
            let sin_pitch = entity.pitch.sin();
            let horizontal_speed =
                (entity.vel.x * entity.vel.x + entity.vel.z * entity.vel.z).sqrt();
            let cos_pitch_sq = cos_pitch * cos_pitch;
            entity.vel.y += self.config.gravity * gravity_multiplier * (-1.0 + cos_pitch_sq * 0.75);

            if entity.vel.y < 0.0 && cos_pitch > 0.0 {
                let m = entity.vel.y * -0.1 * cos_pitch_sq;
                entity.vel.x += look.x * m / cos_pitch;
                entity.vel.y += m;
                entity.vel.z += look.z * m / cos_pitch;
            }
            if pitch < 0.0 && cos_pitch > 0.0 {
                let m = horizontal_speed * -sin_pitch * 0.04;
                entity.vel.x += -look.x * m / cos_pitch;
                entity.vel.y += m * 3.2;
                entity.vel.z += -look.z * m / cos_pitch;
            }
            if cos_pitch > 0.0 {
                entity.vel.x += (look.x / cos_pitch * horizontal_speed - entity.vel.x) * 0.1;
                entity.vel.z += (look.z / cos_pitch * horizontal_speed - entity.vel.z) * 0.1;
            }
            entity.vel.x *= 0.99;
            entity.vel.y *= 0.98;
            entity.vel.z *= 0.99;
            self.move_entity(entity, world, entity.vel.x, entity.vel.y, entity.vel.z);
            if entity.on_ground {
                entity.elytra_flying = false;
            }
        } else {
            let mut acceleration;
            let inertia;
            let block_under = world.get_block(entity.pos.x, entity.pos.y - 1.0, entity.pos.z);
            if entity.on_ground && block_under.is_some() {
                let block_under = block_under.unwrap();
                let mut speed_attr = entity
                    .attributes
                    .as_ref()
                    .and_then(|a| a.get(&self.config.movement_speed_attribute).cloned())
                    .unwrap_or_else(|| AttributeValue::new(self.config.player_speed));
                speed_attr = speed_attr.without_modifier(self.config.sprinting_uuid);
                if entity.control.sprint && !speed_attr.has_modifier(self.config.sprinting_uuid) {
                    speed_attr = speed_attr.with_modifier(AttributeModifier {
                        uuid: SPRINTING_UUID,
                        amount: self.config.sprint_speed,
                        operation: 2,
                    });
                }
                let attribute_speed = speed_attr.compute();
                inertia = self
                    .block_slipperiness
                    .get(&block_under.id)
                    .copied()
                    .unwrap_or(self.config.default_slipperiness)
                    * 0.91;
                acceleration = attribute_speed * (0.1627714 / (inertia * inertia * inertia));
                if acceleration < 0.0 {
                    acceleration = 0.0;
                }
            } else {
                acceleration = self.config.airborne_acceleration;
                inertia = self.config.airborne_inertia;
                if entity.control.sprint {
                    acceleration += self.config.airborne_acceleration * 0.3;
                }
            }

            self.apply_heading(entity, strafe, forward, acceleration);

            if self.is_on_ladder(world, entity.pos) {
                entity.vel.x = clamp(
                    -self.config.ladder_max_speed,
                    entity.vel.x,
                    self.config.ladder_max_speed,
                );
                entity.vel.z = clamp(
                    -self.config.ladder_max_speed,
                    entity.vel.z,
                    self.config.ladder_max_speed,
                );
                entity.vel.y = entity.vel.y.max(if entity.control.sneak {
                    0.0
                } else {
                    -self.config.ladder_max_speed
                });
            }

            self.move_entity(entity, world, entity.vel.x, entity.vel.y, entity.vel.z);

            if self.is_on_ladder(world, entity.pos)
                && (entity.is_collided_horizontally
                    || (self.has_climb_using_jump && entity.control.jump))
            {
                entity.vel.y = self.config.ladder_climb_speed;
            }

            if entity.levitation > 0 {
                entity.vel.y += (0.05 * entity.levitation as f64 - entity.vel.y) * 0.2;
            } else {
                entity.vel.y -= self.config.gravity * gravity_multiplier;
            }
            entity.vel.y *= self.config.airdrag;
            entity.vel.x *= inertia;
            entity.vel.z *= inertia;
        }
    }

    /// Simulate one physics tick.
    pub fn simulate_player(&self, state: &mut PlayerState, world: &dyn PhysicsWorld) {
        let water_bb = self.player_bb(state.pos).contract(0.001, 0.401, 0.001);
        let lava_bb = self.player_bb(state.pos).contract(0.1, 0.4, 0.1);

        state.is_in_water = self.is_in_water_apply_current(world, water_bb, &mut state.vel);
        state.is_in_lava = self.is_material_in_bb(world, lava_bb, &self.lava_ids);

        if state.vel.x.abs() < self.config.negligeable_velocity {
            state.vel.x = 0.0;
        }
        if state.vel.y.abs() < self.config.negligeable_velocity {
            state.vel.y = 0.0;
        }
        if state.vel.z.abs() < self.config.negligeable_velocity {
            state.vel.z = 0.0;
        }

        if state.control.jump || state.jump_queued {
            if state.jump_ticks > 0 {
                state.jump_ticks -= 1;
            }
            if state.is_in_water || state.is_in_lava {
                state.vel.y += 0.04;
            } else if state.on_ground && state.jump_ticks == 0 {
                let below = world.get_block(
                    state.pos.x.floor(),
                    state.pos.y.floor() - 1.0,
                    state.pos.z.floor(),
                );
                state.vel.y = fround(0.42)
                    * if below.map(|b| b.id) == Some(self.honeyblock_id) {
                        self.config.honeyblock_jump_speed
                    } else {
                        1.0
                    };
                if state.jump_boost > 0 {
                    state.vel.y += 0.1 * state.jump_boost as f64;
                }
                if state.control.sprint {
                    let yaw = std::f64::consts::PI - state.yaw;
                    state.vel.x -= yaw.sin() * 0.2;
                    state.vel.z += yaw.cos() * 0.2;
                }
                state.jump_ticks = self.config.autojump_cooldown;
            }
        } else {
            state.jump_ticks = 0;
        }
        state.jump_queued = false;

        let mut strafe = (state.control.right as i32 - state.control.left as i32) as f64 * 0.98;
        let mut forward = (state.control.forward as i32 - state.control.back as i32) as f64 * 0.98;

        if state.control.sneak {
            let sneak_mult = if state.swift_sneak > 0 {
                (0.3 + state.swift_sneak as f64 * 0.15).min(1.0)
            } else {
                self.config.sneak_speed
            };
            strafe *= sneak_mult;
            forward *= sneak_mult;
        }

        state.elytra_flying = state.elytra_flying
            && state.elytra_equipped
            && !state.on_ground
            && state.levitation == 0;

        if state.riptide_ticks > 0 {
            let look = self.look_dir(state);
            let speed = state.riptide_ticks as f64;
            state.vel.x += look.x * speed;
            state.vel.y += look.y * speed;
            state.vel.z += look.z * speed;
            state.riptide_ticks = 0;
        }

        if state.firework_rocket_duration > 0 {
            if !state.elytra_flying {
                state.firework_rocket_duration = 0;
            } else {
                let look = self.look_dir(state);
                state.vel.x += look.x * 0.1 + (look.x * 1.5 - state.vel.x) * 0.5;
                state.vel.y += look.y * 0.1 + (look.y * 1.5 - state.vel.y) * 0.5;
                state.vel.z += look.z * 0.1 + (look.z * 1.5 - state.vel.z) * 0.5;
                state.firework_rocket_duration -= 1;
            }
        }

        self.move_with_heading(state, world, strafe, forward);
    }

    /// Snap a position down to the surface below (used after teleports).
    pub fn adjust_position_height(&self, pos: &mut Mv3, world: &dyn PhysicsWorld) {
        let player_bb = self.player_bb(*pos);
        let query = player_bb.extend(0.0, -1.0, 0.0);
        let surrounding = self.surrounding_bbs(world, query);
        let mut dy = -1.0;
        for &block_bb in &surrounding {
            dy = compute_offset_y(block_bb, player_bb, dy);
        }
        pos.y += dy;
    }
}

/// Build a `PlayerState` from an entity for simulation.
pub fn create_player_state(
    registry: &Registry,
    entity: &Entity,
    controls: PlayerControls,
) -> PlayerState {
    let effect_level = |name: &str| -> i32 {
        let Some(def) = registry.effects_by_name.get(name) else {
            return 0;
        };
        entity
            .effects
            .get(&def.id)
            .map(|e| e.amplifier + 1)
            .unwrap_or(0)
    };

    let mut depth_strider = 0;
    let mut soul_speed = 0;
    let mut swift_sneak = 0;
    if let Some(boots) = entity.equipment.get(2).and_then(|i| i.as_ref()) {
        for e in get_enchants(registry, boots) {
            if e.name == "depth_strider" {
                depth_strider = e.level;
            }
            if e.name == "soul_speed" {
                soul_speed = e.level;
            }
        }
    }
    if let Some(leggings) = entity.equipment.get(3).and_then(|i| i.as_ref()) {
        for e in get_enchants(registry, leggings) {
            if e.name == "swift_sneak" {
                swift_sneak = e.level;
            }
        }
    }
    let elytra_equipped = entity
        .equipment
        .get(4)
        .and_then(|i| i.as_ref())
        .map(|i| i.name == "elytra")
        .unwrap_or(false);

    PlayerState {
        pos: Mv3 {
            x: entity.position.x,
            y: entity.position.y,
            z: entity.position.z,
        },
        vel: Mv3 {
            x: entity.velocity.x,
            y: entity.velocity.y,
            z: entity.velocity.z,
        },
        on_ground: entity.on_ground,
        is_in_water: false,
        is_in_lava: false,
        is_in_web: false,
        is_collided_horizontally: false,
        is_collided_vertically: false,
        elytra_flying: entity.elytra_flying,
        firework_rocket_duration: 0,
        riptide_ticks: 0,
        jump_ticks: 0,
        jump_queued: false,
        yaw: entity.yaw,
        pitch: entity.pitch,
        control: controls,
        attributes: None,
        jump_boost: effect_level("JumpBoost"),
        speed: effect_level("Speed"),
        slowness: effect_level("Slowness"),
        dolphins_grace: effect_level("DolphinsGrace"),
        slow_falling: effect_level("SlowFalling"),
        levitation: effect_level("Levitation"),
        depth_strider,
        soul_speed,
        swift_sneak,
        elytra_equipped,
    }
}

/// Apply simulation results back to an entity.
pub fn apply_player_state(state: &PlayerState, entity: &mut Entity) {
    entity.position = crate::vec3::vec3(state.pos.x, state.pos.y, state.pos.z);
    entity.velocity = crate::vec3::vec3(state.vel.x, state.vel.y, state.vel.z);
    entity.on_ground = state.on_ground;
    entity.elytra_flying = state.elytra_flying;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{BlockCollisionShapes, Registry};

    struct FlatWorld {
        floor_y: i32,
    }
    impl PhysicsWorld for FlatWorld {
        fn get_block(&self, _x: f64, y: f64, _z: f64) -> Option<PhysicsBlock> {
            if (y.floor() as i32) <= self.floor_y {
                Some(PhysicsBlock {
                    id: 1,
                    name: "stone".into(),
                    state_id: 1,
                    shapes: vec![vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0]],
                    bounding_box: "block".into(),
                    properties: HashMap::new(),
                })
            } else {
                None
            }
        }
    }

    fn registry() -> Registry {
        Registry::build(
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            BlockCollisionShapes::default(),
            std::collections::HashMap::new(),
            "26.1.2",
        )
    }

    fn state_at(y: f64) -> PlayerState {
        PlayerState {
            pos: Mv3 { x: 0.5, y, z: 0.5 },
            vel: Mv3::default(),
            on_ground: false,
            is_in_water: false,
            is_in_lava: false,
            is_in_web: false,
            is_collided_horizontally: false,
            is_collided_vertically: false,
            elytra_flying: false,
            firework_rocket_duration: 0,
            riptide_ticks: 0,
            jump_ticks: 0,
            jump_queued: false,
            yaw: 0.0,
            pitch: 0.0,
            control: PlayerControls::default(),
            attributes: None,
            jump_boost: 0,
            speed: 0,
            slowness: 0,
            dolphins_grace: 0,
            slow_falling: 0,
            levitation: 0,
            depth_strider: 0,
            soul_speed: 0,
            swift_sneak: 0,
            elytra_equipped: false,
        }
    }

    #[test]
    fn gravity_pulls_down() {
        let engine = PhysicsEngine::new(&registry());
        let world = FlatWorld { floor_y: -1 }; // no floor near the player
        let mut state = state_at(100.0);
        engine.simulate_player(&mut state, &world);
        // Gravity is applied after the move, so velocity is downward now and
        // position drops on the following tick.
        assert!(state.vel.y < 0.0, "gravity should give downward velocity");
        engine.simulate_player(&mut state, &world);
        assert!(state.pos.y < 100.0);
    }

    #[test]
    fn lands_on_floor() {
        let engine = PhysicsEngine::new(&registry());
        let world = FlatWorld { floor_y: 63 }; // solid up to y=63, top at y=64
        let mut state = state_at(64.05);
        for _ in 0..40 {
            engine.simulate_player(&mut state, &world);
        }
        assert!(state.on_ground, "player should land on the floor");
        assert!(
            (state.pos.y - 64.0).abs() < 0.1,
            "should rest at y≈64, got {}",
            state.pos.y
        );
    }
}
