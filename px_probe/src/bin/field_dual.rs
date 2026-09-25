fn main() {
    px_probe::common::require_gpu();
    px_probe::common::run_checks("field_dual", px_probe::field_dual::checks());
}
