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
    Attachment, Cull, External, Frame, PassKind, Plan, ResolvedGeometry, ResolvedGroup,
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
const EXECUTED: [&str; 5] = [
    "prepass",
    "copy_depth",
    "opaque",
    "transparent",
    "blit",
];

/// 这一档**不执行**的那几条，以及各自的原因（原样进日志）。
const SKIPPED: [(&str, &str); 1] = [(
    "sky",
    "切片 2 的天空盒：它那一笔要的材质名 'skybox' 在文档里没有对应物体\
     （文档的物体只有 planet / atmosphere），宿主也没有天空盒的片元成员\
     （`art/shaders` 里没有 skybox.wgsl）⇒ 那一笔现在建不出来。\
     ⚠ 与『剪影外还剩星空』是**两件事**：星空是**切片 2 的素材**（环境里那份 `generated/stars`，\
     不是这一笔能解决的），而天空盒的**顶点阶段对不上**（`art/frame/vertex_sky.wgsl` 只写 \
     `@builtin(position)`）是另一处的活 —— 归因别糊在一起",
)];

/// 这一档由**宿主建、seed 进池子**的两张深度图（§132）。
///
/// ⚠ 两张都要：`scene_depth` 是主 pass 的深度附件（写），`scene_depth_sample` 是那次拷贝的
/// 目标、也是第 20 格采样的那一张。只 seed 一张，另一张就会落到池子自建的那张上 ——
/// 两张同名纹理，而错法是**一声不吭的错像素**。
const DEPTH_RESOURCES: [&str; 2] = ["scene_depth", "scene_depth_sample"];

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

    // ---- 执行器：**先建**，因为深度图要先 `seed` 进去（§132）----
    let mut executor = px_pass::Executor::new();

    // ---- group 0 的契约：从**某一份物体 shader** 反射（五格超集的那份布局）----
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

    // ---- 深度图：**宿主建、seed 进池子**（§132 定下的形状）----
    //
    // ⚠ 一个资源名只能有**一张**纹理。拷贝那条路要的是**纹理**（`copy_texture_to_texture`），
    //    而 group 0 的第 20 格要的是**同一张纹理的视图** —— 两边各自建一张，就是
    //    "copy 写池里那张、着色器读宿主那张"这种**一声不吭的错像素**。
    //    所以两张深度图都由宿主建，`Executor::seed` 把它们按文档声明的规格收进池子：
    //    从那一刻起，附件、绑定、拷贝用的都是同一张。
    //
    // ⚠ **两张都要 seed**：只 seed 拷贝的目标，源就会落到池子自建的那张上，
    //    而附件用的是宿主那张 —— 又是两张同名纹理、又是静默错像素。
    let mut seeded: Vec<(String, wgpu::Texture)> = Vec::new();
    for name in DEPTH_RESOURCES {
        let Some(resource) = plan.resource(name) else {
            // 老文档（没有帧表）没有这两栏：这一档只在有帧表的产物上跑，缺了就当场说清。
            return Err(format!(
                "文档里没有资源 '{name}'（声明了的：{}）：这一档要把它 seed 成宿主建的那张深度图",
                plan.resources
                    .iter()
                    .map(|resource| resource.name.as_str())
                    .collect::<Vec<_>>()
                    .join(" / ")
            ));
        };
        let (depth_width, depth_height) = resource.size.resolve(width, height);
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(name),
            size: wgpu::Extent3d {
                width: depth_width,
                height: depth_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            // 用途照**文档声明的**来（映射只有一份，住在 `px_pass::texture_usage`）。
            usage: px_pass::texture_usage(resource),
            view_formats: &[],
        });
        executor.seed(resource, depth_width, depth_height, texture.clone())?;
        audit.push(format!(
            "  ⚠ 池子里的 '{name}' 现在就是**宿主建的**这一张（{}×{} {:?}，用途照文档声明）：\
             谁被顶掉了要看得见 —— 不 seed 的话 copy 与绑定会各拿一张同名纹理，\
             而那种错法是**一声不吭的错像素**",
            depth_width,
            depth_height,
            texture.format()
        ));
        seeded.push((name.to_string(), texture));
    }
    let depth_texture = |name: &str| -> Option<&wgpu::Texture> {
        seeded
            .iter()
            .find(|(seeded_name, _)| seeded_name == name)
            .map(|(_, texture)| texture)
    };
    // group 0 第 20 格（`depth_prepass_texture`）绑哪张图：**从帧表里推**，不靠名字约定。
    //
    // 规则：看哪条 `copy` 把"深度附件用的那张图"搬到了别处 —— 搬出来的那一份就是
    // "**当时**那份预通道深度"，而着色器要的正是它（它自己那条 pass 挂着的是同一张图的
    // "现在"这份）。没有那条 copy 时退回深度附件本身。两条都没有就建不出来 ⇒ 说清楚。
    let depth_target_name = executed_plan
        .passes
        .iter()
        .find_map(|pass| match pass.render.depth {
            Attachment::None => None,
            _ => pass.depth_target.as_deref(),
        })
        .ok_or_else(|| {
            "这一档的计划里没有一条 pass 挂深度附件：group 0 的第 20 格就没有来源了".to_string()
        })?;
    let sampled_depth = executed_plan
        .passes
        .iter()
        .find(|pass| pass.kind == PassKind::Copy && pass.reads.first().map(String::as_str) == Some(depth_target_name))
        .and_then(|pass| pass.writes.first().cloned())
        .unwrap_or_else(|| depth_target_name.to_string());
    let sampled_view = depth_texture(&sampled_depth)
        .ok_or_else(|| {
            format!(
                "group 0 第 20 格该绑 '{sampled_depth}'，而宿主这一档只 seed 了 [{}]",
                seeded
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect::<Vec<_>>()
                    .join(" / ")
            )
        })?
        .create_view(&wgpu::TextureViewDescriptor::default());
    audit.push(format!(
        "group 0 第 20 格（depth_prepass_texture）⇒ '{sampled_depth}'（从帧表推出来的：\
         深度附件用 '{depth_target_name}'，而那条 copy 把它搬成了 '{sampled_depth}'）"
    ));

    // ⚠ 第 20 格的守卫（**拦一类形状**，不是拦那件事）：同一条 pass 里，一张图不能
    //    既当（写的）深度附件、又当资源绑进绑定组（wgpu：`TextureUses(DEPTH_STENCIL_WRITE)
    //    is an exclusive usage`）。这条守卫从"拦占位深度那个事件"变成了"拦这一类"：
    //    谁哪天把第 20 格指回它自己挂着的那张深度图（例如去掉那次 copy），这里当场响，
    //    并列出三条出路 —— 它比那件事活得久（§132）。
    for pass in &executed_plan.passes {
        if pass.render.depth == Attachment::None {
            continue;
        }
        if pass.depth_target.as_deref() != Some(sampled_depth.as_str()) {
            continue;
        }
        return Err(format!(
            "pass '{}' 把 '{sampled_depth}' 当深度附件挂着，而 group 0 的第 20 格\
             （depth_prepass_texture）绑的**也是它**：wgpu 不许同一条 pass 里把一张图既当附件、\
             又当资源绑进绑定组（实测：`TextureUses(DEPTH_STENCIL_WRITE) is an exclusive usage \
             and cannot be used with any other usages within the usage scope`）。\
             三条出路：① 帧表里**先拷贝一次**（深度 → 快照，采样那一方读快照；这就是这一档的形状）；\
             ② 那条 pass 不挂深度附件（丢掉遮挡测试）；③ 让执行器按**每一条 pass** 解析材质\
             （现在 `Frame::materials` 是按名字查的一张平表）。⚠ 拷贝那一步属于帧表，宿主不自己插",
            pass.label
        ));
    }
    let zero = build_zero(&sampled_view)?;
    audit.push("group 0（五格全绑，哪怕 shader 只声明了一部分）：".to_string());
    audit.extend(zero.audit.iter().map(|line| format!("  {line}")));

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
    for (index, pass) in executed_plan.passes.iter().enumerate() {
        let mut set: Vec<External<'_>> = Vec::new();
        // ⚠ **`Role::Depth` 那条外部目标的路，这一档不再走**（§132）。
        //
        // 它没错，只是这一档不再需要：那条规则说的是"宿主给的外部目标顶掉同名声明资源"，
        // 将来谁真需要"宿主提供一张**池子拥有**的视图"，它还在、还是对的。
        // 这一档改成了 `Executor::seed` —— 因为深度那张图**必须只有一张**（拷贝要纹理、
        // 绑定要同一张纹理的视图），而 seed 才保证得了"一张"。
        // 两条路同时开着就是"同一个东西两套机制"，那是漂移的温床，所以这里**空着**。
        let _ = (index, &pass.label);
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

    let frame = Frame {
        width,
        height,
        sets: &sets,
        geometries: &resolved_geometry,
        materials: &resolved_materials,
    };
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
