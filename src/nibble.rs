//! Packed 4-bit (nibble) array access.
//!
//! Port of typecraft's `nibble` module. Two nibbles per byte: even indices in
//! the low nibble, odd indices in the high nibble.

/// Reads the nibble at `index` from a packed byte array.
pub fn read_nibble(bytes: &[u8], index: usize) -> u8 {
    if index & 1 == 0 {
        bytes[index >> 1] & 0x0f
    } else {
        bytes[index >> 1] >> 4
    }
}

/// Writes the low 4 bits of `value` at `index` in a packed byte array.
pub fn write_nibble(bytes: &mut [u8], index: usize, value: u8) {
    let byte_index = index >> 1;
    bytes[byte_index] = if index & 1 == 0 {
        (bytes[byte_index] & 0xf0) | (value & 0x0f)
    } else {
        (bytes[byte_index] & 0x0f) | ((value & 0x0f) << 4)
    };
}

/// Allocates a packed nibble array holding `length` nibbles.
pub fn create_nibble_array(length: usize) -> Vec<u8> {
    vec![0u8; length >> 1]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_all_nibbles() {
        let mut bytes = create_nibble_array(16);
        assert_eq!(bytes.len(), 8);
        for i in 0..16 {
            write_nibble(&mut bytes, i, (i as u8) & 0x0f);
        }
        for i in 0..16 {
            assert_eq!(read_nibble(&bytes, i), (i as u8) & 0x0f);
        }
    }

    #[test]
    fn high_and_low_nibbles_independent() {
        let mut bytes = create_nibble_array(2);
        write_nibble(&mut bytes, 0, 0xa);
        write_nibble(&mut bytes, 1, 0x5);
        assert_eq!(bytes[0], 0x5a);
        assert_eq!(read_nibble(&bytes, 0), 0xa);
        assert_eq!(read_nibble(&bytes, 1), 0x5);
    }

    #[test]
    fn masks_to_low_four_bits() {
        let mut bytes = create_nibble_array(2);
        write_nibble(&mut bytes, 0, 0xff);
        assert_eq!(read_nibble(&bytes, 0), 0x0f);
    }
}
