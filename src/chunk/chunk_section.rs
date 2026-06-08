//! A 16×16×16 block section, backed by a palette container.

use super::bit_array::BitArray;
use super::constants::{
    BLOCK_SECTION_VOLUME, GLOBAL_BITS_PER_BLOCK, MAX_BITS_PER_BLOCK, MIN_BITS_PER_BLOCK,
};
use super::palette_container::{
    read_palette_container, write_palette_container, PaletteConfig, PaletteContainer,
};

#[derive(Debug, Clone)]
pub struct ChunkSection {
    pub data: PaletteContainer,
    pub solid_block_count: i32,
    /// 26.1.2 added a second section-header short (fluid block count).
    pub fluid_count: i16,
}

fn block_index(x: usize, y: usize, z: usize) -> usize {
    (y << 8) | (z << 4) | x
}

fn make_block_config(max_bits_per_block: u32) -> PaletteConfig {
    PaletteConfig {
        bits_per_value: MIN_BITS_PER_BLOCK,
        capacity: BLOCK_SECTION_VOLUME,
        max_bits: MAX_BITS_PER_BLOCK,
        global_bits: max_bits_per_block,
    }
}

impl ChunkSection {
    pub fn new(max_bits_per_block: u32, initial_value: u32) -> Self {
        ChunkSection {
            data: PaletteContainer::single(initial_value, make_block_config(max_bits_per_block)),
            solid_block_count: if initial_value != 0 {
                BLOCK_SECTION_VOLUME as i32
            } else {
                0
            },
            fluid_count: 0,
        }
    }

    pub fn get_block(&self, x: usize, y: usize, z: usize) -> u32 {
        self.data.get(block_index(x, y, z))
    }

    pub fn set_block(&mut self, x: usize, y: usize, z: usize, state_id: u32) {
        let idx = block_index(x, y, z);
        let old_block = self.data.get(idx);
        if state_id == 0 && old_block != 0 {
            self.solid_block_count -= 1;
        } else if state_id != 0 && old_block == 0 {
            self.solid_block_count += 1;
        }
        self.data.set(idx, state_id);
    }

    pub fn is_empty(&self) -> bool {
        self.solid_block_count == 0
    }

    /// Create a section from a local palette + BitArray (anvil loading).
    pub fn from_local_palette(palette: Vec<u32>, data: BitArray) -> Self {
        let container = if palette.len() == 1 {
            PaletteContainer::single(
                palette[0],
                PaletteConfig {
                    bits_per_value: MIN_BITS_PER_BLOCK,
                    capacity: BLOCK_SECTION_VOLUME,
                    max_bits: MAX_BITS_PER_BLOCK,
                    global_bits: GLOBAL_BITS_PER_BLOCK,
                },
            )
        } else {
            PaletteContainer::indirect(palette, data, MAX_BITS_PER_BLOCK, GLOBAL_BITS_PER_BLOCK)
        };

        let mut solid_block_count = 0;
        for i in 0..BLOCK_SECTION_VOLUME {
            if container.get(i) != 0 {
                solid_block_count += 1;
            }
        }

        ChunkSection {
            data: container,
            solid_block_count,
            fluid_count: 0,
        }
    }

    /// Read a section from the network binary format. Returns `(section, new_offset)`.
    pub fn read(
        buffer: &[u8],
        mut offset: usize,
        max_bits_per_block: u32,
        no_array_length: bool,
    ) -> (ChunkSection, usize) {
        let solid_block_count = i16::from_be_bytes([buffer[offset], buffer[offset + 1]]) as i32;
        offset += 2;
        let fluid_count = i16::from_be_bytes([buffer[offset], buffer[offset + 1]]);
        offset += 2;

        let (data, new_offset) = read_palette_container(
            buffer,
            offset,
            make_block_config(max_bits_per_block),
            no_array_length,
        );

        (
            ChunkSection {
                data,
                solid_block_count,
                fluid_count,
            },
            new_offset,
        )
    }

    /// Append a section to a growable buffer in the network binary format.
    pub fn write(&self, buffer: &mut Vec<u8>, no_array_length: bool) {
        buffer.extend_from_slice(&(self.solid_block_count as i16).to_be_bytes());
        buffer.extend_from_slice(&self.fluid_count.to_be_bytes());
        write_palette_container(&self.data, buffer, no_array_length);
    }
}
