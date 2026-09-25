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
    px_graphs::insts::gate("moon", &["px_field_op", "px_mesh_op"])?;
    px_cook::apply_store_args()?;
    let graph = begin(GraphSpec {
        name: "moon".to_string(),
    });

    let shape = field_params::Shape {
        width: 780,
        height: 520,
        projection: Domain::Cube,
    };

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
    let height = cached(
        &graph,
        "height",
        elem::Remap,
        node_params(&graph, "height")?,
        elem::RemapInput {
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

    graph.finish()?;
    Ok(())
}
