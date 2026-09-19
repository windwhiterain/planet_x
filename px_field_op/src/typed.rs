//! 场算子的**声明**：身份 + 超参数类型 + 图参数形状 + 输出域 + 怎么算。
//!
//! ⚠ 这里**一行实现都没有** —— `render` 转发到本 crate 里那个既有的 `eval`。
//! 一旦把实现搬进来，图程序就静态链住了算法体 ⇒ 改算法要重编图程序
//! ——「类型检查」与「改实现不重编」两条会互斥。
//!
//! ⚠ **图参数**（哪张图接谁）不在这里：那是**图**的知识。这里声明的只是"我这条形状"。

use px_cook::{px_op, Cooked, Unary1, Unary2, Unary3};
use px_graph_schema::OpKind;

/// 每个算子的身份清单：**它自己的实现文件 + 它依赖的共享件**。
///
/// ⚠ 它同时喂给图侧（`PxOp::source_hash()`）与 dylib 侧（描述符）——同一份，
///   才不会出现"改了算子体却命中旧产物"。数组长度写死（多一条编译期就报）。
macro_rules! sources {
    ($own:literal $(, $shared:literal)*) => {
        [include_str!(concat!("ops/", $own)),
         $(include_str!(concat!("../../", $shared))),*]
    };
}
use px_field_schema::field::Field;
use px_field_schema::params;

/// 无上游。
pub type None = ();

/// 一张场（具名字段：`Unary1 { a: clusters }`）。
pub type Field1 = Unary1<Cooked<Field>>;
pub type Field2 = Unary2<Cooked<Field>>;
pub type Field3 = Unary3<Cooked<Field>>;

macro_rules! field_op {
    (
        $(#[$meta:meta])*
        $name:ident, $id:expr, $version:literal, $params:ty, $inputs:ty, $arity:expr,
        $sources:expr, $expr:expr
    ) => {
        $(#[$meta])*
        pub struct $name;

        px_op! { $name = $id, $version, $params, $inputs, Field, OpKind::Field, $arity, $sources,
                 |p, i, g| $expr(p, i, g) }
    };
}

field_op! {
    /// 常量场（无输入）。
    Constant, params::CONSTANT, 1, params::constant::Params, None, &[],
    sources!("constant.rs", "px_field_schema/src/field.rs", "px_field_schema/src/params.rs"),
    |p: &params::constant::Params, _i: &None, g| crate::ops::constant::eval(p, &[], g)
}

field_op! {
    /// 分形布朗噪声（无输入）。
    Fbm, params::FBM, 4, params::fbm::Params, None, &[],
    sources!("fbm.rs", "px_field_op/src/noise.rs", "px_field_schema/src/field.rs", "px_field_schema/src/noise.rs", "px_field_schema/src/params.rs"),
    |p: &params::fbm::Params, _i: &None, g| crate::ops::fbm::eval(p, &[], g)
}

field_op! {
    /// 脊状噪声（无输入）。
    Ridged, params::RIDGED, 4, params::ridged::Params, None, &[],
    sources!("ridged.rs", "px_field_op/src/noise.rs", "px_field_schema/src/field.rs", "px_field_schema/src/noise.rs", "px_field_schema/src/params.rs"),
    |p: &params::ridged::Params, _i: &None, g| crate::ops::ridged::eval(p, &[], g)
}

field_op! {
    /// 值域重映射（一张场）。
    Remap, params::REMAP, 1, params::remap::Params, Field1, &["field"],
    sources!("remap.rs", "px_field_schema/src/field.rs", "px_field_schema/src/params.rs"),
    |p: &params::remap::Params, i: &Field1, g| crate::ops::remap::eval(p, &[i.a.sample()], g)
}

field_op! {
    /// 切向梯度的一个分量（一张场）。
    Gradient, params::GRADIENT, 1, params::gradient::Params, Field1, &["field"],
    sources!("gradient.rs", "px_field_schema/src/field.rs", "px_field_schema/src/params.rs"),
    |p: &params::gradient::Params, i: &Field1, g| crate::ops::gradient::eval(p, &[i.a.sample()], g)
}

field_op! {
    /// 三张场按第三张当权重混合。
    Mix, params::MIX, 1, params::mix::Params, Field3, &["a", "b", "mask"],
    sources!("mix.rs", "px_field_schema/src/field.rs", "px_field_schema/src/params.rs"),
    |p: &params::mix::Params, i: &Field3, g| {
        crate::ops::mix::eval(p, &[i.a.sample(), i.b.sample(), i.c.sample()], g)
    }
}

field_op! {
    /// 域扭曲（两张场）。
    Warp, params::WARP, 3, params::warp::Params, Field2, &["field", "offset"],
    sources!("warp.rs", "px_field_schema/src/field.rs", "px_field_schema/src/params.rs", "px_verify/src/noise.rs"),
    |p: &params::warp::Params, i: &Field2, g| {
        crate::ops::warp::eval(p, &[i.a.sample(), i.b.sample()], g)
    }
}
