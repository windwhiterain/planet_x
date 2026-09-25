use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const PROTOCOL_WHITELIST: [&str; 2] = ["serde", "serde_json"];
enum Subject {
    Present,
    Reserved,
}

const FORBIDDEN_EDGES: [(&str, &str, Subject); 2] = [
    ("px_render", "px_sim", Subject::Present),
    ("px_web", "px_sim", Subject::Reserved),
];

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
fn protocol_dependencies_are_whitelisted_and_never_pull_the_host() {
    let doc = manifest(&Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"));
    let names = dep_names(&doc, "dependencies");
    let allowed: BTreeSet<String> = PROTOCOL_WHITELIST
        .iter()
        .map(|name| name.to_string())
        .collect();
    assert_eq!(
        names, allowed,
        "px_protocol 的运行时依赖只允许 {PROTOCOL_WHITELIST:?}（协议必须是冻结且极小的）"
    );

    let dev = dep_names(&doc, "dev-dependencies");
    assert!(
        !dev.contains("px_render"),
        "px_protocol 的 dev 依赖里不许有 px_render：那会把 wgpu / naga / winit 编进协议测试。\
         要按真类型构造跨进程形状就用本 crate 自己那份。今天的 dev 依赖：{dev:?}"
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
    let mut scanned: BTreeSet<String> = BTreeSet::new();
    for path in &manifests {
        let doc = manifest(path);
        let name = package_name(&doc, "?");
        scanned.insert(name.clone());
        for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
            let deps = dep_names(&doc, section);
            for (from, to, _) in FORBIDDEN_EDGES {
                assert!(
                    !(name == from && deps.contains(to)),
                    "{name} 在 [{section}] 里依赖了 {to}：渲染器与 sim 只能通过 px_protocol 通信"
                );
            }
        }
        checked += 1;
    }

    for (from, _, subject) in FORBIDDEN_EDGES {
        assert!(
            !matches!(subject, Subject::Present) || scanned.contains(from),
            "禁止边 ({from}, …) 声明了 Subject::Present，但 workspace 里扫不到包 `{from}` \
             —— 它被改名或删掉了？这条边现在**没有主体**，也就是没有人在守。\
             真删了就把它改成 Subject::Reserved（明账）或换掉主体（§157 的先例）。\
             扫到的包：{scanned:?}"
        );
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
        members
            .iter()
            .any(|item| item.as_str() == Some("px_protocol")),
        "px_protocol 必须是 workspace 成员，否则它不会被任何门覆盖"
    );
}
