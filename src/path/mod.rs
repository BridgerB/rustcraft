//! A* pathfinding. Port of typecraft's `path` module (search + movements +
//! goals). The async bot-driving `goto` lives in the bot layer.

mod astar;
mod goals;
mod movements;
mod types;

pub use astar::{compute_path, AStar, NeighborGen};
pub use goals::{
    GoalBlock, GoalCompositeAll, GoalCompositeAny, GoalGetToBlock, GoalInvert, GoalLookAtBlock,
    GoalNear, GoalNearXZ, GoalXZ, GoalY,
};
pub use movements::Movements;
pub use types::{
    pos_hash, BlockQuery, Goal, Move, MovementsConfig, PathResult, PathStatus, PlaceAction,
};

use std::time::Duration;

use crate::world::World;

/// Find a path from `(start_x, start_y, start_z)` to a goal through the world.
pub fn get_path_to(
    world: &World,
    start: (i32, i32, i32),
    goal: &dyn Goal,
    config: MovementsConfig,
    search_radius: f64,
    timeout: Duration,
) -> PathResult {
    let movements = Movements::new(world, config);
    compute_path(
        Move::start(start.0, start.1, start.2),
        goal,
        &movements,
        search_radius,
        timeout,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::{ChunkColumn, ChunkColumnOptions};
    use crate::registry::{BlockCollisionShapes, BlockDefinition, Registry};
    use crate::vec3::vec3;

    fn solid_block(name: &str, id: i32, state: u32) -> BlockDefinition {
        BlockDefinition {
            id,
            name: name.into(),
            display_name: name.into(),
            hardness: Some(1.0),
            resistance: Some(1.0),
            stack_size: 64,
            diggable: true,
            bounding_box: "block".into(),
            material: None,
            transparent: false,
            emit_light: 0,
            filter_light: 15,
            default_state: state,
            min_state_id: state,
            max_state_id: state,
            states: vec![],
            drops: vec![],
            harvest_tools: None,
        }
    }

    fn registry() -> Registry {
        // Stone has a full-cube collision shape (shape 0 = unit cube); air has none.
        let mut shapes = BlockCollisionShapes::default();
        shapes
            .shapes
            .insert("0".into(), vec![vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0]]);
        shapes.blocks.insert("stone".into(), serde_json::json!(0));
        Registry::build(
            vec![solid_block("air", 0, 0), solid_block("stone", 1, 1)],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            shapes,
            std::collections::HashMap::new(),
            "26.1.2",
        )
    }

    #[test]
    fn walks_across_a_floor() {
        let reg = registry();
        let mut world = World::new(&reg);
        // Build a flat stone floor at y=63 across chunk (0,0), air above.
        let mut col = ChunkColumn::new(ChunkColumnOptions {
            min_y: None,
            world_height: None,
            max_bits_per_block: 15,
            max_bits_per_biome: 7,
        });
        for x in 0..16 {
            for z in 0..16 {
                col.set_block_state_id(x, 63, z, 1); // stone floor
            }
        }
        world.set_column(0, 0, col);
        world.take_events();

        // air definition has no collision shape → not physical; stone is physical.
        let goal = GoalBlock::new(8.0, 64.0, 8.0);
        let result = get_path_to(
            &world,
            (1, 64, 1),
            &goal,
            MovementsConfig::default(),
            -1.0,
            Duration::from_secs(5),
        );
        assert_eq!(
            result.status,
            PathStatus::Success,
            "should find a path on the floor"
        );
        let last = result.path.last().unwrap();
        assert_eq!((last.x, last.y, last.z), (8, 64, 8));
        // sanity: world block lookups work
        assert_eq!(world.get_block_state_id(vec3(5.0, 63.0, 5.0)), Some(1));
    }
}
