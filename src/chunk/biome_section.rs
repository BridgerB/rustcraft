//! A 4×4×4 biome section (64 cells), backed by a palette container.

use super::bit_array::BitArray;
use super::constants::{
    BIOME_SECTION_VOLUME, GLOBAL_BITS_PER_BIOME, MAX_BITS_PER_BIOME, MIN_BITS_PER_BIOME,
};
use super::palette_container::{
    read_palette_container, write_palette_container, PaletteConfig, PaletteContainer,
};

#[derive(Debug, Clone)]
pub struct BiomeSection {
    pub data: PaletteContainer,
}

fn biome_index(x: usize, y: usize, z: usize) -> usize {
    (y << 4) | (z << 2) | x
}

const BIOME_CONFIG: PaletteConfig = PaletteConfig {
    bits_per_value: MIN_BITS_PER_BIOME,
    capacity: BIOME_SECTION_VOLUME,
    max_bits: MAX_BITS_PER_BIOME,
    global_bits: GLOBAL_BITS_PER_BIOME,
};

impl BiomeSection {
    pub fn new(initial_value: u32) -> Self {
        BiomeSection {
            data: PaletteContainer::single(initial_value, BIOME_CONFIG),
        }
    }

    pub fn get_biome(&self, x: usize, y: usize, z: usize) -> u32 {
        self.data.get(biome_index(x, y, z))
    }

    pub fn set_biome(&mut self, x: usize, y: usize, z: usize, biome_id: u32) {
        self.data.set(biome_index(x, y, z), biome_id);
    }

    /// Create a biome section from a local palette + BitArray (anvil loading).
    pub fn from_local_palette(palette: Vec<u32>, data: BitArray) -> Self {
        let container = if palette.len() == 1 {
            PaletteContainer::single(palette[0], BIOME_CONFIG)
        } else {
            PaletteContainer::indirect(palette, data, MAX_BITS_PER_BIOME, GLOBAL_BITS_PER_BIOME)
        };
        BiomeSection { data: container }
    }

    /// Read a biome section from the network binary format. Returns `(section, new_offset)`.
    pub fn read(buffer: &[u8], offset: usize, no_array_length: bool) -> (BiomeSection, usize) {
        let (data, new_offset) =
            read_palette_container(buffer, offset, BIOME_CONFIG, no_array_length);
        (BiomeSection { data }, new_offset)
    }

    /// Append a biome section to a growable buffer.
    pub fn write(&self, buffer: &mut Vec<u8>, no_array_length: bool) {
        write_palette_container(&self.data, buffer, no_array_length);
    }
}
