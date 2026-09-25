use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use px_graph_schema::PxOp;
use px_graph_schema::ops;

const DECLARATION_TABLE: &str = "px_decls/src/lib.rs";

fn load<O: PxOp>(loaded: &mut BTreeSet<&'static str>) {
    let name = type_name_of::<O>();
    ops::body::<O>().unwrap_or_else(|err| panic!("`{}` ({name}) does not load: {err}", O::ID));
    loaded.insert(name);
}

fn exclude<O: PxOp>(excluded: &mut BTreeSet<&'static str>) {
    let name = type_name_of::<O>();
    assert!(
        ops::body::<O>().is_err(),
        "`{name}` loads from its library — it must not be in the exclusion list",
    );
    excluded.insert(name);
}

fn type_name_of<O: PxOp>() -> &'static str {
    ::core::any::type_name::<O>()
        .rsplit("::")
        .next()
        .unwrap_or_default()
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("px_graphs must live under the workspace")
        .to_path_buf()
}

fn declaration_table_names() -> BTreeSet<String> {
    let path = workspace_root().join(DECLARATION_TABLE);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("cannot read {}: {err}", path.display()));
    let table = text
        .split_once("pub const TABLE")
        .map(|(_, rest)| rest)
        .unwrap_or_else(|| panic!("{} has no `pub const TABLE`", path.display()));
    let rows = table
        .split_once("\n];")
        .map(|(rows, _)| rows)
        .unwrap_or(table);
    let mut names = BTreeSet::new();
    for row in rows.lines() {
        let Some(rest) = row.trim_start().strip_prefix("(\"") else {
            continue;
        };
        let Some(close) = rest.find('"') else {
            continue;
        };
        names.insert(rest[..close].to_string());
    }
    names
}

#[test]
fn every_declared_operator_loads_from_its_library() {
    let mut loaded = BTreeSet::new();
    load::<px_field_schema::ops::Fbm>(&mut loaded);
    load::<px_field_schema::ops::Ridged>(&mut loaded);
    load::<px_field_schema::ops::Gradient>(&mut loaded);
    load::<px_field_schema::ops::Warp>(&mut loaded);
    load::<px_field_schema::ops::Craters>(&mut loaded);
    load::<px_field_schema::ops::Stamps>(&mut loaded);
    load::<px_volume_schema::ops::CloudCoarse>(&mut loaded);
    load::<px_mesh_schema::ops::CubeSphere>(&mut loaded);
    load::<px_mesh_schema::ops::Proxy>(&mut loaded);
    load::<px_field_schema::ops::Fbm3>(&mut loaded);
    load::<px_field_schema::ops::Ridged3>(&mut loaded);
    load::<px_field_schema::ops::Warp3>(&mut loaded);
    load::<px_volume_schema::ops::Density>(&mut loaded);
    load::<px_volume_schema::ops::Emission>(&mut loaded);
    load::<px_volume_schema::ops::SkyNebula>(&mut loaded);
    load::<px_volume_schema::ops::Stars>(&mut loaded);
    load::<px_nurbs_schema::ops::Circle>(&mut loaded);
    load::<px_nurbs_schema::ops::CurveEval>(&mut loaded);
    load::<px_nurbs_schema::ops::CurveAt>(&mut loaded);
    load::<px_nurbs_schema::ops::CurveHodograph>(&mut loaded);
    load::<px_nurbs_schema::ops::CurveInsert>(&mut loaded);
    load::<px_nurbs_schema::ops::CurveElevate>(&mut loaded);
    load::<px_nurbs_schema::ops::CurveTessellate>(&mut loaded);
    load::<px_nurbs_schema::ops::Sphere>(&mut loaded);
    load::<px_nurbs_schema::ops::SurfaceEval>(&mut loaded);
    load::<px_nurbs_schema::ops::SurfaceAt>(&mut loaded);
    load::<px_nurbs_schema::ops::SurfaceInsert>(&mut loaded);
    load::<px_nurbs_schema::ops::SurfaceElevate>(&mut loaded);
    load::<px_nurbs_schema::ops::SurfaceTessellate>(&mut loaded);
    load::<px_nurbs_schema::ops::SurfaceTessellateGpu>(&mut loaded);
    load::<px_nurbs_schema::ops::CurveTessellateGpu>(&mut loaded);

    let mut excluded = BTreeSet::new();
    exclude::<px_field_schema::ops::FieldRemap>(&mut excluded);

    let declared = declaration_table_names();
    for name in &declared {
        assert!(
            loaded.contains(name.as_str()) || excluded.contains(name.as_str()),
            "declaration `{name}` is neither loaded nor in the counted exclusion list — \
             a declaration must not slip through this gate",
        );
    }
    assert_eq!(
        loaded.len() + excluded.len(),
        declared.len(),
        "loaded {} + excluded {} != declared {} (`{DECLARATION_TABLE}`)",
        loaded.len(),
        excluded.len(),
        declared.len(),
    );
}

#[test]
fn every_library_reports_its_own_source_hash() {
    for lib in [
        "px_field_op",
        "px_volume_op",
        "px_mesh_op",
        "px_nurbs_op",
        "px_nurbs_gpu_op",
    ] {
        let hash = match lib {
            "px_field_op" => <px_field_schema::ops::Fbm as PxOp>::source_hash(),
            "px_volume_op" => <px_volume_schema::ops::CloudCoarse as PxOp>::source_hash(),
            "px_nurbs_op" => <px_nurbs_schema::ops::Circle as PxOp>::source_hash(),
            "px_nurbs_gpu_op" => {
                <px_nurbs_schema::ops::SurfaceTessellateGpu as PxOp>::source_hash()
            }
            _ => <px_mesh_schema::ops::Proxy as PxOp>::source_hash(),
        }
        .unwrap_or_else(|err| panic!("{lib} 的身份读不到：{err}"));
        assert_eq!(hash.len(), 64, "{lib} 的指纹应当是 64 位十六进制：{hash}");
        assert!(
            hash.chars().all(|c| c.is_ascii_hexdigit()),
            "{lib} 的指纹不是十六进制：{hash}"
        );
    }
}

#[test]
fn the_five_libraries_have_distinct_identities() {
    let names = [
        "px_field_op",
        "px_volume_op",
        "px_mesh_op",
        "px_nurbs_op",
        "px_nurbs_gpu_op",
    ];
    let mut hashes = Vec::new();
    for name in names {
        hashes.push((
            name,
            ops::source_hash(name).unwrap_or_else(|err| panic!("{name}：{err}")),
        ));
    }
    for (index, (name, hash)) in hashes.iter().enumerate() {
        for (other_name, other) in &hashes[index + 1..] {
            assert_ne!(
                hash, other,
                "{name} 与 {other_name} 的身份相同 —— 指纹没覆盖到各自的源码"
            );
        }
    }
}
