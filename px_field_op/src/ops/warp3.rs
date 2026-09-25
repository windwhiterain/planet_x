use px_field_schema::field::Field;
use px_field_schema::ops::Warp3;
use px_field_schema::params;
use px_field_schema::volume::{VolumeShape, local_voxel_of};

px_graph_schema::px_body! { Warp3, |p, i| crate::ops::warp3::eval(
    p,
    &[
        i.field.value(),
        i.offset_a.value(),
        i.offset_b.value(),
        i.offset_c.value(),
    ],
) }

pub fn eval(params: &params::Warp3Params, inputs: &[&Field]) -> Field {
    let [source, offset_a, offset_b, offset_c] = inputs else {
        panic!(
            "field.warp3 要 4 张上游场（待扭曲的场 + 三个轴的偏移场），拿到 {}",
            inputs.len()
        );
    };
    let shape = VolumeShape::of_field(source).unwrap_or_else(|| {
        panic!(
            "field.warp3 要一张体网格（域 volume、行数 = res × layers × 6），\
             拿到的是 {:?} {}×{}",
            source.projection, source.width, source.height
        )
    });
    for (name, field) in [
        ("field", source),
        ("offset_a", offset_a),
        ("offset_b", offset_b),
        ("offset_c", offset_c),
    ] {
        assert!(
            shape.matches(field),
            "field.warp3 的 {name} 不是这个形状的体网格：{}×{} / {:?}（要 {}×{} / volume）",
            field.width,
            field.height,
            field.projection,
            shape.res,
            shape.height(),
        );
    }

    let center = |field: &Field| field.stats().mean;
    let center = [center(offset_a), center(offset_b), center(offset_c)];
    let axial = params.axial.clamp(0.0, 1.0);
    let weight = [axial, axial, 1.0];

    let mut field = source.like(0.0);
    for y in 0..field.height {
        let (face, _) = shape.slot_of(y).expect("行号在形状之内");
        for x in 0..field.width {
            let offsets = [
                offset_a.at(x, y) - center[0],
                offset_b.at(x, y) - center[1],
                offset_c.at(x, y) - center[2],
            ];
            let base = local_voxel_of(&shape, face, x, y);
            let mut shifted = [0.0_f32; 3];
            for axis in 0..3 {
                shifted[axis] = base[axis] + offsets[axis] * params.strength * weight[axis];
            }
            field.set(x, y, sample_voxel(source, &shape, face, shifted));
        }
    }
    field
}

fn sample_voxel(source: &Field, shape: &VolumeShape, face: u32, voxel: [f32; 3]) -> f32 {
    let width = shape.res.max(1);
    let last_layer = shape.layers.max(2) - 1;
    let last_cell = width.max(2) - 1;

    let sx = voxel[0] * width as f32 - 0.5;
    let sy = voxel[1] * width as f32 - 0.5;
    let snap = |fraction: f32| {
        if fraction < 1e-4 {
            0.0
        } else if fraction > 1.0 - 1e-4 {
            1.0
        } else {
            fraction
        }
    };
    let x0 = sx.floor();
    let y0 = sy.floor();
    let tx = snap(sx - x0);
    let ty = snap(sy - y0);

    let sz = voxel[2] * last_layer as f32;
    let nearest = sz.round();
    let layer0 = if (sz - nearest).abs() < 1e-3 {
        nearest
    } else {
        sz.floor()
    };
    let tz = snap(sz - layer0);

    let clamp_cell = |value: f32| value.clamp(0.0, last_cell as f32) as u32;
    let (xa, xb) = (clamp_cell(x0), clamp_cell(x0 + 1.0));
    let (ya, yb) = (clamp_cell(y0), clamp_cell(y0 + 1.0));
    let layer_at = |step: f32| {
        let layer = (layer0 + step).clamp(0.0, last_layer as f32) as u32;
        shape.row_of(face, layer)
    };
    let (za, zb) = (layer_at(0.0), layer_at(1.0));

    let corner = |cell_x: u32, t: u32, row: u32| source.at(cell_x, row + t);
    let top = (corner(xa, ya, za) * (1.0 - tx) + corner(xb, ya, za) * tx) * (1.0 - ty)
        + (corner(xa, yb, za) * (1.0 - tx) + corner(xb, yb, za) * tx) * ty;
    let bottom = (corner(xa, ya, zb) * (1.0 - tx) + corner(xb, ya, zb) * tx) * (1.0 - ty)
        + (corner(xa, yb, zb) * (1.0 - tx) + corner(xb, yb, zb) * tx) * ty;
    top * (1.0 - tz) + bottom * tz
}

#[cfg(test)]
mod tests {
    use super::*;
    use px_field_schema::field::CUBE_FACES;
    use px_field_schema::field::Projection;
    use px_field_schema::params::Shape;

    fn shape_of(res: u32, layers: u32) -> Shape {
        Shape {
            width: res,
            height: res * layers * CUBE_FACES,
            projection: Projection::Volume,
        }
    }

    fn fbm3(shape: Shape, seed: u32) -> Field {
        crate::ops::fbm3::eval(
            &params::Fbm3Params {
                seed,
                shape,
                ..Default::default()
            },
            &[],
        )
    }

    fn source_field(shape: VolumeShape) -> Field {
        fbm3(
            shape_of(shape.res, shape.layers),
            params::Fbm3Params::default().seed,
        )
    }

    fn sheet(res: u32, layers: u32, value: f32) -> Field {
        Field::filled_with(
            res,
            px_protocol_volume_height(res, layers),
            value,
            Projection::Volume,
        )
    }

    fn px_protocol_volume_height(res: u32, layers: u32) -> u32 {
        res * layers * CUBE_FACES
    }

    #[test]
    fn the_sampler_returns_the_texel_it_was_pointed_at() {
        let shape = VolumeShape { res: 8, layers: 4 };
        let source = source_field(shape);
        let mut worst = 0.0_f32;
        let mut worst_at = (0_u32, 0_u32, 0_u32, 0.0, 0.0);
        for face in 0..CUBE_FACES {
            for layer in 0..shape.layers {
                let row = shape.row_of(face, layer);
                for x in 0..shape.res {
                    let voxel = local_voxel_of(&shape, face, x, row);
                    let sampled = sample_voxel(&source, &shape, face, voxel);
                    let expected = source.at(x, row);
                    if (sampled - expected).abs() > worst {
                        worst = (sampled - expected).abs();
                        worst_at = (face, layer, x, sampled, expected);
                    }
                }
            }
        }
        assert!(
            worst < 1e-6,
            "格心采样最大偏差 {worst}（面 {} 层 {} x {}：采样 {:.6} 格值 {:.6}）",
            worst_at.0,
            worst_at.1,
            worst_at.2,
            worst_at.3,
            worst_at.4
        );
    }

    #[test]
    fn zero_strength_returns_the_source_point_by_point() {
        let shape = VolumeShape { res: 8, layers: 4 };
        let source = source_field(shape);
        let flat = sheet(shape.res, shape.layers, 0.5);
        let warped = eval(
            &params::Warp3Params {
                strength: 0.0,
                axial: 1.0,
            },
            &[&source, &flat, &flat, &flat],
        );
        let worst = source
            .data
            .iter()
            .zip(warped.data.iter())
            .map(|(one, two)| (one - two).abs())
            .fold(0.0_f32, f32::max);
        assert!(worst < 1e-6, "strength = 0 时最大偏差 {worst}");
    }

    #[test]
    fn a_constant_offset_field_moves_nothing() {
        let shape = VolumeShape { res: 8, layers: 4 };
        let source = source_field(shape);
        let flat = sheet(shape.res, shape.layers, 0.5);
        let warped = eval(
            &params::Warp3Params {
                strength: 0.5,
                axial: 1.0,
            },
            &[&source, &flat, &flat, &flat],
        );
        let worst = source
            .data
            .iter()
            .zip(warped.data.iter())
            .map(|(one, two)| (one - two).abs())
            .fold(0.0_f32, f32::max);
        assert!(worst < 1e-6, "常数偏移场不该挪动任何东西，最大偏差 {worst}");
    }

    #[test]
    fn a_varying_offset_field_actually_displaces_the_samples() {
        let shape = VolumeShape { res: 12, layers: 5 };
        let grid_shape = shape_of(shape.res, shape.layers);
        let source = source_field(shape);
        let offset_a = fbm3(grid_shape, 101);
        let offset_b = fbm3(grid_shape, 202);
        let offset_c = fbm3(grid_shape, 303);
        let warped = eval(
            &params::Warp3Params {
                strength: 0.8,
                axial: 1.0,
            },
            &[&source, &offset_a, &offset_b, &offset_c],
        );
        let changed = source
            .data
            .iter()
            .zip(warped.data.iter())
            .filter(|(one, two)| (*one - *two).abs() > 1e-4)
            .count();
        assert!(
            changed > source.data.len() / 4,
            "只有 {changed} / {} 格被挪动过，扭曲没生效",
            source.data.len()
        );
        let stats = warped.stats();
        assert!(
            stats.min >= 0.0 && stats.max <= 1.0,
            "扭曲不该跑出上游的值域：{stats:?}"
        );
    }
}
