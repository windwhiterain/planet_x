use px_field_schema::field::cube_map_extent;
use px_field_schema::params as field_params;
use px_graph::{GraphSpec, begin, finish, node};
use px_mesh_schema::params as mesh_params;
use px_protocol::art::Domain;
use px_volume_schema::{PATCHES, params as volume_params};

const GRAPH_VERSION: u32 = 5;
const SOURCE_HASH: u64 = px_graph::fnv1a(include_str!("clouds.rs"));
const FACE: u32 = 256;

fn main() {
    let (width, height) = cube_map_extent(FACE);
    begin(GraphSpec {
        name: "clouds".to_string(),
        version: GRAPH_VERSION,
        source_hash: SOURCE_HASH,
        width,
        height,
        projection: Domain::CubeMap,
        cameras: px_graph::cameras::review(),
    });

    let clusters = node(field_params::FBM, "clusters", &[]);
    let billows = node(field_params::FBM, "billows", &[]);
    let flow = node(field_params::FBM, "flow", &[]);
    let carved = node(field_params::WARP, "carved", &[&billows, &flow]);
    let weight = node(field_params::CONSTANT, "weight", &[]);
    let mixed = node(field_params::MIX, "mixed", &[&clusters, &carved, &weight]);
    let coverage = node(field_params::REMAP, "coverage", &[&mixed]);

    let slope_x = node(field_params::GRADIENT, "slope_x", &[&mixed]);
    let slope_y = node(field_params::GRADIENT, "slope_y", &[&mixed]);
    let slope_z = node(field_params::GRADIENT, "slope_z", &[&mixed]);

    // 硬表面的代理：先烘一张立方球参数空间的场网格，再拿它出等值面。
    // 两级分开进缓存 ⇒ 只改等值面参数（比如 depth）时，网格照命中。
    let coarse = node(volume_params::CLOUD_COARSE, "coarse", &[&mixed]);
    let proxy = node(mesh_params::PROXY, "proxy", &[&coarse]);

    // 第二份：同样的算子、烘**含细节的真场**（`field = "final"`，见 art/clouds/coarse_fine.toml）。
    // 它不包住真场（它就是真表面）⇒ 判据从「包住」换成「像素逐字节」。
    let fine = node(volume_params::CLOUD_COARSE, "coarse_fine", &[&mixed]);
    let proxy_fine = node(mesh_params::PROXY, "proxy_fine", &[&fine]);

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

    // ⚠ S8-a：这里原来打的是 bevy 宿主的命令行
    //   （`--planet … --mesh … --palette rocky --clouds … --cloud-slope …`）。
    //   那支宿主已经删掉，本仓只剩 `px_render`，而它**只吃 `.pxart` 场景文档**
    //   —— 没有"现场拿几个成员拼一个场景"这条路。⇒ 打一条**真跑得起来**的命令，
    //   而不是留一句谁也无法执行的提示（把成员配成场景的地方是 `art/scene/*.toml`）。
    // ⚠ 改这几行会动 `SOURCE_HASH`（本文件自己），但它只用来打一句"源码变了"的警告、
    //   **不进任何产物键**（`px_graph/src/driver.rs` 里那条 `graph_source_hash` 的用法）。
    println!(
        "看这一份内容：先 `cargo run -q -p px_graphs --bin scene <档>` 出场景文档（配方在 art/scene/），\
         再 `cargo run -q -p px_render -- --offline --scene <产物> --out x.png --width 960 --height 640`"
    );
    println!(
        "  这一趟烘的成员（要在 art/scene/ 里自己接上）：clouds {}｜slope_x {}｜slope_y {}｜slope_z {}",
        px_graph::artifact_path_of(&mixed.key).display(),
        px_graph::artifact_path_of(&slope_x.key).display(),
        px_graph::artifact_path_of(&slope_y.key).display(),
        px_graph::artifact_path_of(&slope_z.key).display(),
    );
    println!(
        "代理：{}（{} 顶点 / {} 三角形）",
        px_graph::artifact_path_of(&proxy.key).display(),
        proxy.mesh().vertices(),
        proxy.mesh().triangles(),
    );
    println!(
        "细代理：{}（{} 顶点 / {} 三角形）",
        px_graph::artifact_path_of(&proxy_fine.key).display(),
        proxy_fine.mesh().vertices(),
        proxy_fine.mesh().triangles(),
    );

    check("coarse", &mixed, &coarse, &proxy);
    check("coarse_fine", &mixed, &fine, &proxy_fine);

    finish();
}

/// 判据 2（包住）与 `L` 的量法：每次烘完都在真数据上跑一遍，包括全部命中那一次
/// —— 断言的对象是**存下来的产物**，不是内存里刚算出来的东西。
fn check(name: &str, mixed: &px_graph::Artifact, volume: &px_graph::Artifact, proxy: &px_graph::Artifact) {
    // ⚠ 参数走 schema 的类型化解析（驱动只给原文）：判据读的是**同一份 TOML**，
    //   不是自己再抄一遍的数。
    let params = px_volume_schema::params::parse(px_graph::params_text(name).as_deref())
        .unwrap_or_else(|err| panic!("读参数 {name} 失败：{err}"));
    let cloud = px_verify::proxy::from_volume(&params);
    let coverage = mixed.field();
    let mesh = proxy.mesh();
    let final_field = params.field == px_volume_schema::FieldKind::Final;

    let rays: usize = 256;
    let report = px_graphs::cloud_proxy::containment(mesh, &cloud, coverage, &params, rays, 4096);
    px_graphs::cloud_proxy::print_containment(&report, &params);
    // 真场代理不再包住真场（它就是真表面）⇒ 「包住」这条对它不成立也不该成立，
    // 但「一条都不许漏交点」仍然是硬失败（缺几何 = 没有 fragment）。
    let contained = report.missing == 0 && report.worst_slack > -report.worst_cell;
    println!(
        "  {name} 包住：{}（{} 条参照场有交点的方向里漏了 {} 条；余量 {:+.6}，一个单元对角线 {:.6}）",
        if contained { "是" } else { "否" },
        report.rays_with_surface,
        report.missing,
        report.worst_slack,
        report.worst_cell,
    );
    assert_eq!(
        report.missing, 0,
        "{name} 没包住：{} 条方向参照场有交点、代理一个交点都没有",
        report.missing,
    );
    assert!(
        contained || final_field,
        "{name}（粗场）没包住参照场：最差余量 {:+.6}（一个单元对角线 {:.6}）",
        report.worst_slack,
        report.worst_cell,
    );

    // 量 L：默认网格 6 面 × 64² × 48 层；加 --bound 用更密的网格复核。
    let dense = std::env::args().any(|arg| arg == "--bound");
    let (faces, res, layers) = if dense { (6, 256, 96) } else { (6, 64, 48) };
    let bound =
        px_graphs::cloud_proxy::measure_gradient_bound(&cloud, coverage, &params, faces, res, layers);
    println!(
        "  {name} 梯度上界：{faces} 面 × {res}² × {layers} 层上量到 |∇场| ≤ {:.3}（三轴 {:.1} / {:.1} / {:.1}；在面 {} 参数 {:?}，方向 {:?}，场值 {:.4}）；参数里写的 scale = {:.3}",
        bound.bound,
        bound.axes[0],
        bound.axes[1],
        bound.axes[2],
        bound.face,
        bound.at.map(|value| (value * 1000.0).round() / 1000.0),
        bound.direction.map(|value| (value * 1000.0).round() / 1000.0),
        bound.value,
        params.scale,
    );
    assert!(
        bound.bound <= params.scale,
        "{name} 量到的梯度上界 {:.3} 超过参数里的 scale {:.3} ⇒ 归一化没压到 1 以下",
        bound.bound,
        params.scale,
    );
    println!(
        "  {name} 场网格：{} 面 × {}² 射线 × {} 层 = {} 个采样",
        PATCHES,
        volume.volume().res,
        volume.volume().layers,
        volume.volume().samples(),
    );
}
