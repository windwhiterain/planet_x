//! **裸 wgpu 宿主（`px_render`）那张桩表** —— 它就是那个宿主的 group 0 契约。
//!
//! 与 Bevy 宿主的关系（§103.1）：
//!
//! - Bevy 宿主那条路，`#import bevy_pbr::*` 在**运行期**由 naga_oil 拿 Bevy 自己的
//!   `bevy_pbr` 兑现；`assemble::bevy_stub` 只服务"离线把文本拼出来"（离线门与反射）。
//! - 这个宿主**没有 naga_oil**，所以这张表**就是运行期真正用的那一份**：
//!   组装出来的文本直接喂给 `create_shader_module`。
//!
//! ⚠ **为什么这几段文本住在共享的叶子 crate、而不在宿主自己的 crate 里**（S8-a 搬过来）：
//! 这里的规则与 `assemble::HOST_VIEW_STUB` 当初搬过来时是**同一条** —— 谁依赖不到宿主 crate，
//! 谁就得自己抄一份，而抄第二份就是 §66.1 那颗「同一条契约、两处文本」的雷。今天有三个用户：
//!
//! | 谁 | 为什么够不到宿主 |
//! |---|---|
//! | 烘图侧（`px_scene::frame`，反射帧材质） | `px_render` 拖着整棵 wgpu 树（§100：烘图侧要快） |
//! | `px_probe`（把云 shader 编到自己的设备上） | 同上；而且宿主**只有 bin target**，根本没有可依赖的 lib |
//! | 宿主自己（`px_render::stubs`） | 够得到 —— 它只是 `pub use` 这一份，并留着**钉住**它的判据 |
//!
//! ⚠ 桩表的**内容**是宿主的判据来源：谁多认一个已经退休的符号（比如平行光的
//! `fetch_directional_shadow`），离线门就该报「找不到这个符号」，而不是运行期才发现画面不对。
//! 宿主那一侧的钉法（哪三格与 Bevy 不同、各自为什么）住在 `px_render::stubs` 的测试里 ——
//! **文本搬了家，判据没跟着搬**：它钉的是"宿主认下来的那张表"，而那仍然是宿主的事。
//!
//! ## ⚠⚠ 为什么这些符号**还叫 `bevy_pbr::…`** —— 那是**出处指针**，不是"还没改的名字"（S8-b 裁决 a）
//!
//! 那个 crate 已经不在了（S8-a 删除，`Cargo.lock` 里 bevy 一族 0 条）⇒ `bevy_pbr::view`
//! 今天**指不到任何东西**。但"指不到那个 crate"不等于"这个名字是假的"：它说的是这段文本
//! **从哪儿抄来的、照哪儿排的**，而这是本仓的核心纪律之一（每一个数都指得出出处）。
//!
//! | 这一格 | 与 Bevy 的关系 |
//! |---|---|
//! | [`POINT_SHADOW_STUB`]（`shadows::fetch_point_shadow`） | **真实现**：逐句抄 `bevy_pbr-0.19.1/src/render/shadows.wgsl` 那条 Gaussian 路（§109） |
//! | [`DEPTH_NDC_TO_VIEW_Z`]（`view_transformations::`） | **真实现**：Bevy 那个 `-perspective_camera_near()/ndc_depth` 的逐字一份 |
//! | `view` / `lights` / `globals` / `clustered_lights` / `depth_prepass_texture` / `VertexOutput` / `POINT_LIGHT_FLAGS_SHADOWS_ENABLED_BIT` | 纯声明 —— 但**组号与格位是照 Bevy 排的**（§104 第 1 条：绑定号会改像素），所以"照哪儿排的"就是这几格唯一的说明 |
//!
//! （表里另外那三条 `clustered_forward::*` 今天**没有任何 import 子句点到**（§64 之后不走聚类），
//! 它们留在表里是"**认**得出来"而不是"有人用" —— 真要动，动的是它们**自己那一格**，
//! 与谁点了它的名字无关。）
//!
//! ⇒ 把它们改叫 `planet_x::host::…` 是**把一条出处指针换成一条所有权声明**，而所有权是假的
//! （这段文本不是我们写的，是抄的）—— 那不是"用真名换假名"，是**用假名换掉唯一指向出处的那一个**。
//!
//! ## ⚠⚠ 连注释都不许随手改：这张表的文本**是进产物的**
//!
//! `px_graph::shader_key = blake3("px_shader/v2" ‖ SHADER_VERSION ‖ **include 闭包指纹** ‖ 入口文本)`，
//! 而 `Closure::fingerprint` 哈希的是 **`#import` 子句字符串本身** ⇒ 谁引到了上面那几个符号，
//! **改一个名字就换掉全工程的产物键 —— 哪怕组装出来的字一个字节都没动**（S8-b 实测，四份入口）：
//!
//! | 入口 | 组装文本（改名前后） | 闭包指纹（改名前后） |
//! |---|---|---|
//! | `atmosphere.wgsl` | 8311 B → 8323 B（+12） | `33881b68faef8589` → `a9e9142bd625f349` |
//! | `clouds.wgsl` | 52769 B → 52781 B（+12） | `76782061a1bdb006` → `2a1968cc5dfc5630` |
//! | `ring.wgsl`（负对照：只引 `VertexOutput`，那条在两张表里逐字相同） | 1096 B → **1096 B，逐字节相同** | `23d0283f680f89d3` → `ab0d94f0bf8ec473` |
//! | `surface.wgsl` | 25915 B → 25945 B（+30） | `abedb20f868bf99c` → `60c72a59321e8fbc` |
//!
//! ⇒ 两个读数**必须分开看**：组装文本可以一个字节都不动（`#import` 那几行整行不进产物），
//! 而闭包指纹**必然**变 ⇒ 产物键变 ⇒ `art/anchor/frozen/*.pxart` 里钉着的成员键变 ⇒
//! 逃生门那六份**文件字节**对不上 —— 而它们是**判据**（"能被删掉的判据不是判据"）：
//! 重新冻结 = **移动靶子**，那是用户/评审裁的事，不是实现方顺手做的。
//!
//! ⚠ 这条绊线的症状是**最坏的那一种**：改了名之后 J1/J2/J3 的**图**照样全绿
//! （§155 实测：完整改名之后 J1 `63184151909371A5`、J2 `A94F9F2D1437C06C` **逐字节不变**，
//! 因为像素与符号名无关）—— 于是"全绿"里没有一个字提到键已经全换了。
//!
//! ⚠ **同理**：`art/shaders/*.wgsl` 与 `art/frame/*.wgsl` 里**连一句注释都不能顺手改**
//! （注释进模块源码 ⇒ 进闭包 ⇒ 换键，与改符号名是同一条路）。

use crate::assemble::{HOST_VIEW_STUB, bevy_stub};

/// 点光 cube 影子：**真实现**（§109）。
///
/// 逐句抄 `bevy_pbr-0.19.1/src/render/shadows.wgsl:19-69`（`fetch_point_shadow`）与
/// `shadow_sampling.wgsl` 那条 **Gaussian** 路（`ShadowFilteringMethod` 的缺省档）：
/// `sample_shadow_cubemap`（`:517-539`）→ `sample_shadow_cubemap_gaussian`（`:423-460`）
/// → `sample_shadow_cubemap_at_offset`（`:382-396`）→ `sample_shadow_cubemap_hardware`
/// （`:324-341`）。以及 `bevy_render-0.19.1/src/maths.wgsl:80-87` 的 `orthonormalize`。
///
/// ⚠ 三处**不许化简**：
/// 1. `depth` 走"最大绝对轴"那条推导（`zw = -major × light_custom_data.xy +
///    light_custom_data.zw`）—— 那个 `light_custom_data` 是**宿主**按
///    `perspective_inverse_reverse_rh(π/2, 1, near)` 的 z/w 两轴算出来的
///    （`group0::light_of`），不是这里随手推的。
/// 2. 采样坐标要 `flip_z`：cube 是**左手 y-up**，Bevy 的世界是右手（`shadows.wgsl:17`、
///    `:52-68` 那两处注释）。
/// 3. Gaussian 那 8 个点是 **D3D 的 8×MSAA 位置**配 8 个高斯系数（`shadow_sampling.wgsl:70-102`），
///    基向量是 `orthonormalize(normalize(light_local)) × 0.003 × distance_to_light`
///    —— 三个数一个都不许"看起来差不多"。
///
/// ⚠ 它**自带两格的声明**（binding 2 的 cube array 与 binding 3 的比较采样器）：
/// 内容 shader 只 import 这个符号，而 Bevy 那边这两格是 `mesh_view_bindings` 那份
/// import 顺带带进来的。本宿主没有 naga_oil，所以"顺带"这件事必须写出来 ——
/// 而绑定号仍然只有一处（`px_render::group0::POINT_SHADOW_TEXTURES_BINDING` /
/// `px_render::group0::POINT_SHADOW_SAMPLER_BINDING`），由宿主 `group0` 那条反射判据钉住。
///
/// ⚠ 用到的两个符号（`clustered_lights` 与 `light_id` 的下标语义）来自**别的 import**：
/// `surface.wgsl` 引了 `clustered_lights`，所以这里直接用；谁哪天写一支只引
/// `fetch_point_shadow` 的 shader，组装会当场报"找不到 `clustered_lights`"——
/// 那正是我们要的失败方式（同 [`DEPTH_NDC_TO_VIEW_Z`] 那条判据）。
pub const POINT_SHADOW_STUB: &str = "\
@group(0) @binding(2) var point_shadow_textures: texture_depth_2d_array;\n\
@group(0) @binding(3) var point_shadow_textures_comparison_sampler: sampler;\n\
@group(0) @binding(4) var<storage, read> px_shadow_pages: array<u32>;\n\
// ⚠ 每一盏灯那一段**从第几个字开始**（前缀和）：段长跟着 `pages_per_side` 变，\n\
// 所以采样侧只做一次查表，不在 shader 里假设「所有灯段长相同」。\n\
@group(0) @binding(5) var<storage, read> px_shadow_light_offsets: array<u32>;\n\
\n\
// `bevy_render::maths::copysign`（`maths.wgsl:66-68`）：把 b 的符号位抄到 a 上。\n\
//\n\
// ⚠ 它**不是内建** —— 是 Bevy 自己定义的一个函数（正因为 naga 那条链上 `copysign`\n\
// 不是人人都有；本宿主的 naga 29.0.4 的 WGSL 前端里也没有它，`parse/conv.rs` 的\n\
// `map_standard_fun` 那张表里查不到）。Bevy 那句注释写着为什么非它不可：\n\
// `copysign allows proper handling of negative zero to match the rust implementation of\n\
// orthonormalize` —— `-0.0` 上它给 -1.0，而 `select(1.0, -1.0, z < 0.0)` 给 1.0，\n\
// 那是**两个数**，而这两个数会让基向量翻个方向。照抄，一个字都不改。\n\
fn copysign(a: f32, b: f32) -> f32 {\n\
\x20   return bitcast<f32>((bitcast<u32>(a) & 0x7FFFFFFF) | (bitcast<u32>(b) & 0x80000000));\n\
}\n\
\n\
// `bevy_render::maths::orthonormalize`（`maths.wgsl:75-87`）：把一个方向铺成一组正交基。\n\
fn orthonormalize(z_basis: vec3<f32>) -> mat3x3<f32> {\n\
\x20   let sign = copysign(1.0, z_basis.z);\n\
\x20   let a = -1.0 / (sign + z_basis.z);\n\
\x20   let b = z_basis.x * z_basis.y * a;\n\
\x20   let x_basis = vec3<f32>(1.0 + sign * z_basis.x * z_basis.x * a, sign * b, -sign * z_basis.x);\n\
\x20   let y_basis = vec3<f32>(b, sign + z_basis.y * z_basis.y * a, -z_basis.y);\n\
\x20   return mat3x3<f32>(x_basis, y_basis, z_basis);\n\
}\n\
\n\
// D3D 那 8 个 MSAA 位置与对应的高斯系数（`shadow_sampling.wgsl:79-102`）。\n\
const PX_D3D_SAMPLE_POINT_POSITIONS: array<vec2<f32>, 8> = array<vec2<f32>, 8>(\n\
\x20   vec2<f32>( 0.125, -0.375),\n\
\x20   vec2<f32>(-0.125,  0.375),\n\
\x20   vec2<f32>( 0.625,  0.125),\n\
\x20   vec2<f32>(-0.375, -0.625),\n\
\x20   vec2<f32>(-0.625,  0.625),\n\
\x20   vec2<f32>(-0.875, -0.125),\n\
\x20   vec2<f32>( 0.375,  0.875),\n\
\x20   vec2<f32>( 0.875, -0.875),\n\
);\n\
const PX_D3D_SAMPLE_POINT_COEFFS: array<f32, 8> = array<f32, 8>(\n\
\x20   0.157112, 0.157112, 0.138651, 0.130251, 0.114946, 0.114946, 0.107982, 0.079001,\n\
);\n\
\n\
// ---- 虚拟影图（§本轮）：页表 + 手动比较 --------------------------------------\n\
//\n\
// 素材是**每盏投影灯一张稀疏 atlas**（每面一层，层号 = 灯 × 6 + 面），而页表说\n\
// \"虚拟页 → 物理槽位\"。排法见 `px-scene/src/vshadow.rs` 的模块头：\n\
//   [0] virtual_size（低 16 位）| pages_per_side（高 16 位）\n\
//   [1] words_per_row\n\
//   [2] 每一面那一段的字数\n\
//   [3..] 面 0 的行段（每行：基址 1 字 + 掩码 words_per_row 字），接着面 1 ……\n\
const PX_PAGE_SIZE: u32 = 128u;\n\
const PX_PAGE_BITS: u32 = 7u;  // log2(PX_PAGE_SIZE)\n\
const PX_CUBE_FACES: u32 = 6u;\n\
// 一盏灯的页表段**最多**多少字（`px-scene/src/vshadow.rs::TABLE_HEAD_WORDS` +\n\
// `CUBE_FACES × ROWS_PER_FACE × (1 + WORDS_PER_ROW)`）—— 数组下标的上界；\n\
// 而这一盏灯实际用了多少，由头里的 `pages_per_side` 说。\n\

\n\
// Bevy 的 `cube_face_index`（`shadows.wgsl`）那六条：`+X −X +Y −Y +Z −Z`。\n\
//\n\
// ⚠ 它与 `px-scene/src/vshadow.rs::face_uv` 是**同一条契约的两处转写**（那边烘页表、\n\
//    这边采样）：写得不同的话症状是\"影贴到别的面上\"或\"影歪一点\"，而两者都像内容问题。\n\
fn px_shadow_face_uv(light_local: vec3<f32>) -> vec3<f32> {\n\
\x20   let abs = abs(light_local);\n\
\x20   var uv: vec3<f32>;\n\
\x20   if (abs.x > abs.y && abs.x > abs.z) {\n\
\x20       uv = vec3<f32>(select(-1.0, 1.0, light_local.x > 0.0), -light_local.y, -light_local.z) / abs.x;\n\
\x20   } else if (abs.y > abs.z) {\n\
\x20       uv = vec3<f32>(light_local.x, select(-1.0, 1.0, light_local.y > 0.0), light_local.z) / abs.y;\n\
\x20   } else {\n\
\x20       uv = vec3<f32>(light_local.x, -light_local.y, select(-1.0, 1.0, light_local.z > 0.0)) / abs.z;\n\
\x20   }\n\
\x20   return uv;\n\
}\n\
\n\
// 虚拟页坐标 → 物理槽位。返回 `-1` = 这一页没分配（采样侧照\"不在影里\"处理）。\n\
fn px_shadow_page_slot(light_id: u32, face: u32, page_x: u32, page_y: u32) -> i32 {\n\
\x20   let head = px_shadow_pages[px_shadow_light_offsets[light_id]];\n\
\x20   let pages_per_side = head >> 16u;\n\
\x20   if (page_x >= pages_per_side || page_y >= pages_per_side) { return -1; }\n\
\x20   let words_per_row = px_shadow_pages[px_shadow_light_offsets[light_id] + 1u];\n\
\x20   let face_words = px_shadow_pages[px_shadow_light_offsets[light_id] + 2u];\n\
\x20   let row_words = 1u + words_per_row;\n\
\x20   let base = px_shadow_light_offsets[light_id] + 3u\n\
\x20       + face * face_words + page_y * row_words;\n\
\x20   let row_base = px_shadow_pages[base];\n\
\x20   // 这一行里位于我前面（含我）的占用位数 - 1 ⇒ 我在这一行里的第几个。\n\
\x20   let word_index = page_x >> 5u;\n\
\x20   let bit = page_x & 31u;\n\
\x20   var rank: u32 = 0u;\n\
\x20   var word: u32 = 0u;\n\
\x20   loop {\n\
\x20       if (word > word_index) { break; }\n\
\x20       let bits = px_shadow_pages[base + 1u + word];\n\
\x20       if (word == word_index) {\n\
\x20           // 含本位的掩码：`bit == 31` 时全 1（`1u << 32` 是未定义）。\n\
\x20           let mask = select((1u << (bit + 1u)) - 1u, 0xFFFFFFFFu, bit == 31u);\n\
\x20           rank = rank + countOneBits(bits & mask);\n\
\x20           break;\n\
\x20       }\n\
\x20       rank = rank + countOneBits(bits);\n\
\x20       word = word + 1u;\n\
\x20   }\n\
\x20   if (rank == 0u) { return -1; }\n\
\x20   return i32(row_base + rank - 1u);\n\
}\n\
\n\
// 一个采样点：查页 → 取 texel → **手动比较**（reverse-Z ⇒ 影里是 `depth < 存的深度`）。\n\
fn px_sample_shadow_page(\n\
\x20   light_id: u32,\n\
\x20   face: u32,\n\
\x20   uv: vec2<f32>,\n\
\x20   depth: f32,\n\
) -> f32 {\n\
\x20   let head = px_shadow_pages[px_shadow_light_offsets[light_id]];\n\
\x20   let pages_per_side = head >> 16u;\n\
\x20   // `uv` 在 `[-1, 1]` ⇒ 虚拟页格坐标。\n\
\x20   let page_f = (uv * 0.5 + 0.5) * f32(pages_per_side);\n\
\x20   let page_x = u32(clamp(page_f.x, 0.0, f32(pages_per_side) - 1.0));\n\
\x20   let page_y = u32(clamp(page_f.y, 0.0, f32(pages_per_side) - 1.0));\n\
\x20   let slot = px_shadow_page_slot(light_id, face, page_x, page_y);\n\
\x20   if (slot < 0) { return 1.0; }  // 没分配 ⇒ 不受影\n\
\x20   let atlas_pages = pages_per_side;  // 分配器按 2 的幂取层内边长\n\
\x20   let slot_u = u32(slot);\n\
\x20   let texel = vec2<u32>(\n\
\x20       (slot_u % atlas_pages) * PX_PAGE_SIZE,\n\
\x20       (slot_u / atlas_pages) * PX_PAGE_SIZE,\n\
\x20   ) + vec2<u32>(\n\
\x20       u32(clamp((uv.x * 0.5 + 0.5) * f32(pages_per_side) * f32(PX_PAGE_SIZE)\n\
\x20           - f32(page_x) * f32(PX_PAGE_SIZE), 0.0, f32(PX_PAGE_SIZE) - 1.0)),\n\
\x20       u32(clamp((uv.y * 0.5 + 0.5) * f32(pages_per_side) * f32(PX_PAGE_SIZE)\n\
\x20           - f32(page_y) * f32(PX_PAGE_SIZE), 0.0, f32(PX_PAGE_SIZE) - 1.0)),\n\
\x20   );\n\
\x20   let stored = textureLoad(point_shadow_textures, texel, i32(light_id * PX_CUBE_FACES + face), 0);\n\
\x20   // ⚠ **无限 reverse-Z**：近处是 1.0、远处是 0.0（`camera.rs` 那条\n\
\x20   //    `perspective_infinite_reverse_rh`）。所以我在影里 = 我比影图里记的更近\n\
\x20   //    ⇒ `depth < stored`。\n\
\x20   //    这一条写反过的症状不是「影没了」，而是**整颗行星发暗**（`depth > stored` 在\n\
\x20   //    这个方向上几乎恒真）—— 实测：写反那一版行星几乎全黑，只剩边缘一条亮。\n\
\x20   return select(0.0, 1.0, depth < stored);\n\
}\n\
\n\
const PX_POINT_SHADOW_SCALE: f32 = 0.003;\n\
\n\
fn px_sample_shadow_at_offset(\n\
\x20   position: vec2<f32>,\n\
\x20   coeff: f32,\n\
\x20   x_basis: vec3<f32>,\n\
\x20   y_basis: vec3<f32>,\n\
\x20   light_local: vec3<f32>,\n\
\x20   depth: f32,\n\
\x20   light_id: u32,\n\
) -> f32 {\n\
\x20   let dir = light_local + position.x * x_basis + position.y * y_basis;\n\
\x20   let uv = px_shadow_face_uv(dir);\n\
\x20   let face = u32(uv.z);\n\
\x20   return px_sample_shadow_page(light_id, face, uv.xy, depth) * coeff;\n\
}\n\
\n\
fn fetch_point_shadow(\n\
\x20   light_id: u32,\n\
\x20   frag_position: vec4<f32>,\n\
\x20   surface_normal: vec3<f32>,\n\
\x20   frag_coord_xy: vec2<f32>,\n\
) -> f32 {\n\
\x20   let light = &clustered_lights.data[light_id];\n\
\x20   let surface_to_light = (*light).position_radius.xyz - frag_position.xyz;\n\
\x20   let surface_to_light_abs = abs(surface_to_light);\n\
\x20   let distance_to_light = max(\n\
\x20       surface_to_light_abs.x,\n\
\x20       max(surface_to_light_abs.y, surface_to_light_abs.z),\n\
\x20   );\n\
\x20   // ⚠ **normal_offset 按\"当地一个 texel 有多大世界\"缩放**（§本轮）：\n\
\x20   //    从前这里是 `shadow_normal_bias * distance_to_light * N`，而那个\n\
\x20   //    `shadow_normal_bias` 是按固定 1024² 的边长算的（`0.6 × (2/1024) × √2`）\n\
\x20   //    ⇒ 太阳拉远时偏移按距离涨，整颗行星被自己的影压暗一档（实测平均通道差 4.43）。\n\
\x20   //    现在 `shadow_normal_bias` 只说\"偏移几个 texel\"，而 texel 的世界尺寸\n\
\x20   //    = `2 · distance / N_virt`（虚拟面边长由页表头给）⇒ 与\"灯多远\"无关。\n\
\x20   let head = px_shadow_pages[px_shadow_light_offsets[light_id]];\n\
\x20   let virtual_size = f32(head & 0xFFFFu);\n\
\x20   let texel_world = 2.0 * distance_to_light / max(virtual_size, 1.0);\n\
\x20   let normal_offset = (*light).shadow_normal_bias * texel_world * surface_normal.xyz;\n\
\x20   let depth_offset = (*light).shadow_depth_bias * normalize(surface_to_light.xyz);\n\
\x20   let offset_position = frag_position.xyz + normal_offset + depth_offset;\n\
\x20   let frag_ls = offset_position.xyz - (*light).position_radius.xyz;\n\
\x20   let abs_position_ls = abs(frag_ls);\n\
\x20   let major_axis_magnitude = max(\n\
\x20       abs_position_ls.x,\n\
\x20       max(abs_position_ls.y, abs_position_ls.z),\n\
\x20   );\n\
\x20   let zw = -major_axis_magnitude * (*light).light_custom_data.xy\n\
\x20       + (*light).light_custom_data.zw;\n\
\x20   let depth = zw.x / zw.y;\n\
\x20   let light_local = frag_ls * vec3<f32>(1.0, 1.0, -1.0);\n\
\x20   let basis = orthonormalize(normalize(light_local))\n\
\x20       * PX_POINT_SHADOW_SCALE * texel_world;\n\
\x20   var sum: f32 = 0.0;\n\
\x20   sum += px_sample_shadow_at_offset(\n\
\x20       PX_D3D_SAMPLE_POINT_POSITIONS[0], PX_D3D_SAMPLE_POINT_COEFFS[0],\n\
\x20       basis[0], basis[1], light_local, depth, light_id);\n\
\x20   sum += px_sample_shadow_at_offset(\n\
\x20       PX_D3D_SAMPLE_POINT_POSITIONS[1], PX_D3D_SAMPLE_POINT_COEFFS[1],\n\
\x20       basis[0], basis[1], light_local, depth, light_id);\n\
\x20   sum += px_sample_shadow_at_offset(\n\
\x20       PX_D3D_SAMPLE_POINT_POSITIONS[2], PX_D3D_SAMPLE_POINT_COEFFS[2],\n\
\x20       basis[0], basis[1], light_local, depth, light_id);\n\
\x20   sum += px_sample_shadow_at_offset(\n\
\x20       PX_D3D_SAMPLE_POINT_POSITIONS[3], PX_D3D_SAMPLE_POINT_COEFFS[3],\n\
\x20       basis[0], basis[1], light_local, depth, light_id);\n\
\x20   sum += px_sample_shadow_at_offset(\n\
\x20       PX_D3D_SAMPLE_POINT_POSITIONS[4], PX_D3D_SAMPLE_POINT_COEFFS[4],\n\
\x20       basis[0], basis[1], light_local, depth, light_id);\n\
\x20   sum += px_sample_shadow_at_offset(\n\
\x20       PX_D3D_SAMPLE_POINT_POSITIONS[5], PX_D3D_SAMPLE_POINT_COEFFS[5],\n\
\x20       basis[0], basis[1], light_local, depth, light_id);\n\
\x20   sum += px_sample_shadow_at_offset(\n\
\x20       PX_D3D_SAMPLE_POINT_POSITIONS[6], PX_D3D_SAMPLE_POINT_COEFFS[6],\n\
\x20       basis[0], basis[1], light_local, depth, light_id);\n\
\x20   sum += px_sample_shadow_at_offset(\n\
\x20       PX_D3D_SAMPLE_POINT_POSITIONS[7], PX_D3D_SAMPLE_POINT_COEFFS[7],\n\
\x20       basis[0], basis[1], light_local, depth, light_id);\n\
\x20   return sum;\n\
}\n";

/// 视图变换表里**必须**与 Bevy 逐字同语义的那一个符号：`depth_ndc_to_view_z`。
///
/// ⚠ 这是我们与 Bevy 那张桩表的**第二处**语义差别（第一处是 [`POINT_SHADOW_STUB`]），
/// 而它不属于"先凑合"那一类：`px_shader::assemble::bevy_stub` 给的是
/// `-1.0 / max(ndc_depth, 1e-6)`，Bevy 的真本是
/// `-perspective_camera_near() / ndc_depth`
/// （`bevy_pbr-0.19.1/src/render/view_transformations.wgsl:172`），
/// 其中 `perspective_camera_near() = view.clip_from_view[3][2]`。两个式子**差一个 near**。
///
/// 差多少（本机实测：Vulkan／RTX 3060／960×640／`orbit-bare-nolight`）：
/// - 深度预通道在盘心写下 **0.045503**；真距离 = `0.1 / 0.045503` = **2.198**
///   （与相机到行星表面 3.15 − 1.0 ≈ 2.15 吻合），而 `-1/depth` 给的是 **21.98**；
/// - 大气里 `end = min(entry + 2·outer·cos, length(scene))` 于是**永远取前者**（截断不生效）：
///   弦长从 0.067 变成 2.2 ⇒ alpha 从 **≈0.034** 变成 **≈0.48**；
/// - 症状：一颗**被冲淡的灰蓝球**（对照图 `target/tmp/experiment-atm-real-depth.png`），
///   而不是 oracle 那种"薄薄一层纱 + 边缘一圈晕"。
///
/// ⚠ 它**跑得起来、也不报错** —— 建管线、出图、哈希全都有值，只有画面不对。
/// 这一条绊线就是为这种"看起来也能跑"的桩准备的，所以上面那几个数留在现场。
///
/// ⚠ 为什么写 `view.clip_from_view[3][2]` 而不是把 `0.1` 抄进来：near 是**相机矩阵**里的数，
/// 抄进 Rust 侧（或抄进这个函数体）就是 §66.1 那颗「同一条契约、两个数」的雷 ——
/// 换一台相机就漂开，而画面上只表现为"大气的厚薄不对"。
///
/// ⚠ 函数体引用了 `view`，所以它**只能**进那些同时 import 了 `view` 的 shader
/// （`atmosphere.wgsl` / `clouds.wgsl` 都是）。判据在
/// `tests::the_depth_override_assembles_against_the_real_content`：少一个声明，
/// naga 会当场拒 —— 那正是我们要的失败方式。
pub const DEPTH_NDC_TO_VIEW_Z: &str = "fn depth_ndc_to_view_z(ndc_depth: f32) -> f32 {\n\
                                       \x20   return -view.clip_from_view[3][2] / ndc_depth;\n\
                                       }\n";

/// 这个宿主的桩表。函数指针（不是泛型、不是 trait）：组装器只有一份，见 `assemble::Stubs`。
pub fn wgpu_host_stub(symbol: &str) -> Option<&'static str> {
    match symbol {
        "bevy_pbr::shadows::fetch_point_shadow" => Some(POINT_SHADOW_STUB),
        "bevy_pbr::view_transformations::depth_ndc_to_view_z" => Some(DEPTH_NDC_TO_VIEW_Z),
        "bevy_pbr::mesh_view_bindings::view" => Some(HOST_VIEW_STUB),
        other => bevy_stub(other),
    }
}
