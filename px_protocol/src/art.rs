use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::wire::{Blob, BlobHeader, WireError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetKind {
    Field2D,
    OctahedralField,
    CubeField,
    CubeMap,
    Mesh,
    Instances,
    Volume,
    VoxelField,
    Scene,
    Shader,
    Texture,
    StarField,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureFormat {
    Rgba8Srgb,
    Rgba16Float,
}

impl TextureFormat {
    pub const NAMES: [&'static str; 2] = ["rgba8_srgb", "rgba16_float"];

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "rgba8_srgb" => Some(Self::Rgba8Srgb),
            "rgba16_float" => Some(Self::Rgba16Float),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Rgba8Srgb => "rgba8_srgb",
            Self::Rgba16Float => "rgba16_float",
        }
    }

    pub fn code(self) -> f64 {
        match self {
            Self::Rgba8Srgb => 0.0,
            Self::Rgba16Float => 1.0,
        }
    }

    pub fn from_code(code: f64) -> Option<Self> {
        match code as i64 {
            0 => Some(Self::Rgba8Srgb),
            1 => Some(Self::Rgba16Float),
            _ => None,
        }
    }

    pub fn texel_bytes(self) -> usize {
        match self {
            Self::Rgba8Srgb => 4,
            Self::Rgba16Float => 8,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextureShape {
    pub width: u32,
    pub height: u32,
    pub layers: u32,
    pub levels: u32,
    pub format: TextureFormat,
}

impl TextureShape {
    pub fn params(self) -> BTreeMap<String, f64> {
        BTreeMap::from([
            ("width".to_string(), f64::from(self.width)),
            ("height".to_string(), f64::from(self.height)),
            ("layers".to_string(), f64::from(self.layers)),
            ("levels".to_string(), f64::from(self.levels)),
            ("format".to_string(), self.format.code()),
        ])
    }

    pub fn from_params(params: &BTreeMap<String, f64>) -> Result<Self, String> {
        let number = |key: &str| -> Result<u32, String> {
            let value = params
                .get(key)
                .ok_or_else(|| format!("贴图产物缺参数 '{key}'"))?;
            if !(*value >= 0.0 && value.fract() == 0.0 && *value <= u32::MAX as f64) {
                return Err(format!("贴图产物的 '{key}' 应当是非负整数，实际是 {value}"));
            }
            Ok(*value as u32)
        };
        let format = params
            .get("format")
            .and_then(|value| TextureFormat::from_code(*value))
            .ok_or_else(|| {
                format!(
                    "贴图产物的 'format' 不认识（可用：{}）",
                    TextureFormat::NAMES.join(" / ")
                )
            })?;
        let shape = Self {
            width: number("width")?,
            height: number("height")?,
            layers: number("layers")?,
            levels: number("levels")?,
            format,
        };
        if shape.width == 0 || shape.height == 0 {
            return Err(format!(
                "贴图产物尺寸是 {}×{}：宽高都不能是 0",
                shape.width, shape.height
            ));
        }
        if shape.layers != 1 && shape.layers != CUBE_FACES {
            return Err(format!(
                "贴图产物的 'layers' 只能是 1（2D）或 {CUBE_FACES}（cube），实际是 {}",
                shape.layers
            ));
        }
        if shape.levels == 0 {
            return Err("贴图产物的 'levels' 至少是 1".to_string());
        }
        Ok(shape)
    }

    pub fn chain_bytes(self) -> usize {
        let mut total = 0_usize;
        let (mut width, mut height) = (self.width, self.height);
        for _ in 0..self.levels {
            total +=
                width as usize * height as usize * self.layers as usize * self.format.texel_bytes();
            width = (width / 2).max(1);
            height = (height / 2).max(1);
        }
        total
    }
}

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

    pub fn new(
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

fn sign(value: f32) -> f32 {
    if value < 0.0 { -1.0 } else { 1.0 }
}

pub fn octahedral_direction(u: f32, v: f32) -> [f32; 3] {
    let x = u.clamp(0.0, 1.0) * 2.0 - 1.0;
    let y = v.clamp(0.0, 1.0) * 2.0 - 1.0;
    let mut direction = [x, y, 1.0 - x.abs() - y.abs()];
    if direction[2] < 0.0 {
        let (old_x, old_y) = (direction[0], direction[1]);
        direction[0] = (1.0 - old_y.abs()) * sign(old_x);
        direction[1] = (1.0 - old_x.abs()) * sign(old_y);
    }
    let length =
        (direction[0] * direction[0] + direction[1] * direction[1] + direction[2] * direction[2])
            .sqrt();
    if length <= f32::EPSILON {
        return [0.0, 1.0, 0.0];
    }
    [
        direction[0] / length,
        direction[1] / length,
        direction[2] / length,
    ]
}

pub fn octahedral_direction_y_up(u: f32, v: f32) -> [f32; 3] {
    let direction = octahedral_direction(u, v);
    [direction[0], direction[2], -direction[1]]
}

pub fn octahedral_uv(direction: [f32; 3]) -> [f32; 2] {
    let length = direction[0].abs() + direction[1].abs() + direction[2].abs();
    if length <= f32::EPSILON {
        return [0.5, 0.5];
    }
    let mut x = direction[0] / length;
    let mut y = direction[1] / length;
    if direction[2] < 0.0 {
        let old_x = x;
        x = (1.0 - y.abs()) * sign(old_x);
        y = (1.0 - old_x.abs()) * sign(y);
    }
    [x * 0.5 + 0.5, y * 0.5 + 0.5]
}

pub fn octahedral_uv_y_up(direction: [f32; 3]) -> [f32; 2] {
    octahedral_uv([direction[0], -direction[2], direction[1]])
}

pub const POLYLINE_ATTRIBUTES: [&str; 2] = ["positions", "indices"];
pub const POLYLINE_POSITION: usize = 0;
pub const POLYLINE_INDEX: usize = 1;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PolylineData {
    pub positions: Vec<f32>,
    pub indices: Vec<u32>,
}

impl PolylineData {
    pub fn vertices(&self) -> usize {
        self.positions.len() / 3
    }

    pub fn segments(&self) -> usize {
        self.indices.len() / 2
    }

    pub fn blobs(&self) -> Vec<Blob> {
        vec![
            Blob::from_f32(vec![self.vertices() as u32, 3], &self.positions),
            Blob::from_u32(vec![self.indices.len() as u32], &self.indices),
        ]
    }

    pub fn from_blobs(blobs: &[&Blob]) -> Result<Self, WireError> {
        if blobs.len() < POLYLINE_ATTRIBUTES.len() {
            return Err(WireError::TruncatedFrame);
        }
        let line = Self {
            positions: blobs[POLYLINE_POSITION].f32s()?,
            indices: blobs[POLYLINE_INDEX].u32s()?,
        };
        if line.positions.len() % 3 != 0 || line.indices.len() % 2 != 0 {
            return Err(WireError::TruncatedFrame);
        }
        Ok(line)
    }
}

pub const MESH_ATTRIBUTES: [&str; 4] = ["positions", "normals", "uvs", "indices"];
pub const MESH_POSITION: usize = 0;
pub const MESH_NORMAL: usize = 1;
pub const MESH_UV: usize = 2;
pub const MESH_INDEX: usize = 3;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MeshData {
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    pub uvs: Vec<f32>,
    pub indices: Vec<u32>,
}

impl MeshData {
    pub fn vertices(&self) -> usize {
        self.positions.len() / 3
    }

    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn blobs(&self) -> Vec<Blob> {
        vec![
            Blob::from_f32(vec![self.vertices() as u32, 3], &self.positions),
            Blob::from_f32(vec![self.vertices() as u32, 3], &self.normals),
            Blob::from_f32(vec![self.vertices() as u32, 2], &self.uvs),
            Blob::from_u32(vec![self.indices.len() as u32], &self.indices),
        ]
    }

    pub fn from_blobs(blobs: &[&Blob]) -> Result<Self, WireError> {
        if blobs.len() < MESH_ATTRIBUTES.len() {
            return Err(WireError::TruncatedFrame);
        }
        let mesh = Self {
            positions: blobs[MESH_POSITION].f32s()?,
            normals: blobs[MESH_NORMAL].f32s()?,
            uvs: blobs[MESH_UV].f32s()?,
            indices: blobs[MESH_INDEX].u32s()?,
        };
        let vertices = mesh.vertices();
        if mesh.normals.len() != vertices * 3 || mesh.uvs.len() != vertices * 2 {
            return Err(WireError::TruncatedFrame);
        }
        Ok(mesh)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct VolumeData {
    pub res: u32,
    pub layers: u32,
    pub inner: f32,
    pub outer: f32,
    pub lanes: u32,
    pub data: Vec<f32>,
}

impl Default for VolumeData {
    fn default() -> Self {
        Self {
            res: 0,
            layers: 0,
            inner: 0.0,
            outer: 0.0,
            lanes: 1,
            data: Vec::new(),
        }
    }
}

pub const VOLUME_SHAPE: [u32; 4] = [CUBE_FACES, 0, 0, 0];

impl VolumeData {
    #[inline]
    pub fn lane_slot(&self, voxel: usize, lane: usize) -> usize {
        voxel * self.lanes.max(1) as usize + lane
    }

    #[inline]
    pub fn expect_single(&self) -> &Self {
        assert_eq!(
            self.lanes.max(1),
            1,
            "这是 {}-通道体积（发射 = 6 通道交错）；单通道读者（密度/星场）不能吃它 —— \
             要么改用按通道的读法，要么传密度那一档",
            self.lanes
        );
        self
    }

    #[inline]
    pub fn emission_at(&self, voxel: usize, channel: usize) -> f32 {
        assert!(channel < self.lanes.max(1) as usize, "通道越界");
        self.data[self.lane_slot(voxel, channel)]
    }

    pub fn samples(&self) -> usize {
        self.res as usize * self.layers as usize * self.res as usize * CUBE_FACES as usize
    }

    pub fn lanes(&self) -> usize {
        self.lanes.max(1) as usize
    }

    pub fn at(&self, face: u32, layer: u32, t: u32, s: u32) -> f32 {
        assert_eq!(self.lanes(), 1, "多通道体积请逐通道取（见 `lanes`）");
        self.data[(((face * self.layers + layer) * self.res + t) * self.res + s) as usize]
    }

    pub fn blobs(&self) -> Vec<Blob> {
        let mut shape = vec![CUBE_FACES, self.layers, self.res, self.res];
        if self.lanes > 1 {
            shape.push(self.lanes);
        }
        vec![Blob::from_f32(shape, &self.data)]
    }

    pub fn from_blob(blob: &Blob) -> Result<Self, WireError> {
        let shape = &blob.header.shape;
        if shape.len() != 4 && shape.len() != 5 {
            return Err(WireError::NotF32(blob.header.dtype));
        }
        let lanes = shape.get(4).copied().unwrap_or(1).max(1) as usize;
        let data = blob.f32s()?;
        let volume = Self {
            res: shape[3],
            layers: shape[1],
            inner: 0.0,
            outer: 0.0,
            lanes: lanes as u32,
            data,
        };
        if volume.data.len() != volume.samples() * lanes {
            return Err(WireError::TruncatedFrame);
        }
        Ok(volume)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    pub direction: [f32; 3],
    pub distance: f32,
    #[serde(default)]
    pub tag: String,
}

impl Camera {
    pub fn new(direction: [f32; 3], distance: f32, tag: impl Into<String>) -> Self {
        Self::raw(direction, distance, tag).normalized()
    }

    pub fn raw(direction: [f32; 3], distance: f32, tag: impl Into<String>) -> Self {
        Self {
            direction,
            distance,
            tag: tag.into(),
        }
    }

    pub fn normalized(mut self) -> Self {
        let length = (self.direction[0] * self.direction[0]
            + self.direction[1] * self.direction[1]
            + self.direction[2] * self.direction[2])
            .sqrt();
        self.direction = if length > f32::EPSILON {
            [
                self.direction[0] / length,
                self.direction[1] / length,
                self.direction[2] / length,
            ]
        } else {
            [0.0, 0.0, 1.0]
        };
        self.distance = self.distance.max(1e-3);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetManifest {
    pub id: String,
    pub params: BTreeMap<String, f64>,
    pub blobs: Vec<BlobHeader>,
    #[serde(default)]
    pub fingerprint: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ArtBundle {
    pub assets: Vec<AssetManifest>,
}

pub const CUBE_FACES: u32 = 6;
pub const CUBE_COLUMNS: u32 = 3;
pub const CUBE_GUTTER: u32 = 2;

pub fn cube_cell_size(width: u32) -> u32 {
    width / CUBE_COLUMNS
}

pub fn cube_face_size(width: u32) -> u32 {
    cube_cell_size(width).saturating_sub(CUBE_GUTTER * 2)
}

pub fn cube_direction(face: u32, s: f32, t: f32) -> [f32; 3] {
    let a = s * 2.0 - 1.0;
    let b = t * 2.0 - 1.0;
    let direction = match face % CUBE_FACES {
        0 => [1.0, -b, -a],
        1 => [-1.0, -b, a],
        2 => [a, 1.0, b],
        3 => [a, -1.0, -b],
        4 => [a, -b, 1.0],
        _ => [-a, -b, -1.0],
    };
    let length =
        (direction[0] * direction[0] + direction[1] * direction[1] + direction[2] * direction[2])
            .sqrt();
    if length <= f32::EPSILON {
        return [0.0, 1.0, 0.0];
    }
    [
        direction[0] / length,
        direction[1] / length,
        direction[2] / length,
    ]
}

pub fn cube_face_of(direction: [f32; 3]) -> (u32, f32, f32) {
    let (x, y, z) = (direction[0], direction[1], direction[2]);
    let (ax, ay, az) = (x.abs(), y.abs(), z.abs());
    let face = if ax >= ay && ax >= az {
        if x > 0.0 { 0 } else { 1 }
    } else if ay >= az {
        if y > 0.0 { 2 } else { 3 }
    } else if z > 0.0 {
        4
    } else {
        5
    };
    let major = match face {
        0 | 1 => ax,
        2 | 3 => ay,
        _ => az,
    }
    .max(f32::EPSILON);
    let (a, b) = match face {
        0 => (-z, -y),
        1 => (z, -y),
        2 => (x, z),
        3 => (x, -z),
        4 => (x, -y),
        _ => (-x, -y),
    };
    let s = (a / major) * 0.5 + 0.5;
    let t = (b / major) * 0.5 + 0.5;
    (face, s.clamp(0.0, 1.0), t.clamp(0.0, 1.0))
}

pub fn cube_atlas_uv(face: u32, s: f32, t: f32, face_size: u32, gutter: u32) -> [f32; 2] {
    let cell = face_size + gutter * 2;
    let column = face % CUBE_COLUMNS;
    let row = face / CUBE_COLUMNS;
    let x = (column * cell) as f32 + gutter as f32 + s * face_size as f32;
    let y = (row * cell) as f32 + gutter as f32 + t * face_size as f32;
    let width = (cell * CUBE_COLUMNS) as f32;
    let height = (cell * 2) as f32;
    [x / width, y / height]
}

pub fn cube_map_extent(face_size: u32) -> (u32, u32) {
    let face = face_size.max(1);
    (face, face * CUBE_FACES)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
pub enum Domain {
    Equirect,
    Octahedral,
    Cube,
    CubeMap,
    Volume,
}

impl Domain {
    pub fn name(self) -> &'static str {
        match self {
            Self::Equirect => "equirect",
            Self::Octahedral => "octahedral",
            Self::Cube => "cube",
            Self::CubeMap => "cubemap",
            Self::Volume => "volume",
        }
    }

    pub fn code(self) -> u8 {
        match self {
            Self::Equirect => 0,
            Self::Octahedral => 1,
            Self::Cube => 2,
            Self::CubeMap => 3,
            Self::Volume => 4,
        }
    }

    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Equirect),
            1 => Some(Self::Octahedral),
            2 => Some(Self::Cube),
            3 => Some(Self::CubeMap),
            4 => Some(Self::Volume),
            _ => None,
        }
    }
}

pub fn volume_extent(res: u32, layers: u32) -> (u32, u32) {
    let res = res.max(1);
    let layers = layers.max(1);
    (res, res * layers * CUBE_FACES)
}

pub fn volume_layers(height: u32, res: u32) -> Option<u32> {
    let res = res.max(1);
    let block = res.checked_mul(CUBE_FACES)?;
    if height == 0 || height % block != 0 {
        return None;
    }
    let layers = height / block;
    (layers > 0).then_some(layers)
}

pub fn direction_at(domain: Domain, width: u32, height: u32, x: u32, y: u32) -> [f32; 3] {
    direction_at_opt(domain, width, height, x, y).unwrap_or_else(|| {
        panic!(
            "体网格（Domain::Volume）没有「一个方向」这回事：\
             世界点映射走 px_volume_schema::volume::point_of（它知道 inner/outer/res/layers）；\
             要「拿不到方向就自己决定怎么办」请用 direction_at_opt（见 docs/field.md）"
        )
    })
}

/// 同 [`direction_at`]，但**不可得时返回 `None` 而不是 panic**。
///
/// 需要它的理由：`Domain::Volume` 的格子不对应任何方向，而逐格调用方未必需要方向
/// （逐格算术、掩码、重映射都不需要）。⚠ 那个 panic 穿过算子 dylib 边界是**不可捕获**的
/// （`Rust cannot catch foreign exceptions`），会把整个进程带走 —— 所以任何"可能与
/// Volume 同处一条链"的逐格循环都不该走 [`direction_at`]。
pub fn direction_at_opt(
    domain: Domain,
    width: u32,
    height: u32,
    x: u32,
    y: u32,
) -> Option<[f32; 3]> {
    let u = (x as f32 + 0.5) / width.max(1) as f32;
    let v = (y as f32 + 0.5) / height.max(1) as f32;
    match domain {
        Domain::Equirect => {
            let theta = v.clamp(0.0, 1.0) * std::f32::consts::PI;
            let phi = u * std::f32::consts::TAU;
            let ring = theta.sin();
            Some([ring * phi.cos(), theta.cos(), ring * phi.sin()])
        }
        Domain::Octahedral => Some(octahedral_direction_y_up(u, v)),
        Domain::Cube => {
            let cell = cube_cell_size(width).max(1);
            let face_size = cube_face_size(width).max(1);
            let gutter = CUBE_GUTTER;
            let face = (y / cell) * CUBE_COLUMNS + (x / cell);
            let s = (x % cell) as f32 + 0.5 - gutter as f32;
            let t = (y % cell) as f32 + 0.5 - gutter as f32;
            Some(cube_direction(
                face,
                s / face_size as f32,
                t / face_size as f32,
            ))
        }
        Domain::CubeMap => {
            let face_size = width.max(1);
            let face = (y / face_size).min(CUBE_FACES - 1);
            let s = (x as f32 + 0.5) / face_size as f32;
            let t = ((y % face_size) as f32 + 0.5) / face_size as f32;
            Some(cube_direction(face, s, t))
        }
        Domain::Volume => None,
    }
}

pub fn uv_of(domain: Domain, direction: [f32; 3], width: u32, _height: u32) -> [f32; 2] {
    match domain {
        Domain::Equirect => {
            let v = direction[1].clamp(-1.0, 1.0).acos() / std::f32::consts::PI;
            let u = (direction[2].atan2(direction[0]) / std::f32::consts::TAU).rem_euclid(1.0);
            [u, v]
        }
        Domain::Octahedral => octahedral_uv_y_up(direction),
        Domain::Cube => {
            let (face, s, t) = cube_face_of(direction);
            cube_atlas_uv(face, s, t, cube_face_size(width), CUBE_GUTTER)
        }
        Domain::CubeMap => {
            let (face, s, t) = cube_face_of(direction);
            [s, (face as f32 + t) / CUBE_FACES as f32]
        }
        Domain::Volume => {
            let (_, s, t) = cube_face_of(direction);
            [s, t]
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "change", rename_all = "snake_case")]
pub enum AssetChange {
    Added,
    Removed,
    Content,
    Params {
        keys: Vec<String>,
    },
    Shape {
        before: Vec<BlobHeader>,
        after: Vec<BlobHeader>,
    },
}

impl AssetChange {
    pub fn describe(&self) -> String {
        match self {
            Self::Added => "新增".to_string(),
            Self::Removed => "移除".to_string(),
            Self::Content => "内容变了".to_string(),
            Self::Params { keys } => format!("参数变了（{}）", keys.join(",")),
            Self::Shape { before, after } => {
                format!("载荷 {}→{}", shape_text(before), shape_text(after))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetDiff {
    pub id: String,
    pub change: AssetChange,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Diff {
    pub changed: Vec<AssetDiff>,
    pub unchanged: Vec<String>,
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

impl Diff {
    pub fn is_identical(&self) -> bool {
        self.changed.is_empty() && self.added.is_empty() && self.removed.is_empty()
    }

    pub fn touched(&self) -> Vec<&str> {
        self.changed
            .iter()
            .map(|entry| entry.id.as_str())
            .chain(self.added.iter().map(String::as_str))
            .chain(self.removed.iter().map(String::as_str))
            .collect()
    }

    pub fn summary(&self) -> String {
        if self.is_identical() {
            return format!("零变化（{} 个产物逐项相同）", self.unchanged.len());
        }
        let mut parts: Vec<String> = self
            .changed
            .iter()
            .map(|entry| format!("{}：{}", entry.id, entry.change.describe()))
            .collect();
        parts.extend(self.added.iter().map(|id| format!("{id}：新增")));
        parts.extend(self.removed.iter().map(|id| format!("{id}：移除")));
        format!(
            "{} 个产物有变化（{} 个未动）：{}",
            self.changed.len() + self.added.len() + self.removed.len(),
            self.unchanged.len(),
            parts.join(" / ")
        )
    }
}

fn shape_text(headers: &[BlobHeader]) -> String {
    if headers.is_empty() {
        return "空".to_string();
    }
    headers
        .iter()
        .map(|header| format!("{:?}{:?}", header.dtype, header.shape))
        .collect::<Vec<_>>()
        .join("×")
}

pub fn diff(before: &ArtBundle, after: &ArtBundle) -> Diff {
    let mut out = Diff::default();
    for asset in &before.assets {
        let Some(other) = after.assets.iter().find(|entry| entry.id == asset.id) else {
            out.removed.push(asset.id.clone());
            continue;
        };
        match classify(asset, other) {
            Some(change) => out.changed.push(AssetDiff {
                id: asset.id.clone(),
                change,
            }),
            None => out.unchanged.push(asset.id.clone()),
        }
    }
    for asset in &after.assets {
        if !before.assets.iter().any(|entry| entry.id == asset.id) {
            out.added.push(asset.id.clone());
        }
    }
    out
}

fn classify(before: &AssetManifest, after: &AssetManifest) -> Option<AssetChange> {
    if before.fingerprint != 0 && after.fingerprint != 0 && before.fingerprint != after.fingerprint
    {
        return Some(AssetChange::Content);
    }
    let keys: Vec<String> = before
        .params
        .keys()
        .chain(after.params.keys())
        .filter(|key| before.params.get(*key) != after.params.get(*key))
        .cloned()
        .collect::<std::collections::BTreeSet<String>>()
        .into_iter()
        .collect();
    if !keys.is_empty() {
        return Some(AssetChange::Params { keys });
    }
    if before.blobs != after.blobs {
        return Some(AssetChange::Shape {
            before: before.blobs.clone(),
            after: after.blobs.clone(),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::DType;

    fn manifest(id: &str, fingerprint: u64, relief: f64, shape: [u32; 2]) -> AssetManifest {
        let mut params = BTreeMap::new();
        params.insert("relief".to_string(), relief);
        AssetManifest {
            id: id.to_string(),
            params,
            blobs: vec![BlobHeader {
                dtype: DType::F32,
                shape: shape.to_vec(),
            }],
            fingerprint,
        }
    }

    fn bundle(assets: Vec<AssetManifest>) -> ArtBundle {
        ArtBundle { assets }
    }

    #[test]
    fn identical_bundles_are_identical() {
        let one = bundle(vec![manifest("height", 7, 0.3, [4, 4])]);
        let report = diff(&one, &one.clone());
        assert!(report.is_identical());
        assert_eq!(report.unchanged, vec!["height".to_string()]);
        assert!(report.summary().contains("零变化"));
    }

    #[test]
    fn a_changed_fingerprint_beats_equal_params() {
        let before = bundle(vec![manifest("height", 7, 0.3, [4, 4])]);
        let after = bundle(vec![manifest("height", 8, 0.3, [4, 4])]);
        let report = diff(&before, &after);
        assert_eq!(report.changed.len(), 1);
        assert_eq!(report.changed[0].change, AssetChange::Content);
    }

    #[test]
    fn a_param_change_is_reported_key_by_key() {
        let before = bundle(vec![manifest("height", 0, 0.3, [4, 4])]);
        let after = bundle(vec![manifest("height", 0, 0.4, [4, 4])]);
        let report = diff(&before, &after);
        assert_eq!(
            report.changed[0].change,
            AssetChange::Params {
                keys: vec!["relief".to_string()]
            }
        );
        assert!(report.summary().contains("relief"));
    }

    #[test]
    fn added_and_removed_are_not_confused_with_changes() {
        let before = bundle(vec![manifest("height", 1, 0.3, [4, 4])]);
        let after = bundle(vec![manifest("clouds", 1, 0.3, [4, 4])]);
        let report = diff(&before, &after);
        assert_eq!(report.added, vec!["clouds".to_string()]);
        assert_eq!(report.removed, vec!["height".to_string()]);
        assert!(!report.is_identical());
        assert_eq!(report.touched().len(), 2);
    }

    #[test]
    fn a_param_change_is_a_payload_change() {
        let before = manifest("height", 7, 0.3, [4, 4]);
        let mut after = manifest("height", 7, 0.3, [4, 4]);
        after.params.insert("shift".to_string(), 1.0);
        assert!(
            !diff(&bundle(vec![before]), &bundle(vec![after])).is_identical(),
            "清单参数属于产物内容：改了它就该算变化（否则同名覆盖会静默留下旧内容）"
        );
    }

    #[test]
    fn the_manifest_frame_is_readable_from_a_prefix() {
        let bundle = bundle(vec![manifest("height", 7, 0.3, [4, 4])]);
        let mut bytes = Vec::new();
        crate::stream::write_stream(
            &mut bytes,
            &[
                crate::stream::Frame::Art(bundle.clone()),
                crate::stream::Frame::Blob(Blob::from_f32(vec![2, 2], &[0.0, 1.0, 2.0, 3.0])),
            ],
        )
        .unwrap();

        let length = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        let exact = 12 + length;
        assert!(exact < bytes.len(), "载荷应当还在后面");
        assert_eq!(bundle_from_prefix(&bytes[..exact]), Some(bundle.clone()));
        assert_eq!(bundle_from_prefix(&bytes), Some(bundle));
    }

    #[test]
    fn a_stream_that_does_not_start_with_a_manifest_falls_back() {
        let mut bytes = Vec::new();
        crate::stream::write_stream(
            &mut bytes,
            &[crate::stream::Frame::Blob(Blob::from_f32(vec![1], &[1.0]))],
        )
        .unwrap();
        assert_eq!(
            bundle_from_prefix(&bytes),
            None,
            "第一个帧不是清单 ⇒ 交给整读兜底"
        );
        assert_eq!(
            bundle_from_prefix(&bytes[..6]),
            None,
            "前缀太短也不许 panic"
        );
    }

    #[test]
    fn a_camera_normalizes_its_direction_and_keeps_a_distance_floor() {
        let camera = Camera::new([0.0, 0.0, 2.0], 0.0, "front");
        assert!((camera.direction[2] - 1.0).abs() < 1e-6);
        assert!(camera.distance >= 1e-3);
        let degenerate = Camera::new([0.0, 0.0, 0.0], 3.0, "degenerate");
        assert_eq!(degenerate.direction, [0.0, 0.0, 1.0]);
    }
}

pub fn read_shader(path: &std::path::Path) -> Result<String, String> {
    Ok(read_shader_parts(path)?.0)
}

pub fn read_shader_parts(path: &std::path::Path) -> Result<(String, Option<String>), String> {
    let bytes = std::fs::read(path).map_err(|err| format!("读不到 {}：{err}", path.display()))?;
    let frames =
        crate::stream::read_stream(&mut bytes.as_slice()).map_err(|err| err.to_string())?;
    let mut blobs = frames.iter().filter_map(|frame| match frame {
        crate::stream::Frame::Blob(blob) if blob.header.dtype == crate::wire::DType::U8 => {
            Some(blob)
        }
        _ => None,
    });
    let Some(wgsl) = blobs.next() else {
        return Err(format!(
            "{} 里没有 U8 blob（不是 shader 产物？）",
            path.display()
        ));
    };
    let source = String::from_utf8(wgsl.bytes.clone())
        .map_err(|err| format!("{} 的 WGSL 不是合法 UTF-8：{err}", path.display()))?;
    let schema = match blobs.next() {
        Some(blob) => Some(String::from_utf8(blob.bytes.clone()).map_err(|err| {
            format!(
                "{} 的 schema descriptor 不是合法 UTF-8：{err}",
                path.display()
            )
        })?),
        None => None,
    };
    Ok((source, schema))
}

pub fn read_bundle(path: &std::path::Path) -> Result<ArtBundle, String> {
    let bytes = std::fs::read(path).map_err(|err| format!("读不到 {}：{err}", path.display()))?;
    let frames =
        crate::stream::read_stream(&mut bytes.as_slice()).map_err(|err| err.to_string())?;
    bundle_of(&frames)
        .cloned()
        .ok_or_else(|| format!("{} 里没有 Art 帧", path.display()))
}

pub const MANIFEST_PREFIX: usize = 64 * 1024;

pub fn read_manifest(path: &std::path::Path) -> Result<ArtBundle, String> {
    let mut file =
        std::fs::File::open(path).map_err(|err| format!("读不到 {}：{err}", path.display()))?;
    let mut prefix = vec![0_u8; MANIFEST_PREFIX];
    let mut filled = 0;
    while filled < prefix.len() {
        match std::io::Read::read(&mut file, &mut prefix[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(err) => return Err(format!("读不到 {}：{err}", path.display())),
        }
    }
    prefix.truncate(filled);
    match bundle_from_prefix(&prefix) {
        Some(bundle) => Ok(bundle),
        None => read_bundle(path),
    }
}

fn bundle_from_prefix(prefix: &[u8]) -> Option<ArtBundle> {
    let rest = prefix.strip_prefix(&crate::stream::MAGIC[..])?;
    let version = u32::from_le_bytes(rest.get(..4)?.try_into().ok()?);
    if version != crate::stream::STREAM_VERSION {
        return None;
    }
    let length = u32::from_le_bytes(rest.get(4..8)?.try_into().ok()?) as usize;
    let payload = rest.get(8..8 + length)?;
    match crate::stream::Frame::decode(payload) {
        Ok(crate::stream::Frame::Art(bundle)) => Some(bundle),
        _ => None,
    }
}

pub fn bundle_of(frames: &[crate::stream::Frame]) -> Option<&ArtBundle> {
    frames.iter().find_map(|frame| match frame {
        crate::stream::Frame::Art(bundle) => Some(bundle),
        _ => None,
    })
}
