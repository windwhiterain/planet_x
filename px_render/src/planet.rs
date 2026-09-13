use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::color::LinearRgba;
use bevy::image::Image;
use bevy::mesh::{Mesh, VertexAttributeValues};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use px_protocol::art::AssetKind;
use px_protocol::render::Palette;
use px_protocol::stream::{self, Frame};

pub struct PlanetSpec {
    pub field: String,
    pub palette: Palette,
    pub displace: f32,
    pub sea_level: f32,
    pub radius: f32,
    pub spin: f32,
}

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
        let y = ((1.0 - v.clamp(0.0, 1.0)) * (self.height as f32 - 1.0)).round() as u32;
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
            let bands = (latitude * 17.0 + wobble * 1.5).sin() * 0.5 + 0.5;
            let detail = (latitude * 47.0 + wobble * 2.7).sin() * 0.5 + 0.5;
            let t = (bands * 0.68 + detail * 0.32).clamp(0.0, 1.0);
            let mut color = ramp(GAS, t);
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
    }
}

const LAVA_ROCK: &[(f32, [f32; 3])] = &[
    (0.00, [0.035, 0.027, 0.027]),
    (0.35, [0.086, 0.063, 0.055]),
    (0.60, [0.176, 0.125, 0.098]),
    (0.82, [0.290, 0.208, 0.157]),
    (1.00, [0.427, 0.353, 0.310]),
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
        let v = 1.0 - y as f32 / (field.height.max(2) - 1) as f32;
        let latitude = ((v - 0.5).abs() * 2.0).clamp(0.0, 1.0);
        for x in 0..field.width {
            let raw = field.data[(y * field.width + x) as usize];
            let height = field.normalized(raw);
            let (base, emit) = shade(palette, height, sea_level, latitude, raw * 6.283);
            push_color(&mut color, base);
            push_color(&mut glow, emit);
        }
    }

    let color_image = image_from(field.width, field.height, color.clone());
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
    println!(
        "         贴图 {}{}×{}：平均 RGB ({:.0},{:.0},{:.0})，近白像素 {:.1}%",
        palette.name(),
        field.width,
        field.height,
        sums[0] / texels as f64,
        sums[1] / texels as f64,
        sums[2] / texels as f64,
        white as f64 * 100.0 / texels as f64,
    );

    let glow_handle = match palette {
        Palette::Lava => Some(images.add(image_from(field.width, field.height, glow))),
        _ => None,
    };
    (color_handle, glow_handle)
}

fn image_from(width: u32, height: u32, data: Vec<u8>) -> Image {
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
    image.data = Some(data);
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

    let mesh_handle = meshes.add(Sphere::new(1.0).mesh().uv(224, 112));
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
            Quat::from_rotation_y(spec.spin) * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
        ),
    ));

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
        "{}｜{}×{}｜位移 {:.3}（半径 ×{:.3}..×{:.3}）｜海平面 {:.2}",
        spec.field, field.width, field.height, spec.displace, lowest, highest, spec.sea_level
    ))
}
