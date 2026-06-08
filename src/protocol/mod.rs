//! Minecraft protocol: schema-driven packet codec, framing, compression, and
//! connection state machine. Port of typecraft's `protocol` module (minus the
//! Node-specific networking, which the `bot` layer wires up via Tokio).

mod auth;
mod client;
mod codec;
mod component;
mod compression;
mod encryption;
mod framing;
mod ping;
mod plugin_channels;
mod states;
mod value;

pub use auth::{mc_server_hash, rsa_public_encrypt};
pub use client::{Client, ClientOptions};
pub use codec::{read, write, CodecError, PacketCodec, TypeRegistry};
pub use component::{hash_component_data, java_arrays_hashcode, serialize_component_data};
pub use compression::{compress_packet, decompress_packet};
pub use encryption::Cfb8;
pub use framing::{frame_packet, Splitter};
pub use ping::{ping, version_for_protocol, PingResponse};
pub use plugin_channels::{
    raw_serialize, register_deserialize, register_serialize, string_deserialize, string_serialize,
};
pub use states::{Direction, ProtocolState};
pub use value::PValue;

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::Value;

/// The assembled ProtoDef schema, generated from typecraft's `buildProtocol()`.
const SCHEMA_JSON: &str = include_str!("data/protocol-schema.json");

fn schema() -> &'static Value {
    static SCHEMA: OnceLock<Value> = OnceLock::new();
    SCHEMA.get_or_init(|| serde_json::from_str(SCHEMA_JSON).expect("valid protocol schema"))
}

/// The protocol version number from the schema.
pub fn protocol_version() -> i64 {
    schema()["version"]["version"].as_i64().unwrap_or(0)
}

/// The target Minecraft version string from the schema.
pub fn minecraft_version() -> &'static str {
    schema()["version"]["minecraftVersion"]
        .as_str()
        .unwrap_or("")
}

/// The shared (cross-state) named types as a fresh map.
pub(crate) fn shared_types_map() -> HashMap<String, Value> {
    schema()["protocol"]["types"]
        .as_object()
        .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}

/// A named shared type's schema.
pub(crate) fn shared_type(name: &str) -> Option<Value> {
    schema()["protocol"]["types"].get(name).cloned()
}

/// Build a packet codec for a protocol state + direction, merging the shared
/// types with that state's packet definitions.
pub fn packet_codec(state: ProtocolState, dir: Direction) -> Result<PacketCodec, CodecError> {
    let protocol = &schema()["protocol"];
    let shared = protocol["types"]
        .as_object()
        .ok_or_else(|| CodecError("missing shared types".into()))?;
    let state_types = protocol[state.as_str()][dir.as_str()]["types"]
        .as_object()
        .ok_or_else(|| CodecError(format!("no types for {}.{}", state.as_str(), dir.as_str())))?;

    let mut merged: HashMap<String, Value> = HashMap::new();
    for (k, v) in shared {
        merged.insert(k.clone(), v.clone());
    }
    for (k, v) in state_types {
        merged.insert(k.clone(), v.clone());
    }

    PacketCodec::new(merged)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_loads() {
        assert!(protocol_version() > 0);
        assert!(!minecraft_version().is_empty());
    }

    #[test]
    fn handshake_intention_roundtrip() {
        let codec = packet_codec(ProtocolState::Handshaking, Direction::ToServer).unwrap();
        let params = PValue::compound(vec![
            ("protocolVersion", PValue::num(774.0)),
            ("serverHost", PValue::str("localhost")),
            ("serverPort", PValue::num(25565.0)),
            ("nextState", PValue::num(2.0)),
        ]);
        let bytes = codec.write("intention", &params).unwrap();
        // first byte is packet id 0x00
        assert_eq!(bytes[0], 0x00);

        let (name, read_params) = codec.read(&bytes).unwrap();
        assert_eq!(name, "intention");
        assert_eq!(
            read_params.get("protocolVersion").unwrap().as_i64(),
            Some(774)
        );
        assert_eq!(
            read_params.get("serverHost").unwrap().as_str(),
            Some("localhost")
        );
        assert_eq!(read_params.get("serverPort").unwrap().as_i64(), Some(25565));
        assert_eq!(read_params.get("nextState").unwrap().as_i64(), Some(2));
    }

    #[test]
    fn play_codecs_build() {
        // Exercises resolution of many shared types referenced by play packets.
        let to_server = packet_codec(ProtocolState::Play, Direction::ToServer).unwrap();
        let to_client = packet_codec(ProtocolState::Play, Direction::ToClient).unwrap();
        assert!(!to_server.packet_ids.is_empty());
        assert!(!to_client.packet_ids.is_empty());
    }

    #[test]
    fn chat_packet_roundtrip() {
        // chat_command is a simple toServer play packet: { command: string }.
        let codec = packet_codec(ProtocolState::Play, Direction::ToServer).unwrap();
        if codec.packet_ids.contains_key("chat_command") {
            let params = PValue::compound(vec![("command", PValue::str("help"))]);
            let bytes = codec.write("chat_command", &params).unwrap();
            let (name, read) = codec.read(&bytes).unwrap();
            assert_eq!(name, "chat_command");
            assert_eq!(read.get("command").unwrap().as_str(), Some("help"));
        }
    }
}
