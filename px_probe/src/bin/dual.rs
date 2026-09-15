//! 对偶数微分本身的自洽性：分支、夹取、smoothstep、乘积。
//! 纯 CPU，不要设备。退出码 0 = 全过。

fn main() {
    px_probe::common::run_checks("dual", px_probe::dual::checks());
}
