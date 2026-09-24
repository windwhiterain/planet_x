//! **场域的泛型实例图**（`field.remap/waves`）：`field.fbm` → 图侧现写的 `Waves` → 一张场。
//!
//! 这张图的用处是**给"泛型实例"一条能跑的判据**（`docs/system/generic-instances.md` / `docs/system/build-graph.md`）：
//!
//! * `px run field_remap` —— 两个 stage 一条命令：全命中 ⇒ 只读装载、直接算；缺就报命令。
//! * `px run field_remap --build` —— 缺实例库时先跑 stage 1（真的起 cargo 编那一条）。
//! * 改 `art/inst/waves.rs` 里那个函数 ⇒ **只有 `bands` 这一个节点**（以及它的下游）重算，
//!   七个图 exe 一个字节不动；`px list` 里这条实例的 key 换一个。
//!
//! ⚠ **自己的图名**（`field_remap`）：参数目录是 `art/field_remap/`（今天不存在 ⇒ 全部走
//!   `Default`），产物在 `target/pcg/field_remap/`。它与 `art/{planet,desert,clouds}` 那些
//!   既有图的配方与产物**互不相干**（`inst_gate` 那类端到端小图也是这条规矩）。
//!
//! ⚠ 上游那张 `field.fbm` 走的是**预置算子**（`px_field_op` 那份 dylib，按身份装载），
//!   而 `bands` 走的是**实例库**（`target/pcg/inst/<key>.dll`）—— 一张图里两条装载路并存，
//!   这正是"声明住 schema、实现住 dylib、泛型参数住 art/inst"三样同时成立时该有的样子。

use px_cook::{Domain, GraphSpec, begin, cached, field, field_params, node_params};
use px_graph_schema::PxOp;

use px_graphs::insts::Waves;

/// 图侧对错误的统一态度：**当场失败**，不静默跳过。
type Fault = Box<dyn std::error::Error>;

fn main() -> Result<(), Fault> {
    let graph = begin(GraphSpec {
        name: "field_remap".to_string(),
    });

    let shape = field_params::Shape {
        width: 256,
        height: 128,
        projection: Domain::Equirect,
    };

    // 上游：预置的分形噪声（`px_field_op`，运行时按身份装载）。
    let source = cached(
        &graph,
        "source",
        field::Fbm,
        field_params::fbm::Params {
            shape,
            ..node_params(&graph, "source")?
        },
        (),
    )?;
    // 本图的主角：**图侧现写的泛型参数**（`art/inst/waves.rs`）经实例库算出来的那一张场。
    let bands = cached(
        &graph,
        "bands",
        Waves,
        node_params(&graph, "bands")?,
        field::FieldRemapInput {
            input: source.clone(),
        },
    )?;

    let source_stats = source.value().stats();
    let stats = bands.value().stats();
    println!(
        "输入 source：值域 {:.4}..{:.4}｜均值 {:.4}",
        source_stats.min, source_stats.max, source_stats.mean
    );
    println!(
        "输出 bands（{}）：{}×{}｜值域 {:.4}..{:.4}｜均值 {:.4}",
        Waves::ID,
        bands.value().width,
        bands.value().height,
        stats.min,
        stats.max,
        stats.mean,
    );
    // ⚠ 口径来自 `art/inst/waves.rs` 顶上那一条：**场函数自己保证落在 [0,1]**。
    //   越界即缺陷（`clamp` 写漏了、或 gain/bands 取到了没预料的值）⇒ 当场红。
    assert!(
        stats.min >= 0.0 && stats.max <= 1.0,
        "场函数算出了 [0,1] 之外的值：{:.6}..{:.6} ⇒ `art/inst/waves.rs` 里那次 clamp 漏了",
        stats.min,
        stats.max,
    );

    graph.finish();
    Ok(())
}
