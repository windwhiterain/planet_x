use std::time::Instant;

use px_ops::noise::{self, FbmSettings};

fn main() {
    let settings = FbmSettings {
        frequency: 0.55,
        octaves: 6,
        lacunarity: 2.0,
        gain: 0.5,
        seed: 7,
    };

    println!("六阶 fbm 3D，值噪声，单线程");
    for size in [64_u32, 96, 128, 160] {
        let started = Instant::now();
        let mut sum = 0.0_f32;
        let mut peak = f32::NEG_INFINITY;
        for z in 0..size {
            for y in 0..size {
                for x in 0..size {
                    let point = [
                        x as f32 / (size - 1) as f32 * 2.0 - 1.0,
                        y as f32 / (size - 1) as f32 * 2.0 - 1.0,
                        z as f32 / (size - 1) as f32 * 2.0 - 1.0,
                    ];
                    let value = noise::fbm_3(point, &settings);
                    sum += value;
                    peak = peak.max(value);
                }
            }
        }
        let millis = started.elapsed().as_millis();
        let voxels = (size as f64).powi(3);
        println!(
            "  {size}³：{millis:>6} ms｜{:>6.1} MB f32｜均值 {:.4}｜峰值 {:.4}",
            voxels * 4.0 / 1e6,
            sum / voxels as f32,
            peak,
        );
    }

    let started = Instant::now();
    let mut sum = 0.0_f32;
    for y in 0..192_u32 {
        for x in 0..384_u32 {
            let direction = noise::direction(
                x as f32 / 384.0,
                1.0 - y as f32 / 191.0,
            );
            sum += noise::fbm_3(direction, &settings);
        }
    }
    println!(
        "  对照：现在这条 384×192 的球面烘焙 {} ms（均值 {:.4}）",
        started.elapsed().as_millis(),
        sum / (384.0 * 192.0),
    );
}
