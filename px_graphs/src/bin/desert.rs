use px_ops::noise::fnv1a;
use px_ops::ops;
use px_ops::{GraphSpec, begin, finish, node};

const GRAPH_VERSION: u32 = 1;
const SOURCE_HASH: u64 = fnv1a(include_str!("desert.rs"));

fn main() {
    begin(GraphSpec {
        name: "desert".to_string(),
        version: GRAPH_VERSION,
        source_hash: SOURCE_HASH,
        width: 768,
        height: 384,
        projection: px_ops::field::Projection::Equirect,
    });

    let plateaus = node::<ops::fbm::Fbm>("plateaus", &[]);
    let canyons = node::<ops::ridged::Ridged>("canyons", &[]);
    let flow = node::<ops::fbm::Fbm>("flow", &[]);
    let carved = node::<ops::warp::Warp>("carved", &[&canyons, &flow]);
    let blend = node::<ops::constant::Constant>("blend", &[]);
    let terrain = node::<ops::mix::Mix>("terrain", &[&plateaus, &carved, &blend]);
    let height = node::<ops::remap::Remap>("height", &[&terrain]);

    let stats = height.field.stats();
    println!(
        "输出 height：{}×{}，值域 {:.4}..{:.4}，均值 {:.4}",
        height.field.width, height.field.height, stats.min, stats.max, stats.mean,
    );

    finish();
}

