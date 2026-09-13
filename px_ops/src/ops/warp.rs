use serde::{Deserialize, Serialize};

use crate::field::{Field, normalize, tangent_frame};
use crate::noise::fnv1a;
use crate::{FieldOp, Grid};

pub struct Warp;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    pub strength: f32,
    pub lateral: f32,
    pub probe: f32,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            strength: 0.40,
            lateral: 0.50,
            probe: 0.07,
        }
    }
}

impl FieldOp for Warp {
    type Params = Params;
    const ID: &'static str = "field.warp";
    const VERSION: u32 = 3;
    const SOURCE_HASH: u64 = fnv1a(include_str!("warp.rs"));
    const INPUTS: &'static [&'static str] = &["input", "warp"];

    fn eval(params: &Params, inputs: &[&Field], grid: Grid) -> Field {
        let (input, warp) = (inputs[0], inputs[1]);
        let mut field = grid.filled(0.0);

        for y in 0..grid.height {
            for x in 0..grid.width {
                let direction = grid.direction(x, y);
                let (east, north) = tangent_frame(direction);
                let first = warp.sample_direction(direction) - 0.5;
                let probed = normalize([
                    direction[0] + east[0] * params.probe,
                    direction[1] + east[1] * params.probe,
                    direction[2] + east[2] * params.probe,
                ]);
                let second = warp.sample_direction(probed) - 0.5;
                let along = first * params.strength;
                let across = second * params.lateral * params.strength;
                let displaced = normalize([
                    direction[0] + east[0] * along + north[0] * across,
                    direction[1] + east[1] * along + north[1] * across,
                    direction[2] + east[2] * along + north[2] * across,
                ]);

                field.set(x, y, input.sample_direction(displaced));
            }
        }
        field
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::Projection;

    #[test]
    fn a_constant_warp_field_leaves_the_input_alone_in_every_projection() {
        let params = Params {
            strength: 0.2,
            lateral: 0.5,
            probe: 0.07,
        };

        for projection in [
            Projection::Equirect,
            Projection::Octahedral,
            Projection::Cube,
            Projection::CubeMap,
        ] {
            let (width, height) = match projection {
                Projection::Equirect => (96, 48),
                Projection::Octahedral => (64, 64),
                Projection::Cube => (78, 52),
                Projection::CubeMap => (32, 192),
            };
            let grid = Grid {
                width,
                height,
                projection,
            };
            let mut input = Field::filled_with(width, height, 0.0, projection);
            for y in 0..height {
                for x in 0..width {
                    input.set(x, y, input.direction(x, y)[1] * 0.5 + 0.5);
                }
            }
            let flat = Field::filled_with(width, height, 0.5, projection);
            let warped = Warp::eval(&params, &[&input, &flat], grid);

            let mut worst = 0.0_f32;
            for y in 0..height {
                for x in 0..width {
                    worst = worst.max((warped.at(x, y) - input.at(x, y)).abs());
                }
            }
            assert!(
                worst < 0.02,
                "{projection:?} 下常量扭曲场不该移动纹素，最大偏差 {worst}"
            );
        }
    }
}
