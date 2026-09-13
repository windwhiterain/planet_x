use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub stream: String,
    pub round: Option<u32>,
    pub width: u32,
    pub height: u32,
    pub out: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub out: String,
    pub round: u32,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub millis: u64,
    pub warm: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lease {
    pub pid: u32,
    pub port: u16,
    pub protocol_hash: u64,
    pub git_rev: String,
    pub exe: String,
}

impl Lease {
    pub fn matches(&self, id: &crate::ProtocolId) -> bool {
        self.protocol_hash == id.protocol_hash && self.git_rev == id.git_rev
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientError {
    NoServer,
    Refused(String),
    Wire(String),
    Io(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoServer => write!(formatter, "没有在跑的渲染服务"),
            Self::Refused(reason) => write!(formatter, "渲染服务拒绝：{reason}"),
            Self::Wire(message) => write!(formatter, "协议失败：{message}"),
            Self::Io(message) => write!(formatter, "IO 失败：{message}"),
        }
    }
}

impl std::error::Error for ClientError {}
