//! 场算子的**声明**：身份 / 超参数 / 图参数形状 / 输出域。
//!
//! ⚠ **一行实现都没有**：算法在 `px_field_alg`（rlib）里、由 `px_field_op`（dylib）链进去，
//!   运行时按身份装载。这里只说「一个算子是什么、吃什么、吐什么」——于是图侧接错一个上游、
//!   少给一个字段，都是**编译错**，而且编译这一份不需要实现库在场。
//!
//! ⚠ 末尾那条 [`FieldRemap`] 是**泛型实例的声明**（实例复用它）：它甚至没有预置实现，
//!   "这一格的值怎么算"由 `art/inst/*.rs` 里那段图侧现写的函数决定。
//!
//! ⚠ 图参数的形状（`MixInput { a, b, mask }`）**是接口的一部分**，所以它住在声明旁边：
//!   它是"这个算子被接的那个 struct"，不是实现细节。

use px_graph_schema::{Cooked, px_op};

use crate::field::Field;
use crate::params;

/// 一张场（`Remap` / `Gradient` 吃它）。
#[derive(px_derive::PxInputs)]
pub struct FieldInput {
    pub field: Cooked<Field>,
}

/// 两张场（`Warp` 吃它：待扭曲的场 + 偏移场）。
#[derive(px_derive::PxInputs)]
pub struct FieldPairInput {
    pub field: Cooked<Field>,
    pub offset: Cooked<Field>,
}

/// 三张场（`Mix` 吃它：两张待混 + 一张权重）。
#[derive(px_derive::PxInputs)]
pub struct MixInput {
    pub a: Cooked<Field>,
    pub b: Cooked<Field>,
    pub mask: Cooked<Field>,
}

/// **泛型实例**的图参数：上游那一张场。
///
/// ⚠ 它**自己的输入类型**（`FieldRemapInput`，不是 `FieldInput`）：`px_inst!` 复用的那条声明
///   要求 `Inputs` 独立 —— 于是"这一档吃的是什么"不会跟着 `FieldInput` 的将来一起漂。
#[derive(px_derive::PxInputs)]
pub struct FieldRemapInput {
    pub input: Cooked<Field>,
}

/// 打坑（`Craters` 吃它）：要被打坑的那张地形。
///
/// ⚠ 名字是 `base` 而不是 `field`：这个算子的语义是"**在**这张场上打坑"（输出 = 输入 + 坑的
///   剖面），不是"把它当成采样源"。字段名就是它吃的东西的名字。
#[derive(px_derive::PxInputs)]
pub struct CratersInput {
    pub base: Cooked<Field>,
}

/// 体网格上的域扭曲（`Warp3` 吃它）：待扭曲的场 + **三个轴各一张偏移场**。
///
/// ⚠ 为什么是三张而不是一张：位移是一个三维向量，而一张场一格只有一个数。
///   拿一张场的同一个数挪三个轴 ⇒ 位移落在体素空间的那条对角线上（图形上是"错切"，
///   不是扭曲）；而要在一张场里塞三个分量，就得再造一种"三通道场"——
///   那比多接两个上游节点贵得多，也更难在图侧读。
#[derive(px_derive::PxInputs)]
pub struct Warp3Input {
    pub field: Cooked<Field>,
    pub offset_a: Cooked<Field>,
    pub offset_b: Cooked<Field>,
    pub offset_c: Cooked<Field>,
}

// ⚠ 不吃上游的那四个 ⇒ 形状是 `()`（它没有名字问题，住在契约里）。
px_op! {
    /// 处处同一个值的场（当权重/常量用）。
    Constant, "field.constant", "px_field_op", params::constant::Params, (), Field
}

px_op! {
    /// 分形布朗噪声。
    Fbm, "field.fbm", "px_field_op", params::fbm::Params, (), Field
}

px_op! {
    /// 脊状噪声（山脊线是等值面）。
    Ridged, "field.ridged", "px_field_op", params::ridged::Params, (), Field
}

px_op! {
    /// 值域重映射（可平滑）。
    Remap, "field.remap", "px_field_op", params::remap::Params, FieldInput, Field
}

px_op! {
    /// 切向梯度的一个分量（法线/坡度用）。
    Gradient, "field.gradient", "px_field_op", params::gradient::Params, FieldInput, Field
}

px_op! {
    /// 按权重混两张场。
    Mix, "field.mix", "px_field_op", params::mix::Params, MixInput, Field
}

px_op! {
    /// 用偏移场扭曲采样方向（域扭曲）。
    Warp, "field.warp", "px_field_op", params::warp::Params, FieldPairInput, Field
}

px_op! {
    /// 上游场 + **图侧函数** ⇒ 新场（**泛型实例的声明**：`px_inst!` 复用它）。
    ///
    /// ⚠ 这一条**没有预置实现**：`"px_field_op"` 只是载荷签名里的域那一栏（`px_op!` 的形状
    ///   要求它），而 `px_field_op` 里**没有**这个算子的一行 `px_body!`。它存在的意义是给
    ///   图侧那些实例当**声明**用 —— 实例库由 `px build` 按 key 生成并编译，`LIB` 是空串
    ///   （`px_inst!` 给的），装载的是 `target/pcg/inst/<key>.dll`。
    ///
    /// ⚠ 参数是 [`params::RemapParams`] —— 与预置的 [`Remap`]（`params::remap::Params`）
    ///   **不是**同一个东西：见那一份的文档注释。
    /// ⚠ 那三栏**真的到得了图侧函数**（`px_field_alg::field_fn::FieldFn::value` 的第一栏）：
    ///   2026-09-20 之前它们只进键、不进计算，见那一条的文档。
    FieldRemap, "field.remap", "px_field_op", params::RemapParams, FieldRemapInput, Field
}

px_op! {
    /// **陨坑**：在一张地形场上按格点撒坑（坑里凹、坑缘凸）—— 无大气天体的主角。
    ///
    /// ⚠ 它**吃上游**（`CratersInput { base }`）而输出是"输入 + 坑的剖面"：地形艺术里
    ///   "基底 + 叠一层细节"是**加法**，而值域里的加法在这个词汇表里只有这一条路
    ///   （`field.mix` 是插值，插不出"坑缘高过基底"）。要叠几层就接几个 `Craters` 节点，
    ///   每个节点自己那份 TOML 给频率与深度。
    /// ⚠ 球面档按 `direction` 取格点 ⇒ 没有接缝（`uv` 不是球面坐标，见
    ///   `px_field_alg::field_fn::FieldFn` 那一条）。
    Craters, "field.craters", "px_field_op", params::CratersParams, CratersInput, Field
}

px_op! {
    /// **盖章式打坑**：每个印章有自己的**随机半径与年龄**，按年龄序"挖掘"（碗里清掉旧地形、
    /// 再堆坑缘），密度受**上游场当遮罩** —— 见 `params::StampsParams` 的文档（三条机制）。
    ///
    /// ⚠ 与 `field.craters` 是**两个算子**（不是同一份参数的新档）：`craters` 的元胞距离语义
    ///   （每格一个、半径常数、同层不重叠）与这里完全不同 ⇒ 各自留一份，谁也别想悄悄换掉谁。
    ///   同一张图里两个都能用（大盆地用 `craters`、表面麻点用 `stamps` 也成立）。
    Stamps, "field.stamps", "px_field_op", params::StampsParams, CratersInput, Field
}

// ---------------------------------------------------------------------------
// 体网格（`Domain::Volume`）上的三个算子
//
// ⚠ **它们是独立的算子，不是"给上面那几个加一档"**（用户 2026-09-20 的口径：选 B，
//   让 `Domain::Volume` 复用整套场算法）。理由有二：
//     1. 采样点完全不同：球面档按 `direction`（没有径向），体网格按 `(s, t, altitude)`。
//        合成一个算子就得在**每一个**格子上分支，而那是热循环里的分支。
//     2. 既有产物一个字节都不许动：把分支塞进 `Fbm` 就等于让它的实现换一份（键也换），
//        而那会重算全仓所有球面噪声 —— 为了一个新功能付这个代价不值。
//   ⇒ 复用的是**数据与布局**（`Field` + `Domain::Volume` + `Field::at` / `set`），
//     也就是"整套场算法能用同一张网格"这件事；算子各留一份。
// ---------------------------------------------------------------------------

px_op! {
    /// 体网格上的分形噪声（`params::Fbm3Params`）。
    Fbm3, "field.fbm3", "px_field_op", params::Fbm3Params, (), Field
}

px_op! {
    /// 体网格上的脊状噪声（`params::Ridged3Params`）：星云的丝。
    Ridged3, "field.ridged3", "px_field_op", params::Ridged3Params, (), Field
}

px_op! {
    /// 体网格上的域扭曲（`params::Warp3Params`）：按偏移场挪采样点。
    ///
    /// ⚠ 偏移场是一张**同形状的体网格**，它的三个通道由三个上游节点给
    ///   （`offset_a` / `offset_b` / `offset_c` 各来自一个不同种子的 `field.fbm3`）
    ///   —— 见 `Warp3Input`。三个轴必须**互不相关**：只给一张偏移场再用同一个值挪三个轴，
    ///   位移就落在一条对角线上（图形上表现为"沿一个方向的错切"，不是扭曲）。
    Warp3, "field.warp3", "px_field_op", params::Warp3Params, Warp3Input, Field
}

px_op! {
    /// **球面上的星点**（`params::StarsParams`）：稀疏亮点，出的是**场**。
    ///
    /// ⚠ 与 `px_graph::generate::texture::stars`（星空**贴图**）不是一回事：星点要参与
    ///   体渲染的积分（被气遮住、被尘埃染红）⇒ 必须是场。那一份出的是贴图字节。
    Stars, "field.stars", "px_field_op", params::StarsParams, (), Field
}
