use px_field_schema::field::{Field, Projection};
use px_field_schema::params::RemapParams;

use crate::field_fn::FieldFn;

pub trait Cell {
    fn value(
        &self,
        params: &RemapParams,
        normalized_upstream: f32,
        uv: [f32; 2],
        direction: [f32; 3],
    ) -> f32;
}

impl<F: FieldFn> Cell for F {
    fn value(
        &self,
        params: &RemapParams,
        normalized_upstream: f32,
        uv: [f32; 2],
        direction: [f32; 3],
    ) -> f32 {
        FieldFn::value(self, params, normalized_upstream, uv, direction)
    }
}

pub fn identity() -> Scale {
    Scale {
        in_min: 0.0,
        in_max: 1.0,
        out_min: 0.0,
        out_max: 1.0,
        smooth: true,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scale {
    pub in_min: f32,
    pub in_max: f32,
    pub out_min: f32,
    pub out_max: f32,
    pub smooth: bool,
}

impl Scale {
    pub fn normalize(&self, value: f32) -> f32 {
        let span = self.in_max - self.in_min;
        let inv_span = if span.abs() < f32::EPSILON {
            0.0
        } else {
            1.0 / span
        };
        let mut t = ((value - self.in_min) * inv_span).clamp(0.0, 1.0);
        if self.smooth {
            t = t * t * (3.0 - 2.0 * t);
        }
        t
    }

    pub fn map(&self, t: f32) -> f32 {
        self.out_min + t * (self.out_max - self.out_min)
    }
}

pub fn map_grid<C: Cell>(
    scale: &Scale,
    params: &RemapParams,
    gamma: f32,
    upstream: &Field,
    cell: &C,
) -> Field {
    let mut field = upstream.like(0.0);
    let spherical = upstream.projection != Projection::Volume;
    for y in 0..field.height {
        for x in 0..field.width {
            let t = scale.normalize(upstream.at(x, y));
            let (u, v) = if spherical {
                upstream.uv(x, y)
            } else {
                (0.0, 0.0)
            };
            let direction = if spherical {
                upstream.direction(x, y)
            } else {
                [0.0, 1.0, 0.0]
            };
            field.set(
                x,
                y,
                px_field_schema::params::bend(
                    scale.map(cell.value(params, t, [u, v], direction)),
                    gamma,
                ),
            );
        }
    }
    field
}

#[allow(clippy::too_many_arguments)]
pub fn remap_with<F: Cell>(
    scale: &Scale,
    params: &RemapParams,
    gamma: f32,
    upstream: &Field,
    field_fn: &F,
) -> Field {
    map_grid(scale, params, gamma, upstream, field_fn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field_fn::Sampled;
    use px_field_schema::field::Projection;

    fn sample_field() -> Field {
        Field::filled_with(37, 23, 0.0, Projection::Equirect)
    }

    #[test]
    fn gamma_pushes_the_middle_down_and_keeps_the_peaks() {
        let mut input = Field::filled_with(6, 6, 0.0, Projection::Equirect);
        for x in 0..6 {
            for y in 0..6 {
                input.set(x, y, (x as f32 + y as f32) / 10.0);
            }
        }
        let scale = Scale {
            in_min: 0.0,
            in_max: 1.0,
            out_min: 0.0,
            out_max: 1.0,
            smooth: false,
        };
        let remap = |input: &Field, gamma: f32| {
            remap_with(
                &scale,
                &RemapParams::default(),
                gamma,
                input,
                &Sampled { field: input },
            )
        };
        let plain = remap(&input, 1.0);
        let bent = remap(&input, 3.0);
        let mut worst_low = f32::INFINITY;
        let mut worst_high = 0.0_f32;
        for index in 0..plain.data.len() {
            let (p, b) = (plain.data[index], bent.data[index]);
            assert!(b <= p + 1e-6, "弯折不该把值抬高：{p} → {b}");
            if p > 0.02 && p < 0.5 {
                worst_low = worst_low.min(b / p);
            }
            if p > 0.55 {
                worst_high = worst_high.max(b / p);
            }
        }
        assert!(
            worst_high > worst_low + 0.2,
            "低值该掉得更多：低值最大比值 {worst_low:.3}，高值最大比值 {worst_high:.3}"
        );
    }

    fn input(field: &Field) -> Field {
        let mut field = field.like(0.0);
        for y in 0..field.height {
            for x in 0..field.width {
                let (u, v) = field.uv(x, y);
                field.set(x, y, (u * 3.0 - v * 1.5).sin() * 0.5 + 0.5);
            }
        }
        field
    }

    #[test]
    fn the_shared_path_moves_the_input_without_touching_it() {
        let grid = sample_field();
        let params = RemapParams::default();
        let input = input(&grid);

        let plain = Scale {
            in_min: 0.0,
            in_max: 1.0,
            out_min: 0.0,
            out_max: 1.0,
            smooth: false,
        };
        let copied = remap_with(&plain, &params, 1.0, &input, &Sampled { field: &input });
        assert_eq!(
            copied, input,
            "恒等尺子（不平滑）+ 照抄上游，却没算出上游那张场"
        );

        let smooth = identity();
        let bent = remap_with(&smooth, &params, 1.0, &input, &Sampled { field: &input });
        let mut field = input.like(0.0);
        for y in 0..field.height {
            for x in 0..field.width {
                field.set(x, y, smooth.map(smooth.normalize(input.at(x, y))));
            }
        }
        assert_eq!(
            bent, field,
            "平滑那一档与 `Scale::map ∘ Scale::normalize` 那条闭式不是逐位相同"
        );
        assert_ne!(
            bent, input,
            "`identity()` 是 `smooth = true` 的尺子 ⇒ 输出不该与上游逐位相同（尺子没生效？）"
        );
    }

    #[test]
    fn the_field_function_sees_the_normalized_upstream_and_the_texel_center() {
        let grid = sample_field();
        let scale = identity();
        let params = RemapParams::default();
        let input = input(&grid);

        let seen = std::cell::RefCell::new(Vec::new());
        struct Record<'a> {
            seen: &'a std::cell::RefCell<Vec<(f32, [f32; 2], [f32; 3])>>,
        }
        impl FieldFn for Record<'_> {
            fn value(
                &self,
                _params: &RemapParams,
                upstream: f32,
                uv: [f32; 2],
                direction: [f32; 3],
            ) -> f32 {
                self.seen.borrow_mut().push((upstream, uv, direction));
                upstream
            }
        }
        let _ = remap_with(&scale, &params, 1.0, &input, &Record { seen: &seen });

        let seen = seen.into_inner();
        assert_eq!(seen.len(), (grid.width * grid.height) as usize);
        let sample = seen[(2 * grid.width + 3) as usize];
        let (u, v) = input.uv(3, 2);
        assert_eq!(sample.1, [u, v], "坐标不是纹素中心口径");
        assert_eq!(
            sample.0,
            scale.normalize(input.at(3, 2)),
            "场函数拿到的值不是'上游过同一把尺子'之后的"
        );
        assert_eq!(
            sample.2,
            input.direction(3, 2),
            "场函数拿到的方向不是这一格的球面方向（纬向条带这类函数只能拿它算）"
        );
    }

    #[test]
    fn the_node_params_reach_the_field_function() {
        let grid = sample_field();
        let scale = identity();
        let input = input(&grid);
        let params = RemapParams {
            gain: 0.0,
            bias: 0.25,
            bands: 5.0,
        };

        struct ReadsParams;
        impl FieldFn for ReadsParams {
            fn value(
                &self,
                params: &RemapParams,
                _upstream: f32,
                _uv: [f32; 2],
                _direction: [f32; 3],
            ) -> f32 {
                params.bias
            }
        }

        let field = remap_with(&scale, &params, 1.0, &input, &ReadsParams);
        assert!(
            field.data.iter().all(|value| (value - 0.25).abs() < 1e-9),
            "节点参数没到图侧函数（输出不是 `bias`）"
        );
        let other = RemapParams {
            bias: 0.75,
            ..params
        };
        let field = remap_with(&scale, &other, 1.0, &input, &ReadsParams);
        assert!(
            field.data.iter().all(|value| (value - 0.75).abs() < 1e-9),
            "换了参数内容却没换"
        );
    }

    #[test]
    fn the_scale_keeps_the_pre_move_arithmetic() {
        let plain = Scale {
            in_min: 0.0,
            in_max: 1.0,
            out_min: 0.0,
            out_max: 1.0,
            smooth: false,
        };
        assert_eq!(plain.normalize(-3.0), 0.0);
        assert_eq!(plain.normalize(0.25), 0.25);
        assert_eq!(plain.normalize(3.0), 1.0);
        assert_eq!(plain.map(0.25), 0.25);

        let degenerate = Scale {
            in_min: 0.5,
            in_max: 0.5,
            ..plain
        };
        assert_eq!(degenerate.normalize(0.5), 0.0);

        let smooth = Scale {
            smooth: true,
            ..plain
        };
        assert_eq!(smooth.normalize(0.5), 0.5);
        assert!((smooth.normalize(0.25) - 0.15625).abs() < 1e-6);

        let wide = Scale {
            out_min: -1.0,
            out_max: 2.0,
            ..plain
        };
        assert_eq!(wide.map(0.0), -1.0);
        assert_eq!(wide.map(1.0), 2.0);
        assert_eq!(wide.map(0.5), 0.5);
    }
}
