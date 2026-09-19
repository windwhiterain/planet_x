//! **px_graph**：图库本体 —— 驱动、键、清单、CAS、参数，外加 shader 写入与程序化资产。
//!
//! 它**一个算子都不依赖**：算子住在 `px_*_op` 的 dylib 里，运行时按描述符表装载
//! （`px_graph_schema::OpLibrary`）。于是：
//!
//! * 改一个算子 ⇒ 只重编那个 `_op` crate，**图程序一动不动**；
//! * 算子之间、算子与图程序之间只流动 `px_*_schema` 定义的**序列化载荷**；
//! * 键与清单只认描述符（id / version / 参数 JSON / 输入键），不看内存里是什么类型。
//!
//! 图脚本（`px_graphs/src/bin/*.rs`）拿到的 API 就三样：
//! `begin(GraphSpec)` → `node("field.fbm", "continents", &[])` → `finish()`。

pub mod cameras;
pub mod driver;
pub mod generate;

pub use driver::{
    read_cached,
    Artifact, Cache, Driver, Payload, Report, artifact_path_of, begin, cache_root, driver, finish,
    graph_manifest, manifest_key_of, node, params_text, scene_key, shader_key, workspace_root,
    write_graph_manifest, write_shader, SHADER_VERSION,
};
pub use px_graph_schema::{
    GraphSpec, Grid, IndexEntry, Key, ManifestEntry, canonical_params, fnv1a, fnv1a_sources, hex,
    hex_short, node_key, payload_fingerprint,
};
pub use px_field_schema::field::{Field, Projection, Stats};
pub use px_mesh_schema::MeshData;
pub use px_volume_schema::{PATCHES, VolumeData};
