use px_protocol::wire::{Blob, DType, WireError};

pub use px_protocol::art::Domain as Projection;
pub use px_protocol::art::{
    CUBE_FACES, cube_direction, cube_face_of, cube_map_extent, direction_at as art_direction_at,
    direction_at_opt as art_direction_at_opt,
};

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

/// 每个方向返回一组右手正交的切向基 `(east, north)`，`north = direction × east`。
///
/// 契约：`east` 是**把南极点转到 `direction` 的那次旋转**作用在 `x̂` 上的像
/// （`(0,-1,0) × direction` 归一化），所以它是方向的连续函数。全空间只有
/// `direction = (0,-1,0)` 一个方向无定义 —— 极点本身没有切向基；面心落在极点的立方图上
/// 取不到那个方向（面尺寸 `n` 时最近的面心离极点 `1/(3n²)`）。
pub fn tangent_frame(direction: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let east = normalize(cross([0.0, -1.0, 0.0], direction));
    let north = cross(direction, east);
    (east, north)
}

pub fn direction_at(width: u32, height: u32, projection: Projection, x: u32, y: u32) -> [f32; 3] {
    px_protocol::art::direction_at(projection, width, height, x, y)
}

impl Field {
    pub fn like(&self, value: f32) -> Field {
        Field::filled_with(self.width, self.height, value, self.projection)
    }

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

    /// 同 [`Field::direction`]，但投影没有方向时返回 `None`。
    ///
    /// 逐格循环里**必须先探再取**：`Domain::Volume` 没有方向，而逐格去问会 panic ——
    /// 那个 panic 穿过算子 dylib 边界不可捕获，会直接 abort 进程。不需要方向的逐格算术
    /// （重映射、混合、掩码）应当用它，拿 `None` 时自己决定怎么办。
    pub fn direction_probe(&self) -> Option<[f32; 3]> {
        art_direction_at_opt(self.projection, self.width, self.height, 0, 0)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 切向基必须**连续**：拿 `|direction[1]| == 0.99` 当换轴的门槛会让 `east` 在那条环上
    /// 整体翻转 —— 本判据扫过它。
    #[test]
    fn the_tangent_frame_does_not_reverse_along_the_polar_ring() {
        let hold = 0.2_f32;
        let steps = 400;
        let mut previous: Option<([f32; 3], [f32; 3])> = None;
        let mut worst = 0.0_f32;
        for index in 0..=steps {
            let along = 0.985 + 0.01 * index as f32 / steps as f32;
            let direction = normalize([hold, hold * along / (1.0 - along * along).sqrt(), 0.0]);
            let (east, north) = tangent_frame(direction);
            for axis in [east, north] {
                assert!(
                    (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2] - 1.0).abs() < 1e-4,
                    "切向基不是单位向量：{axis:?}"
                );
                assert!(
                    (axis[0] * direction[0] + axis[1] * direction[1] + axis[2] * direction[2])
                        .abs()
                        < 1e-4,
                    "{axis:?} 不与方向 {:?} 垂直",
                    direction
                );
            }
            if let Some((last_east, last_north)) = previous {
                for (here, last) in [(east, last_east), (north, last_north)] {
                    let across = cross(here, last);
                    let sine =
                        (across[0] * across[0] + across[1] * across[1] + across[2] * across[2])
                            .sqrt();
                    let cosine = here[0] * last[0] + here[1] * last[1] + here[2] * last[2];
                    worst = worst.max(sine.atan2(cosine).to_degrees());
                }
            }
            previous = Some((east, north));
        }
        assert!(
            worst < 0.5,
            "切向基在这条环上跳了 {worst:.3} 度 ⇒ 换轴的门槛还在"
        );
    }

    #[test]
    fn a_cube_map_shape_must_be_a_whole_number_of_faces() {
        let whole = crate::params::Shape {
            width: 256,
            height: 256 * 6,
            projection: Projection::CubeMap,
        };
        assert_eq!(whole.check(), Ok(()));
        let truncated = crate::params::Shape {
            width: 512,
            height: 256,
            projection: Projection::CubeMap,
        };
        let message = truncated.check().expect_err("缺面的立方图必须被拒");
        for named in ["width = 512", "height = 256", "3072"] {
            assert!(message.contains(named), "错误信息没点出 {named}：{message}");
        }
        let flat = crate::params::Shape {
            width: 512,
            height: 256,
            projection: Projection::Equirect,
        };
        assert_eq!(flat.check(), Ok(()), "2:1 的等距柱状投影是合法形状");
    }
}
