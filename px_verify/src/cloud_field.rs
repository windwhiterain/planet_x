use crate::dual::{Dual, Dual64, first_derivative};
use crate::noise::{FbmSettings, Scalar, fbm_3};

#[derive(Clone, Copy, Debug)]
pub struct CloudFieldParams {
    pub orientation: [f32; 4],
    pub inner: f32,
    pub outer: f32,
    pub coverage: f32,
    pub base: f32,
    pub top: f32,
    pub detail_scale: f32,
    pub detail_strength: f32,
    pub erode: f32,
    pub taper: f32,
    pub coverage_gain: f32,
    pub seed: u32,
}

#[derive(Clone, Copy)]
pub struct Medium<S> {
    pub direction: [S; 3],
    pub altitude: S,
    pub radius: S,
}

fn cross<S: Scalar>(left: [S; 3], right: [S; 3]) -> [S; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn length<S: Scalar>(vector: [S; 3]) -> S {
    (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt()
}

fn mix<S: Scalar>(low: S, high: S, blend: S) -> S {
    low + (high - low) * blend
}

fn smoothstep<S: Scalar>(low: S, high: S, value: S) -> S {
    let t = ((value - low) / (high - low)).clamp01();
    t * t * (S::from_f32(3.0) - t - t)
}

fn rotate<S: Scalar>(quaternion: [f32; 4], vector: [S; 3]) -> [S; 3] {
    let axis = [
        S::from_f32(quaternion[0]),
        S::from_f32(quaternion[1]),
        S::from_f32(quaternion[2]),
    ];
    let scalar = S::from_f32(quaternion[3]);
    let first = cross(axis, vector);
    let second = cross(axis, first);
    let twice = S::from_f32(2.0);
    [
        vector[0] + (scalar * first[0] + second[0]) * twice,
        vector[1] + (scalar * first[1] + second[1]) * twice,
        vector[2] + (scalar * first[2] + second[2]) * twice,
    ]
}

impl CloudFieldParams {
    pub fn span(&self) -> f32 {
        f32::max(self.outer - self.inner, 1e-5)
    }

    pub fn to_local<S: Scalar>(&self, point: [S; 3]) -> [S; 3] {
        rotate(
            [
                -self.orientation[0],
                -self.orientation[1],
                -self.orientation[2],
                self.orientation[3],
            ],
            point,
        )
    }

    pub fn to_world<S: Scalar>(&self, vector: [S; 3]) -> [S; 3] {
        rotate(self.orientation, vector)
    }

    pub fn medium_of<S: Scalar>(&self, point: [S; 3]) -> Medium<S> {
        let local = self.to_local(point);
        let radius = length(local);
        let safe = radius.max_with(S::from_f32(1e-5));
        let altitude = (radius - S::from_f32(self.inner)) / S::from_f32(self.span());
        Medium {
            direction: [local[0] / safe, local[1] / safe, local[2] / safe],
            altitude,
            radius,
        }
    }

    pub fn cover_from_mask(&self, mask: f32) -> f32 {
        let normalized =
            ((mask - self.coverage) / f32::max(1.0 - self.coverage, 1e-4)).clamp(0.0, 1.0);
        smoothstep(0.0_f32, 0.45, normalized)
    }

    pub fn sampled_noise<S: Scalar>(
        &self,
        direction: [S; 3],
        altitude: S,
        across: f32,
        octaves: u32,
        seed: u32,
    ) -> S {
        let along = across * self.span();
        let scale = S::from_f32(across) + altitude * S::from_f32(along);
        let settings = FbmSettings {
            frequency: 1.0,
            octaves,
            lacunarity: 2.0,
            gain: 0.5,
            seed,
        };
        fbm_3(
            [
                direction[0] * scale,
                direction[1] * scale,
                direction[2] * scale,
            ],
            &settings,
        )
    }

    pub fn billows<S: Scalar>(&self, direction: [S; 3], altitude: S) -> S {
        let tower = self.sampled_noise(direction, altitude, self.detail_scale * 0.35, 3, self.seed);
        let skin = self.sampled_noise(
            direction,
            altitude,
            self.detail_scale * 1.70,
            2,
            self.seed ^ 31,
        );
        (tower * S::from_f32(0.62) + skin * S::from_f32(0.38)).clamp01()
    }

    pub fn stages<S: Scalar>(&self, cover: S, altitude: S, noise: S) -> [S; 6] {
        let height = altitude.clamp01();
        let taper = S::from_f32(self.taper);
        let footprint = (cover - taper * height * height).max_with(S::zero());
        let bias = footprint + noise - S::one();
        let lobed = (bias * S::from_f32(self.coverage_gain)).clamp01();
        let floor_here = smoothstep(
            S::zero(),
            S::from_f32(f32::max(self.base, 1e-3)),
            altitude,
        );
        let ceiling = (S::from_f32(self.top)
            * mix(S::from_f32(1.0 - self.detail_strength), S::one(), noise))
        .max_with(S::from_f32(self.base + 0.02));
        let under_top = S::one() - smoothstep(ceiling, ceiling + S::from_f32(0.20), altitude);
        let raw = (floor_here * under_top * lobed).clamp01();
        let erode = S::from_f32(self.erode);
        let shape = ((raw - erode) / S::from_f32(f32::max(1.0 - self.erode, 1e-4))).clamp01();
        [footprint, lobed, floor_here, ceiling, under_top, shape]
    }

    pub fn shape<S: Scalar>(&self, cover: S, altitude: S, noise: S) -> S {
        self.stages(cover, altitude, noise)[5]
    }

    pub fn density<S: Scalar>(&self, point: [S; 3], cover: S) -> S {
        let medium = self.medium_of(point);
        if medium.altitude.real() < 0.0 || medium.altitude.real() > 1.0 {
            return S::zero();
        }
        if cover.real() <= 0.0 {
            return S::zero();
        }
        let noise = self.billows(medium.direction, medium.altitude);
        self.shape(cover, medium.altitude, noise)
    }

    pub fn gradient(&self, point: [f64; 3], cover: f64) -> [f64; 3] {
        let held = Dual::constant(cover);
        let mut out = [0.0_f64; 3];
        for axis in 0..3 {
            let lifted = [
                Dual::constant(point[0]),
                Dual::constant(point[1]),
                Dual::constant(point[2]),
            ];
            let (_, slope) = first_derivative(
                |seed: Dual64| {
                    let mut seeded = lifted;
                    seeded[axis] = Dual(seed);
                    self.density(seeded, held).0
                },
                point[axis],
            );
            out[axis] = slope;
        }
        out
    }
}
