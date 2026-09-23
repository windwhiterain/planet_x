//! `cloud.emission`：**密度体积 → 逐体素的发射与消光**。
//!
//! ⚠ 光深要按**世界点**采样（`sample_world`），不能拿体素格点凑：格点上的 τ 有台阶，
//!   而台阶在画面上就是一圈圈等值线。这一份只是把"按体素算一遍"接到声明上。
//!
//! ⚠⚠ 2026-09-25：**星光照气体那一笔删掉了**（用户："想当然的非物理元素，散射已经包含"）。
//!   它曾经吃一份星场（`i.stars`）；删掉之后这一档只吃密度 ⇒ 依赖方向变成
//!   `density → emission → stars → sky`（星场反过来吃发射，见 `px_volume_op::stars`）。

use px_volume_schema::ops::Emission;

/// ⚠ 这一档现在跑在 **GPU** 上（`px_volume_gpu_op::bake_emission`）：逐体素的阴影行进
/// 是体积链上最贵的一处，而它逐体素独立 —— 正是 compute 的形状。入参/产物与 CPU 版一致
/// （判据 `the_gpu_emission_matches_the_cpu` 逐体素对到 0.000000），图脚本不改。
fn px_volume_op_gpu_emission(
    density: &px_volume_schema::VolumeData,
    params: &px_volume_schema::params::emission::EmissionParams,
) -> Result<px_volume_schema::VolumeData, String> {
    px_volume_gpu_op::bake_emission(density, params)
}

px_graph_schema::px_body! {
    Emission,
    |p, i, _g| px_volume_op_gpu_emission(i.volume.value(), p)?
}
