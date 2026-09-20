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
    let started = Instant::now();

    // ── 图一：形状 → 密度体积 → 发射体积（画布是体网格）────────────────────
    let shape_graph = begin(GraphSpec {
        name: "nebula".to_string(),
        width: face,
        height: face * face * (face / 2).max(8) * 6,
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
            b: wisps,
            mask: weight,
        },
    )?;
    report("density", density.value());

    let density_volume = cook::<volume::Density>(
        &shape_graph,
        "density_volume",
        volume::DensityInput {
            density: density.clone(),
        },
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
