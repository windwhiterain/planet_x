pub mod art;
pub mod client;
pub mod render;
pub mod sim;
pub mod stream;
pub mod wire;

pub use art::{ArtBundle, AssetKind, AssetManifest, MeshData};
pub use render::{ClientError, Lease, Request, Response, Scene};
pub use sim::{DepartmentView, GoodView, Totals, WorldView};
pub use stream::Frame;
pub use wire::{Blob, BlobHeader, DType, WireError};

pub const SCHEMA_VERSION: u32 = 5;

pub const PROTOCOL_SNAPSHOT: &str = include_str!("../snapshots/protocol.snapshot.json");

pub fn protocol_hash() -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in PROTOCOL_SNAPSHOT.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

pub fn protocol_hash_hex() -> String {
    format!("{:016x}", protocol_hash())
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProtocolId {
    pub schema_version: u32,
    pub protocol_hash: u64,
    pub git_rev: String,
}

impl ProtocolId {
    pub fn local() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            protocol_hash: protocol_hash(),
            git_rev: env!("PX_GIT_REV").to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Handshake {
    pub id: ProtocolId,
    pub pid: u32,
    pub exe: String,
}

impl Handshake {
    pub fn local() -> Self {
        Self {
            id: ProtocolId::local(),
            pid: std::process::id(),
            exe: std::env::current_exe()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
        }
    }

    pub fn verify(&self, peer: &Handshake) -> Result<(), HandshakeError> {
        if self.id.schema_version != peer.id.schema_version {
            return Err(HandshakeError::SchemaVersion {
                ours: self.id.schema_version,
                theirs: peer.id.schema_version,
            });
        }
        if self.id.protocol_hash != peer.id.protocol_hash {
            return Err(HandshakeError::ProtocolHash {
                ours: self.id.protocol_hash,
                theirs: peer.id.protocol_hash,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandshakeError {
    SchemaVersion { ours: u32, theirs: u32 },
    ProtocolHash { ours: u64, theirs: u64 },
}

impl std::fmt::Display for HandshakeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SchemaVersion { ours, theirs } => {
                write!(formatter, "协议版本不一致：本地 {ours}，对端 {theirs}")
            }
            Self::ProtocolHash { ours, theirs } => {
                write!(formatter, "协议指纹不一致：本地 {ours:016x}，对端 {theirs:016x}")
            }
        }
    }
}

impl std::error::Error for HandshakeError {}

