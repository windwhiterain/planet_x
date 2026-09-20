/// 图侧给的**场函数**：**纬向条带**（气态巨行星那种横向云带）。
///
/// 语义：拿这一格的**球面方向**取纬度（`direction.y ∈ [-1,1]`，两极 = ±1），按纬度撒 `bands`
/// 圈带；上游（湍流场）拿去**挪相位**（带边界跟着湍流起伏）**并调制带里的细丝与强弱**
/// —— 于是它既不是一圈死正弦，也不是一张均匀的梳子。
/// `gain` 管对比（band 与 belt 的亮暗差）、`bias` 管整体抬落。
///
/// ⚠ 纬度只能从 `direction` 取，**不能拿 `uv[1]`**：气态巨行星的覆盖度场是 `CubeMap` 投影
///   （六张面沿 `y` 叠成一条，`height = 6 × face`），`uv[1]` 在面与面之间会跳变。
///   `direction` 这一栏是 2026-09-20 才递到场函数的（在那之前球面函数写不进实例库）。
///
/// ⚠ **2026-09-20 第二轮（观感）**：三处改动都对着 `art/reference/gasgiant-saturn.png`
///   （土星：带是**宽过渡**的，只有细丝状结构破坏规则性；天王星那张更极端 —— 几乎无特征）：
///   ① 剖面从**正弦**换成**三角波**（等斜率）—— 正弦在中点斜率最大，看着就是"一条硬边"；
///      三角波的过渡是**匀速**的，同样一圈带看着宽得多；
///   ② 加**细丝**：高频小涟漪，振幅骑在上游场上（亮的地方丝明显）；
///   ③ 加**天气**：沿经度分几个"带被洗淡"的段落（真实巨行星上那种扰动区），
///      让同一颗球上不是每条带都一样清楚。
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
        // ⚠ 2026-09-20（用户："气态行星表面的纹理是很不规则的，你的那个像个西瓜"）：
        //   "纯纬度 + 一圈正弦"出来的就是**西瓜**。真实的气态行星条带：
        //     · 带的**宽窄沿纬度差得很远**，相邻的带会**挤在一起或分开**；
        //     · 有些纬度**整片没有带**（均匀的区）；
        //     · 边界被湍流拉成分叉、打卷的长条。
        //   前两件事在这一支里做（第三件是图上那两级**域扭曲**的活，`field.warp`）：
        //   ① **相位摆动**：叠两个大振幅、低频的摆 —— 振幅与"一圈带"同量级（约 1.4 rad ≈
        //      半个周期），于是条纹会真实的**疏密不均、局部并拢**；
        //   ② **按纬度的淡出**：`fade` 让某些纬度带整体淡下去（那里就是"没有带"的区）。
        // ⚠ 2026-09-20（第 11 轮）：这两条摆从前是**正弦**——而正弦在球面上**周期性重复**
        //   （绕经度一圈回来是同一个样子，只是被"看不见的接缝"藏住了）。换成
        //   `px_field_alg::noise::value_noise3`（格点哈希 + 三线性插值）：同一个方向永远同一个值，
        //   但**没有任何周期** —— 实例库链的就是那个 crate，所以这一档现在两边都拿得到。
        let cell = |scale: f32, offset: f32| {
            [
                direction[0] * scale + offset,
                direction[1] * scale * 1.7 + offset,
                direction[2] * scale - offset,
            ]
        };
        let wobble = (px_field_alg::noise::value_noise3(cell(2.30, 0.0), 41) - 0.5) * 2.0
            + 0.55 * (px_field_alg::noise::value_noise3(cell(5.10, 3.7), 97) - 0.5) * 2.0;
        // ⚠ 2026-09-20（用户："band 的分布太均匀了，一般来说这些东西都在靠近赤道的地方"）：
        //   相位的**纬度映射不是线性的**。线性（`phase ∝ 纬度`）意味着从赤道到极区带一样密
        //   —— 而真实的条带/涡旋由**差动旋转的喷流**定，它们在低纬挤、往高纬被拉开成宽带。
        //   `g(y) = y − 0.24·y³` 的斜率 `g' = 1 − 0.72·y²`：赤道处最陡（带最密），
        //   两极处只有 0.28（带被拉宽约 3.6 倍）。⚠ 用三次项而不是 `|y|`：`|y|` 在赤道有个尖点，
        //   会在赤道压出一条假的窄带。
        let stretched = latitude - 0.24 * latitude * latitude * latitude;
        // 相位：纬度给"几圈带"，摆动与上游各给一部分"边界怎么歪"。
        let phase = stretched * bands * std::f32::consts::PI + wobble * 0.75 - (upstream - 0.5) * 2.4;

        // ① 相位 → **三角波**（等斜率、周期 2π）：`fract` 把它折回 `[0,1)`，再从 1 折回 0。
        //   ⚠ 相对正弦，这一条把"过渡"从"中点最陡"改成"全程同陡" ⇒ 同一条带看着宽得多。
        let folded = (phase * (1.0 / (2.0 * std::f32::consts::PI)))
            .fract()
            .abs();
        let wave = 1.0 - (2.0 * folded - 1.0).abs();

        // ② 细丝：高频小涟漪，振幅骑在上游场上（湍流亮的地方丝更明显）—— 破坏"一圈光板"。
        let fiber = 0.5 + 0.5 * (phase * 5.0 + upstream * 9.0).sin();
        let ripple = 0.085 * fiber * (0.35 + 0.65 * upstream);
        // ③ 天气：沿经度分几段"带被洗淡"（`washed ∈ [0.65, 1]`）—— 同一颗球上不是每条带一样清楚。
        let longitude = direction[2].atan2(direction[0]);
        let weather = 0.5 + 0.5 * (longitude * 2.0 + upstream * 4.0).sin();
        let washed = 1.0 - 0.35 * (1.0 - weather);
        // ④ 按纬度的淡出：`fade ∈ [0,1]`，取 0 的那些纬度**整片没有带**（剩下一点基底起伏）。
        let fade = 0.5 + 0.5 * (latitude * 2.7 + upstream * 2.2).sin();
        // ④b **纬向能量**（同一条用户口径）：带的对比在低纬最强，往高纬淡下去 ——
        //   极区剩下的是**平滑的宽带**，不是又一种细碎的图案。`0.30` = 极区的对比只剩三成。
        let zonal = 1.0 - 0.45 * (latitude * latitude);
        let strength = (0.55 + 0.45 * fade) * zonal;

        // ⑤ **对比度分层**（"有的带深、有的几乎看不见"）：`gain` 按一条**低频、与相位不同源**的
        //    曲线走 ⇒ 相邻的带深浅不一样。⚠ 用同一个相位去调对比会把深浅锁在位置上（那又变回单一层次）。
        let rank = 0.5 + 0.5 * (latitude * 2.6 - upstream * 3.1).sin();
        let local_gain = params.gain * (0.30 + 0.70 * rank);

        // 对比：以 0.5 为轴把落点推开（`gain = 0` ⇒ 不动）。
        let band = 0.5 + (wave - 0.5) * washed * strength + ripple;
        let contrasted = 0.5 + (band - 0.5) * (1.0 + local_gain) + params.bias;
        contrasted.clamp(0.0, 1.0)
    }
}
