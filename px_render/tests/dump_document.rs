//! 临时仪器：把一份 `.pxart` 里的渲染文档打成 JSON，看它到底写了什么。
//!
//! ```text
//! PX_DUMP_SCENE=<路径> cargo test -p px_render --test dump_document -- --nocapture
//! ```

#[test]
fn dump_document() {
    let Ok(path) = std::env::var("PX_DUMP_SCENE") else {
        return;
    };
    let spec = px_protocol::scene::read_scene(std::path::Path::new(&path)).expect("读不出来");
    println!(
        "{}",
        serde_json::to_string_pretty(&spec).unwrap_or_else(|err| format!("{err}"))
    );
}
