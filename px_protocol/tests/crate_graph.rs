use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const PROTOCOL_WHITELIST: [&str; 3] = ["serde", "serde_json", "px-handshake"];
// ⚠ `px-handshake` 为什么在白名单里，而不是"协议又变胖了"：握手身份（`ProtocolId` /
// `SCHEMA_VERSION`）与线格式（`wire`）**宿主也要**，而宿主本来就依赖协议 ⇒ 它们留在这里
// 就是 `px_protocol ⇄ px_host_protocol` 的环。放进叶子 crate 之后依赖图是**无环**的。
// ⚠ 让步的那一格由**另一条断言**补上（本测试末尾）：`px-handshake` 自己的运行时依赖
// 只许 serde / serde_json —— 否则"极小"这条要求会从这一格偷偷漏出去。
// ⚠ S8-c 标注（当时）：`px_render` 已在 §154 删掉 ⇒ 第一条禁止边**当时没有主体**（扫不到那个包名）。
// 它守的是"**渲染宿主不许拖 sim**"，而那时宿主叫 `px_render_wgpu` —— 那个名字**不在这张表里**
// ⇒ 那一格当时**没人在守**。原句留着是因为它是契约的记录。
//
// ⚠ **§157 取代（2026-09-19，同名改名那一笔）：wgpu 宿主改名叫 `px_render`** ⇒ 这条边
// **重新有主体**（扫到的就是本仓唯一那支宿主）。怎么验的（不是"名字看着对"）：
//   ① 绿：本测试过 —— 扫描集里有 `package.name == "px_render"` 的那份 manifest，且它的
//      `[dependencies]` 里没有 `px_sim`；
//   ② 红（负对照）：把这条边临时改成 `("px_render", "px_pass")`（宿主真有的依赖）⇒ 断言当场响，
//      报的是 `px_render 在 [dependencies] 里依赖了 px_pass` ⇒ **证明那个包名确实进了扫描**
//      （没有主体的边是不会响的）。负对照跑完即恢复，读数记在 `15-render-wgpu.md` §157。
// ⚠ 剩下的空档：第二条边 `px_web` 今天**也没有主体**（那个包还不存在）—— 本表是
// "现在 + 将来"两种边混装。
//
// ⚠ **§157 之后取代上面那句"不许加'每条边都必须有主体'的断言"**（本条由评审裁，原句留着是记录）：
// 那句的本意是"别把 `px_web` 判红"，但它连**真正该响的那一格**也一起放过了 ——
// `px_render` 丢掉主体时静默了整整一档，而那正是这条表存在的理由。
// ⇒ 正确的分法不是"要不要主体"，是**"这条边今天该不该有主体"**：
// `Present` = 该有 ⇒ 扫不到就红；`Reserved` = 还没有这个包 ⇒ 扫不到正常，但写在表里就是明账。
enum Subject {
    /// 这个包**今天就在** workspace 里 ⇒ **必须扫得到**，扫不到说明它被改名/删掉了，当场红。
    Present,
    /// 这个包**还不存在**（"将来"那一档）⇒ 扫不到是正常的 —— 但它是明账，不是遗忘。
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

/// 运行时那一格只许 serde / serde_json / px-handshake（协议是冻结且极小的）。
///
/// ⚠ **dev 那一格不许有 `px_render`**：宿主是 wgpu 栈，而 `px_protocol` 的测试要按**真类型**
/// 构造快照里那几份形状 —— 图省事把宿主挂到 dev（曾经就是这么写的）会把 wgpu / naga / winit
/// 整栈编进协议测试，dev 边 `px_protocol` → `px_render` → `px_protocol` 还把**方向**整个反过来。
/// 形状该住哪就住哪：`px_host_protocol`（闭包里没有 GPU）。
#[test]
fn protocol_dependencies_are_whitelisted_and_never_pull_the_host() {
    let doc = manifest(&Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"));
    let names = dep_names(&doc, "dependencies");
    let allowed: BTreeSet<String> = PROTOCOL_WHITELIST.iter().map(|name| name.to_string()).collect();
    assert_eq!(
        names, allowed,
        "px_protocol 的运行时依赖只允许 {PROTOCOL_WHITELIST:?}（协议必须是冻结且极小的）"
    );

    let dev = dep_names(&doc, "dev-dependencies");
    assert!(
        !dev.contains("px_render"),
        "px_protocol 的 dev 依赖里不许有 px_render：那会把 wgpu / naga / winit 编进协议测试，\
         并且把依赖方向反过来（px_protocol → px_render → px_protocol）。\
         要按真类型构造宿主形状就用不带 GPU 栈的 px_host_protocol。今天的 dev 依赖：{dev:?}"
    );
}

/// 白名单放宽给 `px-handshake` 之后，**"极小"这条要求要跟着往下走一格**：
/// 那个叶子 crate 自己的运行时依赖只许 serde / serde_json。
///
/// ⚠ 少了这条，白名单就等于把"协议可以依赖任何东西"从旁门放了进来 —— 叶子只是**搬了一格**，
/// 不是免检。而它必须依赖图**无环**：`px-host-handshake` 不许反向依赖本仓任何 crate。
#[test]
fn the_handshake_leaf_stays_minimal_and_acyclic() {
    let root = root();
    let doc = manifest(&root.join("px_handshake/Cargo.toml"));
    let names = dep_names(&doc, "dependencies");
    let allowed: BTreeSet<String> = ["serde", "serde_json"]
        .iter()
        .map(|name| name.to_string())
        .collect();
    assert_eq!(
        names, allowed,
        "px-handshake 的运行时依赖只许 serde / serde_json：它是握手身份与线格式的叶子，\
         多一条边就等于协议那一格又从旁门长回来了"
    );
    assert_eq!(
        package_name(&doc, "px_handshake"),
        "px-handshake",
        "包名（连字符）与依赖键要对得上，否则上面那格查的是另一份 manifest"
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

    // ⚠ 替我们盯着"一条边悄悄丢了主体"：`Present` 却扫不到 ⇒ 当场红。
    // §156 的教训：`px_render` 被删之后，那条边**留在表里、永远扫不到东西、也永远不会响**
    // ⇒ "渲染宿主不许拖 sim"静默地没人守了。一条**没有主体的门**看起来与一条**守得住的门**
    // 长得一模一样，唯一的区别就是这里多问一句。
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
        members.iter().any(|item| item.as_str() == Some("px_protocol")),
        "px_protocol 必须是 workspace 成员，否则它不会被任何门覆盖"
    );
}
