use bevy::asset::RenderAssetUsages;
use bevy::color::LinearRgba;
use bevy::image::{
    Image, ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor,
};
use bevy::mesh::{Indices, Mesh, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;
use bevy::render::render_resource::{TextureViewDescriptor, TextureViewDimension};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::atmosphere::{AtmosphereMaterial, AtmosphereParams};
use px_protocol::art::{AssetKind, Domain, MeshData};
use px_protocol::stream::{self, Frame};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    pub fn atmosphere(self) -> (LinearRgba, f32, f32) {
        match self {
            Self::Rocky => (LinearRgba::rgb(0.44, 0.64, 0.98), 0.022, 0.50),
            Self::Gas => (LinearRgba::rgb(1.00, 0.86, 0.62), 0.070, 0.40),
            Self::Ice => (LinearRgba::rgb(0.66, 0.87, 1.00), 0.020, 0.60),
            Self::Lava => (LinearRgba::rgb(1.00, 0.46, 0.20), 0.055, 0.45),
            Self::Desert => (LinearRgba::rgb(0.97, 0.79, 0.55), 0.026, 0.46),
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

pub struct PlanetSpec {
    pub field: String,
    pub mesh: Option<String>,
    pub palette: Palette,
    pub displace: f32,
    pub sea_level: f32,
    pub radius: f32,
    pub spin: f32,
    pub rings: f32,
    pub atmo: f32,
}

const SYSTEM_TILT: f32 = 0.34;
const ATMOSPHERE_SHELL: f32 = 1.14;

#[derive(Component)]
pub struct PlanetBody;

#[derive(Component)]
pub struct PlanetRing;

#[derive(Component)]
pub struct PlanetAtmosphere;

pub struct Field {
    pub width: u32,
    pub height: u32,
    pub data: Vec<f32>,
    pub min: f32,
    pub max: f32,
    pub projection: Domain,
}

impl Field {
    pub fn texel_latitude(&self, x: u32, y: u32) -> f32 {
        match self.projection {
            Domain::Equirect => {
                let v = y as f32 / (self.height.max(2) - 1) as f32;
                ((v - 0.5).abs() * 2.0).clamp(0.0, 1.0)
            }
            _ => {
                px_protocol::art::direction_at(self.projection, self.width, self.height, x, y)[1]
                    .abs()
            }
        }
    }

    pub fn ring_mean(&self, row: u32) -> f32 {
        let row = row.min(self.height.saturating_sub(1));
        let start = (row * self.width) as usize;
        let end = start + self.width as usize;
        let slice = &self.data[start..end.min(self.data.len())];
        if slice.is_empty() {
            return 0.0;
        }
        slice.iter().sum::<f32>() / slice.len() as f32
    }

    pub fn sample_capped(&self, u: f32, v: f32) -> f32 {
        let value = self.sample(u, v);
        let rows = pole_cap_rows(self.height) as f32;
        let position = v.clamp(0.0, 1.0) * (self.height as f32 - 1.0);
        let from_pole = position.min(self.height as f32 - 1.0 - position);
        if from_pole >= rows {
            return value;
        }
        let weight = 1.0 - from_pole / rows;
        let row = if position < rows { 0 } else { self.height - 1 };
        value * (1.0 - weight) + self.ring_mean(row) * weight
    }

    pub fn sample(&self, u: f32, v: f32) -> f32 {
        let x = ((u.rem_euclid(1.0)) * self.width as f32) as u32 % self.width.max(1);
        let y = (v.clamp(0.0, 1.0) * (self.height as f32 - 1.0)).round() as u32;
        let y = y.min(self.height.saturating_sub(1));
        self.data[(y * self.width + x) as usize]
    }

    pub fn normalized(&self, value: f32) -> f32 {
        let span = self.max - self.min;
        if span.abs() < f32::EPSILON {
            0.5
        } else {
            (value - self.min) / span
        }
    }
}

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
        Some(other) => {
            return Err(format!(
                "{path} 是 {other:?}，星球需要 Field2D / OctahedralField / CubeField 产物"
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

    let height = blob.header.shape[0];
    let width = blob.header.shape[1];
    let data = blob.f32s().map_err(|err| err.to_string())?;
    let (mut min, mut max) = (f32::INFINITY, f32::NEG_INFINITY);
    for value in &data {
        min = min.min(*value);
        max = max.max(*value);
    }
    Ok(Field {
        width,
        height,
        data,
        min,
        max,
        projection,
    })
}

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

fn push_color(bytes: &mut Vec<u8>, color: [f32; 3]) {
    bytes.push((color[0].clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0).round() as u8);
    bytes.push((color[1].clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0).round() as u8);
    bytes.push((color[2].clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0).round() as u8);
    bytes.push(255);
}

fn surface_textures(
    images: &mut Assets<Image>,
    field: &Field,
    palette: Palette,
    sea_level: f32,
) -> (Handle<Image>, Option<Handle<Image>>) {
    let mut color = Vec::with_capacity((field.width * field.height * 4) as usize);
    let mut glow = Vec::with_capacity((field.width * field.height * 4) as usize);

    for y in 0..field.height {
        for x in 0..field.width {
            let raw = field.data[(y * field.width + x) as usize];
            let height = field.normalized(raw);
            let latitude = field.texel_latitude(x, y);
            let (base, emit) = shade(palette, height, sea_level, latitude, raw * 6.283);
            push_color(&mut color, base);
            push_color(&mut glow, emit);
        }
    }

    if field.projection == Domain::Equirect {
        pole_cap_filter(&mut color, field.width, field.height);
    }
    let color_image = with_wrapping(
        image_from(field.width, field.height, color.clone(), field.projection == Domain::Cube),
        ImageAddressMode::Repeat,
        ImageAddressMode::ClampToEdge,
    );
    let color_handle = images.add(color_image);

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
    println!(
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

    let glow_handle = match palette {
        Palette::Lava => {
            if field.projection == Domain::Equirect {
                pole_cap_filter(&mut glow, field.width, field.height);
            }
            Some(images.add(with_wrapping(
                image_from(field.width, field.height, glow, field.projection == Domain::Cube),
                ImageAddressMode::Repeat,
                ImageAddressMode::ClampToEdge,
            )))
        }
        _ => None,
    };
    (color_handle, glow_handle)
}

fn mip_chain_cube(width: u32, height: u32, base: &[u8]) -> (Vec<u8>, u32) {
    let mut chain = base.to_vec();
    let mut levels = 1_u32;
    let mut source = base.to_vec();
    let (mut w, mut h) = (width.max(1), height.max(1));

    while w > 1 && h > 1 {
        let next_w = (w / 2).max(1);
        let next_h = (h / 2).max(1);
        let cell = (w / px_protocol::art::CUBE_COLUMNS).max(1);
        let next_cell = (next_w / px_protocol::art::CUBE_COLUMNS).max(1);
        let mut next = vec![0_u8; (next_w * next_h * 4) as usize];

        for face in 0..px_protocol::art::CUBE_FACES {
            let column = face % px_protocol::art::CUBE_COLUMNS;
            let row = face / px_protocol::art::CUBE_COLUMNS;
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

fn image_from(width: u32, height: u32, data: Vec<u8>, cube: bool) -> Image {
    let (chain, levels) = if cube {
        mip_chain_cube(width, height, &data)
    } else {
        mip_chain(width, height, &data)
    };
    let mut image = Image::new_fill(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0, 0, 0, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.data = Some(chain);
    image.texture_descriptor.mip_level_count = levels;
    image
}

fn with_wrapping(mut image: Image, horizontal: ImageAddressMode, vertical: ImageAddressMode) -> Image {
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: horizontal,
        address_mode_v: vertical,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 8,
        ..default()
    });
    image
}

pub fn star_cube(face: u32) -> Image {
    let size = face.max(4);
    let mut data = Vec::with_capacity((size * size * 6 * 4) as usize);

    for face_index in 0..px_protocol::art::CUBE_FACES {
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

    let mut image = Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: px_protocol::art::CUBE_FACES,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::Cube),
        ..default()
    });
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        address_mode_w: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        ..default()
    });
    image
}
fn octahedral_mesh(radius: f32, resolution: u32) -> Mesh {
    let n = resolution.max(2);
    let mut positions = Vec::with_capacity((n * n) as usize);
    let mut normals = Vec::with_capacity((n * n) as usize);
    let mut uvs = Vec::with_capacity((n * n) as usize);
    let mut indices = Vec::with_capacity(((n - 1) * (n - 1) * 6) as usize);

    for j in 0..n {
        for i in 0..n {
            let u = (i as f32 + 0.5) / n as f32;
            let v = (j as f32 + 0.5) / n as f32;
            let direction = px_protocol::art::octahedral_direction_y_up(u, v);
            positions.push([
                direction[0] * radius,
                direction[1] * radius,
                direction[2] * radius,
            ]);
            normals.push(direction);
            uvs.push([u, v]);
        }
    }

    for j in 0..n - 1 {
        for i in 0..n - 1 {
            let a = i + j * n;
            let b = a + 1;
            let c = a + n;
            let d = c + 1;
            indices.extend_from_slice(&[a, b, c, b, d, c]);
        }
    }

    let mut flipped = 0_usize;
    let mut total = 0_usize;
    for triangle in indices.chunks_exact(3) {
        let a = Vec3::from(positions[triangle[0] as usize]);
        let b = Vec3::from(positions[triangle[1] as usize]);
        let c = Vec3::from(positions[triangle[2] as usize]);
        let face = (b - a).cross(c - a);
        let centroid = (a + b + c) / 3.0;
        total += 1;
        if face.dot(centroid) < 0.0 {
            flipped += 1;
        }
    }
    println!("八面体网格：{total} 个三角形，其中 {flipped} 个面法线朝内");

    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

pub fn probe_camera(cam: Option<[f32; 3]>) -> Transform {
    let Some([yaw, pitch, distance]) = cam else {
        return Transform::from_xyz(0.0, 0.55, 3.15).looking_at(Vec3::ZERO, Vec3::Y);
    };
    let yaw = yaw.to_radians();
    let pitch = pitch.clamp(-89.5, 89.5).to_radians();
    let direction = Vec3::new(
        pitch.cos() * yaw.sin(),
        pitch.sin(),
        pitch.cos() * yaw.cos(),
    );
    Transform::from_translation(direction * distance).looking_at(Vec3::ZERO, Vec3::Y)
}

pub fn warm_atmosphere(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<AtmosphereMaterial>,
) {
    let Ok(sphere) = Sphere::new(1.14).mesh().ico(16) else {
        return;
    };
    let material = materials.add(AtmosphereMaterial {
        params: AtmosphereParams {
            inner: 1.0,
            outer: 1.14,
            density: 0.035,
            softness: 0.5,
            camera_x: 0.0,
            camera_y: 0.55,
            camera_z: 3.15,
            screen_x: 960.0,
            screen_y: 640.0,
            padding_a: 0.0,
            padding_b: 0.0,
            padding_c: 0.0,
            forward: Vec4::Z,
            right: Vec4::X,
            up: Vec4::Y,
            reserved: Vec4::W,
        },
        tint: LinearRgba::rgb(0.44, 0.64, 0.98),
    });
    commands.spawn((
        crate::ScenePart,
        PlanetAtmosphere,
        Mesh3d(meshes.add(sphere)),
        MeshMaterial3d(material),
        Transform::default(),
    ));
}

fn spawn_scattering(
    commands: &mut Commands,
    media: &mut Assets<bevy::light::atmosphere::ScatteringMedium>,
    radius: f32,
) {
    let medium = media.add(bevy::light::atmosphere::ScatteringMedium::earth(256, 256));
    let scale = radius / 6_360_000.0;
    commands.spawn((
        crate::ScenePart,
        Transform::from_scale(Vec3::splat(scale)),
        GlobalTransform::from_scale(Vec3::splat(scale)),
        bevy::light::Atmosphere::earth(medium),
    ));
}

fn spawn_atmosphere(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<AtmosphereMaterial>,
    parent: Entity,
    camera: Vec3,
    spec: &PlanetSpec,
) {
    if spec.atmo <= 0.0 {
        return;
    }
    let (tint, density, softness) = spec.palette.atmosphere();
    let Ok(sphere) = Sphere::new(spec.radius * ATMOSPHERE_SHELL).mesh().ico(64) else {
        return;
    };
    let material = materials.add(AtmosphereMaterial {
        params: AtmosphereParams {
            inner: spec.radius,
            outer: spec.radius * ATMOSPHERE_SHELL,
            density: density * spec.atmo,
            softness,
            camera_x: camera.x,
            camera_y: camera.y,
            camera_z: camera.z,
            screen_x: 960.0,
            screen_y: 640.0,
            padding_a: 0.0,
            padding_b: 0.0,
            padding_c: 0.0,
            forward: Vec4::Z,
            right: Vec4::X,
            up: Vec4::Y,
            reserved: Vec4::W,
        },
        tint,
    });
    let _ = parent;
    commands.spawn((
        crate::ScenePart,
        PlanetAtmosphere,
        Mesh3d(meshes.add(sphere)),
        MeshMaterial3d(material),
        Transform::default(),
    ));
}

fn spawn_lights(commands: &mut Commands) {
    commands.spawn((
        crate::ScenePart,
        DirectionalLight {
            illuminance: 3800.0,
            ..default()
        },
        Transform::from_xyz(-4.2, 1.15, 2.35).looking_at(Vec3::ZERO, Vec3::Y),
    ));


}

fn spawn_rings(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    spec: &PlanetSpec,
) {
    if spec.rings <= 0.0 {
        return;
    }
    let inner = spec.radius * 1.30;
    let outer = spec.radius * spec.rings.max(1.45);
    commands.spawn((
        crate::ScenePart,
        Transform::from_rotation(Quat::from_rotation_x(SYSTEM_TILT)),
        Visibility::default(),
    )).with_children(|parent| {
        parent.spawn((
            PlanetRing,
            Mesh3d(meshes.add(ring_mesh(inner, outer, 384))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color_texture: Some(images.add(ring_image(1024, 4))),
                alpha_mode: AlphaMode::Blend,
                unlit: true,
                cull_mode: None,
                ..default()
            })),
            Transform::from_rotation(Quat::from_rotation_y(spec.spin)),
        ));
    });
}

pub fn load_mesh(path: &str) -> Result<Mesh, String> {
    let bytes = std::fs::read(path).map_err(|err| format!("读不到 {path}：{err}"))?;
    let frames = stream::read_stream(&mut bytes.as_slice()).map_err(|err| err.to_string())?;
    let kind = frames.iter().find_map(|frame| match frame {
        Frame::Art(bundle) => bundle.assets.first().map(|asset| asset.kind),
        _ => None,
    });
    if kind != Some(AssetKind::Mesh) {
        return Err(format!("{path} 不是 Mesh 产物：{kind:?}"));
    }
    let blobs: Vec<&px_protocol::wire::Blob> = frames
        .iter()
        .filter_map(|frame| match frame {
            Frame::Blob(blob) => Some(blob),
            _ => None,
        })
        .collect();
    let data = MeshData::from_blobs(&blobs).map_err(|err| err.to_string())?;

    let positions: Vec<[f32; 3]> = data
        .positions
        .chunks_exact(3)
        .map(|chunk| [chunk[0], chunk[1], chunk[2]])
        .collect();
    let normals: Vec<[f32; 3]> = data
        .normals
        .chunks_exact(3)
        .map(|chunk| [chunk[0], chunk[1], chunk[2]])
        .collect();
    let uvs: Vec<[f32; 2]> = data
        .uvs
        .chunks_exact(2)
        .map(|chunk| [chunk[0], chunk[1]])
        .collect();

    {
        let mut edges: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
        for triangle in data.indices.chunks_exact(3) {
            for pair in 0..3 {
                let (a, b) = (triangle[pair], triangle[(pair + 1) % 3]);
                let key = if a < b { (a, b) } else { (b, a) };
                *edges.entry(key).or_insert(0) += 1;
            }
        }
        let open = edges.values().filter(|count| **count == 1).count();
        let odd = edges.values().filter(|count| **count > 2).count();
        println!(
            "网格缝合审计：{} 条边，其中 {open} 条只属于一个三角形（开口），{odd} 条属于两个以上",
            edges.len()
        );
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(data.indices));
    weld_normals(&mut mesh);

    if let (Some(VertexAttributeValues::Float32x3(normals)), Some(VertexAttributeValues::Float32x3(positions))) =
        (
            mesh.attribute(Mesh::ATTRIBUTE_NORMAL),
            mesh.attribute(Mesh::ATTRIBUTE_POSITION),
        )
    {
        let mut worst = 1.0_f32;
        for (normal, position) in normals.iter().zip(positions.iter()) {
            let radial = Vec3::new(position[0], position[1], position[2]).normalize_or_zero();
            worst = worst.min(Vec3::from(*normal).dot(radial));
        }
        println!("载入网格法线审计：最小点积 {worst:.3}");
    }
    Ok(mesh)
}

fn grid_normals(mesh: &mut Mesh, resolution: u32) {
    let n = resolution.max(2);
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION).cloned()
    else {
        return;
    };
    let at = |i: u32, j: u32| {
        let i = i.min(n - 1);
        let j = j.min(n - 1);
        Vec3::from(positions[(i + j * n) as usize])
    };

    let mut normals = Vec::with_capacity(positions.len());
    for j in 0..n {
        for i in 0..n {
            let along_u = at(i + 1, j) - at(i.saturating_sub(1), j);
            let along_v = at(i, j + 1) - at(i, j.saturating_sub(1));
            let radial = at(i, j).normalize_or_zero();
            let mut normal = along_u.cross(along_v).normalize_or_zero();
            if normal == Vec3::ZERO {
                normal = radial;
            }
            if normal.dot(radial) < 0.0 {
                normal = -normal;
            }
            normals.push(normal.to_array());
        }
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
}

fn weld_normals(mesh: &mut Mesh) {
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION).cloned()
    else {
        return;
    };
    let Some(VertexAttributeValues::Float32x3(normals)) =
        mesh.attribute(Mesh::ATTRIBUTE_NORMAL).cloned()
    else {
        return;
    };

    let mut groups: std::collections::HashMap<[i32; 3], Vec<usize>> =
        std::collections::HashMap::new();
    for (index, position) in positions.iter().enumerate() {
        let key = [
            (position[0] * 65_536.0).round() as i32,
            (position[1] * 65_536.0).round() as i32,
            (position[2] * 65_536.0).round() as i32,
        ];
        groups.entry(key).or_default().push(index);
    }

    let mut welded = normals.clone();
    for indices in groups.values() {
        if indices.len() < 2 {
            continue;
        }
        let mut sum = Vec3::ZERO;
        for &index in indices {
            sum += Vec3::from(normals[index]);
        }
        let average = sum.normalize_or_zero().to_array();
        for &index in indices {
            welded[index] = average;
        }
    }

    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, welded);
}

fn uv_sphere(radius: f32, sectors: u32, stacks: u32) -> Mesh {
    let sector_count = sectors.max(3);
    let stack_count = stacks.max(2);
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    let north_base = positions.len() as u32;
    for sector in 0..sector_count {
        let u = (sector as f32 + 0.5) / sector_count as f32;
        positions.push([0.0, radius, 0.0]);
        normals.push([0.0, 1.0, 0.0]);
        uvs.push([u, 0.0]);
    }

    let mut rings = Vec::new();
    for stack in 1..stack_count {
        let v = stack as f32 / stack_count as f32;
        let theta = v * std::f32::consts::PI;
        let (sin_theta, cos_theta) = theta.sin_cos();
        rings.push(positions.len() as u32);
        for sector in 0..=sector_count {
            let u = sector as f32 / sector_count as f32;
            let phi = u * std::f32::consts::TAU;
            let (sin_phi, cos_phi) = phi.sin_cos();
            let direction = [sin_theta * cos_phi, cos_theta, sin_theta * sin_phi];
            positions.push([
                direction[0] * radius,
                direction[1] * radius,
                direction[2] * radius,
            ]);
            normals.push(direction);
            uvs.push([u, v]);
        }
    }

    let south_base = positions.len() as u32;
    for sector in 0..sector_count {
        let u = (sector as f32 + 0.5) / sector_count as f32;
        positions.push([0.0, -radius, 0.0]);
        normals.push([0.0, -1.0, 0.0]);
        uvs.push([u, 1.0]);
    }

    let first_ring = rings[0];
    for sector in 0..sector_count {
        indices.extend_from_slice(&[
            north_base + sector,
            first_ring + sector + 1,
            first_ring + sector,
        ]);
    }

    for band in 0..rings.len() - 1 {
        let (upper, lower) = (rings[band], rings[band + 1]);
        for sector in 0..sector_count {
            let (u0, u1) = (upper + sector, upper + sector + 1);
            let (l0, l1) = (lower + sector, lower + sector + 1);
            indices.extend_from_slice(&[u0, u1, l1, u0, l1, l0]);
        }
    }

    let last_ring = *rings.last().unwrap();
    for sector in 0..sector_count {
        indices.extend_from_slice(&[
            south_base + sector,
            last_ring + sector,
            last_ring + sector + 1,
        ]);
    }

    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

fn ring_mesh(inner: f32, outer: f32, segments: u32) -> Mesh {
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

    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

fn ring_image(width: u32, height: u32) -> Image {
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
    with_wrapping(
        image_from(width, height, data, false),
        ImageAddressMode::ClampToEdge,
        ImageAddressMode::Repeat,
    )
}

pub fn spawn_planet(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    stars: &Handle<Image>,
    atmo_materials: &mut Assets<AtmosphereMaterial>,
    media: &mut Assets<bevy::light::atmosphere::ScatteringMedium>,
    scatter: Option<&str>,
    camera: Vec3,
    spec: &PlanetSpec,
) -> Result<String, String> {
    let field = load_field(&spec.field)?;

    if let Some(path) = &spec.mesh {
        let mesh = load_mesh(path)?;
        let vertices = mesh.count_vertices();
        let triangles = mesh.indices().map(|indices| indices.len() / 3).unwrap_or(0);
        let mesh_handle = meshes.add(mesh);
        let (color_texture, glow_texture) =
            surface_textures(images, &field, spec.palette, spec.sea_level);
        let emissive = if glow_texture.is_some() {
            LinearRgba::rgb(3.0, 3.0, 3.0)
        } else {
            LinearRgba::rgb(0.0, 0.0, 0.0)
        };

        let system = commands
            .spawn((
                crate::ScenePart,
                Transform::from_rotation(Quat::from_rotation_x(SYSTEM_TILT)),
                Visibility::default(),
            ))
            .id();

        commands.entity(system).with_children(|parent| {
            parent.spawn((
                PlanetBody,
                Mesh3d(mesh_handle),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color_texture: Some(color_texture),
                    emissive_texture: glow_texture,
                    emissive,
                    perceptual_roughness: 0.88,
                    metallic: 0.0,
                    ..default()
                })),
                Transform::from_rotation(Quat::from_rotation_y(spec.spin)),
            ));
        });

        match scatter {
            Some(_) => spawn_scattering(commands, media, spec.radius),
            None => spawn_atmosphere(commands, meshes, atmo_materials, system, camera, spec),
        }
        spawn_rings(commands, meshes, materials, images, spec);
        spawn_lights(commands);

        return Ok(format!(
            "{}｜{}｜{}×{}｜PCG 网格 {vertices} 顶点 / {triangles} 三角形｜海平面 {:.2}{}",
            spec.palette.name(),
            spec.field,
            field.width,
            field.height,
            spec.sea_level,
            if spec.rings > 0.0 {
                format!("｜环 ×{:.2}", spec.rings)
            } else {
                String::new()
            },
        ));
    }

    let mesh_handle = if field.projection == Domain::Equirect {
        meshes.add(uv_sphere(1.0, 224, 112))
    } else {
        meshes.add(octahedral_mesh(1.0, 320))
    };
    let mut mesh = meshes
        .get_mut(&mesh_handle)
        .ok_or_else(|| "拿不到刚插入的球面网格".to_string())?;

    let uvs = match mesh.attribute(Mesh::ATTRIBUTE_UV_0) {
        Some(VertexAttributeValues::Float32x2(values)) => values.clone(),
        _ => return Err("球面网格没有 UV 属性".to_string()),
    };
    let mut positions = match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
        Some(VertexAttributeValues::Float32x3(values)) => values.clone(),
        _ => return Err("球面网格没有位置属性".to_string()),
    };

    let mut lowest = f32::INFINITY;
    let mut highest = f32::NEG_INFINITY;
    let flat_sea = matches!(spec.palette, Palette::Rocky | Palette::Ice);
    for (position, uv) in positions.iter_mut().zip(uvs.iter()) {
        let height = if field.projection == Domain::Equirect {
            field.normalized(field.sample_capped(uv[0], uv[1]))
        } else {
            field.normalized(field.sample(uv[0], uv[1]))
        };
        let shaped = if flat_sea && height < spec.sea_level {
            spec.sea_level
        } else {
            height
        };
        let lift = 1.0 + spec.displace * (shaped - spec.sea_level);
        lowest = lowest.min(lift);
        highest = highest.max(lift);
        position[0] *= spec.radius * lift;
        position[1] *= spec.radius * lift;
        position[2] *= spec.radius * lift;
    }

    match mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION) {
        Some(VertexAttributeValues::Float32x3(target)) => target.clone_from(&positions),
        _ => return Err("回写位置失败".to_string()),
    }
    if field.projection == Domain::Equirect {
        mesh.compute_smooth_normals();
    } else {
        grid_normals(&mut mesh, 320);
    }
    weld_normals(&mut mesh);

    if let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
    {
        let mut inward = 0_usize;
        let mut worst = 1.0_f32;
        for (normal, position) in normals.iter().zip(positions.iter()) {
            let radial = Vec3::new(position[0], position[1], position[2]).normalize_or_zero();
            let dot = Vec3::from(*normal).dot(radial);
            if dot < 0.0 {
                inward += 1;
            }
            worst = worst.min(dot);
        }
        println!(
            "法线审计：{} 个顶点，{inward} 个与半径反向，最小点积 {worst:.3}",
            normals.len()
        );
    }
    drop(mesh);

    let (color_texture, glow_texture) = surface_textures(images, &field, spec.palette, spec.sea_level);
    let emissive = if glow_texture.is_some() {
        LinearRgba::rgb(3.0, 3.0, 3.0)
    } else {
        LinearRgba::rgb(0.0, 0.0, 0.0)
    };

    let system = commands
        .spawn((
            crate::ScenePart,
            Transform::from_rotation(Quat::from_rotation_x(SYSTEM_TILT)),
            Visibility::default(),
        ))
        .id();

    commands.entity(system).with_children(|parent| {
        parent.spawn((
            PlanetBody,
            Mesh3d(mesh_handle),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color_texture: Some(color_texture),
                emissive_texture: glow_texture,
                emissive,
                perceptual_roughness: 0.88,
                metallic: 0.0,

                ..default()
            })),
            Transform::from_rotation(Quat::from_rotation_y(spec.spin)),
        ));

        if spec.rings > 0.0 {
            let inner = spec.radius * 1.30;
            let outer = spec.radius * spec.rings.max(1.45);
            parent.spawn((
                PlanetRing,
                Mesh3d(meshes.add(ring_mesh(inner, outer, 384))),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color_texture: Some(images.add(ring_image(1024, 4))),
                    alpha_mode: AlphaMode::Blend,
                    unlit: true,
                    cull_mode: None,
                    ..default()
                })),
                Transform::from_rotation(Quat::from_rotation_y(spec.spin)),
            ));
        }
    });


    let mut far = 0.0_f32;
    for position in positions.iter() {
        far = far.max(
            (position[0] * position[0] + position[1] * position[1] + position[2] * position[2])
                .sqrt(),
        );
    }

    Ok(format!(
        "{}｜{}｜{}×{}｜位移 {:.3}（半径 ×{:.3}..×{:.3}，最远顶点 {:.4}）｜海平面 {:.2}{}",
        spec.field,
        spec.palette.name(),
        field.width,
        field.height,
        spec.displace,
        lowest,
        highest,
        far,
        spec.sea_level,
        if spec.rings > 0.0 {
            format!("｜环 ×{:.2}", spec.rings)
        } else {
            String::new()
        },
    ))
}

































































