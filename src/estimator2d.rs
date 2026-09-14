#[cfg(test)]
mod tests;

pub trait Estimator2D {
    fn update(&mut self, x: f32, y: f32, z: f32);
    fn get(&self, x: f32, y: f32) -> f32;
}

/// 一个交易者面对的**兑现率曲面**：`兑现率 = share(a) / (1 + 申报量 / depth(a))`。
///
/// ## 为什么两个响应都必须饱和
///
/// 旧版把 `share` 与 `depth` 都写成力度 `a` 的**无界幂律**，于是
/// `share·depth`（也就是"我最多能卖掉多少"这个需求天花板）**随力度无界增长**，
/// 模型因此相信"只要我够狠，申报多少就能卖掉多少"。
///
/// 这个信念会撞墙：市场对面就那么多量，价格买不动。而观测
/// "无论我怎么报价都只有 113" **落在旧模型类之外**——梯度只能把它解释成
/// "我还不够狠"，于是水平一路下调、要价一路下坠（实测一产在 0.63 的买价下
/// 报到 0.002），而且永远追不上，因为那个量价格根本买不到。
///
/// 现在两个响应都是 **Hill 形状**（饱和幂律）：
///
/// ```text
/// share(a) = M · a^p / (M + a^p)      ·      depth(a) 同形
/// 需求天花板 = M_share × M_depth      ← 有限、可学
/// ```
///
/// 被配给之后，模型学到的会是"**我的天花板是 113**"——这是它**自己从样本里估出来的
/// 隐式市场规模**，不需要任何人替它划一个 sub market；然后收入对报价变成单调递增，
/// 它会停止压价、回头向买价靠。
///
/// 形状同时保证 `a ≪ M` 时 `share ≈ a`，也就是**和旧先验逐点相同**：不引入新的
/// 局部假设，只是把远处那条本来就不该存在的无界尾巴按住了。
pub struct Response {
    log_share_max: f32,
    share_power: f32,
    log_share_half: f32,
    log_depth_max: f32,
    depth_power: f32,
    log_depth_half: f32,
    noise: f32,
    forgetting: f32,
}

impl Response {
    pub const DEFAULT_FORGETTING: f32 = 0.95;
    /// 数值边界，见 [`crate::utils::LOG_LIMIT`]——不是"预测不该超出 1e±3"这种策略选择
    pub const LOG_LIMIT: f32 = crate::utils::LOG_LIMIT;
    /// 噪声下界只能由"f32 分不出更小的相对差"给出，不是"噪声不该小于 0.05"
    pub const MIN_NOISE: f32 = f32::EPSILON;
    pub const NORMALIZATION: f32 = 1e-3;
    pub const RATIO_FLOOR: f32 = 1e-3;
    /// 先验饱和水平。
    ///
    /// ⚠️ **这个常数目前是承重的，而且是被凑出来的。** Hill 形状在 `a = 1` 处给出
    /// `share = M/(M+1)`、天花板 `M²`：取 `M = 4` 时天花板先验是 16。
    /// 实测这个值决定了整件事的成败——
    ///
    /// | `M` | 天花板先验 | 一产卖价 | 结果 |
    /// |---|---|---|---|
    /// | 4 | 16 | 0.058，在 0.05~0.1 摆动 | 稳；价差从 130× 收到 4× |
    /// | 100 | 1e4 | **1e35** | 又炸回 f32 边界 |
    ///
    /// 也就是说：**饱和形状是必要条件，但不是充分条件**——天花板先验太松时，
    /// 学习器在把天花板压下来之前就已经把价格压垮了。
    /// "把补丁换成结构"这句话只在紧先验下成立，这一条还没解决。
    pub const INITIAL_MAX: f32 = 4.0;
    /// 先验斜率与半饱和点。`M = 4, p = 2, 半饱和 = 1` 给出 `share(1) = 2`、
    /// 天花板 16——**实测唯一稳定的那一组**。半饱和点必须是独立参数：
    /// 把它和饱和水平绑死（Hill 的 `h = M^(1/p)`）表达不出这组数，实测会退化。
    pub const INITIAL_POWER: f32 = 2.0;
    pub const INITIAL_HALF: f32 = 1.0;

    pub fn new(forgetting: f32) -> Self {
        Self {
            log_share_max: Self::INITIAL_MAX.ln(),
            share_power: Self::INITIAL_POWER,
            log_share_half: Self::INITIAL_HALF.ln(),
            log_depth_max: Self::INITIAL_MAX.ln(),
            depth_power: Self::INITIAL_POWER,
            log_depth_half: Self::INITIAL_HALF.ln(),
            noise: 0.5,
            forgetting: forgetting.clamp(0.0, 1.0),
        }
    }

    pub fn noise(&self) -> f32 {
        self.noise
    }

    /// 市场能吸收的量（本节点视角的天花板），由样本学出来
    pub fn ceiling(&self) -> f32 {
        Self::scale(self.log_share_max) * Self::scale(self.log_depth_max)
    }

    pub fn share(&self, aggressiveness: f32) -> f32 {
        self.saturating(self.log_share_max, self.share_power, self.log_share_half, aggressiveness)
    }

    pub fn depth(&self, aggressiveness: f32) -> f32 {
        self.saturating(self.log_depth_max, self.depth_power, self.log_depth_half, aggressiveness)
    }

    /// `M·σ(p·(ln a − ln h))`：`a ≪ h` 时回到无界幂律，`a ≫ h` 时饱和到 `M`
    fn saturating(&self, log_max: f32, power: f32, log_half: f32, aggressiveness: f32) -> f32 {
        let log_a = Self::log_aggressiveness(aggressiveness);
        Self::scale(log_max) * Self::logistic(power * (log_a - log_half))
    }

    fn logistic(x: f32) -> f32 {
        if x.is_nan() {
            return 0.0;
        }
        if x >= 0.0 {
            1.0 / (1.0 + (-x).exp())
        } else {
            let e = x.exp();
            e / (1.0 + e)
        }
    }

    /// 参数快照：`[ln 份额上限, 份额斜率, ln 份额半饱和, ln 深度上限, 深度斜率, ln 深度半饱和]`。
    /// `ln` 项已经在**对数尺度**上，所以算术平均就是"对数中心"。
    pub fn params(&self) -> [f32; 6] {
        [
            self.log_share_max,
            self.share_power,
            self.log_share_half,
            self.log_depth_max,
            self.depth_power,
            self.log_depth_half,
        ]
    }

    /// 把参数往 `target` 推 `gain`（0 = 不动，1 = 变成 `target`），收尾用与 `update`
    /// 相同的数值护栏。用途见 [`crate::warehouse::Warehouses::slide_silent_curves_toward_global`]。
    pub fn slide_toward(&mut self, target: [f32; 6], gain: f32) {
        if !gain.is_finite() || gain <= 0.0 {
            return;
        }
        let gain = gain.min(1.0);
        self.log_share_max += gain * (target[0] - self.log_share_max);
        self.share_power += gain * (target[1] - self.share_power);
        self.log_share_half += gain * (target[2] - self.log_share_half);
        self.log_depth_max += gain * (target[3] - self.log_depth_max);
        self.depth_power += gain * (target[4] - self.depth_power);
        self.log_depth_half += gain * (target[5] - self.log_depth_half);
        self.log_share_max = self.log_share_max.clamp(-Self::LOG_LIMIT, Self::LOG_LIMIT);
        self.log_depth_max = self.log_depth_max.clamp(-Self::LOG_LIMIT, Self::LOG_LIMIT);
        self.log_share_half = self.log_share_half.clamp(-Self::LOG_LIMIT, Self::LOG_LIMIT);
        self.log_depth_half = self.log_depth_half.clamp(-Self::LOG_LIMIT, Self::LOG_LIMIT);
        if !self.share_power.is_finite() {
            self.share_power = 0.0;
        }
        if !self.depth_power.is_finite() {
            self.depth_power = 0.0;
        }
    }

    pub fn fill_ratio(&self, volume: f32, aggressiveness: f32) -> f32 {
        self.ratio(volume, aggressiveness).min(1.0)
    }

    fn log_aggressiveness(aggressiveness: f32) -> f32 {
        if aggressiveness.is_finite() && aggressiveness > 0.0 {
            aggressiveness.ln().clamp(-Self::LOG_LIMIT, Self::LOG_LIMIT)
        } else {
            0.0
        }
    }

    fn scale(log_scale: f32) -> f32 {
        if log_scale.is_nan() {
            return 0.0;
        }
        log_scale.clamp(-Self::LOG_LIMIT, Self::LOG_LIMIT).exp()
    }

    fn ratio(&self, volume: f32, aggressiveness: f32) -> f32 {
        let share = self.share(aggressiveness);
        let depth = self.depth(aggressiveness);
        share / (1.0 + volume / depth)
    }
}

impl Estimator2D for Response {
    fn update(&mut self, x: f32, y: f32, z: f32) {
        let volume = x;
        let aggressiveness = y;
        let dealt = z;
        if !volume.is_finite()
            || volume <= 0.0
            || !aggressiveness.is_finite()
            || aggressiveness <= 0.0
            || !dealt.is_finite()
            || dealt < 0.0
        {
            return;
        }
        let ratio = (dealt / volume).clamp(0.0, 1.0);
        let log_aggressiveness = Self::log_aggressiveness(aggressiveness);
        let share = self.share(aggressiveness);
        let depth = self.depth(aggressiveness);
        // 饱和因子 g = 值 / 饱和水平。g→1 表示已经贴着天花板，
        // 此时"再加力度"对预测的推动趋于 0——这正是旧版缺的那一项。
        let g_s = share / Self::scale(self.log_share_max).max(f32::MIN_POSITIVE);
        let g_d = depth / Self::scale(self.log_depth_max).max(f32::MIN_POSITIVE);
        let rationing = volume / depth;
        let u = if rationing.is_finite() {
            let u = rationing / (1.0 + rationing);
            if u.is_finite() {
                u.clamp(0.0, 1.0)
            } else {
                1.0
            }
        } else {
            1.0
        };
        let raw = share * (1.0 - u);
        let predicted = raw.min(1.0);
        if raw >= 1.0 && ratio >= 1.0 {
            return;
        }
        let error = ratio - predicted;
        // ∂预测兑现率 / ∂参数。与旧版同构，只是水平项乘 g、斜率项乘 (1−g)：
        // 旧式 `[raw, raw·L, raw·u, raw·u·L]` 是 g = 1（远离饱和）时的特例。
        let gradient = [
            raw * g_s,
            raw * (1.0 - g_s) * (log_aggressiveness - self.log_share_half),
            -raw * (1.0 - g_s) * self.share_power,
            raw * u * g_d,
            raw * u * (1.0 - g_d) * (log_aggressiveness - self.log_depth_half),
            -raw * u * (1.0 - g_d) * self.depth_power,
        ];
        // 用 f64 累加梯度平方：`raw` 本身可以到 e^44，平方在 f32 里会溢出成 inf，
        // 而 inf 会把 gain 悄悄压成 0（= 静默停止学习）。这是数值实现，不是模型参数。
        let norm: f64 =
            gradient.iter().map(|g| (*g as f64) * (*g as f64)).sum::<f64>() + Self::NORMALIZATION as f64;
        let gain = ((1.0 - self.forgetting) as f64 * error as f64 / norm) as f32;
        self.log_share_max += gain * gradient[0];
        self.share_power += gain * gradient[1];
        self.log_share_half += gain * gradient[2];
        self.log_depth_max += gain * gradient[3];
        self.depth_power += gain * gradient[4];
        self.log_depth_half += gain * gradient[5];
        // 只有"水平"有数值边界（两个这样的量相乘还要留在 f32 里）；
        // 斜率不再有上下限——那是策略，不是数值。
        self.log_share_max = self.log_share_max.clamp(-Self::LOG_LIMIT, Self::LOG_LIMIT);
        self.log_depth_max = self.log_depth_max.clamp(-Self::LOG_LIMIT, Self::LOG_LIMIT);
        self.log_share_half = self.log_share_half.clamp(-Self::LOG_LIMIT, Self::LOG_LIMIT);
        self.log_depth_half = self.log_depth_half.clamp(-Self::LOG_LIMIT, Self::LOG_LIMIT);
        // 斜率只保证有限：非有限会永久污染后续预测（NaN + x = NaN），这是数值护栏
        if !self.share_power.is_finite() {
            self.share_power = 0.0;
        }
        if !self.depth_power.is_finite() {
            self.depth_power = 0.0;
        }
        if gradient[0].is_finite() && gradient[0] != 0.0 {
            let observed = (ratio + Self::RATIO_FLOOR).ln() - (predicted + Self::RATIO_FLOOR).ln();
            if observed.is_finite() {
                self.noise += (1.0 - self.forgetting) * (observed.abs() - self.noise);
                self.noise = self.noise.max(Self::MIN_NOISE);
            }
        }
    }

    fn get(&self, x: f32, y: f32) -> f32 {
        let volume = x;
        let aggressiveness = y;
        if !volume.is_finite()
            || volume <= 0.0
            || !aggressiveness.is_finite()
            || aggressiveness <= 0.0
        {
            return 0.0;
        }
        let dealt = volume * self.fill_ratio(volume, aggressiveness);
        if dealt.is_finite() {
            dealt.max(0.0)
        } else {
            0.0
        }
    }
}
