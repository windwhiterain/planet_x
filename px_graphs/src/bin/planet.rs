use px_field_schema::params;
use px_graph::{GraphSpec, begin, finish, node};
use px_mesh_schema::params as mesh_params;
use px_protocol::art::Domain;

const GRAPH_VERSION: u32 = 1;
const SOURCE_HASH: u64 = px_graph::fnv1a(include_str!("planet.rs"));

fn main() {
    begin(GraphSpec {
        name: "planet".to_string(),
        version: GRAPH_VERSION,
        source_hash: SOURCE_HASH,
        width: 780,
        height: 520,
        projection: Domain::Cube,
        cameras: px_graph::cameras::review(),
    });

    let continents = node(params::FBM, "continents", &[]);
    let mountains = node(params::RIDGED, "mountains", &[]);
    let weight = node(params::CONSTANT, "weight", &[]);
    let terrain = node(params::MIX, "terrain", &[&continents, &mountains, &weight]);
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

    // ⚠ S8-a：这里原来打的是 bevy 宿主的 `--planet … --mesh … --palette rocky`。
    //   那支宿主已经删掉，本仓只剩 `px_render`，而它**只吃 `.pxart` 场景文档**
    //   ⇒ 打一条真跑得起来的命令（配场景的地方是 `art/scene/*.toml`），
    //   而不是留一句谁也无法执行的提示。⚠ 改这里会动本文件的 `SOURCE_HASH`，
    //   而它只用来打一句警告、不进任何产物键（`px_graph/src/driver.rs`）。
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
