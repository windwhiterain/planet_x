//! 诊断探针：复刻 `load_directory` 的每一步，把"到底哪一步空/哪一步装载失败"打出来。
//!
//! ⚠ 一次性的仪器，不进仓（`.gitignore` 里有 `/tools/loadprobe/target/`）。

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "target/debug".to_string());
    println!("目录 = {dir}");
    let mut matched = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("读不了目录") {
        let path = entry.expect("条目坏了").path();
        let stem = path.file_stem().and_then(|stem| stem.to_str()).unwrap_or("?");
        let stem = stem.strip_prefix("lib").unwrap_or(stem);
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("?");
        if matches!(extension, "dll" | "so" | "dylib")
            && stem.starts_with("px_")
            && stem.ends_with("_op")
        {
            let size = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
            matched.push(format!("{stem:<24} {size:>10} B"));
        }
    }
    matched.sort();
    for line in &matched {
        println!("  匹配 {line}");
    }
    match px_graph_schema::load::load_directory(std::path::Path::new(&dir)) {
        Ok(found) => {
            println!("load_directory → Ok({} 个)", found.len());
            for library in &found {
                println!(
                    "  {}｜指纹 {:016x}｜算子 {}",
                    library.path().file_name().unwrap_or_default().to_string_lossy(),
                    library.fingerprint(),
                    library
                        .descriptors()
                        .iter()
                        .map(|op| op.id)
                        .collect::<Vec<_>>()
                        .join(" / "),
                );
            }
        }
        Err(err) => println!("load_directory → Err：{err}"),
    }
}
