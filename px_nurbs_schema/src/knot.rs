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

pub fn distinct(knots: &[f64], degree: usize) -> Vec<f64> {
    let mut out: Vec<f64> = Vec::new();
    for index in degree + 1..knots.len() - degree - 1 {
        if out.last().is_none_or(|last| *last != knots[index]) {
            out.push(knots[index]);
        }
    }
    out
}

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

pub fn basis(knots: &[f64], degree: usize, t: f64, order: usize) -> Vec<Vec<f64>> {
    let order = order.min(degree);
    let count = knots.len() - degree - 1;
    let mut table = vec![vec![vec![0.0_f64; count]; degree + 1]; order + 1];
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

    #[test]
    fn the_knot_count_must_match_the_degree_and_the_control_points() {
        let err = check(&[0.0, 0.0, 0.0, 1.0, 1.0], 2, 5, "夹具").expect_err("条数不对");
        assert!(err.contains("节点 5 个"), "{err}");
        check(&[0.0, 0.0, 0.0, 1.0, 1.0, 1.0], 2, 3, "夹具").expect("三个控制点的二次曲线");
    }

    #[test]
    fn the_clamped_helper_matches_the_hand_written_vector() {
        let knots = clamped(2, 4, &[0.5]).expect("造得出来");
        assert_eq!(knots, vec![0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn an_interior_multiplicity_beyond_the_degree_is_rejected() {
        let err = check(&[0.0, 0.0, 0.0, 0.5, 0.5, 0.5, 1.0, 1.0, 1.0], 2, 6, "夹具")
            .expect_err("重数 3 > 2");
        assert!(err.contains("重数是 3"), "{err}");
    }

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

    #[test]
    fn the_right_endpoint_is_closed() {
        let knots = clamped(2, 9, &[0.25, 0.25, 0.5, 0.5, 0.75, 0.75]).expect("造得出来");
        assert_eq!(span(&knots, 2, 9, 1.0), 8);
        let values = basis(&knots, 2, 1.0, 0);
        assert_eq!(values[0], vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
    }
}
