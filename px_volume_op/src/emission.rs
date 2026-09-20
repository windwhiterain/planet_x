//! `cloud.emission`：**密度体积 → 逐体素的发射与消光**。
//!
//! ⚠ 光深要按**世界点**采样（`sample_world`），不能拿体素格点凑：格点上的 τ 有台阶，
//!   而台阶在画面上就是一圈圈等值线。这一份只是把"按体素算一遍"接到声明上。

use px_volume_schema::ops::Emission;

px_graph_schema::px_body! {
    Emission,
    |p, i, _g| px_volume_alg::bake_emission(i.volume.value(), p)
}
