//! 卫星 / 无大气天体：**基础地形 + 三层陨坑** ⇒ 高度场 + 立方球网格。
//!
//! 与 `planet.rs` / `desert.rs` 同形（普通 Rust，每一步走 `px_cook::cached` 的缓存函数），
//! 差别只在词汇：三层都是 **`field.stamps`**（盖章式打坑）—— 每枚印章有自己的随机半径与
//! 年龄，按年龄序**挖掘**（年轻坑挖掉老坑的坑缘），密度由上游场当遮罩。
//! 球面档按 `direction` 取格点（没有接缝、两极不挤）。
//!
//! ⚠ 三层坑**不是一个节点里的三个 octave**：三个节点各自的 TOML 给频率 / 半径分布 / 深度
//!   ⇒ 大盆地、中坑、小坑可以分开调，而且调一层只重算它自己与下游（上游照命中）。
//!   这正是"节点 = 一次算子调用 + 一份参数"该有的粒度。
//! ⚠ 上游那张场（`terra` 的 fbm）**同时是密度遮罩**：`mask_lo..mask_hi` 那一栏把"这里该不该
//!   长坑"交给图去表达 —— 今天的参数是"亮处长、暗处稀"，换一份上游就换一套地貌。
//!
//! 渲染：`px run scene orbit-moon`（配方在 `art/scene/orbit-moon.toml`）。

use px_cook::{
    Domain, GraphSpec, artifact_path_of, begin, cached, field, field_params, mesh, node_params,
};

type Fault = Box<dyn std::error::Error>;

fn main() -> Result<(), Fault> {
    let graph = begin(GraphSpec {
        name: "moon".to_string(),
    });

    let shape = field_params::Shape {
        width: 780,
        height: 520,
        projection: Domain::Cube,
    };

    // 基础地形：比行星更平（卫星没有板块运动，起伏靠撞击）。
    let terra = cached(
        &graph,
        "terra",
        field::Fbm,
        field_params::fbm::Params {
            shape,
            ..node_params(&graph, "terra")?
        },
        (),
    )?;
    // 三层印章：`base` 是"被打的那张场"（也当密度遮罩），每一层往上面挖。
    let basins = cached(
        &graph,
        "basins",
        field::Stamps,
        node_params(&graph, "basins")?,
        field::CratersInput {
            base: terra.clone(),
        },
    )?;
    let craters = cached(
        &graph,
        "craters",
        field::Stamps,
        node_params(&graph, "craters")?,
        field::CratersInput {
            base: basins.clone(),
        },
    )?;
    let pits = cached(
        &graph,
        "pits",
        field::Stamps,
        node_params(&graph, "pits")?,
        field::CratersInput {
            base: craters.clone(),
        },
    )?;
    // 收口：把高度压回 [0,1]（`Craters` 不钳制 —— 值域是图的事）。
    let height = cached(
        &graph,
        "height",
        field::Remap,
        node_params(&graph, "height")?,
        field::FieldInput {
            field: pits.clone(),
        },
    )?;
    let surface = cached(
        &graph,
        "surface",
        mesh::CubeSphere,
        node_params(&graph, "surface")?,
        mesh::CubeSphereInput {
            height: height.clone(),
        },
    )?;

    let stats = height.value().stats();
    println!(
        "输出 height：{}×{}，值域 {:.4}..{:.4}，均值 {:.4}",
        height.value().width,
        height.value().height,
        stats.min,
        stats.max,
        stats.mean,
    );
    let raw = pits.value().stats();
    println!(
        "  （打坑之后、收口之前：值域 {:.4}..{:.4} —— 越出 [0,1] 是正常的：`field.stamps` 不钳制）",
        raw.min, raw.max,
    );
    println!(
        "输出 surface：{} 顶点 / {} 三角形",
        surface.value().vertices(),
        surface.value().triangles()
    );
    println!(
        "看这一份内容：先 `px run scene orbit-moon`（配方 art/scene/orbit-moon.toml），\
         再 `target/debug/px_render.exe --offline --scene <产物> --out x.png --width 960 --height 640`"
    );
    println!(
        "  这一趟烘的成员：height {}｜surface {}",
        artifact_path_of(&height.key).display(),
        artifact_path_of(&surface.key).display(),
    );

    graph.finish();
    Ok(())
}
