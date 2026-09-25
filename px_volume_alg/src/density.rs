use px_field_schema::field::{CUBE_FACES, Field, cube_direction, cube_face_of};
use px_field_schema::volume::VolumeShape;
use px_volume_schema::VolumeData;
use px_volume_schema::params::density::DensityParams;

const SHELL_WALL_FADE: f32 = 0.14;

fn shell_wall(altitude: f32) -> f32 {
    let ramp = |x: f32| {
        let t = (x / SHELL_WALL_FADE).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    ramp(altitude) * ramp(1.0 - altitude)
}

fn dilate_layers(values: &[f32], layers: u32, reach: u32) -> Vec<f32> {
    let reach = reach as usize;
    let count = layers as usize;
    if reach == 0 || count <= 2 * reach + 1 {
        return values.to_vec();
    }
    let mut out = values.to_vec();
    for layer in 1..count - 1 {
        let low = layer.saturating_sub(reach);
        let high = (layer + reach).min(count - 1);
        let mut best = f32::NEG_INFINITY;
        for other in low..=high {
            best = best.max(values[other]);
        }
        out[layer] = best;
    }
    out
}

fn field_to_volume_slot(shape: &VolumeShape, face: u32, layer: u32, t: u32, s: u32) -> usize {
    let res = shape.res.max(1);
    (((face * shape.layers.max(1) + layer) * res + t) * res + s) as usize
}

fn field_row(shape: &VolumeShape, face: u32, layer: u32, t: u32) -> u32 {
    shape.row_of(face, layer) + t
}

fn sample_field_local(
    field: &Field,
    shape: &VolumeShape,
    face: u32,
    s: f32,
    t: f32,
    altitude: f32,
) -> f32 {
    let res = shape.res.max(1);
    let last_layer = shape.layers.max(2) - 1;
    let snap = |fraction: f32| {
        if fraction < 1e-4 {
            0.0
        } else if fraction > 1.0 - 1e-4 {
            1.0
        } else {
            fraction
        }
    };
    let sx = s * res as f32 - 0.5;
    let sy = t * res as f32 - 0.5;
    let x0 = sx.floor();
    let y0 = sy.floor();
    let tx = snap(sx - x0);
    let ty = snap(sy - y0);
    let sz = altitude * last_layer as f32;
    let nearest = sz.round();
    let layer0 = if (sz - nearest).abs() < 1e-3 {
        nearest
    } else {
        sz.floor()
    };
    let tz = snap(sz - layer0);
    let last_cell = res.max(2) - 1;
    let clamp_cell = |value: f32| value.clamp(0.0, last_cell as f32) as u32;
    let (xa, xb) = (clamp_cell(x0), clamp_cell(x0 + 1.0));
    let (ya, yb) = (clamp_cell(y0), clamp_cell(y0 + 1.0));
    let layer_at = |step: f32| (layer0 + step).clamp(0.0, last_layer as f32) as u32;
    let (la, lb) = (layer_at(0.0), layer_at(1.0));
    let row_of = |layer: u32, cell_t: u32| field_row(shape, face, layer, cell_t);
    let corner = |cell_x: u32, cell_t: u32, layer: u32| field.at(cell_x, row_of(layer, cell_t));
    let top = (corner(xa, ya, la) * (1.0 - tx) + corner(xb, ya, la) * tx) * (1.0 - ty)
        + (corner(xa, yb, la) * (1.0 - tx) + corner(xb, yb, la) * tx) * ty;
    let bottom = (corner(xa, ya, lb) * (1.0 - tx) + corner(xb, ya, lb) * tx) * (1.0 - ty)
        + (corner(xa, yb, lb) * (1.0 - tx) + corner(xb, yb, lb) * tx) * ty;
    top * (1.0 - tz) + bottom * tz
}

pub fn sample_world(volume: &VolumeData, point: [f32; 3]) -> f32 {
    let radius = (point[0] * point[0] + point[1] * point[1] + point[2] * point[2]).sqrt();
    let span = volume.outer - volume.inner;
    let tolerance = span.abs().max(1.0) * 1e-5;
    if radius < volume.inner - tolerance
        || radius > volume.outer + tolerance
        || span.abs() <= f32::EPSILON
    {
        return 0.0;
    }
    let direction = [point[0] / radius, point[1] / radius, point[2] / radius];
    let (face, s, t) = cube_face_of(direction);
    let res = volume.res.max(2);
    let last_layer = volume.layers.max(2) - 1;

    let altitude =
        px_volume_schema::volume::Shell::new(volume.inner, volume.outer).altitude_of(radius);
    let sz = altitude * last_layer as f32;
    let nearest = sz.round();
    let layer0 = if (sz - nearest).abs() < 1e-3 {
        nearest
    } else {
        sz.floor()
    };
    let snap = |fraction: f32| {
        if fraction < 1e-4 {
            0.0
        } else if fraction > 1.0 - 1e-4 {
            1.0
        } else {
            fraction
        }
    };
    let tz = snap(sz - layer0);

    let sx = s * res as f32 - 0.5;
    let sy = t * res as f32 - 0.5;
    let x0 = sx.floor();
    let y0 = sy.floor();
    let tx = snap(sx - x0);
    let ty = snap(sy - y0);
    let layer_at = |step: f32| (layer0 + step).clamp(0.0, last_layer as f32) as u32;
    let (la, lb) = (layer_at(0.0), layer_at(1.0));

    let corner = |cell_s: f32, cell_t: f32, layer: u32| -> f32 {
        let s = (cell_s + 0.5) / res as f32;
        let t = (cell_t + 0.5) / res as f32;
        let (nf, ns, nt) = cube_face_of(cube_direction(face, s, t));
        let cs = ((ns * res as f32) as u32).min(res - 1);
        let ct = ((nt * res as f32) as u32).min(res - 1);
        volume.at(nf, layer.min(volume.layers.max(1) - 1), ct, cs)
    };
    let top = (corner(x0, y0, la) * (1.0 - tx) + corner(x0 + 1.0, y0, la) * tx) * (1.0 - ty)
        + (corner(x0, y0 + 1.0, la) * (1.0 - tx) + corner(x0 + 1.0, y0 + 1.0, la) * tx) * ty;
    let bottom = (corner(x0, y0, lb) * (1.0 - tx) + corner(x0 + 1.0, y0, lb) * tx) * (1.0 - ty)
        + (corner(x0, y0 + 1.0, lb) * (1.0 - tx) + corner(x0 + 1.0, y0 + 1.0, lb) * tx) * ty;
    top * (1.0 - tz) + bottom * tz
}

pub fn bake_density(params: &DensityParams, field: &Field) -> Result<VolumeData, String> {
    let source = VolumeShape::of_field(field).ok_or_else(|| {
        format!(
            "cloud.density 的上游必须是体网格场（域 volume、行数能被 res²×6 整除），\
             拿到的是 {}×{} / {:?}",
            field.width, field.height, field.projection,
        )
    })?;
    let (res, layers) = params.shape_of();
    let shape = VolumeShape { res, layers };
    let same_grid = res == source.res && layers == source.layers;

    let mut data = vec![0.0_f32; (CUBE_FACES * layers * res * res) as usize];
    let mut column = vec![0.0_f32; layers as usize];
    for face in 0..CUBE_FACES {
        for t in 0..res {
            for s in 0..res {
                for layer in 0..layers {
                    let density = if same_grid {
                        let t_source = t * source.res / res.max(1);
                        let layer_source = layer * source.layers / layers.max(1);
                        field.at(s, field_row(&source, face, layer_source, t_source))
                    } else {
                        sample_field_local(
                            field,
                            &source,
                            face,
                            (s as f32 + 0.5) / res as f32,
                            (t as f32 + 0.5) / res as f32,
                            (layer as f32 + 0.5) / layers as f32,
                        )
                    };
                    column[layer as usize] = density;
                }
                let column = dilate_layers(&column, layers, params.reach);
                let wall_at = |altitude: f32| shell_wall(altitude);
                for layer in 0..layers {
                    let altitude = layer as f32 / (layers - 1).max(1) as f32;
                    data[field_to_volume_slot(&shape, face, layer, t, s)] =
                        column[layer as usize] * wall_at(altitude);
                }
            }
        }
    }

    Ok(VolumeData {
        lanes: 1,
        res,
        layers,
        inner: params.inner,
        outer: params.outer,
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use px_field_schema::field::Projection;

    fn shape() -> VolumeShape {
        VolumeShape { res: 4, layers: 3 }
    }

    fn params_for(shape: &VolumeShape) -> DensityParams {
        DensityParams {
            layers: shape.layers,
            ..Default::default()
        }
    }

    fn grid_field(shape: &VolumeShape, fill: impl Fn(u32, u32) -> f32) -> Field {
        let mut field = Field::filled_with(shape.res, shape.height(), 0.0, Projection::Volume);
        for y in 0..field.height {
            for x in 0..field.width {
                field.set(x, y, fill(x, y));
            }
        }
        field
    }

    #[test]
    fn the_layout_carries_every_value_to_its_own_voxel() {
        let shape = VolumeShape { res: 4, layers: 3 };
        let field = grid_field(&shape, |x, y| (x as f32 + 1.0) + (y as f32 + 1.0) * 100.0);
        let params = DensityParams {
            res: shape.res,
            ..params_for(&shape)
        };
        let volume = bake_density(&params, &field).expect("烘密度");

        let mut checked = 0;
        for face in 0..CUBE_FACES {
            for layer in 0..shape.layers {
                for t in 0..shape.res {
                    for s in 0..shape.res {
                        let want = field.at(s, field_row(&shape, face, layer, t))
                            * shell_wall(layer as f32 / (shape.layers - 1).max(1) as f32);
                        let got = volume.at(face, layer, t, s);
                        assert!(
                            (got - want).abs() < 1e-6,
                            "面 {face} 层 {layer} t {t} s {s}：搬到 {got}，应当是 {want}（搬运值 × 壁窗）"
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert_eq!(checked, volume.samples(), "必须逐格都对过");
    }

    #[test]
    fn the_faces_survive_a_resampling_density_bake() {
        let source = VolumeShape { res: 8, layers: 32 };
        let field = grid_field(&source, |_x, y| {
            let face = y / (source.res * source.layers);
            let layer = (y % (source.res * source.layers)) / source.res;
            (face as f32) * 1000.0 + (layer as f32) * 10.0
        });
        let params = DensityParams {
            layers: 8,
            res: source.res,
            ..Default::default()
        };
        let volume = bake_density(&params, &field).expect("烘密度");
        assert_eq!((volume.res, volume.layers), (8, 8), "产物形状按参数走");
        for layer in [2_u32, 4, 6] {
            let base = volume.at(0, layer, 0, 0);
            for face in 1..CUBE_FACES {
                let got = volume.at(face, layer, 0, 0);
                assert!(
                    (got - base).abs() > 1.0,
                    "面 {face} 层 {layer} 读成 {got}，与面 0 的 {base} 相同 —— \
                     六个面塌成了一个（重采样的行号用了产物的形状）"
                );
            }
        }
    }

    #[test]
    fn sampling_by_world_point_respects_the_shell() {
        let volume = VolumeData {
            lanes: 1,
            res: 8,
            layers: 6,
            inner: 1.0,
            outer: 2.0,
            data: vec![0.7; (CUBE_FACES * 6 * 8 * 8) as usize],
        };
        let inside = sample_world(&volume, [0.0, 1.5, 0.0]);
        assert!((inside - 0.7).abs() < 1e-4, "壳里应当是 0.7，实际 {inside}");
        assert_eq!(sample_world(&volume, [0.0, 2.5, 0.0]), 0.0);
        assert_eq!(sample_world(&volume, [0.0, 0.0, 0.0]), 0.0);
        for axis in 0..3 {
            for sign in [-1.0_f32, 1.0] {
                let mut point = [0.0_f32; 3];
                point[axis] = 1.5 * sign;
                let value = sample_world(&volume, point);
                assert!(
                    (value - 0.7).abs() < 1e-4,
                    "轴 {axis} 方向 {sign} 上应当是 0.7，实际 {value}"
                );
            }
        }
    }

    #[test]
    fn the_volume_can_be_coarser_than_the_field() {
        let field_shape = VolumeShape { res: 8, layers: 6 };
        let field = grid_field(&field_shape, |_, y| {
            field_shape.slot_of(y).map(|(_, layer)| layer).unwrap_or(0) as f32 / 5.0
        });
        let params = DensityParams {
            res: 4,
            layers: 6,
            reach: 0,
            ..Default::default()
        };
        let volume = bake_density(&params, &field).expect("烘密度");
        assert_eq!(volume.res, 4, "面内取参数给的绝对数（上游场是 8 ⇒ 粗一半）");
        assert_eq!(volume.layers, 6, "层数由参数自己给，不跟着面内走");
        assert_eq!(volume.samples(), (CUBE_FACES * 6 * 4 * 4) as usize);
        for face in 0..CUBE_FACES {
            for layer in 0..volume.layers {
                let first = volume.at(face, layer, 0, 0);
                for t in 0..volume.res {
                    for s in 0..volume.res {
                        assert!(
                            (volume.at(face, layer, t, s) - first).abs() < 1e-4,
                            "只跟高度有关的密度，层 {layer} 上应当处处相等"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_constant_field_arrives_unchanged() {
        let shape = shape();
        let field = grid_field(&shape, |_, _| 0.37);
        let params = DensityParams {
            res: shape.res,
            layers: shape.layers,
            inner: 2.0,
            outer: 5.0,
            reach: 0,
        };
        let volume = bake_density(&params, &field).expect("烘密度");
        assert_eq!(volume.res, shape.res);
        assert_eq!(volume.layers, shape.layers);
        assert_eq!(volume.inner, 2.0);
        assert_eq!(volume.outer, 5.0);
        assert_eq!(volume.data.len(), volume.samples());
        let mut worst = 0.0_f32;
        for face in 0..CUBE_FACES {
            for layer in 0..shape.layers {
                let want = 0.37 * shell_wall(layer as f32 / (shape.layers - 1).max(1) as f32);
                for t in 0..shape.res {
                    for s in 0..shape.res {
                        worst = worst.max((volume.at(face, layer, t, s) - want).abs());
                    }
                }
            }
        }
        assert!(worst < 1e-6, "常数场搬过去（再乘壁窗）之后最大偏差 {worst}");
    }

    #[test]
    fn every_voxel_lands_at_its_own_coordinate() {
        let shape = shape();
        let field = grid_field(&shape, |x, y| (x as f32 + 1.0) + (y as f32 + 1.0) * 100.0);
        let params = DensityParams {
            res: shape.res,
            layers: shape.layers,
            reach: 0,
            ..Default::default()
        };
        let volume = bake_density(&params, &field).expect("烘密度");
        let mut checked = 0;
        for face in 0..CUBE_FACES {
            for layer in 0..shape.layers {
                for t in 0..shape.res {
                    for s in 0..shape.res {
                        let y = shape.row_of(face, layer) + t;
                        let expected = field.at(s, y)
                            * shell_wall(layer as f32 / (shape.layers - 1).max(1) as f32);
                        let got = volume.at(face, layer, t, s);
                        assert!(
                            (got - expected).abs() < 1e-6,
                            "面 {face} 层 {layer} t {t} s {s}：搬到 {} 应当是 {expected}（搬运值 × 壁窗）",
                            got
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert_eq!(checked, volume.samples());
    }

    #[test]
    fn conservative_dilation_only_raises_the_density() {
        let shape = VolumeShape { res: 4, layers: 6 };
        let field = grid_field(&shape, |_, y| {
            if shape.slot_of(y).map(|(_, layer)| layer) == Some(3) {
                1.0
            } else {
                0.0
            }
        });
        let plain = bake_density(
            &DensityParams {
                reach: 0,
                ..params_for(&shape)
            },
            &field,
        )
        .expect("不保守");
        let dilated = bake_density(
            &DensityParams {
                reach: 1,
                ..params_for(&shape)
            },
            &field,
        )
        .expect("保守");
        for (index, (one, two)) in plain.data.iter().zip(dilated.data.iter()).enumerate() {
            assert!(
                *two >= *one - 1e-6,
                "第 {index} 格被保守化压低了：{one} → {two}"
            );
        }
        assert!(
            dilated.data.iter().sum::<f32>() > plain.data.iter().sum::<f32>(),
            "中间那一层亮着，保守化之后总量必须变大"
        );
        for layer in [2_u32, 4] {
            assert!(dilated.at(0, layer, 1, 1) > 0.5, "层 {layer} 应当被扩到");
        }
        for face in 0..CUBE_FACES {
            assert_eq!(
                dilated.at(face, 0, 1, 1),
                plain.at(face, 0, 1, 1),
                "内壁那一层不许被保守化推动"
            );
            assert_eq!(
                dilated.at(face, shape.layers - 1, 1, 1),
                plain.at(face, shape.layers - 1, 1, 1),
                "外壁那一层不许被保守化推动"
            );
        }
    }

    #[test]
    fn a_field_of_the_wrong_shape_is_rejected() {
        let shape = shape();
        let wrong_columns = Field::filled_with(8, shape.height(), 0.0, Projection::Volume);
        assert!(
            bake_density(&DensityParams::default(), &wrong_columns).is_err(),
            "列数不对的场必须被拒"
        );
        let wrong_rows = Field::filled_with(shape.res, shape.height() + 1, 0.0, Projection::Volume);
        assert!(
            bake_density(&DensityParams::default(), &wrong_rows).is_err(),
            "行数不对的场必须被拒"
        );
        let flat = Field::filled_with(shape.res, shape.height(), 0.0, Projection::CubeMap);
        assert!(
            bake_density(&DensityParams::default(), &flat).is_err(),
            "域不对的场必须被拒"
        );
    }
}
