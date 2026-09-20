//! `field.stars`：**球面上的星点**（稀疏亮点）—— 出的是**场**，不是贴图。
//!
//! ⚠ 为什么不用 `px_graph::generate::texture::stars`（星空贴图）：星点要**参与积分**
//!   （被前面的气遮住、被尘埃染红），所以它必须和发射体积在同一趟里被读到 ⇒ 必须是场。
//!
//! 做法：把方向映到一张**格**上（球面 lattice），每格用哈希决定"这一格里有没有星"、
//!   星在格内的**抖动位置**、以及**亮度**。逐纹素只查自己那一格与相邻格（`3×3×3`），
//!   再按角度算核 + 光晕 —— 于是星是**点**（核只有几个像素宽），而不是一片面积平均值。
//!
//! ⚠ 亮度分布取幂律（`brightness_power`）：均匀分布会得到"满天一样亮的芝麻"，
//!   而真实的星空是**少数亮星 + 大量暗星**，参考图里那簇亮星更是明显的幂律尾巴。

use px_field_alg::noise::{cell_hash, unit};
use px_field_schema::field::{Field, GridField, normalize};
use px_field_schema::ops::Stars;
use px_field_schema::params;
use px_graph_schema::Grid;

px_graph_schema::px_body! { Stars, |p, _i, g| crate::ops::stars::eval(p, &[], g) }

/// 格号 → 这一格里那颗星的方向（没有星时回 `None`）。
fn star_in_cell(cell: [i32; 3], params: &params::StarsParams) -> Option<[f32; 3]> {
    let hash = cell_hash(params.seed, cell);
    // 第一个 8 位段当"这一格里有没有星"的硬币。
    if unit(hash, 0) > params.fill {
        return None;
    }
    // 格心 + 抖动（另外三段各管一个轴的抖动）。
    let jitter = |channel: u32| (unit(hash, channel) - 0.5) * 0.8;
    let centre = [
        cell[0] as f32 + 0.5 + jitter(1),
        cell[1] as f32 + 0.5 + jitter(2),
        cell[2] as f32 + 0.5 + jitter(3),
    ];
    Some(normalize(centre))
}

/// 这一格的亮度（**暗的多、亮的少**）。
///
/// ⚠ 取 `draw^(−power)` 而不是 `draw^power`：亮度分布在 `[0,1]` 上时，`draw^p`
///   （`p > 1`）会把**典型的星推向 1**（中位数冲到饱和）—— 那是"满天一样亮的白点"，
///   正好是幂律的反面。`draw ≤ 1 ⇒ draw^(−p) ≥ 1` 再取倒数，得到的是
///   **中位数很小、尾巴很长**的形状。
fn star_brightness(cell: [i32; 3], params: &params::StarsParams) -> f32 {
    // ⚠ 用**另一个种子**：与"有没有星""在哪儿"共用一段哈希会让三件事相关
    //   （画面里会出现"亮的星总是长在格的同一角"这一类看不见的规律）。
    let hash = cell_hash(params.seed ^ 0x51ed_270b, cell);
    let draw = unit(hash, 0).max(1e-3);
    (1.0 / draw).powf(params.brightness_power).min(64.0)
}

pub fn eval(params: &params::StarsParams, _inputs: &[&Field], grid: Grid) -> Field {
    // ⚠ 格数**按分辨率定**（一格约一个纹素宽），不是参数给的绝对格数：
    //   绝对格数在低分辨率下会让"一格"跨住上百个纹素 ⇒ 一颗星核就糊住半面
    //   （实测 `face = 128`、`cells = 512` 时亮纹素占 98.3%）。星点这件事天生是
    //   分辨率相关的，只有"每格 ≈ 一纹素"才在任何面上都是"几个像素的点"。
    let cells = (grid.width as f32 * params.cells_per_texel)
        .round()
        .max(2.0) as i32;
    let cluster = normalize(params.cluster);
    let texel_angle = std::f32::consts::PI / grid.width.max(1) as f32;
    let core_angle = params.size.max(0.1) * texel_angle;
    let halo_angle = params.halo.max(0.1) * texel_angle;
    let mut field = grid.filled(0.0);
    // 方向 → 格：把单位方向乘上格数再取整。球面上格大小随纬度略变（极区密一点），
    // 但星点本来就不要求均匀 —— 均匀分布反而假。
    for y in 0..grid.height {
        for x in 0..grid.width {
            let direction = grid.direction(x, y);
            let base = [
                (direction[0] * cells as f32).floor() as i32,
                (direction[1] * cells as f32).floor() as i32,
                (direction[2] * cells as f32).floor() as i32,
            ];
            // 这一簇里额外加亮：与簇心方向的夹角越小越亮。
            let cosine =
                (direction[0] * cluster[0] + direction[1] * cluster[1] + direction[2] * cluster[2])
                    .clamp(-1.0, 1.0);
            let angle = cosine.acos();
            let in_cluster = 1.0 - (angle / params.cluster_radius.max(1e-4)).min(1.0);
            let cluster_boost = 1.0 + params.cluster_gain * in_cluster * in_cluster;

            let mut total = 0.0_f32;
            for dz in -1..=1 {
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let cell = [base[0] + dx, base[1] + dy, base[2] + dz];
                        let Some(star) = star_in_cell(cell, params) else {
                            continue;
                        };
                        let dot = (direction[0] * star[0]
                            + direction[1] * star[1]
                            + direction[2] * star[2])
                            .clamp(-1.0, 1.0);
                        let angle = dot.acos();
                        // ⚠ 衰减取**高斯**（`exp(−r²)`）而不是线性斜坡 `1 − r`：
                        //   线性斜坡到半径处才归零、而且整个圆盘里都有可观的值 ⇒ 一颗星
                        //   糊一片（实测"光晕 4 纹素"时 18% 的纹素被点亮，成了纱）。
                        //   高斯把绝大部分能量压在中心一两个纹素里，外圈迅速掉下去 ——
                        //   那才是"远处一个点光源"的样子。
                        let core = (-(angle / core_angle).powi(2)).exp();
                        let halo = (-(angle / halo_angle).powi(2)).exp();
                        let level = star_brightness(cell, params) * cluster_boost;
                        total += level * (core + params.halo_gain * halo);
                    }
                }
            }
            // ⚠ 亮度不钳在 1：星点该**过曝**（少数亮星的白核）。钳住会把整片星压成
            //   同一个亮度，而那正好毁掉"暗的多、亮的少"这个分布。
            field.set(x, y, total.max(0.0));
        }
    }
    field
}

#[cfg(test)]
mod tests {
    use super::*;
    use px_field_schema::field::Projection;

    fn grid(face: u32) -> Grid {
        Grid {
            width: face,
            height: face * 6,
            projection: Projection::CubeMap,
        }
    }

    /// **星是点、不是面**：亮纹素必须极稀疏（参考图里"满天星"也只占百分之几）。
    ///
    /// ⚠ 这一条同时钉住两个旋钮的**乘积**（`cells_per_texel × fill`）：单独调哪一个都可能
    ///   让星空从"稀"变成"一层纱"，而纱在缩略图上看起来也像星空，只有量占比才分得开。
    #[test]
    fn stars_are_sparse_points() {
        let field = eval(&params::StarsParams::default(), &[], grid(128));
        let lit = field.data.iter().filter(|value| **value > 0.25).count();
        let share = lit as f64 / field.data.len() as f64;
        assert!(lit > 0, "一颗星都没有");
        assert!(
            share < 0.08,
            "亮纹素占了 {:.1}% —— 星点太糊（应当是点，不是一层纱）",
            share * 100.0
        );
    }

    /// **有明暗层次**：极亮与普通亮之间要拉开（幂律），不能满天一样亮。
    #[test]
    fn the_brightnesses_have_a_tail() {
        let field = eval(&params::StarsParams::default(), &[], grid(128));
        let mut lit: Vec<f32> = field
            .data
            .iter()
            .copied()
            .filter(|value| *value > 0.05)
            .collect();
        lit.sort_by(|a, b| b.partial_cmp(a).expect("没有 NaN"));
        assert!(lit.len() > 20, "亮纹素太少（{}），判不出分布", lit.len());
        let brightest = lit[0];
        let median = lit[lit.len() / 2];
        assert!(
            brightest > median * 2.0,
            "最亮 {brightest:.3} 与中位 {median:.3} 拉不开（没有幂律尾巴）"
        );
    }

    /// **确定性**：同参数两遍逐点相同。
    #[test]
    fn the_same_parameters_give_the_same_stars() {
        let one = eval(&params::StarsParams::default(), &[], grid(64));
        let two = eval(&params::StarsParams::default(), &[], grid(64));
        assert_eq!(one.data, two.data);
    }
}
