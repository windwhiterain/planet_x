//! `field.fbm3`：**体网格上的分形噪声**（采样点是体素坐标，不是球面方向）。
//!
//! ⚠ 与 `ops::fbm`（球面那一档）的分工见 `px_field_schema::params::Fbm3Params` 的文档：
//!   那一档按 `direction` 取噪声（**没有径向**，出来的是贴在球面上的一层皮），
//!   这一档按 `(s, t, altitude)` 取 ⇒ 云里前中后三层各自不同。
//!
//! ⚠ 采样点由 `px_field_schema::volume::voxel_of` 给（**布局只有那一处真源**）：
//!   面内格心 + 归一化径向高度 + 每面一个固定偏移。

use px_field_schema::field::Field;
use px_field_schema::noise::FbmSettings;
use px_field_schema::ops::Fbm3;
use px_field_schema::params;
use px_field_schema::volume::{VolumeShape, voxel_of};
use px_graph_schema::Grid;

use crate::noise;

px_graph_schema::px_body! { Fbm3, |p, _i, g| crate::ops::fbm3::eval(p, &[], g) }

pub fn eval(params: &params::Fbm3Params, _inputs: &[&Field], grid: Grid) -> Field {
    let shape = VolumeShape::of(&grid).unwrap_or_else(|| {
        panic!(
            "field.fbm3 要一张体网格画布（域 volume、行数 = res² × layers × 6），\
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
    let mut field = Field::filled_with(shape.res, shape.height(), 0.0, grid.projection);
    for y in 0..field.height {
        let (face, _) = shape.slot_of(y).expect("行号在形状之内");
        for x in 0..field.width {
            let mut voxel = voxel_of(&shape, face, x, y);
            // `zonal` 只动**径向**那一维：结构沿径向被拉长／压扁（`1.0` = 各向同性）。
            voxel[2] *= params.zonal;
            field.set(x, y, noise::fbm_3(voxel, &settings));
        }
    }
    field
}

#[cfg(test)]
mod tests {
    use super::*;
    use px_field_schema::field::CUBE_FACES;
    use px_field_schema::field::Projection;

    fn grid(res: u32, layers: u32) -> Grid {
        Grid {
            width: res,
            height: res * res * layers * CUBE_FACES,
            projection: Projection::Volume,
        }
    }

    fn eval_default(grid: Grid) -> Field {
        eval(&params::Fbm3Params::default(), &[], grid)
    }

    /// **形状跟着画布走**，而且值落在 `[0,1]`（fbm 是归一化过的和）。
    #[test]
    fn the_field_takes_the_canvas_shape_and_stays_normalised() {
        let grid = grid(8, 4);
        let field = eval_default(grid);
        assert_eq!(field.width, 8);
        assert_eq!(field.height, grid.height);
        let stats = field.stats();
        assert!(stats.min >= 0.0 && stats.max <= 1.0, "{stats:?}");
        assert!(stats.max - stats.min > 0.05, "噪声得有起伏：{stats:?}");
    }

    /// **径向真的有变化**：同一个面内位置，不同层的值必须不同。
    ///
    /// ⚠ 这条判的就是"体网格与球面场的差别"：球面档按 `direction` 取噪声 ⇒ 同一列上
    ///   所有层的值**完全相同**（那是一层皮）。这一档要是也那样，体渲染就白做了。
    #[test]
    fn the_column_changes_with_the_layer() {
        let shape = VolumeShape { res: 8, layers: 6 };
        let field = eval_default(grid(shape.res, shape.layers));
        let mut changed = 0;
        for face in 0..CUBE_FACES {
            for x in 0..shape.res {
                let column: Vec<f32> = (0..shape.layers)
                    .map(|layer| field.at(x, shape.row_of(face, layer)))
                    .collect();
                let spread = column.iter().cloned().fold(f32::MIN, f32::max)
                    - column.iter().cloned().fold(f32::MAX, f32::min);
                if spread > 1e-4 {
                    changed += 1;
                }
            }
        }
        let total = CUBE_FACES * shape.res;
        assert_eq!(
            changed, total,
            "每一列都该随层变化（{} / {total} 列变了）—— 不变就是退化成球面档了",
            changed
        );
    }

    /// **六面不一样**：同一行同一列在六面上取到的值不该相同（否则出来是六块复制的云）。
    #[test]
    fn the_six_faces_are_not_copies_of_each_other() {
        let shape = VolumeShape { res: 6, layers: 3 };
        let field = eval_default(grid(shape.res, shape.layers));
        let mut equal = 0;
        for layer in 0..shape.layers {
            for x in 0..shape.res {
                let first = field.at(x, shape.row_of(0, layer));
                for face in 1..CUBE_FACES {
                    if (field.at(x, shape.row_of(face, layer)) - first).abs() < 1e-6 {
                        equal += 1;
                    }
                }
            }
        }
        assert_eq!(equal, 0, "六面里有 {equal} 格与面 0 完全相同");
    }

    /// **纯函数**：同样的参数跑两遍逐点相同（缓存键与"可复现"都靠它）。
    #[test]
    fn the_same_parameters_give_the_same_field() {
        let grid = grid(6, 3);
        let one = eval_default(grid);
        let two = eval_default(grid);
        assert_eq!(one.data, two.data);
    }
}
