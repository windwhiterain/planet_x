use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const PROTOCOL_WHITELIST: [&str; 2] = ["serde", "serde_json"];
// ⚠ S8-c 标注：`px_render` 已在 §154 删掉 ⇒ 第一条禁止边**今天没有主体**（扫不到那个包名）。
// 它守的是"**渲染宿主不许拖 sim**"，而宿主今天是 `px_render_wgpu` —— 那个名字**不在这张表里**
// ⇒ 这一格目前**没人在守**。留着原文是因为它是契约的记录；换名字/加回宿主时，
// 请把这条边改成新名字（本单元只标注、不改门：动门的覆盖面要评审裁）。
const FORBIDDEN_EDGES: [(&str, &str); 2] = [("px_render", "px_sim"), ("px_web", "px_sim")];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("px_protocol 必须住在 workspace 下")
        .to_path_buf()
}

fn manifest(path: &Path) -> toml::Value {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("读不到 {}：{err}", path.display()));
    text.parse()
        .unwrap_or_else(|err| panic!("{} 不是合法 TOML：{err}", path.display()))
}

fn dep_names(doc: &toml::Value, section: &str) -> BTreeSet<String> {
    doc.get(section)
        .and_then(|value| value.as_table())
        .map(|table| table.keys().cloned().collect())
        .unwrap_or_default()
}

fn package_name(doc: &toml::Value, fallback: &str) -> String {
    doc.get("package")
        .and_then(|package| package.get("name"))
        .and_then(|name| name.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| fallback.to_string())
}

#[test]
fn protocol_runtime_dependencies_are_whitelisted() {
    let doc = manifest(&Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"));
    let names = dep_names(&doc, "dependencies");
    let allowed: BTreeSet<String> = PROTOCOL_WHITELIST.iter().map(|name| name.to_string()).collect();
    assert_eq!(
        names, allowed,
        "px_protocol 的运行时依赖只允许 {PROTOCOL_WHITELIST:?}（协议必须是冻结且极小的）"
    );
}

#[test]
fn consumers_never_depend_on_sim() {
    let root = root();
    let workspace = manifest(&root.join("Cargo.toml"));
    let mut manifests = vec![root.join("Cargo.toml")];
    let members: Vec<String> = workspace
        .get("workspace")
        .and_then(|value| value.get("members"))
        .and_then(|value| value.as_array())
        .map(|array| {
            array
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    for member in &members {
        let path = root.join(member).join("Cargo.toml");
        if path.exists() {
            manifests.push(path);
        }
    }

    let mut checked = 0usize;
    for path in &manifests {
        let doc = manifest(path);
        let name = package_name(&doc, "?");
        for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
            let deps = dep_names(&doc, section);
            for (from, to) in FORBIDDEN_EDGES {
                assert!(
                    !(name == from && deps.contains(to)),
                    "{name} 在 [{section}] 里依赖了 {to}：渲染器与 sim 只能通过 px_protocol 通信"
                );
            }
        }
        checked += 1;
    }

    assert!(
        checked >= 3,
        "至少要检查到根包 / game / px_protocol 三份 manifest，实际 {checked}"
    );
}

#[test]
fn protocol_is_a_workspace_member() {
    let root = root();
    let workspace = manifest(&root.join("Cargo.toml"));
    let members = workspace
        .get("workspace")
        .and_then(|value| value.get("members"))
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(
        members.iter().any(|item| item.as_str() == Some("px_protocol")),
        "px_protocol 必须是 workspace 成员，否则它不会被任何门覆盖"
    );
}
