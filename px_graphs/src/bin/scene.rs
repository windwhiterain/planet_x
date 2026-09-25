use px_scene::baked::Baked;
use px_scene::recipe;

const DEFAULT_SCENE: &str = "orbit";

fn main() {
    px_cook::apply_store_args().unwrap_or_else(|err| panic!("{err}"));
    let _graph = px_cook::begin(px_cook::GraphSpec {
        name: "scene".to_string(),
    });

    let mut name = DEFAULT_SCENE.to_string();
    let mut with_graph = true;
    for arg in px_cook::args_without_store().unwrap_or_else(|err| panic!("{err}")) {
        match arg.as_str() {
            "--no-frame-graph" => with_graph = false,
            other => name = other.to_string(),
        }
    }

    let file = recipe::load(&name).unwrap_or_else(|err| panic!("{err}"));
    let root = px_cook::cache_root();
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
    let key = px_cook::scene_key(&spec_json, &member_keys);
    let artifact = px_protocol::scene::cas_path(&root, &px_cook::hex(&key))
        .unwrap_or_else(|err| panic!("{err}"));
    let bytes =
        px_protocol::scene::write_scene(&artifact, &compiled.document, px_cook::fnv1a(&spec_json))
            .unwrap_or_else(|err| panic!("{err}"));

    println!("{}", compiled.document.audit());
    println!(
        "产物 scene -> {}（内容键 {}，不是文件字节的 sha256）",
        artifact.display(),
        px_cook::hex_short(&key)
    );

    let entry = px_cook::ManifestEntry {
        node: compiled.document.name.clone(),
        op: "scene.document".to_string(),
        op_version: u64::from(px_protocol::SCENE_SCHEMA),
        key: px_cook::hex(&key),
        hit: false,
        millis: 0,
        bytes,
        detail: format!("{} 个物体", compiled.document.objects.len()),
    };
    let mut entries = px_cook::graph_manifest("scene").unwrap_or_default();
    entries.retain(|old| old.node != entry.node);
    entries.push(entry);
    entries.sort_by(|one, two| one.node.cmp(&two.node));
    let manifest = px_cook::write_graph_manifest("scene", &entries)
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
