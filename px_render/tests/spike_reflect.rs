mod common;

use common::{bevy_stub, module_sources, render_source};
use naga::AddressSpace;

fn describe(module: &naga::Module, handle: naga::Handle<naga::Type>) -> String {
    match &module.types[handle].inner {
        naga::TypeInner::Scalar(scalar) => format!("{:?}{}", scalar.kind, scalar.width * 8),
        naga::TypeInner::Vector { size, scalar } => {
            format!("vec{}<{:?}{}>", *size as u32, scalar.kind, scalar.width * 8)
        }
        naga::TypeInner::Matrix {
            columns,
            rows,
            scalar,
        } => format!(
            "mat{}x{}<{:?}{}>",
            *columns as u32, *rows as u32, scalar.kind, scalar.width * 8
        ),
        naga::TypeInner::Array { base, size, stride } => format!(
            "array<{}, {}> stride {}",
            describe(module, *base),
            match size {
                naga::ArraySize::Constant(value) => format!("{}", value.get()),
                naga::ArraySize::Pending(_) => "?".to_string(),
                naga::ArraySize::Dynamic => "runtime".to_string(),
            },
            stride
        ),
        naga::TypeInner::Struct { members, span } => format!("struct({} 成员, span {})", members.len(), span),
        other => format!("{other:?}"),
    }
}

#[test]
fn spike_what_the_material_uniform_looks_like() {
    let modules = module_sources();
    for name in ["clouds.wgsl", "surface.wgsl", "atmosphere.wgsl"] {
        let path = px_render::shaders::shader_source_of(name);
        let source = std::fs::read_to_string(&path).expect("读不了 shader");
        let mut seen = Vec::new();
        let assembled = render_source(&source, &modules, bevy_stub, &mut seen);
        let module = naga::front::wgsl::parse_str(&assembled)
            .unwrap_or_else(|error| panic!("{name} 解析失败：{}", error.emit_to_string(&assembled)));
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator.validate(&module).expect("校验失败");

        println!("===== {name} =====");
        let mut globals: Vec<_> = module.global_variables.iter().collect();
        globals.sort_by_key(|(_, global)| {
            (
                global.binding.map(|binding| binding.group).unwrap_or(u32::MAX),
                global.binding.map(|binding| binding.binding).unwrap_or(u32::MAX),
            )
        });
        for (handle, global) in globals {
            let Some(binding) = global.binding else {
                continue;
            };
            let space = match global.space {
                AddressSpace::Uniform => "uniform".to_string(),
                AddressSpace::Storage { .. } => "storage".to_string(),
                other => format!("{other:?}"),
            };
            println!(
                "  @group({}) @binding({}) {space} {} : {}",
                binding.group,
                binding.binding,
                global.name.clone().unwrap_or_else(|| "?".to_string()),
                describe(&module, global.ty)
            );
            if let naga::TypeInner::Struct { members, span } = &module.types[global.ty].inner {
                println!("      span = {span}");
                for member in members {
                    println!(
                        "      +{:<4} {:<24} {}",
                        member.offset,
                        member.name.clone().unwrap_or_else(|| "?".to_string()),
                        describe(&module, member.ty)
                    );
                }
            }
            let _ = handle;
        }
    }
}
