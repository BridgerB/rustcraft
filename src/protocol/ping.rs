//! Server list ping — query a server's status (MOTD, players, version) over the
//! STATUS protocol state, plus protocol-version → Minecraft-version mapping.

use std::time::Instant;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::framing::{frame_packet, Splitter};
use super::value::PValue;
use super::{packet_codec, Direction, ProtocolState};

#[derive(Debug, Clone)]
pub struct PingResponse {
    pub version_name: String,
    pub protocol: i64,
    pub players_online: i64,
    pub players_max: i64,
    pub description: serde_json::Value,
    pub latency_ms: u128,
}

/// Ping a Minecraft server and return its status.
pub async fn ping(host: &str, port: u16) -> std::io::Result<PingResponse> {
    let handshake = packet_codec(ProtocolState::Handshaking, Direction::ToServer)
        .map_err(std::io::Error::other)?;
    let status_write =
        packet_codec(ProtocolState::Status, Direction::ToServer).map_err(std::io::Error::other)?;
    let status_read =
        packet_codec(ProtocolState::Status, Direction::ToClient).map_err(std::io::Error::other)?;

    let mut stream = TcpStream::connect((host, port)).await?;
    stream.set_nodelay(true).ok();

    let write_packet =
        |codec: &super::PacketCodec, name: &str, params: PValue| -> std::io::Result<Vec<u8>> {
            let data = codec.write(name, &params).map_err(std::io::Error::other)?;
            Ok(frame_packet(&data))
        };

    stream
        .write_all(&write_packet(
            &handshake,
            "intention",
            PValue::compound(vec![
                (
                    "protocolVersion",
                    PValue::num(super::protocol_version() as f64),
                ),
                ("serverHost", PValue::str(host)),
                ("serverPort", PValue::num(port as f64)),
                ("nextState", PValue::num(1.0)),
            ]),
        )?)
        .await?;
    stream
        .write_all(&write_packet(
            &status_write,
            "status_request",
            PValue::compound(vec![]),
        )?)
        .await?;

    let mut splitter = Splitter::new();
    let mut buf = vec![0u8; 8192];
    let mut server_info: Option<serde_json::Value> = None;
    let mut ping_time = Instant::now();

    loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            return Err(std::io::Error::other("connection closed during ping"));
        }
        for raw in splitter.write(&buf[..n]) {
            let (name, params) = status_read.read(&raw).map_err(std::io::Error::other)?;
            match name.as_str() {
                "status_response" => {
                    let json = params
                        .get("response")
                        .and_then(PValue::as_str)
                        .unwrap_or("{}");
                    let value: serde_json::Value =
                        serde_json::from_str(json).map_err(std::io::Error::other)?;
                    server_info = Some(value);
                    ping_time = Instant::now();
                    stream
                        .write_all(&write_packet(
                            &status_write,
                            "ping_request",
                            PValue::compound(vec![("time", PValue::Long(0))]),
                        )?)
                        .await?;
                }
                "pong_response" => {
                    if let Some(info) = server_info {
                        let latency_ms = ping_time.elapsed().as_millis();
                        let version = &info["version"];
                        let players = &info["players"];
                        return Ok(PingResponse {
                            version_name: version["name"].as_str().unwrap_or("").to_string(),
                            protocol: version["protocol"].as_i64().unwrap_or(0),
                            players_online: players["online"].as_i64().unwrap_or(0),
                            players_max: players["max"].as_i64().unwrap_or(0),
                            description: info["description"].clone(),
                            latency_ms,
                        });
                    }
                }
                _ => {}
            }
        }
    }
}

/// Map a protocol version number to a known Minecraft version string.
pub fn version_for_protocol(protocol: i64) -> Option<&'static str> {
    Some(match protocol {
        775 => "26.1.2",
        774 => "1.21.11",
        769 => "1.21.4",
        768 => "1.21.2",
        767 => "1.21.1",
        766 => "1.21",
        765 => "1.20.4",
        764 => "1.20.2",
        763 => "1.20.1",
        762 => "1.20",
        _ => return None,
    })
}
