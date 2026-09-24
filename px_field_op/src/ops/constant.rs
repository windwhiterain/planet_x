use px_field_schema::field::Field;
use px_field_schema::ops::Constant;
use px_field_schema::params;

px_graph_schema::px_body! { Constant, |p, _i| crate::ops::constant::eval(p, &[]) }

/// **形状从参数来**：生成类算子没有上游 ⇒ 尺寸只有这一个来源。
pub fn eval(params: &params::constant::Params, _inputs: &[&Field]) -> Field {
    params.shape.filled(params.value)
}
