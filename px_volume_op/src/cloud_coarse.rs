//! `cloud.coarse`：**覆盖度场 → 等值面提取要的体积**。
//!
//! ⚠ 存的是 `(场 - τ) / L`（服务网格），与 `cloud.density` 存的"密度本身"不是一回事
//!   —— 见 `px_volume_schema::ops` 那两条的文档。

use px_volume_schema::ops::CloudCoarse;

px_graph_schema::px_body! {
    CloudCoarse,
    |p, i| px_volume_alg::eval_sampled(p, i.coverage.value())
}
