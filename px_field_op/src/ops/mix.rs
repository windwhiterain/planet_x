use px_field_schema::field::Field;
use px_field_schema::ops::Mix;
use px_field_schema::params;

px_graph_schema::px_body! {
    Mix,
    |p, i| crate::ops::mix::eval(p, &[i.a.value(), i.b.value(), i.mask.value()])
}

pub fn eval(params: &params::mix::Params, inputs: &[&Field]) -> Field {
    let (a, b, mask) = (inputs[0], inputs[1], inputs[2]);
    let mut field = a.like(0.0);
    for y in 0..field.height {
        for x in 0..field.width {
            let weight = (mask.at(x, y) + params.bias).clamp(0.0, 1.0);
            let value = a.at(x, y) * (1.0 - weight) + b.at(x, y) * weight;
            field.set(x, y, value);
        }
    }
    field
}
