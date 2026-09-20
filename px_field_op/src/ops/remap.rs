use px_field_alg::{Scale, remap_sampled};
use px_field_schema::field::Field;
use px_field_schema::ops::Remap;
use px_field_schema::params;
use px_graph_schema::Grid;

px_graph_schema::px_body! { Remap, |p, i, g| crate::ops::remap::eval(p, &[i.field.value()], g) }

/// **薄壳**：算法（格点遍历 + 值域映射/钳制）住在 `px_field_alg` 里。
///
/// ⚠ 这一条调的是 `px_field_alg::remap_sampled`，而图侧现写的那一档调的是同一个 crate 的
///   `remap_with` —— 两者走**同一条** `map_grid` ⇒ 同一份参数不会算出两种结果
///   （体积域 `eval_sampled` / `coarse_with` 是同一条规矩）。
pub fn eval(params: &params::remap::Params, inputs: &[&Field], grid: Grid) -> Field {
    // ⚠ **这一条不再自己写循环**：搬出去之前那份循环（钳到 `[0,1]` → 平滑 → 映到
    //   `[out_min, out_max]`）就是 `px_field_alg::Scale`，逐字照抄。
    let scale = Scale {
        in_min: params.in_min,
        in_max: params.in_max,
        out_min: params.out_min,
        out_max: params.out_max,
        smooth: params.smooth,
    };
    remap_sampled(&scale, params.gamma, inputs[0], grid)
}
