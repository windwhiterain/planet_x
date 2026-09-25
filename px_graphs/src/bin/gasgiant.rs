use px_cook::{Domain, GraphSpec, begin, cached, field, field_params, node_params};
use px_field_schema::field::cube_map_extent;

use px_graphs::elem;
use px_graphs::insts::LatBands;

const FACE: u32 = 256;

type Fault = Box<dyn std::error::Error>;

fn main() -> Result<(), Fault> {
    // 这张图任何分支都可能 `cached` 到的域算子（推导自本文件的 `cached` 调用，
    // 与 schema 每条声明的 `LIB` 对齐），逐条走 `source_hash` 全握手；
    // 名单之外的库回退到首个节点的握手拒绝，与今天等价、不会更糟。
    px_graphs::insts::gate("gasgiant", &["px_field_op"])
        .expect("实例库不齐 ⇒ 先 `px build`（stage 1 的正规命令）");
    px_cook::apply_store_args()?;
    let (width, height) = cube_map_extent(FACE);
    let graph = begin(GraphSpec {
        name: "gasgiant".to_string(),
    });

    let shape = field_params::Shape {
        width,
        height,
        projection: Domain::CubeMap,
    };

    let turbulence = cached(
        &graph,
        "turbulence",
        field::Fbm,
        field_params::fbm::Params {
            shape,
            ..node_params(&graph, "turbulence")?
        },
        (),
    )?;
    let bands = cached(
        &graph,
        "bands",
        LatBands,
        node_params(&graph, "bands")?,
        field::FieldRemapInput {
            input: turbulence.clone(),
        },
    )?;
    let swirl = cached(
        &graph,
        "swirl",
        field::Fbm,
        field_params::fbm::Params {
            shape,
            ..node_params(&graph, "swirl")?
        },
        (),
    )?;
    let warped = cached(
        &graph,
        "warped",
        field::Warp,
        node_params(&graph, "warped")?,
        field::FieldPairInput {
            field: bands,
            offset: swirl.clone(),
        },
    )?;
    let spots = cached(
        &graph,
        "spots",
        field::Fbm,
        field_params::fbm::Params {
            shape,
            ..node_params(&graph, "spots")?
        },
        (),
    )?;
    let eddied = cached(
        &graph,
        "eddied",
        field::Warp,
        node_params(&graph, "eddied")?,
        field::FieldPairInput {
            field: warped,
            offset: spots,
        },
    )?;
    let filaments_raw = cached(
        &graph,
        "filaments_raw",
        field::Fbm,
        field_params::fbm::Params {
            shape,
            ..node_params(&graph, "filaments_raw")?
        },
        (),
    )?;
    let _filaments = cached(
        &graph,
        "filaments",
        field::Warp,
        node_params(&graph, "filaments")?,
        field::FieldPairInput {
            field: filaments_raw,
            offset: swirl,
        },
    )?;
    let mixed = cached(
        &graph,
        "mixed",
        elem::Remap,
        node_params(&graph, "mixed")?,
        elem::RemapInput {
            field: eddied.clone(),
        },
    )?;

    let stats = mixed.value().stats();
    println!(
        "输出 mixed：{}×{}（{} 面 × {FACE}²）｜值域 {:.4}..{:.4}｜均值 {:.4}",
        mixed.value().width,
        mixed.value().height,
        mixed.value().height / FACE,
        stats.min,
        stats.max,
        stats.mean,
    );
    println!(
        "  ⚠ 这张图**不产云**：它的消费者是次表面散射材质（`art/shaders/gasgiant.wgsl`）—— \
         场 → 立方贴图（`generate::field_cube`）→ 材质 @binding(7)"
    );
    println!("看这一份内容：px run scene orbit-gasgiant（配方 art/scene/orbit-gasgiant.toml）");

    graph.finish();
    Ok(())
}
