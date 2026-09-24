//! NURBS 算子的**声明**：身份 / 超参数 / 图参数形状 / 输出载荷。
//!
//! ⚠ **一行实现都没有**：算法在 `px_nurbs_op` 里（dylib，运行时按身份装载）。
//!   这里只说「一个算子是什么、吃什么、吐什么」——于是图侧接错一个上游、少给一个字段，
//!   都是**编译错**，而且编译这一份不需要实现库在场。
//!
//! ⚠ 图参数的形状（`CurveInput { curve }` / `CurvePointInput { curve, params }`）
//!   **是接口的一部分**，所以它住在声明旁边。

use px_graph_schema::{Cooked, px_op};
use px_protocol::art::MeshData;

use crate::curve::Curve;
use crate::params;
use crate::point::PointData;
use crate::surface::Surface;

/// 一条曲线的输入。
#[derive(px_derive::PxInputs)]
pub struct CurveInput {
    pub curve: Cooked<Curve>,
}

/// 一张曲面的输入。
#[derive(px_derive::PxInputs)]
pub struct SurfaceInput {
    pub surface: Cooked<Surface>,
}

/// **曲线 + 一份参数**（`CurveAt`：`t` 由上油的 `PointData` 给 ⇒ 在哪求值可以由别的节点算）。
#[derive(px_derive::PxInputs)]
pub struct CurveAtInput {
    pub curve: Cooked<Curve>,
    pub point: Cooked<PointData>,
}

/// **曲面 + 一份参数**（`SurfaceAt`：`(u, v)` 由上游的 `PointData` 给）。
#[derive(px_derive::PxInputs)]
pub struct SurfaceAtInput {
    pub surface: Cooked<Surface>,
    pub point: Cooked<PointData>,
}

px_op! {
    /// **整圆 → 一条有理二次 NURBS 曲线**（精确圆，不是拟合）。
    Circle, "nurbs.circle", "px_nurbs_op", params::curve::CircleParams, (), Curve
}

px_op! {
    /// **曲线求值**：`t → 点`（可选一阶导）。`t` 来自参数（参数文件的 `u`）。
    CurveEval, "nurbs.curve.eval", "px_nurbs_op", params::eval::EvalParams, CurveInput, PointData
}

px_op! {
    /// **曲线在别的节点给的参数上求值**：`(曲线, 参数) → 点`。
    CurveAt, "nurbs.curve.at", "px_nurbs_op", params::eval::EvalParams, CurveAtInput, PointData
}

px_op! {
    /// **曲线的一阶导（hodograph）**：`t → 切向量`。
    CurveHodograph, "nurbs.curve.hodograph", "px_nurbs_op", params::eval::EvalParams, CurveInput, PointData
}

px_op! {
    /// **插入节点**：几何一个字不变，只多控制点（下游要更多自由度时用）。
    CurveInsert, "nurbs.curve.insert", "px_nurbs_op", params::insert::InsertParams, CurveInput, Curve
}

px_op! {
    /// **升阶**：几何一个字不变，次数变高。
    CurveElevate, "nurbs.curve.elevate", "px_nurbs_op", params::elevate::ElevateParams, CurveInput, Curve
}

px_op! {
    /// **曲线细分**：按弦误差摊成一条折线（`MeshData`，落在它自己那张平面里）。
    CurveTessellate, "nurbs.curve.tessellate", "px_nurbs_op", params::tessellate::TessellateParams, CurveInput, MeshData
}

px_op! {
    /// **曲面求值**：`(u, v) → 点`（外加两个偏导与单位法线）。`(u, v)` 来自参数。
    SurfaceEval, "nurbs.surface.eval", "px_nurbs_op", params::eval::EvalParams, SurfaceInput, PointData
}

px_op! {
    /// **曲面在别的节点给的参数上求值**：`(曲面, 参数) → 点`。
    SurfaceAt, "nurbs.surface.at", "px_nurbs_op", params::eval::EvalParams, SurfaceAtInput, PointData
}

px_op! {
    /// **插入节点**（曲面）：沿 u 或 v 插，几何一个字不变。
    SurfaceInsert, "nurbs.surface.insert", "px_nurbs_op", params::insert::InsertParams, SurfaceInput, Surface
}

px_op! {
    /// **升阶**（曲面）：两向一起升到目标次数，几何一个字不变。
    SurfaceElevate, "nurbs.surface.elevate", "px_nurbs_op", params::elevate::ElevateParams, SurfaceInput, Surface
}

px_op! {
    /// **曲面细分**：按弦误差摊成三角网格（法线来自曲面本身，不是三角形平均）。
    SurfaceTessellate, "nurbs.surface.tessellate", "px_nurbs_op", params::tessellate::TessellateParams, SurfaceInput, MeshData
}

px_op! {
    /// **球面 → 一张有理二次 NURBS 曲面**（精确球面，不是细分逼近）。
    Sphere, "nurbs.sphere", "px_nurbs_op", params::curve::SphereParams, (), Surface
}
