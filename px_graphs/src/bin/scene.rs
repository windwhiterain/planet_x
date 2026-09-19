//! 场景图：**一份配方**（`art/scene/*.toml`）→ **一份低层帧图产物**（`.pxart`）。
//!
//! 它就是 pcg 的一张**普通的图**（与 `planet` / `clouds` / `desert` 同形）：一次运行、
//! 产物进 CAS、节点进 `target/pcg/scene/manifest.json`。**没有单独的 CLI、没有另一套入口**
//! —— 语义住在 `px_scene` 里，这一支只负责"读配方、算键、落盘、登记"。
//!
//! 它做三件事（细节全在 `px_scene`）：
//!
//! 1. 把配方里的 part 展开成**物体**：几何（网格产物或内建图元）+ 材质（shader 产物 +
//!    按名字给的参数 + 按绑定下标给的贴图）+ 世界系变换；
//! 2. 把需要程序化生成的东西**烘成产物**（色板贴图 / 覆盖度立方图 / 星空 / 环），
//!    写进 CAS 的 `generated` 图 —— 渲染器只认产物，不生成任何东西；
//! 3. 把"怎么看"（评审相机表）与"照什么"（灯表、环境）以及**帧图**（pass 表 / 中间目标 /
//!    帧自有材质 / 材质实例）一并写进文档。
//!
//! ⚠ **键与字节不许动**：`scene_key`（`px_graph::scene_key`）与
//! `px_protocol::scene::write_scene` 一起定下了那份 `.pxart` 的**文件字节**，而
//! `art/anchor/hashes.txt` §三 那六格判的就是它（逃生门）。改这两个中的任何一个，
//! 那六份冻产物就不再"逐字节可复现"。

use px_scene::baked::Baked;
use px_scene::recipe;

const GRAPH_VERSION: u32 = 1;
const SOURCE_HASH: u64 = px_graph::fnv1a(include_str!("scene.rs"));
const DEFAULT_SCENE: &str = "orbit";

fn main() {
    px_graph::begin(px_graph::GraphSpec {
        name: "scene".to_string(),
        version: GRAPH_VERSION,
        source_hash: SOURCE_HASH,
        width: 0,
        height: 0,
        projection: px_protocol::art::Domain::Cube,
        cameras: Vec::new(),
    });

    // 用法：scene [配方名] [--no-frame-graph]
    // ⚠ `--no-frame-graph` 是**兼容逃生门**（见 `px_scene::recipe::compile` 里那段注释），
    //    不是常规用法：它存在的唯一目的是证明老产物还能逐字节复现。
    let mut name = DEFAULT_SCENE.to_string();
    let mut with_graph = true;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--no-frame-graph" => with_graph = false,
            other => name = other.to_string(),
        }
    }

    let file = recipe::load(&name).unwrap_or_else(|err| panic!("{err}"));
    let root = px_graph::cache_root();
    let mut baked = Baked::new();
    let compiled = recipe::compile(&file, &mut baked, with_graph)
        .unwrap_or_else(|err| panic!("{} 编译失败：{err}", file.name));
    baked
        .finish()
        .unwrap_or_else(|err| panic!("写 generated 清单失败：{err}"));

    let spec_json = serde_json::to_string(&compiled.document).unwrap_or_else(|err| panic!("{err}"));
    let member_keys = compiled
        .document
        .members()
        .iter()
        .map(|member| member.key.clone())
        .collect::<Vec<_>>();
    let key = px_graph::scene_key(&spec_json, &member_keys);
    let artifact = px_protocol::scene::cas_path(&root, &px_graph::hex(&key))
        .unwrap_or_else(|err| panic!("{err}"));
    let bytes = px_protocol::scene::write_scene(
        &artifact,
        &compiled.document,
        px_graph::fnv1a(&spec_json),
    )
    .unwrap_or_else(|err| panic!("{err}"));

    println!("{}", compiled.document.audit());
    // ⚠ 尾巴上那一格是**内容键**（`scene_key` 算出来的、也嵌在文件名里那个），**不是文件字节的
    //    sha256** —— 两者是两个量。`art/anchor/hashes.txt` §三 那六格判的是**文件字节**，
    //    而这一行印的是键：拿这一格去比登记值，六份会**全报 ✗ 而真值其实是对的**。
    println!(
        "产物 scene -> {}（内容键 {}，不是文件字节的 sha256）",
        artifact.display(),
        px_graph::hex_short(&key)
    );

    let entry = px_graph::ManifestEntry {
        node: compiled.document.name.clone(),
        op: "scene.document".to_string(),
        op_version: px_protocol::SCENE_SCHEMA,
        key: px_graph::hex(&key),
        hit: false,
        millis: 0,
        bytes,
        min: 0.0,
        max: 0.0,
        mean: compiled.document.objects.len() as f32,
    };
    // 清单按场景名合并：一台机器上会并存好几份场景（有云 / 无云 / …），
    // 后烘的不许把先烘的挤掉。
    let mut entries = px_graph::graph_manifest("scene").unwrap_or_default();
    entries.retain(|old| old.node != entry.node);
    entries.push(entry);
    entries.sort_by(|one, two| one.node.cmp(&two.node));
    let manifest = px_graph::write_graph_manifest("scene", &entries)
        .unwrap_or_else(|err| panic!("写清单失败：{err}"));
    println!(
        "清单 {}｜共 {} 份场景：{}",
        manifest.display(),
        entries.len(),
        entries
            .iter()
            .map(|entry| format!("{}={}", entry.node, &entry.key[..12]))
            .collect::<Vec<_>>()
            .join(" ")
    );
}
