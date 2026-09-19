//! §28.2 的门：算子的 `SOURCE_HASH` 必须覆盖它的**共享依赖**。
//!
//! 病根：`const SOURCE_HASH: u64 = fnv1a(include_str!("fbm.rs"))` 只哈希算子自己那个文件
//! ⇒ 改 `field.rs` 的方向约定、`noise.rs` 的噪声时，§19.1 那条「源码变了但版本仍是 N」
//! 的告警**一声不响**，而缓存照旧命中 —— 那份告警存在的唯一理由就是拦住这种陈旧命中。
//!
//! 算子拆成 `px_*_op`（dylib）之后这条更硬：共享依赖现在住在 `px_*_schema` 里，
//! 它们不在 dll 的源码集合里，只有 `SOURCE_HASH` 点得到它们。
//!
//! ⚠ 立场变过一次：清单现在**每张图/每个算子各写一份**（`typed.rs` 里那一段），
//! 由 `px_op!` 收进去生成 `const SOURCE_HASH`。于是门扫的是**那段清单**，而不是
//! 某一行 `const`。语言管不住这件事（漏一份照样编译、照样跑），所以用门看住。

use std::path::{Path, PathBuf};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("px_graph 必须住在 workspace 下")
        .to_path_buf()
}

/// 一条规则：哪一段代码里，必须点到哪些共享件。
///
/// `must` 是**子串**（含行内换行由 `\` 接起来的那些也照查），逐个必须命中。
struct Rule {
    file: &'static str,
    /// 在这份文件里找哪一段（`after` 之后的第一个 `until`）。
    after: &'static str,
    until: &'static str,
    /// 这一段里的"一段源码"是怎么写的：
    ///
    /// * `true` —— 直接写 `include_str!`，一个顶一份（网格/体积域）。
    /// * `false` —— 写 `sources!(a, b, c)`，一个顶三份 ⇒ 要**数逗号**（场域；路径相对算子自己的文件）。
    count_parts: bool,
    must: Vec<&'static str>,
}

fn rules() -> Vec<Rule> {
    let mut rules = Vec::new();
    // 场域：每个算子一段 `sources!(...)`，路径相对 `px_field_op/src/`
    let field = [
        ("Constant", vec!["px_field_schema/src/field.rs", "px_field_schema/src/params.rs"]),
        (
            "Fbm",
            vec![
                "px_field_op/src/noise.rs",
                "px_field_schema/src/field.rs",
                "px_field_schema/src/noise.rs",
                "px_field_schema/src/params.rs",
            ],
        ),
        (
            "Ridged",
            vec![
                "px_field_op/src/noise.rs",
                "px_field_schema/src/field.rs",
                "px_field_schema/src/noise.rs",
                "px_field_schema/src/params.rs",
            ],
        ),
        ("Remap", vec!["px_field_schema/src/field.rs", "px_field_schema/src/params.rs"]),
        ("Gradient", vec!["px_field_schema/src/field.rs", "px_field_schema/src/params.rs"]),
        ("Mix", vec!["px_field_schema/src/field.rs", "px_field_schema/src/params.rs"]),
        (
            "Warp",
            vec![
                "px_field_schema/src/field.rs",
                "px_field_schema/src/params.rs",
                "px_verify/src/noise.rs",
            ],
        ),
    ];
    for (name, must) in field {
        let after: &'static str = Box::leak(format!("{name},").into_boxed_str());
        rules.push(Rule {
            file: "px_field_op/src/typed.rs",
            after,
            // 结束标记用块尾那两行（`field_op!` 本身会被子串命中，区间就空了）。
            until: "\n}",
            count_parts: false,
            must,
        });
    }
    // 网格域：每个算子一段 `px_op!`（`include_str!` 数组写在参数里）
    rules.push(Rule {
        file: "px_mesh_op/src/typed.rs",
        after: "CubeSphere =",
        until: "pub struct Proxy",
        count_parts: true,
        must: vec![
            "px_mesh_schema/src/params.rs",
            "px_mesh_schema/src/payload.rs",
            "px_field_schema/src/field.rs",
        ],
    });
    rules.push(Rule {
        file: "px_mesh_op/src/typed.rs",
        after: "Proxy =",
        until: "|p, i, _g|",
        count_parts: true,
        must: vec![
            "px_volume_schema/src/volume.rs",
            "px_volume_schema/src/params.rs",
            "px_volume_schema/src/payload.rs",
            "px_mesh_schema/src/params.rs",
            "px_mesh_schema/src/payload.rs",
        ],
    });
    // 体积域
    rules.push(Rule {
        file: "px_volume_op/src/typed.rs",
        after: "CloudCoarse =",
        until: "|p, i, _g|",
        count_parts: true,
        must: vec![
            "px_volume_schema/src/volume.rs",
            "px_volume_schema/src/params.rs",
            "px_volume_schema/src/payload.rs",
            "px_field_schema/src/field.rs",
            "px_verify/src/cloud_field.rs",
            "px_verify/src/noise.rs",
            "px_verify/src/dual.rs",
            "px_verify/src/proxy.rs",
        ],
    });
    rules
}

#[test]
fn every_operator_source_hash_covers_its_shared_dependencies() {
    let root = workspace();
    let mut checked = 0_usize;

    for rule in rules() {
        let path = root.join(rule.file);
        let label = rule.file.replace('\\', "/");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("读不了 {}：{err}", path.display()));
        let start = text
            .find(rule.after)
            .unwrap_or_else(|| panic!("{label} 里找不到算子 `{}`", rule.after));
        let rest = &text[start..];
        let end = rest
            .find(rule.until)
            .unwrap_or_else(|| panic!("{label} 里 `{}` 之后找不到结束标记", rule.after));
        let section = &rest[..end];

        // 形状：至少两段（自己 + 一个共享件）。漏成单文件版就在这里炸。
        let parts = if rule.count_parts {
            section.matches("include_str!(").count()
        } else {
            // `sources!(a, b, c)` 的参数个数 = 逗号数 + 1
            let args = &section[section.find("sources!(").expect("刚查过")..];
            let args = &args[..args.find(')').expect("sources! 没有收尾")];
            args.matches(',').count() + 1
        };
        assert!(
            parts >= 2,
            "{label} 的 `{}` 只哈希了 {parts} 份源码（§28.2）：{section}",
            rule.after
        );
        for dep in &rule.must {
            assert!(
                section.contains(dep),
                "{label} 的 `{}` 没点到共享依赖 {dep}：{section}",
                rule.after
            );
        }
        checked += 1;
    }

    assert!(checked > 0, "一个算子都没检查到");
    println!("SOURCE_HASH 覆盖共享依赖：检查了 {checked} 个算子");
}

/// 多段哈希本身：长度前缀挡住「拼起来一样」的两种切法。
#[test]
fn the_multi_part_hash_does_not_confuse_a_split() {
    use px_graph::fnv1a_sources;
    assert_ne!(
        fnv1a_sources(&["ab", "c"]),
        fnv1a_sources(&["a", "bc"]),
        "没有长度前缀的话这两种切法会撞"
    );
    assert_ne!(fnv1a_sources(&["a"]), fnv1a_sources(&["a", ""]));
    assert_ne!(
        fnv1a_sources(&["a", "b"]),
        fnv1a_sources(&["b", "a"]),
        "顺序由调用点写死：换了顺序 = 另一份列表"
    );
    assert_eq!(fnv1a_sources(&["a", "b"]), fnv1a_sources(&["a", "b"]));
}
