//! group 0 的**宿主侧**缓冲：`view` / `lights` / `globals` / `clustered_lights`。
//!
//! 这一篇只做一件事：把「shader 声明了什么」变成「缓冲里该放什么字节」。三条口径：
//!
//! 1. **结构体的真本是组装出来的 WGSL**（`stubs.rs` 的桩表就是本宿主的 group 0 契约）。
//!    Rust 侧这几个 `#[repr(C)]` 结构体只是让人能**按名字**填值；它们与 WGSL 的偏移 / 大小 /
//!    成员类型是否一致，由 [`struct_layout`] 从组装后的文本里**反射出来当场对账**
//!    （`cargo test` 里那几条判据）。在 Rust 里再抄一份偏移 = §66.1 那颗
//!    「同一条契约、两个数字」的雷：漂开的那一天画面会变，而任何门都不会响。
//! 2. **绑定号照抄 Bevy**（§104 第 1 条）：`view`=0、`lights`=1、`clustered_lights`=8、
//!    `globals`=11、`depth_prepass_texture`=20。绑定号**会改像素**（§65 记的
//!    「cube 从第 1 格挪到第 5 格就差 22–33 个像素」至今没归因），所以这一组数在 Rust 里
//!    只有**这一份**，并由测试钉在 shader 上 —— 不许多一处、也不许少一处。
//! 3. 这一档用不到的格**填零**，而「零 = 这盏灯不存在」是**内容 shader 自己判的**
//!    （`light.wgsl`：`lit = !all(color_inverse_square_range.rgb == 0)`）——
//!    宿主不认识「太阳」，也不替它兜底（§64.9：宇宙里没有平行光，没有点光源就是没有光）。

use bytemuck::{Pod, Zeroable};
use wgpu::util::{BufferInitDescriptor, DeviceExt};

use crate::mat4::{Mat4, Vec4};

// ---------------------------------------------------------------------------
// 四个结构体：字段与 `stubs.rs` 的 WGSL 声明**逐字对应**
// ---------------------------------------------------------------------------

/// `view`（group 0 binding 0）—— 对应桩里的 `ViewStub`。
///
/// ⚠ 只声明**内容 shader 真读的**那几个字段。Bevy 真正的 `View` uniform 有七十多个字段，
/// 本宿主一个都不需要：桩表给的就是这一份，多写一格就是多一处会漂的数。
/// `world_position` 是 `vec3` 紧跟着 `exposure: f32`（偏移 0 / 12）—— 那不是"省了 4 个字节"，
/// 那就是 WGSL 的布局（`vec3` 对齐 16、大小 12）。
///
/// ⚠ **这是本宿主自己的布局，不是 Bevy 的 `View`**。两者只是**绑定号**相同
/// （§104 第 1 条：绑定号会改像素）：Bevy 那个是 `world_from_view / view_from_world /
/// clip_from_view / view_from_clip / world_position / exposure / viewport / main_pass_viewport /
/// frustum / lod_view_world_position / color_grading / mip_bias / frame_count`，七十多个字段；
/// 这一份是它按本工程用量的**子集**，而且次序是按"哪几格先要用"排的。所以：
/// **字段加在哪里，谁都不许按 Bevy 那份去推** —— 这一份的真本是组装出来的 WGSL
/// （[`px_shader::assemble::HOST_VIEW_STUB`] 就是它），由下面那些判据逐格对账。
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct ViewUniform {
    /// 相机在**世界系**里的位置（`surface.wgsl` / `atmosphere.wgsl` 都从它算视线）。
    pub world_position: [f32; 3],
    /// `Exposure::default()`（EV100 = 9.7）＝ [`exposure`]。内容 shader 自己乘它。
    pub exposure: f32,
    /// `view_from_world`（列主序，就是 `Mat4` 那 16 个数）——**世界 → 视图**那条逆。
    pub view_from_world: [[f32; 4]; 4],
    /// `clip_from_view`（无限 reverse-Z 右手投影）。
    pub clip_from_view: [[f32; 4]; 4],
    /// **绝对像素矩形** `(x, y, w, h)`：片元坐标也是绝对的，`clouds` / `atmosphere` 依赖这一点。
    pub viewport: [f32; 4],
    // ---- 下面两格是§135（天空盒）加的，**一律追加在末尾**：------------------------
    //
    // ⚠ 新字段只能加在**末尾**（这里就是 `viewport` 之后）。WGSL 结构体的偏移由声明次序决定，
    // 而内容 shader 是**按名字**读那五格的：把新字段插在中间不会报错、只会让
    // `view.viewport` 读到别的字节 —— 逐字节判据红，而画面看起来"背景偏了一点"。
    // 追加在末尾 ⇒ 前五格的偏移一个都不动（判据里钉着 0 / 12 / 16 / 80 / 144 这五个数）。
    /// `clip_from_view` 的逆：**裁剪 → 视图**。天空盒的片元阶段用它把片元坐标还原成视线方向
    /// （`coords_to_ray_direction`，`bevy_core_pipeline-0.19.1/src/skybox/skybox.wgsl:50-55`）。
    ///
    /// ⚠ 它必须来自 [`crate::mat4::inverse`] 那个**逐位移植的通用逆**，不许换成解析逆
    /// （§110.1.1 实测：解析刚体逆与通用逆差 1–2 ulp），也不许搬进 shader ——
    /// "数学等价"在逐字节判据下不是等价。
    pub view_from_clip: [[f32; 4]; 4],
    /// 相机位姿（**视图 → 世界**）。同样是天空盒要的：方向要从视图系转回世界系。
    ///
    /// ⚠ 它与 `view_from_world` 不是互为倒数的两处写法：这一格是 `camera.rs` 里
    /// `from_scale_rotation_translation` 直接拼出来的那个矩阵，而 `view_from_world` 是
    /// 它的**通用逆**回去的那个数 —— 两个方向都要，因为两个方向算出来的东西不逐位互逆。
    pub world_from_view: [[f32; 4]; 4],
}

/// `lights`（group 0 binding 1）—— 对应桩里的 `LightsStub`。
///
/// ⚠ 桩里**只有** `ambient_color`：平行光已经从渲染器里删掉了（§64.9），
/// 谁再想读 `directional_lights` 就该在离线门上直接报错。
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct LightsUniform {
    /// 环境光的**线性**色 × 强度。这个场景是 `vec4(80, 80, 80, 80)`（`environment.ambient = 80`）。
    pub ambient_color: [f32; 4],
}

/// `globals`（group 0 binding 11）—— 对应桩里的 `GlobalsStub`。
///
/// 只有 `globals.time` 会被内容 shader 读（`clouds.wgsl` 的细节风）。
/// ⚠ 四档内容里 `wind = wind_skin = 0` ⇒ 这一格与时间无关（只有 `orbit-soft-wind` 那一档不是）。
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GlobalsUniform {
    pub time: f32,
    pub delta_time: f32,
    pub frame_count: u32,
}

/// 一盏点光源在聚类缓冲里的那 80 字节 —— 对应桩里的 `ClusteredLightStub`。
///
/// ⚠ **不是 64**：三块 `vec4`（48）＋ 8 个 4 字节标量（32）＝ **80**，
/// naga 算出来的数组步长也是 80（`vec4` 把结构体对齐顶到 16）。
/// 判据在 [`tests::the_storage_array_stride_is_the_light_struct`]：步长从 shader 里反射出来，
/// 跟这个结构体对 —— 数目写在笔记里会漂，从 shader 里读不会。
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct ClusteredLight {
    pub light_custom_data: [f32; 4],
    /// `rgb` = 颜色 × 强度（点光源的流明要除 4π）；`w` = 1/range²。
    /// ⚠ 判「这盏灯在不在」只能看**颜色**（`light.wgsl` 记过：`position_radius.w` 不是 range）。
    pub color_inverse_square_range: [f32; 4],
    /// `xyz` = 世界位置；`w` = 射程。
    pub position_radius: [f32; 4],
    pub flags: u32,
    pub shadow_depth_bias: f32,
    pub shadow_normal_bias: f32,
    pub spot_light_tan_angle: f32,
    pub soft_shadow_size: f32,
    pub shadow_map_near_z: f32,
    pub decal_index: u32,
    pub range: f32,
}

// ---------------------------------------------------------------------------
// 绑定号：本宿主里**唯一**的一份（判据把它钉在 shader 上）
// ---------------------------------------------------------------------------

/// `view` 的 `(group, binding)`。
pub const VIEW_BINDING: (u32, u32) = (0, 0);
/// `lights` 的 `(group, binding)`。
pub const LIGHTS_BINDING: (u32, u32) = (0, 1);
/// `clustered_lights` 的 `(group, binding)`（**storage**）。
pub const CLUSTERED_LIGHTS_BINDING: (u32, u32) = (0, 8);
/// `globals` 的 `(group, binding)`。
pub const GLOBALS_BINDING: (u32, u32) = (0, 11);
/// `depth_prepass_texture` 的 `(group, binding)`（`texture_depth_2d`，`atmosphere.wgsl` 读它）。
/// 它不进缓冲，但它是 group 0 契约的一部分，所以和上面几个一起记在这儿。
pub const DEPTH_PREPASS_BINDING: (u32, u32) = (0, 20);

// ---------------------------------------------------------------------------
// 值 → 字节
// ---------------------------------------------------------------------------

/// `Exposure::default()`（EV100 = 9.7）＝ `exp2(-9.7) / 1.2`。
///
/// ⚠ **必须按 f32 表达式算**（§110.1）：先按 f64 算再取整是另一个数
/// （1.001907888464288e-3 vs 1.001907978207e-3），而"亮一点点"在逐字节判据上就是红。
/// 位模式 `3A835274`（判据里钉着）。
pub fn exposure() -> f32 {
    f32::exp2(-9.7) / 1.2
}

/// 结构体 → **它自己那几字节**（`size_of` 那么长，没有补齐）。
pub fn to_bytes<T: Pod>(value: &T) -> Vec<u8> {
    bytemuck::bytes_of(value).to_vec()
}

/// 结构体 → **uniform 缓冲该有的字节**：零补齐到 16 的倍数。
///
/// wgpu 的 uniform 绑定要按 16 对齐（`globals` 只有 12 字节，不补齐就用不了）；
/// 补齐的字节是 0，shader 也读不到它们（结构体只到 `span` 为止）。
pub fn to_uniform_bytes<T: Pod>(value: &T) -> Vec<u8> {
    let mut bytes = to_bytes(value);
    bytes.resize(bytes.len().div_ceil(16) * 16, 0);
    bytes
}

impl ViewUniform {
    /// 按名字填一份 `view`；`exposure` 由 [`exposure`] 给（内容 shader 自己乘它）。
    ///
    /// ⚠ 参数次序**就是** WGSL 结构体的声明次序（末尾两格是 §135 追加的逆矩阵）：
    /// 两处各排一次而排得不同，是"同一份契约、两个数"最直白的一种写法。
    pub fn new(
        world_position: [f32; 3],
        view_from_world: [[f32; 4]; 4],
        clip_from_view: [[f32; 4]; 4],
        viewport: [f32; 4],
        view_from_clip: [[f32; 4]; 4],
        world_from_view: [[f32; 4]; 4],
    ) -> ViewUniform {
        ViewUniform {
            world_position,
            exposure: exposure(),
            view_from_world,
            clip_from_view,
            viewport,
            view_from_clip,
            world_from_view,
        }
    }

    /// 探针相机 + 视口 → `view`。矩阵按列主序原样搬（`Mat4` 就是 glam 那 16 个数）。
    ///
    /// ⚠ `view_from_world` 与 `view_from_clip` 用的都是**逐位**算出来的通用逆
    /// （§110.1.1：解析逆差 1–2 ulp，落到顶点裁剪坐标上就是几个顶点换边 ⇒ 逐字节判据红
    /// 且归因不到；天空盒那条逆还要再乘一次片元坐标，同一个 1 ulp 会摊到整幅背景上）。
    /// 两条逆都在 `camera.rs` 里算过一次，这里只搬 —— 宿主每帧只求一次逆。
    pub fn from_camera(camera: &crate::camera::Camera, viewport: [f32; 4]) -> ViewUniform {
        ViewUniform::new(
            [camera.position.x, camera.position.y, camera.position.z],
            columns(&camera.view_from_world),
            columns(&camera.clip_from_view),
            viewport,
            columns(&camera.view_from_clip),
            columns(&camera.world_from_view),
        )
    }
}

/// `Mat4` → 列主序的 16 个数（最后一列是平移）。
fn columns(matrix: &Mat4) -> [[f32; 4]; 4] {
    let axis = |v: Vec4| [v.x, v.y, v.z, v.w];
    [
        axis(matrix.x_axis),
        axis(matrix.y_axis),
        axis(matrix.z_axis),
        axis(matrix.w_axis),
    ]
}

impl LightsUniform {
    /// 环境光：`Color::linear_rgb(1,1,1) × brightness` ⇒ 三个通道都是那个强度。
    /// `w` 在 Bevy 那份里也是同一个数（`ambient_color` 是个 `vec4`）。
    pub fn ambient(brightness: f32) -> LightsUniform {
        LightsUniform {
            ambient_color: [brightness; 4],
        }
    }
}

impl ClusteredLight {
    /// 「这一格没写过」：全零 ⇒ 内容 shader 判 `lit = false`（颜色是判据，见字段注释）。
    pub fn absent() -> ClusteredLight {
        ClusteredLight::zeroed()
    }
}

// ---------------------------------------------------------------------------
// 值 → GPU：**五格全绑**的那一组
//
// 布局是**固定超集**：五格一个不少地进绑定组，哪怕这一帧用到它的 shader 只声明了其中一格。
// 三条理由，每一条都是踩过的：
//
// 1. 一条管线要覆盖它声明的**每一组**，而组里缺一格 ⇒ `create_render_pipeline` 当场拒；
//    五格一次绑齐，换场景/换材质都不用重建布局。
// 2. `depth_prepass_texture`（第 20 格）是**同一张深度图**的另一条入口：大气的片元要
//    采样它，而那张图必须在 group 0 里 —— 所以深度图不许由执行器的池子另建一张
//    （宿主自己建、自己交出去，见 `render.rs` 那段"顶掉文档声明的资源"）。
// 3. 绑定号会改像素（§65 记的那 22–33 个像素至今没归因）⇒ 号只有这一份（上面那五个常量），
//    由测试钉在组装出来的 WGSL 上。
// ---------------------------------------------------------------------------

/// `globals` 的三格：这一帧的时间。⚠ 四档内容里 `wind = wind_skin = 0` ⇒ 它与像素无关
/// （只有 `orbit-soft-wind` 那一档不是），所以这里就是零，而且**打印出来**。
pub fn globals_zero() -> GlobalsUniform {
    GlobalsUniform::default()
}

/// group 0 的绑定组布局（五格超集）。**由契约常量建**，不另写一张表。
pub fn bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let buffer = |binding: u32, ty: wgpu::BufferBindingType| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            // 每份 shader 的结构体大小不同（`globals` 只 12 字节），真实大小由缓冲决定。
            min_binding_size: None,
        },
        count: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("组 0（五格超集）"),
        entries: &[
            buffer(VIEW_BINDING.1, wgpu::BufferBindingType::Uniform),
            buffer(LIGHTS_BINDING.1, wgpu::BufferBindingType::Uniform),
            buffer(
                CLUSTERED_LIGHTS_BINDING.1,
                wgpu::BufferBindingType::Storage { read_only: true },
            ),
            buffer(GLOBALS_BINDING.1, wgpu::BufferBindingType::Uniform),
            wgpu::BindGroupLayoutEntry {
                binding: DEPTH_PREPASS_BINDING.1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ],
    })
}

/// 这一帧的 group 0：布局 + 绑定组。
///
/// ⚠ 缓冲不在这里留字段：`wgpu::BindGroup` **自己持有**它绑的那些资源（引用计数），
/// 建完之后缓冲就可以放手 —— 这是 wgpu 的契约，不是"大概不会出事"。
pub struct GroupZero {
    pub layout: wgpu::BindGroupLayout,
    pub bind_group: wgpu::BindGroup,
    /// 这一帧真的填了什么（进审计文本：出问题时先看这一行）。
    pub audit: Vec<String>,
}

/// 按这一帧的值建 group 0。
///
/// `module` 只用来**反射聚类缓冲的长度与步长**（"能放几盏灯"写在 shader 里，
/// 不在 Rust 里抄第二份）；其余三格的大小由各自的 Rust 结构体定，而结构体与 WGSL 的
/// 偏移/大小由 `cargo test` 那几条判据钉着。
pub fn frame(
    device: &wgpu::Device,
    module: &naga::Module,
    camera: &crate::camera::Camera,
    ambient: f32,
    width: u32,
    height: u32,
    depth: &wgpu::TextureView,
) -> Result<GroupZero, String> {
    let (group, binding) = CLUSTERED_LIGHTS_BINDING;
    let array = storage_array_layout(module, group, binding)
        .map_err(|err| format!("反射聚类缓冲失败：{err}"))?;
    let light = std::mem::size_of::<ClusteredLight>();
    if array.stride as usize != light {
        return Err(format!(
            "聚类数组的步长是 {}，而 `ClusteredLight` 是 {light} 字节：同一份契约的两个数",
            array.stride
        ));
    }

    let view = ViewUniform::from_camera(camera, [0.0, 0.0, width as f32, height as f32]);
    let lights = LightsUniform::ambient(ambient);
    let globals = globals_zero();
    // 「没有点光源」= 全零（`light.wgsl` 判的正是颜色）。这一档那盏灯强度是 0 ⇒
    // 颜色 0 ⇒ 内容 shader 自己判 `lit = false` —— 宿主不认识"太阳"，也不替它兜底（§64.9）。
    // ⚠ 长度与步长来自**反射**（`array`），不是写死的 64：能放几盏灯写在 shader 里。
    let cluster: Vec<u8> = to_bytes(&ClusteredLight::absent()).repeat(array.count as usize);

    let uniform = |label: &str, bytes: &[u8]| {
        device.create_buffer_init(&BufferInitDescriptor {
            label: Some(label),
            usage: wgpu::BufferUsages::UNIFORM,
            contents: bytes,
        })
    };
    let view_buffer = uniform("组 0：view", &to_uniform_bytes(&view));
    let lights_buffer = uniform("组 0：lights", &to_uniform_bytes(&lights));
    let globals_buffer = uniform("组 0：globals", &to_uniform_bytes(&globals));
    let cluster_buffer = device.create_buffer_init(&BufferInitDescriptor {
        label: Some("组 0：clustered_lights"),
        usage: wgpu::BufferUsages::STORAGE,
        contents: &cluster,
    });

    let layout = bind_group_layout(device);
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("组 0（五格超集）"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: VIEW_BINDING.1,
                resource: view_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: LIGHTS_BINDING.1,
                resource: lights_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: CLUSTERED_LIGHTS_BINDING.1,
                resource: cluster_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: GLOBALS_BINDING.1,
                resource: globals_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: DEPTH_PREPASS_BINDING.1,
                resource: wgpu::BindingResource::TextureView(depth),
            },
        ],
    });
    let viewport = view.viewport;
    Ok(GroupZero {
        layout,
        bind_group,
        audit: vec![
            format!(
                "view：world_position ({:.3}, {:.3}, {:.3})｜exposure {:.9e}（位模式 {:08X}）｜viewport ({}, {}, {}, {})",
                camera.position.x,
                camera.position.y,
                camera.position.z,
                view.exposure,
                view.exposure.to_bits(),
                viewport[0],
                viewport[1],
                viewport[2],
                viewport[3]
            ),
            format!(
                "lights：ambient_color ({}, {}, {}, {})",
                lights.ambient_color[0],
                lights.ambient_color[1],
                lights.ambient_color[2],
                lights.ambient_color[3]
            ),
            format!(
                "globals：time {}｜delta_time {}｜frame_count {}",
                globals.time, globals.delta_time, globals.frame_count
            ),
            format!(
                "clustered_lights：{} 格 × {} 字节 = {} 字节，**全零** ⇒ 内容 shader 判 lit = false（这一档那盏灯强度 0）",
                array.count,
                array.stride,
                array.size
            ),
        ],
    })
}

// ---------------------------------------------------------------------------
// 反射：从**组装后的 WGSL**里读出结构体布局与数组步长
// ---------------------------------------------------------------------------

/// 一个 WGSL 结构体在 naga 眼里的布局。成员按**声明次序**（不是按偏移排序）。
#[derive(Clone, Debug, PartialEq)]
pub struct StructLayout {
    pub name: String,
    /// 整块大小（naga 的 `span`）。
    pub size: u32,
    pub align: u32,
    /// `(成员名, 偏移, 类型文本)`。
    pub members: Vec<(String, u32, String)>,
}

/// 一个 WGSL storage 数组的布局（`clustered_lights` 就是它）。
#[derive(Clone, Debug, PartialEq)]
pub struct ArrayLayout {
    pub name: String,
    /// 元素步长（数组里相邻两个元素隔多少字节）。
    pub stride: u32,
    /// 元素个数。
    pub count: u32,
    /// 整块缓冲的字节数 = `stride × count`。
    pub size: u32,
}

/// `(group, binding)` 上那个全局变量声明的**结构体**布局，由 naga 算。
///
/// 反射不出来就当场报错（不许"按能读的读一部分"）：那一格没有全局变量、或者它不是结构体。
pub fn struct_layout(
    module: &naga::Module,
    group: u32,
    binding: u32,
) -> Result<StructLayout, String> {
    let (global, handle) = global_at(module, group, binding)?;
    let name = global.name.clone().unwrap_or_else(|| "?".to_string());
    struct_layout_of(module, handle, &name)
}

/// `(group, binding)` 上那个 storage 变量的**数组元素**（`clustered_lights.data[i]`）的布局。
///
/// 外层那一格是个 `struct { data: array<T, 64> }`，真正要填的是 `T` 的每一个字段 ——
/// 所以这里钻到数组的元素类型上再反射一次。
pub fn element_struct_layout(
    module: &naga::Module,
    group: u32,
    binding: u32,
) -> Result<StructLayout, String> {
    let (global, handle) = global_at(module, group, binding)?;
    let outer = global.name.clone().unwrap_or_else(|| "?".to_string());
    let naga::TypeInner::Struct { members, .. } = &module.types[handle].inner else {
        return Err(format!(
            "@group({group}) @binding({binding}) 的 {outer} 不是结构体：{:?}",
            module.types[handle].inner
        ));
    };
    let member = members
        .first()
        .ok_or_else(|| format!("{outer} 是个空结构体：里面没有数组"))?;
    let naga::TypeInner::Array { base, .. } = &module.types[member.ty].inner else {
        return Err(format!(
            "{outer}.{} 不是数组：{:?}",
            member.name.clone().unwrap_or_else(|| "?".to_string()),
            module.types[member.ty].inner
        ));
    };
    let name = module.types[*base]
        .name
        .clone()
        .unwrap_or_else(|| format!("{} 的元素", outer));
    struct_layout_of(module, *base, &name)
}

/// 一个类型句柄（必须是结构体）的布局。`name` 只用于报错与打印。
fn struct_layout_of(
    module: &naga::Module,
    handle: naga::Handle<naga::Type>,
    name: &str,
) -> Result<StructLayout, String> {
    let naga::TypeInner::Struct { members, span } = &module.types[handle].inner else {
        return Err(format!(
            "{name} 不是结构体：{:?}",
            module.types[handle].inner
        ));
    };
    let layout = layouter(module)?[handle];
    Ok(StructLayout {
        name: name.to_string(),
        size: *span,
        // `Alignment` 这一版没有 `get()`：`round_up(1)` 就是它自己（2 的幂）。
        align: layout.alignment.round_up(1),
        members: members
            .iter()
            .map(|member| {
                (
                    member.name.clone().unwrap_or_else(|| "?".to_string()),
                    member.offset,
                    type_name(module, member.ty),
                )
            })
            .collect(),
    })
}

/// `(group, binding)` 上那个 storage 变量的**数组**布局（`clustered_lights.data`）。
///
/// 桩里的形状是 `struct ClusteredLightsStub { data: array<ClusteredLightStub, 64> }` ——
/// 外面包了一层结构体，所以这里剥一层：**步长与长度都从那一格读出来**，
/// 于是「一盏灯多少字节」「能放几盏」不用在 Rust 里抄第二份。
pub fn storage_array_layout(
    module: &naga::Module,
    group: u32,
    binding: u32,
) -> Result<ArrayLayout, String> {
    let (global, handle) = global_at(module, group, binding)?;
    let name = global.name.clone().unwrap_or_else(|| "?".to_string());
    let naga::TypeInner::Struct { members, .. } = &module.types[handle].inner else {
        return Err(format!(
            "@group({group}) @binding({binding}) 的 {name} 不是结构体：{:?}",
            module.types[handle].inner
        ));
    };
    let member = members
        .first()
        .ok_or_else(|| format!("{name} 是个空结构体：里面没有数组"))?;
    let element = member.name.clone().unwrap_or_else(|| "?".to_string());
    let naga::TypeInner::Array {
        stride,
        size: naga::ArraySize::Constant(count),
        ..
    } = &module.types[member.ty].inner
    else {
        return Err(format!(
            "{name}.{element} 不是定长数组：{:?}",
            module.types[member.ty].inner
        ));
    };
    let count = count.get();
    Ok(ArrayLayout {
        name: format!("{name}.{element}"),
        stride: *stride,
        count,
        size: *stride * count,
    })
}

/// `(group, binding)` 上的全局变量与它的类型句柄。
fn global_at(
    module: &naga::Module,
    group: u32,
    binding: u32,
) -> Result<(&naga::GlobalVariable, naga::Handle<naga::Type>), String> {
    for (_, global) in module.global_variables.iter() {
        if global.binding == Some(naga::ResourceBinding { group, binding }) {
            return Ok((global, global.ty));
        }
    }
    Err(format!(
        "组装后的 WGSL 里没有 @group({group}) @binding({binding}) 这个全局变量"
    ))
}

fn layouter(module: &naga::Module) -> Result<naga::proc::Layouter, String> {
    let mut layouter = naga::proc::Layouter::default();
    layouter
        .update(naga::proc::GlobalCtx {
            types: &module.types,
            constants: &module.constants,
            overrides: &module.overrides,
            global_expressions: &module.global_expressions,
        })
        .map_err(|err| format!("算不出类型布局：{err}"))?;
    Ok(layouter)
}

/// 成员类型的规范文本。只印本契约用得上的那几种，别的一律 `?`（出现了就该看得见）。
fn type_name(module: &naga::Module, handle: naga::Handle<naga::Type>) -> String {
    use naga::{ArraySize, TypeInner};
    match &module.types[handle].inner {
        TypeInner::Scalar(scalar) => scalar_name(scalar),
        TypeInner::Vector { size, scalar } => {
            format!("vec{}<{}>", dimension(*size), scalar_name(scalar))
        }
        TypeInner::Matrix {
            columns,
            rows,
            scalar,
        } => format!(
            "mat{}x{}<{}>",
            dimension(*columns),
            dimension(*rows),
            scalar_name(scalar)
        ),
        TypeInner::Array { base, size, .. } => format!(
            "array<{}, {}>",
            type_name(module, *base),
            match size {
                ArraySize::Constant(count) => count.get().to_string(),
                ArraySize::Pending(_) => "?".to_string(),
                ArraySize::Dynamic => "runtime".to_string(),
            }
        ),
        TypeInner::Struct { .. } => "struct".to_string(),
        other => format!("{other:?}"),
    }
}

fn scalar_name(scalar: &naga::Scalar) -> String {
    use naga::ScalarKind;
    match (scalar.kind, scalar.width) {
        (ScalarKind::Float, 4) => "f32".to_string(),
        (ScalarKind::Uint, 4) => "u32".to_string(),
        (ScalarKind::Sint, 4) => "i32".to_string(),
        other => format!("{other:?}"),
    }
}

fn dimension(size: naga::VectorSize) -> u32 {
    match size {
        naga::VectorSize::Bi => 2,
        naga::VectorSize::Tri => 3,
        naga::VectorSize::Quad => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, offset_of, size_of};

    /// 组一份**把 group 0 那五格都引一遍**的入口文本，再按本宿主的桩表组装、naga 校验。
    ///
    /// 为什么不直接读某一份内容 shader：这一篇判的是**契约**（桩表声明的形状），
    /// 而契约与"哪一份内容 shader 引了它"无关。五格都引一遍 ⇒ 一份文本覆盖全部判据。
    const PROBE: &str = "#import bevy_pbr::forward_io::VertexOutput\n\
                         #import bevy_pbr::mesh_view_bindings::view\n\
                         #import bevy_pbr::mesh_view_bindings::lights\n\
                         #import bevy_pbr::mesh_view_bindings::clustered_lights\n\
                         #import bevy_pbr::mesh_view_bindings::globals\n\
                         #import bevy_pbr::mesh_view_bindings::depth_prepass_texture\n\
                         @fragment\n\
                         fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {\n\
                         \x20   let depth = textureLoad(depth_prepass_texture, vec2<i32>(in.position.xy), 0);\n\
                         \x20   let light = clustered_lights.data[0];\n\
                         \x20   return vec4<f32>(\n\
                         \x20       view.exposure + lights.ambient_color.x + globals.time + globals.delta_time\n\
                         \x20           + f32(light.flags) + f32(light.decal_index) + light.range + depth\n\
                         \x20           + view.view_from_clip[3][2] + view.world_from_view[3][3],\n\
                         \x20       view.world_position.x + view.view_from_world[0][0] + view.clip_from_view[0][0],\n\
                         \x20       view.viewport.x + view.viewport.w + in.world_position.x + in.world_normal.x,\n\
                         \x20       in.uv.x + in.uv.y,\n\
                         \x20   );\n\
                         }\n";

    fn probe_module() -> naga::Module {
        let modules = crate::shader::modules();
        let assembled = crate::shader::assemble(PROBE, &modules, crate::stubs::stubs);
        crate::shader::validate("group0 探针", &assembled).expect("组装 + 校验")
    }

    /// 一个 Rust 结构体的**逐字段对照表**：`(WGSL 成员名, offset_of!, WGSL 类型文本)`。
    ///
    /// 偏移由 `offset_of!` 算（编译器给的），类型文本由 WGSL 那侧说 —— 两边都不是手抄的数字。
    /// 名字必须与 `stubs.rs` 里声明的成员**逐字相同**：对不上就是"两边说的不是同一个字段"。
    trait FieldLayout {
        const FIELDS: &'static [(&'static str, usize, &'static str)];
    }

    impl FieldLayout for ViewUniform {
        const FIELDS: &'static [(&'static str, usize, &'static str)] = &[
            (
                "world_position",
                offset_of!(ViewUniform, world_position),
                "vec3<f32>",
            ),
            ("exposure", offset_of!(ViewUniform, exposure), "f32"),
            (
                "view_from_world",
                offset_of!(ViewUniform, view_from_world),
                "mat4x4<f32>",
            ),
            (
                "clip_from_view",
                offset_of!(ViewUniform, clip_from_view),
                "mat4x4<f32>",
            ),
            ("viewport", offset_of!(ViewUniform, viewport), "vec4<f32>"),
            // ⚠ 末尾这两格是 §135 追加的：它们的偏移（160 / 224）与上面五格的关系是
            // **跟着走的**，所以这条判据同时钉住了"新字段没有把老的挤走"。
            (
                "view_from_clip",
                offset_of!(ViewUniform, view_from_clip),
                "mat4x4<f32>",
            ),
            (
                "world_from_view",
                offset_of!(ViewUniform, world_from_view),
                "mat4x4<f32>",
            ),
        ];
    }

    impl FieldLayout for LightsUniform {
        const FIELDS: &'static [(&'static str, usize, &'static str)] =
            &[("ambient_color", offset_of!(LightsUniform, ambient_color), "vec4<f32>")];
    }

    impl FieldLayout for GlobalsUniform {
        const FIELDS: &'static [(&'static str, usize, &'static str)] = &[
            ("time", offset_of!(GlobalsUniform, time), "f32"),
            ("delta_time", offset_of!(GlobalsUniform, delta_time), "f32"),
            ("frame_count", offset_of!(GlobalsUniform, frame_count), "u32"),
        ];
    }

    impl FieldLayout for ClusteredLight {
        const FIELDS: &'static [(&'static str, usize, &'static str)] = &[
            (
                "light_custom_data",
                offset_of!(ClusteredLight, light_custom_data),
                "vec4<f32>",
            ),
            (
                "color_inverse_square_range",
                offset_of!(ClusteredLight, color_inverse_square_range),
                "vec4<f32>",
            ),
            (
                "position_radius",
                offset_of!(ClusteredLight, position_radius),
                "vec4<f32>",
            ),
            ("flags", offset_of!(ClusteredLight, flags), "u32"),
            (
                "shadow_depth_bias",
                offset_of!(ClusteredLight, shadow_depth_bias),
                "f32",
            ),
            (
                "shadow_normal_bias",
                offset_of!(ClusteredLight, shadow_normal_bias),
                "f32",
            ),
            (
                "spot_light_tan_angle",
                offset_of!(ClusteredLight, spot_light_tan_angle),
                "f32",
            ),
            (
                "soft_shadow_size",
                offset_of!(ClusteredLight, soft_shadow_size),
                "f32",
            ),
            (
                "shadow_map_near_z",
                offset_of!(ClusteredLight, shadow_map_near_z),
                "f32",
            ),
            ("decal_index", offset_of!(ClusteredLight, decal_index), "u32"),
            ("range", offset_of!(ClusteredLight, range), "f32"),
        ];
    }

    /// 把「naga 算的布局」与「Rust 结构体」逐格比一遍，并把对照表打出来。
    ///
    /// 比四样：**成员个数**、**逐个成员的偏移**、**逐个成员的类型**、**整块大小与对齐**。
    /// 少比一样，就等于给"Rust 侧再抄一份布局"留了一条缝。
    fn assert_layout<T: FieldLayout>(layout: &StructLayout) -> String {
        let mut lines = vec![format!(
            "{}｜naga span={} align={}｜Rust size={} align={}",
            layout.name,
            layout.size,
            layout.align,
            size_of::<T>(),
            align_of::<T>()
        )];
        assert_eq!(
            size_of::<T>(),
            layout.size as usize,
            "{} 的整块大小对不上（naga {} / Rust {}）",
            layout.name,
            layout.size,
            size_of::<T>()
        );
        assert_eq!(
            align_of::<T>(),
            layout.align as usize,
            "{} 的对齐对不上（naga {} / Rust {}）",
            layout.name,
            layout.align,
            align_of::<T>()
        );
        assert_eq!(
            T::FIELDS.len(),
            layout.members.len(),
            "{} 的成员个数对不上：naga 声明了 {:?}",
            layout.name,
            layout
                .members
                .iter()
                .map(|member| member.0.as_str())
                .collect::<Vec<_>>()
        );
        for (index, (name, offset, kind)) in T::FIELDS.iter().enumerate() {
            let member = &layout.members[index];
            assert_eq!(
                member.0, *name,
                "{} 第 {index} 个成员的名字对不上：naga 说 '{}'，Rust 说 '{name}'",
                layout.name, member.0
            );
            assert_eq!(
                *offset as u32, member.1,
                "{} 的成员 '{name}' 偏移对不上：naga {} / Rust {offset}",
                layout.name, member.1
            );
            assert_eq!(
                member.2, *kind,
                "{} 的成员 '{name}' 类型对不上：naga 说 {}，Rust 说 {kind}",
                layout.name, member.2
            );
            lines.push(format!(
                "  {name:<26} offset naga={:<4} rust={:<4} type={}",
                member.1, offset, member.2
            ));
        }
        lines.join("\n")
    }

    /// 四条判据：`view` / `lights` / `globals` / `clustered_lights` 的结构体布局，
    /// 逐字段与**组装后的 WGSL** 对齐（§111 第 3 件的判据）。
    #[test]
    fn the_host_structs_match_the_reflected_wgsl_layout() {
        let module = probe_module();
        let mut report = Vec::new();
        for (what, (group, binding)) in [
            ("view", VIEW_BINDING),
            ("lights", LIGHTS_BINDING),
            ("globals", GLOBALS_BINDING),
        ] {
            let layout = struct_layout(&module, group, binding)
                .unwrap_or_else(|err| panic!("反射 {what} 失败：{err}"));
            let text = match what {
                "view" => assert_layout::<ViewUniform>(&layout),
                "lights" => assert_layout::<LightsUniform>(&layout),
                _ => assert_layout::<GlobalsUniform>(&layout),
            };
            report.push(text);
        }
        println!("{}", report.join("\n"));
    }

    /// 绑定号：Rust 里那一份必须**就是** shader 里那一份（§104 第 1 条）。
    ///
    /// 五格全查，包括不在这里放缓冲的 `depth_prepass_texture` —— 它同样会改像素。
    #[test]
    fn the_group_zero_bindings_are_bevys_numbers() {
        let module = probe_module();
        let rows = crate::shader::bindings(&module);
        let mut table: Vec<((u32, u32), String)> = Vec::new();
        for (group, binding, space, var) in &rows {
            table.push(((*group, *binding), format!("{space} {var}")));
        }
        println!("group 0 契约（反射）：{table:?}");
        for (what, expected) in [
            ("view", VIEW_BINDING),
            ("lights", LIGHTS_BINDING),
            ("clustered_lights", CLUSTERED_LIGHTS_BINDING),
            ("globals", GLOBALS_BINDING),
            ("depth_prepass_texture", DEPTH_PREPASS_BINDING),
        ] {
            let found = table
                .iter()
                .find(|(binding, declaration)| {
                    *binding == expected && declaration.ends_with(what)
                })
                .unwrap_or_else(|| {
                    panic!("组装后的 WGSL 里没有 {what} 在 @group({}) @binding({})", expected.0, expected.1)
                });
            println!("{what:<22} @group({}) @binding({}) {}", found.0.0, found.0.1, found.1);
        }
        // 多一格都不许有：这一份探针只引了那五格。
        assert_eq!(rows.len(), 5, "探针引了五格，反射出来却是：{table:?}");
        // `shader::bindings` 按 (组号, 格号) 排序 ⇒ 次序就是 Bevy 那份契约的次序。
        let order: Vec<(u32, u32)> = table.iter().map(|row| row.0).collect();
        assert_eq!(
            order,
            vec![
                VIEW_BINDING,
                LIGHTS_BINDING,
                CLUSTERED_LIGHTS_BINDING,
                GLOBALS_BINDING,
                DEPTH_PREPASS_BINDING,
            ],
            "反射出来的 (group, binding) 次序与 §108.3 那张表不一致"
        );
        assert_eq!(
            table[2].1, "storage clustered_lights",
            "聚类缓冲必须是 storage（不是 uniform）"
        );
        assert_eq!(
            table[4].1, "handle depth_prepass_texture",
            "深度预通道那张图必须是 texture_depth_2d（handle）"
        );
    }

    /// 聚类缓冲：**步长从 shader 里读**，且它就等于 `size_of::<ClusteredLight>()`；
    /// 长度与整块字节数同样从 shader 里读（`64 × 80 = 5120`）。
    #[test]
    fn the_storage_array_stride_is_the_light_struct() {
        let module = probe_module();
        let (group, binding) = CLUSTERED_LIGHTS_BINDING;
        let array = storage_array_layout(&module, group, binding).expect("反射聚类数组");
        println!(
            "{}｜步长 {}｜元素 {} 个｜整块 {} 字节｜Rust size_of::<ClusteredLight>() = {}",
            array.name,
            array.stride,
            array.count,
            array.size,
            size_of::<ClusteredLight>()
        );
        assert_eq!(
            array.stride as usize,
            size_of::<ClusteredLight>(),
            "数组步长与 ClusteredLight 的大小必须是同一个数"
        );
        assert_eq!(array.size, array.stride * array.count);
        let light = struct_layout(&module, group, binding).expect("反射外层结构体");
        assert_eq!(light.name, "clustered_lights");
        println!(
            "外层 {}｜span={}｜成员 {:?}",
            light.name,
            light.size,
            light
                .members
                .iter()
                .map(|member| (member.0.as_str(), member.1, member.2.as_str()))
                .collect::<Vec<_>>()
        );
        // 步长相等还不够：**每一个字段**也要逐格对上（`ClusteredLight` 那 11 个）。
        let element = element_struct_layout(&module, group, binding).expect("反射数组元素");
        assert_eq!(
            element.size, array.stride,
            "元素结构体的大小与数组步长必须是同一个数"
        );
        println!("{}", assert_layout::<ClusteredLight>(&element));
    }

    /// `view` 的**值**也要对：偏移对不对，最有说服力的判据是"按值读回来是不是那几个数"。
    /// 曝光那一格的位模式是 §110.1 钉死的 `3A835274`。
    ///
    /// ⚠ 末尾两格是 §135 加的，它们在这里的判据是"**前五格的偏移一个都没动**"
    /// （144…156 还是 viewport）—— 这条与 `offset_of!` 那张表互为旁证：
    /// 一个是编译器说的，一个是按字节读出来的。
    #[test]
    fn the_view_uniform_carries_the_probe_camera_and_the_exposure_bits() {
        assert_eq!(
            exposure().to_bits(),
            0x3A83_5274,
            "view.exposure 的位模式（§110.1：按 f32 表达式算，不是 f64 取整）"
        );
        let camera = crate::camera::probe_camera(None, 960.0 / 640.0);
        let view = ViewUniform::from_camera(&camera, [0.0, 0.0, 960.0, 640.0]);
        let bytes = to_bytes(&view);
        assert_eq!(bytes.len(), 288, "view 是 160 + 两条逆矩阵的 128 = 288 字节");
        let word = |offset: usize| {
            u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("四个字节"))
        };
        assert_eq!(word(0), camera.position.x.to_bits(), "world_position.x");
        assert_eq!(word(4), camera.position.y.to_bits(), "world_position.y");
        assert_eq!(word(8), camera.position.z.to_bits(), "world_position.z");
        assert_eq!(word(12), 0x3A83_5274, "exposure");
        assert_eq!(
            word(16),
            camera.view_from_world.x_axis.x.to_bits(),
            "view_from_world —— **世界 → 视图**那条逆，不是位姿矩阵"
        );
        assert_eq!(word(76), camera.view_from_world.w_axis.w.to_bits());
        assert_eq!(word(80), camera.clip_from_view.x_axis.x.to_bits());
        assert_eq!(word(140), camera.clip_from_view.w_axis.w.to_bits());
        // ⚠ 前五格到这里就结束了：144 还是 viewport（§135 的追加**不许**动它）。
        assert_eq!(word(144), 0.0f32.to_bits(), "viewport.x = 0");
        assert_eq!(word(148), 0.0f32.to_bits(), "viewport.y = 0");
        assert_eq!(word(152), 960.0f32.to_bits(), "viewport.z = 宽");
        assert_eq!(word(156), 640.0f32.to_bits(), "viewport.w = 高");
        assert_eq!(
            word(160),
            camera.view_from_clip.x_axis.x.to_bits(),
            "view_from_clip 紧跟在 viewport 之后"
        );
        assert_eq!(
            word(284),
            camera.world_from_view.w_axis.w.to_bits(),
            "world_from_view 收尾（224 + 60）"
        );
        // 位姿矩阵与它的逆在**同一个结构体里各占一格**，而不是同一份数据两处名字：
        // 逐位比一次，确认这两格不是同一个数（对相机来说它们是互逆的，不是相等）。
        assert_ne!(
            camera.view_from_world.x_axis.y.to_bits(),
            camera.world_from_view.x_axis.y.to_bits(),
            "位姿与它的逆恰好在这一格上不同（0x00000000 vs 0x80000000）—— \
             同一个数写两处的那种接错法会在这里露出来"
        );
    }

    /// uniform 缓冲要 16 对齐（`globals` 是 12 字节）；补齐的字节全是 0，前缀一个字节都不许动。
    #[test]
    fn uniform_bytes_are_padded_to_sixteen() {
        let globals = GlobalsUniform {
            time: 0.0,
            delta_time: 0.0,
            frame_count: 0,
        };
        let exact = to_bytes(&globals);
        let padded = to_uniform_bytes(&globals);
        assert_eq!(exact.len(), 12, "globals 就是 12 字节（桩里只有三个标量）");
        assert_eq!(padded.len(), 16, "补齐到 16 的倍数");
        assert_eq!(&padded[..exact.len()], &exact[..], "前缀必须是同样的字节");
        assert!(padded[exact.len()..].iter().all(|byte| *byte == 0));
        assert_eq!(to_uniform_bytes(&ViewUniform::from_camera(
            &crate::camera::probe_camera(None, 1.5),
            [0.0, 0.0, 1.0, 1.0],
        )).len(), 288);
        assert_eq!(to_uniform_bytes(&LightsUniform::ambient(80.0)).len(), 16);
    }

    /// `ambient_color` 就是 `vec4(80, 80, 80, 80)`（这个场景 `environment.ambient = 80`）。
    #[test]
    fn the_ambient_light_is_the_scene_number() {
        let bytes = to_bytes(&LightsUniform::ambient(80.0));
        for offset in [0, 4, 8, 12] {
            let word = u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("四个字节"));
            assert_eq!(word, 80.0f32.to_bits(), "ambient_color 第 {} 格", offset / 4);
        }
        assert_eq!(
            to_bytes(&ClusteredLight::absent()).len(),
            80,
            "没写过的那一格是全零 80 字节（内容 shader 靠颜色判它在不在）"
        );
        assert!(to_bytes(&ClusteredLight::absent())
            .iter()
            .all(|byte| *byte == 0));
    }
}
