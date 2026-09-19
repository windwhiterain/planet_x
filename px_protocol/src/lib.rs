//! **px-scene ⇄ px-pass 的交换边界**，只装两边都要认的东西：材质参数/绑定的契约
//! （`material`）、场景文档与帧图（`scene`）、产物的信封（`art` / `stream` / `wire`）。
//!
//! ⚠ pcg **内部**的序列化不在这里，各自归各自的 crate：
//! · 渲染作业的形状与客户端、跨进程的作业信封 → `px_host_protocol`（`render` / `client` / `frame`）；
//! · 经济世界视图 → `game`（`sim`）；
//! · 握手身份与线格式（`ProtocolId` / `SCHEMA_VERSION` / `wire`）→ 叶子 crate **`px_handshake`**。
//!
//! 最后那一条是**必须**分开的：握手身份与线格式**宿主也要**，而宿主本来就依赖协议 ⇒
//! 留在这一份里就是 `px_protocol ⇄ px_host_protocol` 的环。放进叶子 crate 之后，
//! 依赖图是**无环**的：`px_protocol → px_handshake`、`px_host_protocol → px_handshake`。
//!
//! ⚠ 运行时依赖只有 `serde` / `serde_json` / `px_handshake`，`tests/crate_graph.rs` 钉着这一格。

pub mod art;
pub mod material;
pub mod scene;
pub mod stream;

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

/// ⚠ 路径照旧（`px_protocol::wire::…` / `px_protocol::{ProtocolId, Handshake}`），
/// 实现只有一份（叶子 crate `px_handshake`）。
pub use px_handshake::wire;
pub use px_handshake::{
    Handshake, HandshakeError, PROTOCOL_SNAPSHOT, ProtocolId, SCHEMA_VERSION, protocol_hash,
    protocol_hash_hex,
};
