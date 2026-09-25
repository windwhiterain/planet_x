use px_protocol::art::{CUBE_FACES, Domain, cube_direction};

use crate::field::{Field, Projection};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolumeShape {
    pub res: u32,
    pub layers: u32,
}

impl VolumeShape {
    pub fn of(width: u32, height: u32, projection: Projection) -> Option<Self> {
        if projection != Projection::Volume {
            return None;
        }
        let res = width.max(1);
        let layers = px_protocol::art::volume_layers(height, res)?;
        Some(Self { res, layers })
    }

    pub fn of_field(field: &Field) -> Option<Self> {
        Self::of(field.width, field.height, field.projection)
    }

    pub fn height(&self) -> u32 {
        px_protocol::art::volume_extent(self.res, self.layers).1
    }

    pub fn slot_of(&self, y: u32) -> Option<(u32, u32)> {
        let res = self.res.max(1);
        let layers = self.layers.max(1);
        if y >= self.height() {
            return None;
        }
        let plane = res * layers;
        let face = y / plane;
        let layer = (y % plane) / res;
        Some((face.min(CUBE_FACES - 1), layer.min(layers - 1)))
    }

    pub fn row_of(&self, face: u32, layer: u32) -> u32 {
        let res = self.res.max(1);
        let layers = self.layers.max(1);
        face * (res * layers) + layer.min(layers - 1) * res
    }

    pub fn matches(&self, field: &Field) -> bool {
        field.projection == Domain::Volume
            && field.width == self.res.max(1)
            && field.height == self.height()
    }
}

pub const SHELL_FRONT: f32 = std::f32::consts::FRAC_2_PI;

pub fn voxel_of(shape: &VolumeShape, face: u32, x: u32, y: u32) -> [f32; 3] {
    let local = local_voxel_of(shape, face, x, y);
    let radius = SHELL_FRONT + local[2];
    let direction = cube_direction(face % CUBE_FACES, local[0], local[1]);
    [
        direction[0] * radius,
        direction[1] * radius,
        direction[2] * radius,
    ]
}

pub fn in_face_row(shape: &VolumeShape, y: u32) -> u32 {
    let res = shape.res.max(1);
    y % (res * shape.layers.max(1))
}

pub fn row_within_layer(shape: &VolumeShape, y: u32) -> u32 {
    in_face_row(shape, y) % shape.res.max(1)
}

pub fn layer_of(shape: &VolumeShape, y: u32) -> u32 {
    in_face_row(shape, y) / shape.res.max(1)
}

pub fn local_voxel_of(shape: &VolumeShape, face: u32, x: u32, y: u32) -> [f32; 3] {
    let res = shape.res.max(1);
    let last = shape.layers.max(2) - 1;
    let in_face = y.saturating_sub(face * res * shape.layers.max(1));
    let layer = in_face / res;
    let t = in_face % res;
    [
        (x as f32 + 0.5) / res as f32,
        (t as f32 + 0.5) / res as f32,
        layer.min(last) as f32 / last as f32,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape() -> VolumeShape {
        VolumeShape { res: 4, layers: 3 }
    }

    #[test]
    fn a_row_decodes_back_to_its_own_face_and_layer() {
        let shape = shape();
        assert_eq!(shape.height(), 4 * 3 * CUBE_FACES);
        let mut seen = 0;
        for face in 0..CUBE_FACES {
            for layer in 0..shape.layers {
                let row = shape.row_of(face, layer);
                assert_eq!(shape.slot_of(row), Some((face, layer)));
                seen += 1;
            }
        }
        assert_eq!(seen, CUBE_FACES * shape.layers);
        assert_eq!(
            shape.slot_of(shape.height()),
            None,
            "越界的行要明确说不认识"
        );
    }

    #[test]
    fn the_shape_comes_back_out_of_the_shape_params() {
        let shape = shape();
        assert_eq!(
            VolumeShape::of(shape.res, shape.height(), Domain::Volume),
            Some(shape)
        );
        assert_eq!(
            VolumeShape::of(shape.res, shape.height() + 1, Domain::Volume),
            None
        );
        assert_eq!(
            VolumeShape::of(shape.res, shape.height(), Domain::CubeMap),
            None
        );
    }

    #[test]
    fn a_volume_needs_the_domain_and_both_extents() {
        let shape = shape();
        let good = Field::filled_with(shape.res, shape.height(), 0.0, Domain::Volume);
        assert!(shape.matches(&good));
        let wrong_domain = Field::filled_with(shape.res, shape.height(), 0.0, Domain::CubeMap);
        assert!(!shape.matches(&wrong_domain));
        let wrong_rows = Field::filled_with(shape.res, shape.height() - 1, 0.0, Domain::Volume);
        assert!(!shape.matches(&wrong_rows));
        let wrong_columns = Field::filled_with(shape.res + 1, shape.height(), 0.0, Domain::Volume);
        assert!(!shape.matches(&wrong_columns));
    }

    #[test]
    fn a_voxel_coordinate_is_a_point_in_three_dimensional_space() {
        let shape = shape();
        let length = |v: [f32; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        let mut last = f32::NEG_INFINITY;
        for layer in 0..shape.layers {
            let row = shape.row_of(0, layer);
            let radius = length(voxel_of(&shape, 0, 1, row));
            assert!(
                (SHELL_FRONT..=SHELL_FRONT + 1.0 + 1e-4).contains(&radius),
                "半径跑出壳：{radius}"
            );
            assert!(radius > last, "层号越大半径必须越大：{radius}");
            last = radius;
        }
    }

    #[test]
    fn the_two_faces_meet_on_the_shared_edge() {
        let shape = VolumeShape { res: 8, layers: 4 };
        let texel = std::f32::consts::FRAC_PI_2 * SHELL_FRONT / shape.res as f32;
        for layer in 0..shape.layers {
            let left = voxel_of(&shape, 0, 0, shape.row_of(0, layer) + 3);
            let right = voxel_of(&shape, 4, shape.res - 1, shape.row_of(4, layer) + 3);
            let gap = (0..3)
                .map(|axis| (left[axis] - right[axis]).powi(2))
                .sum::<f32>()
                .sqrt();
            assert!(
                gap < texel * 3.0,
                "公共棱两侧的点差了 {gap}（一个纹素约 {texel}）—— 场在棱上是断的"
            );
        }
    }

    #[test]
    fn the_in_face_coordinates_do_not_leak_across_faces() {
        let shape = VolumeShape { res: 4, layers: 3 };
        for face in 0..CUBE_FACES {
            for layer in 0..shape.layers {
                for t in 0..shape.res {
                    let y = shape.row_of(face, layer) + t;
                    assert_eq!(layer_of(&shape, y), layer, "面 {face} 行 {y} 的层号");
                    assert_eq!(
                        row_within_layer(&shape, y),
                        t,
                        "面 {face} 行 {y} 的层内格号"
                    );
                    assert_eq!(
                        in_face_row(&shape, y),
                        layer * shape.res + t,
                        "面 {face} 行 {y} 的面内行号"
                    );
                    let voxel = local_voxel_of(&shape, face, 1, y);
                    assert!(
                        (voxel[2] - layer as f32 / (shape.layers - 1) as f32).abs() < 1e-6,
                        "面 {face} 行 {y} 的高度应当是第 {layer} 层"
                    );
                }
            }
        }
    }

    #[test]
    fn the_six_faces_do_not_share_one_noise_region() {
        let shape = shape();
        let mut points: Vec<[f32; 3]> = Vec::new();
        for face in 0..CUBE_FACES {
            points.push(voxel_of(&shape, face, 1, shape.row_of(face, 1) + 1));
        }
        for one in 0..points.len() {
            for two in (one + 1)..points.len() {
                let gap = (0..3)
                    .map(|axis| (points[one][axis] - points[two][axis]).abs())
                    .fold(0.0_f32, f32::max);
                assert!(
                    gap > 0.3,
                    "面 {one} 与面 {two} 的采样点几乎重合（差 {gap}）"
                );
            }
        }
    }
}
