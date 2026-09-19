//! 体积算子的**声明**：身份 / 超参数 / 图参数形状 / 输出域 / 怎么算 / 源码清单。
//!
//! ⚠ **图参数的形状是算子接口的一部分**（`CloudCoarseInput { coverage }`）——
//! 图侧写错字段、少给上游都是编译错。

use px_cook::{Cooked, px_op};
use px_field_schema::field::Field;
use px_volume_schema::{VolumeData, params};

/// 烘一份体积要吃的东西：**一张覆盖度场**。
#[derive(px_derive::PxInputs)]
pub struct CloudCoarseInput {
    pub coverage: Cooked<Field>,
}

/// 产物形状：`cook::<CloudCoarse>` 返回的就是它。
pub type VolumeOut = Cooked<VolumeData>;

/// 烘一份体积（立方球参数空间）。
pub struct CloudCoarse;

px_op! { CloudCoarse = params::CLOUD_COARSE, params::Params, CloudCoarseInput, VolumeData,
         |p, i, _g| crate::eval_sampled(p, i.coverage.sample()) }
