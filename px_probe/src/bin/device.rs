fn check() {
    let gpu = px_probe::common::require_gpu();
    let info = gpu.adapter.get_info();
    println!(
        "无窗口设备就绪：{}（{:?}，后端 {:?}）",
        info.name, info.device_type, info.backend
    );
    assert!(
        gpu.device.limits().max_compute_workgroup_size_x >= 64,
        "compute workgroup 上限只有 {}，跑不了探针",
        gpu.device.limits().max_compute_workgroup_size_x
    );
}

fn main() {
    px_probe::common::run_checks(
        "device",
        vec![(
            "a_headless_device_comes_up_with_no_window_and_no_swapchain",
            check,
        )],
    );
}
