use px_field_schema::field::Field;
use px_field_schema::noise::FbmSettings;
use px_field_schema::ops::Fbm;
use px_field_schema::params;

use crate::noise;

px_graph_schema::px_body! { Fbm, |p, _i| crate::ops::fbm::eval(p, &[]) }

pub fn eval(params: &params::fbm::Params, _inputs: &[&Field]) -> Field {
    let settings = FbmSettings {
        frequency: params.frequency,
        octaves: params.octaves,
        lacunarity: params.lacunarity,
        gain: params.gain,
        seed: params.seed,
    };
    let mut field = params.shape.filled(0.0);
    for y in 0..params.shape.height {
        for x in 0..params.shape.width {
            let (u, v) = field.uv(x, y);
            let value = if params.spherical {
                let direction = field.direction(x, y);
                noise::fbm_3(
                    [direction[0], direction[1] * params.zonal, direction[2]],
                    &settings,
                )
            } else {
                noise::fbm(u * params.aspect, v, &settings)
            };
            field.set(x, y, value);
        }
    }
    field
}

#[cfg(test)]
mod tests {
    use super::*;
    use px_field_schema::field::Projection;
    use px_field_schema::params::Shape;

    fn anisotropy(zonal: f32) -> f32 {
        let params = params::fbm::Params {
            frequency: 3.0,
            octaves: 4,
            zonal,
            shape: Shape {
                width: 256,
                height: 1536,
                projection: Projection::CubeMap,
            },
            ..Default::default()
        };
        let field = eval(&params, &[]);
        let (mut along_lon, mut along_lat) = (0.0_f64, 0.0_f64);
        for y in 1..params.shape.height {
            for x in 1..params.shape.width {
                let here = field.at(x, y) as f64;
                along_lon += (here - field.at(x - 1, y) as f64).abs();
                along_lat += (here - field.at(x, y - 1) as f64).abs();
            }
        }
        (along_lon / along_lat) as f32
    }

    #[test]
    fn zonal_stretches_the_noise_along_longitude() {
        let isotropic = anisotropy(1.0);
        let stretched = anisotropy(4.0);
        assert!(
            stretched < isotropic * 0.6,
            "`zonal = 4` 应当把经向变化压到各向同性时的六成以下：             各向同性 {isotropic:.3}、拉长后 {stretched:.3}"
        );
        assert!(
            stretched > 0.0,
            "拉长不等于抹平：沿经度仍要有变化（{stretched:.3}）"
        );
    }

    #[test]
    fn zonal_one_is_the_old_behaviour_point_by_point() {
        let shape = Shape {
            width: 64,
            height: 384,
            projection: Projection::CubeMap,
        };
        let with_one = eval(
            &params::fbm::Params {
                zonal: 1.0,
                shape,
                ..Default::default()
            },
            &[],
        );
        let settings = FbmSettings {
            frequency: params::fbm::Params::default().frequency,
            octaves: params::fbm::Params::default().octaves,
            lacunarity: params::fbm::Params::default().lacunarity,
            gain: params::fbm::Params::default().gain,
            seed: params::fbm::Params::default().seed,
        };
        let mut old = shape.filled(0.0);
        for y in 0..shape.height {
            for x in 0..shape.width {
                old.set(x, y, crate::noise::fbm_3(old.direction(x, y), &settings));
            }
        }
        let mut worst = 0.0_f32;
        for y in 0..shape.height {
            for x in 0..shape.width {
                worst = worst.max((with_one.at(x, y) - old.at(x, y)).abs());
            }
        }
        assert!(
            worst == 0.0,
            "`zonal = 1.0` 必须逐点等价于旧行为，最大偏差 {worst}"
        );
    }
}
