//! **px_graph**：图库本体 —— 驱动、清单、CAS、参数、评审相机，外加 shader 写入与程序化资产。
//!
//! 它**一个算子都不依赖**（`px_graphs/tests/crate_graph.rs` 有一道门看着）：驱动只认
//! `Cache` 那几个方法；键与清单只认「算子身份 + 规范参数 + 上游的键」，不看内存里是什么类型。
//!
//! 图脚本（`px_graphs/src/bin/*.rs`）拿到的 API 就三样：
//! `begin(GraphSpec)` → `cook::<O>(&cache, "节点名", 输入)` → `finish()`。

pub mod cameras;
pub mod driver;
pub mod generate;

pub use driver::{
    Graph, SHADER_VERSION, artifact_path_of, begin, cache_root, graph_manifest, manifest_key_of,
    scene_key, shader_key, workspace_root, write_graph_manifest, write_shader,
};
pub use px_graph_schema::{
    Cache, GraphSpec, Grid, Key, ManifestEntry, OpId, Report, canonical_params, fnv1a,
    fnv1a_sources, hex, hex_short, node_key, payload_fingerprint,
};
pub use px_field_schema::field::{Field, Projection, Stats};
pub use px_mesh_schema::MeshData;
pub use px_volume_schema::{PATCHES, VolumeData};
