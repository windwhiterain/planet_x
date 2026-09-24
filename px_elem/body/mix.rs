// `field.mix` 的**算法本体**：这一格算什么。
//
// ⚠ 这一份会被生成的那一份实例库**原样 `include!`**：`use` 一律写全路径
//   （生成物里没有 `crate::` 那个前缀可指）。
// ⚠ 注释一律 `//`（**不是 `//!`**）：`include!` 排在生成物第一段之后。
// ⚠ 逐格那条循环**不在这里**：它在 `px_elem::fill`（element 算子唯一那条循环）。

/// 按 `mask + bias` 在两份场之间插值：`a·(1-w) + b·w`（`w` 钳到 `[0,1]`）。
///
/// ⚠ 上游是**算子在调这一行之前**取出来的那三张场（`i.a` / `i.b` / `i.mask`）——
///   这里只管"这一格的值怎么算"，不碰形状（形状由 `ElementFn::shape` 从上游推）。
pub fn value(
    params: &px_elem::MixParams,
    inputs: &px_elem::MixInput,
    x: u32,
    y: u32,
    _uv: [f32; 2],
    _direction: [f32; 3],
) -> f32 {
    let (a, b, mask) = (inputs.a.value(), inputs.b.value(), inputs.mask.value());
    let weight = (mask.at(x, y) + params.bias).clamp(0.0, 1.0);
    a.at(x, y) * (1.0 - weight) + b.at(x, y) * weight
}

