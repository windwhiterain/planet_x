//! 体积算子的**声明**：身份 / 超参数 / 图参数形状 / 输出域。
//!
//! ⚠ **一行实现都没有**：算法在 `px_volume_op` 里（dylib，运行时按身份装载）。
//!   这里只说「一个算子是什么、吃什么、吐什么」——于是图侧接错一个上游、少给一个字段，
//!   都是**编译错**，而且编译这一份不需要实现库在场。
//!
//! ⚠ 图参数的形状（`CloudCoarseInput { coverage }`）**是接口的一部分**，所以它住在声明旁边：
//!   它是"这个算子被接的那个 struct"，不是实现细节。

use px_field_schema::field::Field;
use px_graph_schema::{Cooked, px_op};

use crate::VolumeData;
use crate::params;

/// 烘一份体积要吃的东西：**一张覆盖度场**。
#[derive(px_derive::PxInputs)]
pub struct CloudCoarseInput {
    pub coverage: Cooked<Field>,
}

/// 产物形状：`cook::<CloudCoarse>` 返回的就是它。
///
/// ⚠ 图脚本那一侧读体积的判据仪器都拿这个别名当签名（它只说明"拿到手的是一份体积"）。
pub type VolumeOut = Cooked<VolumeData>;

px_op! {
    /// 烘一份体积（立方球参数空间）。
    CloudCoarse, "cloud.coarse", "px_volume_op", params::Params, CloudCoarseInput, VolumeData
}
