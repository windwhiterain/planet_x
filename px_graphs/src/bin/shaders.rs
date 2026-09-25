fn main() {
    px_cook::apply_store_args().unwrap_or_else(|err| panic!("{err}"));
    let _graph = px_cook::begin(px_cook::GraphSpec {
        name: "shaders".to_string(),
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
