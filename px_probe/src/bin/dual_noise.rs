//! 噪声的解析梯度 vs 对偶数（纯 CPU）。退出码 0 = 全过。
//!
//! `--diagnose` 额外打印当初那次排查的原始数据（各轴对偶数梯度、四种步长的中心差分、
//! 各八度是否被夹住、fbm 均值）。它**只有打印、没有断言**，所以不算 check ——
//! 一个永远不会 ✗ 的 ✓ 是假绿。默认不跑。

fn main() {
    if std::env::args().any(|arg| arg == "--diagnose") {
        println!("== 诊断（不参与裁决，只是当初排查的原始数据）==");
        px_probe::dual_noise::diagnose();
    }
    px_probe::common::run_checks("dual_noise", px_probe::dual_noise::checks());
}
