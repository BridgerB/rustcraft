//! Player physics simulation. Port of typecraft's `physics` module.

mod aabb;
mod adapter;
mod attribute;
mod physics;

pub use aabb::{compute_offset_x, compute_offset_y, compute_offset_z, Aabb};
pub use adapter::WorldPhysics;
pub use attribute::{AttributeModifier, AttributeValue};
pub use physics::{
    apply_player_state, create_player_state, BubbleDrag, Mv3, PhysicsBlock, PhysicsConfig,
    PhysicsEngine, PhysicsWorld, PlayerControls, PlayerState,
};
