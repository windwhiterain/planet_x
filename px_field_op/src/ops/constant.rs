use px_field_schema::field::{Field, GridField};
use px_field_schema::ops::Constant;
use px_field_schema::params;
use px_graph_schema::Grid;

px_graph_schema::px_body! { Constant, |p, _i, g| crate::ops::constant::eval(p, &[], g) }

pub fn eval(params: &params::constant::Params, _inputs: &[&Field], grid: Grid) -> Field {
    grid.filled(params.value)
}
