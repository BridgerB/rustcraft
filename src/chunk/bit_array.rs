//! Packed integer array storing N-bit values, "NoSpan" variant (values never
//! cross a 64-bit word boundary) — the format used by Minecraft Java 1.16+ for
//! block/biome palettes.
//!
//! typecraft backs this with a `Uint32Array` (pairs of u32 faking 64-bit
//! longs); here we use a native `Vec<u64>`, which removes all the 32-bit
//! half-word juggling.

/// Number of bits needed to represent `value` (0 → 0, 1 → 1, 255 → 8).
pub fn needed_bits(value: u32) -> u32 {
    32 - value.leading_zeros()
}

/// Low-bit mask for `bits` (1..=64), avoiding shift-overflow at 64.
fn mask_for(bits: u32) -> u64 {
    if bits >= 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BitArray {
    pub data: Vec<u64>,
    pub bits_per_value: u32,
    pub capacity: usize,
    values_per_long: usize,
    value_mask: u64,
}

impl BitArray {
    pub fn new(bits_per_value: u32, capacity: usize) -> Self {
        let bits = bits_per_value.clamp(1, 64);
        let values_per_long = (64 / bits) as usize;
        let longs = capacity.div_ceil(values_per_long.max(1));
        BitArray {
            data: vec![0; longs],
            bits_per_value,
            capacity,
            values_per_long,
            value_mask: mask_for(bits),
        }
    }

    pub fn from_data(data: Vec<u64>, bits_per_value: u32, capacity: usize) -> Self {
        let bits = bits_per_value.clamp(1, 64);
        BitArray {
            data,
            bits_per_value,
            capacity,
            values_per_long: (64 / bits) as usize,
            value_mask: mask_for(bits),
        }
    }

    pub fn get(&self, index: usize) -> u32 {
        let start = index / self.values_per_long;
        let offset = (index % self.values_per_long) * self.bits_per_value as usize;
        ((self.data[start] >> offset) & self.value_mask) as u32
    }

    pub fn set(&mut self, index: usize, value: u32) {
        let start = index / self.values_per_long;
        let offset = (index % self.values_per_long) * self.bits_per_value as usize;
        let cleared = self.data[start] & !(self.value_mask << offset);
        self.data[start] = cleared | ((value as u64 & self.value_mask) << offset);
    }

    pub fn resize_bits(&self, new_bits_per_value: u32) -> BitArray {
        let mut result = BitArray::new(new_bits_per_value, self.capacity);
        for i in 0..self.capacity {
            result.set(i, self.get(i));
        }
        result
    }

    pub fn resize_capacity(&self, new_capacity: usize) -> BitArray {
        let mut result = BitArray::new(self.bits_per_value, new_capacity);
        let count = new_capacity.min(self.capacity);
        for i in 0..count {
            result.set(i, self.get(i));
        }
        result
    }

    /// Number of 64-bit longs in the backing store.
    pub fn long_count(&self) -> usize {
        self.data.len()
    }

    /// Convert to `i64` longs for NBT long-array serialization.
    pub fn to_long_array(&self) -> Vec<i64> {
        self.data.iter().map(|&w| w as i64).collect()
    }

    /// Build from NBT long-array values.
    pub fn from_long_array(longs: &[i64], bits_per_value: u32) -> BitArray {
        let values_per_long = (64 / bits_per_value) as usize;
        let capacity = values_per_long * longs.len();
        let data = longs.iter().map(|&l| l as u64).collect();
        BitArray::from_data(data, bits_per_value, capacity)
    }

    /// Wrap a raw nibble byte buffer (little-endian words) as a 4-bit array —
    /// used when loading anvil light sections.
    pub fn from_raw_le_bytes(bytes: &[u8], bits_per_value: u32, capacity: usize) -> BitArray {
        let mut data = Vec::with_capacity(bytes.len() / 8);
        for chunk in bytes.chunks(8) {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            data.push(u64::from_le_bytes(word));
        }
        BitArray::from_data(data, bits_per_value, capacity)
    }

    /// Read big-endian 64-bit longs into the backing store, resizing if needed.
    pub fn read_data(&mut self, buffer: &[u8], mut offset: usize, long_count: usize) -> usize {
        if long_count != self.data.len() {
            self.data = vec![0; long_count];
        }
        for word in self.data.iter_mut() {
            let bytes: [u8; 8] = buffer[offset..offset + 8].try_into().unwrap();
            *word = u64::from_be_bytes(bytes);
            offset += 8;
        }
        offset
    }

    /// Append the backing store as big-endian 64-bit longs.
    pub fn write_data(&self, buffer: &mut Vec<u8>) {
        for &word in &self.data {
            buffer.extend_from_slice(&word.to_be_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needed_bits_values() {
        assert_eq!(needed_bits(0), 0);
        assert_eq!(needed_bits(1), 1);
        assert_eq!(needed_bits(2), 2);
        assert_eq!(needed_bits(3), 2);
        assert_eq!(needed_bits(4), 3);
        assert_eq!(needed_bits(15), 4);
        assert_eq!(needed_bits(16), 5);
        assert_eq!(needed_bits(255), 8);
        assert_eq!(needed_bits(256), 9);
        assert_eq!(needed_bits(65535), 16);
    }

    #[test]
    fn stores_and_retrieves() {
        let mut arr = BitArray::new(4, 16);
        arr.set(0, 5);
        arr.set(1, 10);
        arr.set(15, 15);
        assert_eq!(arr.get(0), 5);
        assert_eq!(arr.get(1), 10);
        assert_eq!(arr.get(15), 15);
    }

    #[test]
    fn initializes_to_zero() {
        let arr = BitArray::new(8, 100);
        for i in 0..100 {
            assert_eq!(arr.get(i), 0);
        }
    }

    #[test]
    fn various_bits_per_value() {
        for bits in [1u32, 2, 4, 5, 8, 14, 16] {
            let max_val = (1u32 << bits) - 1;
            let mut arr = BitArray::new(bits, 64);
            for i in 0..64u32 {
                arr.set(i as usize, i % (max_val + 1));
            }
            for i in 0..64u32 {
                assert_eq!(arr.get(i as usize), i % (max_val + 1));
            }
        }
    }

    #[test]
    fn resizes_bits() {
        let mut arr = BitArray::new(4, 16);
        for i in 0..16 {
            arr.set(i, i as u32);
        }
        let resized = arr.resize_bits(8);
        for i in 0..16 {
            assert_eq!(resized.get(i), i as u32);
        }
    }

    #[test]
    fn resizes_capacity() {
        let mut arr = BitArray::new(4, 16);
        for i in 0..16 {
            arr.set(i, i as u32);
        }
        let resized = arr.resize_capacity(32);
        for i in 0..16 {
            assert_eq!(resized.get(i), i as u32);
        }
        for i in 16..32 {
            assert_eq!(resized.get(i), 0);
        }
    }

    #[test]
    fn roundtrips_through_long_array() {
        let mut arr = BitArray::new(5, 100);
        for i in 0..100 {
            arr.set(i, (i % 32) as u32);
        }
        let longs = arr.to_long_array();
        let restored = BitArray::from_long_array(&longs, 5);
        for i in 0..100 {
            assert_eq!(restored.get(i), (i % 32) as u32);
        }
    }
}
