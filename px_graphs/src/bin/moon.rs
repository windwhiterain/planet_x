//! 卫星 / 无大气天体：**基础地形 + 三层陨坑** ⇒ 高度场 + 立方球网格。
//!
//! 与 `planet.rs` / `desert.rs` 同形（普通 Rust，每一步走 `px_cook` 的缓存辅助函数），
//! 差别只在词汇：**`field.craters`**（这一轮新加的算子）在一张地形场上打坑 ——
//! 坑里凹、坑缘凸，球面档按 `direction` 取格点（没有接缝、两极不挤）。
//!
//! ⚠ 三层坑**不是一个节点里的三个 octave**：三个节点各自的 TOML 给频率 / 半径 / 深度
//!   ⇒ 大盆地、中坑、小坑可以分开调，而且调一层只重算它自己与下游（上游照命中）。
//!   这正是"节点 = 一次算子调用 + 一份参数"该有的粒度。
//!
//! 渲染：`px run scene orbit-moon`（配方在 `art/scene/orbit-moon.toml`）。

use px_cook::{Domain, GraphSpec, artifact_path_of, begin, cameras, cook, field, mesh};

type Fault = Box<dyn std::error::Error>;

fn main() -> Result<(), Fault> {
    let graph = begin(GraphSpec {
        name: "moon".to_string(),
        width: 780,
        height: 520,
        projection: Domain::Cube,
        cameras: cameras::review(),
    });

    // 基础地形：比行星更平（卫星没有板块运动，起伏靠撞击）。
    let terra = cook::<field::Fbm>(&graph, "terra", ())?;
    // 三层坑：`base` 是"被打的那张场"，每一层把剖面**加**上去。
    let basins = cook::<field::Craters>(
        &graph,
        "basins",
        field::CratersInput {
            base: terra.clone(),
        },
    )?;
    let craters = cook::<field::Craters>(
        &graph,
        "craters",
        field::CratersInput {
            base: basins.clone(),
        },
    )?;
    let pits = cook::<field::Craters>(
        &graph,
        "pits",
        field::CratersInput {
            base: craters.clone(),
        },
    )?;
    // 收口：把高度压回 [0,1]（`Craters` 不钳制 —— 值域是图的事）。
    let height = cook::<field::Remap>(
        &graph,
        "height",
        field::FieldInput {
            field: pits.clone(),
        },
    )?;
    let surface = cook::<mesh::CubeSphere>(
        &graph,
        "surface",
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
        "  （打坑之后、收口之前：值域 {:.4}..{:.4} —— 越出 [0,1] 是正常的：`field.craters` 不钳制）",
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
