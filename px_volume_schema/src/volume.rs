use px_protocol::art::{CUBE_FACES, VolumeData, cube_direction};

pub const PATCHES: u32 = CUBE_FACES;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shell {
    pub inner: f32,
    pub outer: f32,
}

impl Shell {
    pub fn new(inner: f32, outer: f32) -> Self {
        Self { inner, outer }
    }

    pub fn radius_of(&self, u: f32) -> f32 {
        let u = u.clamp(0.0, 1.0);
        if self.inner <= 0.0 {
            return self.inner + (self.outer - self.inner) * u;
        }
        self.inner * (self.outer / self.inner).powf(u)
    }

    pub fn altitude_of(&self, radius: f32) -> f32 {
        if self.inner <= 0.0 {
            let span = self.outer - self.inner;
            return if span.abs() <= f32::EPSILON {
                0.0
            } else {
                ((radius - self.inner) / span).clamp(0.0, 1.0)
            };
        }
        let ratio = (self.outer / self.inner).ln();
        if ratio.abs() <= f32::EPSILON {
            return 0.0;
        }
        ((radius.max(f32::MIN_POSITIVE) / self.inner).ln() / ratio).clamp(0.0, 1.0)
    }

    pub fn stretch_of(&self, u: f32) -> f32 {
        if self.inner <= 0.0 {
            return self.outer - self.inner;
        }
        self.radius_of(u) * (self.outer / self.inner).ln()
    }
}

pub fn direction_of(patch: u32, u: f32, v: f32) -> [f32; 3] {
    cube_direction(patch, u.clamp(0.0, 1.0), v.clamp(0.0, 1.0))
}

pub fn point_of(patch: u32, at: [f32; 3], inner: f32, outer: f32) -> [f32; 3] {
    let direction = direction_of(patch, at[0], at[1]);
    let radius = Shell::new(inner, outer).radius_of(at[2]);
    [
        direction[0] * radius,
        direction[1] * radius,
        direction[2] * radius,
    ]
}

pub trait VolumeSampler {
    fn sample(&self, patch: u32, at: [f32; 3]) -> f32;

    fn point(&self, patch: u32, at: [f32; 3]) -> [f32; 3];

    fn parameters(&self, point: [f32; 3]) -> (u32, [f32; 3]);

    fn patches(&self) -> u32 {
        PATCHES
    }
}

pub struct VolumeGrid<'a> {
    volume: &'a VolumeData,
}

impl<'a> VolumeGrid<'a> {
    pub fn new(volume: &'a VolumeData) -> Self {
        Self { volume }
    }

    fn cell(value: f32, last: u32) -> (u32, u32, f32) {
        let scaled = value * last as f32;
        let low = (scaled.floor() as u32).min(last.saturating_sub(1));
        (low, low + 1, scaled - low as f32)
    }
}

impl VolumeSampler for VolumeGrid<'_> {
    fn sample(&self, patch: u32, at: [f32; 3]) -> f32 {
        let volume = self.volume;
        let last_s = volume.res.max(2) - 1;
        let last_t = volume.res.max(2) - 1;
        let last_a = volume.layers.max(2) - 1;
        let (s0, s1, ts) = Self::cell(at[0].clamp(0.0, 1.0), last_s);
        let (t0, t1, tt) = Self::cell(at[1].clamp(0.0, 1.0), last_t);
        let (a0, a1, ta) = Self::cell(at[2].clamp(0.0, 1.0), last_a);
        let mut value = 0.0_f32;
        for (s, ws) in [(s0, 1.0 - ts), (s1, ts)] {
            for (t, wt) in [(t0, 1.0 - tt), (t1, tt)] {
                for (layer, wa) in [(a0, 1.0 - ta), (a1, ta)] {
                    value += ws * wt * wa * volume.at(patch % PATCHES, layer, t, s);
                }
            }
        }
        value
    }

    fn point(&self, patch: u32, at: [f32; 3]) -> [f32; 3] {
        point_of(patch, at, self.volume.inner, self.volume.outer)
    }

    fn parameters(&self, point: [f32; 3]) -> (u32, [f32; 3]) {
        let (face, u, v) = px_protocol::art::cube_face_of(point);
        let radius = (point[0] * point[0] + point[1] * point[1] + point[2] * point[2]).sqrt();
        let altitude = Shell::new(self.volume.inner, self.volume.outer).altitude_of(radius);
        (face, [u, v, altitude])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_radial_law_round_trips() {
        let shell = Shell::new(1.0, 3.0);
        for step in 0..=100 {
            let u = step as f32 / 100.0;
            let radius = shell.radius_of(u);
            let back = shell.altitude_of(radius);
            assert!(
                (back - u).abs() < 1e-5,
                "u = {u} → r = {radius} → u' = {back}"
            );
        }
        assert!((shell.radius_of(0.0) - 1.0).abs() < 1e-6);
        assert!((shell.radius_of(1.0) - 3.0).abs() < 1e-6);
    }

    #[test]
    fn the_cells_are_cubes_at_every_radius() {
        let (inner, outer) = (1.0_f32, 3.0_f32);
        let shell = Shell::new(inner, outer);
        let res = 64_u32;
        let angular = |r: f32| r * 2.0 / res as f32;
        let layers = (1.0 / ((2.0 / res as f32) / (outer / inner).ln())).round() as u32;
        let du = 1.0 / (layers - 1) as f32;
        for step in [0_u32, layers / 2, layers - 1] {
            let u = step as f32 * du;
            let r = shell.radius_of(u);
            let radial = shell.stretch_of(u) * du;
            let across = angular(r);
            let ratio = radial / across;
            assert!(
                (ratio - 1.0).abs() < 0.15,
                "r = {r:.3}：径向 {radial:.4} 对角向 {across:.4} = {ratio:.3}（该是 1）"
            );
        }
    }

    #[test]
    fn the_geometric_ladder_needs_fewer_layers() {
        let (inner, outer) = (1.0_f32, 3.0_f32);
        let res = 64.0_f32;
        let du = (2.0 / res) / (outer / inner).ln();
        let geometric = 1.0 / du;
        let linear = (outer - inner) / (inner * 2.0 / res);
        assert!(
            geometric < linear * 0.7,
            "等比 {geometric:.1} 层 vs 线性 {linear:.1} 层"
        );
    }
}
