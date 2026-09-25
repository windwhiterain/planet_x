use px_cook::{Domain, GraphSpec, begin, cached, field, field_params, node_params};
use px_graph_schema::PxOp;

use px_graphs::insts::Waves;

type Fault = Box<dyn std::error::Error>;

fn main() {
    px_cook::fault::graph_main(run);
}

fn run() -> Result<(), Fault> {
    let graph = begin(GraphSpec {
        name: "field_remap".to_string(),
    });

    let shape = field_params::Shape {
        width: 256,
        height: 128,
        projection: Domain::Equirect,
    };

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
    assert!(
        stats.min >= 0.0 && stats.max <= 1.0,
        "场函数算出了 [0,1] 之外的值：{:.6}..{:.6} ⇒ `art/inst/waves.rs` 里那次 clamp 漏了",
        stats.min,
        stats.max,
    );

    graph.finish();
    Ok(())
}
