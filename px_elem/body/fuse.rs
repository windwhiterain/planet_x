// `field.fuse` 的**算法本体**：整条链在一个格子上算完。
//
// ⚠ 这一份会被生成的那一份实例库**原样 `include!`**：`use` 一律写全路径。
// ⚠ 注释一律 `//`（**不是 `//!`**）：`include!` 排在生成物第一段之后。
// ⚠ 逐格那条循环**不在这里**：它在 `px_elem::fill`（融合省下的是**中间那张场**，不是那条循环）。

/// `remap`（钳 → 归一化 → 映值域 → `gamma`）之后接着 `mix`（按 `mask + bias` 与 `b` 插值）。
///
/// ⚠ 两半的参数语义与各自单独那两条**逐字相同**（同一把 `px_field_alg::Scale`、同一个 `gamma`
///   口径、同一个 `clamp`）—— 于是"融合"与"两个节点接起来"算出**同一批数**，
///   而融合省下中间那张场（`a` 只读一次、`b`/`mask` 各读一次）。
pub fn value(
    params: &px_elem::FuseParams,
    inputs: &px_elem::FuseInput,
    x: u32,
    y: u32,
    _uv: [f32; 2],
    _direction: [f32; 3],
) -> f32 {
    let scale = px_field_alg::Scale {
        in_min: params.in_min,
        in_max: params.in_max,
        out_min: params.out_min,
        out_max: params.out_max,
        smooth: params.smooth,
    };
    let mapped = scale.map(scale.normalize(inputs.a.value().at(x, y)));
    // ⚠ 与 `field.remap` 同一个口径：非正数不弯。
    let mapped = if params.gamma > 0.0 && params.gamma != 1.0 {
        mapped.powf(params.gamma)
    } else {
        mapped
    };
    let weight = (inputs.mask.value().at(x, y) + params.bias).clamp(0.0, 1.0);
    mapped * (1.0 - weight) + inputs.b.value().at(x, y) * weight
}
