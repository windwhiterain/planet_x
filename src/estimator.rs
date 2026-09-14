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

    fn features(x: f32) -> [f32; 2] {
        [x.ln(), 1.0]
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
            self.covariance[1][1] = (covariance - gain * covariance) / self.forgetting;
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
