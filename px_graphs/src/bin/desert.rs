//! 沙漠：**普通 Rust** —— 同 `planet.rs`，每一步走 `px_cook` 那个缓存辅助函数。
//!
//! ⚠ 这里原来是老写法（字符串 id + `&[&Artifact]`）。见 `planet.rs` 顶上那条注释。

use px_cook::{Domain, GraphSpec, artifact_path_of, begin, cameras, cook, field, mesh};

type Fault = Box<dyn std::error::Error>;

fn main() -> Result<(), Fault> {
    let graph = begin(GraphSpec {
        name: "desert".to_string(),
        width: 780,
        height: 520,
        projection: Domain::Cube,
        cameras: cameras::review(),
    });

    let plateaus = cook::<field::Fbm>(&graph, "plateaus", ())?;
    let canyons = cook::<field::Ridged>(&graph, "canyons", ())?;
    let flow = cook::<field::Fbm>(&graph, "flow", ())?;
    let carved = cook::<field::Warp>(
        &graph,
        "carved",
        field::FieldPairInput {
            field: canyons,
            offset: flow,
        },
    )?;
    let blend = cook::<field::Constant>(&graph, "blend", ())?;
    let terrain = cook::<field::Mix>(
        &graph,
        "terrain",
        field::MixInput {
            a: plateaus,
            b: carved,
            mask: blend,
        },
    )?;
    let height = cook::<field::Remap>(
        &graph,
        "height",
        field::FieldInput {
            field: terrain.clone(),
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

    println!(
        "输出 surface：{} 顶点 / {} 三角形",
        surface.value().vertices(),
        surface.value().triangles()
    );

    println!(
        "看这一份内容：先 `cargo run -q -p px_graphs --bin scene <档>` 出场景文档（配方在 art/scene/），\
         再 `cargo run -q -p px_render -- --offline --scene <产物> --out x.png --width 960 --height 640`"
    );
    println!(
        "  这一趟烘的成员（要在 art/scene/ 里自己接上）：height {}｜surface {}",
        artifact_path_of(&height.key).display(),
        artifact_path_of(&surface.key).display(),
    );

    graph.finish();
    Ok(())
}
