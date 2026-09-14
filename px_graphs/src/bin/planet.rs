use px_ops::noise::fnv1a;
use px_ops::ops;
use px_ops::{GraphSpec, begin, finish, node};

const GRAPH_VERSION: u32 = 1;
const SOURCE_HASH: u64 = fnv1a(include_str!("planet.rs"));

fn main() {
    begin(GraphSpec {
        name: "planet".to_string(),
        version: GRAPH_VERSION,
        source_hash: SOURCE_HASH,
        width: 780,
        height: 520,
        projection: px_ops::field::Projection::Cube,
        cameras: px_ops::cameras::review(),
    });

    let continents = node::<ops::fbm::Fbm>("continents", &[]);
    let mountains = node::<ops::ridged::Ridged>("mountains", &[]);
    let weight = node::<ops::constant::Constant>("weight", &[]);
    let terrain = node::<ops::mix::Mix>("terrain", &[&continents, &mountains, &weight]);
    let height = node::<ops::remap::Remap>("height", &[&terrain]);

    let surface = px_ops::mesh_node::<ops::cubesphere::CubeSphere>("surface", &[&height]);

    let stats = height.field().stats();
    println!(
        "输出 height：{}×{}，值域 {:.4}..{:.4}，均值 {:.4}",
        height.field().width, height.field().height, stats.min, stats.max, stats.mean,
    );
    println!(
        "输出 surface：{} 顶点 / {} 三角形",
        surface.mesh().vertices(),
        surface.mesh().triangles()
    );

    println!(
        "渲染：px_render --planet {} --mesh {} --palette rocky",
        px_ops::artifact_path_of(&height.key).display(),
        px_ops::artifact_path_of(&surface.key).display(),
    );

    finish();
}



