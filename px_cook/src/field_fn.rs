//! **场函数**：算子唯一需要的那个抽象 —— 「给一个方向，回一个覆盖度」。
//!
//! 两个实现：
//! * [`SampleField`]：把上游那张**采样好的场**当函数（老路径，dylib 那一半用它）；
//! * 图脚本自己的**闭式场**（普通 Rust 结构 + `impl FieldFn`）—— 于是覆盖度
//!   **不必先栅格化成一张固定分辨率的场、再在 3D 里回采**。
//!
//! ⚠ 它是**泛型参数**，不是 `dyn`：`bake<F: FieldFn>` 在每个 `F` 上单态化一次，
//! 实例由**图脚本那一侧**生成（dylib 自己不持有实例）。
//!
//! ⚠⚠ 代价（对着 `17-typed-ops.md` §161 的读数读）：泛型体一旦被图脚本实例化，
//! 它就**静态链进了图程序** ⇒ 改 `bake` 的体要重编图脚本。老路径（dylib 里那份具体的
//! `bake`）仍然保留，所以「改算子不重编图程序」没有全丢。

use px_field_schema::field::Field;
use px_verify::proxy;
use px_volume_schema::Params;

/// 算子交给场函数的**上下文**：云的那一档形状参数（`proxy::from_volume` 的结果）。
///
/// ⚠ 它由算子建、场函数用 —— 于是「云参数怎么算」只有一处（`proxy::from_volume`），
/// 闭式场不必自己再推一遍 `to_local` / `cover_from_mask` 的口径。
pub type CoverCloud = px_verify::cloud_field::CloudFieldParams;

/// 世界方向 → 覆盖度。与 `px_verify::proxy::cover_at` 同一口径。
pub trait FieldFn {
    fn cover(&self, cloud: &CoverCloud, direction: [f32; 3]) -> f32;
}

/// 把上游那张采样好的场当函数用。
///
/// ⚠ 与 `proxy::cover_at` **逐步同一件事**：转进场的局部系 → 按投影采样 →
/// `cover_from_mask` 重映射。老路径逐位不变就是靠这一条。
pub struct SampleField<'a> {
    pub field: &'a Field,
}

impl FieldFn for SampleField<'_> {
    fn cover(&self, cloud: &CoverCloud, direction: [f32; 3]) -> f32 {
        proxy::cover_at(cloud, self.field, direction)
    }
}

/// 一个常量覆盖度：调试与"不传场函数"那条路的替身。
pub struct ConstantField(pub f32);

impl FieldFn for ConstantField {
    fn cover(&self, _cloud: &CoverCloud, _direction: [f32; 3]) -> f32 {
        self.0
    }
}

/// 闭式场最顺手的那种写法：图谱上现算的覆盖度。
///
/// `f` 收 `(云参数, 世界方向)`；`Params` 这一层不用管 —— 云参数已经建好了。
pub struct ClosedForm<F> {
    pub f: F,
}

impl<F: Fn(&CoverCloud, [f32; 3]) -> f32> FieldFn for ClosedForm<F> {
    fn cover(&self, cloud: &CoverCloud, direction: [f32; 3]) -> f32 {
        (self.f)(cloud, direction)
    }
}

/// 图脚本要用的那个入口：`Params` 是谁的、`Field` 从哪来，都在这两个类型里说清。
pub fn _params_marker(_: &Params) {}
