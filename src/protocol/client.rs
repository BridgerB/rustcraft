//! Async protocol client — TCP connection, codec pipeline, compression,
//! encryption, and the handshake → login → configuration → play state machine.
//!
//! Port of typecraft's EventEmitter-based `client.ts` + `handshake.ts`, adapted
//! to an idiomatic Rust pull loop: the caller drives [`Client::next_packet`].

use std::collections::VecDeque;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;

use super::compression::{compress_packet, decompress_packet};
use super::encryption::Cfb8;
use super::framing::{frame_packet, Splitter};
use super::value::PValue;
use super::{auth, packet_codec, Direction, PacketCodec, ProtocolState};

pub struct ClientOptions {
    pub host: String,
    pub port: u16,
    pub username: String,
    /// Online-mode access token + a Microsoft profile UUID enable session auth.
    pub access_token: Option<String>,
    pub uuid: Option<String>,
}

pub struct Client {
    read_half: OwnedReadHalf,
    write_half: OwnedWriteHalf,
    splitter: Splitter,
    read_buf: Vec<u8>,
    /// Decrypted, length-stripped packets awaiting decode. Decoded lazily — one
    /// at a time — so a compression/state change triggered by one packet takes
    /// effect before the next packet in the same TCP read is decoded.
    raw_queue: VecDeque<Vec<u8>>,

    decryptor: Option<Cfb8>,
    encryptor: Option<Cfb8>,
    compression_threshold: i32,
    read_codec: Option<PacketCodec>,
    write_codec: Option<PacketCodec>,

    pub state: ProtocolState,
    pub username: String,
    pub uuid: String,
    pub protocol_version: i64,
    access_token: Option<String>,
}

const OFFLINE_UUID: &str = "00000000-0000-0000-0000-000000000000";

impl Client {
    /// Connect a TCP socket and initialize the handshaking codecs.
    pub async fn connect(host: &str, port: u16, username: &str) -> std::io::Result<Client> {
        let stream = TcpStream::connect((host, port)).await?;
        stream.set_nodelay(true).ok();
        let (read_half, write_half) = stream.into_split();

        let mut client = Client {
            read_half,
            write_half,
            splitter: Splitter::new(),
            read_buf: vec![0u8; 8192],
            raw_queue: VecDeque::new(),
            decryptor: None,
            encryptor: None,
            compression_threshold: -1,
            read_codec: None,
            write_codec: None,
            state: ProtocolState::Handshaking,
            username: username.to_string(),
            uuid: String::new(),
            protocol_version: super::protocol_version(),
            access_token: None,
        };
        client.set_state(ProtocolState::Handshaking);
        Ok(client)
    }

    /// Switch protocol state, rebuilding codecs and resetting the splitter.
    pub fn set_state(&mut self, state: ProtocolState) {
        self.state = state;
        self.read_codec = packet_codec(state, Direction::ToClient).ok();
        self.write_codec = packet_codec(state, Direction::ToServer).ok();
        self.splitter.reset();
    }

    pub fn set_compression(&mut self, threshold: i32) {
        self.compression_threshold = threshold;
    }

    pub fn set_encryption(&mut self, secret: &[u8]) {
        self.encryptor = Some(Cfb8::encryptor(secret));
        self.decryptor = Some(Cfb8::decryptor(secret));
    }

    /// Serialize and send a packet for the current state.
    pub async fn write(&mut self, name: &str, params: PValue) -> std::io::Result<()> {
        let codec = self
            .write_codec
            .as_ref()
            .ok_or_else(|| std::io::Error::other(format!("no write codec for {:?}", self.state)))?;
        let data = codec
            .write(name, &params)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        let compressed = if self.compression_threshold >= 0 {
            compress_packet(&data, self.compression_threshold as usize)
        } else {
            data
        };
        let framed = frame_packet(&compressed);
        let out = match &mut self.encryptor {
            Some(enc) => enc.update_vec(&framed),
            None => framed,
        };
        self.write_half.write_all(&out).await?;
        self.write_half.flush().await
    }

    /// Read the next decoded packet, or `None` on EOF. Each packet is
    /// decompressed and decoded at pop time, so compression/state changes
    /// applied between packets are honored.
    pub async fn next_packet(&mut self) -> std::io::Result<Option<(String, PValue)>> {
        loop {
            if let Some(raw) = self.raw_queue.pop_front() {
                let data = if self.compression_threshold >= 0 {
                    decompress_packet(&raw)?
                } else {
                    raw
                };
                if let Some(codec) = &self.read_codec {
                    if let Ok(packet) = codec.read(&data) {
                        return Ok(Some(packet));
                    }
                }
                continue; // undecodable packet — skip
            }
            let n = self.read_half.read(&mut self.read_buf).await?;
            if n == 0 {
                return Ok(None);
            }
            let mut chunk = self.read_buf[..n].to_vec();
            if let Some(dec) = &mut self.decryptor {
                dec.update(&mut chunk);
            }
            self.raw_queue.extend(self.splitter.write(&chunk));
        }
    }

    /// Drive the handshake → login → configuration flow until the PLAY state is
    /// reached. Returns once the client is in PLAY.
    pub async fn login(&mut self, options: &ClientOptions) -> std::io::Result<()> {
        self.access_token = options.access_token.clone();
        if let Some(uuid) = &options.uuid {
            self.uuid = uuid.clone();
        }
        let profile_id = if self.uuid.is_empty() {
            OFFLINE_UUID.to_string()
        } else {
            self.uuid.clone()
        };

        // Handshake.
        self.write(
            "intention",
            PValue::compound(vec![
                ("protocolVersion", PValue::num(self.protocol_version as f64)),
                ("serverHost", PValue::str(options.host.clone())),
                ("serverPort", PValue::num(options.port as f64)),
                ("nextState", PValue::num(2.0)),
            ]),
        )
        .await?;

        self.set_state(ProtocolState::Login);
        self.write(
            "hello",
            PValue::compound(vec![
                ("name", PValue::str(options.username.clone())),
                ("profileId", PValue::str(profile_id)),
            ]),
        )
        .await?;

        loop {
            let Some((name, params)) = self.next_packet().await? else {
                return Err(std::io::Error::other("connection closed during login"));
            };
            match name.as_str() {
                "hello" => self.handle_encryption(&params).await?,
                "login_compression" => {
                    let t = params
                        .get("compressionThreshold")
                        .and_then(PValue::as_i64)
                        .unwrap_or(-1);
                    self.set_compression(t as i32);
                }
                "login_finished" => {
                    if let Some(u) = params.get("uuid").and_then(PValue::as_str) {
                        self.uuid = u.to_string();
                    }
                    if let Some(u) = params.get("username").and_then(PValue::as_str) {
                        self.username = u.to_string();
                    }
                    if self.protocol_version >= 764 {
                        self.write("login_acknowledged", PValue::compound(vec![]))
                            .await?;
                        self.set_state(ProtocolState::Configuration);
                    } else {
                        self.set_state(ProtocolState::Play);
                        return Ok(());
                    }
                }
                "login_disconnect" | "disconnect" => {
                    return Err(std::io::Error::other("server disconnected during login"));
                }
                // Configuration state.
                "select_known_packs" => {
                    self.write(
                        "select_known_packs",
                        PValue::compound(vec![("packs", PValue::List(vec![]))]),
                    )
                    .await?;
                }
                "finish_configuration" => {
                    self.write("finish_configuration", PValue::compound(vec![]))
                        .await?;
                    self.set_state(ProtocolState::Play);
                    return Ok(());
                }
                "keep_alive" => {
                    let id = params
                        .get("keepAliveId")
                        .cloned()
                        .unwrap_or(PValue::Long(0));
                    self.write("keep_alive", PValue::compound(vec![("keepAliveId", id)]))
                        .await?;
                }
                "ping" => {
                    if let Some(id) = params.get("id").cloned() {
                        let _ = self.write("pong", PValue::compound(vec![("id", id)])).await;
                    }
                }
                _ => {}
            }
        }
    }

    async fn handle_encryption(&mut self, params: &PValue) -> std::io::Result<()> {
        let public_key = params
            .get("publicKey")
            .and_then(PValue::as_bytes)
            .unwrap_or(&[])
            .to_vec();
        let challenge = params
            .get("challenge")
            .and_then(PValue::as_bytes)
            .unwrap_or(&[])
            .to_vec();
        let server_id = params
            .get("serverId")
            .and_then(PValue::as_str)
            .unwrap_or("")
            .to_string();

        let mut secret = [0u8; 16];
        rand::Rng::fill(&mut rand::thread_rng(), &mut secret);

        // Online mode: notify the Mojang session server before responding.
        if let Some(token) = self.access_token.clone() {
            if !self.uuid.is_empty() {
                join_session_server(&token, &self.uuid, &server_id, &secret, &public_key)
                    .await
                    .map_err(std::io::Error::other)?;
            }
        }

        let encrypted_secret =
            auth::rsa_public_encrypt(&public_key, &secret).map_err(std::io::Error::other)?;
        let encrypted_token =
            auth::rsa_public_encrypt(&public_key, &challenge).map_err(std::io::Error::other)?;

        self.write(
            "key",
            PValue::compound(vec![
                ("keybytes", PValue::Bytes(encrypted_secret)),
                ("encryptedChallenge", PValue::Bytes(encrypted_token)),
            ]),
        )
        .await?;
        self.set_encryption(&secret);
        Ok(())
    }
}

/// Notify the Mojang session server that this client is joining (online mode).
async fn join_session_server(
    access_token: &str,
    profile_id: &str,
    server_id: &str,
    shared_secret: &[u8],
    public_key: &[u8],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let hash = super::mc_server_hash(server_id, shared_secret, public_key);
    let body = serde_json::json!({
        "accessToken": access_token,
        "selectedProfile": profile_id.replace('-', ""),
        "serverId": hash,
    });
    let res = reqwest::Client::new()
        .post("https://sessionserver.mojang.com/session/minecraft/join")
        .json(&body)
        .send()
        .await?;
    if !res.status().is_success() {
        return Err(format!("session server join failed: {}", res.status()).into());
    }
    Ok(())
}
