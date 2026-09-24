//! 市场 / 经济模拟这一侧的 crate。
//!
//! 这个 crate **不是**渲染或 PCG 流水线的一部分：它是另一条血缘（模拟）搬进
//! workspace 之后的落点，`px_protocol` 只在**测试**里用它（快照测试按真类型构造
//! 世界视图那几份形状）。它住在 workspace 里、但**不在 `default-members`** ——
//! `cargo test` 默认不碰它，`cargo test --workspace` 才跑到。
//!
//! ⚠ 世界视图（[`sim`]）的**序列化形状**归这里，不归 `px_protocol`：
//! 那张协议只认 px-scene ⇄ px-pass 的交换，世界视图两边都不认。

pub mod department;
pub mod local_price;
pub mod market;
pub mod market_state;
pub mod sim;
pub mod utils;
pub mod warehouse;
