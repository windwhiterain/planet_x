//! **节点向量**：非降 + 条数必须配得上次数与控制点数。
//!
//! ⚠ 它是 NURBS 唯一那条"外部输入要能自证"的东西：控制点可以随便给（曲线难看是我们自己的
//!   事），而节点向量**条数不对**会让 `find_span` 直接越界。⇒ 装载/构造时就当场拒，
//!   不留给求值那一层去踩。
//!
//! 约定（与 Cox-de Boor 同一条）：次数 `p`、控制点数 `n` ⇒ 节点数 `m = n + p + 1`。
//! 端点的重数 `≤ p + 1`，内部节点的重数 `≤ p` —— 超过就不是一条曲线了。

/// 校验一条节点向量；回一句人话的错，或者 `Ok`。
pub fn check(knots: &[f64], degree: usize, count: usize, what: &str) -> Result<(), String> {
    if degree == 0 {
        return Err(format!(
            "{what}：次数不能是 0（那条曲线的控制点就是折线本身）"
        ));
    }
    if count <= degree {
        return Err(format!(
            "{what}：控制点 {count} 个不够画一条 {degree} 次曲线（至少要 {} 个）",
            degree + 1
        ));
    }
    let want = count + degree + 1;
    if knots.len() != want {
        return Err(format!(
            "{what}：节点 {} 个，而 {count} 个控制点 + {degree} 次要求 {} 个（n + p + 1）",
            knots.len(),
            want,
        ));
    }
    for window in knots.windows(2) {
        if window[1] < window[0] {
            return Err(format!(
                "{what}：节点向量必须非降，{} 后面跟着 {}",
                window[0], window[1]
            ));
        }
    }
    if !knots.iter().all(|value| value.is_finite()) {
        return Err(format!("{what}：节点向量里有非有限值"));
    }
    // 重数：两端 ≤ p + 1，中间 ≤ p。
    let mut run = 1_usize;
    for index in 1..knots.len() {
        if knots[index] == knots[index - 1] {
            run += 1;
        } else {
            run = 1;
        }
        let at_start = index + 1 == run;
        let at_end = index + 1 == knots.len();
        let limit = if at_start || at_end {
            degree + 1
        } else {
            degree
        };
        if run > limit {
            return Err(format!(
                "{what}：节点 {} 的重数是 {run}，而这里最多 {limit}（内部 ≤ p、端点 ≤ p+1）",
                knots[index]
            ));
        }
    }
    if knots[0] == knots[knots.len() - 1] {
        return Err(format!("{what}：节点向量全是一个值 {}", knots[0]));
    }
    Ok(())
}

/// 两端各 `p + 1` 重（clamped）。`breaks` 是内部节点的取值。
pub fn clamped(degree: usize, count: usize, breaks: &[f64]) -> Result<Vec<f64>, String> {
    let inner = count - degree - 1;
    if breaks.len() != inner {
        return Err(format!(
            "内部节点给 {} 个，而 {count} 个控制点 + {degree} 次要 {inner} 个",
            breaks.len()
        ));
    }
    let mut knots = vec![0.0_f64; degree + 1];
    knots.extend_from_slice(breaks);
    knots.extend(std::iter::repeat_n(1.0_f64, degree + 1));
    check(&knots, degree, count, "clamped 节点向量")?;
    Ok(knots)
}

/// 内部节点的**不同取值**（升阶那条算法按段走，要的就是它）。
pub fn distinct(knots: &[f64], degree: usize) -> Vec<f64> {
    let mut out: Vec<f64> = Vec::new();
    for index in degree + 1..knots.len() - degree - 1 {
        if out.last().is_none_or(|last| *last != knots[index]) {
            out.push(knots[index]);
        }
    }
    out
}

/// 参数 `t` 落在那一段（**端点取它自己那一段**）。
///
/// ⚠ 右端点必须回 `count − 1`（最后一段的下标），**不能**回 `degree`：Cox-de Boor 在
///   `t = u_{count}` 上、用 `degree` 那一段算出来的基函数**全是 0**（`u_{i+p}` 落在一串
///   相等的端点上、分母为零 ⇒ 每一项都被判成 0）⇒ 求值会得到 `0/w = 0`。
///   实测症状：圆的 `t = 1` 求出来是原点、`t = 1` 的导数报"权的和为 0"。
///   端点本来就落在最后那一段的**闭右端**上，回 `count − 1` 才是它对的那一段。
pub fn span(knots: &[f64], degree: usize, count: usize, t: f64) -> usize {
    let upper = count - 1;
    if t >= knots[count] {
        return upper;
    }
    if t <= knots[degree] {
        return degree;
    }
    let (mut low, mut high) = (degree, count);
    let mut middle = (low + high) / 2;
    while t < knots[middle] || t >= knots[middle + 1] {
        if t < knots[middle] {
            high = middle;
        } else {
            low = middle;
        }
        middle = (low + high) / 2;
        if middle <= degree {
            return degree;
        }
        if middle >= upper {
            return upper;
        }
    }
    middle
}

/// `p` 次基函数在 `t` 处的值**与前 `order` 阶导**（Cox-de Boor 的逐层递推）。
///
/// 返回 `(order + 1) × count` 的表（`count = knots.len() − degree − 1`）：
/// `[k][i]` 是 `N_{i,p}` 的**第 k 阶导**在 `t` 处的值。
///
/// ⚠ 为什么整条节点向量一起算（而不是只算 `span` 上那 `p + 1` 个）：只算那一段要在
///   "单位那一格落在哪个下标"上做文章，而那个下标在**右端点**（`t = u_count`）上会落到
///   半开区间的外面 —— 实测症状是端点那一段的基函数算成 `[¼, ½, ¼]` 这种"看起来合理、
///   但曲线不再插值端点"的东西。整条一起算没有这个自由度可错，代价是
///   `O(count · degree · order)`（这一档的次数与控制点数都很小）。
///
/// 第 0 行是**单位分解**（和恒为 1），第 k ≥ 1 行是导数（和恒为 0）。
pub fn basis(knots: &[f64], degree: usize, t: f64, order: usize) -> Vec<Vec<f64>> {
    let order = order.min(degree);
    let count = knots.len() - degree - 1;
    // `table[k][level][i]` = `N_{i,level}` 的第 k 阶导。要一路铺到 `degree` 那一层
    // （第 k 阶导要从 `degree−k` 层递推上来），所以按 `degree + 1` 层开。
    let mut table = vec![vec![vec![0.0_f64; count]; degree + 1]; order + 1];
    // ⚠ 右端点按**闭区间**算（`t = u_count` 落在最后那一段上），否则端点求值恒为 0。
    let last = knots[count];
    for index in 0..count {
        let inside = if t >= last {
            index + 1 == count
        } else {
            knots[index] <= t && t < knots[index + 1]
        };
        if inside {
            table[0][0][index] = 1.0;
        }
    }
    for level in 1..=degree {
        for index in 0..count {
            let a = knots[index];
            let b = knots[index + level];
            let left = if b > a {
                (t - a) * table[0][level - 1][index] / (b - a)
            } else {
                0.0
            };
            let a = knots[index + 1];
            let b = knots[index + level + 1];
            let right = if b > a && index + 1 < count {
                (b - t) * table[0][level - 1][index + 1] / (b - a)
            } else {
                0.0
            };
            table[0][level][index] = left + right;
        }
        // 导数那一档：`N'_{i,level} = level/(u_{i+level}−u_i)·N_{i,level−1}
        //                                − level/(u_{i+level+1}−u_{i+1})·N_{i+1,level−1}`。
        for k in 1..=order.min(level) {
            for index in 0..count {
                let a = knots[index];
                let b = knots[index + level];
                let left = if b > a {
                    (level as f64) * table[k - 1][level - 1][index] / (b - a)
                } else {
                    0.0
                };
                let a = knots[index + 1];
                let b = knots[index + level + 1];
                let right = if b > a && index + 1 < count {
                    (level as f64) * table[k - 1][level - 1][index + 1] / (b - a)
                } else {
                    0.0
                };
                table[k][level][index] = left - right;
            }
        }
    }
    (0..=order).map(|k| table[k][degree].clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 条数不对当场拒（这是唯一会把 `find_span` 送越界的那条输入）。
    #[test]
    fn the_knot_count_must_match_the_degree_and_the_control_points() {
        let err = check(&[0.0, 0.0, 0.0, 1.0, 1.0], 2, 5, "夹具").expect_err("条数不对");
        assert!(err.contains("节点 5 个"), "{err}");
        check(&[0.0, 0.0, 0.0, 1.0, 1.0, 1.0], 2, 3, "夹具").expect("三个控制点的二次曲线");
    }

    /// clamped 那条捷径造出来的向量与手写的逐字相同。
    #[test]
    fn the_clamped_helper_matches_the_hand_written_vector() {
        let knots = clamped(2, 4, &[0.5]).expect("造得出来");
        assert_eq!(knots, vec![0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0]);
    }

    /// 内部节点重数超过次数 ⇒ 拒（那不是曲线）。
    #[test]
    fn an_interior_multiplicity_beyond_the_degree_is_rejected() {
        let err = check(&[0.0, 0.0, 0.0, 0.5, 0.5, 0.5, 1.0, 1.0, 1.0], 2, 6, "夹具")
            .expect_err("重数 3 > 2");
        assert!(err.contains("重数是 3"), "{err}");
    }

    /// 基函数那一行（第 0 行）之和恒为 1（单位分解），导数那些行之和恒为 0。
    #[test]
    fn the_basis_is_a_partition_of_unity_on_every_span() {
        let knots = clamped(3, 6, &[0.3, 0.7]).expect("造得出来");
        for step in 0..=100 {
            let t = f64::from(step) / 100.0;
            let values = basis(&knots, 3, t, 1);
            let sum: f64 = values[0].iter().sum();
            let derivative: f64 = values[1].iter().sum();
            assert!((sum - 1.0).abs() < 1e-12, "t={t} 处基函数和 {sum}");
            assert!(derivative.abs() < 1e-12, "t={t} 处导数之和 {derivative}");
        }
    }

    /// **右端点**插值最后一个控制点（那一行必须在最后一格上取 1）。
    #[test]
    fn the_right_endpoint_is_closed() {
        let knots = clamped(2, 9, &[0.25, 0.25, 0.5, 0.5, 0.75, 0.75]).expect("造得出来");
        assert_eq!(span(&knots, 2, 9, 1.0), 8);
        let values = basis(&knots, 2, 1.0, 0);
        assert_eq!(values[0], vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
    }
}
