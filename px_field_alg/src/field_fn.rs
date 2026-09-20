//! **场函数**：场域泛型算子唯一需要的那个抽象 —— 「给这个节点的参数、上游那一格的值、
//! 归一化坐标 `[0,1]²` 与球面方向，回这一格的值」。
//!
//! 一个实现：[`Sampled`] —— 把上游那张**采样好的场**当函数（`(upstream, …) ↦ upstream`）。
//!
//! ⚠ 与体积域的 `px_volume_alg::field_fn::FieldFn` **同性质**：纯函数、只吃 `f32`、
//!   由实现者保证落在值域内。这里的值域是 **[0,1]**。
//!
//! ⚠ **它仍然是泛型参数，不是 `dyn`**：`remap_with<F: FieldFn>` 在每个 `F` 上单态化，
//!   而 `F` 的实现住在**被 `include!` 进来的外部源码**里（`art/inst/waves.rs` 那种）。
//!   `dead_code` 只看本 crate 自己用没用 ⇒ 本模块由 `lib.rs` 那一行的
//!   `#[allow(dead_code)]` 替它收声；这正是这个机制的性质（见 `px_volume_alg::field_fn`
//!   的同一条注释）。

use px_field_schema::field::Field;
use px_field_schema::params::RemapParams;

/// **图侧给的场函数**：给这个节点的参数、上游那一格的值、归一化坐标（`[0,1]²`）与**球面方向**，
/// 返回这一格的值。
///
/// * `params` —— **这个节点在 `art/<图>/<节点名>.toml` 里给的那一份**（`RemapParams`）。
///   ⚠ 2026-09-20 之前它**到不了这里**：`remap_with` 把它丢掉了（`let _ = params`），而图侧函数
///   只能自己读 `RemapParams::default()`。后果是**参数进了键、却不进计算**：改
///   `art/field_remap/bands.toml` 里的 `bands` 会换节点键、会重算，写出来的产物却**逐字节相同**
///   ——实测（`bands = 3 / gain = 0` 对不写文件）内容都是 `bc8ab272f232`，只是 CAS 键不同。
///   体积域那边从第一天起就是把参数递给场函数的（`CoverCloud` 由算子建、`cover` 收它）
///   —— 场域这一档现在是同一条规矩；
/// * `upstream` —— [`Upstream::upstream`] 在这一格采到的值，**已经过共享那把尺子**
///   （归一化 + 钳制 + 可选平滑）；
/// * `uv` —— 这一格的归一化坐标（**纹素中心**口径，与 `Field::uv` 同一件事：`x` 从
///   `0.5/width` 到 `1 - 0.5/width`）。⚠ 它不是"格点编号"，而是能直接喂给 `sin` /
///   距离 / 噪声的那种坐标；
/// * `direction` —— 这一格在**球面上的单位方向**（`Field::direction` 的同一件事：按投影
///   算出来的那一个）。⚠ **行星美术要"这一格在球上哪儿"，只能用它**：`uv` 是**图像坐标**，
///   在 `CubeMap` 投影下 `v` 跨的是"六张面叠起来的那一条"（`height = 6 × face`）而不是纬度
///   —— 拿 `uv[1]` 当纬度会在面与面之间跳变。体积域的 `px_volume_alg::field_fn::FieldFn`
///   从第一天起就收 `direction`（那条线是对的），场域此前只给 `uv`，于是"纬向条带 / 极冠 /
///   陨坑"这类**球面**函数写不进实例库（2026-09-20 补上）；
/// * 返回值 —— 由实现者保证在 `[0,1]`（算子的钳制只管**参数**那一档
///   `RemapParams::out_min` / `out_max`，不替实现者猜值域）。
pub trait FieldFn {
    fn value(&self, params: &RemapParams, upstream: f32, uv: [f32; 2], direction: [f32; 3]) -> f32;
}

/// **上游场在格点上的值** —— 算法这一侧唯一需要的那个输入抽象。
///
/// ⚠ 有它才有"**同一条计算路径**"：预设那一档与图侧现写的那一档都走
///   [`crate::remap_with`]，差别只在 `U` 是谁 —— 于是"同一份参数、同一个上游 ⇒ 同一个场"
///   不靠人工同步两份循环，而靠**只有一份循环**。
pub trait Upstream {
    /// 这一格的归一化坐标（纹素中心，与 `Field::uv` 同一口径）。
    fn uv(&self, x: u32, y: u32) -> [f32; 2];
    /// 这一格在球面上的单位方向（与 `Field::direction` 同一口径）。
    fn direction(&self, x: u32, y: u32) -> [f32; 3];
    /// 这一格的上游值（**不钳制**）。
    fn upstream(&self, x: u32, y: u32) -> f32;
}

/// 把上游那张**采样好的场**当函数用：`(upstream, uv) ↦ upstream`。
///
/// ⚠ 图侧那位实现者自己采样上游时用的也是它（[`Upstream`]）—— 两边取到的是同一个值。
pub struct Sampled<'a> {
    pub field: &'a Field,
}

impl Upstream for Sampled<'_> {
    fn uv(&self, x: u32, y: u32) -> [f32; 2] {
        let (u, v) = self.field.uv(x, y);
        [u, v]
    }

    fn direction(&self, x: u32, y: u32) -> [f32; 3] {
        self.field.direction(x, y)
    }

    fn upstream(&self, x: u32, y: u32) -> f32 {
        self.field.at(x, y)
    }
}

impl FieldFn for Sampled<'_> {
    fn value(
        &self,
        _params: &RemapParams,
        upstream: f32,
        _uv: [f32; 2],
        _direction: [f32; 3],
    ) -> f32 {
        upstream
    }
}
