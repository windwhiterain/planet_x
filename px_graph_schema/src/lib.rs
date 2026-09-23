//! 图协议的**契约层**：键、载荷清单、算子身份与**装载**、以及驱动与算子之间的那道接缝。
//!
//! 它回答的是「一个节点是什么、键怎么算、载荷长什么样、算子怎么调」——所以它必须比谁都低：
//!
//! ```text
//! px_protocol        线格式（`Frame` / `Blob` / `ArtBundle`）
//!      ↓
//! px_graph_schema    这一层：Key / PayloadBundle / Grid / 算子身份与契约（PxOp）+ 装载（ops）
//!      ↓
//! px_*_schema        各领域的数据、参数、以及**算子声明**（`ops.rs`：`px_op!` 那几行）
//!      ↓
//! px_cook            `cached` + 图脚本那一侧唯一的门
//!      ↓
//! px_graph           图库本体：驱动（CAS / 参数 / 清单 / cameras / generate）
//!      ↓
//! px_graphs          图脚本
//! ```
//!
//! 三条不许破的线：
//!
//! 1. **这里没有算子实现、没有 CAS 驱动**：算法在 `px_*_op` 里（dylib，运行时装载），
//!    驱动在 `px_graph` 里。
//! 2. **算子之间只通过这一层定义的序列化数据说话**：跨算子边界的是 `PayloadBundle`
//!    （就是 CAS 里那份字节），不是内存里的 Rust 对象。
//!    ⇒ 「先 key 后 cached」成立：**算键不需要求值**。
//! 3. **图程序不 cargo 依赖实现库**：依赖了就会"改一行实现 ⇒ 重编重链图程序"。
//!    这条线由 `px_graphs/tests/crate_graph.rs` 看着。

pub mod cache;
pub mod contract;
pub mod identity;
pub mod keys;
pub mod ops;
pub mod payload;
pub mod protocol;

pub use cache::{Cache, Report};
pub use contract::{Cooked, PxInputs, PxOp, interface_hash};
pub use identity::blake3;
pub use identity::{FNV_OFFSET, FNV_PRIME, HashField, PxKeyed, fnv1a, fnv1a_bytes, fnv1a_sources};
pub use keys::{
    Key, OpId, canonical_params, hex, hex_short, key_with_cameras, node_key, payload_fingerprint,
};
pub use payload::{Build, PayloadBundle};
pub use protocol::{GraphSpec, Grid, ManifestEntry};

/// **契约这一份源码**的指纹（`build.rs` 按"编译进去的全部源码"算）。
///
/// 装载实现库时与它手里那一份比：对不上 ⇒ 当场拒（见 [`ops::library`]）。
/// ⚠ 这个值**不进键**：它管的是"能不能调"，不是"产物是什么"。
pub const SOURCE_HASH: &str = env!("PX_SOURCE_HASH");

/// **工具链指纹**：`rustc -vV` + target + `RUSTFLAGS` + profile 的哈希（`build.rs` 算的）。
///
/// ⚠ 跨 dylib 的 `extern "Rust"` ABI 靠"同一份 rustc + 同一套布局"成立。**契约握手只保证
/// 后者**（两边编译的是同一份契约源码），前者得单独验：实现库导出它被哪套工具链编的
/// （`__toolchain_hash`），装载时与这一份比 —— 对不上**当场拒**，不要拿错的 ABI 去调。
/// ⚠ 不含 `DEBUG` / `opt-level`：它们不动布局（而 DLL 的文件字节从不进任何键）。
pub const TOOLCHAIN_HASH: &str = env!("PX_TOOLCHAIN_HASH");
