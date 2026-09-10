//! 命令面 (command surface) —— agent 写 `--apply` diff / 读控制面的领域逻辑。
//!
//! 这是与 HTTP/UI **无关** 的引擎能力：CLI（`--control` / `--apply` /
//! `--control-schema`）、`planet_x_web` crate 的 `POST /api/command`、以及
//! `sim`/`autocontrol` 的测试都消费这里暴露的补丁解析/应用与读面构建。
//! 它只依赖 [`crate::model`]，不依赖任何 HTTP/web 栈，因此引擎本体可以
//! 完全不带 axum/tokio/tower-http。
//!
//! 两类出口：
//! * **读面** —— 当前可控状态渲染成可编辑模板（`[`control_surface`]`）或
//!   web 的 `StateView.control` 片段（`[`control_view`]`/`[`scope_view`]`）。
//! * **写面** —— 一个 presence-aware 的多级 diff（`[`CommandReq`]`）被
//!   `[`apply_diff`]`/`[`apply_patch`]` 叠加到 [`State`] 上：只改「出现在 diff 里」
//!   的势力/叶子，缺省的 `value`/`behavior` 保留现值、缺省的 `mode` 保留现模式。

use crate::model::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;


pub mod apply;
pub mod blueprint;
pub mod budget;
pub mod building;
pub mod normalize;
pub mod ship;
pub mod view;
pub mod wire;

pub use apply::*;
pub use blueprint::*;
pub use budget::*;
pub use building::*;
pub use normalize::*;
pub use ship::*;
pub use view::*;
pub use wire::*;

#[cfg(test)]
#[path = "../tests/control/mod.rs"]
mod tests;
