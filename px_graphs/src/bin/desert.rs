use px_cook::{
    Domain, GraphSpec, artifact_path_of, begin, cached, field, field_params, mesh, node_params,
};
use px_graphs::elem;

type Fault = Box<dyn std::error::Error>;

fn main() {
    px_cook::fault::graph_main(run);
}

fn run() -> Result<(), Fault> {
    // 这张图任何分支都可能 `cached` 到的域算子（推导自本文件的 `cached` 调用，
    // 与 schema 每条声明的 `LIB` 对齐），逐条走 `source_hash` 全握手；
    // 名单之外的库回退到首个节点的握手拒绝，与今天等价、不会更糟。
    px_graphs::insts::gate("desert", &["px_field_op", "px_mesh_op"])?;
    px_cook::apply_store_args()?;
    let graph = begin(GraphSpec {
        name: "desert".to_string(),
    });

    let shape = field_params::Shape {
        width: 780,
        height: 520,
        projection: Domain::Cube,
    };

    let plateaus = cached(
        &graph,
        "plateaus",
        field::Fbm,
        field_params::fbm::Params {
            shape,
            ..node_params(&graph, "plateaus")?
        },
        (),
    )?;
    let canyons = cached(
        &graph,
        "canyons",
        field::Ridged,
        field_params::ridged::Params {
            shape,
            ..node_params(&graph, "canyons")?
        },
        (),
    )?;
    let flow = cached(
        &graph,
        "flow",
        field::Fbm,
        field_params::fbm::Params {
            shape,
            ..node_params(&graph, "flow")?
        },
        (),
    )?;
    let carved = cached(
        &graph,
        "carved",
        field::Warp,
        node_params(&graph, "carved")?,
        field::FieldPairInput {
            field: canyons,
            offset: flow,
        },
    )?;
    let blend = cached(
        &graph,
        "blend",
        elem::Constant,
        elem::ConstantParams {
            shape,
            ..node_params(&graph, "blend")?
        },
        (),
    )?;
    let terrain = cached(
        &graph,
        "terrain",
        elem::Mix,
        node_params(&graph, "terrain")?,
        elem::MixInput {
            a: plateaus,
            b: carved,
            mask: blend,
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
