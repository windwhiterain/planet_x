//! 场算子的**声明**：身份 / 超参数 / 图参数形状 / 输出域 / 怎么算 / 源码清单。
//!
//! ⚠ **图参数的形状是算子接口的一部分**，所以住在这里 —— 字段名就是这个算子吃的东西的名字
//! （`WarpInput { field, offset }` 而不是 `Two { a, b }`）。
//! 于是图侧写错一个字段、少给一个上游，都是**编译错**。
//!
//! ⚠ 这里**一行实现都没有** —— `render` 转发到本 crate 里那个既有的 `eval`。

use px_cook::{Cooked, px_op};
use px_field_schema::field::Field;
use px_field_schema::params;
use px_graph_schema::OpKind;

// ── 图参数的形状：一个算子一个 ────────────────────────────────────────────────
//
// 两条 impl 是同一件事的两面：`collect` 把上游的**键**折进自己的键，
// `from_payloads` 把上游的**字节**解成这个 struct。

/// 一张场（`Remap` / `Gradient` 吃它）。
#[derive(px_derive::PxInputs)]
pub struct FieldInput {
    pub field: Cooked<Field>,
}

/// 两张场（`Warp` 吃它）。
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

macro_rules! field_op {
    (
        $(#[$meta:meta])*
        $name:ident, $id:expr, $params:ty, $inputs:ty, $arity:expr, $expr:expr
    ) => {
        $(#[$meta])*
        pub struct $name;

        px_op! { $name = $id, $params, $inputs, Field, OpKind::Field, $arity,
                 |p, i, g| $expr(p, i, g) }
    };
}

// ⚠ 这四条不吃上游 ⇒ 形状是 `()`（它没有名字问题，留在 `px_cook` 的契约里）。

field_op! {
    /// 常量场（无输入）。
    Constant, params::CONSTANT, params::constant::Params, (), &[],
    |p: &params::constant::Params, _i: &(), g| crate::ops::constant::eval(p, &[], g)
}

field_op! {
    /// 分形布朗噪声（无输入）。
    Fbm, params::FBM, params::fbm::Params, (), &[],
    |p: &params::fbm::Params, _i: &(), g| crate::ops::fbm::eval(p, &[], g)
}

field_op! {
    /// 脊状噪声（无输入）。
    Ridged, params::RIDGED, params::ridged::Params, (), &[],
    |p: &params::ridged::Params, _i: &(), g| crate::ops::ridged::eval(p, &[], g)
}

field_op! {
    /// 值域重映射（一张场）。
    Remap, params::REMAP, params::remap::Params, FieldInput, &["field"],
    |p: &params::remap::Params, i: &FieldInput, g| {
        crate::ops::remap::eval(p, &[i.field.sample()], g)
    }
}

field_op! {
    /// 切向梯度的一个分量（一张场）。
    Gradient, params::GRADIENT, params::gradient::Params, FieldInput, &["field"],
    |p: &params::gradient::Params, i: &FieldInput, g| {
        crate::ops::gradient::eval(p, &[i.field.sample()], g)
    }
}

field_op! {
    /// 三张场按第三张当权重混合。
    Mix, params::MIX, params::mix::Params, MixInput, &["a", "b", "mask"],
    |p: &params::mix::Params, i: &MixInput, g| {
        crate::ops::mix::eval(p, &[i.a.sample(), i.b.sample(), i.mask.sample()], g)
    }
}

field_op! {
    /// 域扭曲（两张场：待扭曲的场 + 偏移场）。
    Warp, params::WARP, params::warp::Params, FieldPairInput, &["field", "offset"],
    |p: &params::warp::Params, i: &FieldPairInput, g| {
        crate::ops::warp::eval(p, &[i.field.sample(), i.offset.sample()], g)
    }
}
