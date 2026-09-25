use std::collections::BTreeMap;

use px_graph::ManifestEntry;
use px_graph::generate::{self, Generated};
use px_protocol::art::{MeshData, TextureFormat};
use px_protocol::wire::DType;

pub const GENERATED: &str = "generated";

pub struct Baked {
    generated: Vec<ManifestEntry>,
}

impl Baked {
    pub fn new() -> Self {
        Self {
            generated: Vec::new(),
        }
    }

    pub fn texture(
        &mut self,
        node: &str,
        data: generate::TextureData,
        op: &str,
    ) -> Result<px_protocol::scene::Member, String> {
        let dtype = match data.format {
            TextureFormat::Rgba8Srgb => DType::U8,
            TextureFormat::Rgba16Float => DType::U16,
        };
        let written = generate::write_texture(node, data.shape(), &data.bytes, dtype)
            .map_err(|err| format!("写贴图产物 {node} 失败：{err}"))?;
        self.register(node, op, &written, format!("{} 字节贴图", data.bytes.len()));
        Ok(px_protocol::scene::Member::new(
            GENERATED,
            node,
            &px_graph::hex(&written.key),
        ))
    }

    pub fn mesh(
        &mut self,
        node: &str,
        mesh: &MeshData,
        op: &str,
    ) -> Result<px_protocol::scene::Member, String> {
        let written = generate::write_generated_mesh(node, mesh)
            .map_err(|err| format!("写网格产物 {node} 失败：{err}"))?;
        self.register(
            node,
            op,
            &written,
            format!("{} 顶点 / {} 三角形", mesh.vertices(), mesh.triangles()),
        );
        Ok(px_protocol::scene::Member::new(
            GENERATED,
            node,
            &px_graph::hex(&written.key),
        ))
    }

    fn register(&mut self, node: &str, op: &str, written: &Generated, detail: String) {
        println!(
            "{} {node:<16} {op:<20} {}  {:>9} B{}",
            if written.hit { "命中" } else { "重算" },
            px_graph::hex_short(&written.key),
            written.bytes,
            if written.hit {
                "（CAS 里已有）"
            } else {
                ""
            },
        );
        self.generated.push(ManifestEntry {
            node: node.to_string(),
            op: op.to_string(),
            op_version: 1,
            key: px_graph::hex(&written.key),
            hit: written.hit,
            millis: written.millis,
            bytes: written.bytes,
            detail,
        });
    }

    pub fn finish(&self) -> Result<std::path::PathBuf, String> {
        let mut entries = px_graph::graph_manifest(GENERATED).unwrap_or_default();
        for entry in &self.generated {
            entries.retain(|old| old.node != entry.node);
            entries.push(entry.clone());
        }
        entries.sort_by(|one, two| one.node.cmp(&two.node));
        let path = px_graph::write_graph_manifest(GENERATED, &entries)?;
        println!("清单 {}｜共 {} 份生成物", path.display(), entries.len());
        Ok(path)
    }

    pub fn is_empty(&self) -> bool {
        self.generated.is_empty()
    }

    pub fn keys(&self) -> BTreeMap<String, String> {
        self.generated
            .iter()
            .map(|entry| (entry.node.clone(), entry.key.clone()))
            .collect()
    }
}

impl Default for Baked {
    fn default() -> Self {
        Self::new()
    }
}
