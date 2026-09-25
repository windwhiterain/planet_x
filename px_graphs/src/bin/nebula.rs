use std::time::Instant;

use px_cook::{
    Domain, GraphSpec, begin, cached, field, field_params, node_params, volume, volume_params,
};
use px_field_schema::field::Field;
use px_field_schema::volume::VolumeShape;
use px_graphs::elem;
use px_volume_schema::params::stars::StarsParams;

type Fault = Box<dyn std::error::Error>;

fn face_from_args() -> u32 {
    let args: Vec<String> = px_cook::args_without_store().unwrap_or_default();
    let mut face = 64_u32;
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--face" {
            if let Some(value) = args.get(index + 1).and_then(|text| text.parse().ok()) {
                face = value;
            }
        }
        index += 1;
    }
    face.max(8)
}

fn shape_from_args() -> u32 {
    let args: Vec<String> = px_cook::args_without_store().unwrap_or_default();
    let mut shape = 64_u32;
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--shape" {
            if let Some(value) = args.get(index + 1).and_then(|text| text.parse().ok()) {
                shape = value;
            }
        }
        index += 1;
    }
    shape.max(8)
}

fn layers_from_args() -> Option<u32> {
    let args: Vec<String> = px_cook::args_without_store().unwrap_or_default();
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--layers" {
            if let Some(value) = args
                .get(index + 1)
                .and_then(|text| text.parse::<u32>().ok())
            {
                return Some(value.max(8));
            }
        }
        index += 1;
    }
    None
}

fn volume_layers() -> Result<u32, Fault> {
    let path = px_graph::workspace_root()
        .join("art")
        .join("nebula")
        .join("density_volume.toml");
    let text = std::fs::read_to_string(&path)
        .map_err(|err| format!("读不到 {}：{err}", path.display()))?;
    let params: px_volume_schema::params::density::DensityParams =
        toml::from_str(&text).map_err(|err| format!("{} 解不开：{err}", path.display()))?;
    Ok(params.layers)
}

fn star_params() -> Result<StarsParams, Fault> {
    let path = px_graph::workspace_root()
        .join("art")
        .join("nebulasky")
        .join("stars.toml");
    let text = std::fs::read_to_string(&path)
        .map_err(|err| format!("读不到 {}：{err}", path.display()))?;
    Ok(toml::from_str(&text).map_err(|err| format!("{} 解不开：{err}", path.display()))?)
}

fn report(name: &str, field: &Field) {
    let stats = field.stats();
    println!(
        "  {name}：{}×{}｜值域 {:.4}..{:.4}｜均值 {:.4}",
        field.width, field.height, stats.min, stats.max, stats.mean
    );
}

fn main() -> Result<(), Fault> {
    px_graphs::insts::gate("nebula").expect("实例库不齐 ⇒ 先 `px build`（stage 1 的正规命令）");
    px_cook::apply_store_args()?;
    let face = face_from_args();
    let shape = shape_from_args();
    let started = Instant::now();

    let layers = match layers_from_args() {
        Some(given) => given,
        None => volume_layers()?,
    };
    println!("形状 {shape} × {shape} × {layers} 层 ｜ 天空面 {face}");
    let shape_graph = begin(GraphSpec {
        name: "nebula".to_string(),
    });

    let volume_shape = VolumeShape { res: shape, layers };
    let field_shape = field_params::Shape {
        width: volume_shape.res,
        height: volume_shape.height(),
        projection: Domain::Volume,
    };

    let blobs = cached(
        &shape_graph,
        "blobs",
        field::Fbm3,
        field_params::Fbm3Params {
            shape: field_shape,
            ..node_params(&shape_graph, "blobs")?
        },
        (),
    )?;
    let wisps = cached(
        &shape_graph,
        "wisps",
        field::Ridged3,
        field_params::Ridged3Params {
            shape: field_shape,
            ..node_params(&shape_graph, "wisps")?
        },
        (),
    )?;
    let flow = cached(
        &shape_graph,
        "flow",
        field::Fbm3,
        field_params::Fbm3Params {
            shape: field_shape,
            ..node_params(&shape_graph, "flow")?
        },
        (),
    )?;
    let flow_second = cached(
        &shape_graph,
        "flow_second",
        field::Fbm3,
        field_params::Fbm3Params {
            shape: field_shape,
            ..node_params(&shape_graph, "flow_second")?
        },
        (),
    )?;
    let flow_third = cached(
        &shape_graph,
        "flow_third",
        field::Fbm3,
        field_params::Fbm3Params {
            shape: field_shape,
            ..node_params(&shape_graph, "flow_third")?
        },
        (),
    )?;

    let warped = cached(
        &shape_graph,
        "warped",
        field::Warp3,
        node_params(&shape_graph, "warped")?,
        field::Warp3Input {
            field: blobs,
            offset_a: flow,
            offset_b: flow_second,
            offset_c: flow_third,
        },
    )?;
    let extent = cached(
        &shape_graph,
        "extent",
        elem::Remap,
        node_params(&shape_graph, "extent")?,
        elem::RemapInput {
            field: warped.clone(),
        },
    )?;
    let weight = cached(
        &shape_graph,
        "weight",
        elem::Remap,
        node_params(&shape_graph, "weight")?,
        elem::RemapInput {
            field: warped.clone(),
        },
    )?;
    let density = cached(
        &shape_graph,
        "density",
        elem::Mix,
        node_params(&shape_graph, "density")?,
        elem::MixInput {
            a: warped,
            b: wisps.clone(),
            mask: weight,
        },
    )?;
    let vacuum = cached(
        &shape_graph,
        "vacuum",
        elem::Constant,
        elem::ConstantParams {
            shape: field_shape,
            ..node_params(&shape_graph, "vacuum")?
        },
        (),
    )?;
    let shaped = cached(
        &shape_graph,
        "shaped",
        elem::Mix,
        node_params(&shape_graph, "shaped")?,
        elem::MixInput {
            a: vacuum.clone(),
            b: density,
            mask: extent,
        },
    )?;
    report("shaped", shaped.value());

    let envelope = cached(
        &shape_graph,
        "envelope",
        field::Fbm3,
        field_params::Fbm3Params {
            shape: field_shape,
            ..node_params(&shape_graph, "envelope")?
        },
        (),
    )?;
    let envelope_mask = cached(
        &shape_graph,
        "envelope_mask",
        elem::Remap,
        node_params(&shape_graph, "envelope_mask")?,
        elem::RemapInput { field: envelope },
    )?;
    let shaped2 = cached(
        &shape_graph,
        "shaped2",
        elem::Mix,
        node_params(&shape_graph, "shaped2")?,
        elem::MixInput {
            a: vacuum.clone(),
            b: shaped,
            mask: envelope_mask,
        },
    )?;
    report("shaped2", shaped2.value());

    let carved = cached(
        &shape_graph,
        "carved",
        elem::Remap,
        node_params(&shape_graph, "carved")?,
        elem::RemapInput { field: wisps },
    )?;
    let textured = cached(
        &shape_graph,
        "textured",
        elem::Mix,
        node_params(&shape_graph, "textured")?,
        elem::MixInput {
            a: shaped2,
            b: vacuum,
            mask: carved,
        },
    )?;
    report("textured", textured.value());

    let density_volume = cached(
        &shape_graph,
        "density_volume",
        volume::Density,
        volume_params::density::DensityParams {
            res: shape,
            ..node_params(&shape_graph, "density_volume")?
        },
        volume::DensityInput { density: textured },
    )?;
    let shape_stars = cached(
        &shape_graph,
        "stars",
        volume::Stars,
        star_params()?,
        volume::StarsInput {
            volume: density_volume.clone(),
        },
    )?;
    let emission = cached(
        &shape_graph,
        "emission",
        volume::Emission,
        node_params(&shape_graph, "emission")?,
        volume::EmissionInput {
            volume: density_volume.clone(),
            stars: shape_stars.clone(),
        },
    )?;

    let sky_graph = begin(GraphSpec {
        name: "nebulasky".to_string(),
    });
    let sky = cached(
        &sky_graph,
        "sky",
        volume::SkyNebula,
        node_params(&sky_graph, "sky")?,
        volume::SkyInput {
            volume: emission,
            stars: shape_stars,
        },
    )?;
    println!(
        "天空贴图：{}",
        px_graph_schema::payload::Build::detail(sky.value())
    );

    shape_graph.finish();
    sky_graph.finish();
    println!("共 {:.1} 秒", started.elapsed().as_secs_f64());
    Ok(())
}
