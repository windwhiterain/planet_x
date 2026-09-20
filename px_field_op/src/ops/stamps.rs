use px_field_schema::field::Field;
use px_field_schema::ops::Stamps;
use px_field_schema::params;
use px_graph_schema::Grid;

use crate::noise;

px_graph_schema::px_body! {
    Stamps,
    |p, i, g| crate::ops::stamps::eval(p, &[i.base.value()], g)
}

/// 一个**印章**（一次撞击）：中心（**格**为单位）、半径（格）、年龄、以及"盖不盖"的那枚硬币。
///
/// ⚠ 四个数都从**这一格的哈希**派生 ⇒ 同一格永远同一个印章（图重算两次逐字节相同、
///   跨平台一致、不需要存表）。年龄与硬币**用两次不同的哈希**：年龄要排序、硬币要过遮罩，
///   同一个哈希的两个位段会把"年轻的更容易被盖上"这种系统性偏差混进画面。
#[derive(Clone, Copy)]
struct Stamp {
    centre: [f32; 3],
    radius: f32,
    /// 越小越老（老先打 ⇒ 后被年轻的挖掉）。
    age: u32,
    /// 遮罩那枚硬币 `∈ [0,1]`：`coin > 局部密度` 的印章**整枚不盖**。
    coin: f32,
}

/// **盖章式打坑**：把 `params::StampsParams` 那三条机制落到一张场上。
///
/// ```text
/// 每一格（direction × frequency）住着至多一个印章：位置（jitter）、半径（幂律）、
/// 年龄（哈希）、硬币（哈希）—— 格点只是"离我最近的几个印章是谁"的加速结构。
/// 每个像素：把 27（平面档 9）个邻格里**足迹够得着**的印章挑出来，
///           按年龄**从老到新**依次挖掘：
///             碗内（d < R）          ：sum ← sum·(1 − excavate·bowl)，再 sum ← sum − depth·R·bowl·freshness
///             坑缘（R ≤ d < R(1+rim)）：sum ← sum + height·R·sin(π t)·freshness
/// ```
///
/// ⚠ **年龄序**是"真实压盖"的关键：年轻坑把老坑的坑缘切掉（画面上是一个个"半个坑"）。
///   顺序反过来、或改成"按半径大小"，都会出现真实月面上没有的形状。
/// ⚠ **挖掘 ≠ 叠加**：`sum·(1 − excavate·bowl)` 那一步才是"挖掉"，只做 `−depth·bowl` 的话
///   碗里会留着老坑的坑缘（变成一道横在碗底的疤）。
/// ⚠ **遮罩是整枚印章的决定**（在印章中心取上游值），不是逐像素的 —— 逐像素会让坑被切掉一半。
/// ⚠ 足迹 `R·(1+rim) ≤ 1` 格是 27 邻域够用的前提（`max_radius` 按这一条钳住）。
/// ⚠ 算子**不钳制**输出（与 `field.craters` 同一条口径：值域是图自己的事）。
pub fn eval(params: &params::StampsParams, inputs: &[&Field], grid: Grid) -> Field {
    let base = inputs[0];
    let mut field = base.clone();
    let jitter = params.jitter.clamp(0.0, 1.0);
    let rim = params.rim.max(1e-3);
    // ⚠ 足迹半径 = R·(1+rim) ≤ 1 格（见上面那条前提）。
    let max_radius = params.max_radius.clamp(1e-3, 1.0 / (1.0 + rim));
    let min_radius = params.min_radius.clamp(1e-3, max_radius);
    let power = params.power.max(0.05);
    let excavate = params.excavate.clamp(0.0, 1.0);
    let degrade = params.degrade.clamp(0.0, 1.0);

    let mut frequency = params.frequency;
    let mut amplitude = 1.0_f32;
    let mut weight = 0.0_f32;
    let mut candidates: Vec<Stamp> = Vec::with_capacity(27);
    for octave in 0..params.octaves {
        let seed = params.seed ^ octave.wrapping_mul(0x9e37_79b9);
        for y in 0..grid.height {
            for x in 0..grid.width {
                let point = grid_point(params, base, x, y, frequency);
                let cell = [point[0].floor(), point[1].floor(), point[2].floor()];

                candidates.clear();
                let span = if params.spherical { 1 } else { 0 };
                for dz in -span..=span {
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            let at = [
                                cell[0] + dx as f32,
                                cell[1] + dy as f32,
                                cell[2] + dz as f32,
                            ];
                            let Some(stamp) =
                                stamp_of(at, seed, jitter, max_radius, min_radius, power)
                            else {
                                continue;
                            };
                            // 足迹够不着 ⇒ 直接丢掉（大多数格子在这一步就出局，
                            // 于是下面那次排序只对着两三个元素）。
                            let footprint = stamp.radius * (1.0 + rim);
                            if grid_distance(params, point, stamp.centre) >= footprint {
                                continue;
                            }
                            if !passes_mask(params, base, stamp.centre, frequency, stamp.coin) {
                                continue;
                            }
                            candidates.push(stamp);
                        }
                    }
                }
                if candidates.is_empty() {
                    continue;
                }
                // 老 → 新（`age` 小的是老的）。
                candidates.sort_unstable_by_key(|stamp| stamp.age);
                let oldest = candidates[0].age;
                let newest = candidates[candidates.len() - 1].age;
                let ages = (newest.saturating_sub(oldest)) as f32;
                let mut sum = 0.0_f32;
                for stamp in &candidates {
                    // 老化：越老越浅、缘越平（按它在这一格候选里的相对新旧）。
                    let freshness = if ages > 0.0 {
                        1.0 - degrade * ((newest - stamp.age) as f32 / ages)
                    } else {
                        1.0
                    };
                    sum = excavate_stamp(
                        sum,
                        grid_distance(params, point, stamp.centre),
                        stamp.radius,
                        rim,
                        params.depth,
                        params.height,
                        excavate,
                        freshness,
                    );
                }
                field.set(x, y, base.at(x, y) + sum * amplitude);
            }
        }
        weight += amplitude;
        amplitude *= params.gain;
        frequency *= params.lacunarity;
    }

    if weight > 0.0 && (weight - 1.0).abs() > 1e-6 {
        let data = field
            .data
            .iter()
            .zip(base.data.iter())
            .map(|(value, anchor)| anchor + (value - anchor) / weight)
            .collect();
        field.data = data;
    }
    field
}

/// 这一点在**格空间**里的坐标：球面档 `direction × frequency`，平面档 `(u × aspect, v) × frequency`。
fn grid_point(
    params: &params::StampsParams,
    base: &Field,
    x: u32,
    y: u32,
    frequency: f32,
) -> [f32; 3] {
    if params.spherical {
        let direction = base.direction(x, y);
        [
            direction[0] * frequency,
            direction[1] * frequency,
            direction[2] * frequency,
        ]
    } else {
        let (u, v) = base.uv(x, y);
        [u * params.aspect * frequency, v * frequency, 0.0]
    }
}

/// 格空间里的距离（单位是**格**：`1.0` = 一个格子）。球面档与平面档只在用哪两/三个轴上分岔。
fn grid_distance(params: &params::StampsParams, one: [f32; 3], two: [f32; 3]) -> f32 {
    if params.spherical {
        euclid([one[0] - two[0], one[1] - two[1], one[2] - two[2]])
    } else {
        euclid2([one[0] - two[0], one[1] - two[1]])
    }
}

/// 遮罩：在**印章中心**取上游值 → "盖这一枚"的概率；硬币大于它就不盖。
///
/// ⚠ 整枚决定（见模块文档）：所以这里取的是中心那一处的值，而不是逐像素各判一次。
fn passes_mask(
    params: &params::StampsParams,
    base: &Field,
    centre: [f32; 3],
    frequency: f32,
    coin: f32,
) -> bool {
    let (lo, hi) = if params.mask_hi > params.mask_lo {
        (params.mask_lo, params.mask_hi)
    } else {
        (0.0, 1.0)
    };
    let mask = if params.spherical {
        base.sample_direction(normalize([centre[0], centre[1], centre[2]]))
    } else {
        base.sample_uv(
            centre[0] / (params.aspect * frequency),
            centre[1] / frequency,
        )
    };
    let chance = ((mask - lo) / (hi - lo)).clamp(0.0, 1.0);
    coin <= chance
}

/// 把一枚印章**挖**进当前的起伏和（`sum`），返回新的和。
///
/// ⚠ 顺序在内、加法在外：碗内先按 `excavate` 把**已有起伏**压掉，再挖自己的碗 —— 这就是
///   "年轻坑挖掉老坑的坑缘"。只做减法（不压旧起伏）的话，碗底会横着老坑留下的一道疤。
/// ⚠ `height` 是**相对坑深**的比例（参数文档那一栏：真实约 0.2 上下），所以坑缘高
///   `= height · depth · radius`。⚠ 第一版写成 `height · radius` 了：那样 `height = 0.2`
///   与 `depth = 0.24` 同一个量级 ⇒ 渲染出来是一圈**凸起的环**而不是"浅缘的坑"
///   （实测：月面上那些坑看着像抬起来的圆环）。
#[allow(clippy::too_many_arguments)]
fn excavate_stamp(
    sum: f32,
    distance: f32,
    radius: f32,
    rim: f32,
    depth: f32,
    height: f32,
    excavate: f32,
    freshness: f32,
) -> f32 {
    let radius = radius.max(1e-4);
    let depth = depth.max(0.0) * radius * freshness;
    if distance < radius {
        let t = 1.0 - distance / radius;
        let bowl = t * t;
        let cleared = sum * (1.0 - excavate * bowl);
        cleared - depth * bowl
    } else {
        let rim_width = rim * radius;
        if distance >= radius + rim_width {
            return sum;
        }
        let t = (distance - radius) / rim_width;
        sum + height.max(0.0) * depth * (std::f32::consts::PI * t).sin()
    }
}

/// 一个格点上的印章：位置（格内 jitter）、半径（幂律）、年龄与硬币（两次哈希）。
fn stamp_of(
    cell: [f32; 3],
    seed: u32,
    jitter: f32,
    max_radius: f32,
    min_radius: f32,
    power: f32,
) -> Option<Stamp> {
    let key = [cell[0] as i32, cell[1] as i32, cell[2] as i32];
    let geometry = noise::cell_hash(seed, key);
    let fate = noise::cell_hash(seed ^ 0x51ed_270b, key);
    let unit = |hash: u32, shift: u32| ((hash >> shift) & 0x3ff) as f32 / 1023.0;
    let centre = [
        cell[0] + 0.5 + (unit(geometry, 0) - 0.5) * jitter,
        cell[1] + 0.5 + (unit(geometry, 10) - 0.5) * jitter,
        cell[2] + 0.5 + (unit(geometry, 20) - 0.5) * jitter,
    ];
    // 半径：幂律 `r = min + (max−min)·(1−u)^power` ⇒ `power > 1` 时小坑极多、大坑极少。
    // ⚠ 第一版写成 `max·u^(1/power)`：指数小于 1 时它是**往大坑偏**的（`u = 0.1` 给出 `0.38·max`）
    //   —— 与这一栏的文档正好相反。测试 `the_sizes_follow_the_power_law` 抓的就是这个方向。
    let u = unit(geometry, 4);
    let radius = (min_radius + (max_radius - min_radius) * (1.0 - u).powf(power))
        .clamp(min_radius, max_radius);
    Some(Stamp {
        centre,
        radius,
        age: fate,
        coin: unit(fate, 20),
    })
}

fn euclid(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn euclid2(v: [f32; 2]) -> f32 {
    (v[0] * v[0] + v[1] * v[1]).sqrt()
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = euclid(v);
    if length <= f32::EPSILON {
        return [0.0, 1.0, 0.0];
    }
    [v[0] / length, v[1] / length, v[2] / length]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 印章是**格点的纯函数**：同一格两次必须一模一样（否则图重算两次产物不同、跨平台也不同）。
    #[test]
    fn a_stamp_is_a_pure_function_of_its_cell() {
        let once = stamp_of([3.0, -2.0, 7.0], 91, 0.9, 0.5, 0.1, 2.2).expect("有印章");
        let twice = stamp_of([3.0, -2.0, 7.0], 91, 0.9, 0.5, 0.1, 2.2).expect("有印章");
        assert_eq!(once.centre, twice.centre);
        assert_eq!(once.radius, twice.radius);
        assert_eq!(once.age, twice.age);
        assert_eq!(once.coin, twice.coin);
        // 换一个种子、或换一个格子，就得换一枚印章（不然"随机"是假的）。
        let other = stamp_of([3.0, -2.0, 8.0], 91, 0.9, 0.5, 0.1, 2.2).expect("有印章");
        assert!(other.radius != once.radius || other.age != once.age);
    }

    /// 半径按**幂律**：`power > 1` 时小坑必须占多数（用户口径："打很多上去"，真实的撞击
    /// 尺寸分布就是小坑极多）。判据用"小于几何平均的占比"。
    #[test]
    fn the_sizes_follow_the_power_law() {
        let mut radii = Vec::new();
        for x in 0..18 {
            for y in 0..18 {
                for z in 0..3 {
                    let stamp = stamp_of([x as f32, y as f32, z as f32], 7, 0.9, 0.55, 0.12, 2.4)
                        .expect("有印章");
                    radii.push(stamp.radius);
                }
            }
        }
        let count = radii.len() as f32;
        let mean = radii.iter().sum::<f32>() / count;
        let small = radii.iter().filter(|radius| **radius < mean).count() as f32;
        assert!(
            small / count > 0.6,
            "小于平均半径的印章只有 {:.2}（幂律没起作用？）",
            small / count
        );
        assert!(
            radii
                .iter()
                .all(|radius| *radius >= 0.12 - 1e-6 && *radius <= 0.55 + 1e-6),
            "半径必须落在 [min_radius, max_radius] 里"
        );
    }

    /// **挖掘 ≠ 叠加**（用户口径："按真实世界的物理"）：碗里先把旧起伏清掉再落碗底。
    /// 这一条钉的就是那个差别 —— `excavate = 1` 时旧起伏**一点都不剩**，`excavate = 0` 时全留下。
    #[test]
    fn a_young_bowl_clears_the_older_relief_inside_it() {
        // 碗心（`distance = 0` ⇒ `bowl = 1`）：先清旧起伏、再压自己的碗。
        let cleared = excavate_stamp(0.5, 0.0, 1.0, 0.3, 0.2, 0.4, 1.0, 1.0);
        assert!(
            (cleared + 0.2).abs() < 1e-6,
            "应为 -0.2（清掉 0.5、落底 -0.2）：{cleared}"
        );
        let kept = excavate_stamp(0.5, 0.0, 1.0, 0.3, 0.2, 0.4, 0.0, 1.0);
        assert!((kept - 0.3).abs() < 1e-6, "不清除时应为 0.3：{kept}");
        // 坑缘外一点：碗内那一支不参与，只剩碗自身的深度曲线。
        let half = excavate_stamp(0.5, 0.5, 1.0, 0.3, 0.2, 0.4, 1.0, 1.0);
        assert!(
            (half - (0.5 * 0.75 - 0.05)).abs() < 1e-6,
            "t=0.5 ⇒ bowl=0.25：{half}"
        );
    }

    /// 坑缘高是**相对坑深**的比例（真实简单坑的缘高只有坑深的零头），不是相对半径。
    /// ⚠ 第一版写成 `height · radius` ⇒ 画面上是"凸起的圆环"而不是坑（实测过）。
    #[test]
    fn the_rim_is_only_a_fraction_of_the_depth() {
        // ⚠ 缘带是 `radius .. radius·(1+rim)`，峰值在**中点**（那里的 `sin(π t)` 才等于 1）。
        let rim_peak = excavate_stamp(0.0, 1.15, 1.0, 0.3, 0.2, 0.4, 1.0, 1.0);
        let expected = 0.4 * 0.2 * 1.0; // height · depth · radius
        assert!(
            (rim_peak - expected).abs() < 1e-6,
            "缘峰应为 {expected}：{rim_peak}"
        );
        assert!(rim_peak < 0.2, "缘高必须明显小于坑深（0.2）");
        // 缘宽之外归零（`rim = 0.3` ⇒ 1.3 之外没有它的事）。
        assert_eq!(
            excavate_stamp(0.11, 1.31, 1.0, 0.3, 0.2, 0.4, 1.0, 1.0),
            0.11
        );
    }
}
