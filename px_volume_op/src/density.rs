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
//!
//! ⚠⚠ **链尾的硬闸**（用户 2026-09-25："不为 0 根本就不是稀疏结构"）。
//!   实测（`px_probe` 的稀疏度判据，shape 256）：恰好 0 的体素 31.6%，而
//!   `(0, 0.01]` 的还有 **20.9%** —— 那是一片稀薄雾 ✗。它只可能来自**混合里那个
//!   fbm 操作数**：`carved`/`weight`/`extent` 都是 `remap`（`out_min = 0` ⇒ 0 进 0 出 ✓），
//!   而 **fbm 永远不精确为 0** ✗ ⇒ 线性混合必然泄漏一个 ε 级尾巴。
//!   ⇒ 稀疏是**结构**问题（"这里有没有物质"），不是数值大小问题 ⇒ 只能在链尾一刀切：
//!   `密度 < GATE ⇒ 0`，以上线性重标定（云体值域几乎不变）。放在算子这一层是因为
//!   它是链尾、跑在 CPU 上、且没有 CPU/GPU 对账的负担。
//!   ⚠ 闸门同时**省掉**一整片"稀薄气被附近星光照亮"的积分（背景发红的来源 ✓）。

use px_volume_schema::ops::Density;

/// 密度闸门：低于它一律是精确 0（`0` = 关闭，行为与从前逐位相同）。
///
/// ⚠ 取 `0.02` 的依据：非零分位的 p10 是 0.0002、p50 是 0.05 ⇒ 0.02 把那条
///   "极稀薄尾巴"切掉，而云体（p50 以上）只被线性压缩一点点。
const DENSITY_GATE: f32 = 0.02;

px_graph_schema::px_body! {
    Density,
    |p, i, g| {
        let mut volume = px_volume_alg::bake_density(p, g.width, i.density.value())?;
        if DENSITY_GATE > 0.0 {
            let span = 1.0 - DENSITY_GATE;
            for value in &mut volume.data {
                *value = if *value < DENSITY_GATE {
                    0.0
                } else {
                    (*value - DENSITY_GATE) / span
                };
            }
        }
        volume
    }
}
