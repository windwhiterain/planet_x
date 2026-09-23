//! 星云背景：**一张图，从形状到天空**。
//!
//! ```text
//! 图 nebula（画布 = 体网格）            图 nebulasky（画布 = 立方贴图）
//! ─────────────────────────            ──────────────────────────────
//! field.fbm3 / ridged3 / warp3           field.stars
//!         │                                     │
//!         ▼  cloud.density                      │
//!     VolumeData                                │
//!         │                                     │
//!         ▼  cloud.emission                     │
//!     VolumeData（RGB 发射 + A 消光）            │
//!         └──────────────┬──────────────────────┘
//!                        ▼  sky.nebula ×3      沿视线积分，一条通道一张 CubeMap 场
//!                   3 × Field
//!                        │
//!                        ▼  color_cube + write_texture   拼成 HDR 立方贴图 → CAS
//! ```
//!
//! ⚠ **必须是两张图**：体积那一条链要**体网格**画布（`res × res²·layers·6`），
//!   而星点与天空要**球面**画布（`res × res·6`）。两种画布的"行数"含义不同，
//!   `VolumeShape::of` 会把 `32×192` 解成 `layers = 1/6` 那样的非整数而拒掉
//!   （实测：`field.fbm3 要一张体网格画布…拿到的是 CubeMap 32×192`）。
//!   ⚠ 两张图**不能同名** —— 名字决定 `art/<名字>/*.toml` 与清单落点，同名会撞键。
//!   顺带的好处：星图只烘一次、三条通道共用（放在一张图里会被积三遍）。
//!
//! ⚠ `--face` 是**两个**面分辨率：体积那边是 `cloud.density` 的画布宽度，
//!   天空那边是 `sky.*.toml` 的 `face`（两边不一致会在形状检查那里当场报）。
//!
//! 用法：
//! ```text
//! cargo run --release -p px_graphs --bin nebula -- --face 64    # 快速迭代
//! ```

use std::time::Instant;

use px_cook::{Domain, GraphSpec, begin, cook, field, volume};
use px_field_schema::field::Field;

/// 图侧对错误的统一态度：**当场失败**，不静默跳过。
type Fault = Box<dyn std::error::Error>;

/// 命令行给的面分辨率（`--face <n>`）。
fn face_from_args() -> u32 {
    let args: Vec<String> = std::env::args().collect();
    let mut face = 64_u32;
    let mut index = 1;
    while index < args.len() {
        if args[index] == "--face" {
            if let Some(value) = args.get(index + 1).and_then(|text| text.parse().ok()) {
                face = value;
            }
        }
        index += 1;
    }
    face.max(8)
}

/// **形状那一半的分辨率**（`--shape <n>`）。
///
/// ⚠ 它与天空贴图的分辨率**各是一个旋钮**，这不是留白而是必须的：
///   三维场的格数是 `res² × layers × 6`（体网格布局），而 `layers` 是独立参数
///   ⇒ 形状的分辨率涨一倍，场的格数涨**四倍**，而每个格都要算 7 个倍频的三维噪声。
///   实测形状 80 要 **394 秒**，而压到 64 时同一张图只要几十秒。
///
/// ⚠ 更要紧的是：**这两件事本来无关**。天空贴图的分辨率决定"星点有多锐"，
///   形状的分辨率决定"云和丝有多细"。把它们绑在一起，就只能用"云更细"来换"星更锐"
///   —— 而星的锐度根本不需要更细的场（星是**点**，不是场的结构）。
fn shape_from_args() -> u32 {
    let args: Vec<String> = std::env::args().collect();
    let mut shape = 64_u32;
    let mut index = 1;
    while index < args.len() {
        if args[index] == "--shape" {
            if let Some(value) = args.get(index + 1).and_then(|text| text.parse().ok()) {
                shape = value;
            }
        }
        index += 1;
    }
    shape.max(8)
}

/// **径向层数**（`--layers <n>`）：不给就用 `art/nebula/density_volume.toml` 里那一份。
///
/// ⚠ 单独给这个开关是因为**径向是体网格上最粗的那一维**：`layers = 64` 要覆盖
///   1.0~3.0 的壳厚 ⇒ 每层 0.031 世界单位，而一条视线穿过壳要走 2.0
///   ⇒ 径向的细节被三线性平均得最狠。而它比 `--shape` **便宜**：
///   格数是 `res² × layers`，径向翻倍只让格数翻倍（面内翻倍是四倍）。
fn layers_from_args() -> Option<u32> {
    let args: Vec<String> = std::env::args().collect();
    let mut index = 1;
    while index < args.len() {
        if args[index] == "--layers" {
            if let Some(value) = args
                .get(index + 1)
                .and_then(|text| text.parse::<u32>().ok())
            {
                return Some(value.max(8));
            }
        }
        index += 1;
    }
    None
}

/// `art/nebula/density_volume.toml` 里写的层数（读不到/解不开就 `Err`，不猜）。
///
/// ⚠ 必须读**文件里那一份**，不能用 `DensityParams::default()`：两者不一致时
///   `cloud.density` 会走"重采样"那条路（允许，但白花一次采样），而症状只是"变慢"。
fn volume_layers() -> Result<u32, Fault> {
    let path = px_graph::workspace_root()
        .join("art")
        .join("nebula")
        .join("density_volume.toml");
    let text = std::fs::read_to_string(&path)
        .map_err(|err| format!("读不到 {}：{err}", path.display()))?;
    let params: px_volume_schema::params::density::DensityParams =
        toml::from_str(&text).map_err(|err| format!("{} 解不开：{err}", path.display()))?;
    Ok(params.layers)
}

/// 打印一张场的读数（形状对不对、值域有没有塌掉，一眼就能看出来）。
fn report(name: &str, field: &Field) {
    let stats = field.stats();
    println!(
        "  {name}：{}×{}｜值域 {:.4}..{:.4}｜均值 {:.4}",
        field.width, field.height, stats.min, stats.max, stats.mean
    );
}

fn main() -> Result<(), Fault> {
    let face = face_from_args();
    let shape = shape_from_args();
    let started = Instant::now();

    // ── 图一：形状 → 密度体积 → 发射体积（画布是**体网格**）────────────────
    // ⚠ 画布的形状必须与 `cloud.density` 拿到的参数**逐字一致**：只有"同网格"时它才走
    //   纯搬运，不同网格会按体素坐标重采样（允许，但白花一次采样）。
    //   ⇒ 默认层数从 `art/nebula/density_volume.toml` 读（不用 `Default`）；
    //   `--layers` 给的则同时**覆盖那份 toml 里的值**（见下面 `params_override`）。
    let layers = match layers_from_args() {
        Some(given) => given,
        None => volume_layers()?,
    };
    println!("形状 {shape} × {shape} × {layers} 层 ｜ 天空面 {face}");
    let shape_graph = begin(GraphSpec {
        name: "nebula".to_string(),
        width: shape,
        height: shape * layers * 6,
        projection: Domain::Volume,
        cameras: Vec::new(),
    });

    let blobs = cook::<field::Fbm3>(&shape_graph, "blobs", ())?;
    let wisps = cook::<field::Ridged3>(&shape_graph, "wisps", ())?;
    let flow = cook::<field::Fbm3>(&shape_graph, "flow", ())?;
    let flow_second = cook::<field::Fbm3>(&shape_graph, "flow_second", ())?;
    let flow_third = cook::<field::Fbm3>(&shape_graph, "flow_third", ())?;

    // 三轴扭曲：三个上游各管一个轴。⚠ 顺序上**先叠结构、最后扭曲**：反过来会把脊搅散。
    let warped = cook::<field::Warp3>(
        &shape_graph,
        "warped",
        field::Warp3Input {
            field: blobs,
            offset_a: flow,
            offset_b: flow_second,
            offset_c: flow_third,
        },
    )?;
    // ⚠⚠ **打破球对称的那一刀**（这一条是量出来的，不是风格偏好）。
    //
    // `cloud.emission` 的壳是**均匀包住观察者**的 ⇒ 每条视线都穿过等量的气
    // ⇒ 画面上每一处都有底噪，**黑色不存在**。实测：
    //
    // | | p10 | p50 | p90 | p90/p50 |
    // |---|---|---|---|---|
    // | 参考 | 0.0051 | 0.0171 | 0.0972 | **5.7** |
    // | 只有均匀壳 | 0.0110 | 0.0199 | 0.0341 | **1.7** |
    //
    // 两边"亮于 0.02 的面积"都是 ~45%（总能量相当），差的是**动态范围**：
    // 我的暗部亮一倍、亮部暗三倍。⇒ 需要一大块**真正空掉的**方向，
    // 气只聚在**一片**里（参考图正是"一团云 + 大片黑"）。
    //
    // 这一档把低频的团块场**二值化**成"有气 / 没气"，再乘进密度里。
    let extent = cook::<field::Remap>(
        &shape_graph,
        "extent",
        field::FieldInput {
            field: warped.clone(),
        },
    )?;
    let weight = cook::<field::Remap>(
        &shape_graph,
        "weight",
        field::FieldInput {
            field: warped.clone(),
        },
    )?;
    let density = cook::<field::Mix>(
        &shape_graph,
        "density",
        field::MixInput {
            a: warped,
            b: wisps.clone(),
            mask: weight,
        },
    )?;
    // ⚠ 密度 × 包络 ⇒ 包络为 0 的地方**连消光都是 0**（那才是真空），
    //   不只是"暗一点" —— 这一条决定了暗部能不能真的压到 0 附近。
    //   常数 0 走 `vacuum.toml`（`field.constant` 的参数只有 `value`）。
    let vacuum = cook::<field::Constant>(&shape_graph, "vacuum", ())?;
    let shaped = cook::<field::Mix>(
        &shape_graph,
        "shaped",
        field::MixInput {
            a: vacuum.clone(),
            b: density,
            mask: extent,
        },
    )?;
    report("shaped", shaped.value());

    // ── **暗尘带**：沿脊线雕细缝（参考图的"暗尘埃柱与暗带"）──────────────────
    //
    // ⚠⚠ 与上一轮失败那版（独立的 `filaments` 高频场）的差别就是这一刀的全部要点：
    //   失败版 `mix(0, shaped, 门)` = `shaped × 门`，而门的均值在 0.5 附近
    //   ⇒ **整片密度被砍半**（实测均值 0.055 → 0.021，局域对比反而掉）。
    //   这一版 `mix(shaped, 0, 脊)` = `shaped × (1 − 脊)`，而脊**又细又稀**
    //   （只取 ridged 场的顶部）⇒ 处处保持原样、只在脊线上开缝。
    //   ⇒ "乘（稀疏的 1−x）"与"乘（稠密的 x）"是两回事：前者不动平均，后者必然砍一半。
    //
    // ⚠ 复用**已有的** `wisps`（ridged 场），不另造高频场：
    //   1. 省掉一趟最贵的噪声（上一版 `filaments` 白花了 90 秒级的烘图开销，
    //      而且它算完之后**根本没接进管线**（`density_volume` 一直吃的是 `shaped`）
    //      —— 死重量不是无害的，它每次烘图都在收钱）；
    //   2. 上一版 `frequency = 18` 超出 64³ 网格的奈奎斯特（≈32），
    //      **那层细节根本产生不出来** —— 这是它的第二个败因。
    //   `wisps` 频率 11 ⇒ 脊宽约 1 格、间距约 6 格，正是"尘带"的尺度。
    let carved = cook::<field::Remap>(&shape_graph, "carved", field::FieldInput { field: wisps })?;
    // ⚠ `mix(a, b, mask) = a×(1−mask) + b×mask` ⇒ `mix(shaped, 真空, 脊)`
    //   就是"沿脊线把密度雕低"。`carved.toml` 的 `out_max = 0.8` ⇒ 缝里留 20% 的气
    //   —— 留一点消光，挡住缝后面的星点（雕到 0 会像"破洞"，星会透出来）。
    let textured = cook::<field::Mix>(
        &shape_graph,
        "textured",
        field::MixInput {
            a: shaped,
            b: vacuum,
            mask: carved,
        },
    )?;
    report("textured", textured.value());

    let density_volume = cook::<volume::Density>(
        &shape_graph,
        "density_volume",
        volume::DensityInput { density: textured },
    )?;
    let emission = cook::<volume::Emission>(
        &shape_graph,
        "emission",
        volume::EmissionInput {
            volume: density_volume,
        },
    )?;

    // ── 图二：星点 + 整张天空（画布是立方贴图）────────────────────────────
    let sky_graph = begin(GraphSpec {
        name: "nebulasky".to_string(),
        width: face,
        height: face * 6,
        projection: Domain::CubeMap,
        cameras: Vec::new(),
    });
    let stars = cook::<field::Stars>(&sky_graph, "stars", ())?;
    // ⚠ **一个节点交出一整张天空贴图**（三条通道在算子内部各积一遍）。
    //   `sky` 就是场景文档要引用的那个节点名（`nebulasky::sky`），而它是一条
    //   **正常的图成员** —— 驱动照常进键、落盘、登记清单，这里不必手工拼贴图。
    let sky = cook::<volume::SkyNebula>(
        &sky_graph,
        "sky",
        volume::SkyInput {
            volume: emission,
            stars,
        },
    )?;
    println!(
        "天空贴图：{}",
        px_graph_schema::payload::Build::detail(sky.value())
    );

    // ⚠ **必须收尾**：清单（`target/pcg/<图名>/manifest.json`）是 `finish()` 写出去的
    //   —— 场景文档按 `"图名::节点名"` 取成员键走的就是它。不收尾的话产物**在 CAS 里**
    //   但**没有名字**，场景那边会报"图里没有节点"。
    shape_graph.finish();
    sky_graph.finish();
    println!("共 {:.1} 秒", started.elapsed().as_secs_f64());
    Ok(())
}
