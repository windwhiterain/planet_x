use px_cook::{
    Domain, Graph, GraphSpec, begin, cached, field, field_params, mesh, node_params, volume,
};
use px_field_schema::field::cube_map_extent;
use px_graphs::elem;
use px_volume_schema::PATCHES;

const FACE: u32 = 256;

type Fault = Box<dyn std::error::Error>;

fn main() -> Result<(), Fault> {
    px_cook::apply_store_args()?;
    let (width, height) = cube_map_extent(FACE);
    let graph = begin(GraphSpec {
        name: "clouds".to_string(),
    });

    let shape = field_params::Shape {
        width,
        height,
        projection: Domain::CubeMap,
    };

    let clusters = cached(
        &graph,
        "clusters",
        field::Fbm,
        field_params::fbm::Params {
            shape,
            ..node_params(&graph, "clusters")?
        },
        (),
    )?;
    let billows = cached(
        &graph,
        "billows",
        field::Fbm,
        field_params::fbm::Params {
            shape,
            ..node_params(&graph, "billows")?
        },
        (),
    )?;
    let flow = cached(
        &graph,
        "flow",
        field::Fbm,
        field_params::fbm::Params {
            shape,
            ..node_params(&graph, "flow")?
        },
        (),
    )?;
    let carved = cached(
        &graph,
        "carved",
        field::Warp,
        node_params(&graph, "carved")?,
        field::FieldPairInput {
            field: billows,
            offset: flow,
        },
    )?;
    let weight = cached(
        &graph,
        "weight",
        elem::Constant,
        elem::ConstantParams {
            shape,
            ..node_params(&graph, "weight")?
        },
        (),
    )?;
    let mixed = cached(
        &graph,
        "mixed",
        elem::Mix,
        node_params(&graph, "mixed")?,
        elem::MixInput {
            a: clusters,
            b: carved,
            mask: weight,
        },
    )?;

    let coverage = cached(
        &graph,
        "coverage",
        elem::Remap,
        node_params(&graph, "coverage")?,
        elem::RemapInput {
            field: mixed.clone(),
        },
    )?;
    let slope_x = cached(
        &graph,
        "slope_x",
        field::Gradient,
        node_params(&graph, "slope_x")?,
        field::FieldInput {
            field: mixed.clone(),
        },
    )?;
    let slope_y = cached(
        &graph,
        "slope_y",
        field::Gradient,
        node_params(&graph, "slope_y")?,
        field::FieldInput {
            field: mixed.clone(),
        },
    )?;
    let slope_z = cached(
        &graph,
        "slope_z",
        field::Gradient,
        node_params(&graph, "slope_z")?,
        field::FieldInput {
            field: mixed.clone(),
        },
    )?;

    let coarse = cached(
        &graph,
        "coarse",
        volume::CloudCoarse,
        node_params(&graph, "coarse")?,
        volume::CloudCoarseInput {
            coverage: mixed.clone(),
        },
    )?;
    let proxy = cached(
        &graph,
        "proxy",
        mesh::Proxy,
        node_params(&graph, "proxy")?,
        mesh::ProxyInput {
            volume: coarse.clone(),
        },
    )?;
    let fine = cached(
        &graph,
        "coarse_fine",
        volume::CloudCoarse,
        node_params(&graph, "coarse_fine")?,
        volume::CloudCoarseInput {
            coverage: mixed.clone(),
        },
    )?;
    let proxy_fine = cached(
        &graph,
        "proxy_fine",
        mesh::Proxy,
        node_params(&graph, "proxy_fine")?,
        mesh::ProxyInput {
            volume: fine.clone(),
        },
    )?;

    report(
        &coverage,
        &mixed,
        [&slope_x, &slope_y, &slope_z],
        &coarse,
        &fine,
    );

    check(&graph, "coarse", &mixed, &coarse, &proxy);
    check(&graph, "coarse_fine", &mixed, &fine, &proxy_fine);

    graph.finish();
    Ok(())
}

fn report(
    coverage: &px_cook::Cooked<px_field_schema::field::Field>,
    mixed: &px_cook::Cooked<px_field_schema::field::Field>,
    slopes: [&px_cook::Cooked<px_field_schema::field::Field>; 3],
    coarse: &volume::VolumeOut,
    fine: &volume::VolumeOut,
) {
    let stats = coverage.value().stats();
    println!(
        "输出 coverage：{}×{}（{} 面 × {face}²）｜值域 {:.4}..{:.4}｜均值 {:.4}",
        coverage.value().width,
        coverage.value().height,
        coverage.value().height / FACE,
        stats.min,
        stats.max,
        stats.mean,
        face = FACE,
    );
    let smooth = mixed.value().stats();
    let mut sorted: Vec<f32> = mixed.value().data.clone();
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
    for (name, node) in ["slope_x", "slope_y", "slope_z"].iter().zip(slopes) {
        let stats = node.value().stats();
        println!(
            "输出 {name}：值域 {:.4}..{:.4}｜均值 {:.4}",
            stats.min, stats.max, stats.mean
        );
    }
    println!(
        "看这一份内容：先 `cargo run -q -p px_graphs --bin scene <档>` 出场景文档（配方在 art/scene/），\
         再 `cargo run -q -p px_render -- --offline --scene <产物> --out x.png --width 960 --height 640`"
    );
    for (label, node) in [("coarse", coarse), ("coarse_fine", fine)] {
        println!(
            "  {label} 体积：{} 面 × {}² × {} 层｜{} B",
            PATCHES,
            node.value().res,
            node.value().layers,
            node.bytes,
        );
    }
}

fn check(
    graph: &Graph,
    name: &str,
    mixed: &px_cook::Cooked<px_field_schema::field::Field>,
    volume: &volume::VolumeOut,
    proxy: &px_cook::Cooked<px_mesh_schema::MeshData>,
) {
    let params = node_params(graph, name).unwrap_or_else(|err| panic!("读参数 {name} 失败：{err}"));
    let cloud = px_verify::proxy::from_volume(&params);
    let coverage = mixed.value();
    let mesh = proxy.value();
    let final_field = params.field == px_volume_schema::FieldKind::Final;

    let rays: usize = 256;
    let report = px_graphs::cloud_proxy::containment(mesh, &cloud, coverage, &params, rays, 4096);
    px_graphs::cloud_proxy::print_containment(&report, &params);
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

    let dense = std::env::args().any(|arg| arg == "--bound");
    let (faces, res, layers) = if dense { (6, 256, 96) } else { (6, 64, 48) };
    let bound = px_graphs::cloud_proxy::measure_gradient_bound(
        &cloud, coverage, &params, faces, res, layers,
    );
    println!(
        "  {name} 梯度上界：{faces} 面 × {res}² × {layers} 层上量到 |∇场| ≤ {:.3}（三轴 {:.1} / {:.1} / {:.1}；在面 {} 参数 {:?}，方向 {:?}，场值 {:.4}）；参数里写的 scale = {:.3}",
        bound.bound,
        bound.axes[0],
        bound.axes[1],
        bound.axes[2],
        bound.face,
        bound.at.map(|value| (value * 1000.0).round() / 1000.0),
        bound
            .direction
            .map(|value| (value * 1000.0).round() / 1000.0),
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
        volume.value().res,
        volume.value().layers,
        volume.value().samples(),
    );
}
