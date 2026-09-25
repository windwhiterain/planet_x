//! See docs/operators.md

use px_graph_schema::PxOp;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeclFacts {
    pub interface: u64,
    pub decl_hash: &'static str,
    pub params: &'static str,
    pub inputs: &'static str,
    pub payload: &'static str,
    pub schema: &'static str,
    pub module: &'static str,
    pub type_name: &'static str,
}

pub const TABLE: &[(&str, fn() -> DeclFacts)] = &[
    ("Fbm", || facts::<px_field_schema::ops::Fbm>()),
    ("Ridged", || facts::<px_field_schema::ops::Ridged>()),
    ("Gradient", || facts::<px_field_schema::ops::Gradient>()),
    ("Warp", || facts::<px_field_schema::ops::Warp>()),
    ("Craters", || facts::<px_field_schema::ops::Craters>()),
    ("Stamps", || facts::<px_field_schema::ops::Stamps>()),
    ("FieldRemap", || facts::<px_field_schema::ops::FieldRemap>()),
    ("CloudCoarse", || {
        facts::<px_volume_schema::ops::CloudCoarse>()
    }),
    ("CubeSphere", || facts::<px_mesh_schema::ops::CubeSphere>()),
    ("Proxy", || facts::<px_mesh_schema::ops::Proxy>()),
    ("Fbm3", || facts::<px_field_schema::ops::Fbm3>()),
    ("Ridged3", || facts::<px_field_schema::ops::Ridged3>()),
    ("Warp3", || facts::<px_field_schema::ops::Warp3>()),
    ("Density", || facts::<px_volume_schema::ops::Density>()),
    ("Emission", || facts::<px_volume_schema::ops::Emission>()),
    ("SkyNebula", || facts::<px_volume_schema::ops::SkyNebula>()),
    ("Stars", || facts::<px_volume_schema::ops::Stars>()),
    ("Circle", || facts::<px_nurbs_schema::ops::Circle>()),
    ("CurveEval", || facts::<px_nurbs_schema::ops::CurveEval>()),
    ("CurveAt", || facts::<px_nurbs_schema::ops::CurveAt>()),
    ("CurveHodograph", || {
        facts::<px_nurbs_schema::ops::CurveHodograph>()
    }),
    ("CurveInsert", || {
        facts::<px_nurbs_schema::ops::CurveInsert>()
    }),
    ("CurveElevate", || {
        facts::<px_nurbs_schema::ops::CurveElevate>()
    }),
    ("CurveTessellate", || {
        facts::<px_nurbs_schema::ops::CurveTessellate>()
    }),
    ("Sphere", || facts::<px_nurbs_schema::ops::Sphere>()),
    ("SurfaceEval", || {
        facts::<px_nurbs_schema::ops::SurfaceEval>()
    }),
    ("SurfaceAt", || facts::<px_nurbs_schema::ops::SurfaceAt>()),
    ("SurfaceInsert", || {
        facts::<px_nurbs_schema::ops::SurfaceInsert>()
    }),
    ("SurfaceElevate", || {
        facts::<px_nurbs_schema::ops::SurfaceElevate>()
    }),
    ("SurfaceTessellate", || {
        facts::<px_nurbs_schema::ops::SurfaceTessellate>()
    }),
    ("SurfaceTessellateGpu", || {
        facts::<px_nurbs_schema::ops::SurfaceTessellateGpu>()
    }),
    ("CurveTessellateGpu", || {
        facts::<px_nurbs_schema::ops::CurveTessellateGpu>()
    }),
];

pub fn decl(name: &str) -> Option<DeclFacts> {
    TABLE
        .iter()
        .find(|(listed, _)| *listed == name)
        .map(|(_, facts)| facts())
}

pub fn names() -> Vec<&'static str> {
    TABLE.iter().map(|(name, _)| *name).collect()
}

pub fn entries() -> Vec<(&'static str, DeclFacts)> {
    TABLE.iter().map(|(name, facts)| (*name, facts())).collect()
}

pub const SCHEMAS: &[&str] = &[
    "px_field_schema",
    "px_volume_schema",
    "px_mesh_schema",
    "px_nurbs_schema",
];

pub fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

fn facts<O: PxOp>() -> DeclFacts {
    let path = ::core::any::type_name::<O>();
    let (schema, module) = split_schema_module(path).unwrap_or_else(|| {
        panic!("声明类型的全路径 `{path}` 不是 <crate>::<模块>::<类型> 形状（生成物写不出路径）")
    });
    let type_name = path.rsplit_once("::").map(|(_, name)| name).unwrap_or(path);
    DeclFacts {
        interface: O::interface(),
        decl_hash: O::decl_hash(),
        params: ::core::any::type_name::<O::Params>(),
        inputs: ::core::any::type_name::<O::Inputs>(),
        payload: ::core::any::type_name::<O::Payload>(),
        schema,
        module,
        type_name,
    }
}

fn split_schema_module(path: &str) -> Option<(&str, &str)> {
    let (schema, rest) = path.split_once("::")?;
    let module = match rest.rsplit_once("::") {
        Some((head, _type_name)) => head,
        None => rest,
    };
    Some((schema, module))
}
