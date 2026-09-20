//! 文档 → `px_pass::Plan`（§130 的帧表落地）。
//!
//! 这一篇只做**翻译**，一条规则都不新增：
//!
//! - `kind` / `render` / `format` / `size` / `usage` 五样文本，全部走 `px_pass` 自己的
//!   解析器（[`PassKind::parse`] / [`RenderState::parse`] / [`Format::parse`] /
//!   [`SizeRule::parse`] / [`Use::parse`]）。宿主里再写一份"什么文本算合法"就是
//!   §66.1 那颗「同一份契约、两个数」的雷 —— 而它响的方式是"宿主收了执行器要拒的文档"。
//! - 绑定布局（组 3、参数块第 0 格、12 格贴图）**从材质契约表推**（`px_protocol::material`），
//!   不在这里抄第二张格子表：全屏 pass 与材质共用同一份布局，本来就是那一条契约的推论。
//! - 全屏 pass 的参数块按**那份 shader 自己声明的结构体**打包（`MaterialLayout::pack`），
//!   与材质走同一条路；`reads[k]` 落在 shader 声明的第 k 格贴图上。
//!
//! ⚠ 几何 pass 的**顶点阶段**（`vertex_shader` / `vertex_entry`）逐字照搬文档：
//! 它是宿主与 Bevy 逐位对齐的那一段（§110），由烘图侧内联进产物，宿主不重写。

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

/// 全屏 pass 的布局：**材质那一份**（组 3 / 参数块第 0 格 / 契约表里的 12 格贴图）。
///
/// 为什么全屏 pass 与材质共用：内容 shader 的绑定契约只有一份（§79 的 C 案），
/// `blit.wgsl` 声明的就是 `#{MATERIAL_BIND_GROUP}` 那几格。执行器按 `layout` 建它自己的
/// 绑定组，所以这一份必须是**契约表**推出来的那一份，不是另写一张更小的。
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
                },
            })
            .collect(),
    }
}

/// 一条 pass 的**全屏那一半**：组装后的 WGSL + 打包好的参数 + reads 落在哪几格。
///
/// 为什么是组装后的**全文**而不是成员名：`px_pass` 把 `PassPlan::shader` 直接交给
/// `create_shader_module`（执行器里没有组装器）。给成员名 = 运行期拿一段 `shaders/blit@…`
/// 去当 WGSL，错得很远。
struct Fullscreen {
    source: String,
    entry: String,
    params: Vec<u8>,
    slots: Vec<u32>,
}

/// 文档 → 计划。**五条 pass 全部**翻译并 `check()`，一条都不跳：
/// "这一档执行不执行它"是宿主的事（`render.rs` 里那三条），"它说不说得通"是这里的事。
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
            // 层数是**文档里的一个整数**（不是规则）：烘图侧解出来，这里照搬。
            layers: resource.layers,
            usage,
        });
    }

    let mut passes: Vec<PassPlan> = Vec::with_capacity(spec.passes.len());
    for (index, pass) in spec.passes.iter().enumerate() {
        let label = pass.label_or(index);
        // 状态文本：**空 = 这一版之前的写死行为**（`RenderState::default()`）。
        // ⚠ 老形状的 pass 没有这一栏，而它的语义就是"缺省那一套"（见 `RenderState::Default`）。
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
            // ⚠ 几何 pass 的片元阶段属于**材质**（§129）：`px_pass::Plan::check` 见到
            //    几何 pass 带 shader/entry 就当场拒。文档那边也一样（`SceneSpec::check`）。
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
            params: fullscreen
                .as_ref()
                .map(|screen| screen.params.clone())
                .unwrap_or_default(),
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
            // 写第几层：文档给的是 `light` + `face` + `layer` 三样，这里**对账**那条算式
            // （`layer == light × 6 + face`，`light.rs:2075`）再把它交给执行器。
            //
            // ⚠ 为什么对账住在这里：只有宿主知道 cube 是怎么排的（一盏投影的点光一个 cube、
            //    面序照 `CUBE_MAP_FACES`）；执行器只认"写第 k 层"。于是文档里那个 `layer`
            //    是一条**验证过的**事实，而不是一处"我说了算"的推导 —— 对不上就当场拒。
            layer: layer_of(pass, &label)?,
            vertex_shader: pass.vertex_shader.clone(),
            vertex_entry: pass.vertex_entry.clone(),
            // ⚠ 文档不给它：虚拟影图的一页落在 atlas 的哪一块是**宿主侧**算的
            //    （页数由 `shadow_density` 与包围球决定，见 `render::vshadow_of`）——
            //    所以这一格在翻译这一步是空的，宿主在建 pass 计划时逐条填。
            viewport: None,
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

/// 文档里 `cube_face` 那一格 → 执行器要的"写第几层"，**顺带把算式对一遍**。
///
/// 六面、六个层号，两处必须同时成立：`layer == light × 6 + face`（cube 的排法）与
/// `face < 6`（cube 就是六面）。⚠ 这里**不重算**那个数交给执行器（"重算一遍再交出去"
/// 就是把文档里那个数悄悄丢掉），而是**比**：文档说什么、算式说什么，两样一致才放行。
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

/// 全屏 pass 的三样：组装后的 WGSL、打包好的参数、`reads` 的格位。
fn fullscreen_of(    pass: &PassSpec,
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
    // 参数按**这份 shader 自己声明的结构体**打包：缺参 / 多参 / 类型不符三档当场报错，
    // 与材质同一条路（`MaterialLayout::pack`）。
    let params = loaded
        .layout
        .pack(&pass.params)
        .map_err(|err| format!("pass '{label}' 的参数：{err}"))?;
    // `reads[k]` 落在 shader 声明的第 k 格贴图上。数目对不上就当场拒 ——
    // "读了 N 张、shader 只声明了 M 格"里必然有一张被静默丢掉。
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
    Ok(Fullscreen {
        source: loaded.assembled,
        entry: pass.entry.clone(),
        params,
        slots: declared,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 布局**从契约表推**：组号、参数块格位、12 格贴图一个不差，采样器永远在 `+1`
    /// （执行器的绑定组就是这么建的）。
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
            };
            assert_eq!(slot.dimension, expected);
        }
    }

    /// 文档里的资源文本走 `px_pass` 的解析器：**认不出的格式/尺寸/用途要当场报错**，
    /// 而不是在宿主里悄悄有一份"我认得的取值"。
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
