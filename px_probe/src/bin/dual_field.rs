//! 云场的解析梯度 vs 对偶数（纯 CPU）。这一条原来是整个测试链里最贵的
//! （297 ms，占 85%），因为它是真计算而不是符号对照。
//! 退出码 0 = 全过。

fn main() {
    px_probe::common::run_checks("dual_field", px_probe::dual_field::checks());
}
