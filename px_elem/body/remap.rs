// `field.remap` 的**算法本体**：这一格算什么。
//
// ⚠ 这一份会被生成的那一份实例库**原样 `include!`**：`use` 一律写全路径。
// ⚠ 注释一律 `//`（**不是 `//!`**）：`include!` 排在生成物第一段之后。
// ⚠ 逐格那条循环**不在这里**：它在 `px_elem::fill`。这里用的 `Scale` 是共享的那一把尺子
//   （与前作预置档 `field.remap` 走的是**同一份** `px_field_alg::Scale` ⇒ 同一份参数不会算出
//   两种结果），`gamma` 那一步走的也是**共享的那一个** `px_field_schema::params::bend`。

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
    // ⚠ `gamma` 那一步走**共享的那一个函数**（`px_field_schema::params::bend`）：
    //   "非正数不弯 + 负底数回 0 + `gamma ≈ 1` 当恒等"这三条口径只写在一处。
    //   ⚠ 自己写 `if gamma > 0.0 && gamma != 1.0 { powf }` 是错的：那个判断只管**指数**、
    //   不管**底数**，于是 `out_min < 0` 而 `gamma > 1` 时 `(-1.0).powf(2.5) = NaN`
    //   顺着管线传下去（值域变成 `inf..-inf`、均值 `NaN`）。`bend` 就是为这一条存在的。
    px_field_schema::params::bend(mapped, params.gamma)
}

