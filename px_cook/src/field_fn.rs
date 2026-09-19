//! **场函数**：体积算子唯一需要的那个抽象 —— 「给一个方向，回一个覆盖度」。
//!
//! 一个实现：[`SampleField`] —— 把上游那张**采样好的场**当函数。
//!
//! ⚠ **它仍然是泛型参数，不是 `dyn`**：`bake<F: FieldFn>` 在每个 `F` 上单态化。
//!   但实例**只有图程序那一份** —— 图脚本 `cook::<volume::CloudCoarse>(…)` 时
//!   静态链接进图程序。
//!
//! ⚠⚠ 从前这里还有第二个实现 [`ClosedForm`]（图脚本现写的闭式场函数），配套一条
//!   "生成的单态化实例 + 动态装载"的路：泛型实例编成 dylib，图程序按 op id 在运行时
//!   接上，于是**改场函数不必重编图程序**。那条路连同描述符表 / loader / 生成器一起
//!   删掉了（原型期的决定：那种"运行时按 id 找"正是要避免的）。
//!   ⇒ 现在的代价是明摆着的：**改 `bake` 的体、或改图脚本里现写的场函数，都要重编图程序**。

use px_field_schema::field::Field;
use px_verify::proxy;

/// 算子交给场函数的**上下文**：云的那一档形状参数（`proxy::from_volume` 的结果）。
///
/// ⚠ 它由算子建、场函数用 —— 于是「云参数怎么算」只有一处（`proxy::from_volume`），
/// 场函数不必自己再推一遍 `to_local` / `cover_from_mask` 的口径。
pub type CoverCloud = px_verify::cloud_field::CloudFieldParams;

/// 世界方向 → 覆盖度。与 `px_verify::proxy::cover_at` 同一口径。
pub trait FieldFn {
    fn cover(&self, cloud: &CoverCloud, direction: [f32; 3]) -> f32;
}

/// 把上游那张采样好的场当函数用。
///
/// ⚠ 与 `proxy::cover_at` **逐步同一件事**：转进场的局部系 → 按投影采样 →
/// `cover_from_mask` 重映射。"改算子不重编图程序"那条性质没了的今天，它也是
/// 唯一的实现 —— 逐位不变的读数就是靠这一条。
pub struct SampleField<'a> {
    pub field: &'a Field,
}

impl FieldFn for SampleField<'_> {
    fn cover(&self, cloud: &CoverCloud, direction: [f32; 3]) -> f32 {
        proxy::cover_at(cloud, self.field, direction)
    }
}
