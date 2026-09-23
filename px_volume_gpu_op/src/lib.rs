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
