use px_field_schema::field::{Field, GridField};
use px_field_schema::params;
use px_graph_schema::Grid;

pub fn eval(params: &params::remap::Params, inputs: &[&Field], grid: Grid) -> Field {
    let input = inputs[0];
    let span = params.in_max - params.in_min;
    let inv_span = if span.abs() < f32::EPSILON {
        0.0
    } else {
        1.0 / span
    };
    let mut field = grid.filled(0.0);
    for y in 0..grid.height {
        for x in 0..grid.width {
            let mut t = ((input.at(x, y) - params.in_min) * inv_span).clamp(0.0, 1.0);
            if params.smooth {
                t = t * t * (3.0 - 2.0 * t);
            }
            field.set(x, y, params.out_min + t * (params.out_max - params.out_min));
        }
    }
    field
}
