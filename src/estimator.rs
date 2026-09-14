#[cfg(test)]
mod tests;

pub trait Estimator {
    fn update(&mut self, x: f32, y: f32);
    fn get(&self, x: f32) -> f32;
}

pub struct Linear {
    a: f32,
    b: f32,
}

impl Estimator for Linear {
    fn update(&mut self, _x: f32, _y: f32) {
        todo!()
    }

    fn get(&self, _x: f32) -> f32 {
        todo!()
    }
}

pub struct PowerLaw {
    slope: f32,
    intercept: f32,
    covariance: [[f32; 2]; 2],
    forgetting: f32,
    fixed_slope: bool,
}

impl PowerLaw {
    pub const DEFAULT_FORGETTING: f32 = 0.95;
    /// 数值边界，见 [`crate::utils::LOG_LIMIT`]——不是"价格倍数不该超出 1e±3"
    pub const LOG_LIMIT: f32 = crate::utils::LOG_LIMIT;
    pub const INITIAL_COVARIANCE: f32 = 1.0;
    /// 协方差上限：**数值护栏**，不是"参数不该动得这么快"那种策略边界。
    ///
    /// 遗忘最小二乘在"输入几乎不变"时没有激励方向，协方差按 `P ← P/λ` 每轮涨 `1/λ`。
    /// 实测（常数特征）：λ=0.95 时 200 轮 2.9e4、600 轮 2.3e13、1000 轮 1.9e22；
    /// λ=0.99 在 1000 轮还只有 2.3e4；λ=0.999 只有 2.7。P 一旦涨过 f32 的分辨能力，
    /// 任何一次真实的输入变化都会让 `gain ≈ 1`、把斜率参数打飞几十个数量级——§19.2
    /// 那 10–45 个数量级的振铃就由此进入价格曲线。
    ///
    /// 夹住 P 等价于把更新退化成**有界的归一化梯度**（`|Δ参数| ≲ |残差| / |特征|`）：
    /// 学得快的能力保留，无界增益去掉。取 1e3 是量出来的，见
    /// `estimator/tests.rs::a_forgotten_least_squares_does_not_wind_up`。
    pub const COVARIANCE_LIMIT: f32 = 1e3;

    pub fn new(slope: f32, intercept: f32, forgetting: f32) -> Self {
        let covariance = Self::INITIAL_COVARIANCE;
        Self {
            slope,
            intercept,
            covariance: [[covariance, 0.0], [0.0, covariance]],
            forgetting,
            fixed_slope: false,
        }
    }

    /// 固定阶数：只学水平，不学指数
    pub fn with_fixed_slope(mut self) -> Self {
        self.fixed_slope = true;
        self
    }

    pub fn slope(&self) -> f32 {
        self.slope
    }

    pub fn intercept(&self) -> f32 {
        self.intercept
    }

    pub fn is_fixed_slope(&self) -> bool {
        self.fixed_slope
    }

    pub fn covariance(&self) -> [[f32; 2]; 2] {
        self.covariance
    }

    fn features(x: f32) -> [f32; 2] {
        [x.ln(), 1.0]
    }

    /// 把协方差整体缩回 `max|P_ij| ≤ COVARIANCE_LIMIT`；非有限值直接归零。
    ///
    /// 用**整体缩放**而不是逐元素夹：逐元素会把正定矩阵夹成不定的，那样
    /// `λ + fᵀPf` 可能变负，更新会被静默跳过（学习停摆）。整体缩放保持正定与对称。
    fn clamp_covariance(&mut self) {
        let mut worst = 0.0f32;
        for row in &self.covariance {
            for value in row {
                if !value.is_finite() {
                    self.covariance = [[0.0; 2]; 2];
                    return;
                }
                worst = worst.max(value.abs());
            }
        }
        let limit = Self::COVARIANCE_LIMIT;
        if worst > limit {
            let scale = limit / worst;
            for i in 0..2 {
                for j in 0..2 {
                    self.covariance[i][j] *= scale;
                }
            }
        }
    }
}

impl Estimator for PowerLaw {
    fn update(&mut self, x: f32, y: f32) {
        if !x.is_finite() || x <= 0.0 || !y.is_finite() || y <= 0.0 {
            return;
        }
        if self.fixed_slope {
            let observed = y.ln() - self.slope * x.ln();
            if !observed.is_finite() {
                return;
            }
            let covariance = self.covariance[1][1];
            let denominator = self.forgetting + covariance;
            if !denominator.is_finite() || denominator <= 0.0 {
                return;
            }
            let gain = covariance / denominator;
            self.intercept += gain * (observed - self.intercept);
            self.covariance[1][1] = ((covariance - gain * covariance) / self.forgetting)
                .clamp(0.0, Self::COVARIANCE_LIMIT);
            return;
        }
        let feature = Self::features(x);
        let covariance = self.covariance;
        let projection = [
            covariance[0][0] * feature[0] + covariance[0][1] * feature[1],
            covariance[1][0] * feature[0] + covariance[1][1] * feature[1],
        ];
        let denominator = self.forgetting + feature[0] * projection[0] + feature[1] * projection[1];
        if !denominator.is_finite() || denominator <= 0.0 {
            return;
        }
        let gain = [projection[0] / denominator, projection[1] / denominator];

        let residual = y.ln() - (self.slope * feature[0] + self.intercept * feature[1]);
        self.slope += gain[0] * residual;
        self.intercept += gain[1] * residual;

        let mut updated = [[0.0; 2]; 2];
        for i in 0..2 {
            for j in 0..2 {
                updated[i][j] = (covariance[i][j] - gain[i] * projection[j]) / self.forgetting;
            }
        }
        self.covariance = updated;
        self.clamp_covariance();
    }

    fn get(&self, x: f32) -> f32 {
        if !x.is_finite() || x <= 0.0 {
            return 0.0;
        }
        let log_scale = self.slope * x.ln() + self.intercept;
        if log_scale.is_nan() {
            return 0.0;
        }
        log_scale.clamp(-Self::LOG_LIMIT, Self::LOG_LIMIT).exp()
    }
}