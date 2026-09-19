//! **px-scene**：pcg 图程序用的**高层场景语义层** —— 把内容编译成**低层帧图**（`.pxart`）。
//!
//! 分层（依赖方向是硬的）：
//!
//! ```text
//! px_protocol ─ 类型与 [px-scene ⇄ px-pass] 的交换格式
//! px_graph      ─ 算子 + 内容寻址 CAS（成员按名字取键、生成物落盘）
//! px_shader   ─ WGSL 组装与反射（参数的**名字 ↔ 字节**那一份真源）
//!      ↓
//! px_scene    ─ 场景语义 + **帧图编译器**（这一层）
//!      ↓
//! px_graphs   ─ pcg 的图程序（`bin/*.rs`），像用寻常引擎那样注册材质与 stage
//! ```
//!
//! ## 这里的四条口径
//!
//! 1. **产物是低层帧图**：一份 [`SceneSpec`]（物体 + 灯 + 环境 + 相机 + `resources` /
//!    `passes` / `frame_materials` / `material_instances`）。`px_pass` 只按 `kind` 与状态
//!    分派，它一个标签都不认识（§124）。
//! 2. **pipeline 是一串 stage，每个 stage 只要求 material 里那些 *per pass* 参数的类型**
//!    （[`StageParams`]）：图程序里手写 `impl StageParams<Stage, 内容> for Stage`（不进
//!    WGSL 反推）。编译时逐项对账 —— 缺格 / 塞不进在**烘图时**就红，而不是装载时（§81.4）。
//! 3. **一个物体可以注册多个 stage 的材质**；寻常那一路由 [`Registration::single`] 包成
//!    "一份材质" —— 用起来与寻常引擎一样。⚠ 这份多档状态**只在内存里**：产物仍然是
//!    「一个物体一份材质」（登记在加物体那一刻 `freeze` 成那一份）。
//! 4. **stage 与内容都是类型**（[`Stage`] / [`Content`]），这一层没有类型擦除：漏写某一档的
//!    参数表、或给某份内容登记它不参与的档，都是**编译错误**。反射仍然是"名字 ↔ 字节"的
//!    真源（[`contract`]），两者各管一半。
//!
//! ⚠ 装配场景请用**通用**的 [`SceneBuilder`]：它不认识行星 / 云 / 大气，也不认识 `art/`
//! 下的路径 —— 那些语义住在 [`recipe`]（TOML 配方的适配器）与图程序里。

pub mod baked;
pub mod builder;
pub mod contract;
pub mod frame;
pub mod math;
pub mod members;
pub mod recipe;
pub mod stage;
pub mod vocab;

pub use baked::Baked;
pub use builder::{SceneBuilder, MaterialBuilder};
pub use contract::{merge_named, schema_of, shader_parts_of};
pub use frame::{FrameFile, Sources, DEFAULT_FRAME};
pub use recipe::{compile, load, material_params, recipe_path, SceneFile};
pub use stage::{
    Atmosphere, CheckStages, Clouds, Content, Given, Opaque, PointShadow, Prepass, Registration,
    Ring, Sky, Skybox, Stage, StageParams, Surface, Transparent,
};
pub use vocab::{
    CLOUD_BASE, CLOUD_SHADOW_GAIN, CLOUD_SHADOW_HEIGHT, CLOUD_TOP, SKYBOX_BRIGHTNESS,
    SUN_RANGE_FACTOR, SYSTEM_TILT,
};
