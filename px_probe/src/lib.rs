//! 渲染 / PCG 的探针集合。
//!
//! 这些原来是 `px_render/tests/*.rs` 里的 `#[test]`，但它们是探针不是门：要 GPU、
//! 要几分钟（§44.4 实测每次拿 adapter 4.24 s，而当时是 15 次）。挂在 `cargo test`
//! 上的代价是「跑一次测试链 = 跑一次 probe」，而且 `connect()` 失败还会静默变绿。
//!
//! 现在它们是普通 bin，退出码才是判据：
//!
//! ```text
//! cargo run -p px_probe --bin field_dual   # §46.3 说的那个 arbiter（唯一判据）
//! cargo run -p px_probe --bin gradient     # 探针冒烟 + 门约定 + 逐通道归因
//! cargo run -p px_probe --bin device       # 只要「无窗口设备能起来」
//! cargo run -p px_probe --bin dual         # 对偶数微分本身：分支、夹取、smoothstep、乘积
//! cargo run -p px_probe --bin dual_field   # 云场的解析梯度 vs 对偶数（纯 CPU）
//! cargo run -p px_probe --bin dual_noise   # 噪声的解析梯度 vs 对偶数（纯 CPU）
//! ```
//!
//! `dual*` 三个原在 `px_verify/tests/` 里当 `#[test]`：它们是**判据**不是门
//! （`dual_field` 一条就占当时整个测试链 85% 的时间），所以一样搬成探针。
//! 纯 CPU、不要设备，因此不调 `require_gpu()`。
//!
//! 优化程度从命令行选（只提升本地 crate，不重编 bevy）：
//!
//! ```text
//! cargo run -p px_probe --bin field_dual \
//!   --config 'profile.dev.package.px_ops.opt-level=2' \
//!   --config 'profile.dev.package.px_verify.opt-level=2' \
//!   --config 'profile.dev.package.px_probe.opt-level=2'
//! ```

pub mod common;
pub mod dual;
pub mod dual_field;
pub mod dual_noise;
pub mod field_dual;
pub mod gradient;
pub mod probe;
