//! **跨进程的握手身份**：`ProtocolId` / `Handshake` 与场景描述的**形状版本**。
//!
//! ⚠ 它为什么是一个**叶子** crate（只依赖 serde）：握手身份同时被**协议那一侧**
//! （`stream` 里那一帧 `Protocol`）和**宿主那一侧**（`px_host_protocol` 的信封、租约）
//! 需要。放在任何一边都会把另一边拖成环：
//!
//! ```text
//! px_protocol ─┬─> px_handshake        （协议侧要 ProtocolId）
//!              └─> px_host_protocol ──> px_handshake   （宿主侧也要）
//! ```
//!
//! ⚠ **形状是冻结的**：`ProtocolId` 的字段名与字段序、`SCHEMA_VERSION` 的值都是线格式
//! （`px_protocol/snapshots/protocol.snapshot.json` 与 `tools/harness.ps1` 认的就是它们）。
//! 改这里等于改「握手会拒掉哪些旧对端」，那是要**看一眼**的地方，不是顺手能改的。

pub mod wire;

pub use wire::{Blob, BlobHeader, DType, WireError};

/// **冻结的协议形状**（`material` / `scene` / `art` / `stream` / `wire` 的稳定 JSON）。
///
/// ⚠ 它跟着 [`protocol_hash`] 走，而那个指纹**闸着跨进程握手** ⇒ 改协议必须显式更新快照
/// （`PX_UPDATE_SNAPSHOT=1 cargo test -p px_protocol`）。快照住在这里而不是 `px_protocol`：
/// 指纹是**握手身份**的一部分，而宿主侧构造 `ProtocolId` 时也要它。
pub const PROTOCOL_SNAPSHOT: &str = include_str!("../snapshots/protocol.snapshot.json");

/// 本仓库这一份协议指纹：**快照文本的 FNV-1a**。
pub fn protocol_hash() -> u64 {
    hash_of(PROTOCOL_SNAPSHOT)
}

pub fn protocol_hash_hex() -> String {
    hash_hex(protocol_hash())
}

use serde::{Deserialize, Serialize};

/// 场景描述与跨进程报文的**形状版本**。
///
/// ⚠ v10 → v11：`Job` 多一路 `Stable`、`Report` 多 `pair`；v11 → v12：场景产物从
/// 「行星配方」改成**通用渲染文档**（`SCENE_SCHEMA` 1 → 2），资产多一种 `Texture`、
/// 位深多一档 `U16`。老的两路一个字段都没动，老脚本照走。
pub const SCHEMA_VERSION: u32 = 12;

/// 一份快照文本的指纹（FNV-1a）。
///
/// ⚠ 文本由调用方给：那一份快照（`px_protocol/snapshots/protocol.snapshot.json`）记的是
/// **协议那一侧的形状**，归 `px_protocol` 所有；这里只负责"怎么算"。
pub fn hash_of(snapshot: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in snapshot.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

pub fn hash_hex(hash: u64) -> String {
    format!("{hash:016x}")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolId {
    pub schema_version: u32,
    pub protocol_hash: u64,
    pub git_rev: String,
}

impl ProtocolId {
    /// 本进程这一份：版本取 [`SCHEMA_VERSION`]、指纹取 [`protocol_hash`]。
    pub fn local() -> Self {
        Self::with_hash(protocol_hash())
    }

    /// 指纹由调用方给（测试夹具要造"另一个协议"时走它）。
    pub fn with_hash(protocol_hash: u64) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            protocol_hash,
            git_rev: env!("PX_GIT_REV").to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Handshake {
    pub id: ProtocolId,
    pub pid: u32,
    pub exe: String,
}

impl Handshake {
    /// 本进程这一份（握手帧上发出去的就是它）。
    pub fn local() -> Self {
        Self::new(ProtocolId::local())
    }

    pub fn new(id: ProtocolId) -> Self {
        Self {
            id,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 指纹是文本的纯函数：同一份文本给同一个数，改一个字节就换一个数。
    /// ⚠ 这不是"顺手加的一条" —— `protocol_hash()` 闸着跨进程握手，
    /// 它要是变成了别的东西，在跑的旧服务会**静默**地被认成同一个协议。
    #[test]
    fn the_hash_is_a_pure_function_of_the_snapshot_text() {
        let one = hash_of("{\"a\":1}");
        assert_eq!(one, hash_of("{\"a\":1}"));
        assert_ne!(one, hash_of("{\"a\":2}"));
        assert_eq!(format!("{one:016x}").len(), 16);
        assert_eq!(hash_hex(one), format!("{one:016x}"));
    }

    /// 版本不同 / 指纹不同都拒，而且报错要点名**两边**的值。
    #[test]
    fn a_handshake_names_both_sides_when_it_refuses() {
        let base = ProtocolId {
            schema_version: SCHEMA_VERSION,
            protocol_hash: 7,
            git_rev: "rev".to_string(),
        };
        let one = Handshake::new(base.clone());
        assert!(one.verify(&one).is_ok());

        let other = Handshake::new(ProtocolId {
            schema_version: SCHEMA_VERSION + 1,
            ..base.clone()
        });
        let err = one.verify(&other).expect_err("版本不同");
        assert!(err.to_string().contains("版本"), "{err}");

        let other = Handshake::new(ProtocolId {
            protocol_hash: 8,
            ..base
        });
        let err = one.verify(&other).expect_err("指纹不同");
        assert!(err.to_string().contains("指纹"), "{err}");
    }
}
