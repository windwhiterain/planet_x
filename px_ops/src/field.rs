use px_protocol::art::{AssetKind, octahedral_direction_y_up};
use px_protocol::wire::{Blob, DType, WireError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Projection {
    Equirect,
    Octahedral,
    Cube,
}

impl Projection {
    pub fn asset_kind(self) -> AssetKind {
        match self {
            Self::Equirect => AssetKind::Field2D,
            Self::Octahedral => AssetKind::OctahedralField,
            Self::Cube => AssetKind::CubeField,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Equirect => "equirect",
            Self::Octahedral => "octahedral",
            Self::Cube => "cube",
        }
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

pub fn direction_at(
    width: u32,
    height: u32,
    projection: Projection,
    x: u32,
    y: u32,
) -> [f32; 3] {
    let u = (x as f32 + 0.5) / width.max(1) as f32;
    let v = (y as f32 + 0.5) / height.max(1) as f32;
    match projection {
        Projection::Equirect => {
            let theta = v.clamp(0.0, 1.0) * std::f32::consts::PI;
            let phi = u * std::f32::consts::TAU;
            let ring = theta.sin();
            [ring * phi.cos(), theta.cos(), ring * phi.sin()]
        }
        Projection::Octahedral => px_protocol::art::octahedral_direction_y_up(u, v),
        Projection::Cube => {
            let cell = px_protocol::art::cube_cell_size(width).max(1);
            let face_size = px_protocol::art::cube_face_size(width).max(1);
            let gutter = px_protocol::art::CUBE_GUTTER;
            let face = (y / cell) * px_protocol::art::CUBE_COLUMNS + (x / cell);
            let s = (x % cell) as f32 + 0.5 - gutter as f32;
            let t = (y % cell) as f32 + 0.5 - gutter as f32;
            px_protocol::art::cube_direction(face, s / face_size as f32, t / face_size as f32)
        }
    }
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
        match self.projection {
            Projection::Octahedral => {
                let uv = px_protocol::art::octahedral_uv_y_up(direction);
                self.sample_uv(uv[0], uv[1])
            }
            Projection::Equirect => {
                let v = direction[1].clamp(-1.0, 1.0).acos() / std::f32::consts::PI;
                let u = (direction[2].atan2(direction[0]) / std::f32::consts::TAU).rem_euclid(1.0);
                self.sample_uv(u, v)
            }
            Projection::Cube => {
                let (face, s, t) = px_protocol::art::cube_face_of(direction);
                let face_size = px_protocol::art::cube_face_size(self.width);
                let uv = px_protocol::art::cube_atlas_uv(
                    face,
                    s,
                    t,
                    face_size,
                    px_protocol::art::CUBE_GUTTER,
                );
                self.sample_uv(uv[0], uv[1])
            }
        }
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




