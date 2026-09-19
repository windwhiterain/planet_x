//! 立方球参数空间：代理 mesh 的烘法就住在这里。
//!
//! 世界空间包围盒烘一遍会把约 93% 的采样丢在壳外，所以网格铺在**立方球参数空间**里：
//! 6 个面 × 面内 `[0,1]²` × 径向高度 `[0,1]`。
//!
//! ⚠ 面号是**显式**参数，不折进坐标里。曾经试过把域拼成 `[0, 6) × [0,1]²`（`at[0]` 的
//! 整数部分是面号）：等值面网格在每个面 `u = 1` 那一层会取到**下一个面** `u = 0` 的方向，
//! 而 `cube_direction(0, 1, v)` 与 `cube_direction(1, 0, v)` 不是同一个方向（是它对面的棱）
//! ⇒ 网格上多出一层错位的顶点，实测 887 条开口边、950 条非流形边、68% 的三角形退化成
//! 零面积。面号单独传就没有这个歧义。
//!
//! 相邻两面在共用的那条棱上给出**逐位相同**的方向（`cube_direction` 的公式在棱上重合），
//! 这是等值面算子能把 6 块焊成一张闭合网格的依据。

use px_protocol::art::{CUBE_FACES, VolumeData, cube_direction};

/// 立方球的面数。
pub const PATCHES: u32 = CUBE_FACES;

/// 面内参数 `(u, v)` → 单位方向。
pub fn direction_of(patch: u32, u: f32, v: f32) -> [f32; 3] {
    cube_direction(patch, u.clamp(0.0, 1.0), v.clamp(0.0, 1.0))
}

/// 面内参数 `(u, v, 高度)` → 世界点：高度 0 落在 `inner`、高度 1 落在 `outer`。
pub fn point_of(patch: u32, at: [f32; 3], inner: f32, outer: f32) -> [f32; 3] {
    let direction = direction_of(patch, at[0], at[1]);
    let radius = inner + (outer - inner) * at[2].clamp(0.0, 1.0);
    [
        direction[0] * radius,
        direction[1] * radius,
        direction[2] * radius,
    ]
}

/// 参数空间里的一个标量场：`patch` 是面号（0..PATCHES），`at` 是面内 `(u, v, 径向高度)`。
pub trait VolumeSampler {
    fn sample(&self, patch: u32, at: [f32; 3]) -> f32;

    /// 等值面顶点落在世界里的哪里。与 `sample` 必须用同一份参数→世界映射，
    /// 否则「面上的值」和「顶点的位置」说的是两件事。
    fn point(&self, patch: u32, at: [f32; 3]) -> [f32; 3];

    /// `point` 的逆：世界点落在哪个面的哪个参数上（法线要按世界点回采样时用得上）。
    fn parameters(&self, point: [f32; 3]) -> (u32, [f32; 3]);

    fn patches(&self) -> u32 {
        PATCHES
    }
}

/// 烘好的体积网格上的三线性采样（面内双线性 + 径向线性）。
///
/// 只在**同一个面内**插值：调用方（等值面算子）每个面各自跑一次，`u/v` 不会越出 [0,1]。
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
        let span = self.volume.outer - self.volume.inner;
        let altitude = if span.abs() <= f32::EPSILON {
            0.0
        } else {
            ((radius - self.volume.inner) / span).clamp(0.0, 1.0)
        };
        (face, [u, v, altitude])
    }
}
