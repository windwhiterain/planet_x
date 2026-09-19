//! 最小仪器：复刻 `load_directory` 的每一步，把"到底哪一步空"打出来。
//!
//! ⚠ 它只读目录、只调 `px_graph_schema::load::load_directory`，不碰任何产物。

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "target/debug".to_string());
    println!("目录 = {dir}");

    let mut names = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("读不了目录") {
        let path = entry.expect("条目坏了").path();
        let stem = path.file_stem().and_then(|stem| stem.to_str()).unwrap_or("?");
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("?");
        let stem = stem.strip_prefix("lib").unwrap_or(stem);
        let matched = matches!(extension, "dll" | "so" | "dylib")
            && stem.starts_with("px_")
            && stem.ends_with("_op");
        names.push(format!("{:<28} ext={extension:<5} → {}", stem, matched));
    }
    names.sort();
    for line in &names {
        println!("  {line}");
    }

    match px_graph_schema::load::load_directory(std::path::Path::new(&dir)) {
        Ok(found) => {
            println!("load_directory → Ok({} 个)", found.len());
            for library in &found {
                println!(
                    "  {}｜指纹 {:016x}｜算子 {}",
                    library.path().display(),
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
