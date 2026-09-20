/// 图侧给的**场函数**：**纬向条带**（气态巨行星那种横向云带）。
///
/// 语义：拿这一格的**球面方向**取纬度（`direction.y ∈ [-1,1]`，两极 = ±1），按纬度撒 `bands`
/// 圈正弦；上游（湍流场）拿去**挪相位** —— 于是条带边界跟着湍流起伏，而不是一圈死板的正弦。
/// `gain` 管对比（band 与 belt 的亮暗差）、`bias` 管整体抬落。
///
/// ⚠ 纬度只能从 `direction` 取，**不能拿 `uv[1]`**：气态巨行星的覆盖度场是 `CubeMap` 投影
///   （六张面沿 `y` 叠成一条，`height = 6 × face`），`uv[1]` 在面与面之间会跳变。
///   `direction` 这一栏是 2026-09-20 才递到场函数的（在那之前球面函数写不进实例库）。
///
/// ⚠ 这一份会被 `px build` 生成的实例库**原样 `include!`**（`19` §179.1）：
///   * `use` 一律写全路径（生成物里没有 `crate::` 那个前缀可指）；
///   * 不放 `#[cfg(test)]`、不引任何本 crate 的私有名字 —— 它只认识 `px_field_alg`；
///   * 算出来必须落在 `[0,1]` —— 这是**覆盖度**的口径（`0` = 没有云、`1` = 满）；最后那次
///     `clamp` 就是这条保证的落点。
pub struct LatBands;

impl px_field_alg::field_fn::FieldFn for LatBands {
    /// ⚠ `params` 来自 `art/gasgiant/bands.toml`（`bands` = 条带圈数、`gain` = 对比、`bias` = 抬落）；
    ///   `upstream` 已经过共享那把尺子（归一化到 `[0,1]`）。
    fn value(
        &self,
        params: &px_field_schema::params::RemapParams,
        upstream: f32,
        _uv: [f32; 2],
        direction: [f32; 3],
    ) -> f32 {
        let bands = if params.bands > 0.0 { params.bands } else { 1.0 };
        // 纬度：`direction.y`（球面上的 y 就是自转轴方向）⇒ `[-1, 1]`。
        let latitude = direction[1].clamp(-1.0, 1.0);
        // 上游挪相位：湍流亮的地方条带整体往北偏，于是带边界是**波浪形**而不是一圈死正弦。
        let wave = 0.5 + 0.5 * (latitude * bands * std::f32::consts::PI - (upstream - 0.5) * 3.5).sin();
        // 对比：以 0.5 为轴把落点推开（`gain = 0` ⇒ 不动）。
        let contrasted = 0.5 + (wave - 0.5) * (1.0 + params.gain) + params.bias;
        contrasted.clamp(0.0, 1.0)
    }
}
