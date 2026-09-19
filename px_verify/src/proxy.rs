//! 云硬表面代理的**参照场**：体积算子怎么烘、判据仪器怎么量，走的都是这一条。
//!
//! ⚠ 它住在 `px_verify` 而不是算子里，理由与 §28.2 那条一样：**参照实现与算子必须同一份**，
//! 而且判据那一侧（`px_graphs` 的仪器）不许静态依赖算子（那会把算子链进图程序 exe）。
//! 于是这份映射谁都能用：算子在 dylib 里用它烘，仪器在图脚本里用它判。

use px_field_schema::field::{Field, normalize, tangent_frame};
use px_volume_schema::{FieldKind, PATCHES, Params, direction_of};

use crate::cloud_field::CloudFieldParams;

/// 体积参数 → 参照场的参数。
///
/// 噪声三件套（`detail_scale`/`seed`）在粗场里用不上：噪声取常数 1.0
/// ⇒ `mix(1-detail_strength, 1, 1) = 1`，`ceiling` 只剩 `top`。
pub fn from_volume(params: &Params) -> CloudFieldParams {
    CloudFieldParams {
        orientation: params.orientation,
        inner: params.inner,
        outer: params.outer,
        coverage: params.coverage,
        base: params.base,
        top: params.top,
        detail_scale: 16.0,
        detail_strength: 0.55,
        erode: params.erode,
        taper: params.taper,
        coverage_gain: params.coverage_gain,
        seed: 7,
    }
}

/// 某个世界方向上的覆盖度（shader 的 `coverage_of`：先转进局部系、采覆盖图、再重映射）。
/// 局部系的旋转放在这里，采样和判据就都走同一条路。
pub fn cover_at(cloud: &CloudFieldParams, coverage: &Field, direction: [f32; 3]) -> f32 {
    let local = cloud.to_local(direction);
    cloud.cover_from_mask(coverage.sample_direction(local))
}

/// 粗场：噪声取它的上界 1.0。壳外一律 0（与 shader 的早退逐位一致）。
pub fn coarse_at(cloud: &CloudFieldParams, cover: f32, altitude: f32) -> f32 {
    if altitude < 0.0 || altitude > 1.0 || cover <= 0.0 {
        return 0.0;
    }
    cloud.shape(cover, altitude, 1.0)
}

/// 真场：噪声取 shader 的 `billows`。`direction` 是世界方向，这里自己转局部 ——
/// shader 的 `medium.direction` 也是局部方向，两者必须同一个系，否则噪声不是那一份。
pub fn final_at(cloud: &CloudFieldParams, cover: f32, direction: [f32; 3], altitude: f32) -> f32 {
    if altitude < 0.0 || altitude > 1.0 || cover <= 0.0 {
        return 0.0;
    }
    let local = cloud.to_local(direction);
    cloud.shape(cover, altitude, cloud.billows(local, altitude))
}

/// 按 `params.field` 取场值：烘、判据仪器、量 `L` 都走这一条，不各抄一份。
pub fn field_at(
    cloud: &CloudFieldParams,
    params: &Params,
    cover: f32,
    direction: [f32; 3],
    altitude: f32,
) -> f32 {
    match params.field {
        FieldKind::Coarse => coarse_at(cloud, cover, altitude),
        FieldKind::Final => final_at(cloud, cover, direction, altitude),
    }
}

/// 最粗的那一格的切向宽度（世界单位）。立方球的参数化在面心拉伸、在角上压缩，
/// 取面内各处的最大值 ⇒ 保守。
pub fn tangential_cell(params: &Params) -> f32 {
    let res = params.res.max(2);
    let step = 1.0 / (res - 1) as f32;
    let mut worst = 0.0_f32;
    for face in 0..PATCHES {
        for t in 0..=16 {
            for s in 0..=16 {
                let u = s as f32 / 16.0;
                let v = t as f32 / 16.0;
                let here = direction_of(face, u, v);
                for (du, dv) in [(step, 0.0), (0.0, step)] {
                    let there = direction_of(face, u + du, v + dv);
                    let far = ((there[0] - here[0]).powi(2)
                        + (there[1] - here[1]).powi(2)
                        + (there[2] - here[2]).powi(2))
                    .sqrt()
                        * params.outer;
                    worst = worst.max(far);
                }
            }
        }
    }
    worst
}

/// 归一化方向（切向邻域撒点要用）。
pub fn unit(vector: [f32; 3]) -> [f32; 3] {
    normalize(vector)
}

/// 一支切向标架（切向邻域撒点要用）。
pub fn tangent(direction: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    tangent_frame(direction)
}
