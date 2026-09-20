use px_field_schema::field::{Field, GridField, normalize, tangent_frame};
use px_field_schema::ops::Gradient;
use px_field_schema::params;
use px_graph_schema::Grid;

px_graph_schema::px_body! { Gradient, |p, i, g| crate::ops::gradient::eval(p, &[i.field.value()], g) }

pub fn eval(params: &params::gradient::Params, inputs: &[&Field], grid: Grid) -> Field {
    let input = inputs[0];
    let epsilon = if params.epsilon > 0.0 {
        params.epsilon
    } else {
        let across = match grid.projection {
            px_field_schema::field::Projection::CubeMap => grid.width,
            _ => grid.width.max(1) / 4,
        };
        std::f32::consts::FRAC_PI_2 / across.max(1) as f32
    };
    let component = (params.component as usize).min(2);
    let mut field = grid.filled(0.0);

    for y in 0..grid.height {
        for x in 0..grid.width {
            let direction = grid.direction(x, y);
            let (east, north) = tangent_frame(direction);
            let slope_east =
                (input.sample_direction(step(direction, east, epsilon))
                    - input.sample_direction(step(direction, east, -epsilon)))
                    / (2.0 * epsilon);
            let slope_north =
                (input.sample_direction(step(direction, north, epsilon))
                    - input.sample_direction(step(direction, north, -epsilon)))
                    / (2.0 * epsilon);
            let tangent = [
                slope_east * east[0] + slope_north * north[0],
                slope_east * east[1] + slope_north * north[1],
                slope_east * east[2] + slope_north * north[2],
            ];
            field.set(x, y, tangent[component]);
        }
    }
    field
}

fn step(direction: [f32; 3], along: [f32; 3], distance: f32) -> [f32; 3] {
    normalize([
        direction[0] + along[0] * distance,
        direction[1] + along[1] * distance,
        direction[2] + along[2] * distance,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use px_field_schema::field::Projection;

    const FACE: u32 = 64;

    fn cube_grid() -> Grid {
        Grid {
            width: FACE,
            height: FACE * 6,
            projection: Projection::CubeMap,
        }
    }

    fn bake(grid: Grid, field: &Field, component: u32) -> Field {
        let params = params::gradient::Params {
            component,
            epsilon: 0.0,
        };
        eval(&params, &[field], grid)
    }

    #[test]
    fn a_linear_field_gives_its_tangential_projection_on_a_cube_map() {
        let grid = cube_grid();
        for (component, axis) in [
            (0_u32, [1.0_f32, 0.0, 0.0]),
            (1, [0.0, 1.0, 0.0]),
            (2, [0.0, 0.0, 1.0]),
        ] {
            let mut input = grid.filled(0.0);
            for y in 0..grid.height {
                for x in 0..grid.width {
                    let direction = grid.direction(x, y);
                    input.set(
                        x,
                        y,
                        direction[0] * axis[0] + direction[1] * axis[1] + direction[2] * axis[2],
                    );
                }
            }

            let baked = bake(grid, &input, component);
            let mut worst = 0.0_f32;
            for y in 0..grid.height {
                for x in 0..grid.width {
                    let direction = grid.direction(x, y);
                    let along = axis[0] * direction[0]
                        + axis[1] * direction[1]
                        + axis[2] * direction[2];
                    let wanted = axis[component as usize] - along * direction[component as usize];
                    worst = worst.max((baked.at(x, y) - wanted).abs());
                }
            }
            assert!(
                worst < 0.06,
                "线性场在第 {component} 个分量上的切向梯度应当是它的切向投影，最大偏差 {worst}（一个纹素步长的中心差分，实测精度约 4%）"
            );
        }
    }

    #[test]
    fn a_flat_field_has_no_gradient_anywhere() {
        let grid = cube_grid();
        let input = grid.filled(0.37);
        for component in 0..3 {
            let baked = bake(grid, &input, component);
            let mut worst = 0.0_f32;
            for y in 0..grid.height {
                for x in 0..grid.width {
                    worst = worst.max(baked.at(x, y).abs());
                }
            }
            assert!(worst < 1e-4, "常数场的梯度应当是 0，实测 {worst}");
        }
    }

    #[test]
    fn the_gradient_does_not_jump_across_a_face_edge() {
        let grid = cube_grid();
        let mut input = grid.filled(0.0);
        for y in 0..grid.height {
            for x in 0..grid.width {
                let direction = grid.direction(x, y);
                let bump = direction[0] * 0.5 + direction[1] * 0.3 + direction[2] * 0.2;
                input.set(x, y, bump * bump);
            }
        }

        let baked: Vec<Field> = (0..3).map(|component| bake(grid, &input, component)).collect();
        let mut previous: Option<[f32; 3]> = None;
        let mut worst = 0.0_f32;
        let steps = 400;
        for index in 0..=steps {
            let angle = -0.6 + 1.2 * index as f32 / steps as f32;
            let direction = normalize([angle, 0.25, 1.0]);
            let sample = [
                baked[0].sample_direction(direction),
                baked[1].sample_direction(direction),
                baked[2].sample_direction(direction),
            ];
            if let Some(last) = previous {
                let jump = ((sample[0] - last[0]).powi(2)
                    + (sample[1] - last[1]).powi(2)
                    + (sample[2] - last[2]).powi(2))
                .sqrt();
                worst = worst.max(jump);
            }
            previous = Some(sample);
        }
        assert!(
            worst < 0.08,
            "跨 +Z/+X 棱走一遍，梯度不该跳变，单步最大 {worst}"
        );
    }
}
