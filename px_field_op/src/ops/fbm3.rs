use px_field_schema::field::Field;
use px_field_schema::noise::FbmSettings;
use px_field_schema::ops::Fbm3;
use px_field_schema::params;
use px_field_schema::volume::voxel_of;

use crate::noise;

px_graph_schema::px_body! { Fbm3, |p, _i| crate::ops::fbm3::eval(p, &[]) }

pub fn eval(params: &params::Fbm3Params, _inputs: &[&Field]) -> Field {
    let shape = params.shape.volume_shape().unwrap_or_else(|| {
        panic!(
            "field.fbm3 要一张体网格画布（域 volume、行数 = res² × layers × 6），\
             拿到的是 {:?} {}×{}",
            params.shape.projection, params.shape.width, params.shape.height
        )
    });
    let settings = FbmSettings {
        frequency: params.frequency,
        octaves: params.octaves,
        lacunarity: params.lacunarity,
        gain: params.gain,
        seed: params.seed,
    };
    let width = shape.res as usize;
    let height = shape.height() as usize;
    let data = crate::parallel::rows(width, height, |first, count, out| {
        for row in 0..count {
            let y = (first + row) as u32;
            let (face, _) = shape.slot_of(y).expect("行号在形状之内");
            let base = row * width;
            for x in 0..shape.res {
                let mut voxel = voxel_of(&shape, face, x, y);
                for axis in 0..3 {
                    voxel[axis] *= params.zonal;
                }
                out[base + x as usize] = noise::fbm_3(voxel, &settings);
            }
        }
    });
    Field::with_projection(shape.res, shape.height(), data, params.shape.projection)
}

#[cfg(test)]
mod tests {
    use super::*;
    use px_field_schema::field::CUBE_FACES;
    use px_field_schema::field::Projection;
    use px_field_schema::params::Shape;
    use px_field_schema::volume::VolumeShape;

    fn shape_of(res: u32, layers: u32) -> Shape {
        Shape {
            width: res,
            height: res * layers * CUBE_FACES,
            projection: Projection::Volume,
        }
    }

    fn eval_default(shape: Shape) -> Field {
        eval(
            &params::Fbm3Params {
                shape,
                ..Default::default()
            },
            &[],
        )
    }

    #[test]
    fn the_field_takes_the_shape_params_and_stays_normalised() {
        let shape = shape_of(8, 4);
        let field = eval_default(shape);
        assert_eq!(field.width, 8);
        assert_eq!(field.height, shape.height);
        let stats = field.stats();
        assert!(stats.min >= 0.0 && stats.max <= 1.0, "{stats:?}");
        assert!(stats.max - stats.min > 0.05, "噪声得有起伏：{stats:?}");
    }

    #[test]
    fn the_column_changes_with_the_layer() {
        let shape = VolumeShape { res: 8, layers: 6 };
        let field = eval_default(shape_of(shape.res, shape.layers));
        let mut changed = 0;
        for face in 0..CUBE_FACES {
            for x in 0..shape.res {
                let column: Vec<f32> = (0..shape.layers)
                    .map(|layer| field.at(x, shape.row_of(face, layer)))
                    .collect();
                let spread = column.iter().cloned().fold(f32::MIN, f32::max)
                    - column.iter().cloned().fold(f32::MAX, f32::min);
                if spread > 1e-4 {
                    changed += 1;
                }
            }
        }
        let total = CUBE_FACES * shape.res;
        assert_eq!(
            changed, total,
            "每一列都该随层变化（{} / {total} 列变了）—— 不变就是退化成球面档了",
            changed
        );
    }

    #[test]
    fn the_six_faces_are_not_copies_of_each_other() {
        let shape = VolumeShape { res: 6, layers: 3 };
        let field = eval_default(shape_of(shape.res, shape.layers));
        let mut equal = 0;
        for layer in 0..shape.layers {
            for x in 0..shape.res {
                let first = field.at(x, shape.row_of(0, layer));
                for face in 1..CUBE_FACES {
                    if (field.at(x, shape.row_of(face, layer)) - first).abs() < 1e-6 {
                        equal += 1;
                    }
                }
            }
        }
        assert_eq!(equal, 0, "六面里有 {equal} 格与面 0 完全相同");
    }

    #[test]
    fn the_same_parameters_give_the_same_field() {
        let shape = shape_of(6, 3);
        let one = eval_default(shape);
        let two = eval_default(shape);
        assert_eq!(one.data, two.data);
    }
}
