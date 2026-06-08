//! Plugin-channel codecs for `custom_payload` packet bodies.
//! Port of typecraft's `pluginChannels.ts` serializers.

/// String channel: a single length-prefixed UTF-8 string (brand, etc.).
pub fn string_serialize(value: &str) -> Vec<u8> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() + 1);
    out.push(bytes.len() as u8);
    out.extend_from_slice(bytes);
    out
}

pub fn string_deserialize(data: &[u8]) -> String {
    if data.is_empty() {
        return String::new();
    }
    let len = data[0] as usize;
    String::from_utf8_lossy(&data[1..(1 + len).min(data.len())]).into_owned()
}

/// Raw channel: pass bytes through unchanged.
pub fn raw_serialize(value: &[u8]) -> Vec<u8> {
    value.to_vec()
}

/// REGISTER/UNREGISTER channel: NUL-separated channel names.
pub fn register_serialize(channels: &[String]) -> Vec<u8> {
    channels.join("\0").into_bytes()
}

pub fn register_deserialize(data: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(data)
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_channel_roundtrip() {
        let bytes = string_serialize("vanilla");
        assert_eq!(bytes[0], 7);
        assert_eq!(string_deserialize(&bytes), "vanilla");
        assert_eq!(string_deserialize(&[]), "");
    }

    #[test]
    fn register_roundtrip() {
        let chans = vec!["minecraft:brand".to_string(), "mc:foo".to_string()];
        let bytes = register_serialize(&chans);
        assert_eq!(register_deserialize(&bytes), chans);
    }
}
