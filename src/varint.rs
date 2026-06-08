//! VarInt / VarLong encoding — Minecraft's LEB128 variant.
//!
//! VarInt: 1–5 bytes (32-bit), VarLong: 1–10 bytes (64-bit). Port of
//! typecraft's `protocol/varint.ts`. Read functions take a buffer + offset and
//! return `(value, size)`; write functions take a mutable buffer + offset and
//! return the new offset.

use std::fmt;

#[derive(Debug, PartialEq, Eq)]
pub enum VarIntError {
    TooBig,
    UnexpectedEof,
}

impl fmt::Display for VarIntError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VarIntError::TooBig => write!(f, "VarInt too big"),
            VarIntError::UnexpectedEof => write!(f, "VarInt unexpected end of buffer"),
        }
    }
}

impl std::error::Error for VarIntError {}

/// Reads a VarInt at `offset`. Returns `(value, bytes_read)`.
pub fn read_var_int(buffer: &[u8], offset: usize) -> Result<(i32, usize), VarIntError> {
    let mut value: u32 = 0;
    let mut size: usize = 0;
    loop {
        let byte = *buffer
            .get(offset + size)
            .ok_or(VarIntError::UnexpectedEof)?;
        value |= ((byte & 0x7f) as u32).wrapping_shl((size as u32) * 7);
        size += 1;
        if size > 5 {
            return Err(VarIntError::TooBig);
        }
        if byte & 0x80 == 0 {
            break;
        }
    }
    Ok((value as i32, size))
}

/// Writes `value` as a VarInt at `offset`. Returns the new offset.
pub fn write_var_int(value: i32, buffer: &mut [u8], offset: usize) -> usize {
    let mut v = value as u32;
    let mut offset = offset;
    while v & !0x7f != 0 {
        buffer[offset] = (v as u8 & 0x7f) | 0x80;
        offset += 1;
        v >>= 7;
    }
    buffer[offset] = v as u8;
    offset + 1
}

/// Appends `value` as a VarInt to a growable buffer.
pub fn push_var_int(buffer: &mut Vec<u8>, value: i32) {
    let mut v = value as u32;
    while v & !0x7f != 0 {
        buffer.push((v as u8 & 0x7f) | 0x80);
        v >>= 7;
    }
    buffer.push(v as u8);
}

/// Appends `value` as a VarLong to a growable buffer.
pub fn push_var_long(buffer: &mut Vec<u8>, value: i64) {
    let mut v = value as u64;
    while v & !0x7f != 0 {
        buffer.push((v as u8 & 0x7f) | 0x80);
        v >>= 7;
    }
    buffer.push(v as u8);
}

/// Number of bytes `value` occupies as a VarInt.
pub fn size_of_var_int(value: i32) -> usize {
    let mut v = value as u32;
    let mut size = 0;
    loop {
        v >>= 7;
        size += 1;
        if v == 0 {
            break;
        }
    }
    size
}

/// Reads a VarLong at `offset`. Returns `(value, bytes_read)`.
pub fn read_var_long(buffer: &[u8], offset: usize) -> Result<(i64, usize), VarIntError> {
    let mut value: u64 = 0;
    let mut size: usize = 0;
    loop {
        let byte = *buffer
            .get(offset + size)
            .ok_or(VarIntError::UnexpectedEof)?;
        value |= ((byte & 0x7f) as u64).wrapping_shl((size as u32) * 7);
        size += 1;
        if size > 10 {
            return Err(VarIntError::TooBig);
        }
        if byte & 0x80 == 0 {
            break;
        }
    }
    Ok((value as i64, size))
}

/// Writes `value` as a VarLong at `offset`. Returns the new offset.
pub fn write_var_long(value: i64, buffer: &mut [u8], offset: usize) -> usize {
    let mut v = value as u64;
    let mut offset = offset;
    while v & !0x7f != 0 {
        buffer[offset] = (v as u8 & 0x7f) | 0x80;
        offset += 1;
        v >>= 7;
    }
    buffer[offset] = v as u8;
    offset + 1
}

/// Number of bytes `value` occupies as a VarLong.
pub fn size_of_var_long(value: i64) -> usize {
    let mut v = value as u64;
    let mut size = 0;
    loop {
        v >>= 7;
        size += 1;
        if v == 0 {
            break;
        }
    }
    size
}

#[cfg(test)]
mod tests {
    use super::*;

    const CASES: &[(i32, &[u8])] = &[
        (0, &[0x00]),
        (1, &[0x01]),
        (127, &[0x7f]),
        (128, &[0x80, 0x01]),
        (255, &[0xff, 0x01]),
        (25565, &[0xdd, 0xc7, 0x01]),
        (2097151, &[0xff, 0xff, 0x7f]),
        (2147483647, &[0xff, 0xff, 0xff, 0xff, 0x07]),
        (-1, &[0xff, 0xff, 0xff, 0xff, 0x0f]),
        (-2147483648, &[0x80, 0x80, 0x80, 0x80, 0x08]),
    ];

    #[test]
    fn reads_varint() {
        for &(expected, bytes) in CASES {
            let (value, size) = read_var_int(bytes, 0).unwrap();
            assert_eq!(value, expected);
            assert_eq!(size, bytes.len());
        }
    }

    #[test]
    fn writes_varint() {
        for &(value, expected) in CASES {
            let mut buf = [0u8; 5];
            let end = write_var_int(value, &mut buf, 0);
            assert_eq!(end, expected.len());
            assert_eq!(&buf[..end], expected);
        }
    }

    #[test]
    fn size_of_varint_matches() {
        for &(value, expected) in CASES {
            assert_eq!(size_of_var_int(value), expected.len());
        }
    }

    #[test]
    fn reads_varint_at_offset() {
        let buf = [0xaa, 0xbb, 0xdd, 0xc7, 0x01, 0xcc];
        let (value, size) = read_var_int(&buf, 2).unwrap();
        assert_eq!(value, 25565);
        assert_eq!(size, 3);
    }

    #[test]
    fn errors_on_varint_too_big() {
        let buf = [0x80, 0x80, 0x80, 0x80, 0x80, 0x80];
        assert_eq!(read_var_int(&buf, 0), Err(VarIntError::TooBig));
    }

    #[test]
    fn varlong_reads_writes_zero() {
        let mut buf = [0u8; 10];
        let end = write_var_long(0, &mut buf, 0);
        assert_eq!(end, 1);
        let (value, size) = read_var_long(&buf, 0).unwrap();
        assert_eq!(value, 0);
        assert_eq!(size, 1);
    }

    #[test]
    fn varlong_roundtrips_positive() {
        for v in [1i64, 127, 128, 255, 2147483647, 9223372036854775807] {
            let mut buf = [0u8; 10];
            let end = write_var_long(v, &mut buf, 0);
            let (value, _) = read_var_long(&buf, 0).unwrap();
            assert_eq!(value, v);
            assert_eq!(size_of_var_long(v), end);
        }
    }

    #[test]
    fn varlong_roundtrips_negative() {
        let mut buf = [0u8; 10];
        write_var_long(-1, &mut buf, 0);
        let (value, _) = read_var_long(&buf, 0).unwrap();
        assert_eq!(value, -1);
    }

    #[test]
    fn errors_on_varlong_too_big() {
        let buf = [0x80u8; 11];
        assert_eq!(read_var_long(&buf, 0), Err(VarIntError::TooBig));
    }
}
