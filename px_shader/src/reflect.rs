//! 材质契约的**反射**：从组装好的 WGSL 里读出参数块的布局与贴图格。
//!
//! 为什么是**读 shader**而不是再写一张 Rust 侧的表：Bevy 的绑定布局是编译期的
//! （`Material::fragment_shader()` 是静态函数），产物能决定的只有槽里的源码。
//! 那么「参数怎么排」这件事只能有**一个**来源 —— 就是那份源码。写第二张表
//! 就是第二个会漂开的默认值。
//!
//! 为什么住在这个叶子 crate：烘图侧（`px_graphs` / `px_ops`）也要在**烘图时**反射一次
//! （schema descriptor 进产物、配方参数在烘图时就校验），而 `px_render` 拖着 bevy 进不去。
//! `px_protocol` 那边则被依赖门钉死只有 serde（`px_protocol/tests/crate_graph.rs`），
//! naga 同样进不去 ⇒ 反射只能住这里，契约的**类型**（[`MaterialLayout`] 等）住 `px_protocol::material`。

use px_protocol::material::{
    MATERIAL_BIND_GROUP, MAX_PARAMS_BYTES, MaterialLayout, PARAMS_ALIGN, PARAMS_BINDING, ParamKind,
    ParamSlot, TextureDimension, TextureSlot, texture_bindings, texture_slot_of,
};

/// 反射：从组装好的 WGSL 里读出材质契约。
///
/// `assembled` 必须是**组装过的**文本（`#import` 展开、bevy 外部符号换成桩），
/// 也就是 [`crate::assemble::render_source`] 的产物 —— 没组装过的入口连解析都过不去。
pub fn reflect_assembled(assembled: &str, name: &str) -> Result<MaterialLayout, String> {
    let module = naga::front::wgsl::parse_str(assembled)
        .map_err(|error| format!("{name} 解析失败：{}", error.emit_to_string(assembled)))?;
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .map_err(|error| format!("{name} 校验失败：{error:?}"))?;

    let mut params: Option<(Vec<ParamSlot>, u32)> = None;
    let mut textures: Vec<TextureSlot> = Vec::new();
    let mut samplers: Vec<u32> = Vec::new();

    for (_handle, global) in module.global_variables.iter() {
        let Some(binding) = global.binding else {
            continue;
        };
        if binding.group != MATERIAL_BIND_GROUP {
            continue;
        }
        let inner = &module.types[global.ty].inner;
        if binding.binding == PARAMS_BINDING {
            if !matches!(global.space, naga::AddressSpace::Uniform) {
                return Err(format!(
                    "{name} 的第 {PARAMS_BINDING} 格不是 uniform：参数块必须是 `var<uniform>`"
                ));
            }
            let naga::TypeInner::Struct { members, span } = inner else {
                return Err(format!(
                    "{name} 的第 {PARAMS_BINDING} 格不是结构体：参数块要声明成一个 `struct`"
                ));
            };
            let mut slots = Vec::new();
            for member in members {
                let member_name = member.name.clone().ok_or_else(|| {
                    format!("{name} 的参数块里有没名字的成员：它必须是 `名字: 类型` 的形状")
                })?;
                slots.push(ParamSlot {
                    name: member_name,
                    offset: member.offset,
                    kind: kind_of(&module, member.ty)
                        .map_err(|err| format!("{name} 的参数 '{:?}'：{err}", member.name))?,
                });
            }
            let bytes = span.div_ceil(PARAMS_ALIGN) * PARAMS_ALIGN;
            if bytes > MAX_PARAMS_BYTES {
                return Err(format!(
                    "{name} 的参数块是 {bytes} 字节，超过上限 {MAX_PARAMS_BYTES}"
                ));
            }
            params = Some((slots, bytes));
            continue;
        }
        match inner {
            naga::TypeInner::Image { dim, arrayed, .. } => {
                let dimension = match (dim, arrayed) {
                    (naga::ImageDimension::D2, false) => TextureDimension::D2,
                    (naga::ImageDimension::Cube, false) => TextureDimension::Cube,
                    _ => {
                        return Err(format!(
                            "{name} 第 {} 格是 {dim:?}（arrayed = {arrayed}）的贴图：\
                             材质只认 texture_2d 与 texture_cube",
                            binding.binding
                        ));
                    }
                };
                textures.push(TextureSlot {
                    binding: binding.binding,
                    dimension,
                });
            }
            naga::TypeInner::Sampler { .. } => samplers.push(binding.binding),
            other => {
                return Err(format!(
                    "{name} 第 {} 格是 {other:?}：材质绑定组只有参数块（第 0 格）、\
                     贴图（奇数格）与采样器（贴图 + 1）",
                    binding.binding
                ));
            }
        }
    }

    let Some((slots, params_bytes)) = params else {
        return Err(format!(
            "{name} 没有声明参数块：材质绑定组第 {PARAMS_BINDING} 格必须是 `var<uniform> params: <struct>`"
        ));
    };

    for texture in &textures {
        let Some(dimension) = texture_slot_of(texture.binding) else {
            return Err(format!(
                "{name} 把贴图声明在第 {} 格：贴图只能占 {} 这几格",
                texture.binding,
                texture_bindings()
            ));
        };
        if dimension != texture.dimension {
            return Err(format!(
                "{name} 第 {} 格声明的是 {}，约定里这一格是 {}",
                texture.binding,
                texture.dimension.name(),
                dimension.name()
            ));
        }
        let sampler = texture.binding + 1;
        if !samplers.contains(&sampler) {
            return Err(format!(
                "{name} 第 {} 格有贴图但没有采样器：采样器要声明在第 {sampler} 格",
                texture.binding
            ));
        }
    }

    textures.sort_by_key(|slot| slot.binding);
    Ok(MaterialLayout {
        params: slots,
        params_bytes,
        textures,
    })
}

fn kind_of(
    module: &naga::Module,
    handle: naga::Handle<naga::Type>,
) -> Result<ParamKind, String> {
    match &module.types[handle].inner {
        naga::TypeInner::Scalar(scalar) => scalar_kind(*scalar, 1),
        naga::TypeInner::Vector { size, scalar } => {
            let width = match size {
                naga::VectorSize::Bi => {
                    return Err("vec2 参数还没有产物能表达（Value 只有 数 / 三数 / 四数）".to_string());
                }
                naga::VectorSize::Tri => 3,
                naga::VectorSize::Quad => 4,
            };
            scalar_kind(*scalar, width)
        }
        other => Err(format!("参数块里有不认识的成员类型：{other:?}")),
    }
}

fn scalar_kind(scalar: naga::Scalar, width: usize) -> Result<ParamKind, String> {
    if scalar.width != 4 {
        return Err(format!(
            "参数里的标量是 {} 字节的：只认 4 字节的 f32 / i32 / u32",
            scalar.width
        ));
    }
    match (width, scalar.kind) {
        (1, naga::ScalarKind::Float) => Ok(ParamKind::F32),
        (1, naga::ScalarKind::Sint) => Ok(ParamKind::I32),
        (1, naga::ScalarKind::Uint) => Ok(ParamKind::U32),
        (3, naga::ScalarKind::Float) => Ok(ParamKind::Vec3),
        (4, naga::ScalarKind::Float) => Ok(ParamKind::Vec4),
        (_, kind) => Err(format!(
            "参数里的向量是 {kind:?} × {width}：只认 vec3<f32> / vec4<f32>"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reflect(name: &str) -> MaterialLayout {
        let modules = crate::workspace_modules(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("px_shader 必须住在 workspace 下"),
        )
        .expect("模块表");
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../art/shaders")
                .join(name),
        )
        .unwrap_or_else(|err| panic!("读不了 {name}：{err}"));
        let mut seen = Vec::new();
        let assembled = crate::assemble::render_source(&source, &modules, crate::assemble::bevy_stub, &mut seen);
        reflect_assembled(&assembled, name).unwrap_or_else(|err| panic!("{err}"))
    }

    fn assembled(source: &str) -> String {
        let modules = crate::workspace_modules(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("px_shader 必须住在 workspace 下"),
        )
        .expect("模块表");
        let mut seen = Vec::new();
        crate::assemble::render_source(source, &modules, crate::assemble::bevy_stub, &mut seen)
    }

    /// 契约的**形状**由 shader 自己的结构体说了算（不是 Rust 侧那张老表）。
    /// ⚠ 这里钉的是**名字与类型**，不是具体偏移：偏移是 shader 的事，改布局是作者的权利
    /// （§75 的 W4：原来把偏移钉死，等于让「改结构体」先撞上离线门）。
    #[test]
    fn the_cloud_params_come_from_the_shader_itself() {
        let layout = reflect("clouds.wgsl");
        assert_eq!(layout.params[0].name, "orientation");
        assert_eq!(layout.params[0].offset, 0);
        assert_eq!(layout.params[0].kind, ParamKind::Vec4);
        assert_eq!(layout.param("tint").expect("tint 在").kind, ParamKind::Vec4);
        assert_eq!(layout.param("steps").expect("steps 在").kind, ParamKind::U32);
        assert_eq!(layout.param("seed").expect("seed 在").kind, ParamKind::U32);
        assert_eq!(layout.param("wind_skin").expect("wind_skin 在").kind, ParamKind::F32);
        assert_eq!(layout.params_bytes % PARAMS_ALIGN, 0, "参数块按 16 对齐");
        assert!(
            layout.params_bytes as usize >= layout.params.last().expect("有参数").offset as usize,
            "参数块装得下最后一个参数"
        );
        assert!(
            layout.params.iter().all(|slot| slot.offset % slot.kind.width().min(4) == 0),
            "每个参数都落在自己类型的对齐上"
        );
        assert!(
            layout
                .params
                .iter()
                .all(|slot| slot.offset + slot.kind.width() <= layout.params_bytes),
            "每个参数都落在参数块内"
        );
    }

    /// 贴图格：云只有一张 cube（第 5 格），地表有两张 2D（1、3）+ 一张 cube（5）。
    #[test]
    fn the_texture_slots_follow_the_convention() {
        let clouds = reflect("clouds.wgsl");
        assert_eq!(clouds.textures.len(), 1);
        assert_eq!(clouds.textures[0].binding, 5);
        assert_eq!(clouds.textures[0].dimension, TextureDimension::Cube);

        let surface = reflect("surface.wgsl");
        let bindings: Vec<u32> = surface.textures.iter().map(|slot| slot.binding).collect();
        assert_eq!(bindings, vec![1, 3, 5]);
        assert_eq!(surface.texture(1).expect("在").dimension, TextureDimension::D2);
        assert_eq!(surface.texture(5).expect("在").dimension, TextureDimension::Cube);
    }

    #[test]
    fn a_param_block_that_is_not_a_struct_is_rejected() {
        let source = "#import bevy_pbr::forward_io::VertexOutput\n\
                      @group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: vec4<f32>;\n\
                      @fragment fn fragment(in: VertexOutput) -> @location(0) vec4<f32> { return params; }\n";
        let err = reflect_assembled(&assembled(source), "夹具").expect_err("必须是结构体");
        assert!(err.contains("结构体"), "报错要说清要什么：{err}");
    }

    #[test]
    fn a_texture_on_the_wrong_slot_is_rejected() {
        let source = "#import bevy_pbr::forward_io::VertexOutput\n\
                      struct P { x: f32 };\n\
                      @group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: P;\n\
                      @group(#{MATERIAL_BIND_GROUP}) @binding(2) var cover: texture_cube<f32>;\n\
                      @group(#{MATERIAL_BIND_GROUP}) @binding(3) var cover_sampler: sampler;\n\
                      @fragment fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {\n\
                      \x20   return textureSample(cover, cover_sampler, vec3<f32>(0.0, 0.0, 1.0));\n\
                      }\n";
        let err = reflect_assembled(&assembled(source), "夹具").expect_err("偶数格必须被拒");
        assert!(err.contains("第 2 格"), "报错要点名那一格：{err}");
    }

    /// 组装器必须把 `#{MATERIAL_BIND_GROUP}` 替成**运行期那个数**（Bevy 的 3），
    /// 否则门测的不是同一份文本（§75 的 2/3 那颗雷）。
    #[test]
    fn the_assembled_text_uses_the_runtime_bind_group() {
        let source = "#import bevy_pbr::forward_io::VertexOutput\n\
                      struct P { x: f32 };\n\
                      @group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: P;\n\
                      @fragment fn fragment(in: VertexOutput) -> @location(0) vec4<f32> { return vec4<f32>(params.x); }\n";
        let text = assembled(source);
        assert!(
            text.contains(&format!("@group({MATERIAL_BIND_GROUP})")),
            "组装后的文本里应当只剩运行期那个组号：{MATERIAL_BIND_GROUP}"
        );
        assert!(!text.contains("#{MATERIAL_BIND_GROUP}"), "占位符必须被替掉");
        let layout = reflect_assembled(&text, "夹具").expect("反射");
        assert_eq!(layout.params.len(), 1);
    }
}
