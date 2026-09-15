//! 临时仪器：把两份 WGSL 分别组装（`px_render::shaders::render_source`）后写出来，
//! 用来回答「迁移前后那份 shader 的数学到底一不一样」。
//!
//! ```text
//! PX_ASSEMBLE_A=<a.wgsl> PX_ASSEMBLE_B=<b.wgsl> cargo test -p px_render --test assemble_pair -- --nocapture
//! ```

#[test]
fn assemble_pair() {
    let (Ok(one), Ok(two)) = (
        std::env::var("PX_ASSEMBLE_A"),
        std::env::var("PX_ASSEMBLE_B"),
    ) else {
        return;
    };
    let modules = px_render::shaders::module_sources();
    for (path, tag) in [(one, "a"), (two, "b")] {
        let source = std::fs::read_to_string(&path).expect("读不了");
        let mut seen = Vec::new();
        let assembled = px_render::shaders::render_source(&source, &modules, &mut seen);
        let out = format!("{}/../target/assembled-{tag}.wgsl", env!("CARGO_MANIFEST_DIR"));
        std::fs::write(&out, &assembled).expect("写不出去");
        println!("{path} → {out}（{} 字节）", assembled.len());
    }
}
