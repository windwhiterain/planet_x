//! `field.constant` 的**参数结构**（作者改参数的地方）。
//!
//! ⚠ 标记类型、`ElementFn` 实现与规格都在 `specs.rs` 那一行（`px_elem_specs!`）；
//!   算法在 `px_elem/body/constant.rs`（由 `px build` 编进那一份实例库）。

use px_field_schema::params::Shape;

/// 一整张场填同一个值。
///
/// ⚠ 形状是**参数**（"一切皆参数"那条裁定）：这一档没有上游，尺寸只有这一个来源。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, px_derive::PxParams)]
#[serde(default, deny_unknown_fields)]
pub struct ConstantParams {
    pub shape: Shape,
    pub value: f32,
}

impl Default for ConstantParams {
    fn default() -> Self {
        Self {
            shape: Shape::default(),
            value: 0.5,
        }
    }
}
