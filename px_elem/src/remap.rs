//! `field.remap` 的**参数与上游**（作者改这两样的地方）。
//!
//! ⚠ 标记类型、`ElementFn` 实现与规格都在 `specs.rs` 那一行；算法在 `px_elem/body/remap.rs`。

use px_field_schema::field::Field;
use px_graph_schema::Cooked;

/// 一张待重映的场。
#[derive(px_derive::PxInputs)]
pub struct RemapInput {
    pub field: Cooked<Field>,
}

/// `field.remap`：把上游的值域钳/归一化到 `[in_min, in_max]`，映到 `[out_min, out_max]`，
/// 再按 `gamma` 弯一次（`1.0` = 不弯）。
///
/// ⚠ 逐条字段的语义与代价写在从前那份 `px_field_schema::params::remap::Params` 的注释里
///   （2026-09-27 搬到这里 —— element 函数的参数类型归**函数自己**）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, px_derive::PxParams)]
#[serde(default, deny_unknown_fields)]
pub struct RemapParams {
    pub in_min: f32,
    pub in_max: f32,
    pub out_min: f32,
    pub out_max: f32,
    /// 归一化那一段是否走 smoothstep（`3t² - 2t³`）。
    pub smooth: bool,
    /// **强度非线性**（`1.0` = 不弯）：在 `out_*` **之后**取 `gamma` 次幂。
    ///
    /// ⚠ 为什么这一栏是必须的：收窄 `in_*` 窗口是**线性**拉伸，只能把"中灰"搬成"中灰的
    ///   另一个值"；星云要的是"大片接近 0 + 少数尖峰"，那个形状只能靠非线性拿到。
    ///   非正数一律不弯（负数取幂会得到 `NaN`，顺着管线传下去极难归因）。
    pub gamma: f32,
}

impl Default for RemapParams {
    fn default() -> Self {
        Self {
            in_min: 0.0,
            in_max: 1.0,
            out_min: 0.0,
            out_max: 1.0,
            smooth: false,
            gamma: 1.0,
        }
    }
}
