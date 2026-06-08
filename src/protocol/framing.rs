//! Packet framing — varint length-prefix encoding/decoding.
//! Wire format: `[varint: packet_length][packet_data...]`.

use crate::varint::{push_var_int, read_var_int};

/// Frame a packet by prepending its varint-encoded length.
pub fn frame_packet(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 5);
    push_var_int(&mut out, data.len() as i32);
    out.extend_from_slice(data);
    out
}

/// Extracts complete length-prefixed packets from a byte stream, buffering any
/// partial trailing packet across calls.
#[derive(Default)]
pub struct Splitter {
    buffer: Vec<u8>,
}

impl Splitter {
    pub fn new() -> Self {
        Splitter::default()
    }

    pub fn reset(&mut self) {
        self.buffer.clear();
    }

    /// Append a chunk and return any complete packet payloads it produced.
    pub fn write(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        self.buffer.extend_from_slice(chunk);
        let mut packets = Vec::new();
        let mut consumed = 0;

        loop {
            let remaining = &self.buffer[consumed..];
            if remaining.is_empty() {
                break;
            }
            let (packet_len, len_size) = match read_var_int(remaining, 0) {
                Ok(r) => r,
                Err(_) => break, // incomplete varint
            };
            let packet_len = packet_len as usize;
            if remaining.len() < len_size + packet_len {
                break; // incomplete packet
            }
            packets.push(remaining[len_size..len_size + packet_len].to_vec());
            consumed += len_size + packet_len;
        }

        if consumed > 0 {
            self.buffer.drain(..consumed);
        }
        packets
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_and_splits() {
        let a = frame_packet(&[1, 2, 3]);
        let b = frame_packet(&[9, 9]);
        assert_eq!(a[0], 3);

        let mut s = Splitter::new();
        let mut stream = a.clone();
        stream.extend_from_slice(&b);
        let packets = s.write(&stream);
        assert_eq!(packets, vec![vec![1, 2, 3], vec![9, 9]]);
    }

    #[test]
    fn buffers_partial_packets() {
        let framed = frame_packet(&[1, 2, 3, 4]);
        let mut s = Splitter::new();
        assert!(s.write(&framed[..2]).is_empty());
        let packets = s.write(&framed[2..]);
        assert_eq!(packets, vec![vec![1, 2, 3, 4]]);
    }
}
