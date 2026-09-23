//! `cloud.emission`：**密度体积 → 逐体素的发射与消光**。
//!
//! ⚠ 光深要按**世界点**采样（`sample_world`），不能拿体素格点凑：格点上的 τ 有台阶，
//!   而台阶在画面上就是一圈圈等值线。这一份只是把"按体素算一遍"接到声明上。
//!
//! ⚠⚠ 2026-09-25 晚：**星光照气体那一笔恢复了**（提交 `216c3de` 删过一版）。用户的验收口径是
//!   "星云不能有自发光，它的全部亮度必须来自星光被气体散射" ⇒ 这一档重新吃一份星场
//!   （`i.stars`），依赖方向回到 `density → stars → emission → sky`。
//!   ⚠ 星场吃的是**密度**（不是发射）：发射是六通道交错，而星那一侧的 `sample_world`
//!   按单通道索引 ⇒ 喂发射会读到错位数据（踩过的坑）。
//!   ⚠ 三处语义调整（见 `px_volume_alg::emission`）：散射项**无色**（逐通道加的是同一个
//!   `visible`，不带 `star.tint`）、整体乘 `glow_tint[channel]`（红）、保留逐星
//!   `exp(-τ × shadow_gain)` 遮挡。

use px_volume_schema::ops::Emission;

/// ⚠ 这一档现在跑在 **GPU** 上（`px_volume_gpu_op::bake_emission`）：逐体素的阴影行进
/// 是体积链上最贵的一处，而它逐体素独立 —— 正是 compute 的形状。入参/产物与 CPU 版一致
/// （判据 `the_gpu_emission_matches_the_cpu` 逐体素对到 0.000000），图脚本不改。
fn px_volume_op_gpu_emission(
    density: &px_volume_schema::VolumeData,
    stars: &px_sparse::StarField,
    params: &px_volume_schema::params::emission::EmissionParams,
) -> Result<px_volume_schema::VolumeData, String> {
    px_volume_gpu_op::bake_emission(density, stars, params)
}

px_graph_schema::px_body! {
    Emission,
    |p, i, _g| px_volume_op_gpu_emission(i.volume.value(), i.stars.value(), p)?
}
