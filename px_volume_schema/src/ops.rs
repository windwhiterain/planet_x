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

/// 烘一份**可步进的密度**要吃的东西：**一张三维密度场**（`Domain::Volume`）。
///
/// ⚠ 字段名是 `density` 而不是 `field`：这个算子的语义是"把这张密度场搬进体积"
///   （它是采样源，不是被修改的基底）—— 与 `CloudCoarseInput::coverage` 同一个口径。
#[derive(px_derive::PxInputs)]
pub struct DensityInput {
    pub density: Cooked<Field>,
}

/// 产物形状：`cook::<CloudCoarse>` 返回的就是它。
///
/// ⚠ 图脚本那一侧读体积的判据仪器都拿这个别名当签名（它只说明"拿到手的是一份体积"）。
pub type VolumeOut = Cooked<VolumeData>;

px_op! {
    /// 烘一份体积（立方球参数空间）。
    CloudCoarse, "cloud.coarse", "px_volume_op", params::Params, CloudCoarseInput, VolumeData
}

px_op! {
    /// **三维场 → 可步进的密度体积**。
    ///
    /// ⚠ 与 [`CloudCoarse`] 是**两个算子**（不是同一份参数的新档）：那一档服务**等值面提取**
    ///   （存 `(场-τ)/L`，形状参数是一整套云的形状），这一档服务**体渲染的沿视线积分**
    ///   （存密度本身 + 壳的内外半径，多一个径向保守化）。两者的"值"含义不同，
    ///   混用会让"步进读到的密度"与"网格提取的等值面"说的不是同一件事。
    Density, "cloud.density", "px_volume_op", params::density::DensityParams, DensityInput, VolumeData
}
