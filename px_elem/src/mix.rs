//! `field.mix` 的**参数与上游**（作者改这两样的地方）。
//!
//! ⚠ 上游有**三张场**（`a` / `b` / `mask`）—— element 函数自己声明几个上游，这正是
//!   "还能融合算子"那条裁定的基础：融合 = 一个函数一次拿到全部上游、在一个循环里算完。
//! ⚠ 标记类型、`ElementFn` 实现与规格都在 `specs.rs` 那一行；算法在 `px_elem/body/mix.rs`。

use px_field_schema::field::Field;
use px_graph_schema::Cooked;

/// 两份待混的场 + 一份权重场。
#[derive(px_derive::PxInputs)]
pub struct MixInput {
    pub a: Cooked<Field>,
    pub b: Cooked<Field>,
    pub mask: Cooked<Field>,
}

/// `field.mix`：按 `mask + bias` 在两份场之间插值（权重钳到 `[0,1]`）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, px_derive::PxParams)]
#[serde(default, deny_unknown_fields)]
pub struct MixParams {
    /// 权重场上的整体偏移（`0` = 照 mask 混）。
    pub bias: f32,
}

impl Default for MixParams {
    fn default() -> Self {
        Self { bias: 0.0 }
    }
}
