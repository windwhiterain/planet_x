use px_field_schema::field::{Field, GridField};
use px_field_schema::params;
use px_graph_schema::Grid;

pub fn eval(params: &params::constant::Params, _inputs: &[&Field], grid: Grid) -> Field {
    grid.filled(params.value)
}
