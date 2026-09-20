/// 图侧给的**场函数**：上游值按**径向波纹条带化**，再用 `params` 调对比与偏移（给泛型实例用）。
///
/// 语义一眼能懂：以 UV 中心为圆心，按半径撒 `bands` 圈正弦条带；上游值拿去**挪相位**
/// （上游亮 ⇒ 条带整体往里缩），`gain` 管对比、`bias` 管整体抬落。
///
/// ⚠ 这一份会被 `px build` 生成的实例库**原样 `include!`**（`19` §179.1）：
///   * `use` 一律写全路径（生成物里没有 `crate::` 那个前缀可指）；
///   * 不放 `#[cfg(test)]`、不引任何本 crate 的私有名字 —— 它只认识 `px_field_alg`；
///   * 算出来必须落在 `[0,1]` —— 这是**场**的口径（`0` = 谷、`1` = 峰）；最后那次 `clamp`
///     就是这条保证的落点，`gain` / `bands` / `bias` 取任何值都不会把它顶出去。
pub struct Waves;

impl px_field_alg::field_fn::FieldFn for Waves {
    /// ⚠ `params` 是**这个节点在 `art/field_remap/bands.toml` 里给的那一份**（图参数）。
    ///   2026-09-20 之前这一栏不存在，本文件只能读 `RemapParams::default()` ⇒ 改 TOML
    ///   **只换节点键、不换内容**（那是一处静默失效，见 `px_field_alg::field_fn::FieldFn`）。
    /// ⚠ `upstream` 是**算子在调这一行之前**归一化过的值（`[0,1]`），不是上游那张场的原值
    ///   —— "归一化 + 钳制"那一段住在 `px_field_alg` 里（共享路径），图侧函数只管"这一格
    ///   的值怎么算"。`uv` 是这一格的**纹素中心**坐标（`[0,1]²`，与 `Field::uv` 同一口径）；
    ///   `_direction` 用不上（径向条带按图像中心算就够）。
    fn value(
        &self,
        params: &px_field_schema::params::RemapParams,
        upstream: f32,
        uv: [f32; 2],
        _direction: [f32; 3],
    ) -> f32 {
        // 到 UV 中心的半径：`[0, ~0.707]`（角上）。
        let dx = uv[0] - 0.5;
        let dy = uv[1] - 0.5;
        let radius = (dx * dx + dy * dy).sqrt();
        let bands = if params.bands > 0.0 { params.bands } else { 1.0 };
        // 上游挪相位（`0.25` 是让相位走在 0 附近，别整圈空转）：上游亮 ⇒ 条带整体往里缩。
        let wave = 0.5 + 0.5 * (radius * bands - (upstream - 0.25)).sin();
        // 对比：以 0.5 为轴把落点推开（`gain = 0` ⇒ 不动）。
        let contrasted = 0.5 + (wave - 0.5) * (1.0 + params.gain) + params.bias;
        contrasted.clamp(0.0, 1.0)
    }
}
