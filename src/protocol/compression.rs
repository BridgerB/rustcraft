//! Packet compression — zlib DEFLATE with a varint uncompressed-length prefix.
//! Wire format: `[varint: uncompressed_length][data...]`. A length of 0 means
//! the data is raw (below the compression threshold).

use std::io::{Read, Write};

use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;

use crate::varint::{push_var_int, read_var_int};

/// Compress a packet if it meets the threshold, prefixing the original length.
pub fn compress_packet(data: &[u8], threshold: usize) -> Vec<u8> {
    let mut out = Vec::new();
    if data.len() < threshold {
        push_var_int(&mut out, 0);
        out.extend_from_slice(data);
        return out;
    }
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data).expect("zlib write");
    let compressed = encoder.finish().expect("zlib finish");
    push_var_int(&mut out, data.len() as i32);
    out.extend_from_slice(&compressed);
    out
}

/// Decompress a packet, returning the raw packet data.
pub fn decompress_packet(data: &[u8]) -> std::io::Result<Vec<u8>> {
    let (uncompressed_len, len_size) =
        read_var_int(data, 0).map_err(|e| std::io::Error::other(e.to_string()))?;
    let payload = &data[len_size..];
    if uncompressed_len == 0 {
        return Ok(payload.to_vec());
    }
    let mut out = Vec::with_capacity(uncompressed_len as usize);
    ZlibDecoder::new(payload).read_to_end(&mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_below_threshold() {
        let data = b"hello world";
        let packed = compress_packet(data, 256);
        assert_eq!(packed[0], 0); // uncompressed marker
        assert_eq!(decompress_packet(&packed).unwrap(), data);
    }

    #[test]
    fn roundtrips_above_threshold() {
        let data = vec![42u8; 1000];
        let packed = compress_packet(&data, 256);
        assert_ne!(packed[0], 0);
        assert_eq!(decompress_packet(&packed).unwrap(), data);
    }
}
