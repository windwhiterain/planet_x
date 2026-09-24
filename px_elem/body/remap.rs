// `field.remap` 的**算法本体**：这一格算什么。
//
// ⚠ 这一份会被生成的那一份实例库**原样 `include!`**：`use` 一律写全路径。
// ⚠ 注释一律 `//`（**不是 `//!`**）：`include!` 排在生成物第一段之后。
// ⚠ 逐格那条循环**不在这里**：它在 `px_elem::fill`。这里用的 `Scale` 是共享的那一把尺子
//   （与从前 `field.remap` 走的是**同一份** `px_field_alg::Scale` ⇒ 同一份参数不会算出两种结果）。

/// 钳到 `[in_min, in_max]` → 归一化（可选平滑）→ 映到 `[out_min, out_max]` → 按 `gamma` 弯。
pub fn value(
    params: &px_elem::RemapParams,
    inputs: &px_elem::RemapInput,
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
    let mapped = scale.map(scale.normalize(inputs.field.value().at(x, y)));
    // ⚠ 非正数不弯（`NaN` 顺着管线传下去极难归因）。
    if params.gamma > 0.0 && params.gamma != 1.0 {
        mapped.powf(params.gamma)
    } else {
        mapped
    }
}

