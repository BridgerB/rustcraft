//! World storage and spatial queries. Port of typecraft's `world` module.
//! (`raycast` lives in `raycast.rs`, which depends on `physics`.)

mod iterators;
mod raycast;
mod world;

pub use iterators::{
    manhattan_iterator, octahedron_iterator, raycast_iterator, spiral_iterator_2d, BlockFace,
    ManhattanIterator, OctahedronIterator, RaycastBlock, RaycastHit, RaycastIterator,
    SpiralIterator2d,
};
pub use raycast::{
    direction_from_yaw_pitch, raycast, RaycastResult, PLAYER_EYE_HEIGHT, PLAYER_SNEAK_EYE_HEIGHT,
};
pub use world::{ChunkProvider, World, WorldEvent};

use crate::anvil::AnvilWorld;
use crate::chunk::ChunkColumn;

/// Use an [`AnvilWorld`] as a [`ChunkProvider`].
impl ChunkProvider for AnvilWorld<'_> {
    fn load(&mut self, chunk_x: i32, chunk_z: i32) -> std::io::Result<Option<ChunkColumn>> {
        self.load_chunk(chunk_x, chunk_z)
    }

    fn save(&mut self, chunk_x: i32, chunk_z: i32, column: &ChunkColumn) -> std::io::Result<()> {
        self.save_chunk(chunk_x, chunk_z, column)
    }
}
