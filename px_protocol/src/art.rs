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
    /// 立方球参数空间里的 3D 标量网格（等值面算子的输入）。
    /// **渲染器不读它**：烘代理 mesh 是 PCG 那一侧的事，渲染器只认 `Mesh`。
    ///
    /// ⚠ 它是**体积**（`VolumeData`：`res` / `layers` / `inner` / `outer` 住在清单参数里），
    ///   与 [`Self::VoxelField`] 不是一回事 —— 见那一条。
    Volume,
    /// **体网格当一张场**（`Domain::Volume`）：一张普通的 `[height, width]` f32 网格，
    /// 第三维折进了 `height`（一面 = `res × (layers × res)` 行，见 `px_field_schema::volume`）。
    ///
    /// ⚠ 为什么它必须与 [`Self::Volume`] **分开**：两者的 blob 都是 f32，但**形状与含义不同**
    ///   （体网格场是二维 blob，`VolumeData` 是四维 `[面, 层, t, s]`，且半径另存）。
    ///   更要紧的是**域必须能从资产种类唯一还原**：`load_field` 是"资产种类 → 域"的逆映射，
    ///   把体网格场并进 `Field2D` 就会读回 `Equirect`（静默错域），而域决定"这一格在世界里的哪"
    ///   —— 那正是影不影响像素的开关。
    VoxelField,
    /// 场景配方：这次要渲什么、用什么参数、用哪个 shader 槽。
    Scene,
    /// Shader 源码（U8 blob）。它和场、网格一样是内容寻址的资产。
    Shader,
    /// 贴图载荷：像素 + 整条 mip 链，`layers` 层（1 = 2D、6 = cube）。
    ///
    /// 形状住在清单参数里（`width` / `height` / `layers` / `levels` / `format`）：
    /// 位深与 sRGB 不是 blob 头那一档（`DType`）能表达的东西。
    /// 渲染器**不生成**它 —— 色板、覆盖度立方图、星空都由烘图侧烘成产物（§65）。
    Texture,
}

/// 贴图产物的格式档。数字写进清单参数 `format`（清单参数只有 f64）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureFormat {
    /// 每通道 1 字节、sRGB 采样的 RGBA（颜色贴图、星空）。
    Rgba8Srgb,
    /// 每通道半精度的线性 RGBA（覆盖度立方图：mask + 三轴梯度）。
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

    /// 每个 texel 的字节数。
    pub fn texel_bytes(self) -> usize {
        match self {
            Self::Rgba8Srgb => 4,
            Self::Rgba16Float => 8,
        }
    }
}

/// 一份贴图产物的形状与格式（清单参数那一档）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextureShape {
    pub width: u32,
    pub height: u32,
    /// 1 = 2D，6 = cube。
    pub layers: u32,
    /// mip 链级数（含最细那一级）。
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

    /// 从清单参数里读回来。缺项/取值不认识 ⇒ `Err`，不猜。
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

    /// 整条 mip 链的字节数（从最细那一级逐级减半，直到某一维为 1）。
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

/// 一份贴图的**全部字节**：整条 mip 链，与渲染器今天写进 `Image.data` 的那串逐字节相同。
///
/// ⚠ **它为什么住这里**（与 [`VolumeData`] / [`MeshData`] 同住一处）：`AssetKind::Texture`
///   本来就在本模块，而"一个域的载荷类型与它的编解码住在一起"是全仓的口径
///   （孤儿规则那条）。从前它在 `px_graph::generate`，于是**算子交不出贴图**
///   —— 算子的 `Payload` 必须由 schema 层声明，而 schema 在 `px_graph` **下面**。
///   搬到这里之后 `sky.nebula` 那类算子可以直接把一张烘好的天空当产物交出去。
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

    /// **唯一的构造口**：自检「载荷字节数 = 形状算出来的整条 mip 链字节数」。
    /// 少一级 mip、多层一层、位深写错，都会在这里当场炸，而不是等到渲染器那边采样出错。
    ///
    /// ⚠ `px_graph` 那边原来把它写成**私有**的（"别绕过它"）。搬过来之后私有做不到
    ///   （跨 crate），于是它变成公开的 —— 但那条纪律没变：**要用贴图就过这一道**，
    ///   别去手搓结构体字面量。这也是为什么它叫 `new` 而字段是公开的：
    ///   字段公开是为了让 `Build::decode` 能从字节还原（那时字节已经是自己人写出来的）。
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

/// 立方球参数空间里的一张 3D 标量网格（`kind = Volume`）。
///
/// 排布：`data[((face * layers + layer) * res + t) * res + s]`，共
/// `CUBE_FACES * layers * res * res` 个值。`s`/`t` 是面内参数，`layer` 是径向高度层
/// （0 = `inner`、`layers-1` = `outer`）。
///
/// 它只是**等值面算子的输入**：渲染器不读它（见 `AssetKind::Volume` 的注释）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct VolumeData {
    pub res: u32,
    pub layers: u32,
    pub inner: f32,
    pub outer: f32,
    pub data: Vec<f32>,
}

/// Volume 载荷的 blob 形状：`[面, 径向层, t, s]`。
///
/// ⚠ 多通道体积在**末尾多一维**（通道数，只在 > 1 时写）—— 见 [`VolumeData::blobs`]。
pub const VOLUME_SHAPE: [u32; 4] = [CUBE_FACES, 0, 0, 0];

impl VolumeData {
    pub fn samples(&self) -> usize {
        self.res as usize * self.layers as usize * self.res as usize * CUBE_FACES as usize
    }

    /// 每格几条通道（`data.len() / samples()`）：密度是 1、发射是 6（3 发射 + 3 消光）。
    ///
    /// ⚠⚠ **通道数必须编进 blob 形状**（见 [`Self::blobs`]）—— 这一档踩过一次：
    ///   形状只写单通道的量、字节却是 `samples × 6` ⇒ 产物**自相矛盾**、读回被拒，
    ///   而症状只是"每次烘图都重算"（不报错、不崩溃）。
    pub fn lanes(&self) -> usize {
        let samples = self.samples().max(1);
        self.data.len() / samples
    }

    pub fn at(&self, face: u32, layer: u32, t: u32, s: u32) -> f32 {
        // ⚠ 多通道体积**不能**用 `at`：布局是交错的（`data[格 × lanes + 通道]`），
        //   单通道下标式只对 `lanes == 1` 有意义。
        debug_assert_eq!(self.lanes(), 1, "多通道体积请逐通道取（见 `lanes`）");
        self.data[(((face * self.layers + layer) * self.res + t) * self.res + s) as usize]
    }

    /// ⚠⚠ **形状要装得下所有字节**：`Blob` 的头按 `DType` 自检长度
    ///   （`elems × 4 == 字节数`），而多通道体积的 `data` 是 `samples × lanes`。
    ///   第一版形状只写 `[面, 层, t, s]`（单通道的量）却塞六通道的字节
    ///   ⇒ 写出的产物自相矛盾，读回时被"长度不符"拒收
    ///   （实测：`头部声明 6291456 字节，实际 37748736 字节` —— 正好差 6 倍）
    ///   ⇒ **每次烘图都判未命中、每次重算**，而画面对不对完全看不出这件事。
    ///
    /// 规则：**第 5 维只在 `lanes > 1` 时出现** ⇒ 一份内容只有一种编码，
    /// 而老的单通道产物（4 维形状）**照旧能读**。
    pub fn blobs(&self) -> Vec<Blob> {
        let mut shape = vec![CUBE_FACES, self.layers, self.res, self.res];
        if self.lanes() > 1 {
            shape.push(self.lanes() as u32);
        }
        vec![Blob::from_f32(shape, &self.data)]
    }

    /// 只从载荷里还原数据与形状：`inner`/`outer` 住在清单参数里（`px_graph` 负责补）。
    ///
    /// 形状 4 维 = 单通道（老形状）；第 5 维是通道数（只在 > 1 时写 —— 见 [`Self::blobs`]）。
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
            data,
        };
        if volume.data.len() != volume.samples() * lanes {
            return Err(WireError::TruncatedFrame);
        }
        Ok(volume)
    }
}

/// 一台评审相机。
///
/// `direction` 是**局部坐标系**里的方向（行星还没被 `SYSTEM_TILT` 转过去的那个系），
/// `distance` 以行星半径 1 为单位。渲染器负责套上自己的倾斜
/// （`px_render::planet::camera_for`），所以这里**不含**任何渲染器常数。
///
/// 它住在 `.pxart` 里 ⇒「这个产物该怎么看」跟产物一起走，
/// 不再散在 `tools/probe.ps1` 的 yaw/pitch 与那个 `$tilt = 0.34` 里。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    pub direction: [f32; 3],
    pub distance: f32,
    #[serde(default)]
    pub tag: String,
}

impl Camera {
    /// 方向归一化、距离保底。0 长度方向会让渲染器把相机放在原点。
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
    pub kind: AssetKind,
    pub params: BTreeMap<String, f64>,
    pub blobs: Vec<BlobHeader>,
    /// 载荷内容的 FNV-1a 指纹（0 = 没记过，旧产物）。
    /// diff 靠它分辨「参数一样、值不一样」—— CAS 路径能分辨，同名覆盖分辨不了。
    #[serde(default)]
    pub fingerprint: u64,
    /// 评审相机表。空 = 这个产物没带看法，渲染器走 `--cam`。
    #[serde(default)]
    pub cameras: Vec<Camera>,
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
    /// **三维**：立方球参数空间里的体网格（`res × res × layers × 6 面`）。
    ///
    /// ⚠ 前四个域是"**同一张网格的四种读法**"（都是球面上的一个方向），而这一个多了一维
    ///   —— 它不是"另一种投影"，是**另一类采样空间**。放进同一个枚举是因为算子读的
    ///   处处都是 `Field`（`width × height` 的 f32 网格 + 一个域），而"这一格在世界里
    ///   落在哪儿"完全由域决定 ⇒ 加一个域就让**整套场算法**多了一档可用空间，
    ///   而不必再造一类资产、一套 crate。⚠ 代价：`direction_at` / `uv_of` 这类
    ///   **只对球面有意义**的入口必须显式把这一档挡掉（见那两处的文档）。
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
}

/// 体网格的**画布尺寸**：`(res, res² × layers × 6)`。
///
/// ⚠ 为什么第三维折进 `height` 而不是给 `Field` 加一个 `layers` 字段：`Field` 是
///   `width × height` 的 f32 网格（[`Field::to_blob`] 写的就是 `[height, width]`），
///   加一维要动线格式、动每一个消费方。折进 `height` 之后**体网格就是一张普通场**，
///   逐元素算子（`remap` / `mix`）一行都不用改就能用。
///
/// ⚠ 一面是一块 `res × (layers × res)` 的平面（`res²·layers` 行），不是 `layers` 行 ——
///   一"层"占 `res` 行。行号是 `face × (res·layers) + layer × res + t`
///   （真源与推导见 `px_field_schema::volume` 的文件头）。
pub fn volume_extent(res: u32, layers: u32) -> (u32, u32) {
    let res = res.max(1);
    let layers = layers.max(1);
    (res, res * res * layers * CUBE_FACES)
}

/// 体网格的行数 → 层数（[`volume_extent`] 的逆）。
pub fn volume_layers(height: u32, res: u32) -> Option<u32> {
    let res = res.max(1);
    let block = res.checked_mul(res)?.checked_mul(CUBE_FACES)?;
    if height == 0 || height % block != 0 {
        return None;
    }
    let layers = height / block;
    (layers > 0).then_some(layers)
}

pub fn direction_at(domain: Domain, width: u32, height: u32, x: u32, y: u32) -> [f32; 3] {
    let u = (x as f32 + 0.5) / width.max(1) as f32;
    let v = (y as f32 + 0.5) / height.max(1) as f32;
    match domain {
        Domain::Equirect => {
            let theta = v.clamp(0.0, 1.0) * std::f32::consts::PI;
            let phi = u * std::f32::consts::TAU;
            let ring = theta.sin();
            [ring * phi.cos(), theta.cos(), ring * phi.sin()]
        }
        Domain::Octahedral => octahedral_direction_y_up(u, v),
        Domain::Cube => {
            let cell = cube_cell_size(width).max(1);
            let face_size = cube_face_size(width).max(1);
            let gutter = CUBE_GUTTER;
            let face = (y / cell) * CUBE_COLUMNS + (x / cell);
            let s = (x % cell) as f32 + 0.5 - gutter as f32;
            let t = (y % cell) as f32 + 0.5 - gutter as f32;
            cube_direction(face, s / face_size as f32, t / face_size as f32)
        }
        Domain::CubeMap => {
            let face_size = width.max(1);
            let face = (y / face_size).min(CUBE_FACES - 1);
            let s = (x as f32 + 0.5) / face_size as f32;
            let t = ((y % face_size) as f32 + 0.5) / face_size as f32;
            cube_direction(face, s, t)
        }
        // ⚠ 体网格**不是一个方向**：它多一维（径向层），而这一格的层号在 `height` 里
        //   （`y = face × layers + layer`）—— 这里只有 `height`，解不出 `layers`，
        //   于是拿不到真正的半径。⇒ 给"半径 1 的方向"会让调用方以为这是个方向场，
        //   那是**静默的错**。体网格的世界点映射住在 `px_field_schema::volume::point_of`
        //   （它知道 `inner` / `outer` / `res` / `layers`），球面那些入口一律走它。
        Domain::Volume => panic!(
            "体网格（Domain::Volume）没有「一个方向」这回事：\
             世界点映射走 px_field_schema::volume::point_of（它知道 inner/outer/res/layers）"
        ),
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
        // ⚠ 体网格的那一面里，这一格的面内参数就是 `(s, t)`（径向层不在这个二元组里，
        //   它由行号给）—— 见 [`direction_at`] 那一档的文档。
        Domain::Volume => {
            let (_, s, t) = cube_face_of(direction);
            [s, t]
        }
    }
}

// ---------------------------------------------------------------------------
// 产物 diff
//
// 渲染器拿到新的一批 `.pxart` 时，不该无脑把整场景重建一遍。它先问
// 「跟上次那份比，到底哪个节点的输出变了」—— 这就是这里回答的问题。
//
// 三档判据，从硬到软：指纹（内容）> 参数 > 载荷头（形状/类型）。
// 有了指纹，跨路径（新烘的产物换了 CAS 路径）也能分辨「值真的变了」。
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "change", rename_all = "snake_case")]
pub enum AssetChange {
    Added,
    Removed,
    Kind {
        before: AssetKind,
        after: AssetKind,
    },
    /// 载荷指纹不同 ⇒ 同样的参数烘出了不一样的值（最硬的一档）。
    Content,
    /// 哪些参数变了（含新增/删除）。
    Params {
        keys: Vec<String>,
    },
    /// 载荷的形状/类型变了（blob 头逐项对比）。
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
            Self::Kind { before, after } => format!("类型 {before:?}→{after:?}"),
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

    /// 变了的节点名（渲染器用它决定「哪些子资源要重做」）。
    pub fn touched(&self) -> Vec<&str> {
        self.changed
            .iter()
            .map(|entry| entry.id.as_str())
            .chain(self.added.iter().map(String::as_str))
            .chain(self.removed.iter().map(String::as_str))
            .collect()
    }

    /// 一行能给人/agent 看的摘要。
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

/// 按 `id`（节点名）配对两份清单，逐项给出最硬的那条差异。
///
/// ⚠️ 配对靠节点名。图里的节点名一改，这份 diff 只会说「一个新增一个移除」——
/// 这是诚实的：名字变了就不再是同一样东西。
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
    if before.kind != after.kind {
        return Some(AssetChange::Kind {
            before: before.kind,
            after: after.kind,
        });
    }
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
            kind: AssetKind::Field2D,
            params,
            blobs: vec![BlobHeader {
                dtype: DType::F32,
                shape: shape.to_vec(),
            }],
            fingerprint,
            cameras: Vec::new(),
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
        // 参数逐项相同、只有值变了 —— 正是「换了 CAS 路径 / 同名覆盖」那一档
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
    fn a_camera_change_is_not_a_payload_change() {
        let mut before = manifest("height", 7, 0.3, [4, 4]);
        let mut after = before.clone();
        before.cameras = vec![Camera::new([0.0, 0.0, 1.0], 3.15, "front")];
        after.cameras = vec![Camera::new([1.0, 1.0, 1.0], 1.4, "corner")];
        assert!(
            diff(&bundle(vec![before]), &bundle(vec![after])).is_identical(),
            "相机表属于「怎么看」不属于「是什么」：值没变就不该算变化"
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

        // 前缀恰好停在清单帧末尾（载荷一个字节都没读）也必须解得出来：
        // 服务端就是靠这个先把「变没变」问清楚的。帧头 = magic 4 + 版本 4 + 长度 4。
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

/// 读一份 Shader 产物（`kind = Shader`）里的 WGSL 文本。
/// 它和场、网格走同一条 CAS：内容键 → 路径 → 载荷。
pub fn read_shader(path: &std::path::Path) -> Result<String, String> {
    Ok(read_shader_parts(path)?.0)
}

/// 读一份 Shader 产物的两半：**WGSL 文本** + **schema descriptor**（规范 JSON）。
///
/// 约定：第一个 U8 blob 是 WGSL，第二个（如果有）是 descriptor ——
/// 它由烘图侧用 `px_shader::reflect` 算出来（契约收口之后，§74.3）。
/// 老产物只有第一个 blob ⇒ `None`：调用方该当场拒并给重烘配方，
/// 与「闭包指纹没记过」同款（§52.3）—— 那一版没有 descriptor，认它等于认错契约。
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

/// 从 `.pxart` / `.pxstream` 里取出第一份产物清单。
pub fn read_bundle(path: &std::path::Path) -> Result<ArtBundle, String> {
    let bytes = std::fs::read(path).map_err(|err| format!("读不到 {}：{err}", path.display()))?;
    let frames =
        crate::stream::read_stream(&mut bytes.as_slice()).map_err(|err| err.to_string())?;
    bundle_of(&frames)
        .cloned()
        .ok_or_else(|| format!("{} 里没有 Art 帧", path.display()))
}

/// 读清单要预读多少字节。清单是 JSON（载荷指纹 + 相机表 + blob 头），几 KB 足够；
/// 给到 64 KB 是为了容下相机表以后长胖。
pub const MANIFEST_PREFIX: usize = 64 * 1024;

/// 只读**清单帧**，不碰载荷。
///
/// 服务端每次请求都要先问「这份产物跟上次那份是不是同一份」（缓存键 = 路径 + 载荷指纹），
/// 而清单帧**写在流的最前面**（`px_graph::write_artifact` 如此）⇒ 读一个前缀就够，
/// 不必把 8 MB 的场整个读进来。前缀里没解出清单帧（文件不是那么写的）⇒ 回落到整读。
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

/// 前缀里的**第一个**帧就是清单帧时返回它，否则 `None`（说明这份流不是那么排的，交给整读）。
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

/// 一组帧里的第一份清单。
pub fn bundle_of(frames: &[crate::stream::Frame]) -> Option<&ArtBundle> {
    frames.iter().find_map(|frame| match frame {
        crate::stream::Frame::Art(bundle) => Some(bundle),
        _ => None,
    })
}

/// 一份清单里声明的评审相机（第一份带相机表的产物说了算）。
pub fn cameras_of(bundle: &ArtBundle) -> &[Camera] {
    bundle
        .assets
        .iter()
        .find(|asset| !asset.cameras.is_empty())
        .map(|asset| asset.cameras.as_slice())
        .unwrap_or(&[])
}
