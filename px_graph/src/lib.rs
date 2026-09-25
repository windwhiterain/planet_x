//! See docs/graph.md

pub mod driver;
pub mod generate;

pub use driver::{
    BakedShader, Graph, SHADER_VERSION, apply_store_args, args_without_store, artifact_path_of,
    bake_shader_graph, begin, cache_root, graph_manifest, manifest_key_of, param_root, scene_key,
    shader_key, workspace_root, write_graph_manifest, write_shader,
};
pub use px_field_schema::field::{Field, Projection, Stats};
pub use px_graph_schema::{
    Cache, GraphSpec, Key, ManifestEntry, OpId, Report, canonical_params, fnv1a, fnv1a_sources,
    hex, hex_short, node_key, payload_fingerprint,
};
pub use px_mesh_schema::MeshData;
pub use px_volume_schema::{PATCHES, VolumeData};
