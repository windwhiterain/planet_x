use px_graph_schema::{Cooked, px_op};
use px_protocol::art::{MeshData, PolylineData};

use crate::curve::Curve;
use crate::params;
use crate::point::PointData;
use crate::surface::Surface;

#[derive(px_derive::PxInputs)]
pub struct CurveInput {
    pub curve: Cooked<Curve>,
}

#[derive(px_derive::PxInputs)]
pub struct SurfaceInput {
    pub surface: Cooked<Surface>,
}

#[derive(px_derive::PxInputs)]
pub struct CurveAtInput {
    pub curve: Cooked<Curve>,
    pub point: Cooked<PointData>,
}

#[derive(px_derive::PxInputs)]
pub struct SurfaceAtInput {
    pub surface: Cooked<Surface>,
    pub point: Cooked<PointData>,
}

px_op! {
    Circle, "nurbs.circle", "px_nurbs_op", params::curve::CircleParams, (), Curve
}

px_op! {
    CurveEval, "nurbs.curve.eval", "px_nurbs_op", params::eval::EvalParams, CurveInput, PointData
}

px_op! {
    CurveAt, "nurbs.curve.at", "px_nurbs_op", params::eval::EvalParams, CurveAtInput, PointData
}

px_op! {
    CurveHodograph, "nurbs.curve.hodograph", "px_nurbs_op", params::eval::EvalParams, CurveInput, PointData
}

px_op! {
    CurveInsert, "nurbs.curve.insert", "px_nurbs_op", params::insert::InsertParams, CurveInput, Curve
}

px_op! {
    CurveElevate, "nurbs.curve.elevate", "px_nurbs_op", params::elevate::ElevateParams, CurveInput, Curve
}

px_op! {
    CurveTessellate, "nurbs.curve.tessellate", "px_nurbs_op", params::tessellate::TessellateParams, CurveInput, PolylineData
}

px_op! {
    SurfaceEval, "nurbs.surface.eval", "px_nurbs_op", params::eval::EvalParams, SurfaceInput, PointData
}

px_op! {
    SurfaceAt, "nurbs.surface.at", "px_nurbs_op", params::eval::EvalParams, SurfaceAtInput, PointData
}

px_op! {
    SurfaceInsert, "nurbs.surface.insert", "px_nurbs_op", params::insert::InsertParams, SurfaceInput, Surface
}

px_op! {
    SurfaceElevate, "nurbs.surface.elevate", "px_nurbs_op", params::elevate::ElevateParams, SurfaceInput, Surface
}

px_op! {
    SurfaceTessellate, "nurbs.surface.tessellate", "px_nurbs_op", params::tessellate::TessellateParams, SurfaceInput, MeshData
}

px_op! {
    Sphere, "nurbs.sphere", "px_nurbs_op", params::curve::SphereParams, (), Surface
}

px_op! {
    SurfaceTessellateGpu, "nurbs.surface.tessellate.gpu", "px_nurbs_gpu_op", params::tessellate::TessellateParams, SurfaceInput, MeshData
}

px_op! {
    CurveTessellateGpu, "nurbs.curve.tessellate.gpu", "px_nurbs_gpu_op", params::tessellate::TessellateParams, CurveInput, PolylineData
}
