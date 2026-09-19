//! 场算子的**类型化契约**：图脚本静态链接这一份（**不是** `px_field_op` 的 dylib）。
//!
//! 每个算子的类型化入口都**只是接线**：身份 + 参数类型 + 输入类型 + 转发到
//! 本 crate 里那个已经存在的 `eval`。于是图脚本拿到的类型检查与老路径走的是
//! **同一份实现**，不会出现「两份算法漂移」—— 漂移正是 §28.2 那条门在防的病。
//!
//! ⚠ 参数文件的**位置**（`art/<图>/<名>.toml`）是**图**的知识，不是算子的：
//! 同一个 `field.fbm` 在 `clouds` 里叫 `clusters`、在 `planet` 里叫 `continents`。
//! 所以 `cook_field` 多收一个「参数文件名」参数，算子只管自己那份 `Params` 类型。

use px_cook::{Cooked, Identity, Op};
use px_field_schema::field::Field;
use px_field_schema::params;
use px_graph_schema::{Grid, fnv1a_sources};

/// 算子的身份：`id` + `version` + 覆盖共享依赖的源码哈希（§28.2）。
macro_rules! typed_identity {
    ($id:expr, $version:expr, [$($source:literal),+ $(,)?]) => {
        Identity {
            id: $id,
            version: $version,
            source_hash: fnv1a_sources(&[$($source),+]),
        }
    };
}

/// 「吃 N 张场、吐一张场」的算子骨架：身份 / 输入类型 / 转发全在这里，
/// 每条只剩「参数类型」与「eval 怎么调」两个自由变量。
macro_rules! field_op {
    (
        $(#[$meta:meta])*
        name = $name:ident,
        id = $id:expr,
        version = $version:literal,
        params = $params:ty,
        inputs = $inputs:ty,
        sources = [$($source:literal),+ $(,)?],
        eval = |$p:ident, $i:ident, $g:ident| $body:expr
    ) => {
        $(#[$meta])*
        pub struct $name;

        impl Op for $name {
            const IDENTITY: Identity = typed_identity!($id, $version, [$($source),+]);

            type Params = $params;
            type Inputs<'a> = $inputs;
            type Payload = Field;

            fn cook(
                params: &Self::Params,
                inputs: &Self::Inputs<'_>,
                grid: Grid,
            ) -> Field {
                let $p: &$params = params;
                let $g: Grid = grid;
                let $i = inputs;
                $body
            }
        }
    };
}

field_op! {
    /// 常量场（无输入）。
    name = Constant,
    id = params::CONSTANT,
    version = 1,
    params = params::constant::Params,
    inputs = (),
    sources = [
        "typed.rs", "ops/constant.rs",
        "../../px_field_schema/src/field.rs",
        "../../px_field_schema/src/params.rs",
        "../../px_field_schema/src/payload.rs",
    ],
    eval = |p, _i, g| crate::ops::constant::eval(p, &[], g)
}

field_op! {
    /// 分形布朗噪声（无输入）。
    name = Fbm,
    id = params::FBM,
    version = 1,
    params = params::fbm::Params,
    inputs = (),
    sources = [
        "typed.rs", "ops/fbm.rs", "noise.rs",
        "../../px_field_schema/src/field.rs",
        "../../px_field_schema/src/noise.rs",
        "../../px_field_schema/src/params.rs",
        "../../px_field_schema/src/payload.rs",
    ],
    eval = |p, _i, g| crate::ops::fbm::eval(p, &[], g)
}

field_op! {
    /// 脊状噪声（无输入）。
    name = Ridged,
    id = params::RIDGED,
    version = 1,
    params = params::ridged::Params,
    inputs = (),
    sources = [
        "typed.rs", "ops/ridged.rs", "noise.rs",
        "../../px_field_schema/src/field.rs",
        "../../px_field_schema/src/noise.rs",
        "../../px_field_schema/src/params.rs",
        "../../px_field_schema/src/payload.rs",
    ],
    eval = |p, _i, g| crate::ops::ridged::eval(p, &[], g)
}

field_op! {
    /// 三张场按第三张当权重混合。
    name = Mix,
    id = params::MIX,
    version = 1,
    params = params::mix::Params,
    inputs = &'a [&'a Cooked<Field>; 3],
    sources = [
        "typed.rs", "ops/mix.rs",
        "../../px_field_schema/src/field.rs",
        "../../px_field_schema/src/params.rs",
        "../../px_field_schema/src/payload.rs",
    ],
    eval = |p, i, g| crate::ops::mix::eval(p, &[i[0].field(), i[1].field(), i[2].field()], g)
}

field_op! {
    /// 值域重映射。
    name = Remap,
    id = params::REMAP,
    version = 1,
    params = params::remap::Params,
    inputs = &'a Cooked<Field>,
    sources = [
        "typed.rs", "ops/remap.rs",
        "../../px_field_schema/src/field.rs",
        "../../px_field_schema/src/params.rs",
        "../../px_field_schema/src/payload.rs",
    ],
    eval = |p, i, g| crate::ops::remap::eval(p, &[i.field()], g)
}

field_op! {
    /// 域扭曲。
    name = Warp,
    id = params::WARP,
    version = 3,
    params = params::warp::Params,
    inputs = &'a [&'a Cooked<Field>; 2],
    sources = [
        "typed.rs", "ops/warp.rs", "noise.rs",
        "../../px_field_schema/src/field.rs",
        "../../px_field_schema/src/noise.rs",
        "../../px_field_schema/src/params.rs",
        "../../px_field_schema/src/payload.rs",
    ],
    eval = |p, i, g| crate::ops::warp::eval(p, &[i[0].field(), i[1].field()], g)
}

field_op! {
    /// 切向梯度的一个分量。
    name = Gradient,
    id = params::GRADIENT,
    version = 1,
    params = params::gradient::Params,
    inputs = &'a Cooked<Field>,
    sources = [
        "typed.rs", "ops/gradient.rs", "noise.rs",
        "../../px_field_schema/src/field.rs",
        "../../px_field_schema/src/noise.rs",
        "../../px_field_schema/src/params.rs",
        "../../px_field_schema/src/payload.rs",
    ],
    eval = |p, i, g| crate::ops::gradient::eval(p, &[i.field()], g)
}
