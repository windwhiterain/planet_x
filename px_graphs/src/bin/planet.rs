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

    // ⚠ S8-a：这里原来打的是 bevy 宿主的 `--planet … --mesh … --palette rocky`。
    //   那支宿主已经删掉，本仓只剩 `px_render_wgpu`，而它**只吃 `.pxart` 场景文档**
    //   ⇒ 打一条真跑得起来的命令（配场景的地方是 `art/scene/*.toml`），
    //   而不是留一句谁也无法执行的提示。⚠ 改这里会动本文件的 `SOURCE_HASH`，
    //   而它只用来打一句警告、不进任何产物键（`px_ops/src/lib.rs`）。
    println!(
        "看这一份内容：先 `cargo run -q -p px_graphs --bin scene <档>` 出场景文档（配方在 art/scene/），\
         再 `cargo run -q -p px_render_wgpu -- --offline --scene <产物> --out x.png --width 960 --height 640`"
    );
    println!(
        "  这一趟烘的成员（要在 art/scene/ 里自己接上）：height {}｜surface {}",
        px_ops::artifact_path_of(&height.key).display(),
        px_ops::artifact_path_of(&surface.key).display(),
    );

    finish();
}



