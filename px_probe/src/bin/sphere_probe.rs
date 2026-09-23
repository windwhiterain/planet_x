//! `sphere_probe`：**标准软面球** ⇒ 把"密度"从实验里排除掉。
//!
//! 用户 2026-09-25："就弄一个标准的软面球体做实验。"
//!
//! 之前所有对照都被同一件事干扰：云内是**噪声场**（fbm 混 remap），亮斑到底来自密度还是
//! 来自星光分不清 ✗。这里把密度换成一个**解析**的东西 —— 半径剖面
//!
//! ```text
//! d(r) = 1                          (r ≤ R)
//!      = (R + w − r) / w            (R < r < R + w)     ← 软面
//!      = 0                          (r ≥ R + w)
//! ```
//!
//! 一个偏心的球（放在相机正前方 r ≈ 1.8 处）⇒ 内部**恒定**、边缘**软**、外面**精确 0** ✓。
//! 于是画面里云的形状完全已知：亮的地方**只可能**来自星光（以及视线穿过球的路径长度）✓。
//!
//! 流程与图脚本一致：`bake_stars`（星跟着密度 ⇒ 星落在球里 ✓）→ `bake_emission`
//! → `raymarch_sky` → 取正面写 PNG。

use px_volume_schema::VolumeData;
use px_volume_schema::params::emission::EmissionParams;
use px_volume_schema::params::sky::SkyParams;
use px_volume_schema::params::stars::StarsParams;

/// 面的分辨率（输出 PNG 就是 `FACE × FACE`）。
const FACE: u32 = 512;
/// 每个面的 s/t 采样数（体积的角向分辨率）。
const RES: u32 = 256;
/// 径向层数。
const LAYERS: u32 = 96;
const INNER: f32 = 1.0;
const OUTER: f32 = 3.0;
/// 球心（世界坐标）与半径、软面宽度。
const CENTRE: [f32; 3] = [0.0, 0.0, 1.8];
const RADIUS: f32 = 0.55;
const SOFT: f32 = 0.18;
/// 球**内部**的密度（单一变量：只改它）。
const DENSITY: f32 = 1.0;

fn main() -> Result<(), String> {
    // ⚠ `--dump`：把 `bake_emission` 的**内部结果**打出来（走 CPU 那份对账实现，球缩到
    //   32²×16 层 ⇒ 几秒）。用户："这不都是全红吗，哪里来的遮蔽？" —— 图已经问不出答案了，
    //   要看的是**数**：球内发射通道的分布到底是"尖峰"还是"一片常数"。
    if std::env::args().any(|arg| arg == "--dump") {
        return dump();
    }

    let stars_params = StarsParams {
        count: 1_300,
        // ⚠ 星落在球里：其余参数照 art 的语义给（见 `art/nebulasky/stars.toml`）。
        gas_biased: true,
        gas_contrast: 1.5,
        gas_floor: 0.35,
        gas_depth: 0.0,
        clump_count: 0,
        clump_share: 0.0,
        star_tint: [0.12, 0.30, 1.0],
        ..Default::default()
    };
    let emission_params = EmissionParams {
        // ⚠ 自发光关着（用户要求）：云只能被星照亮。
        emission_gain: 0.0,
        glow_gain: 0.0,
        // 星就是点光源；遮蔽负责塑造"照亮哪一片"。
        starlight_gain: 6.4e-4,
        starlight_radius: 0.4,
        starlight_soft: 0.06,
        starlight_steps: 24,
        starlight_max: 16,
        shadow_gain: 10.0,
        extinction: [10.0, 10.0, 10.0],
        scatter_tint: [1.0, 1.0, 1.0],
        glow_tint: [1.0, 0.03, 0.03],
        ..Default::default()
    };
    let sky_params = SkyParams {
        face: FACE,
        steps: 96,
        star_gain: 0.10,
        star_tint: [0.12, 0.30, 1.0],
        background: [0.0, 0.0, 0.0],
        ..Default::default()
    };

    println!("SkyParams default = {:#?}", SkyParams::default());
    println!("EmissionParams default = {:#?}", EmissionParams::default());
    let sphere = soft_sphere();
    let stars = px_volume_alg::bake_stars(&stars_params, Some(&sphere))?;
    println!("球里落了 {} 颗星", stars.count());
    let emission = px_volume_gpu_op::bake_emission(&sphere, &stars, &emission_params)?;
    // ⚠ GPU 那份发射体积自己的分布（球内，密度 > 0.5 的体素）：与 CPU dump 对一下，
    //   看"图像不随发射变"到底是 GPU 烘焙的问题还是天空那一档的问题。
    {
        let width = emission.res as usize * 6;
        let side = emission.res as usize;
        let layers = emission.layers as usize;
        let shell = px_volume_schema::volume::Shell::new(INNER, OUTER);
        let mut values: Vec<f32> = Vec::new();
        for row in 0..(6 * layers * side) {
            let face = (row / (layers * side)) as u32;
            let rest = row % (layers * side);
            let layer = (rest / side) as u32;
            let t = (rest % side) as u32;
            for s_index in 0..side {
                let radius = shell.radius_of(layer as f32 / (layers - 1) as f32);
                let direction = px_protocol::art::cube_direction(
                    face,
                    (s_index as f32 + 0.5) / side as f32,
                    (t as f32 + 0.5) / side as f32,
                );
                let point = [
                    direction[0] * radius,
                    direction[1] * radius,
                    direction[2] * radius,
                ];
                let offset = [
                    point[0] - CENTRE[0],
                    point[1] - CENTRE[1],
                    point[2] - CENTRE[2],
                ];
                let distance =
                    (offset[0] * offset[0] + offset[1] * offset[1] + offset[2] * offset[2]).sqrt();
                if ((RADIUS + SOFT - distance) / SOFT).clamp(0.0, 1.0) > 0.5 {
                    values.push(emission.data[row * width + s_index * 6]);
                }
            }
        }
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let pick = |q: f64| values[((values.len() - 1) as f64 * q) as usize];
        println!(
            "GPU 发射 R（球内 {} 个体素）：min {:.3e} | p50 {:.3e} | p90 {:.3e} | max {:.3e}",
            values.len(),
            values[0],
            pick(0.5),
            pick(0.9),
            values[values.len() - 1]
        );
    }
    let sky = px_volume_gpu_op::raymarch_sky(&emission, &stars, &sky_params)?;

    // ⚠ 先看清布局：立方贴图的 w/h/layers/levels/format，以及几个采样值。
    println!(
        "天空贴图：{}×{}｜层 {}｜mip {}｜格式 {:?}｜字节 {}",
        sky.width,
        sky.height,
        sky.layers,
        sky.levels,
        sky.format,
        sky.bytes.len()
    );
    for probe in [0_usize, 1000, 100_000] {
        let at = probe * 4;
        if at + 4 <= sky.bytes.len() {
            let raw = [
                sky.bytes[at],
                sky.bytes[at + 1],
                sky.bytes[at + 2],
                sky.bytes[at + 3],
            ];
            println!("  [{}] f32 = {:.6}", probe, f32::from_le_bytes(raw));
        }
    }

    // ⚠ 布局：每纹素 4 个**半精度**通道（Rgba16Float），六面按**层**排。
    let face = FACE as usize;
    let (columns, rows) = (3usize, 2usize);
    let mut rgba = vec![0_u8; face * columns * face * rows * 4];
    for layer in 0..6usize {
        let (cx, cy) = (layer % columns, layer / columns);
        for y in 0..face {
            for x in 0..face {
                let source = ((layer * face + y) * face + x) * 4 * 2;
                let mut rgb = [0.0_f32; 3];
                for channel in 0..3 {
                    let at = source + channel * 2;
                    let bits = u16::from_le_bytes([sky.bytes[at], sky.bytes[at + 1]]);
                    rgb[channel] = half_to_f32(bits).clamp(0.0, 1.0);
                }
                let (px, py) = (cx * face + x, cy * face + y);
                let target = (py * face * columns + px) * 4;
                for channel in 0..3 {
                    rgba[target + channel] = (rgb[channel].powf(1.0 / 2.2) * 255.0 + 0.5) as u8;
                }
                rgba[target + 3] = 255;
            }
        }
    }
    let image = image::RgbaImage::from_raw((face * columns) as u32, (face * rows) as u32, rgba)
        .ok_or_else(|| "像素数与尺寸对不上".to_string())?;
    image::DynamicImage::ImageRgba8(image)
        .to_rgb8()
        .save_with_format("target/probe/sphere.png", image::ImageFormat::Png)
        .map_err(|err| format!("写 PNG 失败：{err}"))?;
    // ⚠ 最终图像本身的分布：球所占那块像素的线性亮度（分级**之前**的 ramps 输入拿不到，
    //   这里读的就是写到 PNG 的那个值）。判据：如果它也是一片常数 ⇒ 结构在 sky 这一档被抹平；
    //   如果它有分布 ⇒ 是我的显示/曝光把它压平了。
    let mut pixels: Vec<f32> = Vec::new();
    for layer in 0..6usize {
        for y in 0..face {
            for x in 0..face {
                let source = ((layer * face + y) * face + x) * 4 * 2;
                let bits = u16::from_le_bytes([sky.bytes[source], sky.bytes[source + 1]]);
                let value = half_to_f32(bits);
                if value > 1e-6 {
                    pixels.push(value);
                }
            }
        }
    }
    pixels.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    if !pixels.is_empty() {
        let pick = |q: f64| pixels[((pixels.len() - 1) as f64 * q) as usize];
        println!(
            "图像亮度（非零 {} / {} 像素）：min {:.3e} | p10 {:.3e} | p50 {:.3e} | p90 {:.3e} | max {:.3e}",
            pixels.len(),
            face * face * 6,
            pixels[0],
            pick(0.10),
            pick(0.50),
            pick(0.90),
            pixels[pixels.len() - 1]
        );
    }
    println!("写出 target/probe/sphere.png（六面 3×2 拼图，每面 {FACE}）");
    Ok(())
}

/// **标准软面球**：解析半径剖面（内部恒 1、软面线性、外面精确 0）。
fn soft_sphere() -> VolumeData {
    let shell = px_volume_schema::volume::Shell::new(INNER, OUTER);
    let mut data = Vec::new();
    for face in 0..6u32 {
        for layer in 0..LAYERS {
            let radius = shell.radius_of(layer as f32 / (LAYERS - 1) as f32);
            for t in 0..RES {
                for s in 0..RES {
                    let direction = px_protocol::art::cube_direction(
                        face,
                        (s as f32 + 0.5) / RES as f32,
                        (t as f32 + 0.5) / RES as f32,
                    );
                    let point = [
                        direction[0] * radius,
                        direction[1] * radius,
                        direction[2] * radius,
                    ];
                    let offset = [
                        point[0] - CENTRE[0],
                        point[1] - CENTRE[1],
                        point[2] - CENTRE[2],
                    ];
                    let distance =
                        (offset[0] * offset[0] + offset[1] * offset[1] + offset[2] * offset[2])
                            .sqrt();
                    let profile = ((RADIUS + SOFT - distance) / SOFT).clamp(0.0, 1.0);
                    let value = profile * DENSITY;
                    data.push(value);
                }
            }
        }
    }
    println!(
        "软面球：{} 面 × {RES}² × {LAYERS} 层｜值域 {:.3}..{:.3}",
        6,
        data.iter().copied().fold(f32::MAX, f32::min),
        data.iter().copied().fold(0.0_f32, f32::max)
    );
    VolumeData {
        res: RES,
        layers: LAYERS,
        inner: INNER,
        outer: OUTER,
        data,
    }
}

/// 半精度 → f32（不值得为一个探针引 `half` crate）。
fn half_to_f32(bits: u16) -> f32 {
    let sign = (bits as u32 >> 15) << 31;
    let exponent = (bits as u32 >> 10) & 0x1f;
    let mantissa = bits as u32 & 0x3ff;
    let out = if exponent == 0 {
        if mantissa == 0 {
            sign
        } else {
            let mut e = 127 - 15 + 1;
            let mut m = mantissa;
            while m & 0x400 == 0 {
                m <<= 1;
                e -= 1;
            }
            sign | (e << 23) | ((m & 0x3ff) << 13)
        }
    } else if exponent == 0x1f {
        sign | 0x7f80_0000 | (mantissa << 13)
    } else {
        sign | ((exponent + 127 - 15) << 23) | (mantissa << 13)
    };
    f32::from_bits(out)
}

/// `--dump`：小球 + CPU 烘焙 ⇒ 打印球内发射通道的统计（判"遮蔽有没有在塑造亮度"）。
fn dump() -> Result<(), String> {
    const SIDE: u32 = 32;
    const LAYERS: u32 = 16;
    let shell = px_volume_schema::volume::Shell::new(INNER, OUTER);
    let mut data = Vec::new();
    for face in 0..6u32 {
        for layer in 0..LAYERS {
            let radius = shell.radius_of(layer as f32 / (LAYERS - 1) as f32);
            for t in 0..SIDE {
                for s in 0..SIDE {
                    let direction = px_protocol::art::cube_direction(
                        face,
                        (s as f32 + 0.5) / SIDE as f32,
                        (t as f32 + 0.5) / SIDE as f32,
                    );
                    let point = [
                        direction[0] * radius,
                        direction[1] * radius,
                        direction[2] * radius,
                    ];
                    let offset = [
                        point[0] - CENTRE[0],
                        point[1] - CENTRE[1],
                        point[2] - CENTRE[2],
                    ];
                    let distance =
                        (offset[0] * offset[0] + offset[1] * offset[1] + offset[2] * offset[2])
                            .sqrt();
                    let profile = ((RADIUS + SOFT - distance) / SOFT).clamp(0.0, 1.0);
                    data.push(profile * DENSITY);
                }
            }
        }
    }
    let sphere = VolumeData {
        res: SIDE,
        layers: LAYERS,
        inner: INNER,
        outer: OUTER,
        data,
    };
    let stars_params = StarsParams {
        count: 1_300,
        gas_biased: true,
        gas_contrast: 1.5,
        gas_floor: 0.35,
        gas_depth: 0.0,
        clump_count: 0,
        clump_share: 0.0,
        star_tint: [0.12, 0.30, 1.0],
        ..Default::default()
    };
    let emission_params = EmissionParams {
        emission_gain: 0.0,
        glow_gain: 0.0,
        starlight_gain: 6.4e-4,
        starlight_radius: 0.4,
        starlight_soft: 0.02,
        starlight_steps: 24,
        starlight_max: 16,
        shadow_gain: 10.0,
        extinction: [10.0, 10.0, 10.0],
        glow_tint: [1.0, 0.03, 0.03],
        ..Default::default()
    };
    let stars = px_volume_alg::bake_stars(&stars_params, Some(&sphere))?;
    println!("小球里落了 {} 颗星", stars.count());
    let emission = px_volume_gpu_op::bake_emission(&sphere, &stars, &emission_params)?;

    // 球内（密度 > 0.5）的发射通道 R 与 σ_R 的分布
    let width = SIDE as usize * 6;
    let mut emit: Vec<f32> = Vec::new();
    let mut sigma: Vec<f32> = Vec::new();
    let rows = (6 * LAYERS * SIDE) as usize;
    for row in 0..rows {
        for s in 0..SIDE as usize {
            let at = row * width + s * 6;
            let density = px_volume_alg::density::sample_world(&sphere, {
                let face = (row / (LAYERS as usize * SIDE as usize)) as u32;
                let rest = row % (LAYERS as usize * SIDE as usize);
                let layer = (rest / SIDE as usize) as u32;
                let t = (rest % SIDE as usize) as u32;
                let shell = px_volume_schema::volume::Shell::new(INNER, OUTER);
                let radius = shell.radius_of(layer as f32 / (LAYERS - 1) as f32);
                let direction = px_protocol::art::cube_direction(
                    face,
                    (s as f32 + 0.5) / SIDE as f32,
                    (t as f32 + 0.5) / SIDE as f32,
                );
                [
                    direction[0] * radius,
                    direction[1] * radius,
                    direction[2] * radius,
                ]
            });
            if density > 0.5 {
                emit.push(emission.data[at]);
                sigma.push(emission.data[at + 3]);
            }
        }
    }
    let stat = |values: &mut Vec<f32>| -> (f32, f32, f32, f32) {
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let pick = |q: f64| values[((values.len() - 1) as f64 * q) as usize];
        (values[0], pick(0.5), pick(0.9), values[values.len() - 1])
    };
    let (e_min, e_p50, e_p90, e_max) = stat(&mut emit);
    let (s_min, s_p50, _, s_max) = stat(&mut sigma);
    println!("球内体素 {} 个", emit.len());
    println!(
        "  emit R: min {:.3e} | p50 {:.3e} | p90 {:.3e} | max {:.3e}",
        e_min, e_p50, e_p90, e_max
    );
    println!(
        "  sigma_R: min {:.3} | p50 {:.3} | max {:.3}",
        s_min, s_p50, s_max
    );
    println!(
        "  ⇒ p90/p50 = {:.2}（1 = 一片常数 ⇒ 遮蔽没在塑造亮度 ✗）",
        e_p90 / e_p50.max(1e-12)
    );
    Ok(())
}
