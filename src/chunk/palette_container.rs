//! Paletted storage for a chunk/biome section: a single value, an indirect
//! palette (BitArray indices into a palette), or a direct global-palette array.

use super::bit_array::{needed_bits, BitArray};
use crate::varint::{push_var_int, read_var_int};

#[derive(Debug, Clone, Copy)]
pub struct PaletteConfig {
    pub bits_per_value: u32,
    pub capacity: usize,
    pub max_bits: u32,
    pub global_bits: u32,
}

#[derive(Debug, Clone)]
pub struct SingleValueContainer {
    pub value: u32,
    pub bits_per_value: u32,
    pub capacity: usize,
    pub max_bits: u32,
    pub global_bits: u32,
}

#[derive(Debug, Clone)]
pub struct IndirectContainer {
    pub data: BitArray,
    pub palette: Vec<u32>,
    pub max_bits: u32,
    pub global_bits: u32,
}

#[derive(Debug, Clone)]
pub struct DirectContainer {
    pub data: BitArray,
}

#[derive(Debug, Clone)]
pub enum PaletteContainer {
    Single(SingleValueContainer),
    Indirect(IndirectContainer),
    Direct(DirectContainer),
}

impl PaletteContainer {
    pub fn single(value: u32, config: PaletteConfig) -> Self {
        PaletteContainer::Single(SingleValueContainer {
            value,
            bits_per_value: config.bits_per_value,
            capacity: config.capacity,
            max_bits: config.max_bits,
            global_bits: config.global_bits,
        })
    }

    pub fn indirect(palette: Vec<u32>, data: BitArray, max_bits: u32, global_bits: u32) -> Self {
        PaletteContainer::Indirect(IndirectContainer {
            data,
            palette,
            max_bits,
            global_bits,
        })
    }

    pub fn direct(data: BitArray) -> Self {
        PaletteContainer::Direct(DirectContainer { data })
    }

    pub fn get(&self, index: usize) -> u32 {
        match self {
            PaletteContainer::Single(c) => c.value,
            PaletteContainer::Indirect(c) => c.palette[c.data.get(index) as usize],
            PaletteContainer::Direct(c) => c.data.get(index),
        }
    }

    /// Set a value, upgrading the container in place if needed
    /// (single → indirect, indirect → direct).
    pub fn set(&mut self, index: usize, value: u32) {
        match self {
            PaletteContainer::Single(c) => {
                if value == c.value {
                    return;
                }
                let mut data = BitArray::new(c.bits_per_value, c.capacity);
                data.set(index, 1);
                *self = PaletteContainer::indirect(
                    vec![c.value, value],
                    data,
                    c.max_bits,
                    c.global_bits,
                );
            }
            PaletteContainer::Indirect(c) => {
                if let Some(direct) = c.set_or_upgrade(index, value) {
                    *self = PaletteContainer::Direct(direct);
                }
            }
            PaletteContainer::Direct(c) => c.data.set(index, value),
        }
    }
}

impl IndirectContainer {
    /// Returns `Some(DirectContainer)` if the palette overflowed `max_bits` and
    /// must convert to a direct (global-palette) container.
    fn set_or_upgrade(&mut self, index: usize, value: u32) -> Option<DirectContainer> {
        let palette_index = match self.palette.iter().position(|&v| v == value) {
            Some(i) => i,
            None => {
                let i = self.palette.len();
                self.palette.push(value);
                let bits = needed_bits(i as u32);
                if bits > self.data.bits_per_value {
                    if bits <= self.max_bits {
                        self.data = self.data.resize_bits(bits);
                    } else {
                        return Some(self.convert_to_direct(index, value));
                    }
                }
                i
            }
        };
        self.data.set(index, palette_index as u32);
        None
    }

    fn convert_to_direct(&self, set_index: usize, set_value: u32) -> DirectContainer {
        let mut data = BitArray::new(self.global_bits, self.data.capacity);
        for i in 0..self.data.capacity {
            data.set(i, self.palette[self.data.get(i) as usize]);
        }
        data.set(set_index, set_value);
        DirectContainer { data }
    }
}

fn calc_long_count(bits_per_value: u32, capacity: usize) -> usize {
    capacity.div_ceil((64 / bits_per_value) as usize)
}

/// Read a palette container at `offset`. Returns `(container, new_offset)`.
pub fn read_palette_container(
    buffer: &[u8],
    mut offset: usize,
    config: PaletteConfig,
    no_array_length: bool,
) -> (PaletteContainer, usize) {
    let bits_per_block = buffer[offset];
    offset += 1;

    // Single value
    if bits_per_block == 0 {
        let (value, size) = read_var_int(buffer, offset).unwrap();
        offset += size;
        if !no_array_length {
            let (_, size) = read_var_int(buffer, offset).unwrap();
            offset += size;
        }
        return (PaletteContainer::single(value as u32, config), offset);
    }

    // Direct palette
    if bits_per_block as u32 > config.max_bits {
        let mut data = BitArray::new(bits_per_block as u32, config.capacity);
        let long_count = if no_array_length {
            calc_long_count(bits_per_block as u32, config.capacity)
        } else {
            let (n, size) = read_var_int(buffer, offset).unwrap();
            offset += size;
            n as usize
        };
        offset = data.read_data(buffer, offset, long_count);
        return (PaletteContainer::direct(data), offset);
    }

    // Indirect palette
    let (palette_len, size) = read_var_int(buffer, offset).unwrap();
    offset += size;
    let mut palette = Vec::with_capacity(palette_len as usize);
    for _ in 0..palette_len {
        let (entry, size) = read_var_int(buffer, offset).unwrap();
        offset += size;
        palette.push(entry as u32);
    }

    let mut data = BitArray::new(bits_per_block as u32, config.capacity);
    let long_count = if no_array_length {
        calc_long_count(bits_per_block as u32, config.capacity)
    } else {
        let (n, size) = read_var_int(buffer, offset).unwrap();
        offset += size;
        n as usize
    };
    offset = data.read_data(buffer, offset, long_count);

    (
        PaletteContainer::indirect(palette, data, config.max_bits, config.global_bits),
        offset,
    )
}

/// Append a palette container to a growable buffer.
pub fn write_palette_container(
    container: &PaletteContainer,
    buffer: &mut Vec<u8>,
    no_array_length: bool,
) {
    match container {
        PaletteContainer::Single(c) => {
            buffer.push(0);
            push_var_int(buffer, c.value as i32);
            if !no_array_length {
                buffer.push(0); // data array length = 0
            }
        }
        PaletteContainer::Indirect(c) => {
            buffer.push(c.data.bits_per_value as u8);
            push_var_int(buffer, c.palette.len() as i32);
            for &entry in &c.palette {
                push_var_int(buffer, entry as i32);
            }
            if !no_array_length {
                push_var_int(buffer, c.data.long_count() as i32);
            }
            c.data.write_data(buffer);
        }
        PaletteContainer::Direct(c) => {
            buffer.push(c.data.bits_per_value as u8);
            if !no_array_length {
                push_var_int(buffer, c.data.long_count() as i32);
            }
            c.data.write_data(buffer);
        }
    }
}
