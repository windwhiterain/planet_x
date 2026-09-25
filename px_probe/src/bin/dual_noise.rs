fn main() {
    if std::env::args().any(|arg| arg == "--diagnose") {
        println!("== 诊断（不参与裁决，只是当初排查的原始数据）==");
        px_probe::dual_noise::diagnose();
    }
    px_probe::common::run_checks("dual_noise", px_probe::dual_noise::checks());
}
