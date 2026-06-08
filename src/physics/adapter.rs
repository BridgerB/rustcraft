//! Bridge from the concrete `World` to the physics `PhysicsWorld` interface,
//! resolving collision shapes per block access.

use crate::block::state_id_to_block;
use crate::world::World;

use super::physics::{PhysicsBlock, PhysicsWorld};

/// A view over a [`World`] that resolves collision shapes for physics.
pub struct WorldPhysics<'a> {
    world: &'a World<'a>,
}

impl<'a> WorldPhysics<'a> {
    pub fn new(world: &'a World<'a>) -> WorldPhysics<'a> {
        WorldPhysics { world }
    }
}

impl PhysicsWorld for WorldPhysics<'_> {
    fn get_block(&self, x: f64, y: f64, z: f64) -> Option<PhysicsBlock> {
        let registry = self.world.registry;
        let state_id =
            self.world
                .get_block_state_id(crate::vec3::vec3(x.floor(), y.floor(), z.floor()))?;
        let def = registry.blocks_by_state_id.get(&state_id)?;

        let shapes = resolve_shapes(registry, &def.name, state_id, def.min_state_id);
        let info = state_id_to_block(registry, state_id);
        Some(PhysicsBlock {
            id: def.id,
            name: def.name.clone(),
            state_id,
            shapes,
            bounding_box: def.bounding_box.clone(),
            properties: info.properties,
        })
    }
}

fn resolve_shapes(
    registry: &crate::registry::Registry,
    name: &str,
    state_id: u32,
    min_state_id: u32,
) -> Vec<Vec<f64>> {
    let shapes = &registry.block_collision_shapes;
    let Some(shape_ref) = shapes.blocks.get(name) else {
        return Vec::new();
    };
    // shape_ref is a number (one shape for all states) or an array per state.
    let shape_id = if let Some(n) = shape_ref.as_u64() {
        n
    } else if let Some(arr) = shape_ref.as_array() {
        arr.get((state_id - min_state_id) as usize)
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
    } else {
        0
    };
    shapes
        .shapes
        .get(&shape_id.to_string())
        .cloned()
        .unwrap_or_default()
}
