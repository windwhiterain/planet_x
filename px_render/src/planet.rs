use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::color::LinearRgba;
use bevy::image::{
    Image, ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor,
};
use bevy::mesh::{Indices, Mesh, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use px_protocol::art::AssetKind;
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
}

pub struct PlanetSpec {
    pub field: String,
    pub palette: Palette,
    pub displace: f32,
    pub sea_level: f32,
    pub radius: f32,
    pub spin: f32,
    pub rings: f32,
}

const SYSTEM_TILT: f32 = 0.34;

pub struct Field {
    pub width: u32,
    pub height: u32,
    pub data: Vec<f32>,
    pub min: f32,
    pub max: f32,
}

impl Field {
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
    match kind {
        Some(AssetKind::Field2D) => {}
        Some(other) => return Err(format!("{path} 是 {other:?}，星球需要一个 Field2D 产物")),
        None => return Err(format!("{path} 里没有 Art 帧")),
    }

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
        let v = y as f32 / (field.height.max(2) - 1) as f32;
        let latitude = ((v - 0.5).abs() * 2.0).clamp(0.0, 1.0);
        for x in 0..field.width {
            let raw = field.data[(y * field.width + x) as usize];
            let height = field.normalized(raw);
            let (base, emit) = shade(palette, height, sea_level, latitude, raw * 6.283);
            push_color(&mut color, base);
            push_color(&mut glow, emit);
        }
    }

    let color_image = with_wrapping(
        image_from(field.width, field.height, color.clone()),
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
        Palette::Lava => Some(images.add(with_wrapping(
            image_from(field.width, field.height, glow),
            ImageAddressMode::Repeat,
            ImageAddressMode::ClampToEdge,
        ))),
        _ => None,
    };
    (color_handle, glow_handle)
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

fn image_from(width: u32, height: u32, data: Vec<u8>) -> Image {
    let (chain, levels) = mip_chain(width, height, &data);
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

pub fn star_image(width: u32, height: u32) -> Image {
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let mut hash = x
                .wrapping_mul(0x9e37_79b9)
                .wrapping_add(y.wrapping_mul(0x85eb_ca6b))
                .wrapping_mul(0xc2b2_ae35);
            hash ^= hash >> 15;
            hash = hash.wrapping_mul(0x2545_f491);
            hash ^= hash >> 13;
            let value = (hash & 0xffff) as f32 / 65535.0;
            let brightness = if value > 0.99935 {
                1.0
            } else if value > 0.99820 {
                0.55
            } else if value > 0.99650 {
                0.22
            } else {
                0.0
            };
            let blue = 0.86 + 0.14 * ((hash >> 16) as f32 / 65535.0);
            let level = (brightness * 255.0) as u8;
            data.push(level);
            data.push(level);
            data.push((level as f32 * blue) as u8);
            data.push(255);
        }
    }
    image_from(width, height, data)
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
        image_from(width, height, data),
        ImageAddressMode::ClampToEdge,
        ImageAddressMode::Repeat,
    )
}

pub fn spawn_planet(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    target: &Handle<Image>,
    stars: &Handle<Image>,
    spec: &PlanetSpec,
) -> Result<String, String> {
    let field = load_field(&spec.field)?;

    let mesh_handle = meshes.add(Sphere::new(1.0).mesh().ico(49).map_err(|err| err.to_string())?);
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
        let height = field.normalized(field.sample(uv[0], uv[1]));
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
    mesh.compute_smooth_normals();
    drop(mesh);

    let (color_texture, glow_texture) = surface_textures(images, &field, spec.palette, spec.sea_level);
    let emissive = if glow_texture.is_some() {
        LinearRgba::rgb(3.0, 3.0, 3.0)
    } else {
        LinearRgba::rgb(0.0, 0.0, 0.0)
    };

    commands.spawn((
        crate::ScenePart,
        Mesh3d(mesh_handle),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color_texture: Some(color_texture),
            emissive_texture: glow_texture,
            emissive,
            perceptual_roughness: 0.88,
            metallic: 0.0,
            ..default()
        })),
        Transform::from_rotation(
            Quat::from_rotation_x(SYSTEM_TILT) * Quat::from_rotation_y(spec.spin),
        ),
    ));

    if spec.rings > 0.0 {
        let inner = spec.radius * 1.30;
        let outer = spec.radius * spec.rings.max(1.45);
        commands.spawn((
            crate::ScenePart,
            Mesh3d(meshes.add(ring_mesh(inner, outer, 384))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color_texture: Some(images.add(ring_image(1024, 4))),
                alpha_mode: AlphaMode::Blend,
                unlit: true,
                cull_mode: None,
                ..default()
            })),
            Transform::from_rotation(
                Quat::from_rotation_x(SYSTEM_TILT) * Quat::from_rotation_y(spec.spin),
            ),
        ));
    }

    commands.spawn((
        crate::ScenePart,
        Mesh3d(meshes.add(Sphere::new(90.0).mesh().uv(64, 32))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color_texture: Some(stars.clone()),
            unlit: true,
            cull_mode: None,
            ..default()
        })),
        Transform::default(),
    ));

    commands.spawn((
        crate::ScenePart,
        DirectionalLight {
            illuminance: 3800.0,
            ..default()
        },
        Transform::from_xyz(-4.2, 1.15, 2.35).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    commands.spawn((
        crate::ScenePart,
        AmbientLight {
            brightness: 16.0,
            ..default()
        },
    ));

    commands.spawn((
        crate::ScenePart,
        Camera3d::default(),
        Msaa::Off,
        RenderTarget::Image(target.clone().into()),
        Transform::from_xyz(0.0, 0.55, 3.15).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    Ok(format!(
        "{}｜{}｜{}×{}｜位移 {:.3}（半径 ×{:.3}..×{:.3}）｜海平面 {:.2}{}",
        spec.field,
        spec.palette.name(),
        field.width,
        field.height,
        spec.displace,
        lowest,
        highest,
        spec.sea_level,
        if spec.rings > 0.0 {
            format!("｜环 ×{:.2}", spec.rings)
        } else {
            String::new()
        },
    ))
}
