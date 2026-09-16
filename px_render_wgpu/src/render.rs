//! 渲染路：**文档 → GPU → 回读 → PNG**（`art/15-render-wgpu.md` 的 S2 后半）。
//!
//! 这一档画出来的是"**背景（星空）+ 行星 + 大气**"，六条 pass **一条都不跳**：
//!
//! - `prepass`（深度-only）、`copy_depth`（深度快照）、`opaque`（清屏色 + 行星）、
//!   `sky`（**天空盒**，§136）、`transparent`（大气）、`blit` 全部执行；
//! - `sky` 那一笔要的材质名解析走**两张表**（物体的 id 表 / 帧自有材质表），
//!   帧自有材质的 WGSL 内联在文档里、由宿主按**与内容材质同一条**组装 / 反射 / 打包路装载
//!   （见 [`material_table`] / [`skybox_slot`] / `art::load_frame_material`）。
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

/// **这一档执行哪几条 pass**（`px_graphs::frame` 的那六个标签）。
///
/// ⚠ 按标签挑是**宿主的切片策略**，不是执行器的分派规则：`px_pass` 一个 pass 名字都不认识，
/// 它只按 `kind` 与状态分派（§124）。
///
/// ⚠ §136 起**一条都不跳**：天空盒那条 `sky` 现在真的画（名字解析 + 帧材质的反射装载 +
/// 程序化几何三件都补齐了）。下面 [`subset`] 仍然把"没执行的"打出来 ——
/// 那一条纪律（"绿是因为跳过了它"是最不能接受的那种绿）不因为今天没得跳就作废。
const EXECUTED: [&str; 6] = [
    "prepass",
    "copy_depth",
    "opaque",
    "sky",
    "transparent",
    "blit",
];

/// 三份布局的**身份**（`ResolvedGroup::layout_id` 那条契约：同布局同 id、异布局异 id）。
///
/// 宿主每种布局只有一份 ⇒ 三个常数。⚠ 帧自有材质与内容材质**共用** group 0 与 group 3
/// 那两份布局，所以它们的 id 也必须是同两个数 —— 两个数分岔就是"同一条键指两条管线"。
const ZERO_LAYOUT_ID: u64 = 0;
const STAGE_LAYOUT_ID: u64 = 1;
const MATERIAL_LAYOUT_ID: u64 = 2;

/// 程序化几何（没有顶点缓冲）画几个顶点：**三个**。
///
/// ⚠ 不是"随手挑的 3"：oracle 的天空盒就是这一笔 ——
/// `bevy_core_pipeline-0.19.1/src/core_3d/main_opaque_pass_3d_node.rs:109` 的
/// `render_pass.draw(0..3, 0..1)`，而它的顶点阶段（`skybox.wgsl:63-72`，本仓
/// `art/frame/vertex_sky.wgsl`）正是拿 `vertex_index` 现算三个顶点盖满屏幕。
/// 执行器的全屏 pass 也是 `draw(0..3, 1..2)`（同一个三角形）。
const PROCEDURAL_VERTICES: u32 = 3;

/// 一笔 draw 的材质名出自**哪张表**（§135）。
///
/// ⚠ 两张表都可能给出同一个名字，而**两张都给了就是歧义**：`objects` 的 id 就是它的材质名
/// （`px_graphs::frame::draws_of` 拿物体 id 当材质名），`frame_materials` 是帧自己的材质。
/// 同一个字符串在两处各有一份真本时，宿主**不许**替调用方挑一个 —— 静默的优先级是一条
/// 没有写在任何地方、也没人会去读的规则。一个都没有时把**两张表都列出来**：
/// 只说"找不到"会让人去翻错的那一张。
///
/// ⚠ 名字是**索引**，不是语义：这里（以及任何地方）都不许出现 `if name == "skybox"`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MaterialTable {
    /// 物体表里的第几个（那个物体的 id 就是它的材质名）。
    Objects(usize),
    /// 帧自有材质表里的第几份。
    Frame(usize),
}

impl MaterialTable {
    fn describe(self) -> String {
        match self {
            MaterialTable::Objects(index) => format!("物体 id 那张表（第 {index} 个物体）"),
            MaterialTable::Frame(index) => {
                format!("**帧自有材质**那张表（第 {index} 份，`frame_materials`）")
            }
        }
    }
}

/// 文档里的两张名字表 → 一笔 draw 的材质名落在哪一张。
fn material_table(spec: &px_protocol::scene::SceneSpec, name: &str) -> Result<MaterialTable, String> {
    let list = |names: Vec<&str>| -> String {
        if names.is_empty() {
            "（一个都没有）".to_string()
        } else {
            names.join(" / ")
        }
    };
    let objects = spec
        .objects
        .iter()
        .map(|object| object.id.as_str())
        .collect::<Vec<_>>();
    let frames = spec
        .frame_materials
        .iter()
        .map(|material| material.name.as_str())
        .collect::<Vec<_>>();
    let object = spec.objects.iter().position(|object| object.id == name);
    let frame = spec
        .frame_materials
        .iter()
        .position(|material| material.name == name);
    match (object, frame) {
        (Some(_), Some(_)) => Err(format!(
            "材质名 '{name}' 在**两张表**里都有：物体 id [{}] 与帧自有材质 [{}]。\n  \
             同一个名字两处真本 ⇒ 宿主只能猜一个，而猜错是**一声不吭的错像素**。\n  \
             ⇒ 这是调用方要改的：物体 id 与帧材质名必须互不相同",
            list(objects),
            list(frames)
        )),
        (Some(index), None) => Ok(MaterialTable::Objects(index)),
        (None, Some(index)) => Ok(MaterialTable::Frame(index)),
        (None, None) => Err(format!(
            "材质名 '{name}' **两张表里都没有**。\n  \
             物体 id（它们的 id 就是材质名）：{}\n  帧自有材质（`frame_materials`）：{}",
            list(objects),
            list(frames)
        )),
    }
}

/// 帧自有材质声明的贴图格 → 环境里那份天空盒落在哪一格（§136）。
///
/// ⚠ 文档的 `frame_materials` 里**没有**"哪一格是哪张图"这一栏（内容材质有
/// `material.textures`）⇒ 落点只能从**帧自己的环境**推：`environment.skybox` 是这一帧
/// 唯一一张帧级贴图，而它该落在哪一格，由**那份 WGSL 自己声明了几格**定。
///
/// 声明的格数不是恰好一格就是**歧义**（两格以上时"天空盒落哪一格"没有任何依据），
/// 歧义当场拒 —— 不许替调用方挑一个（与 [`material_table`] 同一条规矩）。
fn skybox_slot(
    material: &art::LoadedFrameMaterial,
    skybox: Option<&art::Skybox>,
) -> Result<Option<u32>, String> {
    let at = format!("帧自有材质 '{}'", material.name);
    let slots = material
        .textures
        .iter()
        .map(|(binding, dimension)| format!("第 {binding} 格（{}）", dimension.name()))
        .collect::<Vec<_>>()
        .join(" / ");
    match (skybox, material.textures.as_slice()) {
        // 环境里没有天空盒 ⇒ 这份帧材质也不该声明贴图格（声明了就没人能兑现它）。
        (None, []) => Ok(None),
        (None, _) => Err(format!(
            "{at} 声明了贴图格 [{}]，而这一帧的环境里**没有天空盒**（`environment.skybox` 是空的）\
             ：帧自有材质的贴图只有环境那一张来源，没有天空盒时这一格无处可绑",
            slots
        )),
        (Some(skybox), [(binding, dimension)]) => {
            let layers = skybox.texture.shape.layers;
            if dimension.layers() != layers {
                return Err(format!(
                    "{at} 在第 {binding} 格声明的是 {}（{} 层），而环境里那份天空盒 \
                     {} 是 {} 层 —— 同一张图在两边不是同一种东西",
                    dimension.name(),
                    dimension.layers(),
                    skybox.texture.label(),
                    layers
                ));
            }
            Ok(Some(*binding))
        }
        (Some(_), []) => Err(format!(
            "{at} 一格贴图都没声明，而这一帧的环境里有天空盒（`environment.skybox`）——\
             那份天空盒没有去处。要么这份 WGSL 少了一个 texture 声明，要么这一帧不该带天空盒"
        )),
        (Some(_), _) => Err(format!(
            "{at} 声明了 {} 格贴图（{}），而文档的 `frame_materials` 里**没有**\
             \"哪一格是哪张图\"这一栏：天空盒该落在哪一格没有任何依据。\
             ⚠ 不许挑一个 —— 要么这份 WGSL 只声明一格，要么先把那一栏加进契约",
            material.textures.len(),
            slots
        )),
    }
}

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
    /// 这一档**没执行**的 pass 与原因。⚠ §136 起它是空的（六条全跑）——
    /// 这一栏留着是因为那条纪律还在：真开始跳的时候，理由必须**跟着名字一起**出来。
    pub skipped: Vec<(String, String)>,
}

/// 一笔 draw 在 GPU 上的几何：顶点/索引缓冲 + **产物的属性表**。
///
/// ⚠ 布局的字段得活到 `Frame` 之后（执行器借的是它），所以 `attributes` 与 `layout`
/// 都挂在这个结构体上，而不是在函数里现造一个临时的。
///
/// ⚠ 两块缓冲是 `Option`，而**没有第二栏**说"这一笔是程序化的"：`vertices.is_none()`
/// 就是那件事本身（[`procedural_geometry`] 造出来的那一笔两栏都是 `None`）。
/// 再加一个 `bool` 就是同一个事实的第二份副本 —— 两份会漂，而漂开时没人会响。
struct Geometry {
    name: String,
    vertices: Option<wgpu::Buffer>,
    indices: Option<wgpu::Buffer>,
    attributes: Vec<wgpu::VertexAttribute>,
    vertex_count: u32,
    index_count: u32,
}

impl Geometry {
    /// 这一笔是不是程序化的：**没有顶点缓冲**，顶点由顶点阶段按 `vertex_index` 现算。
    fn procedural(&self) -> bool {
        self.vertices.is_none()
    }

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

/// 一块 `MeshStage` 的字节：`world_from_local` + `view_proj` + **法线矩阵**。
///
/// ⚠ 三块矩阵都走 `mat4.rs` 那几份**逐位**移植件：`world_from_local` 用
/// `from_scale_rotation_translation`（文档的 `transform` 就是它的三个入参），
/// `view_proj` 用相机自己算好的 `clip_from_world`（= `clip_from_view × view_from_world`）。
/// 在这里重算一次 `clip_from_view × view_from_world` 就是"同一条契约、两处算"。
///
/// ⚠ 法线矩阵是**第三块**（Bevy 的 `local_from_world_transpose`）：它是
/// `Affine3A::inverse().matrix3.transpose()`（[`Mat4::normal_matrix_3x3`]，判据钉着），
/// **不是** `world_from_local` 的 3×3 —— 均匀缩放下两者数学等价、**浮点不等价**，
/// 而差的那一个末位正好落在"盘内散落的 ±1"上（本仓库这一族的第五次现形）。
/// 每一列补齐到 16 字节（WGSL 的 `mat3x3<f32>` 布局）。
fn stage_bytes(transform: &px_protocol::scene::Transform, camera: &crate::camera::Camera) -> [u8; 176] {
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
    let mut bytes = [0_u8; 176];
    let normal = world_from_local.normal_matrix_3x3();
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
    // 法线矩阵：三列，**每列补齐到 16 字节**（第 4 位补 0，shader 读不到它）。
    for (index, column) in normal.iter().enumerate() {
        for (slot, value) in [column.x, column.y, column.z, 0.0].iter().enumerate() {
            let at = 128 + index * 16 + slot * 4;
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
            vertices: Some(gpu.device.create_buffer_init(&BufferInitDescriptor {
                label: Some("顶点（交错，布局来自产物属性表）"),
                usage: wgpu::BufferUsages::VERTEX,
                contents: &bytes,
            })),
            indices: Some(gpu.device.create_buffer_init(&BufferInitDescriptor {
                label: Some("索引"),
                usage: wgpu::BufferUsages::INDEX,
                contents: &index_bytes,
            })),
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

    // ---- 顶点阶段 ↔ 产物属性表：**当场对账**；程序化那一笔在这里认出来 ----
    //
    // ⚠ 反射的是**文档里那段顶点阶段**（不是"我们以为它要什么"）：它声明的每一个
    //    `@location(n)` 都必须落在产物给的属性表里，而且类型一致。错位在画面上只是
    //    "某几个属性读成了别的格子"，任何门都不会响。
    //
    // ⚠ **程序化几何**不是按名字认的（那会变成 `if name == "skybox"`）：判据是
    //    **那段顶点阶段读不读 `@location`** —— 一个都不读 ⇒ 顶点全靠 `vertex_index` 现算，
    //    顶点缓冲根本用不上（`ResolvedGeometry.vertices = None`）。反过来，
    //    读 `@location` 的顶点阶段必须拿到一张真的属性表，否则当场拒（今天那条守卫）。
    //    两边的错法都是"画出来不对但谁都不报错"，所以两条都要拦。
    for pass in &executed_plan.passes {
        if pass.kind != PassKind::Geometry {
            continue;
        }
        let module = shader::validate(
            &format!("pass '{}' 的顶点阶段", pass.label),
            &pass.vertex_shader,
        )?;
        let inputs = vertex_inputs(&module, &pass.vertex_entry, &pass.label)?;
        let procedural = inputs.is_empty();
        for draw in &pass.draws {
            let known = geometries
                .iter()
                .position(|geometry| geometry.name == draw.geometry);
            let geometry = match (known, procedural) {
                (Some(index), false) => &geometries[index],
                (Some(_), true) => {
                    return Err(format!(
                        "pass '{}' 的顶点阶段（{}）一个 `@location` 都不读（顶点全由 \
                         `vertex_index` 现算），而这一笔点的几何 '{}' 是**物体的网格**：\
                         那份顶点缓冲不会被用到 —— 要么这段顶点阶段写错了，要么这一笔画错了",
                        pass.label, pass.vertex_entry, draw.geometry
                    ))
                }
                (None, true) => {
                    geometries.push(procedural_geometry(draw.geometry.clone()));
                    audit.push(format!(
                        "几何 '{}'：**程序化**（没有顶点缓冲；{} 个顶点由顶点阶段按 \
                         `vertex_index` 现算 —— pass '{}' 的顶点阶段一个 `@location` 都不读）",
                        draw.geometry, PROCEDURAL_VERTICES, pass.label
                    ));
                    geometries.last().expect("刚 push 的")
                }
                (None, false) => {
                    return Err(format!(
                        "pass '{}' 要画几何 '{}'，而宿主这一档没给这个名字（给了：{}）",
                        pass.label,
                        draw.geometry,
                        geometries
                            .iter()
                            .map(|geometry| geometry.name.as_str())
                            .collect::<Vec<_>>()
                            .join(" / ")
                    ))
                }
            };
            if !procedural {
                check_vertex_inputs(&inputs, &pass.label, geometry)?;
            }
        }
        audit.push(format!(
            "顶点阶段（pass '{}'）的输入与产物属性表对得上：{}（声明的输入 {:?}）",
            pass.label, pass.vertex_entry, inputs
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
            // ⚠ 程序化那一笔两栏都给 `None`：它**没有**顶点缓冲，顶点由顶点阶段按
            //    `vertex_index` 现算。给一个空缓冲是另一回事 —— 执行器会拿它去建管线的
            //    顶点布局，而那段顶点阶段不认那个布局。
            vertices: geometry
                .vertices
                .as_ref()
                .map(|buffer| (buffer, geometry.layout())),
            indices: geometry
                .indices
                .as_ref()
                .map(|buffer| (buffer, wgpu::IndexFormat::Uint32, geometry.index_count)),
            vertex_count: geometry.vertex_count,
        })
        .collect();

    // ---- 材质名 → 哪张表（§135）：名字是**索引**，两张表都可能给出它 ----
    //
    // ⚠ 这一轮走的是**计划里每一条 draw**（不只是要执行的那几条）：文档说不说得通，
    //    与"这一档执不执行它"是两件事（§130 的那条口径）。
    for pass in &plan.passes {
        for draw in &pass.draws {
            if draw.material.is_empty() {
                continue;
            }
            let table = material_table(&spec, &draw.material)?;
            audit.push(format!(
                "pass '{}' 的 draw（几何 '{}'）要材质 '{}' ⇒ {}",
                pass.label,
                draw.geometry,
                draw.material,
                table.describe()
            ));
        }
    }

    // ---- 帧自有材质（§135/§136）：全文在文档里，走**同一条**反射 / 打包 / 绑定路 ----
    //
    // ⚠ "帧自有"的意思是**兑现者自有**（那支 WGSL 只由裸 wgpu 宿主兑现），
    //    不是契约自有：它照样走材质那条绑定组构造（12 格超集 + 空槽绑白图）。
    //    这也是能复用 `sampler_of` / `upload_texture` / 兜底那一套的前提。
    let modules = shader::modules();
    let mut frame_materials: Vec<(art::LoadedFrameMaterial, material::MaterialBinding)> =
        Vec::with_capacity(spec.frame_materials.len());
    for declared in &spec.frame_materials {
        let loaded = art::load_frame_material(declared, &modules)?;
        // 它的贴图格只有一个来源：环境里那份天空盒（见 `skybox_slot`）。
        let slot = skybox_slot(&loaded, scene.skybox.as_ref())?;
        let textures: Vec<art::BoundTexture> = match (&scene.skybox, slot) {
            (Some(skybox), Some(binding)) => vec![skybox.bound(binding)],
            _ => Vec::new(),
        };
        let binding = materials.bind_request(
            &gpu.device,
            &gpu.queue,
            &material::BindingRequest {
                key: material::frame_key(loaded.version),
                label: loaded.name.as_str(),
                params: loaded.params.as_slice(),
                textures: textures.as_slice(),
            },
        )?;
        audit.push(format!(
            "帧自有材质 '{}'：entry {}｜组装后 {} 字节（内容键 {:016x}）｜参数 {} 字节｜\
             声明的贴图格 {:?}｜绑上的格 {:?}｜走兜底白图的格 {:?}（**与内容材质同一条 \
             12 格超集的路**：帧材质的「自有」是兑现者自有，不是契约自有）",
            loaded.name,
            loaded.entry,
            loaded.assembled.len(),
            loaded.version,
            loaded.params.len(),
            loaded.textures,
            binding.bound_slots,
            binding.fallback_slots
        ));
        if let (Some(skybox), Some(binding)) = (&scene.skybox, slot) {
            audit.push(format!(
                "  ⚠ 第 {binding} 格绑的是环境里那份天空盒 {}（采样器 {:?} —— \
                 oracle 是**显式**传 `Sampler::clamped()`，不是落回缺省；\
                 它与 `Sampler::default()` 差在 `address_u`）",
                skybox.texture.label(),
                skybox.sampler
            ));
        }
        frame_materials.push((loaded, binding));
    }

    // ⚠ 一笔 draw 的组：**group 0 + group 1 + group 3**。group 1 是**每个物体**的
    //    （两块矩阵），所以"解析好的材质"其实按**物体**给：名字就是物体 id，
    //    而文档里一笔 draw 的 geometry 与 material 用的正是同一个名字（`frame.rs::draws_of`）。
    //    真出现"一条 pass 里同一个物体画两笔"的那天，这里要当场拒 —— 一个名字给不出两组矩阵。
    //
    // ⚠ 帧自有材质**没有 group 1**：它的顶点阶段是程序化的（`vertex_index` 现算），
    //    `MeshStage` 那两块矩阵它一格都不读。少给一组不是省事 —— 给了它反而要求那段
    //    顶点阶段认一个它不认的布局。
    let mut resolved_materials: Vec<ResolvedMaterial<'_>> = scene
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
                        layout_id: ZERO_LAYOUT_ID,
                    },
                    ResolvedGroup {
                        group: 1,
                        bind_group: &stage.bind_group,
                        // 一份布局，所有物体共用（见上面那段）。
                        layout: stage_layout.clone(),
                        layout_id: STAGE_LAYOUT_ID,
                    },
                    ResolvedGroup {
                        group: MATERIAL_BIND_GROUP,
                        bind_group: &binding.bind_group,
                        layout: material_layout.clone(),
                        layout_id: MATERIAL_LAYOUT_ID,
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
    resolved_materials.extend(frame_materials.iter().map(|(loaded, binding)| ResolvedMaterial {
        name: loaded.name.as_str(),
        groups: vec![
            ResolvedGroup {
                group: 0,
                // ⚠ 与内容材质**同一份** group 0 布局 ⇒ 同一个 `layout_id`
                //    （契约：同布局同 id、异布局异 id）。
                bind_group: &zero.bind_group,
                layout: zero.layout.clone(),
                layout_id: ZERO_LAYOUT_ID,
            },
            ResolvedGroup {
                group: MATERIAL_BIND_GROUP,
                bind_group: &binding.bind_group,
                layout: material_layout.clone(),
                layout_id: MATERIAL_LAYOUT_ID,
            },
        ],
        // 混合档与剔除档来自 `material::frame_key`（那两档**不在文档里**：它们是策略，
        // 依据是 oracle 那条天空盒管线自己的固定状态，见那个函数的注释）。
        blend: binding.key.blend(),
        cull: Cull::None,
        // 片元阶段：文档里内联的那份 WGSL 组装后的全文，入口也是文档给的那个名字。
        fragment_shader: loaded.assembled.as_str(),
        fragment_entry: loaded.entry.as_str(),
    }));

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
        // ⚠ §136 起这一栏是空的：文档里六条 pass 全部执行，**一条都不跳**。
        //    留着它是为了那条纪律（"绿是因为跳过了它"）：真开始跳的时候，
        //    理由必须跟着名字一起出来 —— 那时候这里要重新有内容。
        skipped: Vec::new(),
    })
}

/// 按 [`EXECUTED`] 把计划切成"这一档真的交给执行器的那一份"，并把**没执行的**打出来。
///
/// ⚠ 切出来的那一份仍然是 `px_pass::Plan`：资源表、布局、`PassPlan` 全都是原来的那几份，
/// 只是少了没执行的那几条 —— 执行器看不到"切片"这回事。
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
        "本切片执行的 pass：{}（{} / {}{}）",
        EXECUTED.join(" → "),
        EXECUTED.len(),
        plan.passes.len(),
        if not_executed.is_empty() {
            "，**一条都不跳**"
        } else {
            ""
        }
    ));
    // ⚠ 跳过的那些**必须**在这里被点名：不点名的话，"图对上了"与"那条 pass 根本没跑"
    //    在读数上长得一模一样（§107 那条：「一次改两个变量」在这张图上会让读数无法归因）。
    if !not_executed.is_empty() {
        audit.push(format!(
            "⚠ 本切片**没有执行**这 {} 条 pass（它们仍在计划里、也过了 `check()`，只是没交给执行器）：{}",
            not_executed.len(),
            not_executed.join(" / ")
        ));
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

/// 顶点阶段声明的一个输入：`location` 与它要的类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VertexInput {
    location: u32,
    format: wgpu::VertexFormat,
}

/// 一段顶点阶段声明的**全部 `@location` 输入**（按声明次序）。
///
/// ⚠ 反射的是**文档里那段顶点阶段**，不是"我们以为它要什么"：它声明的每一个
/// `@location(n)` 都必须落在产物的属性表里，而且类型一致。错位在画面上只是
/// "某几个属性读成了别的格子"，任何门都不会响。
///
/// ⚠ 返回**空表**不是"没事"，而是一条判据：一个 `@location` 都不读 ⇒ 这一笔是
/// **程序化**的（顶点全由 `vertex_index` 现算，顶点缓冲用不上）。两件事共用这一次反射，
/// 正是为了让"它读什么"只有一个答案。
fn vertex_inputs(
    module: &naga::Module,
    entry_name: &str,
    label: &str,
) -> Result<Vec<VertexInput>, String> {
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
    let mut inputs = Vec::new();
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
        inputs.push(VertexInput { location, format });
    }
    Ok(inputs)
}

/// 顶点阶段要的每一格都必须落在**产物的属性表**里（缺一格 / 类型不符都当场拒）。
fn check_vertex_inputs(
    inputs: &[VertexInput],
    label: &str,
    geometry: &Geometry,
) -> Result<(), String> {
    for input in inputs {
        let found = geometry
            .attributes
            .iter()
            .find(|attribute| attribute.shader_location == input.location);
        match found {
            Some(attribute) if attribute.format == input.format => {}
            Some(attribute) => {
                return Err(format!(
                    "pass '{label}' 的顶点阶段在 location({}) 上要 {:?}，\
                     而几何 '{}' 在那里给的是 {:?}：属性表与顶点阶段说的不是一件事",
                    input.location, input.format, geometry.name, attribute.format
                ))
            }
            None => {
                return Err(format!(
                    "pass '{label}' 的顶点阶段要 location({})（{:?}），\
                     而几何 '{}' 的属性表只有 [{}]：产物没带这个属性",
                    input.location,
                    input.format,
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

/// 一笔**程序化**几何：没有顶点缓冲、没有索引缓冲，顶点数由帧策略定。
///
/// ⚠ `vertices: None` 就是这个意思（`ResolvedGeometry` 那条注释：顶点由顶点着色器按
/// `@builtin(vertex_index)` 现算）。别在这里塞一个"空缓冲"顶上：空缓冲是**有**顶点缓冲，
/// 执行器会拿它去建管线的顶点布局，而那段顶点阶段根本不认那个布局。
fn procedural_geometry(name: String) -> Geometry {
    Geometry {
        name,
        // ⚠ `vertices: None` **就是**"这一笔没有顶点缓冲"那件事本身（`Geometry::procedural`
        // 直接读它）—— 别在这里塞一个"空缓冲"顶上：空缓冲是**有**顶点缓冲，
        // 执行器会拿它去建管线的顶点布局，而那段顶点阶段根本不认那个布局。
        vertices: None,
        indices: None,
        attributes: Vec::new(),
        vertex_count: PROCEDURAL_VERTICES,
        index_count: 0,
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use px_protocol::material::TextureDimension;
    use px_protocol::scene::{Member, SceneSpec};

    /// 一份最小文档：`objects` 与 `frame_materials` 两节的 JSON 由调用方给。
    ///
    /// ⚠ 判据只喂这两节 —— 名字解析这条规则的全部输入就是这两张表。
    fn spec_of(objects: &[&str], frames: &[&str]) -> SceneSpec {
        let object = |id: &str| {
            format!(
                r#"{{"id": "{id}",
                     "geometry": {{"source": "primitive", "name": "icosphere", "params": {{"radius": 1.0}}}},
                     "material": {{"shader": {{"graph": "shaders", "node": "surface", "key": "00"}}}}}}"#
            )
        };
        let frame = |name: &str| {
            format!(
                r#"{{"name": "{name}",
                     "shader": "@fragment fn fragment() -> @location(0) vec4<f32> {{ return vec4<f32>(1.0); }}",
                     "entry": "fragment"}}"#
            )
        };
        let text = format!(
            r#"{{"schema": {}, "name": "夹具", "objects": [{}], "frame_materials": [{}]}}"#,
            px_protocol::SCENE_SCHEMA,
            objects.iter().map(|id| object(id)).collect::<Vec<_>>().join(","),
            frames.iter().map(|name| frame(name)).collect::<Vec<_>>().join(",")
        );
        serde_json::from_str(&text).expect("夹具文档")
    }

    /// 名字落在**哪张表**：物体 id 与帧自有材质各自的名字空间。
    #[test]
    fn a_material_name_resolves_to_exactly_one_table() {
        let spec = spec_of(&["planet", "atmosphere"], &["skybox"]);
        assert_eq!(
            material_table(&spec, "planet").expect("物体那张表"),
            MaterialTable::Objects(0)
        );
        assert_eq!(
            material_table(&spec, "atmosphere").expect("物体那张表"),
            MaterialTable::Objects(1)
        );
        assert_eq!(
            material_table(&spec, "skybox").expect("帧材质那张表"),
            MaterialTable::Frame(0)
        );
        // ⚠ 名字是**索引**：解析结果里只有"第几张表的第几个"，没有一个字节的语义。
        assert!(material_table(&spec, "planet").unwrap().describe().contains("物体"));
        assert!(material_table(&spec, "skybox").unwrap().describe().contains("帧自有材质"));
    }

    /// **两张表都没有** ⇒ 拒，而且**两张表都要列出来**（只说"找不到"会让人去改错的那一张）。
    #[test]
    fn a_name_in_neither_table_is_refused_with_both_lists() {
        let spec = spec_of(&["planet"], &["skybox"]);
        let err = material_table(&spec, "skybx").expect_err("两张表都没有 ⇒ 拒");
        assert!(err.contains("planet"), "要列出物体 id：{err}");
        assert!(err.contains("skybox"), "要列出帧自有材质：{err}");
        assert!(err.contains("两张表里都没有"), "{err}");
    }

    /// **两张表都有** ⇒ 也拒（歧义是调用方要修的，静默的优先级不是规则）。
    #[test]
    fn a_name_in_both_tables_is_refused_as_ambiguity() {
        let spec = spec_of(&["planet"], &["planet"]);
        let err = material_table(&spec, "planet").expect_err("两张表都有 ⇒ 拒");
        assert!(err.contains("两张表"), "{err}");
        assert!(err.contains("物体 id"), "{err}");
        assert!(err.contains("帧自有材质"), "{err}");
    }

    /// 一份**帧材质**夹具：`textures` 就是反射出来的那几格。
    fn frame_material(textures: &[(u32, TextureDimension)]) -> art::LoadedFrameMaterial {
        art::LoadedFrameMaterial {
            name: "skybox".to_string(),
            entry: "fragment".to_string(),
            assembled: String::new(),
            params: Vec::new(),
            textures: textures.to_vec(),
            version: 0,
        }
    }

    /// 一份**天空盒**夹具：只有层数与名字参与判据。
    fn skybox_fixture(layers: u32) -> art::Skybox {
        let shape = px_protocol::art::TextureShape {
            width: 512,
            height: 512,
            layers,
            levels: 1,
            format: px_protocol::art::TextureFormat::Rgba8Srgb,
        };
        art::Skybox {
            sampler: px_protocol::scene::Sampler::clamped(),
            texture: art::LoadedTexture {
                member: Member::new("generated", "stars", "00"),
                shape,
                bytes: Vec::new(),
                image: image::RgbaImage::new(512, 512),
            },
        }
    }

    /// 帧材质的贴图格 → 天空盒落点：**恰好一格**才是可判的，其余三档都当场拒。
    #[test]
    fn the_skybox_lands_on_the_only_slot_the_frame_material_declares() {
        let cube = [(5, TextureDimension::Cube)];
        let skybox = skybox_fixture(6);
        assert_eq!(
            skybox_slot(&frame_material(&cube), Some(&skybox)).expect("一格 ⇒ 就是它"),
            Some(5)
        );

        // 一格都没声明，而环境里有天空盒 ⇒ 那份天空盒没有去处 ⇒ 拒。
        let err = skybox_slot(&frame_material(&[]), Some(&skybox)).expect_err("没有去处 ⇒ 拒");
        assert!(err.contains("没有去处"), "{err}");

        // 两格以上 ⇒ **无从知道**天空盒落哪一格 ⇒ 拒（不许挑一个）。
        let two = [(5, TextureDimension::Cube), (7, TextureDimension::Cube)];
        let err = skybox_slot(&frame_material(&two), Some(&skybox)).expect_err("歧义 ⇒ 拒");
        assert!(err.contains("第 5 格") && err.contains("第 7 格"), "要列出那几格：{err}");
        assert!(err.contains("没有任何依据"), "{err}");

        // 维度不符（声明 2D、图是 6 层 cube）⇒ 拒。
        let flat = [(1, TextureDimension::D2)];
        let err = skybox_slot(&frame_material(&flat), Some(&skybox)).expect_err("维度不符 ⇒ 拒");
        assert!(err.contains("6 层"), "{err}");

        // 环境里**没有**天空盒 ⇒ 帧材质也不该声明贴图格（声明了就没人能兑现它）。
        assert_eq!(
            skybox_slot(&frame_material(&[]), None).expect("两边都空"),
            None
        );
        let err = skybox_slot(&frame_material(&cube), None).expect_err("无处可绑 ⇒ 拒");
        assert!(err.contains("没有天空盒"), "{err}");
    }

    /// 帧自有材质的状态是**策略**，取值来自 oracle 那条管线自己的固定状态：
    /// 不混合（`ColorTargetState { blend: None }`）、两面都画
    /// （`PrimitiveState::default()` 的 `cull_mode: None`）。
    #[test]
    fn the_frame_material_state_follows_the_oracle_pipeline() {
        let key = material::frame_key(0x1234);
        assert_eq!(key.shader, 0x1234);
        assert_eq!(key.cull, material::cull_code(CullMode::None));
        assert_eq!(key.alpha, material::alpha_code(px_protocol::scene::AlphaMode::Opaque));
        assert_eq!(key.blend(), None, "不混合：oracle 那条管线的 blend 就是 None");
        assert_eq!(key.cull_face(), None, "两面都画");
    }

    /// 程序化几何：**两块缓冲都不给**、顶点数是 oracle 自己那个 `draw(0..3, 0..1)`。
    #[test]
    fn procedural_geometry_has_no_buffers_and_three_vertices() {
        let geometry = procedural_geometry("skybox".to_string());
        assert!(geometry.vertices.is_none());
        assert!(geometry.indices.is_none());
        assert!(geometry.attributes.is_empty());
        assert_eq!(geometry.vertex_count, PROCEDURAL_VERTICES);
        assert_eq!(PROCEDURAL_VERTICES, 3, "oracle 的 `render_pass.draw(0..3, 0..1)`");
        assert!(geometry.procedural(), "没有顶点缓冲**就是**程序化");
    }
}
