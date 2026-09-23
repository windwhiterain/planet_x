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

/// **径向律**：参数空间里线性、世界空间里等比。
///
/// ⚠⚠ 这是 2026-09-25 用户定的口径，理由是一条**各向同性**的判据：
///   角向的格子在世界里的尺寸是 `r · Δθ`（∝ r），所以径向步长也必须 ∝ r，
///   格子才在每个半径上是同一个形状。线性径向（`r = inner + span·u`）在 `r = 3`
///   处给出 `3:1` 的"饼"（角向 0.094、径向 0.031，`res = layers = 64`），
///   也就是"近处细、远处粗" —— 而用户要的是**远近均衡**。
///
/// ⚠ 为什么以 `ln` 的形式（而不是直接存一张半径表）：**采样要 O(1)**。
///   参数空间里一切线性（层号 = `floor(u · layers)`），非线性只活在
///   `world ↔ param` 这一对函数里。
///
/// ⚠ 于是"参数空间里步长固定"的射线步进落到世界里就是等比步长（∝ r），
///   与角向格子配成各向同性 —— 这条是**采样密度跟着立体角走**的全部机关。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shell {
    pub inner: f32,
    pub outer: f32,
}

impl Shell {
    pub fn new(inner: f32, outer: f32) -> Self {
        Self { inner, outer }
    }

    /// `u ∈ [0,1]` → 世界半径：`inner · (outer/inner)^u`。
    ///
    /// ⚠ `inner <= 0` 时退化成线性（比值没有定义）：这一档只出现在"壳从球心起"的
    ///   退化配置里，给一条不会炸的分支比拒绝更合适（真实配置里 `inner >= 1`）。
    pub fn radius_of(&self, u: f32) -> f32 {
        let u = u.clamp(0.0, 1.0);
        if self.inner <= 0.0 {
            return self.inner + (self.outer - self.inner) * u;
        }
        self.inner * (self.outer / self.inner).powf(u)
    }

    /// 世界半径 → `u`（[`Self::radius_of`] 的逆）。
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

    /// `dr/du` 在 `u` 处的值：一步 `Δu` 在世界里走多远（`= r · ln(outer/inner)`）。
    pub fn stretch_of(&self, u: f32) -> f32 {
        if self.inner <= 0.0 {
            return self.outer - self.inner;
        }
        self.radius_of(u) * (self.outer / self.inner).ln()
    }
}

/// 面内参数 `(u, v)` → 单位方向。
pub fn direction_of(patch: u32, u: f32, v: f32) -> [f32; 3] {
    cube_direction(patch, u.clamp(0.0, 1.0), v.clamp(0.0, 1.0))
}

/// 面内参数 `(u, v, 高度)` → 世界点：高度 0 落在 `inner`、高度 1 落在 `outer`，
/// 中间按 [`Shell`] 的等比律。
pub fn point_of(patch: u32, at: [f32; 3], inner: f32, outer: f32) -> [f32; 3] {
    let direction = direction_of(patch, at[0], at[1]);
    let radius = Shell::new(inner, outer).radius_of(at[2]);
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
        let altitude = Shell::new(self.volume.inner, self.volume.outer).altitude_of(radius);
        (face, [u, v, altitude])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **往返恒等**：`radius_of` 与 `altitude_of` 必须互逆（差一点点就是"层错位"）。
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

    /// ⚠⚠ **各向同性**（用户那条"远近均衡"的判据）：角向格子在世界的尺寸是 `r·Δθ`，
    ///   径向格子是 `Δr = stretch_of(u)·Δu`。让两者相等 ⇒ **每个半径上的格子都是立方**。
    ///
    ///   线性的旧律在 `r = 3` 处给出 `3:1` 的饼（这条判据会红）。
    #[test]
    fn the_cells_are_cubes_at_every_radius() {
        let (inner, outer) = (1.0_f32, 3.0_f32);
        let shell = Shell::new(inner, outer);
        let res = 64_u32;
        // 角向格子的世界尺寸：面心附近 `r · 2/res`（`cube_direction` 在面心处
        // 每 1/res 的参数走 `2/res` 的世界距离 —— 那是立方图面内最小的那一段）。
        let angular = |r: f32| r * 2.0 / res as f32;
        // 取"让每层世界步长 == 角向尺寸"的层数：`Δu = Δr / stretch`，而 `Δr = r·2/res`
        // ⇒ `Δu = (2/res) / ln(outer/inner)` ⇒ `layers = 1/Δu`。
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

    /// **层数与"等比"的账**：同样的角分辨率下，等比径向比线性径向**省层**。
    ///
    /// ⚠ 这不是美观问题：线性的层数得按**最细的那一端**（`inner`）配，于是外面那一半
    ///   半径被白白细分（`res = 64` 时 64 层 vs 等比 36 层）。
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
