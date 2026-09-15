mod common;

#[test]
fn the_assembled_cloud_module_carries_the_field_and_its_gradient() {
    let _ = (
        px_render::clouds::Ablate::Surface,
        std::mem::size_of::<px_render::clouds::CloudParams>(),
    );
    let assembled = common::assemble("clouds.wgsl");
    for wanted in [
        "fn cloud_field(",
        "fn cloud_field_gradient_analytic(",
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

/// 细节噪声的凹重映射必须**同时**接在值路径与梯度路径上：只改 `billows` 不改
/// `billows_along` 的话，画面会变而法线还是旧的（编译全绿、跑起来不对）。
/// 上界那条 `shape_of(cover, altitude, 1.0)` 则必须原样留着（`√1 = 1` 是这条论证的全部）。
#[test]
fn the_detail_remap_is_wired_into_both_the_field_and_its_gradient() {
    let assembled = common::assemble("clouds.wgsl");
    let body_of = |name: &str| -> String {
        let start = assembled
            .find(&format!("fn {name}("))
            .unwrap_or_else(|| panic!("组装后的 shader 里没有函数 {name}"));
        let rest = &assembled[start..];
        let end = rest[1..]
            .find("\nfn ")
            .unwrap_or_else(|| panic!("函数 {name} 后面没有别的函数，切不出来"));
        rest[..end + 1].to_string()
    };

    let remap = body_of("detail_curve");
    assert!(
        remap.contains("sqrt("),
        "detail_curve 里没有开根：{remap}"
    );
    assert!(
        body_of("billows").contains("detail_curve("),
        "字段那条路（billows）没接重映射"
    );
    let along = body_of("billows_along");
    // 点名到那两个混合变量：`billows_along` 里还有一条 `!with_skin` 的岔路，只查函数名的话
    // 主路径把因子丢了也照样绿（实测过：去掉主路径的 `* detail_curve_slope(blended)` 仍然通过）。
    assert!(
        along.contains("detail_curve(blended)"),
        "billows_along 的**值**没接重映射 ⇒ 法线量的是另一个场"
    );
    assert!(
        along.contains("* detail_curve_slope(blended)"),
        "billows_along 的**梯度**没乘链式因子 ⇒ 法线与场不自洽"
    );
    assert!(
        along.contains("* detail_curve_slope(clamped)"),
        "billows_along 的 `!with_skin` 岔路没乘链式因子"
    );
    assert!(
        assembled.contains("shape_of(cover, medium.altitude, 1.0)"),
        "保守上界那条路被改动了：它的论证只依赖 √1 = 1"
    );
}
