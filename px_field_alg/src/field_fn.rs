//! **场函数**：场域泛型算子唯一需要的那个抽象 —— 「给上游那一格的值与归一化坐标 `[0,1]²`，
//! 回这一格的值」。
//!
//! 一个实现：[`Sampled`] —— 把上游那张**采样好的场**当函数（`(upstream, uv) ↦ upstream`）。
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

/// **图侧给的场函数**：给上游那一格的值与归一化坐标（`[0,1]²`），返回这一格的值。
///
/// * `upstream` —— [`Upstream::upstream`] 在这一格采到的值（**未钳制**：它可能来自
///   任意一条上游链，不保证在 `[0,1]` 里）；
/// * `uv` —— 这一格的归一化坐标（**纹素中心**口径，与 `Field::uv` 同一件事：`x` 从
///   `0.5/width` 到 `1 - 0.5/width`）。⚠ 它不是"格点编号"，而是能直接喂给 `sin` /
///   距离 / 噪声的那种坐标；
/// * 返回值 —— 由实现者保证在 `[0,1]`（算子的钳制只管**参数**那一档
///   `RemapParams::out_min` / `out_max`，不替实现者猜值域）。
pub trait FieldFn {
    fn value(&self, upstream: f32, uv: [f32; 2]) -> f32;
}

/// **上游场在格点上的值** —— 算法这一侧唯一需要的那个输入抽象。
///
/// ⚠ 有它才有"**同一条计算路径**"：预设那一档与图侧现写的那一档都走
///   [`crate::remap_with`]，差别只在 `U` 是谁 —— 于是"同一份参数、同一个上游 ⇒ 同一个场"
///   不靠人工同步两份循环，而靠**只有一份循环**。
pub trait Upstream {
    /// 这一格的归一化坐标（纹素中心，与 `Field::uv` 同一口径）。
    fn uv(&self, x: u32, y: u32) -> [f32; 2];
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

    fn upstream(&self, x: u32, y: u32) -> f32 {
        self.field.at(x, y)
    }
}

impl FieldFn for Sampled<'_> {
    fn value(&self, upstream: f32, _uv: [f32; 2]) -> f32 {
        upstream
    }
}
