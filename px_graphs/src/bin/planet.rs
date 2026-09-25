use px_cook::{
    Domain, GraphSpec, artifact_path_of, begin, cached, field, field_params, mesh, node_params,
};
use px_graphs::elem;

type Fault = Box<dyn std::error::Error>;

fn main() -> Result<(), Fault> {
    px_graphs::insts::gate("planet").expect("实例库不齐 ⇒ 先 `px build`（stage 1 的正规命令）");
    px_cook::apply_store_args()?;
    let graph = begin(GraphSpec {
        name: "planet".to_string(),
    });

    let shape = field_params::Shape {
        width: 780,
        height: 520,
        projection: Domain::Cube,
    };

    let continents = cached(
        &graph,
        "continents",
        field::Fbm,
        field_params::fbm::Params {
            shape,
            ..node_params(&graph, "continents")?
        },
        (),
    )?;
    let mountains = cached(
        &graph,
        "mountains",
        field::Ridged,
        field_params::ridged::Params {
            shape,
            ..node_params(&graph, "mountains")?
        },
        (),
    )?;
    let weight = cached(
        &graph,
        "weight",
        elem::Constant,
        elem::ConstantParams {
            shape,
            ..node_params(&graph, "weight")?
        },
        (),
    )?;
    let terrain = cached(
        &graph,
        "terrain",
        elem::Mix,
        node_params(&graph, "terrain")?,
        elem::MixInput {
            a: continents,
            b: mountains,
            mask: weight,
        },
    )?;
    let height = cached(
        &graph,
        "height",
        elem::Remap,
        node_params(&graph, "height")?,
        elem::RemapInput {
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
