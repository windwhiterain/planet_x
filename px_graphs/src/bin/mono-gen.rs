//! **生成 + 编译一份单态化实例**（那一级的全部机械部分）。
//!
//! 用法：`cargo run -q -p px_graphs --bin mono-gen -- <stage 1 的 .rs 路径>`
//!
//! ⚠ **stage 1 可以在任何位置** —— 生成器只认那个路径，别的都从它推出来：
//!
//! | 从哪来 | 是什么 |
//! |---|---|
//! | 命令行 | stage 1 的 `.rs`（里面是那个要单态化的场函数） |
//! | 同目录、同主名的 `.mono` | 声明（`key = value` 文本）：`lib` / `id` / `version` / `ingredient` |
//! | 同目录的 `template.*` | stage 2 的三份模板（`template.Cargo.toml` / `template.lib.rs` / `template.identity.rs`） |
//! | 计算的 | 生成物落 `target/mono/<库名>/{crate,build}`，装入 `target/debug/<库名>_op.dll` |
//!
//! 四件事，每一步都必要：
//!
//! 1. 按声明 + 内容算这一份实例的身份（`id` / `version` / `source_hash`）；
//! 2. 把模板复制到 `target/mono/<库名>/crate/`（`template.lib.rs` → `src/lib.rs`，
//!    去掉 `template.` 前缀），把身份、库名、依赖路径具体化；
//! 3. `cargo build` 那个目录（`--target-dir target/mono/<库名>/build`，**按库名分开**）；
//! 4. 把 cdylib 复制成 `target/debug/<库名>_op.dll` —— 入口名由库名派生
//!    （`<库名>_op_table`），驱动一行不用改。
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

/// stage 1 旁边那份 `<主名>.mono` 里的声明。
struct Declaration {
    lib: String,
    id: String,
    version: u32,
    ingredients: Vec<String>,
}

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("px_graphs 必须住在 workspace 下")
        .to_path_buf()
}

/// 按行读 `key = value`。`ingredient` 可以出现多次 —— 顺序**进哈希**（见 §28.2）。
fn read_declaration(text: &str, what: &str, stage1: &Path) -> Declaration {
    let mut values: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            panic!("{what} 的这一行不是 `key = value`：{line}");
        };
        values.entry(key.trim()).or_default().push(value.trim());
    }
    let one = |key: &str| -> Option<String> {
        values.get(key).map(|found| {
            assert_eq!(found.len(), 1, "{what} 的 `{key}` 写了 {} 次", found.len());
            found[0].to_string()
        })
    };
    let stem = stage1
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_else(|| panic!("{} 没有文件名", stage1.display()));
    Declaration {
        // 库名默认由 stage 1 的主名推：`<主名>.rs` → `px_mono_<主名>`。
        // ⚠ 每份实例的库名**必须唯一**：装载目录里同名就会互相覆盖。
        lib: one("lib").unwrap_or_else(|| format!("px_mono_{stem}")),
        id: one("id").unwrap_or_else(|| panic!("{what} 缺 `id`（这一份实例的算子 id，每份唯一）")),
        version: one("version")
            .map(|text| text.parse().expect("version 不是数字"))
            .unwrap_or(1),
        ingredients: values
            .remove("ingredient")
            .unwrap_or_default()
            .into_iter()
            .map(str::to_string)
            .collect(),
    }
}

/// 身份哈希：每份配料**带名字**再哈希（换个文件位置也算"内容变了"），
/// 并且用长度前缀挡住"拼起来一样"的两种切法（与 `fnv1a_sources` 同一条口径）。
fn source_hash(parts: &[(String, String)]) -> u64 {
    let mut hash = px_graph_schema::identity::FNV_OFFSET;
    for (name, text) in parts {
        hash = px_graph_schema::identity::fnv1a_bytes(
            &[
                hash.to_le_bytes().as_slice(),
                name.as_bytes(),
                b"\n",
                text.as_bytes(),
            ]
            .concat(),
        );
    }
    hash
}

fn main() {
    let stage1 = std::env::args().nth(1).unwrap_or_else(|| {
        panic!(
            "用法：cargo run -q -p px_graphs --bin mono-gen -- <stage 1 的 .rs 路径>\n\
             例：  cargo run -q -p px_graphs --bin mono-gen -- px_graphs/src/bin/clouds/mono/fields.rs"
        )
    });
    let root = workspace();
    let stage1 = if Path::new(&stage1).is_absolute() {
        PathBuf::from(&stage1)
    } else {
        // 相对路径：先按**当前目录**解释（命令行给的通常就是这样），再退回 workspace 根。
        let from_cwd = PathBuf::from(&stage1);
        if from_cwd.is_file() {
            from_cwd
        } else {
            root.join(&stage1)
        }
    };
    assert!(
        stage1.is_file(),
        "stage 1 不是文件：{}（它就是要单态化的那一半，通常只有一个场函数）",
        stage1.display()
    );
    let home = stage1.parent().expect("stage 1 没有父目录").to_path_buf();

    // ── 声明：同目录、同主名的 `.mono`（可以没有，那就只剩默认值）────────────────
    let sidecar = home.join(format!(
        "{}.mono",
        stage1.file_stem().and_then(|stem| stem.to_str()).unwrap()
    ));
    let declared = if sidecar.is_file() {
        read_declaration(
            &read(&sidecar),
            &sidecar.display().to_string(),
            &stage1,
        )
    } else {
        read_declaration("", &format!("（{} 不存在）", sidecar.display()), &stage1)
    };

    // ── 模板：同目录的 `template.*`（三份，缺一不可）──────────────────────────
    let templates: Vec<(PathBuf, PathBuf)> = [
        ("template.Cargo.toml", "Cargo.toml"),
        ("template.lib.rs", "src/lib.rs"),
        ("template.identity.rs", "src/identity.rs"),
    ]
    .iter()
    .map(|(from, to)| {
        let from = home.join(from);
        assert!(
            from.is_file(),
            "stage 2 的模板缺一份：{}（stage 1 旁边要有 template.Cargo.toml / \
             template.lib.rs / template.identity.rs）",
            from.display()
        );
        (from, PathBuf::from(to))
    })
    .collect();

    // ── 1) 身份 ───────────────────────────────────────────────────────────────
    // stage 1 与 stage 2 的模板由生成器自己收进清单 —— 声明里只列两侧的契约。
    let mut parts: Vec<(String, String)> = vec![(
        stage1
            .strip_prefix(&root)
            .unwrap_or(&stage1)
            .display()
            .to_string(),
        read(&stage1),
    )];
    for (from, _) in &templates {
        parts.push((
            from.strip_prefix(&root).unwrap_or(from).display().to_string(),
            read(from),
        ));
    }
    for name in &declared.ingredients {
        parts.push((name.clone(), read(&root.join(name))));
    }
    let hash = source_hash(&parts);
    println!(
        "单态化实例：lib={} id={} v{} source_hash={hash:016x}（stage 1 = {}，{} 份配料）",
        declared.lib,
        declared.id,
        declared.version,
        stage1.strip_prefix(&root).unwrap_or(&stage1).display(),
        parts.len(),
    );

    // ── 2) 落到按库名分开的位置 ────────────────────────────────────────────────
    let mono_root = root.join("target/mono").join(&declared.lib);
    let crate_dir = mono_root.join("crate");
    let build_dir = mono_root.join("build");
    for (from, relative) in &templates {
        let to = crate_dir.join(relative);
        std::fs::create_dir_all(to.parent().expect("目标没有父目录"))
            .expect("建不了生成目录");
        std::fs::copy(from, &to)
            .unwrap_or_else(|err| panic!("复制 {} → {} 失败：{err}", from.display(), to.display()));
    }
    // stage 1 是**引用**进来的（模板里 `#[path = "fields.rs"]` 那一行），所以复制到旁边。
    // 目标名就叫 `fields.rs` —— 模板跟它是同一个约定。
    std::fs::copy(&stage1, crate_dir.join("src/fields.rs")).unwrap_or_else(|err| {
        panic!("复制 {} 失败：{err}", stage1.display())
    });

    // 具体化：库名 + 身份 + 依赖的相对路径。
    // ⚠ 依赖路径必须**从生成位置算**（`target/mono/<库名>/crate/` 上溯到 workspace 根），
    //   这样模板里的 `../px_cook` 写法与生成位置无关。
    let up = format!(
        "{}./",
        "../".repeat(
            crate_dir
                .strip_prefix(&root)
                .expect("生成位置不在 workspace 里")
                .components()
                .count()
        )
    );
    let identity = read(&crate_dir.join("src/identity.rs"))
        .replace("@MONO_ID@", &format!("\"{}\"", declared.id))
        .replace("@VERSION@", &declared.version.to_string())
        .replace("@SOURCE_HASH@", &hash.to_string());
    std::fs::write(crate_dir.join("src/identity.rs"), identity).expect("写不了 identity.rs");
    // 库名进**两处**：manifest 的 `name`、`lib.rs` 的导出符号名（驱动按 `<库名>_op_table` 找）。
    let manifest = read(&crate_dir.join("Cargo.toml"))
        .replace("@LIB@", &declared.lib)
        .replace("@UP@", &up);
    std::fs::write(crate_dir.join("Cargo.toml"), manifest).expect("写不了 Cargo.toml");
    let wiring = read(&crate_dir.join("src/lib.rs")).replace("@LIB@", &declared.lib);
    std::fs::write(crate_dir.join("src/lib.rs"), wiring).expect("写不了 lib.rs");

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
         现在靠主 workspace 的 `target/debug/` 提供 —— 见笔记 §166.5。"
    );
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|err| panic!("读不了 {}：{err}", path.display()))
}
