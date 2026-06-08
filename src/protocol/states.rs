//! Minecraft protocol states and packet directions.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolState {
    Handshaking,
    Status,
    Login,
    Configuration,
    Play,
}

impl ProtocolState {
    pub fn as_str(self) -> &'static str {
        match self {
            ProtocolState::Handshaking => "handshaking",
            ProtocolState::Status => "status",
            ProtocolState::Login => "login",
            ProtocolState::Configuration => "configuration",
            ProtocolState::Play => "play",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    ToClient,
    ToServer,
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::ToClient => "toClient",
            Direction::ToServer => "toServer",
        }
    }
}
