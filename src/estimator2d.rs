#[cfg(test)]
mod tests;

pub trait Estimator2D {
    fn update(&mut self, x: f32, y: f32, z: f32);
    fn get(&self, x: f32, y: f32) -> f32;
}

pub struct Response {
    log_share: f32,
    share_power: f32,
    log_depth: f32,
    depth_power: f32,
    noise: f32,
    forgetting: f32,
}

impl Response {
    pub const DEFAULT_FORGETTING: f32 = 0.95;
    pub const MIN_LOG_SCALE: f32 = -6.907_755_4;
    pub const MAX_LOG_SCALE: f32 = 6.907_755_4;
    pub const MIN_POWER: f32 = 0.0;
    pub const MAX_POWER: f32 = 8.0;
    pub const MIN_NOISE: f32 = 0.05;
    pub const MAX_NOISE: f32 = 3.0;
    pub const NORMALIZATION: f32 = 1e-3;
    pub const RATIO_FLOOR: f32 = 1e-3;

    pub fn new(forgetting: f32) -> Self {
        Self {
            log_share: 0.0,
            share_power: 1.0,
            log_depth: 0.0,
            depth_power: 1.0,
            noise: 0.5,
            forgetting: forgetting.clamp(0.0, 1.0),
        }
    }

    pub fn noise(&self) -> f32 {
        self.noise
    }

    pub fn share(&self, aggressiveness: f32) -> f32 {
        Self::scale(self.log_share + self.share_power * Self::log_aggressiveness(aggressiveness))
    }

    pub fn depth(&self, aggressiveness: f32) -> f32 {
        Self::scale(self.log_depth + self.depth_power * Self::log_aggressiveness(aggressiveness))
    }

    pub fn fill_ratio(&self, volume: f32, aggressiveness: f32) -> f32 {
        self.ratio(volume, aggressiveness).min(1.0)
    }

    fn log_aggressiveness(aggressiveness: f32) -> f32 {
        if aggressiveness.is_finite() && aggressiveness > 0.0 {
            aggressiveness.ln().clamp(Self::MIN_LOG_SCALE, Self::MAX_LOG_SCALE)
        } else {
            0.0
        }
    }

    fn scale(log_scale: f32) -> f32 {
        if log_scale.is_nan() {
            return 0.0;
        }
        log_scale
            .clamp(Self::MIN_LOG_SCALE, Self::MAX_LOG_SCALE)
            .exp()
    }

    fn ratio(&self, volume: f32, aggressiveness: f32) -> f32 {
        let log_aggressiveness = Self::log_aggressiveness(aggressiveness);
        let share = Self::scale(self.log_share + self.share_power * log_aggressiveness);
        let depth = Self::scale(self.log_depth + self.depth_power * log_aggressiveness);
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
        let share = Self::scale(self.log_share + self.share_power * log_aggressiveness);
        let depth = Self::scale(self.log_depth + self.depth_power * log_aggressiveness);
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
        let gradient = [
            raw,
            raw * log_aggressiveness,
            raw * u,
            raw * u * log_aggressiveness,
        ];
        let norm: f32 = gradient.iter().map(|g| g * g).sum::<f32>() + Self::NORMALIZATION;
        let gain = (1.0 - self.forgetting) * error / norm;
        self.log_share += gain * gradient[0];
        self.share_power += gain * gradient[1];
        self.log_depth += gain * gradient[2];
        self.depth_power += gain * gradient[3];
        self.log_share = self.log_share.clamp(Self::MIN_LOG_SCALE, Self::MAX_LOG_SCALE);
        self.log_depth = self.log_depth.clamp(Self::MIN_LOG_SCALE, Self::MAX_LOG_SCALE);
        self.share_power = self.share_power.clamp(Self::MIN_POWER, Self::MAX_POWER);
        self.depth_power = self.depth_power.clamp(Self::MIN_POWER, Self::MAX_POWER);
        if gradient[0].is_finite() && gradient[0] != 0.0 {
            let observed = (ratio + Self::RATIO_FLOOR).ln() - (predicted + Self::RATIO_FLOOR).ln();
            if observed.is_finite() {
                self.noise += (1.0 - self.forgetting) * (observed.abs() - self.noise);
                self.noise = self.noise.clamp(Self::MIN_NOISE, Self::MAX_NOISE);
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
