mod common;

use common::connect;

#[test]
fn a_headless_device_comes_up_with_no_window_and_no_swapchain() {
    let Some(gpu) = connect() else {
        eprintln!(
            "跳过：没有可用的 wgpu 适配器。这个测试要在真 GPU 或软件适配器上跑；\
             无 GPU 的 Linux 机器需要 lavapipe 加 WGPU_ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER=1"
        );
        return;
    };
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
