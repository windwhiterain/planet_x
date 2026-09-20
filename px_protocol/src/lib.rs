//! **px-scene ⇄ px-pass 的交换边界**，外加**跨进程那一层**。
//!
//! ## 交换边界（crate 名义职责的那句话）
//!
//! 只装两边都要认的东西：材质参数/绑定的契约（`material`）、场景文档与帧图（`scene`）、
//! 产物的信封（`art` / `stream` / `wire`）、以及握手身份（`SCHEMA_VERSION` /
//! `protocol_hash` / `ProtocolId` / `Handshake`）。
//!
//! ⚠ pcg **内部**的序列化不在这里，各自归各自的 crate：经济世界视图 → `game`（`sim`）。
//!
//! ## 跨进程那一层（`render` / `client` / `frame`）
//!
//! 宿主 ⇄ 它的客户端之间那三份形状（作业请求 / 回读报告 / 租约、连接与握手那半、
//! 四种帧与 `[u32 小端长度][载荷]` 的信封）。
//!
//! ⚠ 它们**本该住在宿主 `px_render` 里**，而"不许住在那边"的理由不是分层，是**编译**：
//! 本 crate 的快照测试要按**真类型**构造它们，若它们住在 `px_render`，这条 dev 边就是
//! `px_protocol` → `px_render` → wgpu / naga / winit —— **协议测试要编一整套 GPU 栈**。
//!
//! ⚠ 曾经试过把它们与握手身份拆去**两个**旁支 crate（`px_handshake` / `px_host_protocol`），
//! 那是**让测试去决定生产结构**：`ProtocolId` / `SCHEMA_VERSION` / `wire` 本来就是协议的定义，
//! 把它们搬出协议 crate，只是为了给一条 dev 边让路。⇒ 收回来：本 crate 是**冻结且极小**的
//! 两份声明式的形状（协议 + 跨进程），`tests/crate_graph.rs` 钉着"运行时只有 serde"。
//!
//! ⚠ 运行时依赖只有 `serde` / `serde_json`；这两份形状都**不带 GPU 栈**，所以协议测试
//! 仍然不为它们编 wgpu。

pub mod art;
pub mod client;
pub mod fnv;
pub mod frame;
pub mod material;
pub mod payload;
pub mod render;
pub mod scene;
pub mod stream;
pub mod wire;

pub use art::{ArtBundle, AssetKind, AssetManifest, MeshData, TextureFormat, TextureShape};
pub use material::{
    MATERIAL_BIND_GROUP, MAX_PARAMS_BYTES, MaterialLayout, PARAMS_ALIGN, PARAMS_BINDING, ParamKind,
    ParamSlot, TEXTURE_SLOTS, TextureDimension, TextureSlot,
};
pub use payload::{Build, PayloadBundle};
pub use scene::{
    Address, AlphaMode, CullMode, Environment, Filter, Geometry, Light, LightKind, Material,
    Member, Object, SCENE_SCHEMA, Sampler, SceneSpec, TextureRef, Transform, Value,
};
pub use stream::Frame;
pub use wire::{Blob, BlobHeader, DType, WireError};

pub const PROTOCOL_SNAPSHOT: &str = include_str!("../snapshots/protocol.snapshot.json");

/// 本仓库这一份协议指纹：**快照文本的 FNV-1a**。
///
/// ⚠ 快照（`snapshots/protocol.snapshot.json`）记的是**这一侧**的形状（`material` / `scene` /
/// `art` / `stream` / `wire` + 跨进程那三份的稳定 JSON）；它进 `protocol_hash()`，
/// 而那个指纹**闸着跨进程握手** ⇒ 改协议必须显式更新快照
/// （`PX_UPDATE_SNAPSHOT=1 cargo test -p px_protocol`）。
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

/// v10 → v11：`Job` 多一路 `Stable`（等到条件成立再逐帧采样）、`Report` 多 `pair`（配对差）、
/// `PerfReport` 多 `frames`/分位数/`key`/`waits`/`gpu_ms`/`compare`。
/// 老的两路（`Shots` / `Perf{windows,drop}`）一个字段都没动，老脚本照走。
///
/// v11 → v12：场景产物从「行星配方」（`parts[]` 带 `kind`）改成**通用渲染文档**
/// （`objects[]` + `lights[]` + `environment`，`SCENE_SCHEMA` 1 → 2）；资产多一种
/// `Texture`、位深多一档 `U16`。渲染器不再认识「行星 / 云 / 大气」。
pub const SCHEMA_VERSION: u32 = 12;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProtocolId {
    pub schema_version: u32,
    pub protocol_hash: u64,
    pub git_rev: String,
}

impl ProtocolId {
    /// 本进程这一份：版本取 [`SCHEMA_VERSION`]、指纹取 [`protocol_hash`]。
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
    /// 本进程这一份（握手帧上发出去的就是它）。
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
                write!(
                    formatter,
                    "协议指纹不一致：本地 {ours:016x}，对端 {theirs:016x}"
                )
            }
        }
    }
}

impl std::error::Error for HandshakeError {}
