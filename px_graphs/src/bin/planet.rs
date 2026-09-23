//! 星球：**普通 Rust** —— 一张场接一张场地算下去，每一步走 `px_cook::cached` 那个缓存函数。
//!
//! ⚠ 参数是**普通 Rust 值**：`node_params` 从 `art/planet/<节点>.toml` 读一份打底，
//!   想改哪个字段就在 Rust 里改哪个（`Params { frequency: x, ..node_params(..)? }`）。
//! ⚠ 这里原来是老写法（`node(params::FBM, "continents", &[])`：字符串 id + 一个
//! `&[&Artifact]` 字节边界，类型全擦除）。现在接错一个输入、少给一个上游都是**编译错**。

use px_cook::{
    Domain, GraphSpec, artifact_path_of, begin, cached, cameras, field, mesh, node_params,
};

type Fault = Box<dyn std::error::Error>;

fn main() -> Result<(), Fault> {
    let graph = begin(GraphSpec {
        name: "planet".to_string(),
        width: 780,
        height: 520,
        projection: Domain::Cube,
        cameras: cameras::review(),
    });

    let continents = cached(
        &graph,
        "continents",
        field::Fbm,
        node_params(&graph, "continents")?,
        (),
    )?;
    let mountains = cached(
        &graph,
        "mountains",
        field::Ridged,
        node_params(&graph, "mountains")?,
        (),
    )?;
    let weight = cached(
        &graph,
        "weight",
        field::Constant,
        node_params(&graph, "weight")?,
        (),
    )?;
    let terrain = cached(
        &graph,
        "terrain",
        field::Mix,
        node_params(&graph, "terrain")?,
        field::MixInput {
            a: continents,
            b: mountains,
            mask: weight,
        },
    )?;
    let height = cached(
        &graph,
        "height",
        field::Remap,
        node_params(&graph, "height")?,
        field::FieldInput {
            field: terrain.clone(),
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
