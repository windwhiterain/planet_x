use px_ops::field::cube_map_extent;
use px_ops::noise::fnv1a;
use px_ops::ops;
use px_ops::{GraphSpec, begin, finish, node};

const GRAPH_VERSION: u32 = 1;
const SOURCE_HASH: u64 = fnv1a(include_str!("clouds.rs"));
const FACE: u32 = 256;

fn main() {
    let (width, height) = cube_map_extent(FACE);
    begin(GraphSpec {
        name: "clouds".to_string(),
        version: GRAPH_VERSION,
        source_hash: SOURCE_HASH,
        width,
        height,
        projection: px_ops::field::Projection::CubeMap,
    });

    let clusters = node::<ops::fbm::Fbm>("clusters", &[]);
    let billows = node::<ops::fbm::Fbm>("billows", &[]);
    let flow = node::<ops::fbm::Fbm>("flow", &[]);
    let carved = node::<ops::warp::Warp>("carved", &[&billows, &flow]);
    let weight = node::<ops::constant::Constant>("weight", &[]);
    let mixed = node::<ops::mix::Mix>("mixed", &[&clusters, &carved, &weight]);
    let coverage = node::<ops::remap::Remap>("coverage", &[&mixed]);

    let stats = coverage.field().stats();
    println!(
        "输出 coverage：{}×{}（{} 面 × {face}²）｜值域 {:.4}..{:.4}｜均值 {:.4}",
        coverage.field().width,
        coverage.field().height,
        height / FACE,
        stats.min,
        stats.max,
        stats.mean,
        face = FACE,
    );

    println!(
        "渲染：px_render --planet <HEIGHT.pxart> --mesh <MESH.pxart> --palette rocky --clouds {}",
        px_ops::artifact_path_of(&coverage.key).display(),
    );

    finish();
}
