//! `shaders` 图：`art/shaders/*.wgsl` 里**每一份入口**反射出契约、写进 CAS、写清单。
//!
//! ⚠ 这一支今天是**薄壳**：烘那一套搬进了 [`px_cook::bake_shader_graph`]
//!   （2026-09-20）。理由不是"文件太长"，而是**下游也要能烘**：`px-scene` 的帧图编译
//!   （与它的测试）按成员名去查这张图的清单 —— 从前那份清单只可能由某个人**先手动跑一次
//!   这个 bin** 才有，于是干净 checkout（`target/` 被 gitignore）一跑测试就红
//!   （实测 `px-scene` 的 `frame::tests` 三条全红："图 'shaders' 的清单读不到"）。
//!   判据要的是"靶子在仓库里、产物可重跑" ⇒ "可重跑"必须是一个**能被调用的函数**。
//!
//! 槽表（哪些 `.wgsl` 是入口）也从这一支搬到了那里：判据是**没有 `#define_import_path`**
//! （那是模块的标记，库住 `art/shaders/lib`），排序后烘 ⇒ 同一棵树两次烘出来的清单逐字节相同。

fn main() {
    // ⚠ 这张图**一个节点都不 cook**：`begin` 只要它那一行摘要（图名 / 参数目录 / 缓存条数）。
    let _graph = px_cook::begin(px_cook::GraphSpec {
        name: "shaders".to_string(),
        width: 0,
        height: 0,
        projection: px_protocol::art::Domain::Cube,
        cameras: Vec::new(),
    });

    let baked = px_cook::bake_shader_graph().unwrap_or_else(|err| panic!("{err}"));
    println!("入口 shader {} 份：{}", baked.len(), slots(&baked));
    for shader in &baked {
        println!(
            "产物 {} -> {}（{}，{} 字节 WGSL）",
            shader.slot,
            shader.artifact.display(),
            px_cook::hex_short(&shader.key),
            shader.wgsl_bytes,
        );
        println!("  {}", shader.closure);
    }
    let manifest = px_cook::cache_root().join("shaders").join("manifest.json");
    println!(
        "共 {} 份 shader：{}；清单 {}",
        baked.len(),
        baked
            .iter()
            .map(|shader| format!("{}={}", shader.slot, &px_cook::hex(&shader.key)[..12]))
            .collect::<Vec<_>>()
            .join(" "),
        manifest.display()
    );
}

fn slots(baked: &[px_cook::BakedShader]) -> String {
    baked
        .iter()
        .map(|shader| shader.slot.as_str())
        .collect::<Vec<_>>()
        .join(" / ")
}
