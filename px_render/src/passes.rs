use std::path::Path;
use std::sync::{Arc, Mutex};

use bevy::core_pipeline::Core3dSystems;
use bevy::core_pipeline::schedule::Core3d;
use bevy::prelude::*;
use bevy::render::renderer::{RenderContext, ViewQuery};
use bevy::render::view::ViewTarget;
use bevy::render::{Extract, ExtractSchedule};

use px_protocol::material::{
    MATERIAL_BIND_GROUP, PARAMS_ALIGN, PARAMS_BINDING, TEXTURE_SLOTS, TextureDimension,
};
use px_protocol::scene::{SceneSpec, VIEW_BUILTIN};

use crate::art_cache::ArtCache;

#[derive(Resource, Clone, Default)]
pub struct DocumentPasses(pub Option<Arc<px_pass::Plan>>);

#[derive(Resource, Clone, Default)]
pub struct RenderPasses(pub Option<Arc<px_pass::Plan>>);

#[derive(Resource, Clone, Default)]
pub struct PassFailure(Arc<Mutex<Option<String>>>);

impl PassFailure {
    pub fn get(&self) -> Option<String> {
        self.0.lock().unwrap_or_else(|err| err.into_inner()).clone()
    }

    pub fn set(&self, detail: String) {
        let mut slot = self.0.lock().unwrap_or_else(|err| err.into_inner());
        if slot.is_none() {
            *slot = Some(detail);
        }
    }

    pub fn clear(&self) {
        *self.0.lock().unwrap_or_else(|err| err.into_inner()) = None;
    }
}

#[derive(Resource, Default)]
pub struct PassHost {
    executor: px_pass::Executor,
}

fn validate_fragment(label: &str, entry: &str, source: &str) -> Result<(), String> {
    let module = naga::front::wgsl::parse_str(source)
        .map_err(|error| format!("pass '{label}' 的 shader 解析失败：{}", error.emit_to_string(source)))?;
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .map_err(|error| format!("pass '{label}' 的 shader 校验失败：{error:?}"))?;
    let entries: Vec<&str> = module
        .entry_points
        .iter()
        .filter(|point| point.stage == naga::ShaderStage::Fragment)
        .map(|point| point.name.as_str())
        .collect();
    if !entries.contains(&entry) {
        return Err(format!(
            "pass '{label}' 的 shader 里没有 @fragment 入口 '{entry}'；它有的片段入口：{}",
            if entries.is_empty() {
                "（一个都没有）".to_string()
            } else {
                entries.join(" / ")
            }
        ));
    }
    Ok(())
}

/// 执行器的绑定布局 = **材质的契约**（§85 的 C 案）。
///
/// 为什么要把表读出来填进去、而不是让 `px_pass` 自己去认识它：执行器里一个内建名字都不该有
/// （它不知道"材质"、不知道 `view`、也不知道哪一格是参数块）。而抄一份常量进 `px_pass`
/// 就是第二个会漂开的真相 —— 与 §66.1 那颗「同一条契约、两个数字」的雷同一族。
pub fn executor_layout() -> px_pass::Layout {
    px_pass::Layout {
        group: MATERIAL_BIND_GROUP,
        params_binding: PARAMS_BINDING,
        params_align: PARAMS_ALIGN,
        slots: TEXTURE_SLOTS
            .iter()
            .map(|(binding, dimension)| px_pass::Slot {
                binding: *binding,
                dimension: match dimension {
                    TextureDimension::D2 => px_pass::Dimension::D2,
                    TextureDimension::Cube => px_pass::Dimension::Cube,
                },
            })
            .collect(),
    }
}

pub fn resolve(
    document: &SceneSpec,
    cache: &mut ArtCache,
    pcg_root: &Path,
) -> Result<(Option<Arc<px_pass::Plan>>, String), String> {
    if document.passes.is_empty() {
        return Ok((None, "pass 表：空（只有主 pass，与没有这一节时逐字节相同）".to_string()));
    }
    let mut plan = px_pass::Plan {
        layout: executor_layout(),
        ..Default::default()
    };
    let mut lines = vec![format!("pass 表：{} 条（数组顺序就是执行顺序）", document.passes.len())];

    for resource in &document.resources {
        let at = |err: String| format!("pass 资源 '{}'：{err}", resource.name);
        let format = px_pass::Format::parse(&resource.format).map_err(at)?;
        let size = px_pass::SizeRule::parse(&resource.size).map_err(at)?;
        let mut usage = Vec::new();
        for text in &resource.usage {
            usage.push(px_pass::Use::parse(text).map_err(at)?);
        }
        lines.push(format!(
            "  资源 {}：{}｜{}｜{}",
            resource.name,
            format.name(),
            size.name(),
            usage
                .iter()
                .map(|use_| use_.name())
                .collect::<Vec<_>>()
                .join("+")
        ));
        plan.resources.push(px_pass::ResourceSpec {
            name: resource.name.clone(),
            format,
            size,
            usage,
        });
    }

    for (index, pass) in document.passes.iter().enumerate() {
        let label = pass.label_or(index);
        let kind = match pass.kind.as_str() {
            "fullscreen" => px_pass::PassKind::Fullscreen,
            "compute" => px_pass::PassKind::Compute,
            other => {
                return Err(format!(
                    "pass '{label}' 的 kind 是 '{other}'：认 'fullscreen' 与 'compute'"
                ));
            }
        };
        if kind == px_pass::PassKind::Compute {
            return Err(format!(
                "pass '{label}' 的 kind 是 compute：这一版执行器只有 fullscreen。\
                 声明了执行器不兑现的东西就当场拒 —— 静默跳过正是要避免的那种故障"
            ));
        }
        let path = pass.shader.resolve(pcg_root)?;
        let entry = cache.shader(&path.display().to_string())?;
        let source = entry.value.source.clone();
        if source.contains("#import") {
            return Err(format!(
                "pass '{label}' 的 shader 里有 #import：执行器把 WGSL 直接交给 wgpu，不跑 naga_oil 组装\
                 ⇒ 当场拒（要用库就先离线组装成一份自足的 WGSL）"
            ));
        }
        // 先组装再校验：入口文本里的 `#{MATERIAL_BIND_GROUP}` 还不是合法 WGSL，
        // 组装器把它替成契约里那个数（与材质那条路是同一个函数、同一份表）。
        let assembled = {
            let modules = crate::shaders::module_sources();
            let mut seen = Vec::new();
            crate::shaders::render_source(&source, &modules, crate::shaders::bevy_stub, &mut seen)
        };
        validate_fragment(&label, &pass.entry, &assembled)?;

        // 参数与格位都由**这份 shader 自己的契约**决定：反射出来什么就绑什么，一个数都不猜。
        let version = px_render_shader_version(&pass.shader.key);
        let contract = crate::reflect::layout_of(version, &format!("pass '{label}'"), &source)?;
        if contract
            .textures
            .iter()
            .any(|slot| slot.dimension != TextureDimension::D2)
        {
            return Err(format!(
                "pass '{label}' 的 shader 声明了 cube 贴图格：这一版的 pass 资源（含内建 '{}'）\
                 全是 2D —— 声明了执行器兑现不了的东西就当场拒",
                VIEW_BUILTIN
            ));
        }
        let slots: Vec<u32> = contract.textures.iter().map(|slot| slot.binding).collect();
        if slots.len() != pass.reads.len() {
            return Err(format!(
                "pass '{label}' 的 shader 声明了 {} 个 2D 贴图格（[{}]），而这份 pass 有 {} 个 reads（[{}]）\
                 —— 一条 read 占一格，对不上就是绑错东西",
                slots.len(),
                slots.iter().map(u32::to_string).collect::<Vec<_>>().join(" / "),
                pass.reads.len(),
                pass.reads.join(" / "),
            ));
        }
        let params = contract
            .pack(&pass.params)
            .map_err(|err| format!("pass '{label}' 的参数：{err}"))?;

        lines.push(format!(
            "  {label}：{}｜读 {} → 格 [{}]｜写 [{}]｜参数 {} 个（{} 字节）｜shader {}/{}（版本 {:016x}）{}",
            pass.kind,
            pass.reads.join(" / "),
            slots.iter().map(u32::to_string).collect::<Vec<_>>().join(" / "),
            pass.writes.join(" / "),
            pass.params.len(),
            params.len(),
            pass.shader.graph,
            pass.shader.node,
            version,
            if entry.hit { "缓存命中" } else { "现读产物" },
        ));
        plan.passes.push(px_pass::PassPlan {
            kind,
            label,
            shader: assembled,
            entry: pass.entry.clone(),
            reads: pass.reads.clone(),
            writes: pass.writes.clone(),
            params,
            slots,
            // 这一版从这里来的 pass 全是全屏后处理：状态就是 `RenderState::default()`
            // （清成透明、不挂深度、不剔除）—— 与搬进数据模型之前写在执行器里的那套逐字相同。
            render: px_pass::RenderState::default(),
            // 从文档表来的 pass 全是全屏后处理：`draws`（画什么几何）、`depth_target`
            // 与顶点阶段三栏都空着 —— 几何那一档才用得上它们。
            ..Default::default()
        });
    }

    plan.check()?;
    Ok((Some(Arc::new(plan)), lines.join("\n")))
}

fn px_render_shader_version(key: &str) -> u64 {
    u64::from_str_radix(key.get(..16).unwrap_or("0"), 16).unwrap_or(0)
}

pub fn extract_passes(passes: Extract<Res<DocumentPasses>>, mut commands: Commands) {
    commands.insert_resource(RenderPasses(passes.0.clone()));
}

pub fn run_passes(
    view: ViewQuery<&ViewTarget>,
    passes: Option<Res<RenderPasses>>,
    failure: Option<Res<PassFailure>>,
    mut host: ResMut<PassHost>,
    mut announced: Local<Option<usize>>,
    mut ctx: RenderContext,
) {
    let (Some(passes), Some(failure)) = (passes, failure) else {
        return;
    };
    let Some(plan) = passes.0.as_ref() else {
        return;
    };
    if failure.get().is_some() {
        return;
    }
    let target = view.into_inner();
    let format = target.main_texture_format();
    let pairs: Vec<_> = plan
        .passes
        .iter()
        .filter(|pass| pass.target() == Some(VIEW_BUILTIN))
        .map(|_| target.post_process_write())
        .collect();
    let mut sets: Vec<Vec<px_pass::External>> = Vec::new();
    let mut current = pairs.first().map(|pair| pair.source);
    let mut cursor = 0_usize;
    for pass in &plan.passes {
        let mut set: Vec<px_pass::External> = Vec::new();
        if pass.reads.iter().any(|read| read == VIEW_BUILTIN) {
            if let Some(source) = current {
                set.push(px_pass::External {
                    name: VIEW_BUILTIN,
                    role: px_pass::Role::Read,
                    view: source,
                    format,
                });
            }
        }
        if pass.target() == Some(VIEW_BUILTIN) {
            let pair = &pairs[cursor];
            cursor += 1;
            set.push(px_pass::External {
                name: VIEW_BUILTIN,
                role: px_pass::Role::Write,
                view: pair.destination,
                format,
            });
            current = Some(pair.destination);
        }
        sets.push(set);
    }
    let frame = px_pass::Frame {
        width: target.main_texture().width(),
        height: target.main_texture().height(),
        sets: &sets,
        // 这一版的文档 pass 表里一条几何 pass 都没有（全是全屏后处理）⇒ 解析结果为空表。
        // 要画几何时，宿主在这里把 `Draw { geometry, material }` 里的名字解析成 GPU 句柄。
        geometries: &[],
        materials: &[],
    };
    let key = Arc::as_ptr(plan) as usize;
    let device = ctx.render_device().clone();
    let encoder = ctx.command_encoder();
    match host
        .executor
        .execute(device.wgpu_device(), encoder, plan, &frame)
    {
        Ok(audit) => {
            if *announced != Some(key) {
                println!("{audit}");
                *announced = Some(key);
            }
        }
        Err(err) => {
            println!("pass 执行失败：{err}");
            failure.set(format!("pass 执行失败：{err}"));
        }
    }
}

pub struct PassPlugin;

impl Plugin for PassPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DocumentPasses>();
        let failure = PassFailure::default();
        app.insert_resource(failure.clone());
        let Some(render_app) = app.get_sub_app_mut(bevy::render::RenderApp) else {
            return;
        };
        render_app
            .init_resource::<PassHost>()
            .insert_resource(failure)
            .add_systems(ExtractSchedule, extract_passes)
            .add_systems(Core3d, run_passes.in_set(Core3dSystems::EarlyPostProcess));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use px_protocol::scene::{PassResource, PassSpec};

    fn member() -> px_protocol::scene::Member {
        px_protocol::scene::Member::new("shaders", "grade", &"0".repeat(64))
    }

    fn document(resources: Vec<PassResource>, passes: Vec<PassSpec>) -> SceneSpec {
        SceneSpec {
            schema: px_protocol::scene::SCENE_SCHEMA,
            name: "t".to_string(),
            environment: default(),
            cameras: Vec::new(),
            expects: Vec::new(),
            resources,
            passes,
            lights: Vec::new(),
            objects: vec![px_protocol::scene::Object {
                id: "o".to_string(),
                geometry: px_protocol::scene::Geometry::Primitive {
                    name: "sphere".to_string(),
                    params: Default::default(),
                },
                material: px_protocol::scene::Material {
                    shader: member(),
                    params: Default::default(),
                    textures: Default::default(),
                    alpha: Default::default(),
                    cull: Default::default(),
                    depth_bias: 0.0,
                },
                transform: default(),
                cast_shadow: true,
            }],
        }
    }

    fn pass(label: &str, reads: &[&str], writes: &[&str]) -> PassSpec {
        PassSpec {
            kind: "fullscreen".to_string(),
            shader: member(),
            label: label.to_string(),
            entry: "fs_main".to_string(),
            reads: reads.iter().map(|name| name.to_string()).collect(),
            writes: writes.iter().map(|name| name.to_string()).collect(),
            params: Default::default(),
        }
    }

    #[test]
    fn a_pass_that_writes_the_view_is_accepted() {
        let spec = document(Vec::new(), vec![pass("grade", &["view"], &["view"])]);
        assert!(spec.check().is_ok(), "{:?}", spec.check());
    }

    #[test]
    fn an_undeclared_target_is_refused_by_name() {
        let spec = document(Vec::new(), vec![pass("grade", &[], &["scratch"])]);
        let err = spec.check().expect_err("写到没声明的资源必须拒");
        assert!(err.contains("scratch"), "{err}");
        assert!(err.contains("resources"), "{err}");
    }

    #[test]
    fn a_pass_table_that_never_touches_the_view_is_refused() {
        let spec = document(
            vec![PassResource {
                name: "scratch".to_string(),
                format: "rgba16float".to_string(),
                size: "view".to_string(),
                usage: vec!["render_attachment".to_string()],
            }],
            vec![pass("grade", &[], &["scratch"])],
        );
        let err = spec.check().expect_err("没有一条写 view 的 pass 表必须拒");
        assert!(err.contains("view"), "{err}");
    }

    #[test]
    fn reading_and_writing_the_same_target_is_refused() {
        let spec = document(Vec::new(), vec![pass("grade", &["view"], &["view"])]);
        assert!(spec.check().is_ok());
        let spec = document(
            vec![PassResource {
                name: "scratch".to_string(),
                format: "rgba16float".to_string(),
                size: "view".to_string(),
                usage: vec![
                    "render_attachment".to_string(),
                    "texture_binding".to_string(),
                ],
            }],
            vec![
                pass("first", &[], &["scratch"]),
                pass("second", &["scratch"], &["scratch"]),
                pass("last", &["scratch"], &["view"]),
            ],
        );
        let err = spec.check().expect_err("同时读写同一个目标必须拒");
        assert!(err.contains("同时读和写"), "{err}");
    }

    #[test]
    fn a_resource_named_view_is_refused() {
        let spec = document(
            vec![PassResource {
                name: "view".to_string(),
                format: "rgba16float".to_string(),
                size: "view".to_string(),
                usage: vec!["render_attachment".to_string()],
            }],
            vec![pass("grade", &[], &["view"])],
        );
        let err = spec.check().expect_err("内建名不许被占用");
        assert!(err.contains("内建名"), "{err}");
    }

    #[test]
    fn an_unknown_kind_is_refused() {
        let mut spec = document(Vec::new(), vec![pass("grade", &[], &["view"])]);
        spec.passes[0].kind = "raytracing".to_string();
        let err = spec.check().expect_err("不认识的 kind 必须拒");
        assert!(err.contains("raytracing"), "{err}");
    }

    #[test]
    fn the_plan_keeps_the_array_order() {
        let mut spec = document(Vec::new(), vec![pass("a", &[], &["view"]), pass("b", &["view"], &["view"])]);
        spec.passes[0].label = "a".to_string();
        let plan = px_pass::Plan {
            layout: executor_layout(),
            resources: Vec::new(),
            passes: spec
                .passes
                .iter()
                .enumerate()
                .map(|(index, pass)| px_pass::PassPlan {
                    kind: px_pass::PassKind::Fullscreen,
                    label: pass.label_or(index),
                    shader: "x".to_string(),
                    entry: pass.entry.clone(),
                    reads: pass.reads.clone(),
                    writes: pass.writes.clone(),
                    params: vec![0; 16],
                    slots: (0..pass.reads.len()).map(|_| 1).collect(),
                    render: px_pass::RenderState::default(),
                    ..Default::default()
                })
                .collect(),
        };
        assert_eq!(plan.passes[0].label, "a");
        assert_eq!(plan.passes[1].label, "b");
        let writing = plan
            .passes
            .iter()
            .filter(|pass| pass.target() == Some(VIEW_BUILTIN))
            .count();
        assert_eq!(writing, 2);
        assert!(plan.check().is_ok(), "{:?}", plan.check());
    }

    /// 格位与 reads 对不上 = 绑错东西。这一条不该等到出图时才被发现。
    #[test]
    fn a_pass_whose_slots_do_not_match_its_reads_is_refused() {
        let mut plan = px_pass::Plan {
            layout: executor_layout(),
            resources: Vec::new(),
            passes: vec![px_pass::PassPlan {
                kind: px_pass::PassKind::Fullscreen,
                label: "grade".to_string(),
                shader: "x".to_string(),
                entry: "fs_main".to_string(),
                reads: vec![VIEW_BUILTIN.to_string()],
                writes: vec![VIEW_BUILTIN.to_string()],
                params: vec![0; 16],
                slots: Vec::new(),
                render: px_pass::RenderState::default(),
                ..Default::default()
            }],
        };
        let err = plan.check().expect_err("reads 有 1 个而格位 0 个 ⇒ 拒");
        assert!(err.contains("格位"), "{err}");
        plan.passes[0].slots = vec![4];
        let err = plan.check().expect_err("第 4 格不在布局里 ⇒ 拒");
        assert!(err.contains("布局"), "{err}");
        plan.passes[0].slots = vec![1];
        assert!(plan.check().is_ok(), "{:?}", plan.check());
    }

    /// 参数块必须按契约对齐：没打包过的空参数块不许悄悄编出一条管线。
    #[test]
    fn a_pass_without_a_packed_params_block_is_refused() {
        let mut plan = px_pass::Plan {
            layout: executor_layout(),
            resources: Vec::new(),
            passes: vec![px_pass::PassPlan {
                kind: px_pass::PassKind::Fullscreen,
                label: "grade".to_string(),
                shader: "x".to_string(),
                entry: "fs_main".to_string(),
                reads: Vec::new(),
                writes: vec![VIEW_BUILTIN.to_string()],
                params: Vec::new(),
                slots: Vec::new(),
                render: px_pass::RenderState::default(),
                ..Default::default()
            }],
        };
        let err = plan.check().expect_err("参数块为 0 字节 ⇒ 拒");
        assert!(err.contains("参数块"), "{err}");
        plan.passes[0].params = vec![0; 16];
        assert!(plan.check().is_ok(), "{:?}", plan.check());
    }

    #[test]
    fn a_compute_pass_is_refused_with_the_capability_reason() {
        let spec = document(Vec::new(), vec![pass("grade", &[], &["view"])]);
        let mut plan = px_pass::Plan {
            layout: executor_layout(),
            ..Default::default()
        };
        plan.passes.push(px_pass::PassPlan {
            kind: px_pass::PassKind::Compute,
            label: "reduce".to_string(),
            shader: "x".to_string(),
            entry: "cs_main".to_string(),
            reads: Vec::new(),
            writes: vec!["view".to_string()],
            params: vec![0; 16],
            slots: Vec::new(),
            // ⚠ 这一条测的是"执行器不兑现 compute"那条理由，所以附件要按 compute 该有的
            // 样子留空：挂了颜色附件会被**前一条**（"compute 不挂附件"）先拦下，
            // 而那一条的理由不一样（数据模型 vs 执行器能力），两条各有各的判据。
            render: px_pass::RenderState {
                color: px_pass::Attachment::None,
                ..Default::default()
            },
            ..Default::default()
        });
        let err = plan.check().expect_err("compute 这一版必须当场拒");
        assert!(err.contains("compute"), "{err}");
        assert!(err.contains("静默跳过"), "{err}");
        assert!(spec.check().is_ok());
    }
}
