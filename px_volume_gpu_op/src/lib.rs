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

/// 派发尺寸：每块 64 个线程，与 WGSL 的 `workgroup_size` 一致。
///
/// ⚠⚠ x 方向**必须切在 65535 以内**（`max_compute_workgroups_per_dimension`，WebGPU 规范常数）：
///   天穹 face 1024 要 98304 个工作组，一维派发会越界 —— 症状是烘图报 wgpu 校验错、
///   panic 穿过算子的 dylib 边界、整个烘焙进程 abort，且**没有任何可读信息**（实测踩过）。
///   WGSL 侧用同一把尺子（`flat_index_of` 里的 `WG_X`）把 (x, y) 摊平。
pub fn workgroups(threads: usize) -> (u32, u32, u32) {
    const WG_X: usize = 65535;
    let blocks = threads.div_ceil(64).max(1);
    let x = blocks.min(WG_X);
    let y = blocks.div_ceil(WG_X);
    (x as u32, y as u32, 1)
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
    /// 响应曲线的软肩起点与上限（`(shoulder, ceil, 未用, 未用)`）。
    pub limits: [f32; 4],
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
        for value in self.limits {
            out.extend_from_slice(&value.to_le_bytes());
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
            limits: [0.72, 0.95, 0.0, 0.0],
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
            176,
            "字段变了就要同步 WGSL 的 struct（现为 16*7+64）"
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

/// 步进的**可选**输入（星图与底色）：与分级表一样，参数只有一处真源。
#[derive(Default, Clone, Copy)]
pub struct MarchExtras<'a> {
    /// 星图（立方贴图场，行主序：下标 = y * face_size + x）。`face_size = 0` 表示没有星。
    pub stars: Option<&'a [f32]>,
    /// 星图一面多大（0 = 没有星图）。
    pub star_face: u32,
    /// 星点增益（`SkyParams::star_gain`）。
    pub star_gain: f32,
    /// 星点地板（`SkyParams::star_floor`）。
    pub star_floor: f32,
    /// 太空底色（`SkyParams::background`）。
    pub background: [f32; 3],
}

/// 跑一遍响应曲线（逐格，就地）：输入 luma 数组，输出同一长度。
pub fn tone_of(
    values: &[f32],
    tone_in: [f32; 4],
    tone_out: [f32; 4],
    limits: [f32; 2],
) -> Result<Vec<f32>, String> {
    let Some(gpu) = connect() else {
        return Err("没有可用 GPU".to_string());
    };
    let sky = SkyUniform {
        counts: [0; 4],
        scalars: [0.0; 4],
        background: [0.0; 4],
        tone_in,
        tone_out,
        ramp_luma: [0.0; 4],
        ramp_hue: [[0.0; 4]; 4],
        limits: [limits[0], limits[1], 0.0, 0.0],
    };
    let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    let out = px_gpu::dispatch_slots(
        gpu,
        SAMPLER_WGSL,
        "tone_of",
        &[
            px_gpu::Slot {
                binding: 4,
                value: Binding::Uniform(&sky.to_bytes()),
            },
            px_gpu::Slot {
                binding: 5,
                value: Binding::Write(&bytes),
            },
        ],
        workgroups(values.len()),
    )?;
    Ok(out[0]
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect())
}

#[cfg(test)]
mod tone_tests {
    use super::*;

    /// **响应曲线**：GPU 与 CPU 在同一组锚点上必须一致。
    /// 取点覆盖四段（两端外推 + 中间两段）与软肩折点附近 —— 每一段的错法都不同。
    #[test]
    fn the_gpu_tone_matches_the_cpu() {
        let mut values: Vec<f32> = Vec::new();
        // 对数扫：1e-5 .. 2.0，覆盖两端外推与三段插值。
        let mut l = 1e-5_f32;
        while l < 2.0 {
            values.push(l);
            values.push(l * 1.0007);
            l *= 1.13;
        }
        for extra in [0.0_f32, 1e-9, 0.72, 0.95, 1.0, 5.0] {
            values.push(extra);
        }
        let anchors_in = px_volume_alg::raymarch::TONE_IN;
        let anchors_out = px_volume_alg::raymarch::TONE_OUT;
        let limits = px_volume_alg::TONE_LIMITS;
        let Ok(gpu_side) = tone_of(&values, anchors_in, anchors_out, limits) else {
            println!("px_volume_gpu_op：没有可用 GPU，跳过");
            return;
        };
        let mut worst = 0.0_f32;
        let mut worst_at = 0usize;
        for (index, value) in values.iter().enumerate() {
            let want = px_volume_alg::tone_at(*value);
            let got = gpu_side[index];
            let diff = (got - want).abs() / want.abs().max(1e-3);
            if diff > worst {
                worst = diff;
                worst_at = index;
            }
        }
        assert!(
            worst < 3e-3,
            "响应曲线最大相对偏差 {worst:.6} 在 luma {}（GPU {} 对 CPU {}）",
            values[worst_at],
            gpu_side[worst_at],
            px_volume_alg::tone_at(values[worst_at])
        );
        println!(
            "px_volume_gpu_op：响应曲线最大相对偏差 {worst:.6}（{} 点）",
            values.len()
        );
    }
}

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
    extras: &MarchExtras<'_>,
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
        counts: [steps, face, lane, extras.star_face],
        scalars: [0.0, extras.star_gain, extras.star_floor, enter],
        background: [
            extras.background[0],
            extras.background[1],
            extras.background[2],
            0.0,
        ],
        tone_in: [0.0; 4],
        tone_out: [0.0; 4],
        ramp_luma: [0.0; 4],
        ramp_hue: [[0.0; 4]; 4],
        limits: [0.0; 4],
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
            px_gpu::Slot {
                binding: 6,
                value: Binding::Storage(&match extras.stars {
                    Some(stars) => bytes(stars),
                    None => vec![0_u8; 4],
                }),
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
        let Ok(gpu_side) = march(
            8,
            256,
            0,
            inner,
            res,
            layers,
            inner,
            outer,
            &data,
            &MarchExtras::default(),
        ) else {
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

#[cfg(test)]
mod crosscheck_tests {
    use super::*;
    use px_volume_schema::VolumeData;

    /// 一份发射与消光**各自不同**的体：让透过率递推真正参与进来（常值体测不出递推）。
    /// 六条通道都填：lane 0..2 发射、3..5 消光。
    fn varying_volume(res: u32, layers: u32, inner: f32, outer: f32) -> VolumeData {
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
                        let wave = (3.0 * d[0]).sin() * (4.0 * d[1]).cos() * (5.0 * d[2]).sin();
                        let at = flat_index(res, layers, face, layer, t, s, 0);
                        for lane in 0..LANES {
                            data[at + lane] = match lane {
                                0..=2 => (0.4 + 0.3 * wave).max(0.0) * (1.0 + 0.1 * lane as f32),
                                _ => (0.2 + 0.5 * (wave * 0.5 + 0.5)).max(0.0),
                            };
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

    /// 真体积上与 CPU 对账：同一步进参数下，GPU 与 `raymarch_channel` 必须一致。
    ///
    /// 这条判据的价值在于它**同时**校验四件事：方向参数化（`art_direction_at` 对
    /// `cube_direction`）、格点布局、跨面采样、以及步进的起点/步长约定。
    /// 前三条已各自单测过，这里是它们合起来的结果。
    #[test]
    fn the_gpu_march_matches_the_cpu_channel_on_a_real_volume() {
        let (res, layers) = (8_u32, 4_u32);
        let (inner, outer) = (1.0_f32, 2.0_f32);
        let volume = varying_volume(res, layers, inner, outer);
        let face = 8_u32;
        let steps = 64_u32;
        let params = px_volume_schema::params::sky::SkyParams {
            face,
            steps,
            jitter: 0.0,
            ..Default::default()
        };
        let reference = px_volume_alg::raymarch_channel(&volume, None, &params, 0);
        let Ok(gpu_side) = march(
            face,
            steps,
            0,
            inner,
            res,
            layers,
            inner,
            outer,
            &volume.data,
            &MarchExtras::default(),
        ) else {
            println!("px_volume_gpu_op：没有可用 GPU，跳过");
            return;
        };
        assert_eq!(gpu_side.len(), reference.data.len(), "texel 数");
        let mut worst = 0.0_f32;
        let mut worst_at = 0usize;
        for (index, value) in gpu_side.iter().enumerate() {
            let want = reference.data[index];
            if (value - want).abs() > worst {
                worst = (value - want).abs();
                worst_at = index;
            }
        }
        assert!(
            worst < 5e-3,
            "最大偏差 {worst:.6} 在第 {worst_at} 个 texel（GPU {} 对 CPU {}）",
            gpu_side[worst_at],
            reference.data[worst_at]
        );
        println!(
            "px_volume_gpu_op：真体积对账最大偏差 {worst:.6}（{} 个 texel）",
            gpu_side.len()
        );
    }
}

#[cfg(test)]
mod star_tests {
    use super::*;
    use px_field_schema::field::{Field, Projection};
    use px_volume_schema::VolumeData;

    /// **星点 + 底色**也要与 CPU 一致：这一条把"加性部分"（乘透射率的星与背景）
    /// 与已对上的步进语义分开钉住。
    #[test]
    fn the_gpu_march_matches_the_cpu_with_stars_and_background() {
        let (res, layers) = (8_u32, 4_u32);
        let (inner, outer) = (1.0_f32, 2.0_f32);
        let mut data = vec![0.0_f32; (6 * layers * res * res * LANES as u32) as usize];
        for lane in 0..LANES {
            for (index, value) in data.iter_mut().enumerate() {
                if index % LANES == lane {
                    *value = if lane < 3 { 0.5 } else { 0.8 };
                }
            }
        }
        let volume = VolumeData {
            res,
            layers,
            inner,
            outer,
            data,
        };
        // 星图：一面 4 格，值散开（含低于地板的一档，测地板分支）。
        let star_face = 4_u32;
        let stars: Vec<f32> = (0..(star_face * star_face * 6) as usize)
            .map(|index| {
                if index % 7 == 0 {
                    0.1
                } else {
                    0.2 + (index % 5) as f32 * 0.15
                }
            })
            .collect();
        let stars_field =
            Field::with_projection(star_face, star_face * 6, stars.clone(), Projection::CubeMap);
        let params = px_volume_schema::params::sky::SkyParams {
            face: 8,
            steps: 32,
            jitter: 0.0,
            star_gain: 0.07,
            star_floor: 0.25,
            background: [0.0011, 0.0007, 0.0009],
            ..Default::default()
        };
        let reference = px_volume_alg::raymarch_channel(&volume, Some(&stars_field), &params, 0);
        let extras = MarchExtras {
            stars: Some(&stars),
            star_face,
            star_gain: params.star_gain,
            star_floor: params.star_floor,
            background: params.background,
        };
        let Ok(gpu_side) = march(
            params.face,
            params.steps,
            0,
            inner,
            res,
            layers,
            inner,
            outer,
            &volume.data,
            &extras,
        ) else {
            println!("px_volume_gpu_op：没有可用 GPU，跳过");
            return;
        };
        let mut worst = 0.0_f32;
        for (index, value) in gpu_side.iter().enumerate() {
            worst = worst.max((value - reference.data[index]).abs());
        }
        assert!(worst < 5e-3, "星点+底色对账最大偏差 {worst:.6}");
        println!("px_volume_gpu_op：星点+底色对账最大偏差 {worst:.6}");
    }
}

/// 跑一遍档位色相：入参是**键**（每格一个亮度），出参是每个键的目标色相（3 个 f32 一组）。
pub fn hue_of(
    keys: &[f32],
    ramp_luma: [f32; 4],
    ramp_hue_table: [[f32; 4]; 4],
) -> Result<Vec<[f32; 3]>, String> {
    let Some(gpu) = connect() else {
        return Err("没有可用 GPU".to_string());
    };
    let sky = SkyUniform {
        counts: [0; 4],
        scalars: [0.0; 4],
        background: [0.0; 4],
        tone_in: [0.0; 4],
        tone_out: [0.0; 4],
        ramp_luma,
        ramp_hue: ramp_hue_table,
        limits: [0.0; 4],
    };
    let mut packed: Vec<f32> = Vec::with_capacity(keys.len() * 3);
    for key in keys {
        packed.extend_from_slice(&[*key, 0.0, 0.0]);
    }
    let bytes: Vec<u8> = packed.iter().flat_map(|v| v.to_le_bytes()).collect();
    let out = px_gpu::dispatch_slots(
        gpu,
        SAMPLER_WGSL,
        "hue_of_keys",
        &[
            px_gpu::Slot {
                binding: 4,
                value: Binding::Uniform(&sky.to_bytes()),
            },
            px_gpu::Slot {
                binding: 5,
                value: Binding::Write(&bytes),
            },
        ],
        workgroups(keys.len()),
    )?;
    Ok(out[0]
        .chunks_exact(12)
        .map(|chunk| {
            [
                f32::from_le_bytes(chunk[0..4].try_into().unwrap()),
                f32::from_le_bytes(chunk[4..8].try_into().unwrap()),
                f32::from_le_bytes(chunk[8..12].try_into().unwrap()),
            ]
        })
        .collect())
}

#[cfg(test)]
mod hue_tests {
    use super::*;

    /// **档位色相**：GPU 与 CPU 在同一张档位表上必须一致。
    /// 取点铺满四档与三段过渡的**收窄带**，含 0.0279/0.028/0.034/0.058/0.35 这些边界值。
    #[test]
    fn the_gpu_hue_ramp_matches_the_cpu() {
        let ramp_luma = px_volume_alg::raymarch::RAMP_LUMA;
        let table = px_volume_alg::raymarch::RAMP_HUE;
        let ramp_hue_table = [
            [table[0][0], table[0][1], table[0][2], 0.0],
            [table[1][0], table[1][1], table[1][2], 0.0],
            [table[2][0], table[2][1], table[2][2], 0.0],
            [table[3][0], table[3][1], table[3][2], 0.0],
        ];
        let mut keys: Vec<f32> = Vec::new();
        let mut key = 1e-4_f32;
        while key < 1.0 {
            keys.push(key);
            key *= 1.07;
        }
        for extra in [0.0_f32, 0.0279, 0.028, 0.034, 0.058, 0.35, 2.0] {
            keys.push(extra);
        }
        let Ok(gpu_side) = hue_of(&keys, ramp_luma, ramp_hue_table) else {
            println!("px_volume_gpu_op：没有可用 GPU，跳过");
            return;
        };
        let mut worst = 0.0_f32;
        let mut worst_at = 0usize;
        for (index, key) in keys.iter().enumerate() {
            let want = px_volume_alg::ramp_hue_at(*key);
            for channel in 0..3 {
                let diff = (gpu_side[index][channel] - want[channel]).abs();
                if diff > worst {
                    worst = diff;
                    worst_at = index;
                }
            }
        }
        assert!(
            worst < 3e-3,
            "档位色相最大偏差 {worst:.6} 在键 {}（GPU {:?} 对 CPU {:?}）",
            keys[worst_at],
            gpu_side[worst_at],
            px_volume_alg::ramp_hue_at(keys[worst_at])
        );
        println!(
            "px_volume_gpu_op：档位色相最大偏差 {worst:.6}（{} 个键）",
            keys.len()
        );
    }
}

/// 跑一遍**整条天空**（GPU 版 `raymarch_sky` 的辐射+分级部分）：
/// 返回每个 texel 三个 f32（分级后的线性 RGB，尚未打包成 Rgba16Float）。
#[allow(clippy::too_many_arguments)]
pub fn sky(
    face: u32,
    steps: u32,
    enter: f32,
    res: u32,
    layers: u32,
    inner: f32,
    outer: f32,
    data: &[f32],
    extras: &MarchExtras<'_>,
    tone: ([f32; 4], [f32; 4], [f32; 2]),
    ramp: ([f32; 4], [[f32; 4]; 4]),
    grade_strength: f32,
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
    let sky_uniform = SkyUniform {
        counts: [steps, face, 0, extras.star_face],
        scalars: [0.0, extras.star_gain, extras.star_floor, enter],
        background: [
            extras.background[0],
            extras.background[1],
            extras.background[2],
            0.0,
        ],
        tone_in: tone.0,
        tone_out: tone.1,
        ramp_luma: ramp.0,
        ramp_hue: ramp.1,
        limits: [tone.2[0], tone.2[1], grade_strength, 0.0],
    };
    let texels = (face * face * 6) as usize;
    let bytes = |values: &[f32]| -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    };
    let out = px_gpu::dispatch_slots(
        gpu,
        SAMPLER_WGSL,
        "sky_radiance",
        &[
            px_gpu::Slot { binding: 0, value: Binding::Uniform(&volume_uniform) },
            px_gpu::Slot { binding: 1, value: Binding::Storage(&bytes(data)) },
            px_gpu::Slot { binding: 4, value: Binding::Uniform(&sky_uniform.to_bytes()) },
            px_gpu::Slot { binding: 5, value: Binding::Write(&vec![0_u8; texels * 12]) },
            px_gpu::Slot {
                binding: 6,
                value: Binding::Storage(&match extras.stars {
                    Some(stars) => bytes(stars),
                    None => vec![0_u8; 4],
                }),
            },
        ],
        workgroups(texels),
    )?;
    let radiance = out[0]
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect::<Vec<f32>>();
    grade_pixels(gpu, &radiance, &sky_uniform)
}

/// 分级那一趟（就地）：拿 `sky_radiance` 的输出再走一遍响应曲线 + 色相斜坡。
fn grade_pixels(
    gpu: &px_gpu::Gpu,
    radiance: &[f32],
    sky_uniform: &SkyUniform,
) -> Result<Vec<f32>, String> {
    let bytes: Vec<u8> = radiance.iter().flat_map(|v| v.to_le_bytes()).collect();
    let out = px_gpu::dispatch_slots(
        gpu,
        SAMPLER_WGSL,
        "grade_pixels",
        &[
            px_gpu::Slot { binding: 4, value: Binding::Uniform(&sky_uniform.to_bytes()) },
            px_gpu::Slot { binding: 5, value: Binding::Write(&bytes) },
        ],
        workgroups(radiance.len() / 3),
    )?;
    Ok(out[0]
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect())
}

/// **烘焙时量出来的输入锚点**：辐射亮度的分位（对数分箱直方图）。
///
/// ⚠ 锚点的定义就是"本次烘焙的输入分位 -> 参考的输出分位" ⇒ 只有**量**出来才与分辨率、
///   面数、体积解耦；写死一组常数的话，换分辨率就得回来重拟（实测：192^3/face 4096 时
///   p50 从 0.0194 漂到 0.0246）。
pub fn anchors_from_radiance(
    gpu: &px_gpu::Gpu,
    radiance: &[f32],
    sky_uniform: &SkyUniform,
) -> Result<[f32; 4], String> {
    const BIN_COUNT: usize = 512;
    let bytes: Vec<u8> = radiance.iter().flat_map(|v| v.to_le_bytes()).collect();
    let out = px_gpu::dispatch_slots(
        gpu,
        SAMPLER_WGSL,
        "bin_luma",
        &[
            px_gpu::Slot { binding: 4, value: Binding::Uniform(&sky_uniform.to_bytes()) },
            px_gpu::Slot { binding: 5, value: Binding::Write(&bytes) },
            px_gpu::Slot { binding: 7, value: Binding::Write(&vec![0_u8; BIN_COUNT * 4]) },
        ],
        workgroups(radiance.len() / 3),
    )?;
    let counts: Vec<u32> = out[1]
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes(chunk.try_into().unwrap()))
        .collect();
    let (log_min, log_max) = (-16.0_f32, 4.0_f32);
    let total: u64 = counts.iter().map(|c| *c as u64).sum();
    if total == 0 {
        return Err("直方图是空的（辐射全零？）".to_string());
    }
    let targets = [0.10_f64, 0.50, 0.85, 0.99];
    let mut anchors = [0.0_f32; 4];
    let mut cumulative = 0_u64;
    let mut next = 0;
    for (bin, count) in counts.iter().enumerate() {
        cumulative += *count as u64;
        while next < 4 && cumulative as f64 >= targets[next] * total as f64 {
            let position = (bin as f32 + 0.5) / BIN_COUNT as f32;
            anchors[next] = 2.0_f32.powf(log_min + position * (log_max - log_min));
            next += 1;
        }
    }
    for value in anchors.iter_mut() {
        if *value <= 0.0 {
            *value = 1e-4;
        }
    }
    // ⚠⚠ 分位**撞进同一个箱**时锚点会相等 ⇒ 响应曲线里 `log(hi/lo) = 0` ⇒ 斜率除零 ⇒
    //   那一段的输出变成 NaN/∞（症状是画面上一块突然错开，而不是"暗一点"）。
    //   这里强制**严格递增**并留最小间隔。
    for index in 1..4 {
        let floor = anchors[index - 1] * 1.06;
        if anchors[index] < floor {
            anchors[index] = floor;
        }
    }
    Ok(anchors)
}

#[cfg(test)]
mod sky_tests {
    use super::*;
    use px_field_schema::field::{Field, Projection};
    use px_volume_schema::VolumeData;

    /// 标准 IEEE-754 binary16 -> f32（**判据侧自带**：`px_volume_alg::half` 只有编码口，
    /// 而这里要读回它产出的 `Rgba16Float`。格式是公开标准，与被测逻辑无关）。
    fn f32_from_half(bits: u16) -> f32 {
        let sign = if bits & 0x8000 != 0 { -1.0_f32 } else { 1.0 };
        let exponent = ((bits >> 10) & 0x1f) as i32;
        let mantissa = (bits & 0x3ff) as f32;
        match exponent {
            0 => sign * mantissa * 2.0_f32.powi(-24),
            31 => {
                if mantissa == 0.0 { sign * f32::INFINITY } else { f32::NAN }
            }
            _ => sign * (1.0 + mantissa / 1024.0) * 2.0_f32.powi(exponent - 15),
        }
    }

    fn fixture(res: u32, layers: u32, inner: f32, outer: f32) -> VolumeData {
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
                        let wave = (3.0 * d[0]).sin() * (4.0 * d[1]).cos() * (5.0 * d[2]).sin();
                        let at = flat_index(res, layers, face, layer, t, s, 0);
                        for lane in 0..LANES {
                            data[at + lane] = match lane {
                                0..=2 => (0.45 + 0.3 * wave).max(0.0) * (1.0 + 0.12 * lane as f32),
                                _ => (0.25 + 0.5 * (wave * 0.5 + 0.5)).max(0.0),
                            };
                        }
                    }
                }
            }
        }
        VolumeData { res, layers, inner, outer, data }
    }

    /// **整条天空逐 texel 对账**：三条通道的步进 + 星点/底色 + 亮度响应 + 色相分级，
    /// 合起来必须与 CPU 的 `raymarch_sky` 一致（容差取半精度的量级：参考图那一侧是
    /// `Rgba16Float`，有效位就那么多）。
    #[test]
    fn the_gpu_sky_matches_the_cpu_end_to_end() {
        let (res, layers) = (8_u32, 4_u32);
        let (inner, outer) = (1.0_f32, 2.0_f32);
        let volume = fixture(res, layers, inner, outer);
        let star_face = 4_u32;
        let stars: Vec<f32> = (0..(star_face * star_face * 6) as usize)
            .map(|index| if index % 7 == 0 { 0.1 } else { 0.2 + (index % 5) as f32 * 0.15 })
            .collect();
        let stars_field =
            Field::with_projection(star_face, star_face * 6, stars.clone(), Projection::CubeMap);
        let params = px_volume_schema::params::sky::SkyParams {
            face: 8,
            steps: 32,
            jitter: 0.0,
            star_gain: 0.07,
            star_floor: 0.25,
            background: [0.0011, 0.0007, 0.0009],
            ..Default::default()
        };
        let reference =
            px_volume_alg::raymarch_sky(&volume, &stars_field, &params).expect("CPU 出图");
        let table = px_volume_alg::raymarch::RAMP_HUE;
        let ramp_hue_table = [
            [table[0][0], table[0][1], table[0][2], 0.0],
            [table[1][0], table[1][1], table[1][2], 0.0],
            [table[2][0], table[2][1], table[2][2], 0.0],
            [table[3][0], table[3][1], table[3][2], 0.0],
        ];
        let extras = MarchExtras {
            stars: Some(&stars),
            star_face,
            star_gain: params.star_gain,
            star_floor: params.star_floor,
            background: params.background,
        };
        let Ok(gpu_side) = sky(
            params.face,
            params.steps,
            inner,
            res,
            layers,
            inner,
            outer,
            &volume.data,
            &extras,
            (
                px_volume_alg::raymarch::TONE_IN,
                px_volume_alg::raymarch::TONE_OUT,
                px_volume_alg::TONE_LIMITS,
            ),
            (px_volume_alg::raymarch::RAMP_LUMA, ramp_hue_table),
            px_volume_alg::GRADE_STRENGTH,
        ) else {
            println!("px_volume_gpu_op：没有可用 GPU，跳过");
            return;
        };
        // 参考是 Rgba16Float：每 texel 8 字节（前三个 half 是 RGB，第四个是 1.0）。
        let texels = reference.bytes.len() / 8;
        assert_eq!(gpu_side.len(), texels * 3, "texel 数");
        let mut worst = 0.0_f32;
        let mut worst_at = 0usize;
        for index in 0..texels {
            for channel in 0..3 {
                let at = index * 8 + channel * 2;
                let want = f32_from_half(u16::from_le_bytes([
                    reference.bytes[at],
                    reference.bytes[at + 1],
                ]));
                let got = gpu_side[index * 3 + channel];
                let diff = (got - want).abs() / want.abs().max(1e-2);
                if diff > worst {
                    worst = diff;
                    worst_at = index * 3 + channel;
                }
            }
        }
        assert!(
            worst < 5e-3,
            "整链最大相对偏差 {worst:.6} 在第 {worst_at} 个分量（GPU {} 对 CPU {})",
            gpu_side[worst_at],
            f32_from_half(u16::from_le_bytes([
                reference.bytes[(worst_at / 3) * 8 + (worst_at % 3) * 2],
                reference.bytes[(worst_at / 3) * 8 + (worst_at % 3) * 2 + 1],
            ]))
        );
        println!(
            "px_volume_gpu_op：整链 {} texel 最大相对偏差 {worst:.6}",
            texels
        );
    }
}

/// **GPU 版整条天空**：与 `px_volume_alg::raymarch_sky` 同一签名 —— 图脚本那一行不用改，
/// 换的只是这一档背后的实现。
///
/// 三个阶段（都是 GPU 上的独立派发）：
/// 1. `sky_radiance`：三条通道各积一遍，得到**未分级**的辐射；
/// 2. `bin_luma` + `anchors_from_radiance`：**量出本次烘焙的输入分位**当响应曲线的锚点
///    （锚点的定义就是"本次输入分位 -> 参考输出分位"，所以必须量、不能写死）；
/// 3. `grade_pixels`：亮度响应 + 色相斜坡，逐格亮度守恒。
///
/// ⚠ 分级表与参考分位仍取自 `px_volume_alg`（唯一真源那一份）；半精度打包复用同一份
///   `half_from_f32` —— 格式只写一次。
/// ---- 设计决策（第 70 轮，用户拍板）----
///
/// 用户原话："球体坐标作为储存方式能很好地平衡近处和远处的分辨率，但**噪声场的定义应当
/// 定义在世界三维坐标**，用一个 projection 把噪声投影到球体坐标。"
///
/// ⇒ 两条结论：
/// 1. **存储不动**（六面立方球是刻意的：面内格密、径向格疏，近远分辨率平衡）。
///    我先前"改成统一三维格"的提法是错的方向。
/// 2. 折痕的根因在**重建**这一侧：场是世界定义的没错（`voxel_of` 就是那个 projection，
///    把存储格投到世界点再取噪声），但**三线性重建是按面各做一份**的 ——
///    跨棱时格架朝向切换 ⇒ 插值导数跳变（C¹ 折痕）。用户的原话"真实统一的世界场不会这样"
///    说的就是这个：**重建要跨面一致**。
///
/// ⇒ 下一个动作（R71）：在**两侧**（CPU 的 `sample_volume` 与 WGSL 的 `sample_volume`）
///   把"跨棱"的格子改成**多面混合**：只对落在面棱附近一个格宽内的采样点，
///   同时用相邻各面的格架各算一次三线性，按平滑权重混合 ⇒ C¹ 连续，折痕消失。
///   ⚠ 必须两侧同时改：`the_gpu_sampler_matches_the_cpu` 那条判据就是用来钉住它们一致的。
///   ⚠ 混合权重必须**随离棱距离平滑到 0**（否则把折痕换成一条更软的带）。
pub fn raymarch_sky(
    emission: &px_volume_schema::VolumeData,
    stars: &px_field_schema::field::Field,
    sky_params: &px_volume_schema::params::sky::SkyParams,
) -> Result<px_volume_schema::TextureData, String> {
    let face = sky_params.face.max(1);
    let steps = sky_params.steps.max(1);
    let extras = MarchExtras {
        stars: Some(&stars.data),
        star_face: stars.width,
        star_gain: sky_params.star_gain,
        star_floor: sky_params.star_floor,
        background: sky_params.background,
    };
    let hue = px_volume_alg::raymarch::RAMP_HUE;
    let ramp_hue_table = [
        [hue[0][0], hue[0][1], hue[0][2], 0.0],
        [hue[1][0], hue[1][1], hue[1][2], 0.0],
        [hue[2][0], hue[2][1], hue[2][2], 0.0],
        [hue[3][0], hue[3][1], hue[3][2], 0.0],
    ];
    let Some(gpu) = connect() else {
        return Err("没有可用 GPU：这一档现在完全跑在 GPU 上".to_string());
    };
    let volume_uniform = [
        emission.res.to_le_bytes(),
        emission.layers.to_le_bytes(),
        (LANES as u32).to_le_bytes(),
        0_u32.to_le_bytes(),
        emission.inner.to_le_bytes(),
        emission.outer.to_le_bytes(),
        0.0_f32.to_le_bytes(),
        0.0_f32.to_le_bytes(),
    ]
    .concat();
    let mut uniform = SkyUniform {
        counts: [steps, face, 0, extras.star_face],
        scalars: [0.0, extras.star_gain, extras.star_floor, emission.inner],
        background: [
            extras.background[0],
            extras.background[1],
            extras.background[2],
            0.0,
        ],
        tone_in: [0.0; 4],
        tone_out: px_volume_alg::raymarch::TONE_OUT,
        ramp_luma: px_volume_alg::raymarch::RAMP_LUMA,
        ramp_hue: ramp_hue_table,
        limits: [
            px_volume_alg::TONE_LIMITS[0],
            px_volume_alg::TONE_LIMITS[1],
            px_volume_alg::GRADE_STRENGTH,
            0.0,
        ],
    };
    let bytes = |values: &[f32]| -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    };
    let texels = (face * face * 6) as usize;
    let star_bytes = bytes(&extras.stars.map(|s| s.to_vec()).unwrap_or_else(|| vec![0.0; 1]));

    // 1) 辐射。
    let out = px_gpu::dispatch_slots(
        gpu,
        SAMPLER_WGSL,
        "sky_radiance",
        &[
            px_gpu::Slot { binding: 0, value: Binding::Uniform(&volume_uniform) },
            px_gpu::Slot { binding: 1, value: Binding::Storage(&bytes(&emission.data)) },
            px_gpu::Slot { binding: 4, value: Binding::Uniform(&uniform.to_bytes()) },
            px_gpu::Slot { binding: 5, value: Binding::Write(&vec![0_u8; texels * 12]) },
            px_gpu::Slot { binding: 6, value: Binding::Storage(&star_bytes) },
        ],
        workgroups(texels),
    )?;
    let radiance: Vec<f32> = out[0]
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect();

    // 2) 量输入分位 ⇒ 响应曲线的锚点。
    uniform.tone_in = anchors_from_radiance(gpu, &radiance, &uniform)?;

    // 3) 分级（响应 + 色相斜坡）。
    let graded = grade_pixels(gpu, &radiance, &uniform)?;
    if graded.len() != texels * 3 {
        return Err(format!(
            "GPU 出图长度不对：{}（应为 {}）",
            graded.len(),
            texels * 3
        ));
    }
    // 打包成 Rgba16Float：alpha = 1（与 CPU 那一侧逐字一致）。
    let mut packed = Vec::with_capacity(texels * 8);
    for index in 0..texels {
        for channel in 0..3 {
            packed.extend_from_slice(
                &px_volume_alg::half::half_from_f32(graded[index * 3 + channel]).to_le_bytes(),
            );
        }
        packed.extend_from_slice(&px_volume_alg::half::half_from_f32(1.0).to_le_bytes());
    }
    Ok(px_volume_schema::TextureData::new(
        face,
        face,
        6,
        1,
        px_volume_schema::TextureFormat::Rgba16Float,
        packed,
    ))
}




#[cfg(test)]
mod size_tests {
    use super::*;

    /// **真实尺寸**（shape 128 的体积 = 6 面 x 128^2 x 128 层 x 6 通道 x 4 B = 302 MB）
    /// 必须能派发。烘焙里那一档就是这个尺寸 —— 小尺寸的判据全绿也说明不了它。
    /// 失败时把 wgpu 的原文打出来（而不是让进程消失）。
    #[test]
    fn a_real_size_volume_goes_through() {
        let (res, layers, face) = (128_u32, 128_u32, 1024_u32);
        let (inner, outer) = (1.0_f32, 2.0_f32);
        let data = vec![0.5_f32; (6 * layers * res * res * LANES as u32) as usize];
        let result = march(
            face,
            4,
            0,
            inner,
            res,
            layers,
            inner,
            outer,
            &data,
            &MarchExtras::default(),
        );
        match result {
            Ok(values) => {
                assert_eq!(values.len(), (face * face * 6) as usize, "texel 数");
                println!("px_volume_gpu_op：真实尺寸（302 MB 体积 + face 1024）派发通过");
            }
            Err(message) => panic!("真实尺寸派发失败：{message}"),
        }
    }
}

#[cfg(test)]
mod seam_tests {
    use super::*;
    use px_field_schema::field::{Field, Projection};
    use px_volume_schema::VolumeData;

    fn f32_from_half(bits: u16) -> f32 {
        let sign = if bits & 0x8000 != 0 { -1.0_f32 } else { 1.0 };
        let exponent = ((bits >> 10) & 0x1f) as i32;
        let mantissa = (bits & 0x3ff) as f32;
        match exponent {
            0 => sign * mantissa * 2.0_f32.powi(-24),
            31 => {
                if mantissa == 0.0 { sign * f32::INFINITY } else { f32::NAN }
            }
            _ => sign * (1.0 + mantissa / 1024.0) * 2.0_f32.powi(exponent - 15),
        }
    }

    /// **面棱两侧的连续性**（烘焙侧的决定性判据）。
    ///
    /// ⚠ 为什么必须关掉抖动：抖动是**逐 texel 的哈希**（按方向取种子），棱两侧的抖动模式
    ///   本来就互不相关 ⇒ 每个 texel 都带一份独立噪声，会把"结构性错配"淹掉。
    ///   `jitter = 0` 之后，棱两侧的差只可能来自**采样/布局/面序**。
    ///
    /// 判法：每条棱上的 texel，找**另一面**上方向最接近的那一格，比它们的差；
    /// 再拿同面内相邻 texel 的差当基准。棱上的差若显著大于面内基准 ⇒ 烘焙侧有缝。
    /// ⚠⚠ **已知失败**（这条判据现在红着，故意的）。实测：
    /// * 面棱上的差 **0.04767** = 面内基准 **0.00789** 的 **6.04 倍**（最大 0.16117）；
    /// * 形状是**系统性**的：六个面一致（0.040~0.051）、沿整条棱均匀（分段 0.032~0.059）
    ///   ⇒ 不是面序/朝向错（那会按面、按棱给出不同图案），而是**每一条棱都发生**的东西。
    /// * 它同时存在于 GPU 与 CPU 两条路（GPU 那份与 CPU 逐 texel 只差 0.000482，
    ///   是有意对账过的）⇒ **两边同源**，所以"GPU 对 CPU"这类判据抓不到它。
    /// * 细密噪声那几版在成图上被纹理掩盖；尺度改大（基频 1.4）后一眼可见 —— 说明它一直在。
    ///
    /// 下一轮从这里二分：先做**采样器级**的连续性判据（同一份逐格随机夹具，直接比
    /// `sample_volume` 在棱两侧的点），把"采样语义"与"光线步进"分开；
    /// 再查 `sample_at` 的角点约定（主格用 `floor(s*res - 0.5)`，角点却用
    /// `u32(ns*res)` 截断 —— 两者差半个纹素的可能就在这儿）。
    #[test]
    fn the_baked_sky_is_continuous_across_face_edges() {
        let (res, layers) = (8_u32, 4_u32);
        let (inner, outer) = (1.0_f32, 2.0_f32);
        let mut data = vec![0.0_f32; (6 * layers * res * res * LANES as u32) as usize];
        // ⚠⚠ 夹具必须**无偏**：`index % 97` 那种在摊平下标上平滑的图案，会让"面内相邻格"
        //   共享相位、而"跨棱相邻格"不共享 ⇒ 人为造出 2x 的比值（实测：采样器 1.99x、
        //   步进 2.08x），再被分级放大成 6x ⇒ 看起来像烘焙侧的缝，其实是夹具的伪影。
        //   哈希随机夹具下面内与跨棱的相关性同样弱，比值才有意义（实测 1.06x）。
        for (index, value) in data.iter_mut().enumerate() {
            *value = ((index.wrapping_mul(2654435761)) % 1000) as f32 / 1000.0;
        }
        let volume = VolumeData { res, layers, inner, outer, data };
        // 星图：一面 2 格 ⇒ 高 2*6 = 12，像素数 24。
        let stars_field = Field::with_projection(2, 12, vec![0.0; 24], Projection::CubeMap);
        let params = px_volume_schema::params::sky::SkyParams {
            face: 32,
            steps: 16,
            jitter: 0.0,
            star_gain: 0.0,
            star_floor: 0.9,
            ..Default::default()
        };
        let texture = match raymarch_sky(&volume, &stars_field, &params) {
            Ok(texture) => texture,
            Err(message) => panic!("烘焙失败：{message}"),
        };
        let face = params.face;
        let lum = |x: u32, y: u32| -> f32 {
            let at = ((y * face + x) * 8) as usize;
            f32_from_half(u16::from_le_bytes([texture.bytes[at], texture.bytes[at + 1]]))
        };
        let direction = |face_index: u32, x: u32, y: u32| -> [f32; 3] {
            px_volume_schema::direction_of(
                face_index,
                (x as f32 + 0.5) / face as f32,
                (y as f32 + 0.5) / face as f32,
            )
        };
        // 面内基准：同一面里左右相邻 texel 的平均差。
        let mut interior = 0.0_f32;
        let mut interior_count = 0.0_f32;
        for face_index in 0..6_u32 {
            for y in 0..face {
                for x in 0..face - 1 {
                    interior += (lum(x + 1, y) - lum(x, y)).abs();
                    interior_count += 1.0;
                }
            }
        }
        let interior = interior / interior_count;
        // 棱上：每一面 s=0 那一列的 texel，找另一面上方向最接近的 texel。
        let mut edge = 0.0_f32;
        let mut edge_count = 0.0_f32;
        let mut worst = 0.0_f32;
        for face_index in 0..6_u32 {
            for y in 0..face {
                let here = direction(face_index, 0, y);
                let mut best = f32::MAX;
                let mut best_value = 0.0_f32;
                for other in 0..6_u32 {
                    if other == face_index {
                        continue;
                    }
                    for oy in 0..face {
                        for ox in 0..face {
                            let there = direction(other, ox, oy);
                            let dot = here[0] * there[0] + here[1] * there[1] + here[2] * there[2];
                            let angle = 1.0 - dot;
                            if angle < best {
                                best = angle;
                                best_value = lum(ox, oy);
                            }
                        }
                    }
                }
                let diff = (lum(0, y) - best_value).abs();
                edge += diff;
                edge_count += 1.0;
                worst = worst.max(diff);
            }
        }
        let edge = edge / edge_count;
        let ratio = edge / interior.max(1e-6);
        // 先看**形状**：逐面、以及棱上 t 的分布（两端 = 角点，中间 = 棱身）。
        let mut per_face = [0.0_f32; 6];
        let mut per_band = [0.0_f32; 4];
        let mut per_band_count = [0.0_f32; 4];
        for face_index in 0..6_u32 {
            for y in 0..face {
                let here = direction(face_index, 0, y);
                let mut best = f32::MAX;
                let mut best_value = 0.0_f32;
                for other in 0..6_u32 {
                    if other == face_index { continue; }
                    for oy in 0..face {
                        for ox in 0..face {
                            let there = direction(other, ox, oy);
                            let dot = here[0]*there[0] + here[1]*there[1] + here[2]*there[2];
                            if 1.0 - dot < best { best = 1.0 - dot; best_value = lum(ox, oy); }
                        }
                    }
                }
                let diff = (lum(0, y) - best_value).abs();
                per_face[face_index as usize] += diff;
                let band = ((y * 4) / face).min(3) as usize;
                per_band[band] += diff;
                per_band_count[band] += 1.0;
            }
        }
        // ⚠ 先量**角度距离**：跨棱找到的"最近格"若比面内相邻格更远，那 6 倍就只是
        //   色相过渡段把更大的角度差放大出来的，不是不连续。
        let mut edge_angle = 0.0_f32;
        let mut edge_angle_count = 0.0_f32;
        for face_index in 0..6_u32 {
            for row in 0..face {
                let here = direction(face_index, 0, row);
                let mut best = f32::MAX;
                for other in 0..6_u32 {
                    if other == face_index { continue; }
                    for oy in 0..face {
                        for ox in 0..face {
                            let there = direction(other, ox, oy);
                            let dot = here[0]*there[0] + here[1]*there[1] + here[2]*there[2];
                            if 1.0 - dot < best { best = 1.0 - dot; }
                        }
                    }
                }
                edge_angle += best;
                edge_angle_count += 1.0;
            }
        }
        // 面内相邻格的角度距离（同一面里左右相邻）。
        let a0 = direction(0, 0, 5);
        let a1 = direction(0, 1, 5);
        let interior_angle = 1.0 - (a0[0]*a1[0] + a0[1]*a1[1] + a0[2]*a1[2]);
        println!(
            "角度距离：跨棱最近格 {:.6} / 面内相邻格 {:.6} = {:.2}x",
            edge_angle / edge_angle_count,
            interior_angle,
            (edge_angle / edge_angle_count) / interior_angle.max(1e-9)
        );
        println!("逐面棱差：{:?}", per_face.map(|v| (v / face as f32 * 1000.0).round() / 1000.0));
        println!("棱上分段（0=一端 3=另一端）：{:?}", std::array::from_fn::<f32, 4, _>(|i| per_band[i] / per_band_count[i].max(1.0) * 1000.0).map(|v| (v).round() / 1000.0));
        println!("面内基准 {interior:.5}（乘 1000 后 {:.3}）", interior * 1000.0);
        assert!(
            ratio < 2.0,
            "面棱上的差 {edge:.5} 是面内基准 {interior:.5} 的 {ratio:.2} 倍（最大 {worst:.5}）\
             —— 棱两侧对不上，缝在**烘焙侧**（采样/布局/面序）"
        );
        println!(
            "px_volume_gpu_op：面棱差 {edge:.5} / 面内 {interior:.5} = {ratio:.2}x（最大 {worst:.5}）"
        );
    }
}

#[cfg(test)]
mod sampler_seam_tests {
    use super::*;
    use px_volume_schema::VolumeData;

    /// **采样器级连续性**：把"采样语义"与"光线步进"分开。
    ///
    /// 法：同一半径上取两组点，组内两点都相差**一个纹素**的角度 ——
    /// * 跨棱组：`s = +0.5/res` 与 `s = -0.5/res`（后者落到相邻面上去，
    ///   因为 `cube_direction` 是线性映射，`s` 越界就是越过棱）；
    /// * 面内组：`s = +0.5/res` 与 `s = +1.5/res`。
    /// 采样是连续的话，两组的差应当**同量级**；跨棱那组显著更大 ⇒ 缝在采样语义里。
    ///
    /// ⚠ 这条只跑 CPU 的 `sample_volume`（GPU 那份已被证明与它逐点一致到 1e-6 ⇒
    ///   同源问题两边都有，跑一边就够，也快得多）。
    #[test]
    fn the_sampler_is_continuous_across_a_face_edge() {
        let (res, layers) = (8_u32, 4_u32);
        let (inner, outer) = (1.0_f32, 2.0_f32);
        // 逐格随机（不是光滑场）：任何"取错格"都会立刻显形。
        let mut data = vec![0.0_f32; (6 * layers * res * res * LANES as u32) as usize];
        for (index, value) in data.iter_mut().enumerate() {
            *value = ((index * 2654435761usize) % 1000) as f32 / 1000.0;
        }
        let volume = VolumeData { res, layers, inner, outer, data };
        // 多半径扫描：缝若只在某些半径上出现，就能直接指到"层/高度"那一层的约定。
        let mut worst = (0.0_f32, 0.0_f32, 0.0_f32);
        for step_index in 0..16 {
            let radius = inner + (outer - inner) * (step_index as f32 + 0.5) / 16.0;
            let mut edge = 0.0_f32;
            let mut inside = 0.0_f32;
            let mut count = 0.0_f32;
            for face in 0..6_u32 {
                for t_index in 1..res - 1 {
                    let t = (t_index as f32 + 0.5) / res as f32;
                    let at = |s: f32| -> f32 {
                        let d = px_volume_schema::direction_of(face, s, t);
                        let point = [d[0] * radius, d[1] * radius, d[2] * radius];
                        px_volume_alg::sample_volume(&volume, point, 0)
                    };
                    let step = 0.5 / res as f32;
                    edge += (at(step) - at(-step)).abs();
                    inside += (at(1.0 + step) - at(step)).abs();
                    count += 1.0;
                }
            }
            let edge = edge / count;
            let inside = inside / count;
            let ratio = edge / inside.max(1e-6);
            println!("  半径 {radius:.3}（高度 {:.3}）：跨棱 {edge:.5} / 面内 {inside:.5} = {ratio:.2}x",
                (radius - inner) / (outer - inner));
            if ratio > worst.2 {
                worst = (radius, edge, ratio);
            }
        }
        println!("采样器：最差在半径 {:.3} —— {:.2}x", worst.0, worst.2);
        assert!(worst.2 < 2.0, "半径 {:.3} 上跨棱的采样差是面内的 {:.2} 倍 —— 采样语义在该处不连续", worst.0, worst.2);
    }
}

#[cfg(test)]
mod march_seam_tests {
    use super::*;
    use px_volume_schema::VolumeData;

    /// **步进级连续性**（不含分级）：CPU 的 `raymarch_channel` 出的就是辐射，没有响应曲线
    /// 也没有色相斜坡 ⇒ 拿它一比，就能把"步进"与"分级"分开。
    ///
    /// 测法：同一张天空纹理上，取每面 `x = 0`（棱）那一列的 texel，与**另一面**上方向最接近的
    /// texel 比；面内相邻 texel 的差当基准。`jitter = 0`（抖动是逐 texel 哈希，会把结构性错配淹掉）。
    #[test]
    fn the_march_is_continuous_across_a_face_edge() {
        let (res, layers) = (8_u32, 4_u32);
        let (inner, outer) = (1.0_f32, 2.0_f32);
        let mut data = vec![0.0_f32; (6 * layers * res * res * LANES as u32) as usize];
        for (index, value) in data.iter_mut().enumerate() {
            *value = ((index * 2654435761usize) % 1000) as f32 / 1000.0;
        }
        let volume = VolumeData { res, layers, inner, outer, data };
        let face = 32_u32;
        let params = px_volume_schema::params::sky::SkyParams {
            face,
            steps: 16,
            jitter: 0.0,
            star_gain: 0.0,
            star_floor: 0.9,
            ..Default::default()
        };
        let field = px_volume_alg::raymarch_channel(&volume, None, &params, 0);
        let lum = |x: u32, y: u32| -> f32 { field.at(x, y) };
        let direction = |face_index: u32, x: u32, y: u32| -> [f32; 3] {
            px_volume_schema::direction_of(
                face_index,
                (x as f32 + 0.5) / face as f32,
                (y as f32 + 0.5) / face as f32,
            )
        };
        let mut interior = 0.0_f32;
        let mut interior_count = 0.0_f32;
        for face_index in 0..6_u32 {
            for row in 0..face {
                for x in 0..face - 1 {
                    interior += (lum(x + 1, row) - lum(x, row)).abs();
                    interior_count += 1.0;
                }
            }
        }
        let interior = interior / interior_count;
        let mut edge = 0.0_f32;
        let mut count = 0.0_f32;
        let mut worst = 0.0_f32;
        for face_index in 0..6_u32 {
            for row in 0..face {
                let here = direction(face_index, 0, row);
                let mut best = f32::MAX;
                let mut best_value = 0.0_f32;
                for other in 0..6_u32 {
                    if other == face_index { continue; }
                    for oy in 0..face {
                        for ox in 0..face {
                            let there = direction(other, ox, oy);
                            let dot = here[0]*there[0] + here[1]*there[1] + here[2]*there[2];
                            if 1.0 - dot < best {
                                best = 1.0 - dot;
                                best_value = lum(ox, other * face + oy);
                            }
                        }
                    }
                }
                let diff = (lum(0, face_index * face + row) - best_value).abs();
                edge += diff;
                count += 1.0;
                worst = worst.max(diff);
            }
        }
        let edge = edge / count;
        println!(
            "步进：棱上差 {edge:.5} / 面内 {interior:.5} = {:.2}x（最大 {worst:.5}）",
            edge / interior.max(1e-6)
        );
        assert!(
            edge / interior.max(1e-6) < 2.0,
            "步进侧在棱上不连续：{:.2} 倍",
            edge / interior.max(1e-6)
        );
    }
}

#[cfg(test)]
mod identity_grade_tests {
    use super::*;
    use px_field_schema::field::{Field, Projection};
    use px_volume_schema::VolumeData;

    fn f32_from_half(bits: u16) -> f32 {
        let sign = if bits & 0x8000 != 0 { -1.0_f32 } else { 1.0 };
        let exponent = ((bits >> 10) & 0x1f) as i32;
        let mantissa = (bits & 0x3ff) as f32;
        match exponent {
            0 => sign * mantissa * 2.0_f32.powi(-24),
            31 => {
                if mantissa == 0.0 { sign * f32::INFINITY } else { f32::NAN }
            }
            _ => sign * (1.0 + mantissa / 1024.0) * 2.0_f32.powi(exponent - 15),
        }
    }

    /// **恒等分级下，辐射本身在棱上连不连续？**
    ///
    /// 把响应设成恒等（`tone_in == tone_out`、肩推到无穷）且色相强度 0 ⇒ 出来的就是**原始辐射**。
    /// 缝若消失 ⇒ 病在**分级**；若仍在 ⇒ 病在**步进**（GPU 那一侧，CPU 步进已证连续）。
    #[test]
    fn the_radiance_is_continuous_under_an_identity_grade() {
        let (res, layers) = (8_u32, 4_u32);
        let (inner, outer) = (1.0_f32, 2.0_f32);
        let mut data = vec![0.0_f32; (6 * layers * res * res * LANES as u32) as usize];
        for (index, value) in data.iter_mut().enumerate() {
            *value = ((index * 2654435761usize) % 1000) as f32 / 1000.0;
        }
        let volume = VolumeData { res, layers, inner, outer, data };
        let face = 32_u32;
        let stars_field = Field::with_projection(2, 12, vec![0.0; 24], Projection::CubeMap);
        let params = px_volume_schema::params::sky::SkyParams {
            face,
            steps: 16,
            jitter: 0.0,
            star_gain: 0.0,
            star_floor: 0.9,
            ..Default::default()
        };
        let identity = [0.01_f32, 0.1, 1.0, 10.0];
        let texture = sky(
            face,
            params.steps,
            inner,
            res,
            layers,
            inner,
            outer,
            &volume.data,
            &MarchExtras { stars: Some(&[0.0; 24]), star_face: 2, star_gain: 0.0, star_floor: 0.9, background: [0.0; 3] },
            (identity, identity, [1.0e9, 1.0e9]),
            (px_volume_alg::raymarch::RAMP_LUMA, [[1.0, 1.0, 1.0, 0.0]; 4]),
            0.0,
        )
        .expect("恒等分级下出图");
        // `sky()` 回来的是 f32 的分级结果（每 texel 三个）⇒ 直接取 R 通道。
        let lum = |x: u32, y: u32| -> f32 { texture[((y * face + x) * 3) as usize] };
        let direction = |face_index: u32, x: u32, y: u32| -> [f32; 3] {
            px_volume_schema::direction_of(
                face_index,
                (x as f32 + 0.5) / face as f32,
                (y as f32 + 0.5) / face as f32,
            )
        };
        let mut interior = 0.0_f32;
        let mut interior_count = 0.0_f32;
        for face_index in 0..6_u32 {
            for row in 0..face {
                for x in 0..face - 1 {
                    interior += (lum(x + 1, row) - lum(x, row)).abs();
                    interior_count += 1.0;
                }
            }
        }
        let interior = interior / interior_count;
        let mut edge = 0.0_f32;
        let mut count = 0.0_f32;
        for face_index in 0..6_u32 {
            for row in 0..face {
                let here = direction(face_index, 0, row);
                let mut best = f32::MAX;
                let mut best_value = 0.0_f32;
                for other in 0..6_u32 {
                    if other == face_index { continue; }
                    for oy in 0..face {
                        for ox in 0..face {
                            let there = direction(other, ox, oy);
                            let dot = here[0]*there[0] + here[1]*there[1] + here[2]*there[2];
                            if 1.0 - dot < best {
                                best = 1.0 - dot;
                                best_value = lum(ox, other * face + oy);
                            }
                        }
                    }
                }
                edge += (lum(0, face_index * face + row) - best_value).abs();
                count += 1.0;
            }
        }
        let edge = edge / count;
        println!(
            "恒等分级：棱上差 {edge:.5} / 面内 {interior:.5} = {:.2}x",
            edge / interior.max(1e-6)
        );
    }
}

#[cfg(test)]
mod chain_tests {
    use super::*;
    use px_field_schema::field::{Field, Projection};
    use px_volume_schema::VolumeData;

    const RES: u32 = 8;
    const LAYERS: u32 = 4;
    const INNER: f32 = 1.0;
    const OUTER: f32 = 2.0;
    const FACE: u32 = 32;

    /// **缝判据用的那一份夹具**（`index % 97`）。⚠ 上一轮的错误就是拿别的夹具下结论。
    fn fixture() -> VolumeData {
        let mut data = vec![0.0_f32; (6 * LAYERS * RES * RES * LANES as u32) as usize];
        for (index, value) in data.iter_mut().enumerate() {
            *value = 0.2 + 0.6 * ((index % 97) as f32 / 97.0);
        }
        VolumeData { res: RES, layers: LAYERS, inner: INNER, outer: OUTER, data }
    }

    fn direction(face_index: u32, x: u32, y: u32) -> [f32; 3] {
        px_volume_schema::direction_of(
            face_index,
            (x as f32 + 0.5) / FACE as f32,
            (y as f32 + 0.5) / FACE as f32,
        )
    }

    /// 跨棱差 / 面内差的比值（与缝判据同一套量法）。
    fn ratio_of(sample: &dyn Fn(u32, u32, u32) -> f32) -> f32 {
        let mut interior = 0.0_f32;
        let mut interior_count = 0.0_f32;
        for face_index in 0..6_u32 {
            for row in 0..FACE {
                for x in 0..FACE - 1 {
                    interior += (sample(face_index, x + 1, row) - sample(face_index, x, row)).abs();
                    interior_count += 1.0;
                }
            }
        }
        let interior = interior / interior_count;
        let mut edge = 0.0_f32;
        let mut count = 0.0_f32;
        for face_index in 0..6_u32 {
            for row in 0..FACE {
                let here = direction(face_index, 0, row);
                let mut best = f32::MAX;
                let mut best_value = 0.0_f32;
                for other in 0..6_u32 {
                    if other == face_index {
                        continue;
                    }
                    for oy in 0..FACE {
                        for ox in 0..FACE {
                            let there = direction(other, ox, oy);
                            let dot =
                                here[0] * there[0] + here[1] * there[1] + here[2] * there[2];
                            if 1.0 - dot < best {
                                best = 1.0 - dot;
                                best_value = sample(other, ox, oy);
                            }
                        }
                    }
                }
                edge += (sample(face_index, 0, row) - best_value).abs();
                count += 1.0;
            }
        }
        (edge / count) / interior.max(1e-6)
    }

    /// **同一份夹具上的二分链**：采样 -> 步进(CPU) -> 辐射(GPU) -> 整链(带分级)。
    #[test]
    fn the_seam_appears_at_one_specific_stage() {
        let volume = fixture();
        let params = px_volume_schema::params::sky::SkyParams {
            face: FACE,
            steps: 16,
            jitter: 0.0,
            star_gain: 0.0,
            star_floor: 0.9,
            ..Default::default()
        };
        // 1) 采样器（半径取壳中）
        let radius = (INNER + OUTER) * 0.5;
        let sampler = ratio_of(&|face_index, x, y| {
            let d = direction(face_index, x, y);
            px_volume_alg::sample_volume(
                &volume,
                [d[0] * radius, d[1] * radius, d[2] * radius],
                0,
            )
        });
        // 2) 步进（CPU，无分级）
        let field = px_volume_alg::raymarch_channel(&volume, None, &params, 0);
        let march = ratio_of(&|face_index, x, y| field.at(x, face_index * FACE + y));
        // 3) GPU 辐射（恒等分级）
        let identity = [0.01_f32, 0.1, 1.0, 10.0];
        let stars = vec![0.0_f32; 24];
        let radiance = sky(
            FACE,
            params.steps,
            INNER,
            RES,
            LAYERS,
            INNER,
            OUTER,
            &volume.data,
            &MarchExtras {
                stars: Some(&stars),
                star_face: 2,
                star_gain: 0.0,
                star_floor: 0.9,
                background: [0.0; 3],
            },
            (identity, identity, [1.0e9, 1.0e9]),
            (px_volume_alg::raymarch::RAMP_LUMA, [[1.0, 1.0, 1.0, 0.0]; 4]),
            0.0,
        )
        .expect("GPU 辐射");
        let gpu_radiance =
            ratio_of(&|face_index, x, y| radiance[((face_index * FACE + y) * FACE + x) as usize * 3]);
        // 4) 整链（自动分位 + 真分级）
        let stars_field =
            Field::with_projection(2, 12, stars.clone(), Projection::CubeMap);
        let texture = raymarch_sky(&volume, &stars_field, &params).expect("整链");
        let full = ratio_of(&|face_index, x, y| {
            let at = (((face_index * FACE + y) * FACE + x) * 8) as usize;
            let sign = if texture.bytes[at + 1] & 0x80 != 0 { -1.0_f32 } else { 1.0 };
            let bits = u16::from_le_bytes([texture.bytes[at], texture.bytes[at + 1]]);
            let exponent = ((bits >> 10) & 0x1f) as i32;
            let mantissa = (bits & 0x3ff) as f32;
            match exponent {
                0 => sign * mantissa * 2.0_f32.powi(-24),
                _ => sign * (1.0 + mantissa / 1024.0) * 2.0_f32.powi(exponent - 15),
            }
        });
        println!("同一夹具上的二分链（跨棱/面内）：");
        println!("  1 采样器        {sampler:.2}x");
        println!("  2 步进 CPU      {march:.2}x");
        println!("  3 GPU 辐射      {gpu_radiance:.2}x");
        println!("  4 整链（分级）  {full:.2}x");
        assert!(sampler < 2.0, "采样器这一步就 {sampler:.2}x");
    }
}

/// **GPU 版发射烘焙**：与 `px_volume_alg::bake_emission` 同一入参/产物。
///
/// ⚠ 为什么值得搬：这是体积链上最贵的一处（每体素一次阴影行进），而且逐体素独立。
///   CPU 版在 shape 192 上把核跑满还要几分钟（加了中心星团后每体素 64 步）。
#[allow(clippy::too_many_arguments)]
pub fn bake_emission(
    density: &px_volume_schema::VolumeData,
    params: &px_volume_schema::params::emission::EmissionParams,
) -> Result<px_volume_schema::VolumeData, String> {
    let Some(gpu) = connect() else {
        return Err("没有可用 GPU".to_string());
    };
    let (res, layers) = (density.res, density.layers);
    // ⚠⚠ **密度体积是单通道**（`px_protocol::art::VolumeData::at` 里有
    //   `debug_assert_eq!(lanes, 1)`），而 WGSL 那套采样器把通道数编成了常量 6
    //   （它服务的是**六通道**的发射体积）。这里把单通道补成六通道：只读 lane 0
    //   ⇒ 其余填 0。代价是临时的 6 倍内存（shape 192 下约 300 MB），换来的是
    //   **不动那份已经逐点验过的采样器** —— 这个取舍在"搬 GPU"这一轮的性价比最高。
    let samples = (6 * layers * res * res) as usize;
    let wanted = if density.data.len() == samples {
        let mut widened = vec![0.0_f32; samples * LANES];
        for (index, value) in density.data.iter().enumerate() {
            widened[index * LANES] = *value;
        }
        widened
    } else {
        density.data.clone()
    };
    let volume_uniform = [
        res.to_le_bytes(),
        layers.to_le_bytes(),
        (LANES as u32).to_le_bytes(),
        0_u32.to_le_bytes(),
        density.inner.to_le_bytes(),
        density.outer.to_le_bytes(),
        0.0_f32.to_le_bytes(),
        0.0_f32.to_le_bytes(),
    ]
    .concat();
    let uniform = [
        params.light_radius, params.shadow_gain, params.emission_power, params.emission_gain,
        params.light[0], params.light[1], params.light[2], 0.0,
        params.glow_gain, params.glow_power, params.glow_threshold, 0.0,
        params.glow_tint[0], params.glow_tint[1], params.glow_tint[2], 0.0,
        params.extinction[0], params.extinction[1], params.extinction[2], params.extinction_power,
        params.dust_bias, params.dust_threshold, 0.0, 0.0,
        params.cluster_count as f32, params.cluster_gain, params.cluster_steps as f32, params.cluster_spread,
        params.cluster_tint[0], params.cluster_tint[1], params.cluster_tint[2], 0.0,
    ]
    .iter()
    .flat_map(|value| value.to_le_bytes())
    .chain(
        [res, layers, params.shadow_steps, 0]
            .iter()
            .flat_map(|value| value.to_le_bytes()),
    )
    .collect::<Vec<u8>>();
    let voxels = (6 * layers * res * res) as usize;
    let bytes = |values: &[f32]| -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    };
    let out = px_gpu::dispatch_slots(
        gpu,
        SAMPLER_WGSL,
        "bake_emission",
        &[
            px_gpu::Slot { binding: 0, value: Binding::Uniform(&volume_uniform) },
            px_gpu::Slot { binding: 1, value: Binding::Storage(&bytes(&wanted)) },
            px_gpu::Slot { binding: 8, value: Binding::Uniform(&uniform) },
            px_gpu::Slot { binding: 9, value: Binding::Write(&vec![0_u8; voxels * 6 * 4]) },
        ],
        workgroups(voxels),
    )?;
    let data: Vec<f32> = out[0]
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect();
    Ok(px_volume_schema::VolumeData {
        res,
        layers,
        inner: density.inner,
        outer: density.outer,
        data,
    })
}

#[cfg(test)]
mod emission_tests {
    use super::*;
    use px_volume_schema::VolumeData;

    /// **发射烘焙：GPU 与 CPU 逐体素一致**（含中心星团那条路）。
    /// 小体积即可 —— 这里验的是语义，不是性能。
    #[test]
    fn the_gpu_emission_matches_the_cpu() {
        let (res, layers) = (8_u32, 4_u32);
        let (inner, outer) = (1.0_f32, 2.0_f32);
        // ⚠ **单通道**：密度体积就是这个形状（CPU 的 `at()` 会断言 lanes == 1）。
        let mut data = vec![0.0_f32; (6 * layers * res * res) as usize];
        for (index, value) in data.iter_mut().enumerate() {
            *value = ((index.wrapping_mul(2654435761)) % 1000) as f32 / 1000.0;
        }
        let density = VolumeData { res, layers, inner, outer, data };
        let params = px_volume_schema::params::emission::EmissionParams {
            light: [0.4, 0.7, -0.3],
            light_radius: 0.2,
            shadow_steps: 8,
            shadow_gain: 1.6,
            cluster_count: 3,
            cluster_gain: 0.8,
            cluster_tint: [0.72, 0.86, 1.0],
            cluster_steps: 5,
            cluster_spread: 0.35,
            ..Default::default()
        };
        let reference = px_volume_alg::bake_emission(&density, &params);
        let gpu_side = match bake_emission(&density, &params) {
            Ok(volume) => volume,
            Err(message) => panic!("GPU 发射烘焙失败：{message}"),
        };
        assert_eq!(gpu_side.data.len(), reference.data.len(), "体素数");
        let mut worst = 0.0_f32;
        let mut worst_at = 0usize;
        for (index, value) in gpu_side.data.iter().enumerate() {
            let diff = (value - reference.data[index]).abs();
            if diff > worst {
                worst = diff;
                worst_at = index;
            }
        }
        assert!(
            worst < 5e-4,
            "发射烘焙最大偏差 {worst:.6} 在第 {worst_at} 个分量（GPU {} 对 CPU {}）",
            gpu_side.data[worst_at],
            reference.data[worst_at]
        );
        println!("px_volume_gpu_op：发射烘焙最大偏差 {worst:.6}（{} 个体素 x 6）", gpu_side.data.len() / 6);
    }
}
