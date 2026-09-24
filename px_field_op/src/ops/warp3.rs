//! `field.warp3`：体网格上的**域扭曲** —— 按三张偏移场把采样点挪开。
//!
//! ⚠ 这是把"团块"变成"丝絮"的那一步：单靠 fbm 得到的是圆滚滚的疙瘩，而沿三个轴各自
//!   挪一段（挪多少由另外三张噪声场决定）之后，等值面会被拉长、打卷、撕裂。
//!
//! ⚠ **体素空间里的采样是"钳制"而不是"环绕"**（见 [`sample_voxel`]）：体网格是**六块**
//!   分开的面，面与面之间没有环绕关系；沿着 `x` 环绕会从"这一面的右边"跳回"同一面的
//!   左边"（两者在世界里差着 90°，`cube_direction` 在那一圈上并不相同）⇒ 接缝上会多出
//!   一条谁也没动过的错位。钳制则最多把边缘"糊住"，方向是安全的那一边。

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
    // ⚠ 输出与**第一张上游场**（待扭曲那张）同形：形状只有一个来源。
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

    // 位移的中位数 → 0：三张噪声场的均值都在 0.5 上下，直接乘 `strength` 会让整块场
    // 整体平移半格的量级 —— 那不是扭曲，那是"错位"。去掉各自的均值之后就只剩"波动"
    // 那一部分在挪采样点。
    let center = |field: &Field| field.stats().mean;
    let center = [center(offset_a), center(offset_b), center(offset_c)];
    let axial = params.axial.clamp(0.0, 1.0);
    // 三个轴的权重：`axial = 0` ⇒ 只沿径向挪（第三张偏移场说了算）。
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

/// 体素坐标 → 这张体网格里的值（**三线性**，边缘**钳制**）。
///
/// ⚠ **`voxel` 是"面内坐标"（不含面偏移）**：网格的行列只按面内位置排。整条算式一律在
///   面内坐标里做，绝不"加上偏移再减回来" —— 偏移量级可以到 `31`，`f32` 上那样往返会丢
///   低位，实测 `res = 8` 时格心采样偏差达 `0.55`（半个值域）。⇒ 位移在
///   [`local_voxel_of`] 的那一套坐标里加（见 `eval`），不要在 [`voxel_of`] 的输出上加。
///
/// ⚠ 第三维要按"行号 = `face × layers + layer`"落回行上：层号与行号差着一整个面的偏移，
///   直接把 `z × height` 当行号是错的（那会读到别的面上去）。
fn sample_voxel(source: &Field, shape: &VolumeShape, face: u32, voxel: [f32; 3]) -> f32 {
    let width = shape.res.max(1);
    let last_layer = shape.layers.max(2) - 1;
    let last_cell = width.max(2) - 1;

    // 面内两维：格心在 `(i + 0.5) / res` ⇒ 反解时先减半格。
    //
    // ⚠ 小数部分要**吸附**到 0 或 1 附近：格心坐标是 `(x + 0.5) / res` 这种除不尽的数，
    //   乘回来会落在 `x ± 1e-7` 上 —— 于是"恰好指向格心"的那一次采样变成"两个相邻格按
    //   1e-7 混合"。对平滑场看不出来，但密度场在丝状结构上极陡 ⇒ 那一下就是半个值域的偏差。
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

    // 径向：层心在整数上（`altitude = layer / (layers-1)`）。
    //
    // ⚠⚠ **这里必须把"层号"吸附回整数**，不能只用 `snap` 吸小数：`f32` 上
    //   `1/3 × 3 = 1.0000001` ⇒ `floor` 给出 1（而不是 0）⇒ **采样跳到下一层**。
    //   实测：格心采样偏差 0.40（近半个值域），而画面上只表现为"形状不对"。
    //   吸附到最近的整数之后，`1.0000001` 与 `0.9999999` 都落回层 1。
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

    // ⚠ 这里用 `row` / `t` 两个名字，**不许**用 `y`：`y` 是"整张网格的行号"（跨面跨层连续），
    //   而这个闭包要的是"**面内**的行"（`0..res`）。两者同名的话闭包参数会**遮住**外层，
    //   算式变成 `层号 + 面内行` —— 实测那让格心采样偏差 0.40，而画面上只是"形状不对"。
    //   正确的关系是 `行号 = 层号 × res + 面内行`（一层之内行是连续的，见 `volume.rs` 的文件头）。
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

    /// 体网格那一档的形状参数（`width = res`、`height = res × layers × 6`）。
    fn shape_of(res: u32, layers: u32) -> Shape {
        Shape {
            width: res,
            height: res * layers * CUBE_FACES,
            projection: Projection::Volume,
        }
    }

    /// 一张体网格 fbm3（⚠ 这一档是生成类算子，尺寸由**参数**给）。
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

    /// 上游场：默认参数的那张 fbm3，尺寸换成 `shape`。
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

    /// 体网格的行数（`res² × layers × 6`）—— 测试里只此一处，免得每个字面量各自记一条公式。
    fn px_protocol_volume_height(res: u32, layers: u32) -> u32 {
        res * layers * CUBE_FACES
    }

    /// **采样器在格心上返回那一格自己的值**（不带位移时的逐点恒等）。
    ///
    /// ⚠ 这条把"采样器"与"扭曲"分开：上面那两条判的是整个算子，一旦红了分不清是
    ///   "采样点算错了"还是"位移算错了"。这一条只喂**精确的格心坐标**。
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

    /// **`strength = 0` 就是原样**：位移为零时逐点等于上游。
    ///
    /// ⚠ 这是这个算子的不动点，也是"扭没扭"的零点 —— 它错了的话后面每一张图都错。
    #[test]
    fn zero_strength_returns_the_source_point_by_point() {
        let shape = VolumeShape { res: 8, layers: 4 };
        // 上游用一张有起伏的场，才看得出一条"原样"到底是不是原样。
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

    /// **一张常数场扭不动任何东西**：偏移处处等于自己的均值 ⇒ 去掉均值后位移为零。
    ///
    /// ⚠ 这条钉的是"去均值"那一步：不去均值的话，一张常数 0.5 的偏移场会把整块场
    ///   平移半格（`0.5 × strength`），而那是**整体错位**不是扭曲 —— 画面上看不出来
    ///   （只是云挪了位置），但它会让"接缝对不对"这类判据全部失效。
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

    /// **真的会扭**：换成有起伏的偏移场，输出必须与上游不同。
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
