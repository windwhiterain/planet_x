//! **图侧现写算子**那一档的真样本：泛型实例落在图程序里，不进任何实现库。
//!
//! 这一篇守的是目标里那句「agent 在图侧现写泛型算子」，量三件事：
//!
//! 1. 现写的算子**能算**、能落盘、再跑一次能命中（`cook` 那条路对它一视同仁）；
//! 2. 它的**身份是本图程序**这一份源码（不是某个 dylib 的）；
//! 3. 同一个图程序里两个现写的算子**互不相同**（id 进键）。
//!
//! ⚠ 它**不碰** `art/` 下任何既有图：用的是自己的图名（缓存落在 `target/pcg/local-op/`）
//! ⇒ "加一个图侧算子"这件事动不到六份冻产物的字节。
//! ⚠ 「参数改了键就变」那条性质**不在这里验**：参数只从 `art/<图>/<节点>.toml` 来，
//!   而这一篇不该往 `art/` 写文件。那条性质由 `px_graph/tests/keys.rs`（规范 JSON 进键）
//!   与三张真图的产物判据看着。

use px_cook::{Domain, Graph, GraphSpec, begin, cook, px_local_op};
use px_field_schema::field::{Field, GridField};
use px_graph_schema::{Grid, PxOp};
use serde::{Deserialize, Serialize};

/// 现写算子的超参数 —— 图侧自己定义（`PxKeyed` 由 derive 生成 ⇒ 加字段自动进键）。
#[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
#[serde(default, deny_unknown_fields)]
struct BandParams {
    frequency: f32,
    gain: f32,
    axis: u32,
}

impl Default for BandParams {
    fn default() -> Self {
        Self {
            frequency: 6.0,
            gain: 1.0,
            axis: 2,
        }
    }
}

/// **图侧的泛型实现**：这个函数对 `F` 单态化，而 `F` 由调用方（图脚本）给。
///
/// ⚠ 它就是"泛型算子"那一档的全部机关：泛型参数是**一段现写的代码**，
///   实例化发生在图程序里 —— 实现库不参与，也不需要知道 `F` 是什么。
fn bake<F: Fn([f32; 3]) -> f32>(grid: Grid, sample: F) -> Field {
    let mut field = grid.filled(0.0);
    for y in 0..grid.height {
        for x in 0..grid.width {
            field.set(x, y, sample(field.direction(x, y)));
        }
    }
    field
}

/// 一个只住在本图程序里的算子：沿参数给的那根轴起波带。
struct Band;

px_local_op! { Band, "local.band", BandParams, (), Field, |p, _i, g| bake(g, |d| {
    // ⚠ 这条闭包捕捉的是**参数**：实例化在图侧，实现库里没有它的任何痕迹。
    let axis = (p.axis as usize).min(2);
    ((d[axis] * p.frequency).sin() * 0.5 + 0.5) * p.gain
}) }

/// 第二个现写的算子：与 [`Band`] **共用** `bake` 那个泛型函数，但闭包不同
/// ⇒ 图程序里有两个 `bake` 的单态化实例。
struct Rings;

px_local_op! { Rings, "local.rings", BandParams, (), Field, |p, _i, g| bake(g, |d| {
    let radial = (d[0] * d[0] + d[1] * d[1]).sqrt();
    ((radial * p.frequency).cos() * 0.5 + 0.5) * p.gain
}) }

fn graph() -> Graph {
    begin(GraphSpec {
        name: "local-op".to_string(),
        // 小画布：这一篇判的是机制，不是数值。
        width: 8,
        height: 4,
        projection: Domain::Cube,
        cameras: Vec::new(),
    })
}

/// ⚠ 三组断言合在**一个**测试里：一个测试二进制里的多个测试是并行的，而它们会写
/// 同一个 `target/pcg/local-op/manifest.json` —— 分开写会变成一场竞态。
///
/// ⚠ 断言**不许依赖缓存是空的**（测试会重跑，而 CAS 是持久的）：所以下面判的是
/// "同一份键稳定 ⇒ 第二次必命中"与"闭包真的被用上"，不是"第一次一定重算"。
#[test]
fn a_graph_local_operator_is_a_first_class_operator() {
    // 0) 泛型实现本身：**同一份泛型函数、两个闭包 ⇒ 两个单态化实例**（这是"图侧泛型"的定义）。
    let region = GraphSpec {
        name: "local-op".to_string(),
        width: 8,
        height: 4,
        projection: Domain::Cube,
        cameras: Vec::new(),
    };
    let grid = Grid {
        width: region.width,
        height: region.height,
        projection: region.projection,
    };
    let one = bake(grid, |_| 1.0).stats().mean;
    let two = bake(grid, |_| 2.0).stats().mean;
    assert!(
        (one - 1.0).abs() < 1e-6 && (two - 2.0).abs() < 1e-6,
        "同一份泛型函数对两个闭包算出 {one:.4} / {two:.4} ⇒ 泛型实例没被真的分开"
    );

    let graph = graph();

    // 1) 能算、能落盘、值不是常数。
    let band = cook::<Band>(&graph, "band", ()).expect("现写的算子应当能算");
    let value = band.value();
    assert_eq!((value.width, value.height), (8, 4));
    let stats = value.stats();
    println!(
        "图侧算子：{}×{}｜值域 {:.4}..{:.4}｜均值 {:.4}（命中={}）",
        value.width, value.height, stats.min, stats.max, stats.mean, band.hit
    );
    assert!(
        stats.max - stats.min > 1e-3,
        "现写的算子算出了常数场 ⇒ 闭包没被真的调用"
    );

    // 2) 身份 = **本图程序**这一份源码。
    let hash = Band::source_hash().expect("图侧算子应当有身份");
    assert!(Band::LIB.is_empty(), "图侧算子不该宣称自己住在一个实现库里");
    assert_eq!(
        hash,
        px_graphs::SOURCE_HASH,
        "图侧算子的身份必须是本图程序这一份源码的指纹"
    );
    assert_eq!(hash.len(), 64);

    // 3) 键稳定（第二次必命中）、两个现写算子互不相同（id 进键）。
    let rings = cook::<Rings>(&graph, "rings", ()).expect("第二个现写算子");
    assert_ne!(band.key, rings.key, "两个现写算子算出同一个键 ⇒ id 没进键");
    let again = cook::<Band>(&graph, "band", ()).expect("第二次");
    assert!(
        again.hit,
        "同一个节点再算一次没命中 ⇒ 键不稳定（现写算子的身份没钉住）"
    );
    assert_eq!(band.key, again.key);

    graph.finish();
}
