//! 图协议的**契约层**：键、载荷清单、算子描述符，以及 **dyn lib 的调用**。
//!
//! 它回答的是「一个算子是什么、怎么叫、叫完回什么」——所以它必须比谁都低：
//!
//! ```text
//! px_protocol        线格式（`Frame` / `Blob` / `ArtBundle`）
//!      ↓
//! px_graph_schema    这一层：Key / PayloadBundle / 描述符 / OpLibrary（装载与调用）
//!      ↓
//! px_*_schema        各领域的数据与参数（场 / 体积 / 网格）
//!      ↓
//! px_*_op            各领域的算子（dylib；只按这一层的契约说话）
//!      ↓
//! px_graph           图库本体：驱动（CAS / 参数 / 清单 / cameras / generate）
//!      ↓
//! px_graphs          图脚本
//! ```
//!
//! 两条不许破的线：
//!
//! 1. **这里没有算子实现、没有 CAS 驱动**：算法在 `px_*_op` 里，驱动在 `px_graph` 里。
//! 2. **算子之间只通过这一层定义的序列化数据说话**：跨边界的是字节，不是内存里的 Rust 对象。
//!    ⇒ §17.1 那套「先 key 后 cook」原样成立：**算键不需要求值**（参数规范化也走描述符表，
//!    不进求值路径）。

pub mod identity;
pub mod keys;
pub mod load;
pub mod op;
pub mod payload;
pub mod protocol;

pub use identity::{FNV_OFFSET, FNV_PRIME, fnv1a, fnv1a_bytes, fnv1a_sources};
pub use keys::{
    Key, canonical_params, hex, hex_short, key_with_cameras, node_key, payload_fingerprint,
};
pub use load::OpLibrary;
pub use op::{OpCall, OpDescriptor, OpKind, OpTable, ParamsCanonical};
pub use payload::PayloadBundle;
pub use protocol::{GraphSpec, Grid, IndexEntry, ManifestEntry};
