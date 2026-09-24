// `field.constant` 的**算法本体**：这一格算什么。
//
// ⚠ 这一份会被生成的那一份实例库**原样 `include!`**（`px build`）：`use` 一律写全路径
//   （生成物里没有 `crate::` 那个前缀可指），不放 `#[cfg(test)]`、不引本 crate 的私有名字。
// ⚠ 它**不在 `px_elem/src/` 下** ⇒ 不进那个 crate 的源码指纹：改这一行只换**这一条**实例的键。
// ⚠ 注释一律 `//`（**不是 `//!`**）：`include!` 在生成物里排在"复用什么"那两行之后
//   ⇒ 内层文档注释在那儿报 `error[E0753]: expected outer doc comment`（实测）。
//   本仓同一个坑在 `px_graphs/src/inst_recipe.rs` 头上已经记过一次（那边也是被 `include!` 的）。

/// 铺一个常数：不看坐标、不看方向、没有上游。
pub fn value(
    params: &px_elem::ConstantParams,
    _inputs: &(),
    _uv: [f32; 2],
    _direction: [f32; 3],
) -> f32 {
    params.value
}
