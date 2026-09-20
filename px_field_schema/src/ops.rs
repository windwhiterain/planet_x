//! 场算子的**声明**：身份 / 超参数 / 图参数形状 / 输出域。
//!
//! ⚠ **一行实现都没有**：算法在 `px_field_alg`（rlib）里、由 `px_field_op`（dylib）链进去，
//!   运行时按身份装载。这里只说「一个算子是什么、吃什么、吐什么」——于是图侧接错一个上游、
//!   少给一个字段，都是**编译错**，而且编译这一份不需要实现库在场。
//!
//! ⚠ 末尾那条 [`FieldRemap`] 是**泛型实例的声明**（`px_inst!` 复用它）：它甚至没有预置实现，
//!   "这一格的值怎么算"由 `art/inst/*.rs` 里那段图侧现写的函数决定。
//!
//! ⚠ 图参数的形状（`MixInput { a, b, mask }`）**是接口的一部分**，所以它住在声明旁边：
//!   它是"这个算子被接的那个 struct"，不是实现细节。

use px_graph_schema::{Cooked, px_op};

use crate::field::Field;
use crate::params;

/// 一张场（`Remap` / `Gradient` 吃它）。
#[derive(px_derive::PxInputs)]
pub struct FieldInput {
    pub field: Cooked<Field>,
}

/// 两张场（`Warp` 吃它：待扭曲的场 + 偏移场）。
#[derive(px_derive::PxInputs)]
pub struct FieldPairInput {
    pub field: Cooked<Field>,
    pub offset: Cooked<Field>,
}

/// 三张场（`Mix` 吃它：两张待混 + 一张权重）。
#[derive(px_derive::PxInputs)]
pub struct MixInput {
    pub a: Cooked<Field>,
    pub b: Cooked<Field>,
    pub mask: Cooked<Field>,
}

/// **泛型实例**的图参数：上游那一张场。
///
/// ⚠ 它**自己的输入类型**（`FieldRemapInput`，不是 `FieldInput`）：`px_inst!` 复用的那条声明
///   要求 `Inputs` 独立 —— 于是"这一档吃的是什么"不会跟着 `FieldInput` 的将来一起漂。
#[derive(px_derive::PxInputs)]
pub struct FieldRemapInput {
    pub input: Cooked<Field>,
}

// ⚠ 不吃上游的那四个 ⇒ 形状是 `()`（它没有名字问题，住在契约里）。
px_op! {
    /// 处处同一个值的场（当权重/常量用）。
    Constant, "field.constant", "px_field_op", params::constant::Params, (), Field
}

px_op! {
    /// 分形布朗噪声。
    Fbm, "field.fbm", "px_field_op", params::fbm::Params, (), Field
}

px_op! {
    /// 脊状噪声（山脊线是等值面）。
    Ridged, "field.ridged", "px_field_op", params::ridged::Params, (), Field
}

px_op! {
    /// 值域重映射（可平滑）。
    Remap, "field.remap", "px_field_op", params::remap::Params, FieldInput, Field
}

px_op! {
    /// 切向梯度的一个分量（法线/坡度用）。
    Gradient, "field.gradient", "px_field_op", params::gradient::Params, FieldInput, Field
}

px_op! {
    /// 按权重混两张场。
    Mix, "field.mix", "px_field_op", params::mix::Params, MixInput, Field
}

px_op! {
    /// 用偏移场扭曲采样方向（域扭曲）。
    Warp, "field.warp", "px_field_op", params::warp::Params, FieldPairInput, Field
}

px_op! {
    /// 上游场 + **图侧函数** ⇒ 新场（**泛型实例的声明**：`px_inst!` 复用它）。
    ///
    /// ⚠ 这一条**没有预置实现**：`"px_field_op"` 只是载荷签名里的域那一栏（`px_op!` 的形状
    ///   要求它），而 `px_field_op` 里**没有**这个算子的一行 `px_body!`。它存在的意义是给
    ///   图侧那些实例当**声明**用 —— 实例库由 `px build` 按 key 生成并编译，`LIB` 是空串
    ///   （`px_inst!` 给的），装载的是 `target/pcg/inst/<key>.dll`。
    ///
    /// ⚠ 参数是 [`params::RemapParams`] —— 与预置的 [`Remap`]（`params::remap::Params`）
    ///   **不是**同一个东西：见那一份的文档注释。
    FieldRemap, "field.remap", "px_field_op", params::RemapParams, FieldRemapInput, Field
}
