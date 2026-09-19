//! 程序化贴图与网格的生成：色板贴图、发光贴图、覆盖度立方图、星空、环带与环网格。
//!
//! 它是从渲染器（`px_render/src/planet.rs` 的 `Palette` / `surface_textures` /
//! `mip_chain` / `pole_cap_filter` / `star_cube` / `ring_mesh` / `ring_image` 与
//! `px_render/src/clouds.rs` 的 `coverage_image` / `half_from_f32`）**逐字搬过来**的：
//! 像素公式、四舍五入、`powf(1.0 / 2.2)`、mip 的 `% w` 环绕与 `.min(h - 1)` 夹取、
//! `pole_cap_filter` 的权重、`mip_chain_cube` 的按面分块，一个字符都没改。
//! 搬运的判据是**逐字节相同**（`target/legacy-gen` ↔ `px_graphs --bin genprobe`）。
//!
//! 只换了两样东西（都是"取数的地方"，不是算法）：
//!   · 场从渲染器的 `Field`（`data` / `min` / `max` / `texel_latitude`）换成
//!     `px_graph::field::Field`（`data` / `stats()`；取数口径逐字照抄，见 [`load_field`]）；
//!   · 产物从 bevy 的 `Image` / `Mesh` 换成 [`TextureData`] / `px_protocol::art::MeshData`
//!     —— 写进 `Image.data` 的那串字节原样就是 [`TextureData::bytes`]。
//!
//! 采样器（`with_wrapping` 的 address mode / filter）与 `TextureViewDescriptor`
//! （cube 视图）**没有搬**：它们说的是"怎么采"不是"是什么"，进不了字节。
//! 渲染器照 [`TextureData::layers`] / `format` 重建 Image 时按产物那一档设即可。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use px_protocol::art::{
    ArtBundle, AssetKind, AssetManifest, CUBE_COLUMNS, CUBE_FACES, Domain, TextureShape,
};
use px_protocol::stream::{self, Frame};
use px_protocol::wire::{Blob, BlobHeader, DType};

pub use px_protocol::art::{MeshData, TextureFormat};

use px_field_schema::field::Field;
use px_graph_schema::Key;

// ---------------------------------------------------------------------------
// 色板
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Palette {
    Rocky,
    Gas,
    Ice,
    Lava,
    Desert,
}

impl Palette {
    pub const NAMES: [&'static str; 5] = ["rocky", "gas", "ice", "lava", "desert"];

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "rocky" => Some(Self::Rocky),
            "gas" => Some(Self::Gas),
            "ice" => Some(Self::Ice),
            "lava" => Some(Self::Lava),
            "desert" => Some(Self::Desert),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Rocky => "rocky",
            Self::Gas => "gas",
            Self::Ice => "ice",
            Self::Lava => "lava",
            Self::Desert => "desert",
        }
    }

    /// 大气的三件套：**色调（线性 RGB）+ 密度 + 软度**。
    ///
    /// 渲染器那份返回的是 bevy 的 `LinearRgba`；`px_graph` 不认识 bevy，所以色调那一项
    /// 落成 `[f32; 3]`。三个通道的**数值**与渲染器逐字相同，取用侧自己包成 `LinearRgba`
    /// （`LinearRgba::rgb(tint[0], tint[1], tint[2])`）。
    pub fn atmosphere(self) -> ([f32; 3], f32, f32) {
        match self {
            Self::Rocky => ([0.44, 0.64, 0.98], 0.300, 0.50),
            Self::Gas => ([1.00, 0.86, 0.62], 0.430, 0.40),
            Self::Ice => ([0.66, 0.87, 1.00], 0.270, 0.60),
            Self::Lava => ([1.00, 0.46, 0.20], 0.340, 0.45),
            Self::Desert => ([0.97, 0.79, 0.55], 0.340, 0.46),
        }
    }

    pub fn defaults(self) -> (f32, f32, f32) {
        match self {
            Self::Rocky => (0.075, 0.520, 0.0),
            Self::Gas => (0.010, 0.450, 2.35),
            Self::Ice => (0.055, 0.500, 0.0),
            Self::Lava => (0.095, 0.480, 0.0),
            Self::Desert => (0.085, 0.520, 0.0),
        }
    }
}

// ---------------------------------------------------------------------------
// 色带与着色（逐字搬自 px_render/src/planet.rs）
// ---------------------------------------------------------------------------

fn ramp(stops: &[(f32, [f32; 3])], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    let mut previous = stops[0];
    for stop in stops {
        if t <= stop.0 {
            let span = stop.0 - previous.0;
            let k = if span.abs() < f32::EPSILON {
                0.0
            } else {
                (t - previous.0) / span
            };
            return [
                previous.1[0] + (stop.1[0] - previous.1[0]) * k,
                previous.1[1] + (stop.1[1] - previous.1[1]) * k,
                previous.1[2] + (stop.1[2] - previous.1[2]) * k,
            ];
        }
        previous = *stop;
    }
    stops[stops.len() - 1].1
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

const GAS: &[(f32, [f32; 3])] = &[
    (0.00, [0.286, 0.208, 0.145]),
    (0.30, [0.545, 0.427, 0.318]),
    (0.55, [0.804, 0.729, 0.616]),
    (0.78, [0.902, 0.812, 0.663]),
    (1.00, [0.616, 0.475, 0.353]),
];

const WATER: &[(f32, [f32; 3])] = &[
    (0.00, [0.006, 0.020, 0.070]),
    (0.55, [0.031, 0.109, 0.259]),
    (1.00, [0.153, 0.353, 0.478]),
];

const LAND: &[(f32, [f32; 3])] = &[
    (0.00, [0.706, 0.663, 0.502]),
    (0.05, [0.259, 0.435, 0.216]),
    (0.32, [0.169, 0.325, 0.153]),
    (0.60, [0.404, 0.376, 0.318]),
    (0.82, [0.612, 0.596, 0.573]),
    (1.00, [0.965, 0.973, 1.000]),
];

const FROZEN: &[(f32, [f32; 3])] = &[
    (0.00, [0.086, 0.153, 0.227]),
    (0.55, [0.278, 0.443, 0.565]),
    (1.00, [0.686, 0.804, 0.867]),
];

const SHEET: &[(f32, [f32; 3])] = &[
    (0.00, [0.549, 0.667, 0.741]),
    (0.45, [0.749, 0.847, 0.902]),
    (1.00, [0.965, 0.988, 1.000]),
];

const LAVA_ROCK: &[(f32, [f32; 3])] = &[
    (0.00, [0.035, 0.027, 0.027]),
    (0.35, [0.086, 0.063, 0.055]),
    (0.60, [0.176, 0.125, 0.098]),
    (0.82, [0.290, 0.208, 0.157]),
    (1.00, [0.427, 0.353, 0.310]),
];

const BASIN: &[(f32, [f32; 3])] = &[
    (0.00, [0.165, 0.122, 0.094]),
    (1.00, [0.376, 0.271, 0.184]),
];

const DUNE: &[(f32, [f32; 3])] = &[
    (0.00, [0.310, 0.196, 0.122]),
    (0.18, [0.475, 0.302, 0.173]),
    (0.42, [0.706, 0.510, 0.290]),
    (0.70, [0.816, 0.663, 0.435]),
    (1.00, [0.882, 0.796, 0.651]),
];

fn shade(
    palette: Palette,
    height: f32,
    sea: f32,
    latitude: f32,
    wobble: f32,
) -> ([f32; 3], [f32; 3]) {
    let sea = sea.clamp(0.02, 0.98);
    let land_t = ((height - sea) / (1.0 - sea)).clamp(0.0, 1.0);
    let water_t = (height / sea).clamp(0.0, 1.0);

    match palette {
        Palette::Rocky => {
            let mut color = if height < sea {
                ramp(WATER, water_t)
            } else {
                ramp(LAND, land_t)
            };
            let cap = ((latitude - 0.80) / 0.20).clamp(0.0, 1.0);
            color = mix(color, [0.941, 0.965, 1.000], cap * 0.92);
            (color, [0.0, 0.0, 0.0])
        }
        Palette::Gas => {
            let bands = (latitude * 17.0 + wobble * 0.55).sin() * 0.5 + 0.5;
            let detail = (latitude * 47.0 + wobble * 0.90).sin() * 0.5 + 0.5;
            let t = (bands * 0.72 + detail * 0.28).clamp(0.0, 1.0);
            let mut color = ramp(GAS, t);
            let storm = ((wobble * 1.7).sin() * 0.5 + 0.5).powf(6.0);
            color = mix(color, [0.784, 0.514, 0.353], storm * 0.45);
            let pole = ((latitude - 0.74) / 0.26).clamp(0.0, 1.0);
            color = mix(color, [0.180, 0.145, 0.129], pole * 0.85);
            (color, [0.0, 0.0, 0.0])
        }
        Palette::Ice => {
            let mut color = if height < sea {
                ramp(FROZEN, water_t)
            } else {
                ramp(SHEET, land_t)
            };
            let crack = ((sea * 0.75 - height) / (sea * 0.75).max(1e-3)).clamp(0.0, 1.0);
            color = mix(color, [0.098, 0.169, 0.259], crack * 0.65);
            let cap = ((latitude - 0.62) / 0.38).clamp(0.0, 1.0);
            color = mix(color, [0.996, 1.000, 1.000], cap * 0.9);
            (color, [0.0, 0.0, 0.0])
        }
        Palette::Lava => {
            let rock = ramp(LAVA_ROCK, land_t);
            let crack = (1.0 - water_t).clamp(0.0, 1.0).powf(1.4);
            let glow = [crack * 1.000, crack * 0.330, crack * 0.060];
            let color = mix(rock, [0.996, 0.443, 0.094], crack * 0.92);
            (color, glow)
        }
        Palette::Desert => {
            let mut color = if height < sea {
                ramp(BASIN, water_t)
            } else {
                ramp(DUNE, land_t)
            };
            let strata = (height * 37.0).sin() * 0.5 + 0.5;
            color = mix(color, [0.529, 0.290, 0.196], strata * land_t * 0.16);
            let cap = ((latitude - 0.88) / 0.12).clamp(0.0, 1.0);
            color = mix(color, [0.902, 0.925, 0.941], cap * 0.45);
            (color, [0.0, 0.0, 0.0])
        }
    }
}

fn push_color(bytes: &mut Vec<u8>, color: [f32; 3]) {
    bytes.push((color[0].clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0).round() as u8);
    bytes.push((color[1].clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0).round() as u8);
    bytes.push((color[2].clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0).round() as u8);
    bytes.push(255);
}

// ---------------------------------------------------------------------------
// 场的那两个小助手（`px_graph::field::Field` 上没有，照渲染器那份的数学原样搬）
// ---------------------------------------------------------------------------

/// `Field::normalized`：`px_graph` 把 `min` / `max` 放在 `stats()` 里，口径一样。
fn normalized(min: f32, max: f32, value: f32) -> f32 {
    let span = max - min;
    if span.abs() < f32::EPSILON {
        0.5
    } else {
        (value - min) / span
    }
}

/// `Field::texel_latitude`（逐字照抄，含 Equirect 与"其余投影走方向"这两支）。
pub fn texel_latitude(field: &Field, x: u32, y: u32) -> f32 {
    match field.projection {
        Domain::Equirect => {
            let v = y as f32 / (field.height.max(2) - 1) as f32;
            ((v - 0.5).abs() * 2.0).clamp(0.0, 1.0)
        }
        _ => {
            px_protocol::art::direction_at(field.projection, field.width, field.height, x, y)[1].abs()
        }
    }
}

// ---------------------------------------------------------------------------
// 贴图载荷
// ---------------------------------------------------------------------------

/// 一份贴图的**全部字节**：整条 mip 链，与渲染器今天写进 `Image.data` 的那串逐字节相同。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureData {
    pub width: u32,
    pub height: u32,
    pub layers: u32,
    pub levels: u32,
    pub format: TextureFormat,
    pub bytes: Vec<u8>,
}

impl TextureData {
    pub fn shape(&self) -> TextureShape {
        TextureShape {
            width: self.width,
            height: self.height,
            layers: self.layers,
            levels: self.levels,
            format: self.format,
        }
    }

    /// 唯一的构造口：自检「载荷字节数 = 形状算出来的整条 mip 链字节数」。
    /// 少一级 mip、多层一层、位深写错，都会在这里当场炸，而不是等到渲染器那边采样出错。
    fn new(
        width: u32,
        height: u32,
        layers: u32,
        levels: u32,
        format: TextureFormat,
        bytes: Vec<u8>,
    ) -> Self {
        let data = Self {
            width,
            height,
            layers,
            levels,
            format,
            bytes,
        };
        let expected = data.shape().chain_bytes();
        assert_eq!(
            data.bytes.len(),
            expected,
            "贴图载荷与形状不符：{}×{}×{} 层、{} 级、{:?} 应当是 {} 字节，实际 {} 字节",
            data.width,
            data.height,
            data.layers,
            data.levels,
            data.format,
            expected,
            data.bytes.len(),
        );
        data
    }
}

// ---------------------------------------------------------------------------
// 逐 texel 上色 + 整条 mip 链
// ---------------------------------------------------------------------------

/// 色板 → (颜色贴图, 可选发光贴图, 审计文本)。
///
/// 审计文本照抄今天那条（缓存命中时要重放它，仪器不能因为走了缓存就哑掉）：
/// 它是**值**不是副作用，所以这里返回、不打印。
pub fn surface_color(
    field: &Field,
    palette: Palette,
    sea_level: f32,
) -> (TextureData, Option<TextureData>, String) {
    let stats = field.stats();
    let mut color = Vec::with_capacity((field.width * field.height * 4) as usize);
    let mut glow = Vec::with_capacity((field.width * field.height * 4) as usize);

    for y in 0..field.height {
        for x in 0..field.width {
            let raw = field.data[(y * field.width + x) as usize];
            let height = normalized(stats.min, stats.max, raw);
            let latitude = texel_latitude(field, x, y);
            let (base, emit) = shade(palette, height, sea_level, latitude, raw * 6.283);
            push_color(&mut color, base);
            push_color(&mut glow, emit);
        }
    }

    if field.projection == Domain::Equirect {
        pole_cap_filter(&mut color, field.width, field.height);
    }
    let color_image = image_from(
        field.width,
        field.height,
        color.clone(),
        field.projection == Domain::Cube,
    );

    let texels = color.len() / 4;
    let mut sums = [0.0_f64; 3];
    for pixel in color.chunks_exact(4) {
        for channel in 0..3 {
            sums[channel] += pixel[channel] as f64;
        }
    }
    let white = color
        .chunks_exact(4)
        .filter(|pixel| pixel[0] > 200 && pixel[1] > 200 && pixel[2] > 200)
        .count();
    let mip_levels = (field.width.max(field.height).max(1) as f32).log2().floor() as u32 + 1;
    let mip_bytes = (field.width * field.height) as f64 * 4.0 * 4.0 / 3.0;
    let audit = format!(
        "         贴图 {}{}×{}：平均 RGB ({:.0},{:.0},{:.0})，近白像素 {:.1}%，mip {mip_levels} 级（约 {:.1} MB）",
        palette.name(),
        field.width,
        field.height,
        sums[0] / texels as f64,
        sums[1] / texels as f64,
        sums[2] / texels as f64,
        white as f64 * 100.0 / texels as f64,
        mip_bytes / 1e6,
    );

    let glow_image = match palette {
        Palette::Lava => {
            if field.projection == Domain::Equirect {
                pole_cap_filter(&mut glow, field.width, field.height);
            }
            Some(image_from(
                field.width,
                field.height,
                glow,
                field.projection == Domain::Cube,
            ))
        }
        _ => None,
    };
    (color_image, glow_image, audit)
}

// ---------------------------------------------------------------------------
// 覆盖度立方图
// ---------------------------------------------------------------------------

fn half_from_f32(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32;
    let mantissa = bits & 0x007f_ffff;

    if exponent == 0xff {
        let payload = if mantissa == 0 { 0 } else { 0x0200 };
        return sign | 0x7c00 | payload;
    }

    let unbiased = exponent - 127 + 15;
    if unbiased >= 0x1f {
        return sign | 0x7c00;
    }
    if unbiased <= 0 {
        if unbiased < -10 {
            return sign;
        }
        let mantissa = mantissa | 0x0080_0000;
        let shift = (14 - unbiased) as u32;
        let mut half = (mantissa >> shift) as u16;
        if (mantissa >> (shift - 1)) & 1 == 1 {
            half += 1;
        }
        return sign | half;
    }

    let mut half = ((unbiased as u32) << 10) as u16 | (mantissa >> 13) as u16;
    if mantissa & 0x1000 != 0 {
        half += 1;
    }
    sign | half
}

/// 覆盖度 + 三轴梯度 → RGBA16F 立方图（6 层，1 级 mip）。校验逐字照搬。
pub fn coverage_cube(mask: &Field, slopes: [&Field; 3]) -> Result<TextureData, String> {
    let field = mask;
    if field.projection != Domain::CubeMap {
        return Err(format!(
            "云覆盖度需要 CubeMap 产物，这份是 {:?}",
            field.projection
        ));
    }
    let face = field.width.max(1);
    if field.height != face * CUBE_FACES {
        return Err(format!(
            "云覆盖度的行数应当是 {face} × {CUBE_FACES} = {}，实际 {}",
            face * CUBE_FACES,
            field.height
        ));
    }
    for slope in slopes {
        if slope.projection != Domain::CubeMap || slope.width != face || slope.height != field.height
        {
            return Err(format!(
                "云的梯度场必须和覆盖度同形（{face}×{}），这份是 {:?} {}×{}",
                field.height,
                slope.projection,
                slope.width,
                slope.height
            ));
        }
    }

    let mut bytes = Vec::with_capacity(field.data.len() * 8);
    for index in 0..field.data.len() {
        for value in [
            field.data[index],
            slopes[0].data[index],
            slopes[1].data[index],
            slopes[2].data[index],
        ] {
            bytes.extend_from_slice(&half_from_f32(value).to_le_bytes());
        }
    }

    Ok(TextureData::new(
        face,
        face,
        CUBE_FACES,
        1,
        TextureFormat::Rgba16Float,
        bytes,
    ))
}

// ---------------------------------------------------------------------------
// 星空
// ---------------------------------------------------------------------------

/// 星空：`face` 面的 6 层立方图（1 级 mip，与渲染器 `star_cube` 写进 `Image.data` 的相同）。
pub fn stars(face: u32) -> TextureData {
    let size = face.max(4);
    let mut data = Vec::with_capacity((size * size * 6 * 4) as usize);

    for face_index in 0..CUBE_FACES {
        for y in 0..size {
            for x in 0..size {
                let s = (x as f32 + 0.5) / size as f32;
                let t = (y as f32 + 0.5) / size as f32;
                let direction = px_protocol::art::cube_direction(face_index, s, t);
                let mut hash = (face_index as u64)
                    .wrapping_mul(0x9e37_79b9)
                    .wrapping_add((x as u64).wrapping_mul(0x85eb_ca6b))
                    .wrapping_mul(0xc2b2_ae35)
                    .wrapping_add((y as u64).wrapping_mul(0x27d4_eb2f));
                hash ^= hash >> 15;
                hash = hash.wrapping_mul(0x2545_f491);
                hash ^= hash >> 13;
                let value = (hash & 0xffff) as f32 / 65535.0;
                let brightness = if value > 0.99900 {
                    1.0
                } else if value > 0.99750 {
                    0.55
                } else if value > 0.99550 {
                    0.22
                } else {
                    0.0
                };
                let blue = 0.86 + 0.14 * ((hash >> 16) as f32 / 65535.0);
                let warp = 0.90 + 0.10 * (direction[1] * 0.5 + 0.5);
                let level = (brightness * 255.0 * warp) as u8;
                data.push(level);
                data.push(level);
                data.push((level as f32 * blue) as u8);
                data.push(255);
            }
        }
    }

    TextureData::new(size, size, CUBE_FACES, 1, TextureFormat::Rgba8Srgb, data)
}

// ---------------------------------------------------------------------------
// 环：贴图与网格
// ---------------------------------------------------------------------------

/// 环带贴图（从 `ring_image` 逐字搬）。
pub fn ring_band(width: u32, height: u32) -> TextureData {
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    for _row in 0..height {
        for x in 0..width {
            let t = x as f32 / (width.max(2) - 1) as f32;
            let mut density = 0.34 + 0.32 * (t * 47.0).sin().abs();
            density *= 0.58 + 0.42 * (t * 13.0 + 0.7).sin().abs();
            density = (density + 0.18 * ((t * 211.0).sin() * 0.5 + 0.5)).clamp(0.0, 1.0);
            if (t - 0.635).abs() < 0.030 {
                density *= 0.08;
            }
            if (t - 0.340).abs() < 0.012 {
                density *= 0.34;
            }
            let edge = (t / 0.07).clamp(0.0, 1.0) * ((1.0 - t) / 0.10).clamp(0.0, 1.0);
            let alpha = (density * edge).clamp(0.0, 1.0);
            let shade = 0.70 + 0.30 * (0.5 + 0.5 * (t * 61.0).sin());
            push_color(
                &mut data,
                [0.878 * shade, 0.827 * shade, 0.729 * shade],
            );
            let last = data.len() - 1;
            data[last] = (alpha * 240.0) as u8;
        }
    }
    image_from(width, height, data, false)
}

/// 环网格（从 `ring_mesh` 逐字搬）。
pub fn ring_mesh(inner: f32, outer: f32, segments: u32) -> MeshData {
    let mut positions = Vec::with_capacity((segments as usize + 1) * 2);
    let mut normals = Vec::with_capacity((segments as usize + 1) * 2);
    let mut uvs = Vec::with_capacity((segments as usize + 1) * 2);
    let mut indices = Vec::with_capacity(segments as usize * 6);

    for index in 0..=segments {
        let angle = index as f32 / segments as f32 * std::f32::consts::TAU;
        let (sin, cos) = angle.sin_cos();
        let v = index as f32 / segments as f32;
        positions.push([cos * inner, 0.0, sin * inner]);
        positions.push([cos * outer, 0.0, sin * outer]);
        normals.push([0.0, 1.0, 0.0]);
        normals.push([0.0, 1.0, 0.0]);
        uvs.push([0.0, v]);
        uvs.push([1.0, v]);
    }
    for index in 0..segments {
        let base = index * 2;
        indices.extend_from_slice(&[base, base + 2, base + 1, base + 1, base + 2, base + 3]);
    }

    MeshData {
        positions: flatten3(&positions),
        normals: flatten3(&normals),
        uvs: flatten2(&uvs),
        indices,
    }
}

fn flatten3(values: &[[f32; 3]]) -> Vec<f32> {
    let mut out = Vec::with_capacity(values.len() * 3);
    for value in values {
        out.extend_from_slice(value);
    }
    out
}

fn flatten2(values: &[[f32; 2]]) -> Vec<f32> {
    let mut out = Vec::with_capacity(values.len() * 2);
    for value in values {
        out.extend_from_slice(value);
    }
    out
}

// ---------------------------------------------------------------------------
// mip 链与极冠（逐字搬自 px_render/src/planet.rs）
// ---------------------------------------------------------------------------

fn mip_chain_cube(width: u32, height: u32, base: &[u8]) -> (Vec<u8>, u32) {
    let mut chain = base.to_vec();
    let mut levels = 1_u32;
    let mut source = base.to_vec();
    let (mut w, mut h) = (width.max(1), height.max(1));

    while w > 1 && h > 1 {
        let next_w = (w / 2).max(1);
        let next_h = (h / 2).max(1);
        let cell = (w / CUBE_COLUMNS).max(1);
        let next_cell = (next_w / CUBE_COLUMNS).max(1);
        let mut next = vec![0_u8; (next_w * next_h * 4) as usize];

        for face in 0..CUBE_FACES {
            let column = face % CUBE_COLUMNS;
            let row = face / CUBE_COLUMNS;
            let left = column * cell;
            let top = row * cell;
            for y in 0..next_cell {
                for x in 0..next_cell {
                    let sample_x = left + (x * 2).min(cell - 1);
                    let sample_y = top + (y * 2).min(cell - 1);
                    let sample_x1 = (sample_x + 1).min(left + cell - 1);
                    let sample_y1 = (sample_y + 1).min(top + cell - 1);
                    let mut sums = [0_u32; 4];
                    for (ax, ay) in [
                        (sample_x, sample_y),
                        (sample_x1, sample_y),
                        (sample_x, sample_y1),
                        (sample_x1, sample_y1),
                    ] {
                        let index = (ay * w + ax) as usize * 4;
                        for channel in 0..4 {
                            sums[channel] += source[index + channel] as u32;
                        }
                    }
                    let dx = column * next_cell + x;
                    let dy = row * next_cell + y;
                    if dx < next_w && dy < next_h {
                        let out = (dy * next_w + dx) as usize * 4;
                        for channel in 0..4 {
                            next[out + channel] = (sums[channel] / 4) as u8;
                        }
                    }
                }
            }
        }

        chain.extend_from_slice(&next);
        source = next;
        w = next_w;
        h = next_h;
        levels += 1;
    }

    (chain, levels)
}

fn mip_chain(width: u32, height: u32, base: &[u8]) -> (Vec<u8>, u32) {
    let mut chain = base.to_vec();
    let mut levels = 1_u32;
    let mut source = base.to_vec();
    let (mut w, mut h) = (width.max(1), height.max(1));

    while w > 1 || h > 1 {
        let next_w = (w / 2).max(1);
        let next_h = (h / 2).max(1);
        let mut next = vec![0_u8; (next_w * next_h * 4) as usize];

        for y in 0..next_h {
            for x in 0..next_w {
                let mut sums = [0_u32; 4];
                for step_y in 0..2 {
                    for step_x in 0..2 {
                        let sample_x = ((x * 2 + step_x) % w) as usize;
                        let sample_y = ((y * 2 + step_y).min(h - 1)) as usize;
                        let index = (sample_y * w as usize + sample_x) * 4;
                        for channel in 0..4 {
                            sums[channel] += source[index + channel] as u32;
                        }
                    }
                }
                let out = ((y * next_w + x) * 4) as usize;
                for channel in 0..4 {
                    next[out + channel] = (sums[channel] / 4) as u8;
                }
            }
        }

        chain.extend_from_slice(&next);
        source = next;
        w = next_w;
        h = next_h;
        levels += 1;
    }

    (chain, levels)
}

fn pole_cap_rows(height: u32) -> u32 {
    (height / 32).max(2)
}

fn pole_cap_filter(pixels: &mut [u8], width: u32, height: u32) {
    let rows = pole_cap_rows(height);
    for offset in 0..rows {
        let weight = 1.0 - offset as f32 / rows as f32;
        for row in [offset, height - 1 - offset] {
            let start = (row * width) as usize * 4;
            let end = start + width as usize * 4;
            let Some(slice) = pixels.get_mut(start..end) else {
                continue;
            };
            let mut mean = [0.0_f32; 3];
            for pixel in slice.chunks_exact(4) {
                for channel in 0..3 {
                    mean[channel] += pixel[channel] as f32;
                }
            }
            for value in mean.iter_mut() {
                *value /= width as f32;
            }
            for pixel in slice.chunks_exact_mut(4) {
                for channel in 0..3 {
                    pixel[channel] = (pixel[channel] as f32 * (1.0 - weight)
                        + mean[channel] * weight)
                        .round()
                        .clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
}

/// 像素 → 整条 mip 链。渲染器那份在这里还顺手包了个 bevy `Image`；字节与这里相同。
fn image_from(width: u32, height: u32, data: Vec<u8>, cube: bool) -> TextureData {
    let (chain, levels) = if cube {
        mip_chain_cube(width, height, &data)
    } else {
        mip_chain(width, height, &data)
    };
    TextureData::new(
        width,
        height,
        1,
        levels,
        TextureFormat::Rgba8Srgb,
        chain,
    )
}

// ---------------------------------------------------------------------------
// 产物读取：按清单里的键读一份场（读法与渲染器 `load_field` 相同）
// ---------------------------------------------------------------------------

/// 读一份场产物。投影由清单里的 `AssetKind` 决定 —— 与渲染器 `planet::load_field`
/// 同一套映射（`Field2D` → Equirect、`OctahedralField` → Octahedral、
/// `CubeField` → Cube、`CubeMap` → CubeMap）。**这一步不能省**：投影决定
/// `texel_latitude` 走哪一支、颜色贴图走 `mip_chain` 还是 `mip_chain_cube`、
/// 要不要 `pole_cap_filter`。
pub fn load_field(path: &str) -> Result<Field, String> {
    let bytes = std::fs::read(path).map_err(|err| format!("读不到 {path}：{err}"))?;
    let frames = stream::read_stream(&mut bytes.as_slice()).map_err(|err| err.to_string())?;

    let mut kind = None;
    for frame in &frames {
        if let Frame::Art(bundle) = frame {
            if let Some(asset) = bundle.assets.first() {
                kind = Some(asset.kind);
            }
        }
    }
    let projection = match kind {
        Some(AssetKind::Field2D) => Domain::Equirect,
        Some(AssetKind::OctahedralField) => Domain::Octahedral,
        Some(AssetKind::CubeField) => Domain::Cube,
        Some(AssetKind::CubeMap) => Domain::CubeMap,
        Some(other) => {
            return Err(format!(
                "{path} 是 {other:?}，星球需要 Field2D / OctahedralField / CubeField / CubeMap 产物"
            ));
        }
        None => return Err(format!("{path} 里没有 Art 帧")),
    };

    let blob = frames
        .iter()
        .find_map(|frame| match frame {
            Frame::Blob(blob) => Some(blob),
            _ => None,
        })
        .ok_or_else(|| format!("{path} 里没有数据块"))?;
    if blob.header.shape.len() != 2 {
        return Err(format!("{path} 的场不是二维的：{:?}", blob.header.shape));
    }

    let mut field = Field::from_blob(blob).map_err(|err| err.to_string())?;
    field.projection = projection;
    Ok(field)
}

// ---------------------------------------------------------------------------
// 落盘：内容寻址（键 = 完整产物字节的 blake3）
// ---------------------------------------------------------------------------

/// 一份落进 CAS 的生成物。
#[derive(Debug, Clone)]
pub struct Generated {
    pub id: String,
    pub key: Key,
    pub path: PathBuf,
    pub bytes: u64,
    /// 这份内容在 CAS 里已经有了（没重写文件）。
    pub hit: bool,
    pub millis: u64,
}

/// 把一份生成物写进 CAS。
///
/// **键 = 内容**：键是对完整产物字节（清单帧 + 载荷帧，就是 `stream::write_stream`
/// 的输出）算的 blake3，路径是 `px_protocol::scene::cas_path(cache_root, hex)`。
/// 文件已存在 ⇒ `hit = true` 且一个字节都不重写。
fn write_cas(
    root: &Path,
    id: &str,
    kind: AssetKind,
    params: BTreeMap<String, f64>,
    blobs: Vec<Blob>,
) -> Result<Generated, String> {
    let started = Instant::now();
    let fingerprint = crate::payload_fingerprint(id, &blobs);
    let bundle = ArtBundle {
        assets: vec![AssetManifest {
            id: id.to_string(),
            kind,
            params,
            blobs: blobs.iter().map(|blob| blob.header.clone()).collect(),
            fingerprint,
            cameras: Vec::new(),
        }],
    };
    let mut frames = vec![Frame::Art(bundle)];
    frames.extend(blobs.into_iter().map(Frame::Blob));

    let mut out = Vec::new();
    stream::write_stream(&mut out, &frames).map_err(|err| err.to_string())?;

    let key = *blake3::hash(&out).as_bytes();
    let path = px_protocol::scene::cas_path(root, &crate::hex(&key))?;
    let hit = path.exists();
    if !hit {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        std::fs::write(&path, &out).map_err(|err| format!("写 {} 失败：{err}", path.display()))?;
    }
    Ok(Generated {
        id: id.to_string(),
        key,
        path,
        bytes: out.len() as u64,
        hit,
        millis: started.elapsed().as_millis() as u64,
    })
}

/// 把一份贴图写进 CAS：清单帧（`kind = AssetKind::Texture`、
/// `params = TextureShape::params()`、`fingerprint` = 对载荷算的 FNV-1a 指纹）
/// + 一个 blob（`Rgba8Srgb` → `DType::U8`；`Rgba16Float` → `DType::U16`，字节原样）。
///
/// 需要先 `px_graph::begin(GraphSpec)`（它给出 `cache_root()`）；不方便依赖它时用
/// [`write_texture_at`] 显式给一个根。
pub fn write_texture(
    id: &str,
    shape: TextureShape,
    payload: &[u8],
    dtype: DType,
) -> Result<Generated, String> {
    write_texture_at(&crate::cache_root(), id, shape, payload, dtype)
}

/// [`write_texture`] 的显式根变体。
pub fn write_texture_at(
    root: &Path,
    id: &str,
    shape: TextureShape,
    payload: &[u8],
    dtype: DType,
) -> Result<Generated, String> {
    let expected = shape.chain_bytes();
    if payload.len() != expected {
        return Err(format!(
            "贴图载荷 {id} 是 {} 字节，形状（{}×{}×{} 层、{} 级、{:?}）说应当是 {expected} 字节",
            payload.len(),
            shape.width,
            shape.height,
            shape.layers,
            shape.levels,
            shape.format,
        ));
    }
    let wanted = match shape.format {
        TextureFormat::Rgba8Srgb => DType::U8,
        TextureFormat::Rgba16Float => DType::U16,
    };
    if dtype != wanted {
        return Err(format!(
            "贴图载荷 {id} 的格式是 {:?}，位深应当是 {wanted:?}，实际给了 {dtype:?}",
            shape.format
        ));
    }
    let elems = payload.len() / dtype.elem_size();
    let blob = Blob::new(
        BlobHeader {
            dtype,
            shape: vec![elems as u32],
        },
        payload.to_vec(),
    )
    .map_err(|err| err.to_string())?;

    write_cas(root, id, AssetKind::Texture, shape.params(), vec![blob])
}

/// 网格产物同理（`kind = AssetKind::Mesh`，用 `MeshData::blobs()`），供环用。
///
/// 需要先 `px_graph::begin(GraphSpec)`；不方便依赖它时用 [`write_generated_mesh_at`]。
pub fn write_generated_mesh(id: &str, mesh: &MeshData) -> Result<Generated, String> {
    write_generated_mesh_at(&crate::cache_root(), id, mesh)
}

/// [`write_generated_mesh`] 的显式根变体。
pub fn write_generated_mesh_at(
    root: &Path,
    id: &str,
    mesh: &MeshData,
) -> Result<Generated, String> {
    let params = BTreeMap::from([
        ("vertices".to_string(), mesh.vertices() as f64),
        ("triangles".to_string(), mesh.triangles() as f64),
    ]);
    write_cas(root, id, AssetKind::Mesh, params, mesh.blobs())
}

/// 一份生成物的指纹（与 `px_graph::write_artifact` 用的是同一个函数）。审计/对账用。
pub fn fingerprint_of(id: &str, blobs: &[Blob]) -> u64 {
    crate::payload_fingerprint(id, blobs)
}
