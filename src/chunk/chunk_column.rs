//! A full-height column of block + biome sections, plus light and block
//! entities.

use std::collections::HashMap;

use super::biome_section::BiomeSection;
use super::bit_array::BitArray;
use super::chunk_section::ChunkSection;
use crate::nbt::NbtCompound;

const DEFAULT_MIN_Y: i32 = -64;
const DEFAULT_WORLD_HEIGHT: i32 = 384;

pub struct ChunkColumnOptions {
    pub min_y: Option<i32>,
    pub world_height: Option<i32>,
    pub max_bits_per_block: u32,
    pub max_bits_per_biome: u32,
}

/// Serialized light data, mirroring typecraft's `dumpChunkLight` return.
pub struct ChunkLightDump {
    pub sky_light: Vec<Vec<u8>>,
    pub block_light: Vec<Vec<u8>>,
    pub sky_light_mask: Vec<i64>,
    pub block_light_mask: Vec<i64>,
    pub empty_sky_light_mask: Vec<i64>,
    pub empty_block_light_mask: Vec<i64>,
}

pub struct ChunkColumn {
    pub min_y: i32,
    pub world_height: i32,
    pub num_sections: usize,
    pub max_bits_per_block: u32,
    pub max_bits_per_biome: u32,

    pub sections: Vec<ChunkSection>,
    pub biomes: Vec<BiomeSection>,

    pub sky_light_mask: BitArray,
    pub empty_sky_light_mask: BitArray,
    pub sky_light_sections: Vec<Option<BitArray>>,

    pub block_light_mask: BitArray,
    pub empty_block_light_mask: BitArray,
    pub block_light_sections: Vec<Option<BitArray>>,

    pub block_entities: HashMap<(i32, i32, i32), NbtCompound>,
}

fn light_section_index(y: i32, min_y: i32) -> usize {
    (((y - min_y) as f64 / 16.0).floor() as i32 + 1) as usize
}

fn section_block_index(y: i32, z: i32, x: i32, min_y: i32) -> usize {
    ((((y - min_y) & 15) << 8) | (z << 4) | x) as usize
}

impl ChunkColumn {
    pub fn new(options: ChunkColumnOptions) -> Self {
        let min_y = options.min_y.unwrap_or(DEFAULT_MIN_Y);
        let world_height = options.world_height.unwrap_or(DEFAULT_WORLD_HEIGHT);
        let num_sections = (world_height >> 4) as usize;
        let light_len = num_sections + 2;

        ChunkColumn {
            min_y,
            world_height,
            num_sections,
            max_bits_per_block: options.max_bits_per_block,
            max_bits_per_biome: options.max_bits_per_biome,
            sections: (0..num_sections)
                .map(|_| ChunkSection::new(options.max_bits_per_block, 0))
                .collect(),
            biomes: (0..num_sections).map(|_| BiomeSection::new(0)).collect(),
            sky_light_mask: BitArray::new(1, light_len),
            empty_sky_light_mask: BitArray::new(1, light_len),
            sky_light_sections: (0..light_len).map(|_| None).collect(),
            block_light_mask: BitArray::new(1, light_len),
            empty_block_light_mask: BitArray::new(1, light_len),
            block_light_sections: (0..light_len).map(|_| None).collect(),
            block_entities: HashMap::new(),
        }
    }

    // ── Block access ──

    pub fn get_block_state_id(&self, x: i32, y: i32, z: i32) -> u32 {
        let idx = ((y - self.min_y) >> 4) as usize;
        match self.sections.get(idx) {
            Some(section) => {
                section.get_block(x as usize, ((y - self.min_y) & 0xf) as usize, z as usize)
            }
            None => 0,
        }
    }

    pub fn set_block_state_id(&mut self, x: i32, y: i32, z: i32, state_id: u32) {
        let idx = ((y - self.min_y) >> 4) as usize;
        let local_y = ((y - self.min_y) & 0xf) as usize;
        if let Some(section) = self.sections.get_mut(idx) {
            section.set_block(x as usize, local_y, z as usize, state_id);
        }
    }

    // ── Light access ──

    pub fn get_block_light(&self, x: i32, y: i32, z: i32) -> u32 {
        match &self.block_light_sections[light_section_index(y, self.min_y)] {
            Some(section) => section.get(section_block_index(y, z, x, self.min_y)),
            None => 0,
        }
    }

    pub fn set_block_light(&mut self, x: i32, y: i32, z: i32, light: u32) {
        let idx = light_section_index(y, self.min_y);
        if self.block_light_sections[idx].is_none() {
            if light == 0 {
                return;
            }
            self.block_light_sections[idx] = Some(BitArray::new(4, 4096));
            self.block_light_mask.set(idx, 1);
        }
        let bi = section_block_index(y, z, x, self.min_y);
        self.block_light_sections[idx]
            .as_mut()
            .unwrap()
            .set(bi, light);
    }

    pub fn get_sky_light(&self, x: i32, y: i32, z: i32) -> u32 {
        match &self.sky_light_sections[light_section_index(y, self.min_y)] {
            Some(section) => section.get(section_block_index(y, z, x, self.min_y)),
            None => 0,
        }
    }

    pub fn set_sky_light(&mut self, x: i32, y: i32, z: i32, light: u32) {
        let idx = light_section_index(y, self.min_y);
        if self.sky_light_sections[idx].is_none() {
            if light == 0 {
                return;
            }
            self.sky_light_sections[idx] = Some(BitArray::new(4, 4096));
            self.sky_light_mask.set(idx, 1);
        }
        let bi = section_block_index(y, z, x, self.min_y);
        self.sky_light_sections[idx]
            .as_mut()
            .unwrap()
            .set(bi, light);
    }

    // ── Biome access ──

    pub fn get_biome_id(&self, x: i32, y: i32, z: i32) -> u32 {
        let idx = ((y - self.min_y) >> 4) as usize;
        match self.biomes.get(idx) {
            Some(biome) => biome.get_biome(
                (x >> 2) as usize,
                (((y - self.min_y) & 0xf) >> 2) as usize,
                (z >> 2) as usize,
            ),
            None => 0,
        }
    }

    pub fn set_biome_id(&mut self, x: i32, y: i32, z: i32, biome_id: u32) {
        let idx = ((y - self.min_y) >> 4) as usize;
        let local_y = (((y - self.min_y) & 0xf) >> 2) as usize;
        if let Some(biome) = self.biomes.get_mut(idx) {
            biome.set_biome((x >> 2) as usize, local_y, (z >> 2) as usize, biome_id);
        }
    }

    // ── Block entities ──

    pub fn get_block_entity(&self, x: i32, y: i32, z: i32) -> Option<&NbtCompound> {
        self.block_entities.get(&(x, y, z))
    }

    pub fn set_block_entity(&mut self, x: i32, y: i32, z: i32, entity: NbtCompound) {
        self.block_entities.insert((x, y, z), entity);
    }

    pub fn remove_block_entity(&mut self, x: i32, y: i32, z: i32) {
        self.block_entities.remove(&(x, y, z));
    }

    // ── Anvil section loading ──

    #[allow(clippy::too_many_arguments)]
    pub fn load_section_from_anvil(
        &mut self,
        section_y: i32,
        block_palette: Vec<u32>,
        block_data: BitArray,
        biome_palette: Vec<u32>,
        biome_data: BitArray,
        block_light: Option<&[u8]>,
        sky_light: Option<&[u8]>,
    ) {
        let min_cy = (self.min_y >> 4).unsigned_abs() as i32;
        let idx = (section_y + min_cy) as usize;

        self.sections[idx] = ChunkSection::from_local_palette(block_palette, block_data);
        self.biomes[idx] = BiomeSection::from_local_palette(biome_palette, biome_data);

        if let Some(bl) = block_light {
            self.block_light_mask.set(idx + 1, 1);
            self.block_light_sections[idx + 1] = Some(BitArray::from_raw_le_bytes(bl, 4, 4096));
        }
        if let Some(sl) = sky_light {
            self.sky_light_mask.set(idx + 1, 1);
            self.sky_light_sections[idx + 1] = Some(BitArray::from_raw_le_bytes(sl, 4, 4096));
        }
    }

    // ── Network I/O ──

    /// Serialize chunk data to the network protocol format.
    pub fn dump(&self, no_array_length: bool) -> Vec<u8> {
        let mut buffer = Vec::new();
        for i in 0..self.num_sections {
            self.sections[i].write(&mut buffer, no_array_length);
            self.biomes[i].write(&mut buffer, no_array_length);
        }
        buffer
    }

    /// Load chunk data from the network protocol format.
    pub fn load(&mut self, data: &[u8], no_array_length: bool) {
        let mut offset = 0;
        for i in 0..self.num_sections {
            let (section, new_offset) =
                ChunkSection::read(data, offset, self.max_bits_per_block, no_array_length);
            self.sections[i] = section;
            offset = new_offset;
            let (biome, new_offset) = BiomeSection::read(data, offset, no_array_length);
            self.biomes[i] = biome;
            offset = new_offset;
        }
    }

    /// Serialize light data for the network protocol.
    pub fn dump_light(&self) -> ChunkLightDump {
        let collect_light = |sections: &[Option<BitArray>], mask: &BitArray| {
            let mut out = Vec::new();
            for (i, section) in sections.iter().enumerate() {
                if let Some(section) = section {
                    if mask.get(i) != 0 {
                        let mut buf = Vec::new();
                        section.write_data(&mut buf);
                        out.push(buf);
                    }
                }
            }
            out
        };

        ChunkLightDump {
            sky_light: collect_light(&self.sky_light_sections, &self.sky_light_mask),
            block_light: collect_light(&self.block_light_sections, &self.block_light_mask),
            sky_light_mask: self.sky_light_mask.to_long_array(),
            block_light_mask: self.block_light_mask.to_long_array(),
            empty_sky_light_mask: self.empty_sky_light_mask.to_long_array(),
            empty_block_light_mask: self.empty_block_light_mask.to_long_array(),
        }
    }

    /// Load parsed light data from the network protocol.
    #[allow(clippy::too_many_arguments)]
    pub fn load_light(
        &mut self,
        sky_light_data: &[Vec<u8>],
        block_light_data: &[Vec<u8>],
        sky_light_mask_longs: &[i64],
        block_light_mask_longs: &[i64],
        empty_sky_light_mask_longs: &[i64],
        empty_block_light_mask_longs: &[i64],
    ) {
        load_light_sections(
            &mut self.sky_light_sections,
            &mut self.sky_light_mask,
            &mut self.empty_sky_light_mask,
            sky_light_data,
            sky_light_mask_longs,
            empty_sky_light_mask_longs,
        );
        load_light_sections(
            &mut self.block_light_sections,
            &mut self.block_light_mask,
            &mut self.empty_block_light_mask,
            block_light_data,
            block_light_mask_longs,
            empty_block_light_mask_longs,
        );
    }
}

fn load_light_sections(
    sections: &mut [Option<BitArray>],
    light_mask: &mut BitArray,
    empty_mask: &mut BitArray,
    data: &[Vec<u8>],
    incoming_light_mask_longs: &[i64],
    incoming_empty_mask_longs: &[i64],
) {
    let incoming_light_mask = BitArray::from_long_array(incoming_light_mask_longs, 1);
    let incoming_empty_mask = BitArray::from_long_array(incoming_empty_mask_longs, 1);
    let mut current_section_index = 0;

    for y in 0..sections.len() {
        let is_empty = incoming_empty_mask.get(y);
        if incoming_light_mask.get(y) == 0 && is_empty == 0 {
            continue;
        }
        empty_mask.set(y, is_empty);
        light_mask.set(y, 1 - is_empty);

        let mut arr = BitArray::new(4, 4096);
        if is_empty == 0 {
            let buf = &data[current_section_index];
            current_section_index += 1;
            arr.read_data(buf, 0, arr.data.len());
        }
        sections[y] = Some(arr);
    }
}
