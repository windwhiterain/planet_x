//! `field.fuse` 的**参数与上游**：一条链（remap → mix）的**融合**写法。
//!
//! ⚠ 这一条是"**还能融合算子**"那条裁定的样本：三个上游一次拿到、整条链在**一个循环**里
//!   算完 ⇒ 一个节点、一份产物、上游只读一次（对照：拆成 `elem::Remap` + `elem::Mix`
//!   两个节点 ⇒ 两张中间场、两次读盘/两次循环）。
//! ⚠ 融合**不是新机制**：element 这一档本来就是"一个函数一次拿到全部上游"，
//!   上游几个由函数自己声明（这里三个）。所以"融合"= 多写一行规格 + 一个体文件。

use px_field_schema::field::Field;
use px_graph_schema::Cooked;

/// 待重映的场 `a` + 混入的场 `b` + 权重场 `mask`（与 `field.mix` 同形）。
#[derive(px_derive::PxInputs)]
pub struct FuseInput {
    pub a: Cooked<Field>,
    pub b: Cooked<Field>,
    pub mask: Cooked<Field>,
}

/// `field.fuse` 的参数 = `remap` 那一半 ∪ `mix` 那一半。
///
/// ⚠ 两半的语义逐字沿用 [`crate::RemapParams`] 与 [`crate::MixParams`]（同一把 `Scale`、
///   同一个 `gamma` 口径、同一个 `bias` 钳法）—— 融合只该省下中间那张场，不该换算法。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, px_derive::PxParams)]
#[serde(default, deny_unknown_fields)]
pub struct FuseParams {
    pub in_min: f32,
    pub in_max: f32,
    pub out_min: f32,
    pub out_max: f32,
    pub smooth: bool,
    pub gamma: f32,
    /// `mix` 那一半：权重场上的整体偏移。
    pub bias: f32,
}

impl Default for FuseParams {
    fn default() -> Self {
        Self {
            in_min: 0.0,
            in_max: 1.0,
            out_min: 0.0,
            out_max: 1.0,
            smooth: false,
            gamma: 1.0,
            bias: 0.0,
        }
    }
}
