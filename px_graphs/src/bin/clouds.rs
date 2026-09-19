//! 云：**普通 Rust** —— 一张场接一张场地算下去，每一步走 `px_cook` 那个缓存辅助函数。
//!
//! 与老写法（`node("field.fbm", "clusters", &[])`，字符串 id + 字节边界）的差别：
//! * 参数类型、**输入个数**、输出域全是**编译期**的事（接错一个输入编不过）；
//! * 参数文件的**位置**（`art/clouds/<名>.toml`）留在图脚本这一侧 —— 同一个
//!   `field.fbm` 在别的图里叫别的名字，算子不该知道；
//! * 键里多了算子的源码哈希 ⇒ 改算子体必然重算，不靠人记得升版本。

use px_cook::{cook_field, cook_mesh, cook_volume};
use px_field_op::typed as field;
use px_field_schema::field::cube_map_extent;
use px_graph::{GraphSpec, begin, finish, params_text};
use px_graph_schema::Grid;
use px_mesh_op::typed as mesh;
use px_protocol::art::Domain;
use px_volume_op::typed as volume;
use px_volume_schema::PATCHES;

/// `clouds` 的单态化声明：`src/bin/clouds/mono/fields.mono`
/// （与 stage 1 的 `fields.rs` 同目录、同主名）。
/// ⚠ 它是 `key = value` 文本、不是 Rust 模块 ⇒ 这里 `include_str!` 进来按行读。
/// 于是 **id 只有一处**，图脚本与生成器不会各自抄一份。
const MONO_DECLARATION: &str = include_str!("clouds/mono/fields.mono");

/// 从声明里取一个 `key = value`。
fn declared(key: &str) -> &'static str {
    MONO_DECLARATION
        .lines()
        .filter_map(|line| line.trim().split_once('='))
        .find(|(name, _)| name.trim() == key)
        .map(|(_, value)| value.trim())
        .unwrap_or_else(|| panic!("clouds/mono.rs 里没有 `{key}`"))
}

const GRAPH_VERSION: u32 = 6;
const SOURCE_HASH: u64 = px_graph::fnv1a(include_str!("clouds.rs"));
const FACE: u32 = 256;

/// 图侧对错误的统一态度：**当场失败**，不静默跳过（§62 那条口径的同一面）。
/// `cook_*` 回的是 `Result<_, String>`，`String` 天然能进 `Box<dyn Error>`。
type Fault = Box<dyn std::error::Error>;

fn main() -> Result<(), Fault> {
    let (width, height) = cube_map_extent(FACE);
    let canvas = Grid {
        width,
        height,
        projection: Domain::CubeMap,
    };
    begin(GraphSpec {
        name: "clouds".to_string(),
        version: GRAPH_VERSION,
        source_hash: SOURCE_HASH,
        width,
        height,
        projection: Domain::CubeMap,
        cameras: px_graph::cameras::review(),
    });
    let cache = px_graph::driver();

    // ⚠ 这一支是**演示 + 判据**：它走的是「生成的单态化实例 + 动态装载」那一级。
    // 用法：`cargo run -p px_graphs --bin clouds -- --closed-cover`
    //
    // 关键：这个算子**不在图程序里**，也不在 `px_volume_op` 里 ——
    // 它在 `target/debug/px_mono_clouds_op.dll`（由 `--bin mono-gen -- clouds` 生成 + 编），
    // 由 `load_directory` 扫出来接上。所以图程序**一个字节都不用重编**。
    if std::env::args().any(|arg| arg == "--closed-cover") {
        use px_cook::field_fn::{FieldFn, SampleField};
        use px_volume_schema::params as volume_params;

        let params = volume_params::parse(params_text("coarse").as_deref())?;
        // ① 老路：先用一个上游算子烘出整张覆盖度场，再在 3D 里按方向回采
        let source = cook_field::<field::Fbm>(&cache, "clusters", (), canvas)?;
        let sampled = SampleField { field: source.field() };
        // 两条路混用时的桥：类型化的 `Cooked` → 老路径的 `Artifact`（键同一个，不重算）
        let source_artifact = px_cook::Cooked {
            key: source.key,
            value: source.value.clone(),
            hit: source.hit,
            millis: source.millis,
            bytes: source.bytes,
        }
        .into_artifact()?;

        // ② 生成的单态化实例：**动态装载**进来的那一份。
        //    它的 `bake<ClosedForm<…>>` 是在那个 dylib **内部**单态化出来的。
        //    ⚠ 描述符声明了 `inputs = ["coverage"]`（老路径的接口形状），所以这一格必须给；
        //      但**闭式那一档不看它** —— 覆盖度是 dylib 里现算的。
        let closed = px_graph::node(declared("id"), "coarse_closed", &[&source_artifact]);

        // 在**同一批方向**上把两条路的覆盖度并排量出来（这才是可比的东西：
        // 比烘出来的体积没有信息量 —— `shape` 在覆盖度低于阈值时两边都归零）。
        let cloud = px_verify::proxy::from_volume(&params);
        let mut open_range = (f32::INFINITY, f32::NEG_INFINITY);
        let steps = 2000_u32;
        for index in 0..steps {
            // 一条**确定性**的扫描线（不带随机数，跨进程一致）
            let t = index as f32 / steps as f32;
            let direction = px_field_schema::field::normalize([
                (t * std::f32::consts::TAU).cos(),
                (t * 3.7).sin() * 0.7,
                (t * std::f32::consts::TAU).sin(),
            ]);
            let opened = sampled.cover(&cloud, direction);
            open_range = (open_range.0.min(opened), open_range.1.max(opened));
        }
        let volume = closed.volume();
        println!(
            "覆盖度对账（{steps} 条方向）：老路（回采混合场）{:.4}..{:.4}",
            open_range.0, open_range.1,
        );
        println!(
            "  生成的实例：{}（键 {}）{} 面 × {}² × {} 层 = {} 个采样，值域 {:.4}..{:.4}",
            declared("id"),
            px_graph::hex_short(&closed.key),
            PATCHES,
            volume.res,
            volume.layers,
            volume.samples(),
            volume.data.iter().cloned().fold(f32::INFINITY, f32::min),
            volume.data.iter().cloned().fold(f32::NEG_INFINITY, f32::max),
        );
        println!(
            "  ⇒ 同一个 `bake<F>` 泛型体：老路把覆盖度烘成 {} 个采样的场再回采；\
             闭式路在 dylib 内部现算，**一次栅格化都没有**",
            source.value.data.len(),
        );
        finish();
        return Ok(());
    }

    // ── 场：七步，每一步都是「普通函数调用 + 隐式缓存」 ───────────────────────
    let clusters = cook_field::<field::Fbm>(&cache, "clusters", (), canvas)?;
    let billows = cook_field::<field::Fbm>(&cache, "billows", (), canvas)?;
    let flow = cook_field::<field::Fbm>(&cache, "flow", (), canvas)?;
    let carved = cook_field::<field::Warp>(&cache, "carved", &[&billows, &flow], canvas)?;
    let weight = cook_field::<field::Constant>(&cache, "weight", (), canvas)?;
    let mixed = cook_field::<field::Mix>(
        &cache,
        "mixed",
        &[&clusters, &carved, &weight],
        canvas,
    )?;
    let coverage = cook_field::<field::Remap>(&cache, "coverage", &mixed, canvas)?;

    let slope_x = cook_field::<field::Gradient>(&cache, "slope_x", &mixed, canvas)?;
    let slope_y = cook_field::<field::Gradient>(&cache, "slope_y", &mixed, canvas)?;
    let slope_z = cook_field::<field::Gradient>(&cache, "slope_z", &mixed, canvas)?;

    // ── 体积：粗场（包住真场）与含细节的真场，参数文件不同、算子同一个 ─────────
    let coarse = cook_volume::<volume::CloudCoarse>(&cache, "coarse", &mixed, canvas)?;
    let proxy = cook_mesh::<mesh::Proxy>(&cache, "proxy", &coarse, canvas)?;
    let fine = cook_volume::<volume::CloudCoarse>(&cache, "coarse_fine", &mixed, canvas)?;
    let proxy_fine = cook_mesh::<mesh::Proxy>(&cache, "proxy_fine", &fine, canvas)?;

    report(
        &coverage,
        &mixed,
        [&slope_x, &slope_y, &slope_z],
        &coarse,
        &fine,
    );

    check("coarse", &mixed, &coarse, &proxy);
    check("coarse_fine", &mixed, &fine, &proxy_fine);

    finish();
    Ok(())
}

fn report(
    coverage: &px_cook::Cooked<px_field_schema::field::Field>,
    mixed: &px_cook::Cooked<px_field_schema::field::Field>,
    slopes: [&px_cook::Cooked<px_field_schema::field::Field>; 3],
    coarse: &px_cook::Cooked<px_volume_schema::VolumeData>,
    fine: &px_cook::Cooked<px_volume_schema::VolumeData>,
) {
    let stats = coverage.stats();
    println!(
        "输出 coverage：{}×{}（{} 面 × {face}²）｜值域 {:.4}..{:.4}｜均值 {:.4}",
        coverage.width,
        coverage.height,
        coverage.height / FACE,
        stats.min,
        stats.max,
        stats.mean,
        face = FACE,
    );
    let smooth = mixed.stats();
    let mut sorted: Vec<f32> = mixed.data.clone();
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
        let stats = node.stats();
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
            PATCHES, node.res, node.layers, node.bytes,
        );
    }
}

/// 判据 2（包住）与 `L` 的量法：每次烘完都在真数据上跑一遍，包括全部命中那一次
/// —— 断言的对象是**存下来的产物**，不是内存里刚算出来的东西。
fn check(
    name: &str,
    mixed: &px_cook::Cooked<px_field_schema::field::Field>,
    volume: &px_cook::Cooked<px_volume_schema::VolumeData>,
    proxy: &px_cook::Cooked<px_mesh_schema::MeshData>,
) {
    // ⚠ 参数走 schema 的类型化解析（驱动只给原文）：判据读的是**同一份 TOML**，
    //   不是自己再抄一遍的数。
    let params = px_volume_schema::params::parse(params_text(name).as_deref())
        .unwrap_or_else(|err| panic!("读参数 {name} 失败：{err}"));
    let cloud = px_verify::proxy::from_volume(&params);
    let coverage = mixed.field();
    let mesh = proxy.mesh();
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
        volume.res,
        volume.layers,
        volume.value.samples(),
    );
}
