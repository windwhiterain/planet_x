use px_field_schema::field::{Field, GridField};
use px_field_schema::ops::Mix;
use px_field_schema::params;
use px_graph_schema::Grid;

px_graph_schema::px_body! {
    Mix,
    |p, i, g| crate::ops::mix::eval(p, &[i.a.value(), i.b.value(), i.mask.value()], g)
}

pub fn eval(params: &params::mix::Params, inputs: &[&Field], grid: Grid) -> Field {
    let (a, b, mask) = (inputs[0], inputs[1], inputs[2]);
    let mut field = grid.filled(0.0);
    for y in 0..grid.height {
        for x in 0..grid.width {
            let weight = (mask.at(x, y) + params.bias).clamp(0.0, 1.0);
            let value = a.at(x, y) * (1.0 - weight) + b.at(x, y) * weight;
            field.set(x, y, value);
        }
    }
    field
}
