//! `cloud.density`：**三维场 → 可步进的密度体积**（体渲染的输入）。
//!
//! ⚠ 与 `cloud.coarse` 是两个算子（不是同一份参数的新档）：那一档服务**等值面提取**
//!   （存 `(场-τ)/L`，参数是一整套云的形状），这一档服务**沿视线的积分**
//!   （存密度本身 + 壳的内外半径 + 径向保守化）。两者的"值"含义不同。
//!
//! ⚠ 体网格的分辨率从**画布宽度**取（`DensityParams::shape_of`）：上游那张三维场是照
//!   画布造的，参数再写一遍 `res` 只会多一个"两处必须一致"的地方。网格 `g` 是算子拿得到
//!   的唯一"画布"来源。
//!
//! ⚠ 失败要**当场说清**（`bake_density` 返回 `Result`），所以这里用 `?` 把它往外传 ——
//!   `px_body!` 外面会套一层 `Ok`，直接写 `bake_density(...)` 就成了 `Ok(Result<…>)`
//!   （类型对不上，而且真出错时会被当成"成功"）。

use px_volume_schema::ops::Density;

px_graph_schema::px_body! {
    Density,
    |p, i, g| px_volume_alg::bake_density(p, g.width, i.density.value())?
}
