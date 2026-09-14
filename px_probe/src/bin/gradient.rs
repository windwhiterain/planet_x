//! 梯度探针（原 `px_render/tests/gradient.rs`）。
//!
//! 生产路径的梯度对错由 `--bin field_dual` 的 arbiter 断言（§46.3）；
//! 这里管「探针本人可用 + 门约定 + 残差归因」。

fn main() {
    px_probe::common::require_gpu();
    px_probe::common::run_checks("gradient", px_probe::gradient::checks());
}
