//! 气态巨行星：**纬向条带**（图侧实例 `field.remap/latbands`）+ 湍流挪相位 + 风暴椭圆
//! ⇒ **一张条带场**（`CubeMap`）。
//!
//! ⚠ 这张图**不产云**（2026-09-20 用户口径：「气态巨星应当用次表面散射」）：气态巨行星不是
//!   "一颗石头外面糊一层云的壳"，而是**一整球的气体** —— 渲染侧的材质是 `art/shaders/gasgiant.wgsl`
//!   （次表面散射：穿透深度 → 每通道吸收 → 包裹漫反射 → 边缘散射），而这张图烘的是那份材质
//!   要吃的**条带场**：哪一条带更厚、更深。
//!   ⇒ 场 →（`px_graph::generate::field_cube`）→ 立方贴图 → 材质 `@binding(7)`。
//!   所以这里没有 `coarse` / `proxy` / `slope_*`：那几样是"云壳"那条路的东西，没有消费者。
//!
//! 两条不同的装载路在这张图里并存（与 `field_remap` 一样，但这里是**真管线**而不是最小例子）：
//!
//! * `field.fbm` / `field.craters` / `field.remap` 走**预置实现库**（`px_field_op`，按身份装载）；
//! * `bands` 走**泛型实例库**（`target/pcg/inst/<key>.dll`，`art/inst/latbands.rs` 里那段图侧函数）。
//!
//! ⚠ **纬度只能从 `direction` 取**：这张图的画布是 `CubeMap`（六张面沿 `y` 叠成一条），
//!   `uv[1]` 在面与面之间会跳变。`FieldFn::value` 的球面方向与节点参数两栏都是 2026-09-20
//!   才补上的 —— 在那之前这一张图写不出来（也没有"参数真的生效"这回事）。
//!
//! 渲染：`px run scene orbit-gasgiant`（配方在 `art/scene/orbit-gasgiant.toml`）。

use px_cook::{Domain, GraphSpec, begin, cameras, cook, field};
use px_field_schema::field::cube_map_extent;

use px_graphs::insts::LatBands;

const FACE: u32 = 256;

type Fault = Box<dyn std::error::Error>;

fn main() -> Result<(), Fault> {
    let (width, height) = cube_map_extent(FACE);
    let graph = begin(GraphSpec {
        name: "gasgiant".to_string(),
        width,
        height,
        projection: Domain::CubeMap,
        cameras: cameras::review(),
    });

    // 湍流：给条带边界"挪相位"的那一张场（球面 fbm）。
    let turbulence = cook::<field::Fbm>(&graph, "turbulence", ())?;
    // 主角：纬向条带（**实例库**）。`bands` 参数就是"几圈条带"。
    let bands = cook::<LatBands>(
        &graph,
        "bands",
        field::FieldRemapInput {
            input: turbulence.clone(),
        },
    )?;
    // 收口：**软**条带（宽过渡、低对比）—— 锐化是材质那一侧的事（`band_contrast`），
    // 因为"看多软"是观感参数，改它不该重烘这张图。
    //
    // ⚠ 这里**故意没有** `field.craters`：第一版拿它当"风暴椭圆"（低频浅坑），出来的是一圈圈
    //   细而硬的环（陨坑缘那一条半正弦），在球面上看着像**海岸线**而不是涡旋 —— 实测图
    //   `target/shot-gasgiant-craters.png`。气态巨行星的涡旋不是"撞出来的坑"，
    //   它的边界由湍流相位（`bands` 实例里那一项）给，这里就不叠异物了。
    let mixed = cook::<field::Remap>(
        &graph,
        "mixed",
        field::FieldInput {
            field: bands.clone(),
        },
    )?;

    let stats = mixed.value().stats();
    println!(
        "输出 mixed：{}×{}（{} 面 × {FACE}²）｜值域 {:.4}..{:.4}｜均值 {:.4}",
        mixed.value().width,
        mixed.value().height,
        mixed.value().height / FACE,
        stats.min,
        stats.max,
        stats.mean,
    );
    println!(
        "  ⚠ 这张图**不产云**：它的消费者是次表面散射材质（`art/shaders/gasgiant.wgsl`）—— \
         场 → 立方贴图（`generate::field_cube`）→ 材质 @binding(7)"
    );
    println!("看这一份内容：px run scene orbit-gasgiant（配方 art/scene/orbit-gasgiant.toml）");

    graph.finish();
    Ok(())
}
