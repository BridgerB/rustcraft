//! Source RCON client for Minecraft servers.
//! Protocol: TCP, little-endian i32 framing.
//! Packet: `[size:i32][id:i32][type:i32][body:utf8][0x00][0x00]`.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const TYPE_AUTH: i32 = 3;
const TYPE_AUTH_RESPONSE: i32 = 2;
const TYPE_COMMAND: i32 = 2;

pub struct RconOptions {
    pub host: String,
    pub port: u16,
    pub password: String,
    pub retry_ms: u64,
    pub max_retries: u32,
}

impl Default for RconOptions {
    fn default() -> Self {
        RconOptions {
            host: "localhost".into(),
            port: 25575,
            password: String::new(),
            retry_ms: 2000,
            max_retries: 15,
        }
    }
}

pub struct RconClient {
    stream: TcpStream,
    request_id: i32,
}

fn encode_packet(id: i32, ty: i32, body: &str) -> Vec<u8> {
    let body = body.as_bytes();
    let length = 4 + 4 + body.len() + 2;
    let mut buf = Vec::with_capacity(4 + length);
    buf.extend_from_slice(&(length as i32).to_le_bytes());
    buf.extend_from_slice(&id.to_le_bytes());
    buf.extend_from_slice(&ty.to_le_bytes());
    buf.extend_from_slice(body);
    buf.push(0);
    buf.push(0);
    buf
}

struct Packet {
    id: i32,
    ty: i32,
    body: String,
}

impl RconClient {
    /// Connect and authenticate, retrying on connection refusal.
    pub async fn connect(options: RconOptions) -> std::io::Result<RconClient> {
        let addr = format!("{}:{}", options.host, options.port);
        let mut attempt = 0;
        let stream = loop {
            match TcpStream::connect(&addr).await {
                Ok(s) => break s,
                Err(e) => {
                    let retriable = matches!(
                        e.kind(),
                        std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::TimedOut
                    );
                    if attempt < options.max_retries && retriable {
                        attempt += 1;
                        tokio::time::sleep(Duration::from_millis(options.retry_ms)).await;
                        continue;
                    }
                    return Err(e);
                }
            }
        };

        let mut client = RconClient {
            stream,
            request_id: 10,
        };
        client.send(1, TYPE_AUTH, &options.password).await?;
        let resp = client.read_packet().await?;
        if resp.id == 1 && resp.ty == TYPE_AUTH_RESPONSE {
            Ok(client)
        } else {
            Err(std::io::Error::other("RCON authentication failed"))
        }
    }

    async fn send(&mut self, id: i32, ty: i32, body: &str) -> std::io::Result<()> {
        self.stream.write_all(&encode_packet(id, ty, body)).await
    }

    async fn read_packet(&mut self) -> std::io::Result<Packet> {
        let mut len_buf = [0u8; 4];
        self.stream.read_exact(&mut len_buf).await?;
        let len = i32::from_le_bytes(len_buf) as usize;
        let mut buf = vec![0u8; len];
        self.stream.read_exact(&mut buf).await?;
        let id = i32::from_le_bytes(buf[0..4].try_into().unwrap());
        let ty = i32::from_le_bytes(buf[4..8].try_into().unwrap());
        let body = String::from_utf8_lossy(&buf[8..len.saturating_sub(2)]).into_owned();
        Ok(Packet { id, ty, body })
    }

    /// Send a command and return the server's response body.
    pub async fn command(&mut self, cmd: &str) -> std::io::Result<String> {
        self.request_id += 1;
        let id = self.request_id;
        self.send(id, TYPE_COMMAND, cmd).await?;
        let resp = self.read_packet().await?;
        Ok(resp.body)
    }
}

/// Strip Minecraft color codes (`§X`) from a string.
pub fn strip_colors(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '§' {
            chars.next(); // drop the formatting code char
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_packet_framing() {
        let pkt = encode_packet(1, TYPE_AUTH, "pw");
        // length prefix = 4(id)+4(type)+2(body)+2(pad) = 12
        assert_eq!(i32::from_le_bytes(pkt[0..4].try_into().unwrap()), 12);
        assert_eq!(i32::from_le_bytes(pkt[4..8].try_into().unwrap()), 1);
        assert_eq!(i32::from_le_bytes(pkt[8..12].try_into().unwrap()), 3);
        assert_eq!(&pkt[12..14], b"pw");
        assert_eq!(&pkt[14..16], &[0, 0]);
    }

    #[test]
    fn strips_color_codes() {
        assert_eq!(strip_colors("§aHello §lWorld"), "Hello World");
        assert_eq!(strip_colors("no codes"), "no codes");
    }
}
