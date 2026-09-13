mod common;

#[test]
fn the_assembled_cloud_module_carries_the_field_and_its_gradient() {
    let assembled = common::assemble("clouds.wgsl");
    for wanted in [
        "fn cloud_field(",
        "fn cloud_field_gradient(",
        "fn fbm_3_grad(",
        "fn gradient_noise_3_grad(",
    ] {
        assert!(
            assembled.contains(wanted),
            "组装出来的云 shader 里找不到 {wanted}：噪声库可能没被内联进来"
        );
    }

    let module = naga::front::wgsl::parse_str(&assembled)
        .unwrap_or_else(|error| panic!("{}", error.emit_to_string(&assembled)));
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .unwrap_or_else(|error| panic!("组装后的云 shader 校验失败：{error:?}"));
}
