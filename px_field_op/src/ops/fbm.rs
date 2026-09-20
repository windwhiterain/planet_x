use px_field_schema::field::{Field, GridField};
use px_field_schema::noise::FbmSettings;
use px_field_schema::ops::Fbm;
use px_field_schema::params;
use px_graph_schema::Grid;

use crate::noise;

px_graph_schema::px_body! { Fbm, |p, _i, g| crate::ops::fbm::eval(p, &[], g) }

pub fn eval(params: &params::fbm::Params, _inputs: &[&Field], grid: Grid) -> Field {
    let settings = FbmSettings {
        frequency: params.frequency,
        octaves: params.octaves,
        lacunarity: params.lacunarity,
        gain: params.gain,
        seed: params.seed,
    };
    let mut field = grid.filled(0.0);
    for y in 0..grid.height {
        for x in 0..grid.width {
            let (u, v) = field.uv(x, y);
            let value = if params.spherical {
                // ⚠ `zonal` 只动采样点的**纬度分量**：噪声在经度方向被拉长 `zonal` 倍
                //   （`1.0` 时这一行就是原样，逐字节等价于没有这一栏）。
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

    /// 在球面档上量"沿经度 vs 沿纬度"的平均 |Δ|（同一张 CubeMap 画布）。
    fn anisotropy(zonal: f32) -> f32 {
        let grid = Grid {
            width: 256,
            height: 1536,
            projection: Projection::CubeMap,
        };
        let params = params::fbm::Params {
            frequency: 3.0,
            octaves: 4,
            zonal,
            ..Default::default()
        };
        let field = eval(&params, &[], grid);
        let (mut along_lon, mut along_lat) = (0.0_f64, 0.0_f64);
        for y in 1..grid.height {
            for x in 1..grid.width {
                let here = field.at(x, y) as f64;
                along_lon += (here - field.at(x - 1, y) as f64).abs();
                along_lat += (here - field.at(x, y - 1) as f64).abs();
            }
        }
        (along_lon / along_lat) as f32
    }

    /// **`zonal` 让噪声沿经度拉长**：同一张画布上，沿经度的平均变化必须**明显小于**沿纬度的。
    ///
    /// ⚠ 判的是**方向**（比值），不是幅度 —— 幅度随频率/种子都在变，方向才是这一栏的语义。
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

    /// **默认不许动既有产物**：`zonal = 1.0` 时必须与"没有这一栏"逐点相同（这正是它默认值的意义）。
    #[test]
    fn zonal_one_is_the_old_behaviour_point_by_point() {
        let grid = Grid {
            width: 64,
            height: 384,
            projection: Projection::CubeMap,
        };
        let with_one = eval(
            &params::fbm::Params {
                zonal: 1.0,
                ..Default::default()
            },
            &[],
            grid,
        );
        // 手算一遍"没有这一栏"的那条路（与改动前那一行等价）。
        let settings = FbmSettings {
            frequency: params::fbm::Params::default().frequency,
            octaves: params::fbm::Params::default().octaves,
            lacunarity: params::fbm::Params::default().lacunarity,
            gain: params::fbm::Params::default().gain,
            seed: params::fbm::Params::default().seed,
        };
        let mut old = grid.filled(0.0);
        for y in 0..grid.height {
            for x in 0..grid.width {
                old.set(x, y, crate::noise::fbm_3(old.direction(x, y), &settings));
            }
        }
        let mut worst = 0.0_f32;
        for y in 0..grid.height {
            for x in 0..grid.width {
                worst = worst.max((with_one.at(x, y) - old.at(x, y)).abs());
            }
        }
        assert!(
            worst == 0.0,
            "`zonal = 1.0` 必须逐点等价于旧行为，最大偏差 {worst}"
        );
    }
}
