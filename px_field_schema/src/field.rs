//! 场：一张 `width × height` 的 f32 网格 + 它的投影。
//!
//! 画布（`Grid`）住在 `px_graph_schema`（它是「图交给算子的东西」）；这里给它补上场域
//! 需要的那两个 helper（`GridField`）—— Rust 的孤儿规则不许给外部类型写固有 impl。

use px_graph_schema::Grid;
use px_protocol::art::AssetKind;
use px_protocol::wire::{Blob, DType, WireError};

pub use px_protocol::art::Domain as Projection;
pub use px_protocol::art::{
    CUBE_FACES, cube_direction, cube_face_of, cube_map_extent, direction_at as art_direction_at,
};

pub trait ProjectionKind {
    fn asset_kind(self) -> AssetKind;
}

impl ProjectionKind for Projection {
    fn asset_kind(self) -> AssetKind {
        match self {
            Self::Equirect => AssetKind::Field2D,
            Self::Octahedral => AssetKind::OctahedralField,
            Self::Cube => AssetKind::CubeField,
            Self::CubeMap => AssetKind::CubeMap,
            // ⚠ **必须有自己的资产种类**（不能借 `Field2D`）：`load_field` 是
            //   "资产种类 → 域"的逆映射，两种域共用一个种类就读不回来了（体网格会被读成
            //   `Equirect`）—— 而域决定"这一格在世界里的哪"，是影不影响像素的开关。
            //   `AssetKind::Volume` 也不能借：那是**立方球体网格**（`VolumeData`，半径住在
            //   清单参数里、blob 是四维的），与这一档的"二维 blob + 折叠的第三维"不同形。
            Self::Volume => AssetKind::VoxelField,
        }
    }
}

/// 画布 → 场：`filled` 与 `direction` 是算子写得最顺手的两个入口。
pub trait GridField {
    fn filled(&self, value: f32) -> Field;
    fn direction(&self, x: u32, y: u32) -> [f32; 3];
}

impl GridField for Grid {
    fn filled(&self, value: f32) -> Field {
        Field::filled_with(self.width, self.height, value, self.projection)
    }

    fn direction(&self, x: u32, y: u32) -> [f32; 3] {
        direction_at(self.width, self.height, self.projection, x, y)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub width: u32,
    pub height: u32,
    pub data: Vec<f32>,
    pub projection: Projection,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stats {
    pub min: f32,
    pub max: f32,
    pub mean: f32,
}

pub fn normalize(vector: [f32; 3]) -> [f32; 3] {
    let length = (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt();
    if length <= f32::EPSILON {
        return [0.0, 1.0, 0.0];
    }
    [vector[0] / length, vector[1] / length, vector[2] / length]
}

pub fn cross(one: [f32; 3], two: [f32; 3]) -> [f32; 3] {
    [
        one[1] * two[2] - one[2] * two[1],
        one[2] * two[0] - one[0] * two[2],
        one[0] * two[1] - one[1] * two[0],
    ]
}

pub fn tangent_frame(direction: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let up = if direction[1].abs() > 0.99 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let east = normalize(cross(up, direction));
    let north = cross(direction, east);
    (east, north)
}

pub fn direction_at(width: u32, height: u32, projection: Projection, x: u32, y: u32) -> [f32; 3] {
    px_protocol::art::direction_at(projection, width, height, x, y)
}

impl Field {
    pub fn new(width: u32, height: u32, data: Vec<f32>) -> Self {
        Self::with_projection(width, height, data, Projection::Equirect)
    }

    pub fn with_projection(
        width: u32,
        height: u32,
        data: Vec<f32>,
        projection: Projection,
    ) -> Self {
        assert_eq!(
            data.len(),
            width as usize * height as usize,
            "场的像素数与尺寸不符"
        );
        Self {
            width,
            height,
            data,
            projection,
        }
    }

    pub fn filled(width: u32, height: u32, value: f32) -> Self {
        Self::filled_with(width, height, value, Projection::Equirect)
    }

    pub fn filled_with(width: u32, height: u32, value: f32, projection: Projection) -> Self {
        Self {
            width,
            height,
            data: vec![value; width as usize * height as usize],
            projection,
        }
    }

    pub fn at(&self, x: u32, y: u32) -> f32 {
        self.data[y as usize * self.width as usize + x as usize]
    }

    pub fn set(&mut self, x: u32, y: u32, value: f32) {
        self.data[y as usize * self.width as usize + x as usize] = value;
    }

    pub fn uv(&self, x: u32, y: u32) -> (f32, f32) {
        (
            (x as f32 + 0.5) / self.width.max(1) as f32,
            (y as f32 + 0.5) / self.height.max(1) as f32,
        )
    }

    pub fn direction(&self, x: u32, y: u32) -> [f32; 3] {
        direction_at(self.width, self.height, self.projection, x, y)
    }

    pub fn sample_direction(&self, direction: [f32; 3]) -> f32 {
        if self.projection == Projection::CubeMap {
            return self.sample_cube_map(direction);
        }
        let uv = px_protocol::art::uv_of(self.projection, direction, self.width, self.height);
        self.sample_uv(uv[0], uv[1])
    }

    fn cube_map_texel(&self, direction: [f32; 3]) -> f32 {
        let face_size = self.width.max(1);
        let (face, s, t) = px_protocol::art::cube_face_of(direction);
        let x = ((s * face_size as f32) as u32).min(face_size - 1);
        let y = ((t * face_size as f32) as u32).min(face_size - 1);
        self.at(x, face * face_size + y)
    }

    fn sample_cube_map(&self, direction: [f32; 3]) -> f32 {
        let face_size = self.width.max(1) as f32;
        let (face, s, t) = px_protocol::art::cube_face_of(direction);
        let x = s * face_size - 0.5;
        let y = t * face_size - 0.5;
        let (x0, y0) = (x.floor(), y.floor());
        let (tx, ty) = (x - x0, y - y0);

        let mut top = 0.0_f32;
        let mut bottom = 0.0_f32;
        for corner in 0..2 {
            let column = (x0 + corner as f32 + 0.5) / face_size;
            let weight = if corner == 1 { tx } else { 1.0 - tx };
            let upper = px_protocol::art::cube_direction(face, column, (y0 + 0.5) / face_size);
            let lower = px_protocol::art::cube_direction(face, column, (y0 + 1.5) / face_size);
            top += weight * self.cube_map_texel(upper);
            bottom += weight * self.cube_map_texel(lower);
        }
        top * (1.0 - ty) + bottom * ty
    }

    pub fn sample_uv(&self, u: f32, v: f32) -> f32 {
        let x = u * self.width.max(1) as f32 - 0.5;
        let y = v * self.height.max(1) as f32 - 0.5;
        self.sample_bilinear(x, y)
    }

    pub fn sample_bilinear(&self, x: f32, y: f32) -> f32 {
        let width = self.width.max(1);
        let height = self.height.max(1);
        let x = x.rem_euclid(width as f32);
        let y = y.clamp(0.0, height as f32 - 1.0);

        let x0 = x.floor() as u32 % width;
        let x1 = (x0 + 1) % width;
        let y0 = (y.floor() as u32).min(height - 1);
        let y1 = (y0 + 1).min(height - 1);
        let tx = x - x.floor();
        let ty = y - y.floor();

        let top = self.at(x0, y0) * (1.0 - tx) + self.at(x1, y0) * tx;
        let bottom = self.at(x0, y1) * (1.0 - tx) + self.at(x1, y1) * tx;
        top * (1.0 - ty) + bottom * ty
    }

    pub fn stats(&self) -> Stats {
        let mut min = f32::INFINITY;
        let mut max = f32::NEG_INFINITY;
        let mut sum = 0.0_f64;
        for value in &self.data {
            min = min.min(*value);
            max = max.max(*value);
            sum += *value as f64;
        }
        Stats {
            min,
            max,
            mean: if self.data.is_empty() {
                0.0
            } else {
                (sum / self.data.len() as f64) as f32
            },
        }
    }

    pub fn to_blob(&self) -> Blob {
        Blob::from_f32(vec![self.height, self.width], &self.data)
    }

    pub fn from_blob(blob: &Blob) -> Result<Self, WireError> {
        if blob.header.shape.len() != 2 {
            return Err(WireError::NotF32(blob.header.dtype));
        }
        let height = blob.header.shape[0];
        let width = blob.header.shape[1];
        let data = blob.f32s()?;
        Ok(Self::new(width, height, data))
    }

    pub fn is_f32_blob(blob: &Blob) -> bool {
        blob.header.dtype == DType::F32 && blob.header.shape.len() == 2
    }
}
