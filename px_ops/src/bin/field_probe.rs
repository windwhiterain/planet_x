use px_protocol::stream::{self, Frame};

fn main() {
    let path = std::env::args().nth(1).expect("用法：field_probe <FIELD.pxart>");
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
        println!("    y={y:>4}（跨 v={:.4}）：{value:.6}", (y - 1) as f32 / (height - 1) as f32);
    }
    println!("  列间 |Δ| 最大的 3 列：");
    for (value, x) in column_gaps.iter().take(3) {
        println!("    x={x:>4}：{value:.6}");
    }
    let median = row_gaps[row_gaps.len() / 2].0;
    println!("  行间 |Δ| 中位数 = {median:.6}");
}
