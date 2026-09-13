use px_ops::field::cube_map_extent;
use px_ops::noise::fnv1a;
use px_ops::ops;
use px_ops::{GraphSpec, begin, finish, node};

const GRAPH_VERSION: u32 = 4;
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

    let slope_x = node::<ops::gradient::Gradient>("slope_x", &[&mixed]);
    let slope_y = node::<ops::gradient::Gradient>("slope_y", &[&mixed]);
    let slope_z = node::<ops::gradient::Gradient>("slope_z", &[&mixed]);

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
    let smooth = mixed.field().stats();
    let mut sorted: Vec<f32> = mixed.field().data.clone();
    sorted.sort_by(|one, two| one.partial_cmp(two).unwrap_or(std::cmp::Ordering::Equal));
    let share = |fraction: f64| sorted[((sorted.len() - 1) as f64 * fraction) as usize];
    println!(
        "输出 mixed：值域 {:.4}..{:.4}｜均值 {:.4}｜分位 50% {:.4}／80% {:.4}／88% {:.4}／95% {:.4}",
        smooth.min,
        smooth.max,
        smooth.mean,
        share(0.50),
        share(0.80),
        share(0.88),
        share(0.95),
    );
    for (name, node) in [
        ("slope_x", &slope_x),
        ("slope_y", &slope_y),
        ("slope_z", &slope_z),
    ] {
        let stats = node.field().stats();
        println!(
            "输出 {name}：值域 {:.4}..{:.4}｜均值 {:.4}",
            stats.min, stats.max, stats.mean
        );
    }

    println!(
        "渲染：px_render --planet <HEIGHT.pxart> --mesh <MESH.pxart> --palette rocky --clouds {} --cloud-slope {},{},{}",
        px_ops::artifact_path_of(&mixed.key).display(),
        px_ops::artifact_path_of(&slope_x.key).display(),
        px_ops::artifact_path_of(&slope_y.key).display(),
        px_ops::artifact_path_of(&slope_z.key).display(),
    );

    finish();
}
