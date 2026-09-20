//! **一张场产物的读数探针**：接缝、行/列梯度，以及"**这张场有多少是纯纬度的函数**"。
//!
//! 用法：`field_probe <FIELD.pxart> [投影]`（投影默认 `cube_map`，取值同
//! `Projection` 的名字：`equirect` / `octahedral` / `cube` / `cube_map`）。
//!
//! ⚠ 最后那一条读数是 2026-09-20 加的，起因是用户对气态巨行星的一句评价：
//!   **"你的那个像个西瓜"**（条带是纯纬度的函数）。"不规则"这件事在那之前只能靠眼睛看，
//!   于是把它变成一个数：
//!     * **纬向可解释度 `R²`** = 按纬度分箱之后"箱间方差 / 总方差"。纯纬度的函数 ⇒ 接近 1；
//!       越不规则越低。⚠ 它**不是**"好看"的判据，只是"像不像西瓜"的判据。
//!     * **经/纬梯度比** = 沿经度的平均 |Δ| ÷ 沿纬度的平均 |Δ|。西瓜的经向梯度接近 0，
//!       比值很小；条带被扭曲之后两者可比。
//!   ⚠ 读数只对**球面投影**（`cube_map` / `equirect` / `octahedral`）有意义：它们才有"纬度"。

use px_field_schema::field::{Field, Projection};
use px_protocol::stream::{self, Frame};

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("用法：field_probe <FIELD.pxart> [投影]");
    let projection = match std::env::args().nth(2).as_deref() {
        None | Some("cube_map") => Projection::CubeMap,
        Some("cube") => Projection::Cube,
        Some("equirect") => Projection::Equirect,
        Some("octahedral") => Projection::Octahedral,
        Some(other) => panic!("不认识的投影 `{other}`"),
    };
    let bytes = std::fs::read(&path).expect("读不到文件");
    let frames = stream::read_stream(&mut bytes.as_slice()).expect("解流失败");
    let blob = frames
        .iter()
        .find_map(|frame| match frame {
            Frame::Blob(blob) => Some(blob),
            _ => None,
        })
        .expect("没有数据块");

    let height = blob.header.shape[0];
    let width = blob.header.shape[1];
    let data = blob.f32s().expect("不是 f32 数据");

    let at = |x: u32, y: u32| data[(y * width + x) as usize];

    let mut row_gaps: Vec<(f32, u32)> = Vec::new();
    for y in 1..height {
        let mut sum = 0.0_f64;
        for x in 0..width {
            sum += (at(x, y) - at(x, y - 1)).abs() as f64;
        }
        row_gaps.push(((sum / width as f64) as f32, y));
    }
    row_gaps.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());

    let mut column_gaps: Vec<(f32, u32)> = Vec::new();
    for x in 1..width {
        let mut sum = 0.0_f64;
        for y in 0..height {
            sum += (at(x, y) - at(x - 1, y)).abs() as f64;
        }
        column_gaps.push(((sum / height as f64) as f32, x));
    }
    column_gaps.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());

    let wrap = (0..height)
        .map(|y| (at(0, y) - at(width - 1, y)).abs())
        .sum::<f32>()
        / height as f32;

    println!("{path}：{width}×{height}");
    println!("  经度环绕接缝（列 0 vs 列 W-1）平均 |Δ| = {wrap:.6}");
    println!("  行间 |Δ| 最大的 6 行（相邻两行高度差的平均）：");
    for (value, y) in row_gaps.iter().take(6) {
        println!(
            "    y={y:>4}（跨 v={:.4}）：{value:.6}",
            (y - 1) as f32 / (height - 1) as f32
        );
    }
    println!("  列间 |Δ| 最大的 3 列：");
    for (value, x) in column_gaps.iter().take(3) {
        println!("    x={x:>4}：{value:.6}");
    }
    let median = row_gaps[row_gaps.len() / 2].0;
    println!("  行间 |Δ| 中位数 = {median:.6}");

    report_band_structure(&path, width, height, &data, projection);
}

/// **这张场有多少是纯纬度的函数**（"西瓜度"）—— 见文件头那一段。
fn report_band_structure(
    path: &str,
    width: u32,
    height: u32,
    data: &[f32],
    projection: Projection,
) {
    let mut field = Field::new(width, height, data.to_vec());
    field.projection = projection;

    const BINS: usize = 48;
    let mut count = vec![0.0_f64; BINS];
    let mut sum = vec![0.0_f64; BINS];
    let mut total = 0.0_f64;
    let mut total_sq = 0.0_f64;
    let mut along_lon = 0.0_f64;
    let mut along_lat = 0.0_f64;
    let mut samples = 0.0_f64;
    for y in 0..height {
        for x in 0..width {
            let value = field.at(x, y) as f64;
            let latitude = field.direction(x, y)[1].clamp(-1.0, 1.0) as f64;
            let bin = ((latitude + 1.0) * 0.5 * (BINS as f64 - 1.0)).round() as usize;
            let bin = bin.min(BINS - 1);
            count[bin] += 1.0;
            sum[bin] += value;
            total += value;
            total_sq += value * value;
            if x > 0 {
                along_lon += (value - field.at(x - 1, y) as f64).abs();
            }
            if y > 0 {
                along_lat += (value - field.at(x, y - 1) as f64).abs();
            }
            samples += 1.0;
        }
    }
    let mean = total / samples;
    let variance = (total_sq / samples) - mean * mean;
    let mut between = 0.0_f64;
    for bin in 0..BINS {
        if count[bin] > 0.0 {
            let bin_mean = sum[bin] / count[bin];
            between += count[bin] * (bin_mean - mean) * (bin_mean - mean);
        }
    }
    let r2 = if variance > 0.0 {
        (between / samples) / variance
    } else {
        1.0
    };
    let lon = along_lon / (samples - height as f64).max(1.0);
    let lat = along_lat / (samples - width as f64).max(1.0);
    println!("  纬向可解释度 R²（西瓜度）= {r2:.4}（1 = 纯纬度的函数）");
    println!(
        "  经/纬梯度比 = {:.3}（沿经度 {lon:.5} ÷ 沿纬度 {lat:.5}）",
        if lat > 0.0 { lon / lat } else { 0.0 }
    );
    println!("  （读数属于 {path}，投影 {projection:?}）");
}
