use std::path::Path;

use px_pass::{
    Dimension, Draw, Format, Layout, PassKind, PassPlan, Plan, RenderState, ResourceSpec, SizeRule,
    Slot, Use,
};
use px_protocol::material::{MATERIAL_BIND_GROUP, PARAMS_ALIGN, PARAMS_BINDING, TEXTURE_SLOTS};
use px_protocol::scene::{PassSpec, SceneSpec};

use crate::art;
use crate::group0::SHADOW_CUBE_FACES;
use crate::shader;

pub const STAGE_BIND_GROUP: u32 = 1;
pub const STAGE_PARAMS_BINDING: u32 = 0;

pub fn layout() -> Layout {
    Layout {
        group: MATERIAL_BIND_GROUP,
        params_binding: PARAMS_BINDING,
        params_align: PARAMS_ALIGN,
        slots: TEXTURE_SLOTS
            .iter()
            .map(|(binding, dimension)| Slot {
                binding: *binding,
                dimension: match dimension {
                    px_protocol::material::TextureDimension::D2 => Dimension::D2,
                    px_protocol::material::TextureDimension::Cube => Dimension::Cube,
                    px_protocol::material::TextureDimension::D2Array => Dimension::D2Array,
                },
                depth: false,
            })
            .collect(),
        geometry_group: STAGE_BIND_GROUP,
        geometry_params_binding: STAGE_PARAMS_BINDING,
    }
}

pub const GEOMETRY_PARAMS_SIZE: usize = 96;
pub const VIEW_PAGE_OFFSET: usize = 64;

fn page_params_of(pass: &PassSpec, label: &str) -> Result<Vec<u8>, String> {
    let mut bytes = vec![0_u8; GEOMETRY_PARAMS_SIZE];
    if let Some(value) = pass.params.get("view_page") {
        let px_protocol::scene::Value::Quad(rect) = value else {
            return Err(format!(
                "pass '{label}' 的参数 'view_page' 是 {}：这一页在面 NDC 里的矩形是四个数",
                match value {
                    px_protocol::scene::Value::Num(_) => "一个数",
                    px_protocol::scene::Value::Text(_) => "一段文本",
                    px_protocol::scene::Value::Triple(_) => "三个数",
                    px_protocol::scene::Value::Quad(_) => "四个数",
                }
            ));
        };
        for (index, number) in rect.iter().enumerate() {
            let at = VIEW_PAGE_OFFSET + index * 4;
            bytes[at..at + 4].copy_from_slice(&number.to_le_bytes());
        }
    }
    Ok(bytes)
}

struct Fullscreen {
    source: String,
    entry: String,
    params: Vec<u8>,
    slots: Vec<u32>,
    texture_slots: Vec<Slot>,
}

pub fn build(spec: &SceneSpec, pcg_root: &Path) -> Result<Plan, String> {
    let modules = shader::modules();
    let mut resources: Vec<ResourceSpec> = Vec::with_capacity(spec.resources.len());
    for resource in &spec.resources {
        let mut usage: Vec<Use> = Vec::with_capacity(resource.usage.len());
        for text in &resource.usage {
            usage.push(
                Use::parse(text)
                    .map_err(|err| format!("资源 '{}' 的用途：{err}", resource.name))?,
            );
        }
        resources.push(ResourceSpec {
            name: resource.name.clone(),
            format: Format::parse(&resource.format)
                .map_err(|err| format!("资源 '{}' 的格式：{err}", resource.name))?,
            size: SizeRule::parse(&resource.size)
                .map_err(|err| format!("资源 '{}' 的尺寸：{err}", resource.name))?,
            layers: resource.layers,
            usage,
        });
    }

    let mut passes: Vec<PassPlan> = Vec::with_capacity(spec.passes.len());
    for (index, pass) in spec.passes.iter().enumerate() {
        let label = pass.label_or(index);
        let render = if pass.render.trim().is_empty() {
            RenderState::default()
        } else {
            RenderState::parse(&pass.render)
                .map_err(|err| format!("第 {index} 条 pass '{label}' 的状态：{err}"))?
        };
        let kind = PassKind::parse(&pass.kind)
            .map_err(|err| format!("第 {index} 条 pass '{label}' 的类型：{err}"))?;
        let fullscreen = match kind {
            PassKind::Fullscreen => Some(fullscreen_of(pass, &label, pcg_root, &modules)?),
            _ => None,
        };
        passes.push(PassPlan {
            kind,
            label: label.clone(),
            shader: fullscreen
                .as_ref()
                .map(|screen| screen.source.clone())
                .unwrap_or_default(),
            entry: fullscreen
                .as_ref()
                .map(|screen| screen.entry.clone())
                .unwrap_or_default(),
            reads: pass.reads.clone(),
            writes: pass.writes.clone(),
            texture_slots: fullscreen
                .as_ref()
                .map(|screen| screen.texture_slots.clone())
                .filter(|slots| !slots.is_empty()),
            params: match (&fullscreen, kind) {
                (Some(screen), _) => screen.params.clone(),
                (None, PassKind::Geometry) if !pass.params.is_empty() => {
                    page_params_of(pass, &label)?
                }
                (None, _) => Vec::new(),
            },
            slots: fullscreen
                .as_ref()
                .map(|screen| screen.slots.clone())
                .unwrap_or_default(),
            render,
            draws: pass
                .draws
                .iter()
                .map(|draw| Draw {
                    geometry: draw.geometry.clone(),
                    material: draw.material.clone(),
                })
                .collect(),
            depth_target: pass.depth_target.clone(),
            layer: layer_of(pass, &label)?,
            vertex_shader: pass.vertex_shader.clone(),
            vertex_entry: pass.vertex_entry.clone(),
            viewport: pass.viewport,
            params_offset: 0,
        });
    }

    let plan = Plan {
        layout: layout(),
        resources,
        passes,
    };
    plan.check()
        .map_err(|err| format!("这份文档翻成计划之后说不通：{err}"))?;
    Ok(plan)
}

fn layer_of(pass: &PassSpec, label: &str) -> Result<Option<u32>, String> {
    let Some(cube) = &pass.cube_face else {
        return Ok(None);
    };
    if cube.face >= SHADOW_CUBE_FACES {
        return Err(format!(
            "pass '{label}' 的 cube_face.face 是 {}：cube 只有 {SHADOW_CUBE_FACES} 面",
            cube.face
        ));
    }
    let expected = cube.light * SHADOW_CUBE_FACES + cube.face;
    if cube.layer != expected {
        return Err(format!(
            "pass '{label}' 的 cube_face 说 layer={}，而 light={} × {SHADOW_CUBE_FACES} + face={} = {expected}：\
             两处对不上（层号是说出来让人对账的，不是唯一的真本）",
            cube.layer, cube.light, cube.face
        ));
    }
    Ok(Some(cube.layer))
}

fn fullscreen_of(
    pass: &PassSpec,
    label: &str,
    pcg_root: &Path,
    modules: &px_shader::ModuleTable,
) -> Result<Fullscreen, String> {
    let member = pass
        .shader
        .as_ref()
        .ok_or_else(|| format!("第 '{label}' 条 pass 是 fullscreen，却没说用哪份 shader 成员"))?;
    let loaded = art::load_shader(member, pcg_root, modules)
        .map_err(|err| format!("pass '{label}' 的片元阶段：{err}"))?;
    let params = loaded
        .layout
        .pack(&pass.params)
        .map_err(|err| format!("pass '{label}' 的参数：{err}"))?;
    let declared: Vec<u32> = loaded
        .layout
        .textures
        .iter()
        .map(|slot| slot.binding)
        .collect();
    if declared.len() != pass.reads.len() {
        return Err(format!(
            "pass '{label}' 读了 {} 张（{}），而 shader {} 声明了 {} 格贴图（{}）：\
             一格一张才对得上",
            pass.reads.len(),
            pass.reads.join(" / "),
            member,
            declared.len(),
            declared
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(" / ")
        ));
    }
    let texture_slots = px_shader::reflect::reflect_assembled(&loaded.assembled, label)
        .map_err(|err| format!("pass '{label}'：{err}"))?
        .textures
        .into_iter()
        .map(|slot| Slot {
            binding: slot.binding,
            dimension: match slot.dimension {
                px_protocol::material::TextureDimension::D2 => Dimension::D2,
                px_protocol::material::TextureDimension::Cube => Dimension::Cube,
                px_protocol::material::TextureDimension::D2Array => Dimension::D2Array,
            },
            depth: slot.depth,
        })
        .collect();
    Ok(Fullscreen {
        source: loaded.assembled,
        entry: pass.entry.clone(),
        params,
        slots: declared,
        texture_slots,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fullscreen_layout_is_the_material_contract() {
        let layout = layout();
        assert_eq!(layout.group, MATERIAL_BIND_GROUP);
        assert_eq!(layout.params_binding, PARAMS_BINDING);
        assert_eq!(layout.params_align, PARAMS_ALIGN);
        assert_eq!(layout.slots.len(), TEXTURE_SLOTS.len());
        for (slot, (binding, dimension)) in layout.slots.iter().zip(TEXTURE_SLOTS.iter()) {
            assert_eq!(slot.binding, *binding);
            let expected = match dimension {
                px_protocol::material::TextureDimension::D2 => Dimension::D2,
                px_protocol::material::TextureDimension::Cube => Dimension::Cube,
                px_protocol::material::TextureDimension::D2Array => Dimension::D2Array,
            };
            assert_eq!(slot.dimension, expected);
        }
    }

    #[test]
    fn resource_text_goes_through_the_executors_parsers() {
        assert!(Format::parse("rgba8unorm-srgb").is_ok());
        assert!(Format::parse("rgba8").is_err());
        assert!(SizeRule::parse("view").is_ok());
        assert!(SizeRule::parse("half").is_ok());
        assert!(SizeRule::parse("320x200").is_ok());
        assert!(SizeRule::parse("大").is_err());
        assert!(Use::parse("render_attachment").is_ok());
        assert!(Use::parse("color_attachment").is_err());
    }
}
