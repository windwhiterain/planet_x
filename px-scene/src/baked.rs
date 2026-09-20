//! **生成物**（贴图 / 网格）：写进 CAS，并在 `generated` 图里登记。
//!
//! 场景编译器烘的东西不是"图的产品"（没有 `art/<图>/<节点>.toml` 那种参数文件），
//! 而是**由场景语义算出来的**：色板贴图、覆盖度立方图、星空、环的网格与环带。
//! 它们仍然进 CAS（键 = 内容，§17.1），节点名进 `target/pcg/generated/manifest.json`。
//!
//! ⚠ 写的**次序**是内容的一部分：清单按节点名排序，所以次序不进键；但同一份场景在不同
//! 次序下会烘出不同的 `hit` 打印，而"审计不能因为走缓存而哑掉"（框架不变式）。
//! ⇒ 次序照搬，别顺手重排。

use std::collections::BTreeMap;

use px_graph::generate::{self, Generated};
use px_graph::ManifestEntry;
use px_protocol::art::{MeshData, TextureFormat};
use px_protocol::wire::DType;

/// 生成物的图名：文档里的成员写成 `generated::surface_color` 这种。
pub const GENERATED: &str = "generated";

/// 场景烘图**这一次运行**里写的生成物。
pub struct Baked {
    generated: Vec<ManifestEntry>,
}

impl Baked {
    pub fn new() -> Self {
        Self {
            generated: Vec::new(),
        }
    }

    /// 一份贴图产物 + 它在 `generated` 图里的成员引用。
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
        self.register(
            node,
            op,
            &written,
            format!("{} 字节贴图", data.bytes.len()),
        );
        Ok(px_protocol::scene::Member::new(
            GENERATED,
            node,
            &px_graph::hex(&written.key),
        ))
    }

    /// 一份网格产物 + 它的成员引用。
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
            if written.hit { "（CAS 里已有）" } else { "" },
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

    /// 把这一次写的那些**合并**进 `generated` 的清单（老的保住，同名的换掉）。
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

    /// 空转一次：没有任何生成物时也要把清单写出来（与老行为一致）。
    pub fn is_empty(&self) -> bool {
        self.generated.is_empty()
    }

    /// 这一批写进去的那些节点名 → 键（审计 / 对账用）。
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
