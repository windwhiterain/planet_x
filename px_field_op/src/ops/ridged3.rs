use px_field_schema::field::Field;
use px_field_schema::noise::FbmSettings;
use px_field_schema::ops::Ridged3;
use px_field_schema::params;
use px_field_schema::volume::voxel_of;

use crate::noise;

px_graph_schema::px_body! { Ridged3, |p, _i| crate::ops::ridged3::eval(p, &[])? }

pub fn eval(params: &params::Ridged3Params, _inputs: &[&Field]) -> Result<Field, String> {
    let shape = params.shape.volume_shape().unwrap_or_else(|| {
        panic!(
            "field.ridged3 要一张体网格画布（域 volume、行数 = res² × layers × 6），\
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
                out[base + x as usize] = noise::ridged_3(voxel, &settings, params.sharpness);
            }
        }
    })?;
    Ok(Field::with_projection(
        shape.res,
        shape.height(),
        data,
        params.shape.projection,
    ))
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

    #[test]
    fn the_ridges_are_normalised_and_vary_through_the_volume() {
        let shape = VolumeShape { res: 8, layers: 5 };
        let field = eval(
            &params::Ridged3Params {
                shape: shape_of(shape.res, shape.layers),
                ..Default::default()
            },
            &[],
        )
        .expect("测试夹具的行带不 panic");
        let stats = field.stats();
        assert!(stats.min >= 0.0 && stats.max <= 1.0, "{stats:?}");
        assert!(stats.max - stats.min > 0.05, "脊得有起伏：{stats:?}");
        let column: Vec<f32> = (0..shape.layers)
            .map(|layer| field.at(3, shape.row_of(0, layer)))
            .collect();
        let spread = column.iter().cloned().fold(f32::MIN, f32::max)
            - column.iter().cloned().fold(f32::MAX, f32::min);
        assert!(spread > 1e-4, "同一列的各层必须不同（实际跨度 {spread}）");
    }

    #[test]
    fn a_larger_sharpness_makes_the_ridges_thinner() {
        let shape = shape_of(16, 4);
        let share = |sharpness: f32| -> f64 {
            let field = eval(
                &params::Ridged3Params {
                    sharpness,
                    shape,
                    ..Default::default()
                },
                &[],
            )
            .expect("测试夹具的行带不 panic");
            let high = field.data.iter().filter(|value| **value > 0.5).count();
            high as f64 / field.data.len() as f64
        };
        let broad = share(1.0);
        let thin = share(4.0);
        assert!(
            thin < broad,
            "sharpness 变大应当让高值区变少：1.0 → {broad:.3}、4.0 → {thin:.3}"
        );
    }
}
