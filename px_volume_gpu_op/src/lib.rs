//! **体网格在 GPU 上的摊平下标**：一处公式、两侧共用。
//!
//! 布局与 `VolumeData::data` 逐字相同（`px_protocol::art::VolumeData`）：
//!
//! ```text
//!   data[((((face * layers + layer) * res + t) * res + s) * LANES) + lane]
//! ```
//!
//! ⚠⚠ 为什么这一层要单独存在、还要配 GPU↔CPU 判据：天空立方贴图那道**竖缝**的成因，
//!   就是两处"从世界点反查格点"的实现里有一处把八个角**困在本面内**（`wrap_cell` +
//!   面号写死）。搬到 WGSL 之后，"同一套下标"这件事又要在两种语言里各写一遍 ——
//!   这里把它压成一条判据：GPU 按 WGSL 的公式重排一遍数组，Rust 按本文件的公式算期望，
//!   逐元素必须相等。**先钉死下标，再谈光线步进。**

//! # 移植规格：CPU 的 sample_at → WGSL（逐字对齐，别自由发挥）
//!
//! 这一段是把 px_volume_alg::raymarch::sample_at 的**语义**原样记下来（真源在那边，
//! 改那边必须改这里）。WGSL 侧必须**逐步照做**，否则会重现那道竖缝或引入新边。
//!
//! `	ext
//! 输入：世界点 p；体积 (res, layers, inner, outer)
//!  1. r = |p|；span = outer - inner
//!     容差 tolerance = |span|.max(1) * 1e-5
//!     若 r < inner - tol 或 r > outer + tol 或 |span| <= eps ⇒ 壳外（读出 0）
//!  2. dir = p / r  ⇒  (face, s, t) = cube_face_of(dir)          // 见下
//!  3. altitude = clamp((r - inner) / span, 0, 1)
//!     sz = altitude * (layers - 1)
//!     layer0 = (|sz - round(sz)| < 1e-3) ? round(sz) : floor(sz)   // ⚠ 1e-3 的贴齐
//!     tz = snap(sz - layer0)
//!     la = clamp(layer0, 0, layers-1)；lb = clamp(layer0 + 1, 0, layers-1)
//!  4. sx = s * res - 0.5；sy = t * res - 0.5
//!     x0 = floor(sx)；y0 = floor(sy)；tx = snap(sx - x0)；ty = snap(sy - y0)
//!     snap(f) = f < 1e-4 ? 0 : (f > 1-1e-4 ? 1 : f)
//!  5. ⚠⚠ **八个角逐个跨面反查**（不能在本面里 wrap/clamp）：
//!       corner_slot(cs, ct, layer):
//!         s = (cs + 0.5) / res；t = (ct + 0.5) / res
//!         (nf, ns, nt) = cube_face_of(cube_direction(face, s, t))
//!         cs' = min(u32(ns * res), res-1)；ct' = min(u32(nt * res), res-1)
//!         slot = (((nf * layers + min(layer, layers-1)) * res + ct') * res + cs') * 6
//!       八个角 = {(x0|y0|la), (x0+1|y0|la), (x0|y0+1|la), (x0+1|y0+1|la),
//!                 (x0|y0|lb), (x0+1|y0|lb), (x0|y0+1|lb), (x0+1|y0+1|lb)}
//!  6. 三线性权重：(wx, wy) = (1 - tx, 1 - ty)；wz、以及 8 个权重按 x/y/z 组合
//!     gather(lane) = Σ w[i] * data[corners[i] + lane]
//! `
//!
//! 两个几何核（px_protocol::art，真源在那边）：
//!
//! `	ext
//! cube_direction(face, s, t): a = 2s-1, b = 2t-1
//!   face 0 => ( 1, -b, -a) | 1 => (-1, -b,  a) | 2 => (a,  1,  b)
//!   face 3 => ( a, -1, -b) | 4 => ( a, -b,  1) | 5 => (-a, -b, -1)
//!   再归一化（长度为 0 时返回 (0,1,0)）
//!
//! cube_face_of(d): (ax, ay, az) = (|x|, |y|, |z|)
//!   face = (ax >= ay && ax >= az) ? (x > 0 ? 0 : 1)
//!        : (ay >= az)            ? (y > 0 ? 2 : 3)
//!        :                         (z > 0 ? 4 : 5)
//!   major = (face in {0,1}) ? ax : (face in {2,3}) ? ay : az；再 max(eps)
//!   (a, b) = face 0 => (-z,-y) | 1 => (z,-y) | 2 => (x, z)
//!          | face 3 => ( x,-z) | 4 => (x,-y) | 5 => (-x,-y)
//!   s = (a/major)*0.5 + 0.5；t = (b/major)*0.5 + 0.5；两者 clamp 到 [0,1]
//! `

/// **跨面三线性采样核**（WGSL 源与 crate 同住：src/sampler.wgsl）。
///
/// 语义与 px_volume_alg::raymarch::sample_at 逐条对齐（规格见本文件的"移植规格"一节）。
/// ⚠ 它现在只是"随 crate 一起装运的源"：**接上判据那一刻**才会被 GPU 真正编译，
///   所以在那之前 cargo test 不会替它把关 —— 下一轮的第一件事就是给它配
///   GPU↔CPU 逐点等价判据（多方向 + 棱上取点）。
pub const SAMPLER_WGSL: &str = include_str!("sampler.wgsl");

use px_gpu::{Binding, connect, dispatch};

/// 每个体素几条通道：`[发射 R, G, B, σ_R, σ_G, σ_B]`。
pub const LANES: usize = 6;

/// 体素的**摊平下标**（世界点 → 数据那一格）。
///
/// ⚠ 参数顺序与 WGSL 侧一致（`face, layer, t, s, lane`）：两侧签名不一样时，
///   "哪一个是 s、哪一个是 t"这种错会在画面上只表现为"云位置不对"，归因极远。
pub fn flat_index(
    res: u32,
    layers: u32,
    face: u32,
    layer: u32,
    t: u32,
    s: u32,
    lane: u32,
) -> usize {
    (((((face * layers + layer) * res + t) * res + s) * LANES as u32) + lane) as usize
}

/// **重排核**：`dst[i] = src[flat_index_of(i)]`，其中 `i` 按
/// `(face, layer, t, s, lane)` 的字典序遍历。GPU 侧用 WGSL 里的同一套公式算。
pub const REPACK_WGSL: &str = r#"
struct Shape {
    res: u32,
    layers: u32,
    lanes: u32,
    faces: u32,
};

@group(0) @binding(0) var<uniform> shape: Shape;
@group(0) @binding(1) var<storage, read> src: array<f32>;
@group(0) @binding(2) var<storage, read_write> dst: array<f32>;

@compute @workgroup_size(64)
fn repack(@builtin(global_invocation_id) id: vec3<u32>) {
    let cells = shape.faces * shape.layers * shape.res * shape.res;
    if (id.x >= cells * shape.lanes) {
        return;
    }
    let lane = id.x % shape.lanes;
    let cell = id.x / shape.lanes;
    let s = cell % shape.res;
    let t = (cell / shape.res) % shape.res;
    let layer = (cell / (shape.res * shape.res)) % shape.layers;
    let face = cell / (shape.res * shape.res * shape.layers);
    let at = (((face * shape.layers + layer) * shape.res + t) * shape.res + s) * shape.lanes + lane;
    dst[id.x] = src[at];
}
"#;

/// 一维网格要几个工作组（每块 64 个线程，与 WGSL 的 `workgroup_size` 一致）。
pub fn workgroups(threads: usize) -> (u32, u32, u32) {
    ((threads.div_ceil(64).max(1)) as u32, 1, 1)
}

/// 跑一遍重排核；返回 GPU 写出的数组。
pub fn repack(res: u32, layers: u32, faces: u32, data: &[f32]) -> Result<Vec<f32>, String> {
    let Some(gpu) = connect() else {
        return Err("没有可用 GPU".to_string());
    };
    let shape = [
        res.to_le_bytes(),
        layers.to_le_bytes(),
        (LANES as u32).to_le_bytes(),
        faces.to_le_bytes(),
    ]
    .concat();
    let bytes =
        |values: &[f32]| -> Vec<u8> { values.iter().flat_map(|v| v.to_le_bytes()).collect() };
    let src = bytes(data);
    let out = dispatch(
        gpu,
        REPACK_WGSL,
        "repack",
        &[
            Binding::Uniform(&shape),
            Binding::Storage(&src),
            Binding::Write(&vec![0_u8; src.len()]),
        ],
        workgroups(data.len()),
    )?;
    Ok(out[0]
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⚠⚠ **GPU 与 CPU 必须对同一套摊平下标**。这一条是"缝"那一类错误的守门判据：
    ///   数组里放"每格都不一样的可逆编码"（就是它自己的下标），GPU 按 WGSL 的公式重排，
    ///   Rust 按 [`flat_index`] 算期望 —— 逐元素相等才算过。差一点点都会在成图上表现为
    ///   "云的位置不对"或"面上有一道边"，而两者都归因不到下标。
    ///
    /// ⚠ 没有可用设备时**跳过**（不是失败）：判据测的是映射，不是必须有卡。
    #[test]
    fn the_gpu_agrees_with_the_cpu_on_the_flat_index() {
        let (res, layers, faces) = (4_u32, 3_u32, 6_u32);
        let cells = (faces * layers * res * res) as usize;
        let data: Vec<f32> = (0..cells * LANES).map(|index| index as f32).collect();
        let Ok(gpu_side) = repack(res, layers, faces, &data) else {
            println!("px_volume_gpu_op：没有可用 GPU，跳过");
            return;
        };
        assert_eq!(gpu_side.len(), data.len(), "长度");

        // Rust 侧的期望：按 (face, layer, t, s, lane) 字典序把**那个格子自己的值**排出来。
        let mut index = 0_usize;
        for face in 0..faces {
            for layer in 0..layers {
                for t in 0..res {
                    for s in 0..res {
                        for lane in 0..LANES as u32 {
                            let want = flat_index(res, layers, face, layer, t, s, lane) as f32;
                            assert_eq!(
                                gpu_side[index], want,
                                "第 {index} 项：GPU 给了 {}，CPU 期望 {want}（face {face} 层 {layer} t {t} s {s} 通道 {lane}）",
                                gpu_side[index]
                            );
                            index += 1;
                        }
                    }
                }
            }
        }
        assert_eq!(index, data.len(), "必须逐格都对过");
    }
}

/// 跑一遍采样核：points 是 [x, y, z] 一串，返回每个点在第 lane 条通道上的值。
pub fn sample_points(
    res: u32,
    layers: u32,
    inner: f32,
    outer: f32,
    data: &[f32],
    points: &[[f32; 3]],
    lane: u32,
) -> Result<Vec<f32>, String> {
    let Some(gpu) = connect() else {
        return Err("没有可用 GPU".to_string());
    };
    let uniform = [
        res.to_le_bytes(),
        layers.to_le_bytes(),
        (LANES as u32).to_le_bytes(),
        lane.to_le_bytes(),
        inner.to_le_bytes(),
        outer.to_le_bytes(),
        0.0_f32.to_le_bytes(),
        0.0_f32.to_le_bytes(),
    ]
    .concat();
    let bytes =
        |values: &[f32]| -> Vec<u8> { values.iter().flat_map(|v| v.to_le_bytes()).collect() };
    let flat: Vec<f32> = points.iter().flat_map(|p| p.iter().copied()).collect();
    let output = vec![0_u8; points.len() * 4];
    let out = dispatch(
        gpu,
        SAMPLER_WGSL,
        "sample_points",
        &[
            Binding::Uniform(&uniform),
            Binding::Storage(&bytes(data)),
            Binding::Storage(&bytes(&flat)),
            Binding::Write(&output),
        ],
        workgroups(points.len()),
    )?;
    Ok(out[0]
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect())
}

#[cfg(test)]
mod sampler_tests {
    use super::*;
    use px_volume_schema::VolumeData;

    /// 一份**光滑**的体：值只随"格心方向"变（三线性插值才有意义），六条通道同值。
    fn smooth_volume(res: u32, layers: u32, inner: f32, outer: f32) -> VolumeData {
        let mut data = vec![0.0_f32; (6 * layers * res * res * LANES as u32) as usize];
        for face in 0..6_u32 {
            for layer in 0..layers {
                let altitude = layer as f32 / (layers - 1).max(1) as f32;
                let radius = inner + (outer - inner) * altitude;
                for t in 0..res {
                    for s in 0..res {
                        let d = px_volume_schema::direction_of(
                            face,
                            (s as f32 + 0.5) / res as f32,
                            (t as f32 + 0.5) / res as f32,
                        );
                        let value = 0.3
                            + 0.2 * (5.0 * d[0]).sin() * (5.0 * d[1]).sin() * (5.0 * d[2]).sin()
                            + 0.05 * (radius - inner);
                        let at = flat_index(res, layers, face, layer, t, s, 0);
                        for lane in 0..LANES {
                            data[at + lane] = value * radius;
                        }
                    }
                }
            }
        }
        VolumeData {
            res,
            layers,
            inner,
            outer,
            data,
        }
    }

    /// ⚠⚠ **GPU 的采样必须与 CPU 逐点一致**（容差内）。
    ///
    /// 取点刻意分成三档，因为它们的错法各不相同：
    ///  * **面内一般点** —— 三线性权重、ltitude → layer0 的错法；
    ///  * **面棱上的点**（s = 0 / 	 = 0 那一列）—— 这正是那道竖缝的现场：
    ///    八个角里有一半必须落到**相邻面**上去；
    ///  * **壳壁附近**（inner / outer 一个纹素之内）—— 边界容差与 clamp 的错法。
    #[test]
    fn the_gpu_sampler_agrees_with_the_cpu_point_by_point() {
        let (res, layers) = (8_u32, 4_u32);
        let (inner, outer) = (1.0_f32, 2.0_f32);
        let volume = smooth_volume(res, layers, inner, outer);
        let mut points: Vec<[f32; 3]> = Vec::new();
        // 面内一般点 + 棱上取点：扫一遍方向，其中 s/t 取 0 与 1 就是棱。
        for face in 0..6_u32 {
            for &s in &[0.0_f32, 0.13, 0.5, 0.87, 1.0] {
                for &t in &[0.0_f32, 0.13, 0.5, 0.87, 1.0] {
                    for &radius in &[inner + 0.01, 1.5, outer - 0.01] {
                        let d = px_volume_schema::direction_of(face, s, t);
                        points.push([d[0] * radius, d[1] * radius, d[2] * radius]);
                    }
                }
            }
        }
        let Ok(gpu_side) = sample_points(res, layers, inner, outer, &volume.data, &points, 0)
        else {
            println!("px_volume_gpu_op：没有可用 GPU，跳过");
            return;
        };
        assert_eq!(gpu_side.len(), points.len(), "点数");
        let mut worst = 0.0_f32;
        let mut worst_at = 0;
        for (index, point) in points.iter().enumerate() {
            let want = px_volume_alg::sample_volume(&volume, *point, 0);
            let got = gpu_side[index];
            let diff = (got - want).abs();
            if diff > worst {
                worst = diff;
                worst_at = index;
            }
        }
        assert!(
            worst < 2e-3,
            "最大偏差 {worst:.6} 在第 {worst_at} 个点（GPU {} 对 CPU {}）—— 两侧的采样语义不一致",
            gpu_side[worst_at],
            px_volume_alg::sample_volume(&volume, points[worst_at], 0)
        );
        println!("px_volume_gpu_op：{} 个点最大偏差 {worst:.6}", points.len());
    }
}

// ------------------------- 天空那一档的只读参数块 -------------------------
//
// 分级表与调参常量只有一处真源（Rust）：WGSL 只读这一块，不复制任何手调数字。
// 否则「改了色相却只改了 CPU 那份」会表现为「GPU 与 CPU 出图不同」，而归因不到常量。
// 布局按 uniform 的 16 字节规矩排（每 4 个 f32/u32 一组一个 vec4），
// size_of 与 to_bytes().len() 必须相等，并且有判据钉住。

/// raymarch_channel 的步进参数 + raymarch_sky 的星点与分级表。
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyUniform {
    /// (steps, face, 未用, 未用)
    pub counts: [u32; 4],
    /// (jitter, star_gain, star_floor, enter) —— enter 是壳的内半径（步进起点）。
    pub scalars: [f32; 4],
    /// 太空底色（rgb + 未用）。
    pub background: [f32; 4],
    /// 响应曲线的输入锚点（本次烘焙实测的输入分位）。
    pub tone_in: [f32; 4],
    /// 响应曲线的输出锚点（参考图的分位）。
    pub tone_out: [f32; 4],
    /// 逐格分级的档位亮度（4 档）。
    pub ramp_luma: [f32; 4],
    /// 逐格分级的档位色相（4 x rgb + 未用）。
    pub ramp_hue: [[f32; 4]; 4],
}

impl SkyUniform {
    /// 按内存布局导出：WGSL 那边的 struct 必须逐字段对上（判据钉住字节数）。
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(std::mem::size_of::<Self>());
        for value in self.counts {
            out.extend_from_slice(&value.to_le_bytes());
        }
        for value in self.scalars {
            out.extend_from_slice(&value.to_le_bytes());
        }
        for group in [self.background, self.tone_in, self.tone_out, self.ramp_luma] {
            for value in group {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
        for hue in self.ramp_hue {
            for value in hue {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
        out
    }
}

#[cfg(test)]
mod uniform_tests {
    use super::*;

    /// 布局判据：uniform 的 16 字节规矩 + Rust 与 WGSL 两侧字段一处不差。
    /// 错法的症状是「画面整体错位/花屏」，根因在字节排布 —— 归因极远。
    #[test]
    fn the_sky_uniform_layout_is_pinned() {
        let value = SkyUniform {
            counts: [96, 1024, 0, 0],
            scalars: [1.0, 0.024, 0.4, 1.0],
            background: [0.0006, 0.0004, 0.0005, 0.0],
            tone_in: [0.002672, 0.014921, 0.041914, 0.110530],
            tone_out: [0.0051, 0.0171, 0.0746, 0.2489],
            ramp_luma: [0.028, 0.034, 0.12, 0.35],
            ramp_hue: [
                [1.0, 0.24, 0.41, 0.0],
                [1.0, 0.42, 0.58, 0.0],
                [1.0, 1.14, 2.30, 0.0],
                [1.0, 0.95, 1.05, 0.0],
            ],
        };
        let bytes = value.to_bytes();
        assert_eq!(
            bytes.len(),
            std::mem::size_of::<SkyUniform>(),
            "逐字段导出应当正好等于内存布局"
        );
        assert_eq!(
            bytes.len() % 16,
            0,
            "uniform 块必须是 16 的倍数，实际 {}",
            bytes.len()
        );
        assert_eq!(
            bytes.len(),
            160,
            "字段变了就要同步 WGSL 的 struct（现为 16*6+64）"
        );
    }
}

// --------------------- 移植路线上的两个硬约束（WGSL 语言层面） ---------------------
//
// 记在这里是因为它们不是风格问题，而是会让人反复撞墙的机制：
//
// 1. **WGSL 没有 include**。采样那套函数（cube_direction / cube_face_of / snap /
//    corner_slot / gather_at / sample_volume）要在两个入口之间共用，只能靠 Rust 侧
//    concat!(include_str!(...)) 把「只有函数的文件」拼进各个入口模块。
//    复制一份进每个入口是错的：那份副本不会跟着真源走。
//
// 2. **一个模块里同一 binding 号只能有一种类型**。采样入口用
//    0=体积 uniform，1=体数据，2=点表，3=输出；步进入口要的是
//    0=体积 uniform，1=体数据，2=天空参数 uniform，3=图。binding 2/3 的类型冲突，
//    而 wgpu 的管线布局是按入口实际用到的绑定建的 —— 于是入口必须分模块，
//    函数共用的部分单独一个文件。
//
// 因此文件结构定为：
//   sampler_fn.wgsl   只有函数（无绑定、无入口）
//   sample_points.wgsl  入口 + 绑定（判据已绿：450 点与 CPU 逐点一致）
//   march.wgsl          入口 + 绑定（下一步）
//   lib.rs 用 concat! 拼出两个模块，绑定的**类型**只在各入口里声明一次。
//
// 步进入口的第一条判据打算走**解析解**而不是 CPU 对照：常发射 e、常消光 s、路径长 L 时
// 行进结果应当收敛到 (e/s)(1 - exp(-s*L))。它不依赖任何 CPU 实现，因而能先把
// 「步长约定 / 透过率递推 / 起点 enter」这三件事单独钉死；等这一条绿了，
// 再拿真体积与 px_volume_alg::raymarch_channel 对账。

/// 跑一遍步进核（单通道）；返回 面 x 面 x 6 个 texel 的辐射，布局 row = 面 * face + y。
///
/// ⚠ 起点约定（与 CPU 那份对齐前先自己说清）：中点取样，第 i 步在
/// enter + (i + 0.5) * h，h = (outer - enter) / steps。先有这条约定，才谈得上对账。
pub fn march(
    face: u32,
    steps: u32,
    lane: u32,
    enter: f32,
    res: u32,
    layers: u32,
    inner: f32,
    outer: f32,
    data: &[f32],
) -> Result<Vec<f32>, String> {
    let Some(gpu) = connect() else {
        return Err("没有可用 GPU".to_string());
    };
    let volume_uniform = [
        res.to_le_bytes(),
        layers.to_le_bytes(),
        (LANES as u32).to_le_bytes(),
        0_u32.to_le_bytes(),
        inner.to_le_bytes(),
        outer.to_le_bytes(),
        0.0_f32.to_le_bytes(),
        0.0_f32.to_le_bytes(),
    ]
    .concat();
    let sky = SkyUniform {
        counts: [steps, face, lane, 0],
        scalars: [0.0, 0.0, 0.0, enter],
        background: [0.0; 4],
        tone_in: [0.0; 4],
        tone_out: [0.0; 4],
        ramp_luma: [0.0; 4],
        ramp_hue: [[0.0; 4]; 4],
    };
    let texels = (face * face * 6) as usize;
    let bytes =
        |values: &[f32]| -> Vec<u8> { values.iter().flat_map(|v| v.to_le_bytes()).collect() };
    let out = px_gpu::dispatch_slots(
        gpu,
        SAMPLER_WGSL,
        "march",
        &[
            px_gpu::Slot {
                binding: 0,
                value: Binding::Uniform(&volume_uniform),
            },
            px_gpu::Slot {
                binding: 1,
                value: Binding::Storage(&bytes(data)),
            },
            px_gpu::Slot {
                binding: 4,
                value: Binding::Uniform(&sky.to_bytes()),
            },
            px_gpu::Slot {
                binding: 5,
                value: Binding::Write(&vec![0_u8; texels * 4]),
            },
        ],
        workgroups(texels),
    )?;
    Ok(out[0]
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect())
}

#[cfg(test)]
mod march_tests {
    use super::*;

    /// 步进入口的第一条判据走**解析解**：常发射 e、常消光 s、路径 L = outer - enter 时
    /// 积分应当收敛到 (e/s)(1 - exp(-s*L))。它不依赖任何 CPU 实现，因而能把
    /// 「步长约定 / 透过率递推 / 起点 enter」这三件事**单独**钉死。
    /// 中点黎曼和的误差是 O(h^2)，256 步时远小于容差。
    #[test]
    fn the_march_converges_to_the_analytic_solution() {
        let (res, layers) = (8_u32, 4_u32);
        let (inner, outer) = (1.0_f32, 2.0_f32);
        // e = s = 1（六条通道同值），于是解析值是 1 - 1/e。
        let data = vec![1.0_f32; (6 * layers * res * res * LANES as u32) as usize];
        let Ok(gpu_side) = march(8, 256, 0, inner, res, layers, inner, outer, &data) else {
            println!("px_volume_gpu_op：没有可用 GPU，跳过");
            return;
        };
        assert_eq!(gpu_side.len(), 8 * 8 * 6, "texel 数");
        let want = 1.0 - (-1.0_f32).exp();
        let worst = gpu_side
            .iter()
            .map(|value| (value - want).abs())
            .fold(0.0_f32, f32::max);
        assert!(
            worst < 2e-3,
            "步进结果应当收敛到解析解 {want:.6}，最大偏差 {worst:.6}"
        );
        println!("px_volume_gpu_op：步进 {worst:.6} 偏差（解析解 {want:.6}）");
    }
}
