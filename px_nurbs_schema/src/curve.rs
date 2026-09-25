use crate::knot;

#[derive(Debug, Clone, PartialEq)]
pub struct Curve {
    pub degree: usize,
    pub control: Vec<f64>,
    pub weights: Vec<f64>,
    pub knots: Vec<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub point: [f64; 3],
    pub tangent: [f64; 3],
}

impl Curve {
    pub fn new(
        degree: usize,
        control: Vec<f64>,
        weights: Vec<f64>,
        knots: Vec<f64>,
    ) -> Result<Self, String> {
        let curve = Self {
            degree,
            control,
            weights,
            knots,
        };
        curve.check()?;
        Ok(curve)
    }

    pub fn count(&self) -> usize {
        self.control.len() / 3
    }

    pub fn check(&self) -> Result<(), String> {
        if self.control.len() % 3 != 0 {
            return Err(format!(
                "曲线的控制点是 {} 个数 —— 它不是 3 的倍数（扁平 [x, y, z, …]）",
                self.control.len()
            ));
        }
        let count = self.count();
        if !self.weights.is_empty() && self.weights.len() != count {
            return Err(format!(
                "曲线有 {count} 个控制点、{} 个权 —— 两者必须一样多（空权 = 非有理）",
                self.weights.len()
            ));
        }
        if self.control.iter().any(|value| !value.is_finite()) {
            return Err("曲线的控制点里有非有限值".to_string());
        }
        if self.weights.iter().any(|value| !value.is_finite()) {
            return Err("曲线的权里有非有限值".to_string());
        }
        knot::check(&self.knots, self.degree, count, "曲线")
    }

    pub fn domain(&self) -> (f64, f64) {
        (self.knots[self.degree], self.knots[self.count()])
    }

    fn weight_of(&self, index: usize) -> f64 {
        if self.weights.is_empty() {
            1.0
        } else {
            self.weights[index]
        }
    }

    fn homogeneous(&self, index: usize) -> [f64; 4] {
        let weight = self.weight_of(index);
        let at = index * 3;
        [
            self.control[at] * weight,
            self.control[at + 1] * weight,
            self.control[at + 2] * weight,
            weight,
        ]
    }

    pub fn derivative(&self, order: usize, t: f64) -> Result<[f64; 3], String> {
        if order > self.degree {
            return Ok([0.0; 3]);
        }
        let count = self.count();
        let basis = knot::basis(&self.knots, self.degree, t, order);
        let mut layers = vec![[0.0_f64; 4]; order + 1];
        for k in 0..=order {
            for index in 0..count {
                let point = self.homogeneous(index);
                for lane in 0..4 {
                    layers[k][lane] += basis[k][index] * point[lane];
                }
            }
        }
        let weight = layers[0][3];
        if weight.abs() <= f64::EPSILON {
            return Err(format!(
                "曲线在 t={t} 处齐次权的和为 0（这份数据不是一条曲线）"
            ));
        }
        let mut points = vec![[0.0_f64; 3]; order + 1];
        for lane in 0..3 {
            points[0][lane] = layers[0][lane] / weight;
        }
        for k in 1..=order {
            let mut value = [layers[k][0], layers[k][1], layers[k][2]];
            let mut binomial = 1.0_f64;
            for j in 1..=k {
                binomial = binomial * (k - j + 1) as f64 / j as f64;
                let factor = binomial * layers[j][3];
                for lane in 0..3 {
                    value[lane] -= factor * points[k - j][lane];
                }
            }
            for lane in 0..3 {
                points[k][lane] = value[lane] / weight;
            }
        }
        Ok(points[order])
    }

    pub fn point(&self, t: f64) -> Result<[f64; 3], String> {
        self.derivative(0, t)
    }

    pub fn sample(&self, t: f64) -> Result<Sample, String> {
        Ok(Sample {
            point: self.point(t)?,
            tangent: self.derivative(1, t)?,
        })
    }

    pub fn blobs(&self) -> Result<Vec<px_protocol::wire::Blob>, String> {
        let mut blobs = vec![
            crate::payload::blob("控制点", &self.control)?,
            crate::payload::blob("节点向量", &self.knots)?,
        ];
        if !self.weights.is_empty() {
            blobs.push(crate::payload::blob("权", &self.weights)?);
        }
        Ok(blobs)
    }
}

pub fn insert_knot(curve: &Curve, t: f64, times: usize) -> Result<Curve, String> {
    if times == 0 {
        return Ok(curve.clone());
    }
    if times > curve.degree {
        return Err(format!(
            "插入重数 {times} 超过次数 {}：那会让这条曲线在这一点上不再是次数 {}",
            curve.degree, curve.degree,
        ));
    }
    let degree = curve.degree;
    let count = curve.count();
    let (low, high) = curve.domain();
    let t = t.clamp(low, high);
    let existing = curve.knots.iter().filter(|knot| **knot == t).count();
    if existing + times > degree + 1 {
        return Err(format!(
            "t={t} 上已经有 {existing} 重节点，再插 {times} 次就超过 {} 了",
            degree + 1
        ));
    }

    let rational = !curve.weights.is_empty();
    let mut points: Vec<[f64; 4]> = (0..curve.count())
        .map(|index| curve.homogeneous(index))
        .collect();
    let mut knots = curve.knots.clone();

    for _ in 0..times {
        let read = knots.partition_point(|knot| *knot <= t).min(count);
        let k = read.saturating_sub(1).max(degree);
        let mut next = vec![[0.0_f64; 4]; count + 1];
        let head = read - degree;
        next[..head].copy_from_slice(&points[..head]);
        for index in k..count {
            next[index + 1] = points[index];
        }
        for index in head..=k {
            let denominator = knots[index + degree] - knots[index];
            let alpha = if denominator.abs() <= f64::EPSILON {
                0.0
            } else {
                (t - knots[index]) / denominator
            };
            for lane in 0..4 {
                next[index][lane] =
                    (1.0 - alpha) * points[index - 1][lane] + alpha * points[index][lane];
            }
        }
        knots.insert(read, t);
        points = next;
    }

    let mut control = Vec::with_capacity(points.len() * 3);
    let mut weights = if rational {
        Vec::with_capacity(points.len())
    } else {
        Vec::new()
    };
    for point in &points {
        let weight = point[3];
        if rational {
            control.extend_from_slice(&[point[0] / weight, point[1] / weight, point[2] / weight]);
            weights.push(weight);
        } else {
            control.extend_from_slice(&[point[0], point[1], point[2]]);
        }
    }

    Curve::new(degree, control, weights, knots)
}

pub fn elevate_degree(curve: &Curve, target: usize) -> Result<Curve, String> {
    if target == curve.degree {
        return Ok(curve.clone());
    }
    if target < curve.degree {
        return Err(format!(
            "升阶只能往上：这一条是 {} 次，目标是 {target} 次",
            curve.degree
        ));
    }
    if target > 16 {
        return Err(format!("目标次数 {target} 太离谱了（这一档最多到 16）"));
    }
    let mut work = curve.clone();
    for t in knot::distinct(&work.knots, work.degree) {
        let existing = work.knots.iter().filter(|knot| **knot == t).count();
        if existing < work.degree {
            work = insert_knot(&work, t, work.degree - existing)?;
        }
    }

    let p = work.degree;
    let count = work.count();
    let mut basis: Vec<[f64; 4]> = Vec::with_capacity(count);
    for index in 0..count {
        let weight = if work.weights.is_empty() {
            1.0
        } else {
            work.weights[index]
        };
        let at = index * 3;
        basis.push([
            work.control[at] * weight,
            work.control[at + 1] * weight,
            work.control[at + 2] * weight,
            weight,
        ]);
    }
    let segments = (count - 1) / p;
    let knots = knot::distinct(&work.knots, p);
    if count != segments * p + 1 || segments != knots.len() + 1 {
        return Err(format!(
            "Bézier 抽取对不上：{count} 个控制点 / {} 次 ⇒ {segments} 段，而内部节点有 {} 个",
            p,
            knots.len()
        ));
    }

    let mut raised: Vec<[f64; 4]> = Vec::with_capacity(segments * (target + 1));
    for segment in 0..segments {
        let from = segment * p;
        let bezier = &basis[from..from + p + 1];
        let mut current: Vec<[f64; 4]> = bezier.to_vec();
        let mut degree = p;
        while degree < target {
            let mut next = vec![[0.0_f64; 4]; degree + 2];
            next[0] = current[0];
            next[degree + 1] = current[degree];
            for index in 1..=degree {
                let alpha = (index as f64) / (degree as f64 + 1.0);
                for lane in 0..4 {
                    next[index][lane] =
                        alpha * current[index - 1][lane] + (1.0 - alpha) * current[index][lane];
                }
            }
            current = next;
            degree += 1;
        }
        let take = if segment == 0 { target + 1 } else { target };
        let start = current.len() - take;
        raised.extend_from_slice(&current[start..]);
    }

    let raise = target - p;
    let mut new_knots = Vec::with_capacity(raised.len() + target + 1);
    new_knots.extend(std::iter::repeat_n(work.knots[0], target + 1));
    for t in &knots {
        let times = work.knots.iter().filter(|knot| *knot == t).count() + raise;
        new_knots.extend(std::iter::repeat_n(*t, times));
    }
    new_knots.extend(std::iter::repeat_n(
        work.knots[work.knots.len() - 1],
        target + 1,
    ));

    let mut control = Vec::with_capacity(raised.len() * 3);
    let mut weights = Vec::with_capacity(raised.len());
    let mut rational = false;
    for point in &raised {
        if point[3].abs() <= f64::EPSILON {
            return Err("升阶之后有一个齐次权是 0（这份数据不是一条曲线）".to_string());
        }
        control.extend_from_slice(&[
            point[0] / point[3],
            point[1] / point[3],
            point[2] / point[3],
        ]);
        weights.push(point[3]);
        if (point[3] - 1.0).abs() > 1e-12 {
            rational = true;
        }
    }
    Curve::new(
        target,
        control,
        if rational { weights } else { Vec::new() },
        new_knots,
    )
}

pub fn circle(radius: f64, plane: &str, center: [f64; 3]) -> Result<Curve, String> {
    if !(radius > 0.0) || !radius.is_finite() {
        return Err(format!("半径必须是个正数，给的是 {radius}"));
    }
    let (first, second) = match plane {
        "xy" => ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        "xz" => ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        "yz" => ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
        other => return Err(format!("平面只能是 `xy` / `xz` / `yz`，给的是 `{other}`")),
    };
    let root = std::f64::consts::FRAC_1_SQRT_2;
    let layout = [
        (1.0, 0.0, 1.0_f64),
        (1.0, 1.0, root),
        (0.0, 1.0, 1.0),
        (-1.0, 1.0, root),
        (-1.0, 0.0, 1.0),
        (-1.0, -1.0, root),
        (0.0, -1.0, 1.0),
        (1.0, -1.0, root),
        (1.0, 0.0, 1.0),
    ];
    let mut control = Vec::with_capacity(layout.len() * 3);
    let mut weights = Vec::with_capacity(layout.len());
    for (a, b, weight) in layout {
        control.extend_from_slice(&[
            center[0] + radius * (a * first[0] + b * second[0]),
            center[1] + radius * (a * first[1] + b * second[1]),
            center[2] + radius * (a * first[2] + b * second[2]),
        ]);
        weights.push(weight);
    }
    let knots = knot::clamped(2, layout.len(), &[0.25, 0.25, 0.5, 0.5, 0.75, 0.75])?;
    Curve::new(2, control, weights, knots)
}

pub fn plane_normal(curve: &Curve) -> [f64; 3] {
    let count = curve.count();
    let mut normal = [0.0_f64; 3];
    let point = |index: usize| -> [f64; 3] {
        let at = index * 3;
        [
            curve.control[at],
            curve.control[at + 1],
            curve.control[at + 2],
        ]
    };
    for index in 0..count {
        let one = point(index);
        let two = point((index + 1) % count);
        normal[0] += one[1] * two[2] - one[2] * two[1];
        normal[1] += one[2] * two[0] - one[0] * two[2];
        normal[2] += one[0] * two[1] - one[1] * two[0];
    }
    let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
    if length <= 1e-12 {
        return [0.0, 0.0, 1.0];
    }
    [normal[0] / length, normal[1] / length, normal[2] / length]
}
