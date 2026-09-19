//! **生成 + 编译一张图的单态化实例**（那一级的全部机械部分）。
//!
//! 用法：`cargo run -q -p px_graphs --bin mono-gen -- <图名>`（例如 `clouds`）。
//!
//! 它读 `src/bin/<图名>/mono.rs`（那是一个 `key = value` **声明**，不是 Rust 模块），
//! 然后做四件事：
//!
//! 1. 按声明 + 内容算这一份实例的身份（`id` / `version` / `source_hash`）；
//! 2. 把 `src/bin/<图名>/mono/` 那三份模板复制到 `target/mono/<库名>/crate/`，
//!    并把身份与库名具体化；
//! 3. `cargo build` 那个目录（`--target-dir target/mono/<库名>/build`，**按库名分开**）；
//! 4. 把 cdylib 复制成 `target/debug/px_<库名>_op.dll`
//!    —— 入口名由库名派生（`px_graph_schema::op::table_symbol`），驱动一行不用改。
//!
//! ⚠ 两条**试过且是错的**做法，留在这里当路标：
//!
//! * 让生成物**共用主 workspace 的 `target/`** ⇒ 它的依赖图不同（自己一份
//!   `Cargo.lock`、自己的特征统一）⇒ cargo 把 `px_volume_op.dll` 等**再编一份**
//!   进同一个 `target/debug/deps/` 覆盖主 workspace 那份 ⇒ `LoadLibraryExW failed`；
//! * **构建目录也按内容分** ⇒ 每次改一行场函数都在新目录里从零编依赖
//!   （实测 20.8 s/次，比不带这一级还差）。
//!
//! ⇒ 正解：**内容决定身份（写在源码里），路径只决定"当前那一份"落在哪**
//!   （源码位置固定、构建目录按库名固定）。身份变了，键就变；路径不动，依赖就能复用。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// `src/bin/<图名>/mono.rs` 里那份声明。
struct Declaration {
    lib: String,
    id: String,
    version: u32,
    fields: String,
    template: String,
    ingredients: Vec<String>,
}

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("px_graphs 必须住在 workspace 下")
        .to_path_buf()
}

/// 按行读 `key = value`。`ingredient` 可以出现多次 —— 顺序**进哈希**（见 §28.2）。
fn read_declaration(text: &str, what: &str) -> Declaration {
    let mut values: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            panic!("{what} 的这一行不是 `key = value`：{line}");
        };
        values
            .entry(key.trim())
            .or_default()
            .push(value.trim());
    }
    let one = |key: &str| -> String {
        let found = values
            .get(key)
            .unwrap_or_else(|| panic!("{what} 缺 `{key}`"))
            .clone();
        assert_eq!(found.len(), 1, "{what} 的 `{key}` 写了 {} 次", found.len());
        found[0].to_string()
    };
    Declaration {
        lib: one("lib"),
        id: one("id"),
        version: one("version").parse().expect("version 不是数字"),
        fields: one("fields"),
        template: one("template"),
        ingredients: values.remove("ingredient").unwrap_or_default()
            .into_iter().map(str::to_string).collect(),
    }
}

/// 身份哈希：**定长数组**（`fnv1a_sources` 收 `&[&str]`，`.map()` 只有定长数组才有）——
/// 所以配料条数写死在类型里，多一条少一条都会在编译期报。
fn source_hash(parts: &[(&str, String)]) -> u64 {
    let mut hash = px_graph_schema::identity::FNV_OFFSET;
    for (name, text) in parts {
        // 每份配料**带名字**再哈希：换个文件位置也算"内容变了"。
        hash = px_graph_schema::identity::fnv1a_bytes(
            &[hash.to_le_bytes().as_slice(), name.as_bytes(), b"\n", text.as_bytes()].concat(),
        );
    }
    hash
}

fn main() {
    let graph = std::env::args().nth(1).unwrap_or_else(|| {
        panic!(
            "用法：cargo run -q -p px_graphs --bin mono-gen -- <图名>\n\
             （现在有的：{}）",
            graphs(Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("workspace"))
                .join(" / "),
        )
    });
    let root = workspace();
    let bin = root.join("px_graphs/src/bin");
    let graph_dir = bin.join(&graph);
    let declaration_path = graph_dir.join("mono.rs");
    let text = std::fs::read_to_string(&declaration_path).unwrap_or_else(|err| {
        panic!(
            "读不了 {}：{err}\n（图 `{graph}` 还没有单态化声明？）",
            declaration_path.display()
        )
    });
    let what = declaration_path.display().to_string();
    let declared = read_declaration(&text, &what);

    // ── 1) 身份 ───────────────────────────────────────────────────────────────
    // stage 1 与 stage 2 的模板由生成器自己收进清单 —— 声明里只列两侧的契约。
    let stage1 = graph_dir.join(&declared.fields);
    let template = graph_dir.join(&declared.template);
    let mut parts: Vec<(String, String)> = vec![
        (declared.fields.clone(), read(&stage1)),
        (
            format!("{}/lib.rs", declared.template),
            read(&template.join("lib.rs")),
        ),
        (
            format!("{}/identity.rs", declared.template),
            read(&template.join("identity.rs")),
        ),
        (
            format!("{}/Cargo.toml", declared.template),
            read(&template.join("Cargo.toml")),
        ),
    ];
    for name in &declared.ingredients {
        parts.push((name.clone(), read(&root.join(name))));
    }
    let hash = source_hash(
        &parts
            .iter()
            .map(|(name, text)| (name.as_str(), text.clone()))
            .collect::<Vec<_>>(),
    );
    println!(
        "图 {graph} 的单态化实例：lib={} id={} v{} source_hash={hash:016x}（{} 份配料）",
        declared.lib,
        declared.id,
        declared.version,
        parts.len(),
    );

    // ── 2) 落到按库名分开的位置 ────────────────────────────────────────────────
    let mono_root = root.join("target/mono").join(&declared.lib);
    let crate_dir = mono_root.join("crate");
    let build_dir = mono_root.join("build");
    let src = crate_dir.join("src");
    std::fs::create_dir_all(&src).expect("建不了生成目录");
    for (from, to) in [
        (template.join("Cargo.toml"), crate_dir.join("Cargo.toml")),
        (template.join("lib.rs"), src.join("lib.rs")),
        (template.join("identity.rs"), src.join("identity.rs")),
    ] {
        std::fs::copy(&from, &to)
            .unwrap_or_else(|err| panic!("复制 {} → {} 失败：{err}", from.display(), to.display()));
    }
    // stage 1 是**引用**进来的（模板里 `#[path = "fields.rs"]`），所以复制一份到旁边。
    std::fs::copy(&stage1, src.join("fields.rs")).unwrap_or_else(|err| {
        panic!("复制 {} 失败：{err}", stage1.display())
    });

    // 具体化：库名 + 身份。⚠ 占位符在模板里是**裸值**（引号在 Rust 源里，不在声明里）。
    let identity = read(&src.join("identity.rs"))
        .replace("@MONO_ID@", &format!("\"{}\"", declared.id))
        .replace("@VERSION@", &declared.version.to_string())
        .replace("@SOURCE_HASH@", &hash.to_string());
    std::fs::write(src.join("identity.rs"), identity).expect("写不了 identity.rs");
    // ⚠ 库名要进**两处**：manifest 的 `name` 与 `lib.rs` 的导出符号名
    //   （驱动按 `<库名>_op_table` 找）。
    let manifest = read(&crate_dir.join("Cargo.toml")).replace("@LIB@", &declared.lib);
    std::fs::write(crate_dir.join("Cargo.toml"), manifest).expect("写不了 Cargo.toml");
    let wiring = read(&src.join("lib.rs")).replace("@LIB@", &declared.lib);
    std::fs::write(src.join("lib.rs"), wiring).expect("写不了 lib.rs");

    // ── 3) 编 ─────────────────────────────────────────────────────────────────
    let status = Command::new("cargo")
        .args([
            "build",
            "--manifest-path",
            crate_dir.join("Cargo.toml").to_str().expect("路径不是 UTF-8"),
            "--target-dir",
            build_dir.to_str().expect("路径不是 UTF-8"),
        ])
        .status()
        .expect("起不了 cargo");
    assert!(status.success(), "编单态化实例失败：{status}");

    // ── 4) 放进装载目录 ────────────────────────────────────────────────────────
    let built = build_dir.join(format!("debug/{}.dll", declared.lib));
    // 库名自己已经带 `px_` 前缀（声明里写的就是它）⇒ 这里只补 `_op`：
    // 入口名由**库名**派生（`<库名>_op_table`），所以库名必须以 `_op` 结尾才被装载器认。
    let deposit = root.join(format!("target/debug/{}_op.dll", declared.lib));
    let bytes = std::fs::copy(&built, &deposit)
        .unwrap_or_else(|err| panic!("复制 {} 失败：{err}", built.display()));
    println!(
        "装入 {}（{:.2} MB）",
        deposit.display(),
        bytes as f64 / (1024.0 * 1024.0),
    );
    println!(
        "⚠ 这一份 dylib 在运行时还要它自己的上游 DLL（`dylib` 这个 crate-type 的 ABI）；\
         现在靠主 workspace 的 `target/debug/` 提供 —— 见笔记 §166.4。"
    );
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("读不了 {}：{err}", path.display()))
}

/// `src/bin/` 下带 `mono.rs` 的目录 —— 也就是"有单态化实例的图"。
fn graphs(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root.join("px_graphs/src/bin")) {
        for entry in entries.flatten() {
            if entry.path().join("mono.rs").is_file() {
                found.push(entry.file_name().to_string_lossy().to_string());
            }
        }
    }
    found.sort();
    found
}
