use px_field_schema::params;
use px_graph::{GraphSpec, begin, finish, node};
use px_mesh_schema::params as mesh_params;
use px_protocol::art::Domain;


fn main() {
    begin(GraphSpec {
        name: "desert".to_string(),
        width: 780,
        height: 520,
        projection: Domain::Cube,
        cameras: px_graph::cameras::review(),
    });

    let plateaus = node(params::FBM, "plateaus", &[]);
    let canyons = node(params::RIDGED, "canyons", &[]);
    let flow = node(params::FBM, "flow", &[]);
    let carved = node(params::WARP, "carved", &[&canyons, &flow]);
    let blend = node(params::CONSTANT, "blend", &[]);
    let terrain = node(params::MIX, "terrain", &[&plateaus, &carved, &blend]);
    let height = node(params::REMAP, "height", &[&terrain]);

    let surface = node(mesh_params::CUBESPHERE, "surface", &[&height]);

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
        px_graph::artifact_path_of(&height.key).display(),
        px_graph::artifact_path_of(&surface.key).display(),
    );

    finish();
}
