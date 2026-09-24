//! `sky.nebula`：**沿视线积分**（发射体积 + 星场 → 一张天空立方贴图）。
//!
//! ⚠ 三条通道在**算子内部**各积一遍（参数里没有 `channel`）：逐通道消光意味着三张本来
//!   就不同，一次交出一张贴图才是这个算子该有的形状。
//!
//! ⚠ 输出是**贴图**（`TextureData`）⇒ 天空是一条**正常的图产物**：驱动照常进键、落盘、
//!   登记清单，场景文档按 `"图名::节点名"` 引用它当 `environment.skybox`，
//!   渲染器一个字节不用改。
//!
//! ⚠ 失败要当场说清（`raymarch_sky` 返回 `Result`）⇒ 用 `?` 往外传：
//!   `px_body!` 外面会套一层 `Ok`，直接写就成了 `Ok(Result<…>)`。

use px_volume_schema::ops::SkyNebula;

/// 这一档的实现**在 GPU 上**（`px_volume_gpu_op::raymarch_sky`）：天穹是最贵的一段，
/// 而它逐 texel 独立、正是 compute 的形状。输入/产物与从前完全一样（`VolumeData` +
/// **R3 星场** → `TextureData`）⇒ 图脚本一行不改。
fn px_volume_op_gpu(
    emission: &px_volume_schema::VolumeData,
    stars: &px_sparse::StarField,
    params: &px_volume_schema::params::sky::SkyParams,
) -> Result<px_volume_schema::TextureData, String> {
    px_volume_gpu_op::raymarch_sky(emission, stars, params)
}

px_graph_schema::px_body! {
    SkyNebula,
    |p, i| px_volume_op_gpu(i.volume.value(), i.stars.value(), p)?
}
