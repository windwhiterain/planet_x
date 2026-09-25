/// 图侧给的覆盖度函数：覆盖度在**从中心看出去的那个方向上起波带**（样本，给 recipe 里那条实例用）。
///
/// ⚠ 这一份会被 `px build` 生成的实例库**原样 `include!`**：
///   * `use` 一律写全路径（生成物里没有 `crate::` 那个前缀可指）；
///   * 不放 `#[cfg(test)]`、不引任何本 crate 的私有名字 —— 它只认识 `px_volume_alg`；
///   * 算出来必须落在 `[0,1]` —— 这是**覆盖度**的口径（`0` = 没有云、`1` = 满）。
pub struct Band;

impl px_volume_alg::field_fn::FieldFn for Band {
    /// ⚠ `_cloud` 不用：波带**自己**就是覆盖度的来源（不再去采样上游那张场）。
    ///   `bake` 会把云那一档形状参数递进来，但本样本只按方向算。
    fn cover(
        &self,
        _cloud: &px_volume_alg::field_fn::CoverCloud,
        direction: [f32; 3],
    ) -> f32 {
        // 纬度带：0.5 + 0.5·sin ⇒ 本来就落在 [0,1]（再 clamp 一次是防空壳口径的保险）。
        (0.5 + 0.5 * (direction[1] * 12.0).sin()).clamp(0.0, 1.0)
    }
}
