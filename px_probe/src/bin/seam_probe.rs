//! **跨面折痕（C¹ 折痕）的单点探针**：取一份烘好的体网格 `.pxart`，在每条面棱的两侧量
//! **同一个物理切向**的斜率，看它跨棱跳不跳 —— 折痕的定义就是"值连续、斜率不连续"。
//!
//! 用法：
//!
//! ```text
//! cargo run --release -p px_probe --bin seam_probe -- <VOLUME.pxart> [lane] [条数]
//! ```
//!
//! ⚠ 两侧必须用**同一个三维切向**去差分：两面的面内参数标度不一样（`cube_direction` 里
//!   `u` 方向是 `2/major`，而 `major` 沿棱在变），拿参数差分直接相减量到的是标度差。
//!   做法：在本面棱上取一点，算出"朝面内"的三维单位方向，**两个面都用它**去采样。
//!
//! ⚠ 为什么 C⁰ 判据抓不到它：`the_baked_sky_is_continuous_across_face_edges` 量的是**值**，
//!   而逐格随机夹具下共享角点让两侧插值逐位相同 ⇒ 值天生连续（实测跨棱/面内 1.06x）。

use px_protocol::art::{AssetKind, cube_direction};
use px_protocol::stream::{self, Frame};
use px_volume_schema::VolumeData;

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn norm(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

fn scale(a: [f32; 3], k: f32) -> [f32; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}

/// 面内参数 → 单位方向。
fn direction(face: u32, u: f32, v: f32) -> [f32; 3] {
    cube_direction(face, u, v)
}

/// 在 `(face, u, v)` 处**沿棱**（沿 `v`）走的三维单位切向；`u` 端点上不能沿 `u` 走。
fn along_edge_tangent(face: u32, u: f32, v: f32, h: f32) -> [f32; 3] {
    let back = direction(face, u, (v - h).clamp(0.0, 1.0));
    let fwd = direction(face, u, (v + h).clamp(0.0, 1.0));
    let step = sub(fwd, back);
    scale(step, 1.0 / norm(step).max(1e-20))
}

/// 在棱点处**朝面内**的三维单位切向：`u` 朝面内走（`v` 不动）。
fn inward_tangent(face: u32, edge: f32, v: f32, h: f32) -> [f32; 3] {
    let du = if edge < 0.5 { 1.0 } else { -1.0 };
    let du = if (edge + du * h).clamp(0.0, 1.0) != edge + du * h {
        -du
    } else {
        du
    };
    let base = direction(face, edge, v);
    let moved = direction(face, edge + du * h, v);
    let step = sub(moved, base);
    scale(step, 1.0 / norm(step).max(1e-20))
}

/// 把单位方向沿切向挪 `step`，再归一化回单位方向。
fn offset_dir(base: [f32; 3], tangent: [f32; 3], step: f32) -> [f32; 3] {
    let d = [
        base[0] + tangent[0] * step,
        base[1] + tangent[1] * step,
        base[2] + tangent[2] * step,
    ];
    let n = norm(d).max(1e-20);
    [d[0] / n, d[1] / n, d[2] / n]
}

/// 在这一侧**沿三维切向**的采样值：`k` 是以弧长为单位的偏移（相对半径）。
fn sample_along(
    volume: &VolumeData,
    base: [f32; 3],
    tangent: [f32; 3],
    k: f32,
    radius: f32,
    lane: usize,
) -> f32 {
    let d = [
        base[0] + tangent[0] * k,
        base[1] + tangent[1] * k,
        base[2] + tangent[2] * k,
    ];
    let n = norm(d).max(1e-20);
    let p = [d[0] / n * radius, d[1] / n * radius, d[2] / n * radius];
    px_volume_alg::sample_volume(volume, p, lane)
}

/// 四阶单侧差分（`h` 与 `h/2` 各一次再 Richardson），`h` 是**弧长**步长。
fn slope4(
    volume: &VolumeData,
    base: [f32; 3],
    tangent: [f32; 3],
    h: f32,
    radius: f32,
    lane: usize,
) -> f32 {
    let d = |step: f32| {
        let f = |k: f32| sample_along(volume, base, tangent, k * step, radius, lane);
        let (f0, f1, f2) = (f(1.0), f(2.0), f(3.0));
        (-3.0 * f0 + 4.0 * f1 - f2) / (2.0 * step)
    };
    (4.0 * d(h * 0.5) - d(h)) / 3.0
}

/// `face` 的 `u = edge`（`v` 固定）这条棱的伙伴面 + 伙伴面上**物理重合**的参数点。
fn partner(face: u32, edge: f32, v: f32) -> Option<(u32, f32, f32, f32)> {
    let here = direction(face, edge, v);
    let mut best = (f32::MAX, None, 0.0_f32, 0.0_f32);
    for other in 0..6u32 {
        if other == face {
            continue;
        }
        for k in 0..=4000 {
            let t = k as f32 / 4000.0;
            for &u in &[0.0_f32, 1.0] {
                let gap = 1.0 - dot(here, direction(other, u, t));
                if gap < best.0 {
                    best = (gap, Some(other), u, t);
                }
            }
        }
    }
    let (gap, other, u, t) = best;
    // 棱上两侧应当逐位同值（实测最大方向间隙 1.19e-7）；差一个纹素就不算棱。
    if gap < 1e-6 {
        Some((other?, u, t, gap))
    } else {
        None
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .expect("用法：seam_probe <VOLUME.pxart> [lane] [条数]");
    let lane: usize = args.get(2).map(|v| v.parse().unwrap()).unwrap_or(0);
    let rows: usize = args.get(3).map(|v| v.parse().unwrap()).unwrap_or(9);

    let bytes = std::fs::read(path).expect("读不到文件");
    let frames = stream::read_stream(&mut bytes.as_slice()).expect("解流失败");
    let mut params = std::collections::BTreeMap::new();
    for frame in &frames {
        if let Frame::Art(bundle) = frame {
            for asset in &bundle.assets {
                if asset.kind == AssetKind::Volume {
                    params = asset.params.clone();
                }
            }
        }
    }
    let blob = frames
        .iter()
        .find_map(|frame| match frame {
            Frame::Blob(blob) => Some(blob),
            _ => None,
        })
        .expect("没有数据块");
    let res = params["res"] as u32;
    let layers = params["layers"] as u32;
    let inner = params["inner"] as f32;
    let outer = params["outer"] as f32;
    let data = blob.f32s().expect("不是 f32 数据");
    let volume = VolumeData {
        lanes: 6,
        res,
        layers,
        inner,
        outer,
        data,
    };
    println!(
        "体积 {res}×{layers}×{res}×6，壳 {inner}..{outer}，lane {lane}，通道数 {}",
        volume.lanes()
    );

    // 弧长步长：面内一个纹素 ≈ (2/res)/√3 的弧度（面心处 major≈1）。取一个纹素量级。
    let h = 1.0 / (res as f32 * 4.0);
    println!("\n== 同一物理切向的斜率：棱两侧（C¹） ==");
    println!(
        "弧长步长 h = {h:.6}（约 {:.2} 个纹素）",
        h * res as f32 / 2.0 * 3.0_f32.sqrt()
    );
    let radii = [
        inner + (outer - inner) * 0.2,
        inner + (outer - inner) * 0.5,
        inner + (outer - inner) * 0.8,
    ];
    for radius in radii {
        let mut worst = (0.0_f32, String::new());
        let mut sum_ratio = 0.0_f32;
        let mut sum_count = 0.0_f32;
        for face in 0..6u32 {
            for edge in [0.0_f32, 1.0_f32] {
                let mut jump = 0.0_f32;
                let mut rate = 0.0_f32;
                let mut count = 0.0_f32;
                for row in 0..rows {
                    let v = 0.2 + 0.6 * row as f32 / (rows.max(2) - 1) as f32;
                    // ⚠ 两侧可比的那个量是**沿棱**的斜率：两个面的"朝面内"方向在棱上互相垂直
                    //   （本面朝内 = 伙伴面的沿棱），所以拿"朝面内"的两侧相减没有意义。
                    //   沿棱的三维方向只有一个 ⇒ 两侧量的是同一个物理量。
                    let Some((other, pu, pv, _)) = partner(face, edge, v) else {
                        continue;
                    };
                    let base_a = direction(face, edge, v);
                    let base_b = direction(other, pu, pv);
                    let along_a = along_edge_tangent(face, edge, v, h);
                    let along_b = along_edge_tangent(other, pu, pv, h);
                    if dot(along_a, along_b).abs() < 0.999 {
                        continue; // 两侧的"沿棱"方向没对上（角点附近），不算
                    }
                    let sa = slope4(&volume, base_a, along_a, h, radius, lane);
                    let sb = slope4(&volume, base_b, along_b, h, radius, lane);
                    // 分母：面内、离棱三个纹素处的**同一个沿棱方向**的斜率
                    let far = 6.0 * h;
                    let inner_tan = inward_tangent(face, edge, v, h);
                    let mid = offset_dir(base_a, inner_tan, far);
                    let sc = slope4(&volume, mid, along_a, h, radius, lane);
                    jump += (sb - sa).abs();
                    rate += (sc - sa).abs() / far;
                    count += 1.0;
                }
                if count == 0.0 {
                    println!("  r={radius:.3} 面{face} u={edge}: 没有可用的对照点");
                    continue;
                }
                let jump = jump / count;
                let rate = rate / count;
                let ratio = jump / rate.max(1e-9);
                sum_ratio += ratio;
                sum_count += 1.0;
                println!(
                    "  r={radius:.3} 面{face} u={edge}: 斜率跳变 {jump:.5} ｜ 面内斜率变化率 {rate:.4} ｜ 比值 {ratio:.2}"
                );
                if ratio > worst.0 {
                    worst = (ratio, format!("r={radius:.3} 面{face} u={edge}"));
                }
            }
        }
        println!(
            "  ⇒ r={radius:.3} 平均比值 {:.2}，最差 {:.2}（{}）\n",
            sum_ratio / sum_count.max(1.0),
            worst.0,
            worst.1,
        );
    }
}
