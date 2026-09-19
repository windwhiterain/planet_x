use px_ops::noise::fnv1a;
use px_ops::ops;
use px_ops::{GraphSpec, begin, finish, node};

const GRAPH_VERSION: u32 = 2;
const SOURCE_HASH: u64 = fnv1a(include_str!("desert.rs"));

fn main() {
    begin(GraphSpec {
        name: "desert".to_string(),
        version: GRAPH_VERSION,
        source_hash: SOURCE_HASH,
        width: 780,
        height: 520,
        projection: px_ops::field::Projection::Cube,
        cameras: px_ops::cameras::review(),
    });

    let plateaus = node::<ops::fbm::Fbm>("plateaus", &[]);
    let canyons = node::<ops::ridged::Ridged>("canyons", &[]);
    let flow = node::<ops::fbm::Fbm>("flow", &[]);
    let carved = node::<ops::warp::Warp>("carved", &[&canyons, &flow]);
    let blend = node::<ops::constant::Constant>("blend", &[]);
    let terrain = node::<ops::mix::Mix>("terrain", &[&plateaus, &carved, &blend]);
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

    // ⚠ S8-a：同 `planet.rs` —— bevy 宿主删了，提示改成新宿主**真跑得起来**的那条命令。
    println!(
        "看这一份内容：先 `cargo run -q -p px_graphs --bin scene <档>` 出场景文档（配方在 art/scene/），\
         再 `cargo run -q -p px_render -- --offline --scene <产物> --out x.png --width 960 --height 640`"
    );
    println!(
        "  这一趟烘的成员（要在 art/scene/ 里自己接上）：height {}｜surface {}",
        px_ops::artifact_path_of(&height.key).display(),
        px_ops::artifact_path_of(&surface.key).display(),
    );

    finish();
}




