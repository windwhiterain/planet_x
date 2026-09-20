//! `volume.density`：**一张三维场 → 一份可步进的密度体积**。
//!
//! 这一档是"造场"与"步进"之间的那道桥：
//!
//! ```text
//! field.fbm3 / warped3 / ridged3 …（体素坐标里的三维场，Domain::Volume）
//!         │
//!         ▼  cloud.density（这一份）
//! VolumeData（立方球网格 + 壳的内外半径）
//!         │
//!         ▼  沿视线积分（体渲染）
//! ```
//!
//! ⚠ **为什么要有这一档**（而不是让步进直接读那张场）：场活在**体素坐标** `(s, t, altitude)`
//!   里，它不知道世界尺度（壳的内外半径）；而步进要的是"世界里的一段区间"。把场搬进
//!   `VolumeData` 时把半径一并钉下来，步进那一侧就只需要"方向 + 距离"。
//!
//! ⚠ **采样点的坐标要先把面偏移减掉**：上游那张场的**六面各自占噪声空间里一块区域**
//!   （见 `px_field_schema::volume::face_offset`），而网格的行列只按面内位置排 ⇒
//!   "读网格"必须走面内坐标。这一条在 `field.warp3` 里踩过一次（格心采样偏 0.40）。

use px_field_schema::field::{CUBE_FACES, Field};
use px_field_schema::volume::VolumeShape;
use px_volume_schema::VolumeData;
use px_volume_schema::params::density::DensityParams;

/// 一行一列上的**径向保守化**半径（`reach` 格）。
///
/// ⚠ 只沿**径向**取最大值（不是三维邻域）：视线在参数空间里主要沿径向推进，面内那一维
///   被三线性插值平滑掉了 ⇒ 需要保守的只有径向。取三维邻域会让面内的细节也往外糊一层，
///   那是白花的代价（`res²` 倍的采样）而换不到东西。
///
/// ⚠⚠ **两头那两层不动**（与 `px_volume_alg::bake` 那条径向滤波同一个口径）：壳的上下壁
///   上密度本来就是 0，一动就变正 ⇒ 壳的**外表面被往外推一格**，雾会糊出壳的边界之外。
///   内层才滤。
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

/// 体网格场的一格 → 密度体积的一格。**没有插值**，是纯粹的搬运。
///
/// ⚠⚠ **这里不该有三线性采样**（第一版写了，是错的）：上游那张三维场与这份体积**是同一张
///   网格**（同样的 `res` / `layers`、同样的立方球布局），所以"场 → 体积"是一次
///   **坐标搬运**，不是重采样。写成插值除了白花时间，还会引入两种必须自己保证的精度陷阱
///   （浮点反解跳层、面偏移往返丢低位），而它们**本来不必存在**。
///   三线性插值真正的用武之地在**步进**那一侧（按任意世界点读体积），不在这里。
///
/// 搬运要做的只有一件事：**换布局**。
///   * 场是行主序 `[y][x]`，行号 `= face × (res·layers) + layer × res + t`；
///   * `VolumeData` 是 `data[((face × layers + layer) × res + t) × res + s]`。
fn field_to_volume_slot(shape: &VolumeShape, face: u32, layer: u32, t: u32, s: u32) -> usize {
    let res = shape.res.max(1);
    (((face * shape.layers.max(1) + layer) * res + t) * res + s) as usize
}

/// `(面, 层, t, s)` 那一格在上游场里的行号（[`VolumeShape::row_of`] 那个布局）。
fn field_row(shape: &VolumeShape, face: u32, layer: u32, t: u32) -> u32 {
    shape.row_of(face, layer) + t
}

/// 三维场 → 密度体积（立方球网格 + 壳的内外半径）。
///
/// 值的含义就是**密度本身**（不是 `(场-τ)/L` 那一档）：步进要按它算消光与发射，
/// 所以这里**不许**做阈值或归一化 —— 那是下游调参的事（同 `params` 里那句：
/// "算子不钳制输出，要钳就在下游接一个 `field.remap`"）。
pub fn bake_density(params: &DensityParams, field: &Field) -> Result<VolumeData, String> {
    let shape = VolumeShape {
        res: params.res.max(2),
        layers: params.layers.max(2),
    };
    if !shape.matches(field) {
        return Err(format!(
            "cloud.density 的上游必须是 res {} × layers {} 的体网格场（域 volume、\
             行数 = res² × layers × 6 = {}），拿到的是 {}×{} / {:?}",
            shape.res,
            shape.layers,
            shape.height(),
            field.width,
            field.height,
            field.projection,
        ));
    }

    let res = shape.res;
    let layers = shape.layers;
    let mut data = vec![0.0_f32; (CUBE_FACES * layers * res * res) as usize];
    let mut column = vec![0.0_f32; layers as usize];
    for face in 0..CUBE_FACES {
        for t in 0..res {
            for s in 0..res {
                for layer in 0..layers {
                    column[layer as usize] = field.at(s, field_row(&shape, face, layer, t));
                }
                let column = dilate_layers(&column, layers, params.reach);
                for layer in 0..layers {
                    data[field_to_volume_slot(&shape, face, layer, t, s)] = column[layer as usize];
                }
            }
        }
    }

    Ok(VolumeData {
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

    fn grid_field(shape: &VolumeShape, fill: impl Fn(u32, u32) -> f32) -> Field {
        let mut field = Field::filled_with(shape.res, shape.height(), 0.0, Projection::Volume);
        for y in 0..field.height {
            for x in 0..field.width {
                field.set(x, y, fill(x, y));
            }
        }
        field
    }

    /// **搬运是"换布局"，不是重采样**：每一格的值必须原样出现在体积里对应的 `(面, 层, t, s)` 上。
    ///
    /// ⚠ 这条是这一档的核心判据。两套布局**不一样**（场是行主序、`VolumeData` 是
    ///   `[面, 层, t, s]`），而错了只会表现为"图像里的云位置不对"，很难归因到搬运。
    #[test]
    fn the_layout_carries_every_value_to_its_own_voxel() {
        let shape = VolumeShape { res: 4, layers: 3 };
        // 值 = 坐标的可逆编码（每一格都不一样，重排一点点都看得出来）。
        let field = grid_field(&shape, |x, y| (x as f32 + 1.0) + (y as f32 + 1.0) * 100.0);
        let volume = bake_density(
            &DensityParams {
                res: shape.res,
                layers: shape.layers,
                reach: 0,
                ..Default::default()
            },
            &field,
        )
        .expect("烘密度");

        let mut checked = 0;
        for face in 0..CUBE_FACES {
            for layer in 0..shape.layers {
                for t in 0..shape.res {
                    for s in 0..shape.res {
                        let want = field.at(s, field_row(&shape, face, layer, t));
                        let got = volume.at(face, layer, t, s);
                        assert!(
                            (got - want).abs() < 1e-6,
                            "面 {face} 层 {layer} t {t} s {s}：搬到 {got}，应当是 {want}"
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert_eq!(checked, volume.samples(), "必须逐格都对过");
    }

    /// **逐点搬运**：一份常数密度的场搬进体积还是那个常数（不重排、不缩放）。
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
        let worst = volume
            .data
            .iter()
            .map(|value| (value - 0.37).abs())
            .fold(0.0_f32, f32::max);
        assert!(worst < 1e-6, "常数场搬过去之后最大偏差 {worst}");
    }

    /// **坐标不重排**：值只跟 `(面, 层, t, s)` 有关，与摊平顺序无关。
    ///
    /// ⚠ 体网格场与 `VolumeData` 的摊平顺序**不同**（`[面][层][t][s]` vs 场的行主序）——
    ///   这条判的就是那一次重排写对了：给每一格一个由坐标唯一决定的数，再逐格对回来。
    #[test]
    fn every_voxel_lands_at_its_own_coordinate() {
        let shape = shape();
        // 值 = 坐标的可逆编码（每一格都不一样）。
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
                        let expected = field.at(s, y);
                        let got = volume.at(face, layer, t, s);
                        assert!(
                            (got - expected).abs() < 1e-6,
                            "面 {face} 层 {layer} t {t} s {s}：搬到 {} 应当是 {expected}",
                            got
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert_eq!(checked, volume.samples());
    }

    /// **`reach` 只让密度变大**（保守化）：逐格不小于 `reach = 0` 的那一份。
    ///
    /// ⚠ 方向是安全的那一边才有意义：步进会跨过径向的细结构，取邻域最大值让边界往外长
    ///   （多了看得见、少了是洞）。反过来（取最小）会把薄丝抹掉 —— 那是"信号消失"，
    ///   比"边界外扩一格"坏得多。
    #[test]
    fn conservative_dilation_only_raises_the_density() {
        let shape = VolumeShape { res: 4, layers: 6 };
        // 只有中间那一层是亮的 ⇒ 保守化应当把它往外各扩一层。
        let field = grid_field(&shape, |_, y| {
            if shape.slot_of(y).map(|(_, layer)| layer) == Some(3) {
                1.0
            } else {
                0.0
            }
        });
        let plain = bake_density(
            &DensityParams {
                res: shape.res,
                layers: shape.layers,
                reach: 0,
                ..Default::default()
            },
            &field,
        )
        .expect("不保守");
        let dilated = bake_density(
            &DensityParams {
                res: shape.res,
                layers: shape.layers,
                reach: 1,
                ..Default::default()
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
        // 扩出来的那两层的具体位置（层 2 与层 4）也该亮。
        for layer in [2_u32, 4] {
            assert!(dilated.at(0, layer, 1, 1) > 0.5, "层 {layer} 应当被扩到");
        }
        // ⚠ 壳的**两头不动**（见 `dilate_layers` 的文档）：壁上原来是 0，保守化之后还得是 0。
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

    /// **形状对不上就当场拒**（而不是烘出一份错位的体积）。
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
