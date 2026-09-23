//! `field.ridged3`：体网格上的**脊状噪声** —— 星云那些一丝一丝的纤维结构。
//!
//! ⚠ 与 `field.fbm3` 只差"每一档取 `1 - |2x-1|` 再取幂"（见 `px_field_op::noise::ridged_3`）：
//!   fbm 给的是**团块**，ridged 给的是**骨架**。星云的丝、云气的亮脊来自后者。

use px_field_schema::field::Field;
use px_field_schema::noise::FbmSettings;
use px_field_schema::ops::Ridged3;
use px_field_schema::params;
use px_field_schema::volume::{VolumeShape, voxel_of};
use px_graph_schema::Grid;

use crate::noise;

px_graph_schema::px_body! { Ridged3, |p, _i, g| crate::ops::ridged3::eval(p, &[], g) }

pub fn eval(params: &params::Ridged3Params, _inputs: &[&Field], grid: Grid) -> Field {
    let shape = VolumeShape::of(&grid).unwrap_or_else(|| {
        panic!(
            "field.ridged3 要一张体网格画布（域 volume、行数 = res² × layers × 6），\
             拿到的是 {:?} {}×{}",
            grid.projection, grid.width, grid.height
        )
    });
    let settings = FbmSettings {
        frequency: params.frequency,
        octaves: params.octaves,
        lacunarity: params.lacunarity,
        gain: params.gain,
        seed: params.seed,
    };
    // ⚠ **按行带并行**（见 [`crate::parallel`]）：逐格结果与串行逐位相同。
    let width = shape.res as usize;
    let height = shape.height() as usize;
    let data = crate::parallel::rows(width, height, |first, count, out| {
        for row in 0..count {
            let y = (first + row) as u32;
            let (face, _) = shape.slot_of(y).expect("行号在形状之内");
            let base = row * width;
            for x in 0..shape.res {
                let mut voxel = voxel_of(&shape, face, x, y);
                // `zonal` 只动**径向**：采样点是"方向 × 半径" ⇒ 整体乘一个系数就是沿径向
                // 拉长／压扁（切向不受影响），`1.0` = 各向同性。
                for axis in 0..3 {
                    voxel[axis] *= params.zonal;
                }
                out[base + x as usize] = noise::ridged_3(voxel, &settings, params.sharpness);
            }
        }
    });
    Field::with_projection(shape.res, shape.height(), data, grid.projection)
}

#[cfg(test)]
mod tests {
    use super::*;
    use px_field_schema::field::CUBE_FACES;
    use px_field_schema::field::Projection;

    fn grid(res: u32, layers: u32) -> Grid {
        Grid {
            width: res,
            height: res * layers * CUBE_FACES,
            projection: Projection::Volume,
        }
    }

    /// **值落在 `[0,1]`、有起伏、随层变化**（与 fbm3 同一套底线）。
    #[test]
    fn the_ridges_are_normalised_and_vary_through_the_volume() {
        let shape = VolumeShape { res: 8, layers: 5 };
        let field = eval(
            &params::Ridged3Params::default(),
            &[],
            grid(shape.res, shape.layers),
        );
        let stats = field.stats();
        assert!(stats.min >= 0.0 && stats.max <= 1.0, "{stats:?}");
        assert!(stats.max - stats.min > 0.05, "脊得有起伏：{stats:?}");
        let column: Vec<f32> = (0..shape.layers)
            .map(|layer| field.at(3, shape.row_of(0, layer)))
            .collect();
        let spread = column.iter().cloned().fold(f32::MIN, f32::max)
            - column.iter().cloned().fold(f32::MAX, f32::min);
        assert!(spread > 1e-4, "同一列的各层必须不同（实际跨度 {spread}）");
    }

    /// **`sharpness` 把脊变细**：越大，高值区的占比越小。
    ///
    /// ⚠ 判的是**方向**（占比随 sharpness 单调下降），不是某个具体数 —— 具体数随频率与
    ///   种子都在变，方向才是这一栏的语义。
    #[test]
    fn a_larger_sharpness_makes_the_ridges_thinner() {
        let grid = grid(16, 4);
        let share = |sharpness: f32| -> f64 {
            let field = eval(
                &params::Ridged3Params {
                    sharpness,
                    ..Default::default()
                },
                &[],
                grid,
            );
            let high = field.data.iter().filter(|value| **value > 0.5).count();
            high as f64 / field.data.len() as f64
        };
        let broad = share(1.0);
        let thin = share(4.0);
        assert!(
            thin < broad,
            "sharpness 变大应当让高值区变少：1.0 → {broad:.3}、4.0 → {thin:.3}"
        );
    }
}
