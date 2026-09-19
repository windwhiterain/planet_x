//! 体积算子的**声明**：身份 + 超参数类型 + 图参数形状 + 输出域 + 怎么算。
//!
//! ⚠ 输入（哪张图接哪张场）在**图**那一侧：这里只声明"我吃一张场"。

use px_cook::{Cooked, Unary1, px_op};
use px_field_schema::field::Field;
use px_volume_schema::{VolumeData, params};

/// 一张场（上游由**图**接）。
/// **输入**形状：吃一张场。
pub type FieldInput = Unary1<Cooked<Field>>;

/// **产物**形状：吐一份体积（`cook::<CloudCoarse, …>` 的返回就是它）。
/// ⚠ 它被图侧包进 `Unary1` 当上游传，所以这里给的是裸的 `Cooked`。
pub type VolumeOut = Cooked<VolumeData>;

/// 烘一份体积（立方球参数空间）。
pub struct CloudCoarse;

px_op! { CloudCoarse = params::CLOUD_COARSE, params::Params, FieldInput, VolumeData,
         |p, i, _g| crate::eval_sampled(p, i.a.sample()) }
