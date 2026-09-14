//! §46.3 的 arbiter：f64 精确、无步长、无折点余量、两侧对照。
//!
//! 「梯度对不对」现在只有这一个判据（原 `px_render/tests/field_dual.rs`）。
//! 退出码 0 = 全过；任何一条断言失败 = 非 0。

fn main() {
    px_probe::common::require_gpu();
    px_probe::common::run_checks("field_dual", px_probe::field_dual::checks());
}
