//! High-level raycast — steps through blocks along a ray, tests collision
//! shapes, returns the first solid block hit. Port of typecraft's
//! `world/raycast.ts`.

use crate::physics::{PhysicsWorld, WorldPhysics};
use crate::vec3::{vec3, Vec3};

use super::iterators::{raycast_iterator, BlockFace};
use super::world::World;

#[derive(Debug, Clone)]
pub struct RaycastResult {
    pub position: Vec3,
    pub face: BlockFace,
    pub intersect: Vec3,
    pub name: String,
    pub state_id: u32,
}

/// Cast a ray through the world, returning the first solid block hit.
pub fn raycast(
    world: &World,
    from: Vec3,
    direction: Vec3,
    max_distance: f64,
    match_fn: Option<&dyn Fn(&str) -> bool>,
) -> Option<RaycastResult> {
    let physics = WorldPhysics::new(world);
    let mut iter = raycast_iterator(from, direction, max_distance);

    while let Some(block) = iter.next() {
        let pos = vec3(block.x as f64, block.y as f64, block.z as f64);
        if let Some(p_block) = physics.get_block(pos.x, pos.y, pos.z) {
            if p_block.bounding_box == "block" && match_fn.map(|f| f(&p_block.name)).unwrap_or(true)
            {
                if !p_block.shapes.is_empty() {
                    if let Some(hit) = iter.intersect(&p_block.shapes, pos) {
                        return Some(RaycastResult {
                            position: pos,
                            face: hit.face,
                            intersect: hit.pos,
                            name: p_block.name,
                            state_id: p_block.state_id,
                        });
                    }
                }
            }
        }
    }
    None
}

/// Direction vector from yaw and pitch (radians).
pub fn direction_from_yaw_pitch(yaw: f64, pitch: f64) -> Vec3 {
    vec3(
        -yaw.sin() * pitch.cos(),
        -pitch.sin(),
        yaw.cos() * pitch.cos(),
    )
}

/// Standard eye height for a standing player.
pub const PLAYER_EYE_HEIGHT: f64 = 1.62;
/// Standard eye height for a sneaking player.
pub const PLAYER_SNEAK_EYE_HEIGHT: f64 = 1.27;
