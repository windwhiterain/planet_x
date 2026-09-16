//! 切片 1 的渲染路：**文档 → GPU → 回读 → PNG**（`art/15-render-wgpu.md` 的 S2 前半）。
//!
//! 这一档画出来的是"**背景 + 行星，别的什么都没有**"：
//!
//! - `prepass`（深度-only）与 `opaque`（清屏色 + 行星）真的执行；
//! - `blit` 把 `scene_color_a` 搬到内建目标 `view`（宿主自己的 `shot::Target`）；
//! - `sky` 与 `transparent` **不执行**，而且**大声说明**（见 [`SKIPPED`] 那段）。
//!
//! 三条口径，每一条都是"换个做法就会出另一种图，而门不会响"：
//!
//! 1. **`scene_depth` 由宿主建、当外部目标交出去**（`Role::Depth`）。文档把它声明成了
//!    `resources` 里的一张图，执行器本来会自己池化一张 —— 但大气的 group 0 binding 20 要
//!    采样**同一张**深度图，而那张 bind group 是宿主建的：池里那张的视图宿主**拿不到**。
//!    所以宿主给一份同名的外部目标（§130 的裁决：宿主的外部目标赢），并且**把顶掉这件事打印出来**
//!    （静默顶掉才是唯一不能接受的那种）。
//!    ⚠ 还有一条更硬的约束（本切片实测）：wgpu **不许**同一条 pass 里把一张图既当深度附件
//!    （写的附件）又当资源绑进绑定组 —— 所以"第 20 格"按物体分成两份：被**写深度**的 pass
//!    画的那些绑一张 1×1 占位深度图，只被**只读深度**的 pass 画的那些绑真图。
//!    每一份的归属都打印出来（见 `run` 里那几行审计）。
//! 2. **`view` 是宿主这一帧的最后目标**（`shot::Target` 的视图）：`blit` 的 `writes: ["view"]`
//!    指的就是它。判据要读回来的就是这张图，所以它绝不能是执行器池子里那张。
//! 3. **相机只从 `camera.rs` 来**（逐位对齐 oracle），`--width/--height` 只驱动视口与长宽比。
//!    这里不许再算一次投影：两条宿主在两套矩阵算法上分岔是逐字节判据最怕的漂移（§110.1.1）。

use std::path::Path;

use px_pass::{
    Attachment, Cull, Executor, External, Frame, PassKind, Plan, ResolvedGeometry, ResolvedGroup,
    ResolvedMaterial, Role,
};
use px_protocol::material::MATERIAL_BIND_GROUP;
use px_protocol::scene::CullMode;
use wgpu::util::{BufferInitDescriptor, DeviceExt};

use crate::art;
use crate::gpu::Gpu;
use crate::group0;
use crate::material::{self, Materials, FRAGMENT_ENTRY};
use crate::mat4::{Mat4, Quat, Vec3};
use crate::mesh::Mesh;
use crate::plan;
use crate::shader;
use crate::shot;

/// **这一档执行哪几条 pass**（`px_graphs::frame` 的那五个标签）。
///
/// ⚠ 按标签挑是**宿主的切片策略**，不是执行器的分派规则：`px_pass` 一个 pass 名字都不认识，
/// 它只按 `kind` 与状态分派（§124）。这一档之所以只能跑这三条，理由写在 [`SKIPPED`] 里，
/// 而且每一条都要**打出来**——"绿是因为跳过了它"是本仓库最不能接受的那种绿。
const EXECUTED: [&str; 3] = ["prepass", "opaque", "blit"];

/// 这一档**不执行**的那两条，以及各自的原因（原样进日志）。
const SKIPPED: [(&str, &str); 2] = [
    (
        "sky",
        "切片 2 的天空盒：它那一笔要的材质名 'skybox' 在文档里没有对应物体\
         （文档的物体只有 planet / atmosphere），宿主也没有天空盒的片元成员\
         （`art/shaders` 里没有 skybox.wgsl）⇒ 那一笔现在建不出来",
    ),
    (
        "transparent",
        "切片 3 的大气：本切片的判据是「背景 + 行星，别的什么都没有」，\
         执行它就没法把差异归因（§107：一次只动一个变量）",
    ),
];

/// 一份画好的图：紧凑 RGBA8 + 这一帧的审计文本。
///
/// ⚠ 不留目标纹理：回读在 [`run`] 里就做完了，"目标还在手上"是这一档用不到的余量。
pub struct Rendered {
    pub pixels: Vec<u8>,
    pub audit: Vec<String>,
    pub executed: Vec<String>,
    pub skipped: Vec<(String, String)>,
}

/// 一个物体在 GPU 上的几何：顶点/索引缓冲 + **产物的属性表**。
///
/// ⚠ 布局的字段得活到 `Frame` 之后（执行器借的是它），所以 `attributes` 与 `layout`
/// 都挂在这个结构体上，而不是在函数里现造一个临时的。
struct Geometry {
    name: String,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    attributes: Vec<wgpu::VertexAttribute>,
    vertex_count: u32,
    index_count: u32,
}

impl Geometry {
    fn layout(&self) -> wgpu::VertexBufferLayout<'_> {
        wgpu::VertexBufferLayout {
            array_stride: u64::from(Mesh::stride()),
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &self.attributes,
        }
    }
}

/// 一个物体的 group 1（`MeshStage`）：两块矩阵，每个物体一份。
///
/// ⚠ 布局不在这里：它是**一份**（见 `run` 里那段），所有物体共用 —— 每个物体各建一份
/// "内容相同、对象不同"的布局会让管线缓存键与 wgpu 的布局比对说不到一块去。
struct Stage {
    _buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

/// 一块 `MeshStage` 的字节：`world_from_local` + `view_proj`。
///
/// ⚠ 两块矩阵都走 `mat4.rs` 那几份**逐位**移植件：`world_from_local` 用
/// `from_scale_rotation_translation`（文档的 `transform` 就是它的三个入参），
/// `view_proj` 用相机自己算好的 `clip_from_world`（= `clip_from_view × view_from_world`）。
/// 在这里重算一次 `clip_from_view × view_from_world` 就是"同一条契约、两处算"。
fn stage_bytes(transform: &px_protocol::scene::Transform, camera: &crate::camera::Camera) -> [u8; 128] {
    let rotation = Quat::from_xyzw(
        transform.rotation[0],
        transform.rotation[1],
        transform.rotation[2],
        transform.rotation[3],
    );
    let world_from_local = Mat4::from_scale_rotation_translation(
        Vec3::new(
            transform.scale[0],
            transform.scale[1],
            transform.scale[2],
        ),
        rotation,
        Vec3::new(
            transform.translation[0],
            transform.translation[1],
            transform.translation[2],
        ),
    );
    let mut bytes = [0_u8; 128];
    for (index, column) in [
        world_from_local.x_axis,
        world_from_local.y_axis,
        world_from_local.z_axis,
        world_from_local.w_axis,
        camera.clip_from_world.x_axis,
        camera.clip_from_world.y_axis,
        camera.clip_from_world.z_axis,
        camera.clip_from_world.w_axis,
    ]
    .iter()
    .enumerate()
    {
        for (slot, value) in [column.x, column.y, column.z, column.w].iter().enumerate() {
            let at = index * 16 + slot * 4;
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
    bytes
}

/// 三档剔除：文档的 `CullMode` → 执行器的 `Cull`。
///
/// ⚠ 三个词**逐字相同**（`none` / `front` / `back`，见 `px_protocol::scene::CullMode` 的 serde
/// 名字与 `px_pass::Cull::parse`），但这是**两个类型**之间的翻译，不是一份规则的副本：
/// 取值集合由两边各自保证，这里只搬一格。
fn cull_of(cull: CullMode) -> Cull {
    match cull {
        CullMode::Back => Cull::Back,
        CullMode::Front => Cull::Front,
        CullMode::None => Cull::None,
    }
}

/// 跑这一帧。
pub fn run(
    gpu: &Gpu,
    scene_path: &Path,
    width: u32,
    height: u32,
) -> Result<Rendered, String> {
    let root = art::default_pcg_root();
    let spec = art::read_spec(scene_path)?;
    let scene = art::load_scene(scene_path, &root)?;
    let mut audit: Vec<String> = Vec::new();
    audit.push(scene.audit());

    // ---- 计划：五条 pass 全部翻出来并 `check()`，一条都不跳 ----
    let plan = plan::build(&spec, &root)?;
    audit.push(format!(
        "计划：{} 个资源 / {} 条 pass（全部解析自文档，`px_pass::Plan::check()` 已过）",
        plan.resources.len(),
        plan.passes.len()
    ));
    for (index, pass) in plan.passes.iter().enumerate() {
        audit.push(format!(
            "  [{index}] '{}'｜{}｜写 [{}]｜深度目标 {}｜读 [{}]｜draws {}｜状态 {}",
            pass.label,
            pass.kind.name(),
            pass.writes.join(" / "),
            pass.depth_target.as_deref().unwrap_or("（不挂）"),
            pass.reads.join(" / "),
            pass.draws
                .iter()
                .map(|draw| if draw.material.is_empty() {
                    draw.geometry.clone()
                } else {
                    format!("{}+{}", draw.geometry, draw.material)
                })
                .collect::<Vec<_>>()
                .join(" / "),
            pass.render.name()
        ));
    }
    let executed_plan = subset(&plan, &mut audit)?;

    // ---- 相机：`camera.rs` 原样（逐位对齐 oracle），只有长宽比来自命令行 ----
    let camera = crate::camera::probe_camera(None, width as f32 / height as f32);
    audit.push(format!(
        "相机：from_xyz({}, {}, {}).looking_at(ZERO, Y)｜aspect {}（位模式 {:08X}）｜无限 reverse-Z / Depth32Float / 清 0.0 / GreaterEqual",
        camera.position.x,
        camera.position.y,
        camera.position.z,
        width as f32 / height as f32,
        (width as f32 / height as f32).to_bits()
    ));

    // ---- 深度图：**宿主自己建**，等会儿当外部目标交出去 ----
    //
    // ⚠ 另外建一张 1×1 的**占位深度图**：wgpu 不许同一条 pass 里既把一张图当深度附件**写**、
    //    又把它当资源绑进绑定组（实测文本：`TextureUses(DEPTH_STENCIL_WRITE) is an exclusive
    //    usage and cannot be used with any other usages within the usage scope`）——
    //    而 group 0 的第 20 格（`depth_prepass_texture`）**必须**有个视图。写深度的那几条
    //    pass 因此绑占位图：反正这一档没有任何一条 shader 采样它（surface 不声明那一格）。
    //    ⚠ 只读深度的那几条（`depth_write = false`，切片 3 的天空/透明）**可以**绑真图 ——
    //    `DEPTH_STENCIL_READ | RESOURCE` 两种都是"包容用法"，实测不冲突。
    let depth = create_depth(&gpu.device, width, height, "scene_depth（宿主建的，顶掉文档声明的池资源）");
    let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
    let placeholder = create_depth(&gpu.device, 1, 1, "group 0 第 20 格的占位深度图（1×1）");
    let placeholder_view = placeholder.create_view(&wgpu::TextureViewDescriptor::default());

    // ---- 每个物体该配哪一份 group 0：看**画它的那几条执行的 pass** 写不写深度 ----
    //
    // ⚠ 一条物体要是既被"写深度"的 pass 画、又被"只读深度"的 pass 画，**没有一份** group 0
    //    能同时满足两边（第 20 格只能有一个视图）⇒ 当场拒，并说清该怎么改（执行器的
    //    `Frame::materials` 是按名字查的**一张平表**，而这一格随 pass 变）。
    let contract = scene
        .objects
        .first()
        .ok_or_else(|| "场景里一个物体都没有：拿不到一份可反射的 shader".to_string())?;
    let contract_module = shader::validate(
        &format!("{}（group 0 契约）", contract.id),
        &contract.shader.assembled,
    )?;
    let build_zero = |view: &wgpu::TextureView| -> Result<group0::GroupZero, String> {
        group0::frame(
            &gpu.device,
            &contract_module,
            &camera,
            scene.ambient,
            width,
            height,
            view,
        )
    };

    let mut needs_writing: Vec<&str> = Vec::new();
    let mut needs_reading: Vec<&str> = Vec::new();
    let mut undrawn: Vec<&str> = Vec::new();
    // ⚠ 第 20 格（`depth_prepass_texture`）的守卫。两条事实合起来才是这条守卫：
    //    ① wgpu **不许**同一条 pass 里把一张图既当（写的）深度附件、又当资源绑进绑定组；
    //    ② 所以"挂深度附件的 pass"里，第 20 格只能绑**别的东西**（那张 1×1 占位图）。
    //    ⇒ 谁的片元阶段**真的读**第 20 格，谁就不能被"挂深度附件"的 pass 画：
    //       绑占位图 ⇒ 读到的是 0（**像素是错的**，而且一声不吭）；
    //       绑真图   ⇒ wgpu 当场拒（报错文本见 §131）。
    //    正确形状是**先拷贝一次**：`prepass` 写 `scene_depth` → 一步 `copy` 出
    //    `scene_depth_sample` → 主 pass 照旧挂 `scene_depth`（对真的预通道结果做深度测试），
    //    只有采样那一方读 `scene_depth_sample`。⚠ 那一步拷贝**属于帧表**（谁提供未定），
    //    宿主不许自己发一次 `copy_texture_to_texture` —— 那等于宿主又开始决定帧序。
    //    这一条就是让这件事在切片 3 一开工就响的东西。
    let depth_binding = group0::DEPTH_PREPASS_BINDING;
    let mut sampling: Vec<&str> = Vec::new();
    for object in &scene.objects {
        let module = shader::validate(&object.id, &object.shader.assembled)?;
        let samples_depth = shader::bindings(&module).iter().any(|(group, binding, _, _)| {
            (*group, *binding) == depth_binding
        });
        if !samples_depth {
            continue;
        }
        sampling.push(object.id.as_str());
        for pass in &executed_plan.passes {
            if !pass.draws.iter().any(|draw| draw.material == object.id) {
                continue;
            }
            if pass.render.depth == Attachment::None {
                continue;
            }
            return Err(format!(
                "物体 '{}' 的片元阶段声明了 @group({}) @binding({})（depth_prepass_texture），\
                 而 pass '{}' 把 '{}' 当深度附件挂着。wgpu 不许同一条 pass 里既把这张图当附件、\
                 又当资源绑进绑定组（实测：`TextureUses(DEPTH_STENCIL_WRITE) is an exclusive usage`），\
                 所以这一笔**没有**正确的绑法：绑真图 ⇒ 建不出这条 pass；绑占位图 ⇒ 读到 0，\
                 像素是错的且不会报错。正确形状是**帧表里先拷贝一次**\
                 （prepass 写 scene_depth → copy 出 scene_depth_sample → 主 pass 仍挂 scene_depth、\
                 采样那一方读 scene_depth_sample）。⚠ 拷贝那一步属于帧表，宿主不自己插",
                object.id,
                depth_binding.0,
                depth_binding.1,
                pass.label,
                pass.depth_target.as_deref().unwrap_or("（没写 depth_target）")
            ));
        }
    }
    for object in &scene.objects {
        let drawn_by: Vec<&str> = executed_plan
            .passes
            .iter()
            .filter(|pass| pass.draws.iter().any(|draw| draw.material == object.id))
            .filter(|pass| pass.render.depth != Attachment::None)
            .map(|pass| pass.label.as_str())
            .collect();
        let writes = executed_plan
            .passes
            .iter()
            .filter(|pass| pass.draws.iter().any(|draw| draw.material == object.id))
            .any(|pass| pass.render.depth != Attachment::None && pass.render.depth_write);
        let reads = executed_plan
            .passes
            .iter()
            .filter(|pass| pass.draws.iter().any(|draw| draw.material == object.id))
            .any(|pass| pass.render.depth != Attachment::None && !pass.render.depth_write);
        if writes && reads {
            return Err(format!(
                "物体 '{}' 同时被「写深度」与「只读深度」的 pass 画（{}）：第 20 格只能有一个视图，\
                 没有一份 group 0 能同时配两边。要么把这一档的物体分开（各画各的 pass），\
                 要么让执行器按**每一条 pass** 解析材质（现在 `Frame::materials` 是按名字查的一张平表）",
                object.id,
                drawn_by.join(" / ")
            ));
        }
        if writes {
            needs_writing.push(object.id.as_str());
        } else if reads {
            needs_reading.push(object.id.as_str());
        } else {
            // 这一档**没有执行的 pass** 画它（例如切片 1 里的大气：画它的 `transparent` 没执行）
            // ⇒ 第 20 格随便配一份，反正这一帧用不到。**说出来**，不许悄悄挑一个。
            undrawn.push(object.id.as_str());
        }
        audit.push(format!(
            "  物体 '{}' 的 group 0 第 20 格 ⇒ {}{}",
            object.id,
            if writes {
                "1×1 占位深度图（它被「写深度」的 pass 画）"
            } else if reads {
                "真的 scene_depth（它只被「只读深度」的 pass 画）"
            } else {
                "随便一份（这一档没有执行的 pass 画它）"
            },
            // 占位图**为什么无害**：这个物体的片元阶段声明里根本没有第 20 格 ⇒ 它不会被读到。
            // （真的有人要读它时，上面那条守卫会先响。）
            if sampling.contains(&object.id.as_str()) {
                "；⚠ 它的片元阶段**声明了**第 20 格 ⇒ 见上面那条守卫"
            } else {
                "；它的片元阶段没有声明第 20 格 ⇒ 占位图不会被读到"
            }
        ));
    }
    let zero_writing = if needs_writing.is_empty() {
        None
    } else {
        Some(build_zero(&placeholder_view)?)
    };
    let mut zero_reading = if needs_reading.is_empty() {
        None
    } else {
        Some(build_zero(&depth_view)?)
    };
    if zero_writing.is_none() && zero_reading.is_none() {
        // 一条挂深度的 pass 都没有（当前帧表不会这样，但"没有"也得能建出来）：
        // 第 20 格绑真图那份，因为没有任何 pass 会写它。
        zero_reading = Some(build_zero(&depth_view)?);
    }
    audit.push("group 0（五格全绑，哪怕 shader 只声明了一部分）：".to_string());
    let described = zero_reading.as_ref().or(zero_writing.as_ref()).expect("至少有一份");
    audit.extend(described.audit.iter().map(|line| format!("  {line}")));
    audit.push(format!(
        "  ⚠ 第 20 格（depth_prepass_texture）建了 {} 份：写深度的那些物体 [{}] 绑 1×1 占位图，\
         只读深度的那些 [{}] 绑真的 scene_depth —— 同一张图不能在同一条 pass 里既当写的附件又当资源",
        usize::from(zero_writing.is_some()) + usize::from(zero_reading.is_some()),
        if needs_writing.is_empty() { "（无）".to_string() } else { needs_writing.join(" / ") },
        if needs_reading.is_empty() { "（无）".to_string() } else { needs_reading.join(" / ") }
    ));
    if !undrawn.is_empty() {
        audit.push(format!(
            "  ⚠ 这一档没有任何执行的 pass 画这些物体：[{}] ⇒ 它们的 group 0 第 20 格随便配一份\
             （本帧用不到）。它们在文档里，只是画它们的 pass 没执行",
            undrawn.join(" / ")
        ));
    }

    // ---- 几何：按物体 id 上传（`Draw::geometry` 那个名字就是物体 id）----
    let mut geometries: Vec<Geometry> = Vec::with_capacity(scene.objects.len());
    for object in &scene.objects {
        let mesh = object.geometry.mesh();
        let bytes = mesh.interleaved().map_err(|err| {
            format!("物体 '{}'（{}）的顶点流：{err}", object.id, object.geometry.describe())
        })?;
        let index_bytes: Vec<u8> = mesh
            .indices
            .iter()
            .flat_map(|index| index.to_le_bytes())
            .collect();
        geometries.push(Geometry {
            name: object.id.clone(),
            vertices: gpu.device.create_buffer_init(&BufferInitDescriptor {
                label: Some("顶点（交错，布局来自产物属性表）"),
                usage: wgpu::BufferUsages::VERTEX,
                contents: &bytes,
            }),
            indices: gpu.device.create_buffer_init(&BufferInitDescriptor {
                label: Some("索引"),
                usage: wgpu::BufferUsages::INDEX,
                contents: &index_bytes,
            }),
            attributes: Mesh::vertex_attributes(),
            vertex_count: mesh.vertex_count() as u32,
            index_count: mesh.indices.len() as u32,
        });
        audit.push(format!(
            "几何 '{}'：{} 顶点 / {} 三角形｜顶点流 {} 字节（stride {}，属性 {:?}）｜索引 {} 字节",
            object.id,
            mesh.vertex_count(),
            mesh.triangle_count(),
            bytes.len(),
            Mesh::stride(),
            Mesh::attributes()
                .iter()
                .map(|(location, format, from)| format!("@{location}={format:?}（{from}）"))
                .collect::<Vec<_>>(),
            index_bytes.len()
        ));
    }

    // ---- 顶点阶段 ↔ 产物属性表：**当场对账** ----
    //
    // ⚠ 反射的是**文档里那段顶点阶段**（不是"我们以为它要什么"）：它声明的每一个
    //    `@location(n)` 都必须落在产物给的属性表里，而且类型一致。错位在画面上只是
    //    "某几个属性读成了别的格子"，任何门都不会响。
    for pass in &executed_plan.passes {
        if pass.kind != PassKind::Geometry {
            continue;
        }
        let module = shader::validate(
            &format!("pass '{}' 的顶点阶段", pass.label),
            &pass.vertex_shader,
        )?;
        for draw in &pass.draws {
            let geometry = geometries
                .iter()
                .find(|geometry| geometry.name == draw.geometry)
                .ok_or_else(|| {
                    format!(
                        "pass '{}' 要画几何 '{}'，而宿主这一档没给这个名字（给了：{}）",
                        pass.label,
                        draw.geometry,
                        geometries
                            .iter()
                            .map(|geometry| geometry.name.as_str())
                            .collect::<Vec<_>>()
                            .join(" / ")
                    )
                })?;
            check_vertex_inputs(&module, &pass.vertex_entry, &pass.label, geometry)?;
        }
        audit.push(format!(
            "顶点阶段（pass '{}'）的输入与产物属性表对得上：{}",
            pass.label,
            pass.vertex_entry
        ));
    }

    // ---- 材质：group 3 由 `material.rs` 反射建（空槽绑兜底白图），group 1 每个物体一份 ----
    let mut materials = Materials::new(&gpu.device, &gpu.queue);
    let material_layout = materials.bind_group_layout().clone();
    // ⚠ group 1 的**布局只有一份**（所有物体共用同一份 `MeshStage` 形状）：每个物体各建一份
    //    "内容相同但对象不同"的布局，会踩到 `ResolvedGroup::layout_id` 那条契约的边界 ——
    //    管线缓存键相同、而 wgpu 那边比的是布局对象本身。一份布局 + 每个物体一个绑定组，
    //    这两件事就都干净了。
    let stage_layout = gpu
        .device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("组 1：MeshStage"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
    let mut stages: Vec<Stage> = Vec::with_capacity(scene.objects.len());
    let mut bindings = Vec::with_capacity(scene.objects.len());
    for object in &scene.objects {
        let binding = materials.bind(&gpu.device, &gpu.queue, object)?;
        audit.push(format!(
            "材质 '{}'：shader {}｜参数 {} 字节｜绑上的格 {:?}｜走兜底白图的格 {:?}（布局是 12 格超集）",
            object.id,
            object.shader.member,
            object.params.len(),
            binding.bound_slots,
            binding.fallback_slots
        ));
        let buffer = gpu
            .device
            .create_buffer_init(&BufferInitDescriptor {
                label: Some("组 1：MeshStage（world_from_local + view_proj）"),
                usage: wgpu::BufferUsages::UNIFORM,
                contents: &stage_bytes(&object.transform, &camera),
            });
        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("组 1：MeshStage"),
            layout: &stage_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });
        stages.push(Stage {
            _buffer: buffer,
            bind_group,
        });
        bindings.push(binding);
    }

    // ---- 执行器要的那几张表 ----
    let resolved_geometry: Vec<ResolvedGeometry<'_>> = geometries
        .iter()
        .map(|geometry| ResolvedGeometry {
            name: geometry.name.as_str(),
            vertices: Some((&geometry.vertices, geometry.layout())),
            indices: Some((
                &geometry.indices,
                wgpu::IndexFormat::Uint32,
                geometry.index_count,
            )),
            vertex_count: geometry.vertex_count,
        })
        .collect();

    // ⚠ 一笔 draw 的组：**group 0 + group 1 + group 3**。group 1 是**每个物体**的
    //    （两块矩阵），所以"解析好的材质"其实按**物体**给：名字就是物体 id，
    //    而文档里一笔 draw 的 geometry 与 material 用的正是同一个名字（`frame.rs::draws_of`）。
    //    真出现"一条 pass 里同一个物体画两笔"的那天，这里要当场拒 —— 一个名字给不出两组矩阵。
    let resolved_materials: Vec<ResolvedMaterial<'_>> = scene
        .objects
        .iter()
        .zip(bindings.iter())
        .zip(stages.iter())
        .map(|((object, binding), stage)| {
            // 这个物体配哪一份 group 0（写深度那份绑占位图，只读那份绑真图，没被画到的随便一份）。
            let zero = if needs_writing.contains(&object.id.as_str()) {
                zero_writing.as_ref()
            } else {
                zero_reading.as_ref()
            }
            .or(zero_writing.as_ref())
            .or(zero_reading.as_ref())
            .expect("至少建过一份：上面有一条兜底");
            Ok(ResolvedMaterial {
                name: object.id.as_str(),
                groups: vec![
                    ResolvedGroup {
                        group: 0,
                        bind_group: &zero.bind_group,
                        layout: zero.layout.clone(),
                        // 契约：同布局同 id、异布局异 id。宿主每种布局只有一份 ⇒ 三个常数。
                        layout_id: 0,
                    },
                    ResolvedGroup {
                        group: 1,
                        bind_group: &stage.bind_group,
                        // 一份布局，所有物体共用（见上面那段）。
                        layout: stage_layout.clone(),
                        layout_id: 1,
                    },
                    ResolvedGroup {
                        group: MATERIAL_BIND_GROUP,
                        bind_group: &binding.bind_group,
                        layout: material_layout.clone(),
                        layout_id: 2,
                    },
                ],
                // 混合档来自**材质契约**那一份（`Add` 与 `Premultiplied` 在 Bevy 0.19 里同档）。
                blend: material::key_of(object)?.blend(),
                cull: cull_of(object.cull),
                // 片元阶段：**材质就是那支 shader**（§129），组装好的全文（不是成员名）。
                fragment_shader: object.shader.assembled.as_str(),
                fragment_entry: FRAGMENT_ENTRY,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    // ---- 外部目标：深度图与最终目标 ----
    let target = shot::Target::new(&gpu.device, width, height);
    let mut sets: Vec<Vec<External<'_>>> = Vec::with_capacity(executed_plan.passes.len());
    let mut depth_overrides: Vec<String> = Vec::new();
    for (index, pass) in executed_plan.passes.iter().enumerate() {
        let mut set: Vec<External<'_>> = Vec::new();
        if let Some(name) = pass.depth_target.as_deref() {
            if plan.resource(name).is_some() {
                // ⚠ **宿主顶掉文档声明的资源**（§130）：打印出来是硬要求，静默顶掉不行。
                depth_overrides.push(format!(
                    "[{index}] '{}' 的 Role::Depth '{name}'",
                    pass.label
                ));
                set.push(External {
                    name,
                    role: Role::Depth,
                    view: &depth_view,
                    format: wgpu::TextureFormat::Depth32Float,
                });
            }
        }
        if pass.writes.first().map(String::as_str) == Some(px_protocol::scene::VIEW_BUILTIN) {
            set.push(External {
                name: px_protocol::scene::VIEW_BUILTIN,
                role: Role::Write,
                view: &target.view,
                format: shot::FORMAT,
            });
        }
        sets.push(set);
    }
    if !depth_overrides.is_empty() {
        audit.push(format!(
            "⚠ 顶掉文档声明的资源：'scene_depth' 由宿主这一帧自己建、以 Role::Depth 交出去（{}）。\
             原因：大气的 group 0 binding 20 要采样**同一张**深度图，而池里那张的视图宿主拿不到（§130）",
            depth_overrides.join(" / ")
        ));
    }

    let frame = Frame {
        width,
        height,
        sets: &sets,
        geometries: &resolved_geometry,
        materials: &resolved_materials,
    };
    let mut executor = Executor::new();
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("切片 1 的一帧"),
        });
    let audit_text = executor.execute(&gpu.device, &mut encoder, &executed_plan, &frame)?;
    gpu.queue.submit(Some(encoder.finish()));
    audit.push(format!("执行器审计：\n{audit_text}"));

    let pixels = shot::read_back(&gpu.device, &gpu.queue, &target)?;
    Ok(Rendered {
        pixels,
        audit,
        executed: executed_plan
            .passes
            .iter()
            .map(|pass| pass.label.clone())
            .collect(),
        skipped: SKIPPED
            .iter()
            .map(|(label, why)| (label.to_string(), why.to_string()))
            .collect(),
    })
}

/// 按 [`EXECUTED`] 把计划切成"这一档真的交给执行器的那一份"，并把**没执行的**打出来。
///
/// ⚠ 切出来的那一份仍然是 `px_pass::Plan`：资源表、布局、`PassPlan` 全都是原来的那几份，
/// 只是少了没执行的两条 —— 执行器看不到"切片"这回事，它只看到一张三行的 pass 表。
fn subset(plan: &Plan, audit: &mut Vec<String>) -> Result<Plan, String> {
    let mut passes = Vec::with_capacity(EXECUTED.len());
    for label in EXECUTED {
        let pass = plan
            .passes
            .iter()
            .find(|pass| pass.label == *label)
            .ok_or_else(|| {
                format!(
                    "这一档要执行 '{label}'，而文档里没有这个标签的 pass（文档有：{}）",
                    plan.passes
                        .iter()
                        .map(|pass| pass.label.as_str())
                        .collect::<Vec<_>>()
                        .join(" / ")
                )
            })?;
        passes.push(pass.clone());
    }
    let not_executed: Vec<&str> = plan
        .passes
        .iter()
        .filter(|pass| !EXECUTED.contains(&pass.label.as_str()))
        .map(|pass| pass.label.as_str())
        .collect();
    audit.push(format!(
        "本切片执行的 pass：{}（{} / {}）",
        EXECUTED.join(" → "),
        EXECUTED.len(),
        plan.passes.len()
    ));
    if !not_executed.is_empty() {
        audit.push(format!(
            "⚠ 本切片**没有执行**这 {} 条 pass（它们仍在计划里、也过了 `check()`，只是没交给执行器）：",
            not_executed.len()
        ));
        for label in not_executed {
            let why = SKIPPED
                .iter()
                .find(|(skipped, _)| *skipped == label)
                .map(|(_, why)| *why)
                .unwrap_or("（没有登记原因：这是宿主自己的疏漏，不是文档的问题）");
            audit.push(format!("    '{label}'：{why}"));
        }
    }
    let subset = Plan {
        layout: plan.layout.clone(),
        resources: plan.resources.clone(),
        passes,
    };
    subset
        .check()
        .map_err(|err| format!("切给执行器的那一份计划说不通：{err}"))?;
    Ok(subset)
}

/// `width × height` 的 `Depth32Float` 图：无限 reverse-Z 只有这一种深度格式（§110.1）。
fn create_depth(device: &wgpu::Device, width: u32, height: u32, label: &str) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

/// 顶点阶段声明的**每一个输入**都必须落在产物的属性表里（位置/法线/uv）。
///
/// ⚠ 这条判据是 §131 那次"顶点阶段只写 position、而片元读三个 location"的**同一条推理的
/// 另一半**：错位 / 缺属性在画面上只是"某几个属性读成了别的格子"，而任何门都不会响。
/// 反射的是**文档里那段顶点阶段**，不是"我们以为它要什么"。
fn check_vertex_inputs(
    module: &naga::Module,
    entry_name: &str,
    label: &str,
    geometry: &Geometry,
) -> Result<(), String> {
    let entry = module
        .entry_points
        .iter()
        .find(|entry| entry.name == entry_name && entry.stage == naga::ShaderStage::Vertex)
        .ok_or_else(|| {
            format!(
                "pass '{label}' 的顶点阶段里没有入口 '{entry_name}'（它有的入口：{}）",
                module
                    .entry_points
                    .iter()
                    .map(|entry| entry.name.as_str())
                    .collect::<Vec<_>>()
                    .join(" / ")
            )
        })?;
    for argument in &entry.function.arguments {
        let Some(naga::Binding::Location { location, .. }) = argument.binding else {
            continue;
        };
        let format = vertex_format(module, argument.ty).ok_or_else(|| {
            format!(
                "pass '{label}' 的顶点阶段在 location({location}) 上要的类型这一版认不出来：{:?}",
                module.types[argument.ty].inner
            )
        })?;
        let found = geometry
            .attributes
            .iter()
            .find(|attribute| attribute.shader_location == location);
        match found {
            Some(attribute) if attribute.format == format => {}
            Some(attribute) => {
                return Err(format!(
                    "pass '{label}' 的顶点阶段在 location({location}) 上要 {format:?}，\
                     而几何 '{}' 在那里给的是 {:?}：属性表与顶点阶段说的不是一件事",
                    geometry.name, attribute.format
                ))
            }
            None => {
                return Err(format!(
                    "pass '{label}' 的顶点阶段要 location({location})（{format:?}），\
                     而几何 '{}' 的属性表只有 [{}]：产物没带这个属性",
                    geometry.name,
                    geometry
                        .attributes
                        .iter()
                        .map(|attribute| format!(
                            "{}={:?}",
                            attribute.shader_location, attribute.format
                        ))
                        .collect::<Vec<_>>()
                        .join(" / ")
                ))
            }
        }
    }
    Ok(())
}

/// naga 的类型 → 顶点属性格式。只认产物真会带的那几种（`f32` / `vec2<f32>` / `vec3<f32>`），
/// 其余一律 `None` ⇒ 调用方当场报错（"认不出来"必须看得见，不许默默跳过这一格）。
fn vertex_format(module: &naga::Module, ty: naga::Handle<naga::Type>) -> Option<wgpu::VertexFormat> {
    use naga::{ScalarKind, TypeInner, VectorSize};
    match &module.types[ty].inner {
        TypeInner::Scalar(scalar) => match (scalar.kind, scalar.width) {
            (ScalarKind::Float, 4) => Some(wgpu::VertexFormat::Float32),
            (ScalarKind::Uint, 4) => Some(wgpu::VertexFormat::Uint32),
            (ScalarKind::Sint, 4) => Some(wgpu::VertexFormat::Sint32),
            _ => None,
        },
        TypeInner::Vector { size, scalar } => match (scalar.kind, scalar.width, size) {
            (ScalarKind::Float, 4, VectorSize::Bi) => Some(wgpu::VertexFormat::Float32x2),
            (ScalarKind::Float, 4, VectorSize::Tri) => Some(wgpu::VertexFormat::Float32x3),
            (ScalarKind::Float, 4, VectorSize::Quad) => Some(wgpu::VertexFormat::Float32x4),
            _ => None,
        },
        _ => None,
    }
}
