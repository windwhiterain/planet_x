//! **实例的端到端探针**：把一条实例库真装进来、真算一遍。
//!
//! 用法：`inst_probe [op_id]`（默认 `cloud.coarse/band` —— 那条体积域的 `Band`）。
//!
//! ⚠ **为什么这是一支探针、不是一条测试**（2026-09-20 从 `tests/inst_gate.rs` 搬出来的）：
//!   它要跑起来必须先有 `px build` 烘出来的实例库（`target/pcg/inst/<key>.dll`），而那是
//!   **构建产物**：`cargo test` 不保证它存在、也不该替它去编（测试里起 cargo 就是 20 分钟）。
//!   搬出来之前那一段在库里写着"**库不在盘上就打印一行、然后 return**"——
//!   那就是"跳过"，而本仓那条不变式是"**任何『跳过』都是判据的敌人**"：
//!   一条永远可能什么都不查的判据，比没有这条判据更坏（它给人一种查过了的错觉）。
//!   ⇒ 判据分成两件**各自都硬**的事：
//!     * `tests/inst_gate.rs`：**计划那一半**（recipe ↔ 生成物 ↔ 图，纯事实，不需要任何产物）；
//!     * 这一支：**运行那一半**（装载 + cook + 命中 + 键稳定），缺库就**当场报错并给出命令**。
//!
//! ⚠ 这条探针也是"实例库真能装载"这件事**唯一**的判据：`PxOp::LIB` 是空串、
//!   库按 key 在运行期 `dlopen`、符号按声明名拼 —— 三件事里任何一件错，图跑起来才炸。

use px_cook::inst::BuildGraph;
use px_cook::{Cooked, Domain, GraphSpec, begin, cook, volume};
use px_field_schema::field::Field;
use px_graph_schema::PxOp;
use px_graphs::insts::Band;

fn main() {
    let op_id = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "cloud.coarse/band".to_string());
    assert_eq!(
        op_id, "cloud.coarse/band",
        "今天的探针只认识 `cloud.coarse/band`（要加别的实例，照着下面那段抄一份即可）"
    );

    // 库在哪：从**同一张 build graph** 里找这条实例（与 `cargo build` 时 stage 1 看的是同一份计划）。
    let mut graph = BuildGraph::new();
    px_graphs::insts::build(&mut graph);
    let info = graph
        .into_nodes()
        .into_iter()
        .find(|node| node.op_id == op_id)
        .unwrap_or_else(|| panic!("build graph 里没有 `{op_id}` 这条实例"));
    let key = Band::source_hash().expect("图侧算得出实例 key");
    println!("实例 {op_id}｜key {key}｜库 {}", info.library);
    if !std::path::Path::new(&info.library).is_file() {
        panic!(
            "实例库不在盘上（{}）⇒ 先跑 `{} build`",
            info.library,
            px_cook::inst::driver_command(),
        );
    }
    assert_eq!(
        std::path::Path::new(&info.library)
            .file_stem()
            .and_then(|stem| stem.to_str()),
        Some(key),
        "`info_of_facts` 给的库路径与 `source_hash()`（实例 key）不是同一个 key"
    );

    // ⚠ 自己的图名（`inst-op`），不碰 `art/` 下任何既有图。
    let graph = begin(GraphSpec {
        name: "inst-op".to_string(),
        width: 8,
        height: 4,
        projection: Domain::Cube,
        cameras: Vec::new(),
    });
    // 上游那张覆盖度场：实例复用的是 `CloudCoarse` 的**声明** ⇒ 输入形状就是它那个
    // `CloudCoarseInput { coverage }`。⚠ `Band` 自己就是覆盖度的来源，这张场**不参与计算**，
    // 但接口要它在场（`19` §179.1：体逐字套在复用的声明上）。
    let coverage = Cooked::new(
        *px_cook::blake3::hash(b"inst-op/coverage").as_bytes(),
        Field::filled_with(8, 4, 1.0, Domain::Cube),
        false,
        0,
        0,
    );
    // `CloudCoarseInput` 不吃 `Clone` ⇒ 两条输入各建一次（同一份上游 ⇒ 同一个键）。
    let inputs = || volume::CloudCoarseInput {
        coverage: coverage.clone(),
    };
    let first = cook::<Band>(&graph, "band", inputs()).expect("实例算子应当能算");
    let again = cook::<Band>(&graph, "band", inputs()).expect("第二次");
    println!(
        "实例算子：{}×{}×{}｜命中={} → {}｜key {}",
        first.value().res,
        first.value().layers,
        first.value().data.len(),
        first.hit,
        again.hit,
        px_cook::hex_short(&first.key),
    );
    assert!(again.hit, "同一个节点再算一次没命中 ⇒ 实例 key 不稳定");
    assert_eq!(first.key, again.key);
    assert!(!first.value().data.is_empty(), "实例算子算出了一份空体积");
    graph.finish();
    println!("OK：装载、算、命中、键稳定四件事都过了");
}
