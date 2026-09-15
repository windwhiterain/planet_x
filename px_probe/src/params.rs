//! 云参数在**探针这一侧**的镜像：`art/shaders/clouds.wgsl` 里 `CloudParams` 的逐字对应物。
//!
//! 它原来住在 `px_render::clouds`（渲染器的材质结构体）。通用渲染之后渲染器不再认识云
//! —— 参数由烘图侧算好、按 shader 自己声明的结构体打包（`px_render::reflect`），
//! 渲染器里没有第二个 Rust 结构体。**探针仍然需要一份**：它的 compute shader 读的是同一块
//! uniform，而它是「梯度对错的唯一判据」（§46）的输入。
//!
//! ⚠ 字段顺序 = WGSL 结构体的顺序，一个都不能动（动了就是"同一个键、不同内容"）。
//! `px_render` 的反射门会读出 shader 那份的偏移，两边对不上的话是**编译期之外**的错误，
//! 所以这里只做镜像，不做加工。

use encase::ShaderType;
use glam::Vec4;

pub const CLOUD_BASE: f32 = 1.01;
pub const CLOUD_TOP: f32 = 1.06;

#[derive(Clone, Copy, Debug, ShaderType)]
pub struct CloudParams {
    pub orientation: Vec4,
    pub tint: Vec4,
    pub inner: f32,
    pub outer: f32,
    pub density: f32,
    pub coverage: f32,
    pub base: f32,
    pub top: f32,
    pub detail_scale: f32,
    pub detail_strength: f32,
    pub erode: f32,
    pub phase: f32,
    pub shadow: f32,
    pub steps: u32,
    pub bump: f32,
    pub seed: u32,
    pub ablate: u32,
    pub slope_scale: f32,
    pub taper: f32,
    pub coverage_gain: f32,
    pub surface_level: f32,
    pub bound: u32,
    pub gradient: u32,
    pub wind: f32,
    pub wind_skin: f32,
}

impl CloudParams {
    /// 云的"老那一档"缺省值（原来是 `px_render::clouds::CloudShape::default()` 盖上去的）。
    /// 探针的判据是"生产参数下两边一致"，所以这份数必须与当年那一档逐项相同。
    pub fn new(inner: f32, outer: f32, density: f32) -> Self {
        Self {
            orientation: Vec4::new(0.0, 0.0, 0.0, 1.0),
            tint: Vec4::new(1.0, 0.99, 0.97, 1.0),
            inner,
            outer,
            density,
            coverage: 0.35,
            base: 0.06,
            top: 0.62,
            detail_scale: 16.0,
            detail_strength: 0.55,
            erode: 0.0,
            phase: 0.62,
            shadow: 1.0,
            steps: 56,
            bump: 0.85,
            seed: 7,
            ablate: 0,
            slope_scale: 0.12,
            taper: 0.45,
            coverage_gain: 2.6,
            surface_level: 0.20,
            bound: 0,
            gradient: 1,
            wind: 0.0,
            wind_skin: 0.0,
        }
    }
}

// 这一档的字节（进 compute shader 的 uniform）由各自那一篇里的 `params_bytes` 写
// （`probe.rs` / `gradient.rs` 各一份，与它们自己的管线绑定在一起）—— 这里不再重复一份。
