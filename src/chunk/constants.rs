//! Chunk geometry and palette bit-width constants.

pub const SECTION_WIDTH: usize = 16;
pub const SECTION_HEIGHT: usize = 16;
pub const BLOCK_SECTION_VOLUME: usize = SECTION_WIDTH * SECTION_WIDTH * SECTION_HEIGHT;
pub const BIOME_SECTION_VOLUME: usize = BLOCK_SECTION_VOLUME / (4 * 4 * 4); // 64

pub const MIN_BITS_PER_BLOCK: u32 = 4;
pub const MAX_BITS_PER_BLOCK: u32 = 8;
pub const GLOBAL_BITS_PER_BLOCK: u32 = 16;

pub const MIN_BITS_PER_BIOME: u32 = 1;
pub const MAX_BITS_PER_BIOME: u32 = 3;
pub const GLOBAL_BITS_PER_BIOME: u32 = 6;
