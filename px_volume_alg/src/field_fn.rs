//! **场函数**：体积算子唯一需要的那个抽象 —— 「给一个方向，回一个覆盖度」。
//!
//! 一个实现：[`SampleField`] —— 把上游那张**采样好的场**当函数。
//!
//! ⚠ **它仍然是泛型参数，不是 `dyn`**：`bake<F: FieldFn>` 在每个 `F` 上单态化。
//!   但它是**本实现库内部**的抽象：唯一那个实例（[`SampleField`]）就编在这份 dylib 里，
//!   图脚本那一侧只把一个**已经采样好的场**交给算子 —— 泛型怎么实例化，图侧看不见。
//!
//! ⚠⚠ 从前这里还有第二个实现 [`ClosedForm`]（图脚本现写的闭式场函数），配套一条
//!   "生成的单态化实例 + 按 op id 动态装载"的路：泛型实例编成 dylib，图程序在运行时
//!   接上，于是**改场函数不必重编图程序**。那条路连同生成器一起删掉了
//!   （原型期的决定：那种"运行时按 id 找"正是要避免的）。
//!   ⇒ 今天泛型算子那一条走的是**另一条**路：把泛型参数在图侧实例化成一个**场**
//!   （见 `px_graphs/src/bin/clouds.rs` 的 `CloudCoarseInput { coverage }`）。
//!
//! ⚠ 这一份现在住在 **`px_volume_alg`（rlib）**：`px_volume_op`（dylib 薄壳）与
//!   泛型实例库（`px_inst!` 生成的）都把它链进去 —— 两边必须共用同一份抽象，
//!   否则"同一个覆盖度场"会经两条不同的包装走到两个不同的结果上。
//!   （上面那句"本实现库"因此要读成"本算法 crate"：`px_volume_op` 只是它的壳。）

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
/// `cover_from_mask` 重映射 —— 逐位不变的读数就是靠这一条。
pub struct SampleField<'a> {
    pub field: &'a Field,
}

impl FieldFn for SampleField<'_> {
    fn cover(&self, cloud: &CoverCloud, direction: [f32; 3]) -> f32 {
        proxy::cover_at(cloud, self.field, direction)
    }
}
