use px_cook::inst::BuildGraph;
use px_cook::{Cooked, Domain, GraphSpec, begin, cached, field_params, node_params, volume};
use px_graph_schema::PxOp;
use px_graphs::insts::Band;

fn main() {
    px_cook::fault::install_panic_hook();
    let op_id = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "cloud.coarse/band".to_string());
    assert_eq!(
        op_id, "cloud.coarse/band",
        "今天的探针只认识 `cloud.coarse/band`（要加别的实例，照着下面那段抄一份即可）"
    );

    let mut graph = BuildGraph::new();
    px_graphs::insts::build(&mut graph);
    let info = graph
        .into_nodes()
        .into_iter()
        .find(|node| node.op_id == op_id)
        .unwrap_or_else(|| panic!("build graph 里没有 `{op_id}` 这条实例"));
    let key = Band::source_hash().expect("图侧算得出实例 key");
    println!("实例 {op_id}｜key {key}｜库 {}", info.library);
    if !std::path::Path::new(&info.library).is_file() {
        panic!(
            "实例库不在盘上（{}）⇒ 先跑 `{} build`",
            info.library,
            px_cook::inst::driver_command(),
        );
    }
    assert_eq!(
        std::path::Path::new(&info.library)
            .file_stem()
            .and_then(|stem| stem.to_str()),
        Some(key),
        "`info_of_facts` 给的库路径与 `source_hash()`（实例 key）不是同一个 key"
    );

    let graph = begin(GraphSpec {
        name: "inst-op".to_string(),
    });

    let shape = field_params::Shape {
        width: 8,
        height: 4,
        projection: Domain::Cube,
    };

    let coverage = Cooked::new(
        *px_cook::blake3::hash(b"inst-op/coverage").as_bytes(),
        shape.filled(1.0),
        false,
        0,
        0,
    );
    let inputs = || volume::CloudCoarseInput {
        coverage: coverage.clone(),
    };
    let first = cached(
        &graph,
        "band",
        Band,
        node_params(&graph, "band").expect("参数（这个图没有 art/inst-op/band.toml ⇒ 走 Default）"),
        inputs(),
    )
    .expect("实例算子应当能算");
    let again = cached(
        &graph,
        "band",
        Band,
        node_params(&graph, "band").expect("参数"),
        inputs(),
    )
    .expect("第二次");
    println!(
        "实例算子：{}×{}×{}｜命中={} → {}｜key {}",
        first.value().res,
        first.value().layers,
        first.value().data.len(),
        first.hit,
        again.hit,
        px_cook::hex_short(&first.key),
    );
    assert!(again.hit, "同一个节点再算一次没命中 ⇒ 实例 key 不稳定");
    assert_eq!(first.key, again.key);
    assert!(!first.value().data.is_empty(), "实例算子算出了一份空体积");
    graph.finish();
    println!("OK：装载、算、命中、键稳定四件事都过了");
}
