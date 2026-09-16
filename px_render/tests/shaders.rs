mod common;

use common::{bevy_stub, expand, import_path_of, module_sources, render_source, shader_files};
use std::path::Path;

#[test]
fn the_backend_shader_stays_small_enough_for_a_driver() {
    let modules = module_sources();
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("px_render 必须住在 workspace 下")
        .join("target/shader-size");
    std::fs::create_dir_all(&directory).expect("建不了输出目录");

    let mut report: Vec<(String, usize, usize)> = Vec::new();

    for path in shader_files() {
        let source = std::fs::read_to_string(&path).expect("读不了 shader");
        if import_path_of(&source).is_some() {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let mut seen = Vec::new();
        let assembled = render_source(&source, &modules, bevy_stub, &mut seen);

        let module = naga::front::wgsl::parse_str(&assembled)
            .unwrap_or_else(|error| panic!("{name} 解析失败：{}", error.emit_to_string(&assembled)));
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        let info = validator
            .validate(&module)
            .unwrap_or_else(|error| panic!("{name} 校验失败：{error:?}"));

        let options = naga::back::hlsl::Options::default();
        let pipeline_options = naga::back::hlsl::PipelineOptions::default();
        let mut text = String::new();
        let mut writer = naga::back::hlsl::Writer::new(&mut text, &options, &pipeline_options);
        writer
            .write(&module, &info, None)
            .unwrap_or_else(|error| panic!("{name} 写 HLSL 失败：{error:?}"));
        std::fs::write(directory.join(format!("{name}.hlsl")), &text).expect("写不了 HLSL");

        report.push((name, assembled.lines().count(), text.lines().count()));
    }

    report.sort();
    for (name, wgsl, hlsl) in &report {
        println!("{name}：组装后 WGSL {wgsl} 行 → HLSL {hlsl} 行");
    }
    assert!(!report.is_empty(), "一个后端 shader 都没导出");

    let (_, _, biggest) = report
        .iter()
        .max_by_key(|(_, _, hlsl)| *hlsl)
        .expect("上面刚断言过非空");
    assert!(
        *biggest < 4000,
        "后端 shader 膨胀到 {biggest} 行：内联是按调用点复制的，\
         循环体里每多一个噪声函数，这个数就翻一倍"
    );
}

#[test]
fn every_shader_parses_and_validates() {
    let modules = module_sources();
    let mut checked = 0_usize;

    for path in shader_files() {
        if import_path_of(&std::fs::read_to_string(&path).expect("读不了 shader")).is_some()
            && !path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.starts_with("common"))
                .unwrap_or(false)
        {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("读不了 shader");
        if import_path_of(&source).is_some() {
            continue;
        }
        let mut seen = Vec::new();
        let assembled = render_source(&source, &modules, bevy_stub, &mut seen);
        let name = path.file_name().unwrap().to_string_lossy().to_string();

        let module = naga::front::wgsl::parse_str(&assembled).unwrap_or_else(|error| {
            panic!(
                "{name} 解析失败：\n{}\n---- 组装后的源码 ----\n{assembled}",
                error.emit_to_string(&assembled)
            )
        });
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator.validate(&module).unwrap_or_else(|error| {
            panic!("{name} 校验失败：{error:?}");
        });
        checked += 1;
    }

    assert!(checked > 0, "一个 shader 都没检查到");
    println!("shader 校验通过：{checked} 个");
}

/// 槽里那两个占位 shader 住成 Rust 内联常量（没有文件），所以它们不在
/// `shader_files()` 的扫描范围里 —— 必须单独拉进来，否则「每个 shader 都要
/// 解析并校验」这道门就对它们静默失效了。
#[test]
fn slot_placeholders_parse_and_validate() {
    let modules = module_sources();
    let mut checked = 0_usize;

    for (name, source) in px_render::slots::placeholders() {
        let mut seen = Vec::new();
        let assembled = render_source(&source, &modules, bevy_stub, &mut seen);
        let module = naga::front::wgsl::parse_str(&assembled).unwrap_or_else(|error| {
            panic!(
                "槽占位 {name} 解析失败：\n{}\n---- 组装后的源码 ----\n{assembled}",
                error.emit_to_string(&assembled)
            )
        });
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .unwrap_or_else(|error| panic!("槽占位 {name} 校验失败：{error:?}"));
        checked += 1;
    }

    assert_eq!(checked, 1, "只有一个槽：通用材质那份占位");
}

#[test]
fn the_checker_catches_a_broken_shader() {
    let broken = "fn bad() -> f32 { return vec3<f32>(1.0, 2.0, 3.0); }";
    let parsed = naga::front::wgsl::parse_str(broken);
    let rejected = match parsed {
        Err(_) => true,
        Ok(module) => {
            let mut validator = naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            );
            validator.validate(&module).is_err()
        }
    };
    assert!(rejected, "校验器必须能拒绝类型错误的 shader");
}

#[test]
fn an_unknown_import_is_not_silently_ignored() {
    let modules = module_sources();
    let mut seen = Vec::new();
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        expand("planet_x::missing::thing", &modules, bevy_stub, &mut seen)
    }))
    .is_err();
    assert!(caught, "未知 import 必须报错，不能静默略过");
}



