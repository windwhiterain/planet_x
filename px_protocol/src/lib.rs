//! **px-scene ⇄ px-pass 的交换边界**，只装两边都要认的东西：材质参数/绑定的契约
//! （`material`）、场景文档与帧图（`scene`）、产物的信封（`art` / `stream` / `wire`），
//! 以及跨进程握手（`SCHEMA_VERSION` / `protocol_hash` / `ProtocolId` / `Handshake`）。
//!
//! ⚠ pcg **内部**的序列化不在这里，各自归各自的 crate：渲染作业的形状与客户端搬去了
//! `px_host_protocol`（`render` / `client` / `frame`），经济世界视图搬去了 `game`（`sim`）。
//! 它们原来都住在这一份协议里，而"这份协议"的名义职责只有一句话 ——
//! 边界每多装一样，跨进程握手就要为一个**只有一边认**的形状背一次版本。
//!
//! ⚠ 运行时依赖仍然只有 `serde` / `serde_json` 两样，`tests/crate_graph.rs` 钉着这一格。

pub mod art;
pub mod material;
pub mod scene;
pub mod stream;
pub mod wire;

pub use art::{ArtBundle, AssetKind, AssetManifest, MeshData, TextureFormat, TextureShape};
pub use material::{
    MATERIAL_BIND_GROUP, MAX_PARAMS_BYTES, MaterialLayout, PARAMS_ALIGN, PARAMS_BINDING, ParamKind,
    ParamSlot, TEXTURE_SLOTS, TextureDimension, TextureSlot,
};
pub use scene::{
    Address, AlphaMode, CullMode, Environment, Filter, Geometry, Light, LightKind, Material, Member,
    Object, SCENE_SCHEMA, Sampler, SceneSpec, TextureRef, Transform, Value,
};
pub use stream::Frame;
pub use wire::{Blob, BlobHeader, DType, WireError};

/// v10 → v11：`Job` 多一路 `Stable`（等到条件成立再逐帧采样）、`Report` 多 `pair`（配对差）、
/// `PerfReport` 多 `frames`/分位数/`key`/`waits`/`gpu_ms`/`compare`。
/// 老的两路（`Shots` / `Perf{windows,drop}`）一个字段都没动，老脚本照走。
///
/// v11 → v12：场景产物从「行星配方」（`parts[]` 带 `kind`）改成**通用渲染文档**
/// （`objects[]` + `lights[]` + `environment`，`SCENE_SCHEMA` 1 → 2）；资产多一种
/// `Texture`、位深多一档 `U16`。渲染器不再认识「行星 / 云 / 大气」。
pub const SCHEMA_VERSION: u32 = 12;

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


