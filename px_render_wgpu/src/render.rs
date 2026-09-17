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

/// **这一档执行哪几条 pass**：文档声明的**每一条**。
///
/// ⚠ 这里曾经是一张写死的标签表（§130–§136：那时宿主只兑现了六条里的几条，表就是
/// "兑现了哪些"的读数）。§139 起那张表**不再成立**：影子那六条 pass 的标签是**生成**的
/// （`point_shadow_0_+x`…），写死的表认识不了它们，而"按名字前缀认"就是把生成格式塞进
/// 宿主 —— 而生成格式**不进任何契约**（改它一个字都不该动画面）。
/// 于是策略只剩一条：文档声明的每一条都执行。
///
/// ⚠ 那张表带来的**读数**没有消失：`Rendered::skipped` 那一栏还在（今天恒为空）。
/// "绿是因为跳过了它"是最不能接受的那种绿 —— 真开始跳的那一天，理由必须跟着名字一起出来。

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
    /// 生成的材质实例表里的第几份（§139）。照的是哪一份由 `base` 说。
    Instance(usize),
}

impl MaterialTable {
    fn describe(self) -> String {
        match self {
            MaterialTable::Objects(index) => format!("物体 id 那张表（第 {index} 个物体）"),
            MaterialTable::Frame(index) => {
                format!("**帧自有材质**那张表（第 {index} 份，`frame_materials`）")
            }
            MaterialTable::Instance(index) => {
                format!("**生成的材质实例**那张表（第 {index} 份，`material_instances`）")
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
    // 生成的材质实例（§139）：名字照的是**另一份**（`base`）。⚠ 这里**只判"它落在哪张表"**，
    // 不在这里解析名字的格式：磁盘上那份 `.pxart` 说了算（`material_instances`），
    // 而"这个名字照的是谁"是那张表的两栏之一。
    let instance = spec
        .material_instances
        .iter()
        .position(|instance| instance.name == name);
    match (object, frame, instance) {
        (Some(_), Some(_), _) => Err(format!(
            "材质名 '{name}' 在**两张表**里都有：物体 id [{}] 与帧自有材质 [{}]。\n  \
             同一个名字两处真本 ⇒ 宿主只能猜一个，而猜错是**一声不吭的错像素**。\n  \
             ⇒ 这是调用方要改的：物体 id 与帧材质名必须互不相同",
            list(objects),
            list(frames)
        )),
        (Some(_), _, Some(_)) | (_, Some(_), Some(_)) => Err(format!(
            "材质名 '{name}' 既是真本（物体 id 或帧自有材质），又是生成的实例：\
             一个名字只能有一份真本"
        )),
        (Some(index), None, None) => Ok(MaterialTable::Objects(index)),
        (None, Some(index), None) => Ok(MaterialTable::Frame(index)),
        (None, None, Some(index)) => Ok(MaterialTable::Instance(index)),
        (None, None, None) => Err(format!(
            "材质名 '{name}' **三张表里都没有**。\n  \
             物体 id（它们的 id 就是材质名）：{}\n  帧自有材质（`frame_materials`）：{}\n  \
             生成的实例（`material_instances`）：{}",
            list(objects),
            list(frames),
            list(spec
                .material_instances
                .iter()
                .map(|instance| instance.name.as_str())
                .collect())
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

/// 点光 cube 影图那份资源在**帧配方**里的名字（`art/frame/default.toml` 的 `[[resources]]`）。
///
/// ⚠ 这是宿主与帧配方之间**唯一**一个按名字的约定，而它与 `scene_depth` / `skybox`
/// 那几个是同一类：**渲染器的形状**（哪一格绑哪张图）本来就住在宿主这一侧（§104 第 1 条）。
/// 文档那边「有没有这份资源」是**内容/形状的合成结果**（一盏投影的点光都没有时它就不烘），
/// 所以宿主必须自己判"有没有"，而不是假定它一定在。
const SHADOW_TEXTURE_RESOURCE: &str = "point_shadow_textures";

/// 一份画好的图：紧凑 RGBA8 + 这一帧的审计文本。
///
/// ⚠ 不留目标纹理：回读在 [`run`] 里就做完了，"目标还在手上"是这一档用不到的余量。
pub struct Rendered {
    pub pixels: Vec<u8>,
    /// 这张图的尺寸。⚠ **不是**请求里的 `width/height`：对照图的请求给的是**一格**的尺寸，
    /// 而落盘那张是 `(格宽×列数) × (格高×行数)`（Bevy 那边 `job.width/height` 也是这么改的）。
    /// 报告与 PNG 两处都从这两个数走 —— 少一处对上就是"报告说 960×640、文件是 3840×1920"。
    pub width: u32,
    pub height: u32,
    pub audit: Vec<String>,
    pub executed: Vec<String>,
    /// 这一档**没执行**的 pass 与原因。⚠ §136 起它是空的（六条全跑）——
    /// 这一栏留着是因为那条纪律还在：真开始跳的时候，理由必须**跟着名字一起**出来。
    pub skipped: Vec<(String, String)>,
    /// 这份产物声明了 `clouds` 这个期望标签（`SceneSpec::expects`）。
    ///
    /// ⚠ 它在这里返回，而不是让服务那一侧再读一遍文档：`run` 手上本来就有 `spec`，
    /// 而"再读一遍"就是同一条真本的第二份副本 —— 两次读之间文档被换掉的话，
    /// 报告里的"该有云"与画出来的那张图说的就不是同一份产物了。
    pub declared_clouds: bool,
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

/// 组 1 的**一份视图**（`PassView`）：这一条 pass 的 `view_proj` + 那一份**全帧共用的实例数组**。
///
/// ⚠ 原来这里叫 `Stage`，是"每个 (物体, 视图) 一份 176 字节的 `MeshStage`"；§142 的参数分类
/// 把它拆成两格：**super（`view_proj`）一条 pass 一份**、**instance（世界矩阵 + 法线矩阵）
/// 长度 = 物体数的一份数组**（按 `@builtin(instance_index)` 选格）。这个结构体因此只剩
/// "一份视图"那一半 —— 实例数组是**一份**，所有视图绑的是同一个缓冲。
///
/// ⚠ 布局不在这里：它是**一份**（见 `run` 里那段），所有视图共用 —— 每个视图各建一份
/// "内容相同、对象不同"的布局会让管线缓存键与 wgpu 的布局比对说不到一块去。
struct Stage {
    /// 这一份 `PassView` 的 uniform 缓冲。只为了活着（绑进组里的是它的引用）。
    _view: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

/// 点光 cube 的**一面**：那个面当相机时的一套状态（§139）。
///
/// 一条影子 pass 要用的是它：group 0 那份 `view`（这一面的矩阵）与组 1 里那一份
/// `PassView`（这一面的 `clip_from_world`）。名字是**生成的**（`material_instances`），
/// 而"名字 → 哪一面"由**用到它的那条 pass** 说（`PassSpec::cube_face`）。
struct Face {
    /// cube 的下标（= 聚类缓冲的下标 = 着色器里的 `light_id`）。
    light: u32,
    /// 0..5，次序照 `bevy_camera::primitives::CUBE_MAP_FACES`。
    face: u32,
    /// 这一面的"相机"（`camera::face_view` 那条链算出来的四块矩阵）。
    ///
    /// ⚠ 这一档**没有**"面的 group 0"：影子那一笔的顶点阶段读的是组 1 那份 `PassView`
    /// （里面已经是这一面的 `clip_from_world`），而组 0 里第 2 格绑的正是这条
    /// pass 正在写的 cube —— 绑上它 wgpu 当场拒（实测：
    /// `TextureUses(DEPTH_STENCIL_WRITE) is an exclusive usage and cannot be used with
    /// any other usages within the usage scope`）。所以那一面的一切都从 `camera` 进
    /// `PassView`，不进组 0。
    camera: crate::camera::Camera,
}

/// **一格实例**（`MeshInstance`）的字节：`world_from_local` + **法线矩阵**。
///
/// ⚠ 这里**没有** `view_proj`：它是 super，住在 `PassView` 里（一条 pass 一份）。
/// 拆开之后这一格是 112 字节，112 是 16 的倍数 ⇒ 它作为数组元素的步长合法
/// （WGSL：`array<T>` 的元素步长必须是 16 的倍数）。
///
/// ⚠ 两块矩阵都走 `mat4.rs` 那几份**逐位**移植件：`world_from_local` 用
/// `from_scale_rotation_translation`（文档的 `transform` 就是它的三个入参）。
///
/// ⚠ 法线矩阵是**第二块**（Bevy 的 `local_from_world_transpose`）：它是
/// `Affine3A::inverse().matrix3.transpose()`（[`Mat4::normal_matrix_3x3`]，判据钉着），
/// **不是** `world_from_local` 的 3×3 —— 均匀缩放下两者数学等价、**浮点不等价**，
/// 而差的那一个末位正好落在"盘内散落的 ±1"上（本仓库这一族的第五次现形）。
/// 每一列补齐到 16 字节（WGSL 的 `mat3x3<f32>` 布局）。
fn instance_bytes(transform: &px_protocol::scene::Transform) -> [u8; 112] {
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
    let mut bytes = [0_u8; 112];
    let normal = world_from_local.normal_matrix_3x3();
    for (index, column) in [
        world_from_local.x_axis,
        world_from_local.y_axis,
        world_from_local.z_axis,
        world_from_local.w_axis,
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
            let at = 64 + index * 16 + slot * 4;
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
    bytes
}

/// **一份 `PassView`** 的字节：这一条 pass 的 `clip_from_world`。
///
/// ⚠ 它**与物体无关**：同一个视图下所有物体共用这一份（原来它是每 (物体, 视图) 各一份
/// `MeshStage` 里的第二块矩阵，值一模一样、抄了 N 遍 —— §148 之后它就住在这里）。
fn view_bytes(camera: &crate::camera::Camera) -> [u8; 64] {
    let mut bytes = [0_u8; 64];
    for (index, column) in [
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

/// 这一次出图**要几台相机、怎么摆**（"怎么看"那一栏；内容一律来自文档）。
///
/// ⚠ 它就是 Bevy 那边的 `job.view`（`View { cam, sheet, columns }`）：`sheet = false` 那一路是
/// `probe_camera(step.cam)`，`sheet = true` 那一路是产物自带的相机表（§110.1）。
/// 两路**共用**同一段渲染代码，差别只有"相机从哪来"与"落在哪一块"。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Views {
    /// 一张：`--cam`（`None` = 固定探针机位）。
    Single(Option<[f32; 3]>),
    /// 对照图（J2）：产物自带的评审相机表，按 `columns` 列排开。
    Sheet { columns: u32 },
}

/// 一台相机 + 它落在**宿主目标**里的那一块。
struct Placement {
    camera: crate::camera::Camera,
    /// 这一格的绝对像素矩形 `(x, y, w, h)`；`None` = 整幅（单张那条路）。
    ///
    /// ⚠ 它同时是 `Frame::viewport`（交给执行器）与 `view.viewport`（交给内容 shader）那一份：
    /// 两个数必须**同源**。分成两处算的话，"pass 画在哪儿"与"shader 以为自己在哪儿"
    /// 会各自漂开，而症状是每格都像左上角那一格的视角 —— 门不会响。
    rect: Option<[u32; 4]>,
    /// 第几格（审计用；单张时是 0）。
    index: usize,
    /// 这一格是谁（审计用：相机来自产物表时带上 tag）。
    note: String,
}

impl Placement {
    /// 交给执行器/`view` uniform 的那一份（f32；单张是 `None`）。
    fn viewport(&self) -> Option<[f32; 4]> {
        self.rect
            .map(|rect| [rect[0] as f32, rect[1] as f32, rect[2] as f32, rect[3] as f32])
    }

    /// `view.viewport`：单张那条路是整幅 `(0, 0, 宽, 高)`（与加格子之前逐字节相同）。
    fn uniform_viewport(&self, cell: (u32, u32)) -> [f32; 4] {
        let rect = self.rect.unwrap_or([0, 0, cell.0, cell.1]);
        [rect[0] as f32, rect[1] as f32, rect[2] as f32, rect[3] as f32]
    }
}

/// 相机表 → 一台台相机 + 它们各自那一块；顺带算**宿主目标**的尺寸。
///
/// 两条路的规则都逐字照搬 Bevy 宿主（`px_render/src/main.rs:1686-1732`）：
///
/// - `Single`：一台探针相机，整幅，目标 = 请求的尺寸；
/// - `Sheet`：`columns = view.columns.max(1)`（Bevy 同一条），
///   `rows = 相机数.div_ceil(columns)`，目标 = `(格宽 × columns, 格高 × rows)`，
///   **第 i 台相机落在 `(i % columns, i / columns)`**（行优先，`SheetCell` 的排布就是它）。
///   ⚠ `columns = 0` 兜成 1 不是"随手"：协议那一栏的缺省是 0，而 0 列排不出格子
///   （Bevy 也是 `.max(1)`）。
///
/// ⚠ 长宽比取的是**格子的**尺寸（`width/height`），不是整幅目标：Bevy 的投影是
/// `camera_projection.update(logical_viewport_size)`（`bevy_render-0.19.1/src/camera.rs:425-433`），
/// 带 viewport 的相机拿到的是 **viewport 的大小**。拿整幅去算，12 格的视野会一起变宽。
fn placements(
    views: Views,
    spec: &px_protocol::scene::SceneSpec,
    width: u32,
    height: u32,
) -> Result<(Vec<Placement>, (u32, u32)), String> {
    let aspect = width as f32 / height as f32;
    match views {
        Views::Single(cam) => Ok((
            vec![Placement {
                camera: crate::camera::probe_camera(cam, aspect),
                rect: None,
                index: 0,
                note: match cam {
                    Some([yaw, pitch, distance]) => {
                        format!("--cam {yaw},{pitch},{distance}")
                    }
                    None => "探针机位（不给 --cam）".to_string(),
                },
            }],
            (width, height),
        )),
        Views::Sheet { columns } => {
            if spec.cameras.is_empty() {
                // 与 Bevy 同一句话：它说的是"这一步没带相机表"，不是"这一版没做"。
                return Err(
                    "--sheet 用的是产物自带的相机表：这一步没带（烘场景时用 \
                     px_ops::cameras::review() 灌进 .pxart）"
                        .to_string(),
                );
            }
            let columns = columns.max(1);
            let rows = (spec.cameras.len() as u32).div_ceil(columns);
            let mut out = Vec::with_capacity(spec.cameras.len());
            for (index, camera) in spec.cameras.iter().enumerate() {
                let column = index as u32 % columns;
                let row = index as u32 / columns;
                out.push(Placement {
                    camera: crate::camera::review_camera(camera, aspect),
                    rect: Some([
                        column * width,
                        row * height,
                        width,
                        height,
                    ]),
                    index,
                    note: format!(
                        "第 {} 格（{columns} 列排开的第 {row} 行第 {column} 列）｜产物相机 '{}'",
                        index, camera.tag
                    ),
                });
            }
            Ok((out, (width * columns, height * rows)))
        }
    }
}

/// 跑这一帧。
///
/// ⚠ 四个「从哪儿来」的参数都是**调用方给的**，一个都不在这里取缺省：
///
/// - `pcg_root`：CAS 根。`--scene` 那条离线路传 [`art::default_pcg_root`]，服务那条路传
///   租约进程自己的 `--pcg-root`（**服务端说了算**，客户端的同名旗标只报一声 ——
///   解析成员的是渲染进程，这是协议里就定下的）。
/// - `views`：`Request::view`（`cam` / `sheet` / `columns`）。它是**怎么看**，所以住在请求上，
///   不住在文档里（§110.1：产物自带的相机表只给 `--sheet`）。
/// - `width` / `height`：**一格的**尺寸。对照图那张图是 `(格宽×列数) × (格高×行数)`，
///   格子怎么排是**渲染器的事**（`SheetCell` 的注释：相机表来自 `.pxart`，格子的排布是渲染器的）。
pub fn run(
    gpu: &Gpu,
    scene_path: &Path,
    pcg_root: &Path,
    views: Views,
    width: u32,
    height: u32,
) -> Result<Rendered, String> {
    let root = pcg_root.to_path_buf();
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
    let executed_plan = all_passes(&plan, &mut audit)?;

    // ---- 相机与格子：产物自带的那 12 台（`--sheet`）或者一台探针相机 ----
    let (placements, target) = placements(views, &spec, width, height)?;
    audit.push(format!(
        "怎么看：{:?}｜{} 台相机｜一格 {}×{}（aspect {}，位模式 {:08X}）⇒ 目标 {}×{}\
         ｜无限 reverse-Z / Depth32Float / 清 0.0 / GreaterEqual",
        views,
        placements.len(),
        width,
        height,
        width as f32 / height as f32,
        (width as f32 / height as f32).to_bits(),
        target.0,
        target.1
    ));
    for placement in &placements {
        audit.push(format!(
            "  {}｜from_xyz({}, {}, {}).looking_at(ZERO, Y)｜viewport {}",
            placement.note,
            placement.camera.position.x,
            placement.camera.position.y,
            placement.camera.position.z,
            match placement.uniform_viewport((width, height)) {
                [x, y, w, h] => format!("({x}, {y}, {w}, {h})"),
            }
        ));
    }

    // ---- 灯：文档那几盏 → 聚类缓冲的那几格（`group0::lights_of`，逐字复刻 oracle 的打包）----
    //
    // ⚠ 这里是**内容 → 数值**的那一步，不是"摆一盏灯"：有没有灯、哪一盏、开不开影子
    //    全部来自文档的 `lights`（§60：渲染器里没有 `SUN_DIRECTION` 这种常量）。
    //    宿主只负责把那些数按 oracle 的字段语义摆进 80 字节，次序也照 oracle 的排序键。
    let cluster = group0::lights_of(&spec.lights)?;
    audit.push(format!(
        "灯：文档 {} 盏 → 聚类缓冲前 {} 格（次序 = oracle 的排序键：开影子的在前，同档按文档次序）｜{}",
        spec.lights.len(),
        cluster.len(),
        if spec.lights.is_empty() {
            "（一盏都没有 ⇒ 整块全零 ⇒ 内容 shader 判 lit = false）".to_string()
        } else {
            spec.lights
                .iter()
                .map(|light| format!("{}（{:?}，影子 {}）", light.id, light.kind, light.shadows))
                .collect::<Vec<_>>()
                .join(" / ")
        }
    ));

    // ---- 执行器：**先建**，因为深度图要先 `seed` 进去（§132）----
    let mut executor = px_pass::Executor::new();

    // ---- 点光 cube 影图（§109）：**文档烘了就是它，没烘就是兜底的那份** ----
    //
    // ⚠ 为什么会有"没烘"这一档：一盏投影的点光都没有时，帧图里那份 cube 资源与那六条
    //    pass 一起不烘（`px_graphs::frame::build` 会把跳过的东西打印出来），而内容 shader
    //    **仍然声明**了 group 0 的 binding 2 ⇒ 管线布局必须有这一格、必须绑得上。
    //    那一档绑一份 1×1×6 全 0 的兜底图，并把"绑的是兜底"**打印出来** ——
    //    没有投影的灯时没有任何一条路会去采它（`surface.wgsl` 那个 `shadow_maps` 位）。
    let (shadow_texture, shadow_view, shadow_note) = match plan.resource(SHADOW_TEXTURE_RESOURCE) {
        Some(resource) => {
            let (cube_width, cube_height) = resource.size.resolve(target.0, target.1);
            let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(resource.name.as_str()),
                size: wgpu::Extent3d {
                    width: cube_width,
                    height: cube_height,
                    depth_or_array_layers: resource.layers,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: px_pass::texture_usage(resource),
                view_formats: &[],
            });
            // ⚠ 和深度预通道那两张一样：**一张纹理**，`seed` 进池子 —— 六条影子 pass 的
            //    附件（按 `PassPlan::layer` 建的单层视图）与 group 0 第 2 格（整份 cube 的
            //    `CubeArray` 视图）必须是**同一张纹理**的两个视图。各建一张就是静默错像素。
            executor.seed(resource, cube_width, cube_height, texture.clone())?;
            // 整份 cube 的视图：`CubeArray` + `DepthOnly`（`light.rs:1416-1444` 那一份）。
            let view = texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("点光 cube 影图（CubeArray/DepthOnly）"),
                format: None,
                dimension: Some(wgpu::TextureViewDimension::CubeArray),
                usage: None,
                aspect: wgpu::TextureAspect::DepthOnly,
                base_mip_level: 0,
                mip_level_count: None,
                base_array_layer: 0,
                array_layer_count: Some(resource.layers),
            });
            let note = format!(
                "文档烘的那份（{cube_width}×{cube_height} × {} 层 = {} 个 cube，每面一层，层号 = 灯×6 + 面）",
                resource.layers,
                resource.layers / group0::SHADOW_CUBE_FACES
            );
            audit.push(format!(
                "  ⚠ 池子里的 '{SHADOW_TEXTURE_RESOURCE}' 现在就是**宿主建的**这一张\
                 （{cube_width}×{cube_height} × {} 层）：六面各挂它的**单层**视图，\
                 group 0 第 2 格挂整份的 CubeArray 视图",
                resource.layers
            ));
            (Some(texture), view, note)
        }
        None => {
            let (texture, view) = group0::fallback_cube(&gpu.device);
            audit.push(format!(
                "⚠ 文档里没有 '{SHADOW_TEXTURE_RESOURCE}'：这一帧**没有投影的点光**\
                 （帧图把那份资源与那六条 pass 一起跳过了）⇒ group 0 第 2 格绑**兜底**的\
                 1×1×6 全 0 cube。它永远不会被采到（内容 shader 只在 `shadow_maps` 那一位\
                 立起来时才进 `fetch_point_shadow`，而那个位来自 `flags`）"
            ));
            (
                Some(texture),
                view,
                "**兜底**的 1×1×6 全 0 cube（这一帧没有投影的点光）".to_string(),
            )
        }
    };
    // ⚠ 纹理要活到这一帧画完（`wgpu::BindGroup` 持的是视图、视图持的是纹理 ——
    //    引用计数保证它不会先死；这里留一个绑定只是让"谁活着"这件事看得见）。
    let _shadow_texture = shadow_texture;
    let shadow_sampler = group0::point_shadow_sampler(&gpu.device);

    // ---- group 0 的契约：从**某一份物体 shader** 反射（五格超集的那份布局）----
    let contract = scene
        .objects
        .first()
        .ok_or_else(|| "场景里一个物体都没有：拿不到一份可反射的 shader".to_string())?;
    let contract_module = shader::validate(
        &format!("{}（group 0 契约）", contract.id),
        &contract.shader.assembled,
    )?;
    // ⚠ 每一格各建一份 group 0：`view` 那一格里**相机与 viewport 都是每格不同的**
    //    （`view.viewport` 是绝对矩形，见 `group0::frame`）。其余六格（灯 / 聚类 /
    //    globals / 影图 / 采样器 / 深度快照）内容一样，但绑定组是每格一个 ——
    //    "一份组给 12 格用"这件事在 wgpu 里不存在（组里的 uniform 就是那一格的）。
    let build_zero = |camera: &crate::camera::Camera,
                      view: &wgpu::TextureView,
                      viewport: [f32; 4]|
     -> Result<group0::GroupZero, String> {
        group0::frame(
            &gpu.device,
            &contract_module,
            camera,
            scene.ambient,
            &cluster,
            viewport,
            view,
            &shadow_view,
            &shadow_sampler,
            &shadow_note,
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
        // ⚠ 解析用的是**目标**尺寸（对照图那张整幅 3840×1920），不是一格的尺寸：
        //    `view` 这条尺寸规则说的是"跟这一帧的画布一样大"，而画布就是那张整幅 ——
        //    拿格子尺寸去建，12 格的中间目标就只剩 960×640，而 pass 的 attachment
        //    却要按格子视口画到整幅上（wgpu 当场拒，或者更糟：视口落到附件外面）。
        let (depth_width, depth_height) = resource.size.resolve(target.0, target.1);
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
    // ---- group 0 **每格一份**（相机与 viewport 都在它里面），所以这里不建 ----
    //
    // ⚠ 别把它提前建一份给 12 格用：`view.viewport` 是**每格不同**的绝对矩形，
    //    一份组给 12 格用 = 后 11 格的片元坐标反算全错（每格画成左上角那一格的视角），
    //    而画面上只是"格子里的东西位置偏了"。组 0 的建法见下面逐格那一段。
    audit.push(
        "group 0（七格全绑，哪怕 shader 只声明了一部分）：**每格一份**（相机与 `view.viewport` \
         都在里面），逐格建在下面那一段"
            .to_string(),
    );

    // ---- 点光 cube 的**每一面**：一套 view；那一面的 `PassView` 由 §148 那一节按面建 ----
    //
    // 哪几面要用，**由文档说了算**：带 `cube_face` 的那些 pass 点名的 (灯, 面)。一条都没有
    // ⇒ 这里一个面都不建（那一帧也就没有影子 pass）。
    //
    // ⚠ 这一档**不是**"同一个材质换一套组"（那需要执行器有一条 per-pass 的优先级规则，
    //    而那条路被用户裁决否掉了）：文档为每一面各生成了**自己的材质名**
    //    （`material_instances`：名字 → 照哪一份），宿主给每个名字建**一套组**。
    //    "一个名字恰好一套组"这条契约因此原样成立。
    //
    // ⚠ 面矩阵的**位模式**由 `camera::face_view` 负责（照抄 glam 的乘法链，§109.2）。
    //    这里只把结果绑出去，一个数都不重算。
    let face_keys: Vec<(u32, u32)> = {
        let mut keys: Vec<(u32, u32)> = executed_plan
            .passes
            .iter()
            .filter_map(|pass| {
                spec.passes
                    .iter()
                    .enumerate()
                    .find(|(index, document)| document.label_or(*index) == pass.label)
                    .and_then(|(_, document)| document.cube_face)
                    .map(|cube| (cube.light, cube.face))
            })
            .collect();
        keys.sort_unstable();
        keys.dedup();
        keys
    };
    let mut faces: Vec<Face> = Vec::with_capacity(face_keys.len());
    for (light, face) in face_keys {
        // 灯的位置：**从已经打包好的聚类缓冲里取**（按 cube 下标 —— 影子 pass 的 `light`
        // 就是那个下标）。不另查一次文档：两处各查一次就是"同一件事两个来源"。
        let clustered = cluster.get(light as usize).ok_or_else(|| {
            format!(
                "文档里有一条 cube_face 指着第 {light} 个 cube，而聚类缓冲只有 {} 格",
                cluster.len()
            )
        })?;
        let light_position = Vec3::new(
            clustered.position_radius[0],
            clustered.position_radius[1],
            clustered.position_radius[2],
        );
        // 影子的近平面那一格来自 group 0 的策略常量（`PointLight::shadow_map_near_z` 的缺省）。
        let face_camera = crate::camera::face_view(
            light_position,
            face,
            group0::POINT_LIGHT_SHADOW_MAP_NEAR_Z,
        );
        // 这一面的 group 0 **不建**：一来影子那一笔根本不会绑它（见下面实例那一段：
        // 绑了会撞 wgpu 的排他用法 —— 组 0 第 2 格就是这条 pass 正在写的 cube），
        // 二来顶点阶段读的是组 1 那一份 `PassView`（那一面的矩阵已经在里面了，§110 的
        // 那条"宿主算好、shader 只乘"）。所以这一面的信息全在 `camera` 里。
        let _ = &face_camera;
        let layer = light * group0::SHADOW_CUBE_FACES + face;
        audit.push(format!(
            "  面 (灯 {light}, 面 {face}, 层 {layer})：world_from_view 的平移 ({:.6}, {:.6}, {:.6})｜\
             clip_from_world 的 w 列 ({:.9e}, {:.9e}, {:.9e}, {:.9e})",
            face_camera.world_from_view.w_axis.x,
            face_camera.world_from_view.w_axis.y,
            face_camera.world_from_view.w_axis.z,
            face_camera.clip_from_world.w_axis.x,
            face_camera.clip_from_world.w_axis.y,
            face_camera.clip_from_world.w_axis.z,
            face_camera.clip_from_world.w_axis.w
        ));
        faces.push(Face {
            light,
            face,
            camera: face_camera,
        });
    }

    // ⚠ 这里建的是**面**（不是每个物体的矩阵）：§148 之后 `view_proj` 是按**视图**建的一份
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

    // ---- 材质：group 3 由 `material.rs` 反射建（空槽绑兜底白图），group 1 每视图一份 ----
    let mut materials = Materials::new(&gpu.device, &gpu.queue);
    let material_layout = materials.bind_group_layout().clone();
    // ⚠ group 1 的**布局只有一份**（所有视图共用同一份形状：`PassView` + 实例数组）：
    //    每个视图各建一份"内容相同但对象不同"的布局，会踩到 `ResolvedGroup::layout_id`
    //    那条契约的边界 —— 管线缓存键相同、而 wgpu 那边比的是布局对象本身。
    //    一份布局 + 每个视图一个绑定组，这两件事就都干净了。
    let stage_layout = gpu
        .device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("组 1：PassView + MeshInstance 数组"),
            entries: &[
                // binding 0：**super** —— 这一条 pass 的 `view_proj`。
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // binding 1：**instance** —— 长度 = 物体数的那份数组。
                //
                // ⚠ 地址空间跟着 oracle 走：Bevy 那份每实例数据是
                //    `var<storage> mesh: array<Mesh>`（`mesh_bindings.wgsl:9`）。
                //    两档读出来的浮点位模式一模一样 —— 选它**不是**为了性能，
                //    是为了下一次对账时不必先怀疑这里。
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

    // ---- 实例数组：**一份**（长度 = 物体数），全帧、所有视图共用（§142）----
    //
    // ⚠ "长度 = 物体数"是这一档的**形状**，不是省字节的顺手之作：执行器按
    //    `@builtin(instance_index)` 选格，而下标就是物体在 `objects[]` 里的次序
    //    （几何表也是照那个次序建的 ⇒ 两者同源）。
    let mut instance_stream: Vec<u8> = Vec::with_capacity(scene.objects.len() * 112);
    for object in &scene.objects {
        instance_stream.extend_from_slice(&instance_bytes(&object.transform));
    }
    let instance_buffer = gpu
        .device
        .create_buffer_init(&BufferInitDescriptor {
            label: Some("组 1：MeshInstance 数组（长度 = 物体数）"),
            usage: wgpu::BufferUsages::STORAGE,
            contents: &instance_stream,
        });

    // ---- 每个**视图**一份 `PassView`（相机一份 + 影子每个面一份），实例数组是同一份 ----
    //
    // ⚠ 这一节就是"super 在 pass 配"的落地：`view_proj` 与物体无关（同一个视图下所有
    //    物体共用它），所以它按**视图**建，不按 (物体, 视图) 建。视图是哪一个由
    //    **文档的 pass** 说（`cube_face` → 那一面；没有就是相机）—— 见下面 `faces`。
    let make_stage = |label: &str, view: &crate::camera::Camera| -> Stage {
        let buffer = gpu
            .device
            .create_buffer_init(&BufferInitDescriptor {
                label: Some(label),
                usage: wgpu::BufferUsages::UNIFORM,
                contents: &view_bytes(view),
            });
        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &stage_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: instance_buffer.as_entire_binding(),
                },
            ],
        });
        Stage {
            _view: buffer,
            bind_group,
        }
    };
    // 影子那几面（`face_stages[k]` 与 `faces[k]` 同一次序 —— `face_of` 查的就是它）。
    //
    // ⚠ 面那一份 `PassView` **与格子无关**：影子图是灯的东西（六面各自的矩阵来自灯位），
    //    12 台相机共用同一张 cube ⇒ 每一格重画一遍影子图，画出来的字节也一模一样。
    //    所以它建一次、12 格共用（相机那一份 `PassView` 才是每格一份的）。
    let face_stages: Vec<Stage> = faces
        .iter()
        .map(|face| {
            make_stage(
                &format!("组 1：PassView（灯 {} 面 {}）", face.light, face.face),
                &face.camera,
            )
        })
        .collect();
    // 相机那一份 `PassView` 是**每格一份**的（见下面逐格那一段），所以这里不建。
    audit.push(format!(
        "组 1（§142 的两类参数）：**super** = 每视图一份 `PassView`（{} 份 × 64 B = {} B：\
         相机 **每格一份**（{} 格）+ 影子面 {}）｜**instance** = 全帧**一份**实例数组\
         （{} 个物体 × 112 B = {} B，按 `@builtin(instance_index)` 选格）。拆之前是每 \
         (物体, 视图) 一份 176 B 的 `MeshStage`（{} 份 = {} B）⇒ 数据从 (物体 × 视图) 那一维上下来了",
        faces.len() + placements.len(),
        (faces.len() + placements.len()) * 64,
        placements.len(),
        faces.len(),
        scene.objects.len(),
        instance_stream.len(),
        (faces.len() + placements.len()) * scene.objects.len(),
        (faces.len() + placements.len()) * scene.objects.len() * 176,
    ));

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
        bindings.push(binding);
    }

    // ---- 执行器要的那几张表 ----
    let resolved_geometry: Vec<ResolvedGeometry<'_>> = geometries
        .iter()
        .enumerate()
        .map(|(index, geometry)| ResolvedGeometry {
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
            // ⚠ 实例下标 = **物体在 `objects[]` 里的次序**，而几何表就是照那个次序建的
            //    （实例数组也是）⇒ 两者同源，"下标越界"在结构上不可能发生。
            //    一格宽的区间 ⇒ `instance_index` 恒为 `index`：每个算式与拆之前逐位相同。
            //
            // ⚠ 程序化那一笔（天空盒）**不是物体**：它画一次（`instance_index` 恒 0），
            //    而它的顶点阶段一个 `@group(1)` 都不读（顶点全由 `vertex_index` 现算）。
            //    `0..1` 说的是这件事本身，不是一个"兜底默认值"。
            instances: if index < scene.objects.len() {
                index as u32..index as u32 + 1
            } else {
                0..1
            },
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

    // ---- 外部目标：宿主这一帧那张图（**整幅**：对照图就是 12 格拼出来的那一张）----
    let host_target = shot::Target::new(&gpu.device, target.0, target.1);
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
                view: &host_target.view,
                format: shot::FORMAT,
            });
        }
        sets.push(set);
    }

    // ---- 逐格执行：每一格一套 group 0 / `PassView` / 材质表，格子交给 `Frame::viewport` ----
    //
    // ⚠ 12 次执行共用**同一个执行器**（池子与管线缓存都是它的）：12 格因此共用同一批
    //    中间纹理（`scene_color_a/b` 与两张深度）。那正是 oracle 的形状 —— Bevy 的 12 台相机
    //    虽然各有 `ViewTarget`，但主纹理是按**目标**（`camera.target`）从纹理池里取的
    //    （`bevy_render-0.19.1/src/view/mod.rs:1253-1284` 的 `MainTextureKey`：键里没有 viewport），
    //    12 台相机同指一张 Image ⇒ **同一对 a/b 纹理**，各自 `set_viewport` 画自己那一格。
    //
    // ⚠ 每一格都把**整份计划**跑完（含 prepass / 影子 / copy / blit）：少跑一条就是
    //    "有些格子的图没画全"，而那种错不会有任何门响。
    //
    // ⚠ 一格清一次 `scene_color_a` 是安全的：清屏发生在**上一格的 blit 已经记进编码器之后**
    //    （命令按记录次序执行），而中间目标里的内容本来就只在"这一格的那一块"有意义。
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("px_render_wgpu 的一帧（一格一次）"),
        });
    for placement in &placements {
        let viewport = placement.uniform_viewport((width, height));
        let zero = build_zero(&placement.camera, &sampled_view, viewport)?;
        audit.push(format!(
            "—— {} 的 group 0（`view.viewport` = ({}, {}, {}, {})）——",
            placement.note, viewport[0], viewport[1], viewport[2], viewport[3]
        ));
        audit.extend(zero.audit.iter().map(|line| format!("  {line}")));
        // 这一格的 `PassView`：`view_proj` 是"每一条 pass 一份"的 super，而它是每格一份的。
        let camera_stage = make_stage(
            &format!("组 1：PassView（相机，第 {} 格）", placement.index),
            &placement.camera,
        );
        // ⚠ 那些守卫（"一个名字恰好一套组"那三条）判的是**文档说不说得通**，与格子无关：
        //    第 0 格把话说进 `audit`，其余各格写进一份丢弃的 Vec —— 守卫照跑（坏了当场拒），
        //    话只说一遍（每条实例一行 × 12 格会把审计淹掉）。
        let mut quiet: Vec<String> = Vec::new();
        let audit_sink: &mut Vec<String> = if placement.index == 0 {
            &mut audit
        } else {
            &mut quiet
        };
        let cell = cell_materials(
            &zero,
            &camera_stage,
            &scene,
            &bindings,
            &spec,
            &executed_plan,
            &faces,
            &face_stages,
            &stage_layout,
            &material_layout,
            &frame_materials,
            audit_sink,
        )?;
        let frame = Frame {
            // ⚠ **整幅**的尺寸：池子里那些 `size = "view"` 的资源按它建（对照图是 3840×1920），
            //    每一格靠 `frame.viewport` 落到自己那一块上。
            width: target.0,
            height: target.1,
            viewport: placement.viewport(),
            sets: &sets,
            geometries: &resolved_geometry,
            materials: &cell,
        };
        let audit_text = executor.execute(&gpu.device, &mut encoder, &executed_plan, &frame)?;
        audit.push(format!("第 {} 格的执行器审计：\n{audit_text}", placement.index));
    }
    // ⚠ 实例数组要活到这一帧画完（`wgpu::BindGroup` 持的是它的引用 —— 引用计数保证它不会
    //    先死）。这一行只是让"谁活着"这件事看得见（同上面那条 `_shadow_texture`），
    //    而且它必须落在 `make_stage` 最后一次被调用**之后**：`make_stage` 那个闭包借的
    //    就是这块缓冲，提前 move 会变成编译错误（本单元实测：E0505）。
    let _instance_buffer = instance_buffer;
    gpu.queue.submit(Some(encoder.finish()));

    let pixels = shot::read_back(&gpu.device, &gpu.queue, &host_target)?;
    Ok(Rendered {
        pixels,
        width: target.0,
        height: target.1,
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
        declared_clouds: spec.expects.iter().any(|tag| tag == "clouds"),
    })
}

/// 一格的材质表：名字 → 那几套组（`zero` 与 `camera_stage` 是**这一格**的那两份）。
///
/// ⚠ 为什么它是**一个函数**而不是 `run` 里的一段：J2 的对照图要同一份文档画 12 格，
/// 而"名字 → 一套组"这张表是**每格一份**的（组 0 的 `view`、组 1 的 `PassView` 都在里面）。
/// `zero` / `camera_stage` 之外的几张表（材质的绑定、帧自有材质、影子六面的组）与格无关，
/// 由调用方建一次、逐格传进来。
///
/// ⚠ 里面那三条守卫（"一个名字恰好一套组"那一族）判的是**文档说不说得通**，与格子无关：
/// 调用方对第 0 格把审计写进真 `audit`，其余各格写进一份丢弃的 Vec —— 守卫照跑（坏了当场拒），
/// 话只说一遍（12 格 × 每条实例一行会把审计淹掉）。
#[allow(clippy::too_many_arguments)]
fn cell_materials<'a>(
    zero: &'a group0::GroupZero,
    camera_stage: &'a Stage,
    scene: &'a art::LoadedScene,
    bindings: &'a [material::MaterialBinding],
    spec: &'a px_protocol::scene::SceneSpec,
    plan: &'a Plan,
    faces: &'a [Face],
    face_stages: &'a [Stage],
    stage_layout: &'a wgpu::BindGroupLayout,
    material_layout: &'a wgpu::BindGroupLayout,
    frame_materials: &'a [(art::LoadedFrameMaterial, material::MaterialBinding)],
    audit: &mut Vec<String>,
) -> Result<Vec<ResolvedMaterial<'a>>, String> {
    // ⚠ 一笔 draw 的组：**group 0 + group 1 + group 3**。group 1 现在是**每个视图**一份的
    //    （§142：super = 那一条 pass 的 `view_proj`；instance = 全帧那一份数组，
    //    靠 `@builtin(instance_index)` 选格），所以内容材质那一档**共用同一份组 1** ——
    //    "解析好的材质"仍然按**物体**给（名字是物体 id），因为**混合/剔除/片元阶段**是
    //    每个物体一份的（§127/§129），而组 1 不是。
    //    ⚠ 实例下标**不在这里**：它挂在几何那一格上（见上面 `resolved_geometry`）——
    //    一笔 draw 的三个数（顶点、索引、实例区间）住在一起。
    //    真出现"一条 pass 里同一个物体画两笔、两笔要不同实例区间"的那天，这里要当场拒 ——
    //    几何是按**名字**查的，一个名字给不出两个区间。
    //
    // ⚠ 帧自有材质**没有 group 1**：它的顶点阶段是程序化的（`vertex_index` 现算），
    //    组 1 那两格它一格都不读。少给一组不是省事 —— 给了它反而要求那段
    //    顶点阶段认一个它不认的布局。
    let mut resolved_materials: Vec<ResolvedMaterial<'_>> = scene
        .objects
        .iter()
        .zip(bindings.iter())
        .map(|(object, binding)| {
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
                        // ⚠ **所有物体共用同一份**：组 1 现在是"相机那一份 `PassView`
                        //    + 全帧那一份实例数组"，与物体无关 —— 每个物体各建一份
                        //    "只有绑的对象不同"的组，就是拆之前那个形状的残渣。
                        bind_group: &camera_stage.bind_group,
                        // 一份布局，所有视图共用（见上面那段）。
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

    // ---- 生成的材质实例（§139）：**影子六面各一套组 1（那一面的 `PassView`）** ----
    //
    // ⚠ 这一节是"文档里那些不同的名字"的落地：每一份实例的名字在宿主这张表里
    //    **恰好一条**（执行器只有"名字 → 一套组"这一条规则，没有覆盖、没有优先级）。
    //    名字照的是哪一份材质，由文档的 `material_instances` 说；而"哪一面"由
    //    **用到这个名字的那条 pass** 说（`cube_face`）—— 一个事实一处。
    //
    // ⚠ 三条当场拒（都是"猜一个就会静默画错"的形状）：
    //    ① 同一个实例名被**两条面不同的 pass** 用（一个名字两套组 ⇒ 说不清）；
    //    ② 带 `cube_face` 的 pass 用了一个**不是实例**的材质名（它会拿到相机的
    //       那一份 `PassView` ⇒ 影子贴到相机那面去，而画面上只是"影子歪了"）；
    //    ③ 实例的 `base` 与那一笔的 `geometry` 不同名 —— ⚠ **拆分之后这条的病因换了**
    //       （见下面那条注释：矩阵现在按**几何**的实例下标选，不再按 base 选），
    //       但它拦的仍然是一件真事：这一笔会拿 A 的几何配 B 的剔除/混合/片元档。
    if !spec.material_instances.is_empty() {
        let face_of = |light: u32, face: u32| -> Result<usize, String> {
            faces
                .iter()
                .position(|candidate| candidate.light == light && candidate.face == face)
                .ok_or_else(|| format!("这一面（灯 {light}，面 {face}）没有建出来（内部不一致）"))
        };
        for instance in &spec.material_instances {
            // 用到这个名字的 pass（应当恰好一条带 cube_face 的）。
            let users: Vec<(&str, Option<px_protocol::scene::PassCubeFace>)> = plan
                .passes
                .iter()
                .filter_map(|pass| {
                    if !pass.draws.iter().any(|draw| draw.material == instance.name) {
                        return None;
                    }
                    let document = spec
                        .passes
                        .iter()
                        .enumerate()
                        .find(|(index, document)| document.label_or(*index) == pass.label)
                        .map(|(_, document)| document);
                    Some((pass.label.as_str(), document.and_then(|d| d.cube_face)))
                })
                .collect();
            let cube = match users.as_slice() {
                [] => {
                    return Err(format!(
                        "生成的材质实例 '{}' 没有任何一条 pass 用它（文档那边应当已经拒过）",
                        instance.name
                    ))
                }
                [(label, Some(cube))] => {
                    audit.push(format!(
                        "实例 '{}' ⇒ 照 '{}'，用在 pass '{label}' 上 ⇒ 那一面的 PassView\
                         （灯 {}，面 {}，层 {}）",
                        instance.name, instance.base, cube.light, cube.face, cube.layer
                    ));
                    *cube
                }
                [(label, None)] => {
                    return Err(format!(
                        "实例 '{}' 被 pass '{label}' 用了，而那条 pass 没有 `cube_face`：\
                         实例是要照某一面的 PassView 建的，没有那一面就没有依据",
                        instance.name
                    ))
                }
                many => {
                    let faces: Vec<String> = many
                        .iter()
                        .map(|(label, cube)| match cube {
                            Some(cube) => format!("{label}（灯 {}，面 {}）", cube.light, cube.face),
                            None => format!("{label}（没有 cube_face）"),
                        })
                        .collect();
                    return Err(format!(
                        "实例 '{}' 被 {} 条 pass 用了：{}。一个名字只能解析出**一套组** —— \
                         两处用、面不同就是「哪一面赢」说不清（那正是被否掉的优先级规则）",
                        instance.name,
                        many.len(),
                        faces.join(" / ")
                    ));
                }
            };
            let face_index = face_of(cube.light, cube.face)?;
            // `base` 必须在两张表里找得到（协议已经判过；这里再判一次是因为**要建组**，
            // 而建组那一刻的失败离病因太远）。
            let base_index = resolved_materials
                .iter()
                .position(|material| material.name == instance.base)
                .ok_or_else(|| {
                    format!(
                        "实例 '{}' 照的那份 '{}' 不在材质表里（表里有：{}）",
                        instance.name,
                        instance.base,
                        resolved_materials
                            .iter()
                            .map(|material| material.name)
                            .collect::<Vec<_>>()
                            .join(" / ")
                    )
                })?;
            let base = &resolved_materials[base_index];
            let object_index = scene
                .objects
                .iter()
                .position(|object| object.id == instance.base);
            // 每一笔用它的 draw 都要 base == geometry（③）。
            //
            // ⚠ **拆分之后这条守卫的病因换了，措辞也得跟着换**（§141 那一族教训：
            //    一条响得对、说得不对的守卫与不响的守卫一样贵）：矩阵**不再按 base 选**
            //    ——实例下标挂在**几何**那一格上（`resolved_geometry`），所以"另一个物体的
            //    位置"这件事已经不可能发生。它现在拦的是：这一笔拿 **A 的几何**配
            //    **B 的剔除/混合/片元档**（那三样来自实例照的那份材质）。今天两者都是
            //    "投影的那个物体"，写错时画面上只是"影子的剔除反了"，没有任何别的门会响。
            for pass in plan.passes.iter() {
                for draw in &pass.draws {
                    if draw.material != instance.name {
                        continue;
                    }
                    if draw.geometry != instance.base {
                        return Err(format!(
                            "pass '{}' 拿实例 '{}'（照 '{}'）去画几何 '{}'：名字是照**物体**\
                             生成的，而这一笔的剔除/混合/片元阶段来自 '{}'、几何与实例下标来自\
                             '{}' —— 两者指不同的物体时，这一笔就是「用 A 的网格跑 B 的材质档」",
                            pass.label, instance.name, instance.base, draw.geometry, instance.base,
                            draw.geometry
                        ));
                    }
                }
            }
            // 一套新的组：**只留组 1**（那一面的 `PassView` + 那份全帧实例数组）。
            //
            // ⚠ 组 0 **不能绑**：它的第 2 格就是这条 pass 正在写的 cube，绑上它 wgpu
            //    当场拒（排他用法）。而影子那一笔没有片元阶段 ⇒ 组 0 那几格（view/lights/
            //    cluster/影图/预通道深度）一个都不会被读。组 3（材质的参数与贴图）同理：
            //    没有片元阶段，没人读它。**只留真正被读的那一组** —— 这一笔的管线布局
            //    因此与内容材质那一笔不同，那是应该的（它们是两条不同的管线）。
            //
            // ⚠ 这一份组 1 是**那一面的**（六面各一份）：`view_proj` 是 super、按 pass 配，
            //    而实例数组是同一份 ⇒ 六个面画的都是那份数组，只是矩阵不同。
            if object_index.is_none() {
                return Err(format!(
                    "实例 '{}' 照的是帧自有材质 '{}'：生成的实例只能照**物体**的材质\
                     （`material_instances` 是烘图侧按「哪个物体投影」生成的），\
                     照一份帧自有材质（天空盒那种）会静静地拿到它的剔除档去投影",
                    instance.name, instance.base
                ));
            }
            let groups = vec![ResolvedGroup {
                group: 1,
                bind_group: &face_stages[face_index].bind_group,
                layout: stage_layout.clone(),
                layout_id: STAGE_LAYOUT_ID,
            }];
            resolved_materials.push(ResolvedMaterial {
                name: instance.name.as_str(),
                groups,
                blend: base.blend,
                cull: base.cull,
                fragment_shader: base.fragment_shader,
                fragment_entry: base.fragment_entry,
            });
        }
        // ② 带 `cube_face` 的 pass 只许用实例名（见上面 ② 那段）。
        for pass in plan.passes.iter() {
            let Some(document) = spec
                .passes
                .iter()
                .enumerate()
                .find(|(index, document)| document.label_or(*index) == pass.label)
                .map(|(_, document)| document)
            else {
                continue;
            };
            if document.cube_face.is_none() {
                continue;
            }
            for draw in &pass.draws {
                if draw.material.is_empty() {
                    continue;
                }
                if !spec
                    .material_instances
                    .iter()
                    .any(|instance| instance.name == draw.material)
                {
                    return Err(format!(
                        "pass '{}' 带 `cube_face`（写 cube 的某一层），而它的 draw 用的是材质\
                         '{}' —— 那不是生成的材质实例：这一笔会拿到**相机**的那一份 PassView，\
                         影子会贴到相机那一面去（画面上只是「影子歪了」）。\
                         带 cube_face 的 pass 只能用 `material_instances` 里的名字",
                        pass.label, draw.material
                    ));
                }
            }
        }
    }

    Ok(resolved_materials)
}

/// 交给执行器的那一份计划：**文档声明的每一条 pass**（见 [`EXECUTED`] 那一段）。
///
/// ⚠ 返回的仍然是 `px_pass::Plan`（资源表、布局、`PassPlan` 全是原来那几份）：执行器
/// 看不到"切片"这回事。
///
/// ⚠ 这里仍然把"没执行的"算一遍并打出来 —— 今天恒为空，而**那一条纪律不因为今天没得跳
/// 就作废**：一个读数"绿"的原因如果是"那几条根本没跑"，它一文不值（§107）。
fn all_passes(plan: &Plan, audit: &mut Vec<String>) -> Result<Plan, String> {
    let executed: Vec<&str> = plan
        .passes
        .iter()
        .map(|pass| pass.label.as_str())
        .collect();
    audit.push(format!(
        "本切片执行的 pass：{}（{} / {}，**按文档全执行**）",
        executed.join(" → "),
        executed.len(),
        plan.passes.len()
    ));
    let plan = plan.clone();
    plan.check()
        .map_err(|err| format!("交给执行器的那一份计划说不通：{err}"))?;
    Ok(plan)
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

    // -----------------------------------------------------------------------
    // 对照图（J2）：**相机表 → 一台台相机 + 它们各自那一块**
    //
    // ⚠ 这一档的判据必须在**仓库里**（不像 §148 那样只在 target/ 下的仪器里）：
    //    "第 i 台相机落在第几行第几列、目标多大"是**渲染器的策略**，
    //    而它与 oracle 的关系只有一条（判据那张图的哈希）；策略本身的形状要能被单测钉住。
    // -----------------------------------------------------------------------

    /// 12 台相机、4 列 ⇒ 4×3 格、每格 960×640、目标 3840×1920，**行优先**。
    #[test]
    fn the_sheet_is_four_columns_of_three_rows_for_twelve_cameras() {
        let mut spec = spec_of(&["planet"], &[]);
        spec.cameras = (0..12)
            .map(|index| {
                px_protocol::art::Camera::raw([0.0, 0.0, 1.0], 3.15, format!("c{index}"))
            })
            .collect();
        let (placements, target) = placements(Views::Sheet { columns: 4 }, &spec, 960, 640)
            .expect("12 台相机排得下");
        assert_eq!(target, (3840, 1920));
        assert_eq!(placements.len(), 12);
        for (index, placement) in placements.iter().enumerate() {
            let column = (index % 4) as u32;
            let row = (index / 4) as u32;
            assert_eq!(
                placement.rect,
                Some([column * 960, row * 640, 960, 640]),
                "第 {index} 台相机落在第 {row} 行第 {column} 列"
            );
            // 内容 shader 看到的 `view.viewport` 是**绝对**矩形（片元坐标也是绝对的）。
            assert_eq!(
                placement.uniform_viewport((960, 640)),
                [
                    (column * 960) as f32,
                    (row * 640) as f32,
                    960.0,
                    640.0
                ]
            );
        }
    }

    /// 单张那条路：一台相机、整幅、`viewport = None`（执行器一个调用都不发）。
    #[test]
    fn a_single_view_has_no_cell() {
        let spec = spec_of(&["planet"], &[]);
        let (placements, target) = placements(Views::Single(None), &spec, 960, 640).unwrap();
        assert_eq!(target, (960, 640));
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].rect, None);
        assert_eq!(placements[0].viewport(), None, "单张那条路不设 viewport");
        assert_eq!(
            placements[0].uniform_viewport((960, 640)),
            [0.0, 0.0, 960.0, 640.0],
            "`view.viewport` 仍然是整幅（与加格子之前逐字节相同）"
        );
    }

    /// 产物**没带相机表** ⇒ 拒，而且拒词说的是"这一步没带"（与 Bevy 逐字同一条），
    /// 不是"这一版没做"。
    #[test]
    fn a_sheet_without_a_camera_table_names_the_real_reason() {
        let spec = spec_of(&["planet"], &[]);
        // ⚠ 不用 `unwrap_err()`：那要求 `Ok` 那一半实现 `Debug`，而 `Placement` 里是相机
        //    （`camera::Camera` 没有 `Debug`，也不该为了一个测试给它加一个）。
        let Err(why) = placements(Views::Sheet { columns: 4 }, &spec, 960, 640) else {
            panic!("没有相机表时 --sheet 必须当场拒");
        };
        assert!(why.contains("产物自带的相机表"), "{why}");
        assert!(why.contains("px_ops::cameras::review()"), "要指路：{why}");
    }

    /// 列数 = 0 兜成 1（协议那一栏的缺省就是 0）：12 台相机 ⇒ 1 列 12 行，
    /// 目标 960×7680。**兜底只有这一处** —— Bevy 也是 `.max(1)`。
    #[test]
    fn zero_columns_fall_back_to_one_column() {
        let mut spec = spec_of(&["planet"], &[]);
        spec.cameras = (0..12)
            .map(|index| {
                px_protocol::art::Camera::raw([0.0, 0.0, 1.0], 3.15, format!("c{index}"))
            })
            .collect();
        let (placements, target) = placements(Views::Sheet { columns: 0 }, &spec, 960, 640).unwrap();
        assert_eq!(target, (960, 7680));
        assert_eq!(placements[7].rect, Some([0, 7 * 640, 960, 640]));
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

    /// **三张表都没有** ⇒ 拒，而且**三张表都要列出来**（只说"找不到"会让人去改错的那一张）。
    ///
    /// ⚠ 下面那条 `contains("三张表里都没有")` 是**故意钉住表数**的：
    /// 加第四张表却忘了改措辞时，它会当场变红 —— 这不是脆弱，这是它该做的。
    /// （它已经响过一次：`material_instances` 加进来时这条先红了。）
    #[test]
    fn a_name_in_neither_table_is_refused_with_all_lists() {
        let spec = spec_of(&["planet"], &["skybox"]);
        let err = material_table(&spec, "skybx").expect_err("三张表都没有 ⇒ 拒");
        assert!(err.contains("planet"), "要列出物体 id：{err}");
        assert!(err.contains("skybox"), "要列出帧自有材质：{err}");
        assert!(err.contains("生成的实例"), "要列出生成的实例表：{err}");
        assert!(err.contains("三张表里都没有"), "{err}");
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
