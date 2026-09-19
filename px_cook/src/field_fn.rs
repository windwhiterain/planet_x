//! **场函数**：图侧现写的闭式场（`Fn([f32;3]) -> f32` 那一路）。
//!
//! ⚠ 它**故意不是**泛型单态化的入口：泛型要求「泛型定义」与「类型参数」在同一个编译单元组里，
//! 而这条边界的另一边是 dylib ⇒ 图脚本定义的场函数**过不去**。
//! 所以图侧给的是 `&dyn FieldFn`，vtable 每次采样一跳 —— 相对每步的噪声是噪声级开销。

/// 一个方向上的场值。`cover` 是上游给的覆盖度（0 = 壳外，算子可以据此早退）。
pub trait FieldFn {
    fn cover(&self, direction: [f32; 3]) -> f32;

    /// 壳外/覆盖度为 0 时的早退判据（与 `px_verify::proxy::cover_at` 同一口径）。
    fn is_empty(&self, direction: [f32; 3]) -> bool {
        self.cover(direction) <= 0.0
    }
}

/// 一个常量场：调试与「不传场函数」那条老路径的替身。
pub struct ConstantField(pub f32);

impl FieldFn for ConstantField {
    fn cover(&self, _direction: [f32; 3]) -> f32 {
        self.0
    }
}
