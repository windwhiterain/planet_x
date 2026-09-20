//! `cloud.density`：**三维场 → 可步进的密度体积**（体渲染的输入）。
//!
//! ⚠ 与 `cloud.coarse` 是两个算子（不是同一份参数的新档）：那一档服务**等值面提取**
//!   （存 `(场-τ)/L`，参数是一整套云的形状），这一档服务**沿视线的积分**
//!   （存密度本身 + 壳的内外半径 + 径向保守化）。两者的"值"含义不同。
//!
//! ⚠ 失败要**当场说清**（`bake_density` 返回 `Result`），所以这里用 `?` 把它往外传 ——
//!   `px_body!` 外面会套一层 `Ok`，直接写 `bake_density(...)` 就成了 `Ok(Result<…>)`
//!   （类型对不上，而且真出错时会被当成"成功"）。

use px_volume_schema::ops::Density;

px_graph_schema::px_body! {
    Density,
    |p, i, _g| px_volume_alg::bake_density(p, i.density.value())?
}
