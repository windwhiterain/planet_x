fn main() {
    px_probe::common::require_gpu();
    px_probe::common::run_checks("gradient", px_probe::gradient::checks());
}
