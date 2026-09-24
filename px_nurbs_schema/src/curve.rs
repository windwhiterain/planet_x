//! **NURBS 曲线**：有理 B 样条 + 求值 / 导数 / 节点插入 / 升阶。
//!
//! 全部按 **f64** 算（`f32` 连单位圆都表示不准：控制点里那个 `√2/2` 一舍入，
//! 圆就不再是圆了）。要进网格时才降到 `f32`（细分算子那一步）。
//!
//! 口径（与 `knot.rs` 同一套）：次数 `p`、控制点 `n` 个、节点 `m = n + p + 1` 个。
//! 权**空** = 非有理（全 1）；权非空 = 有理，逐点除。

use crate::knot;

/// 一条 NURBS 曲线。
#[derive(Debug, Clone, PartialEq)]
pub struct Curve {
    pub degree: usize,
    /// 控制点，扁平 `[x, y, z, …]`。
    pub control: Vec<f64>,
    /// 有理权。**空 = 非有理**（等价于全 1，但不占载荷）。
    pub weights: Vec<f64>,
    pub knots: Vec<f64>,
}

/// 一条曲线上的读数：点 + 一阶导（参数域，不是弧长）。
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

    /// 控制点数。
    pub fn count(&self) -> usize {
        self.control.len() / 3
    }

    /// 自检：控制点/权/节点的条数必须互相配得上。
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

    /// 参数区间（clamped 曲线是 `[0, 1]`）。
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

    /// 控制点的**齐次**坐标 `(w·x, w·y, w·z, w)`。
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

    /// **de Boor**：`order` 阶导（`order = 0` 就是点）。
    ///
    /// 两步，而两步都是**同一条线索**（齐次坐标上做线性运算，最后除一次权）：
    ///
    /// 1. 齐次曲线 `A(t) = (w·x, w·y, w·z, w)` 的第 `k` 阶导 = **同一个** `p` 次基函数的
    ///    第 `k` 阶导加权（`A(t) = Σ N_{i,p}(t)·P_i` 逐项求导，没有别的自由度）；
    /// 2. 有理那一层用**商的递推**把权除掉：
    ///    `P⁽ᵏ⁾ = (A⁽ᵏ⁾ − Σ_{j=1..k} C(k,j)·w⁽ʲ⁾·P⁽ᵏ⁻ʲ⁾) / w`。
    ///
    /// ⚠ 齐次导数的 **xyz 三栏与第 4 栏要一起算**：权的高阶导进了递推的每一项，
    ///   少算一项（比如只按一阶商的法则写、`order ≥ 2` 时不迭代）在低阶上看着对、
    ///   二阶导就错。
    pub fn derivative(&self, order: usize, t: f64) -> Result<[f64; 3], String> {
        if order > self.degree {
            return Ok([0.0; 3]);
        }
        let count = self.count();
        let basis = knot::basis(&self.knots, self.degree, t, order);
        // 齐次的各阶导 `A⁽ᵏ⁾`（第 4 栏是 `w⁽ᵏ⁾`）。
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
        // `P⁽ᵏ⁾` 逐阶递推（`P⁽⁰⁾` 就是那个点）。
        let mut points = vec![[0.0_f64; 3]; order + 1];
        for lane in 0..3 {
            points[0][lane] = layers[0][lane] / weight;
        }
        for k in 1..=order {
            let mut value = [layers[k][0], layers[k][1], layers[k][2]];
            let mut binomial = 1.0_f64; // C(k, j) 从 j=1 起递推。
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

    /// 点（`derivative(0, t)` 的别名，读起来顺）。
    pub fn point(&self, t: f64) -> Result<[f64; 3], String> {
        self.derivative(0, t)
    }

    /// 点 + 一阶导。
    pub fn sample(&self, t: f64) -> Result<Sample, String> {
        Ok(Sample {
            point: self.point(t)?,
            tangent: self.derivative(1, t)?,
        })
    }

    /// 清单里那一格的写法（blob 顺序）：控制点 / 权（可空）/ 节点。
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

/// **插入一个节点**（Böhm）：几何一个字不变，控制点变多。
///
/// `times` 是插入的重数（1 = 插一次）。`t` 落在参数区间外时按端点钳住
/// （`t < u0` 就在 `u0` 处插、`t > u1` 就在 `u1` 处插）—— 这是 Böhm 算法里
/// `k` 只能取 `[p, n]` 那条限制的落点。
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
    // 已经有几重了：`s` ⇒ 最多还能插 `p − s` 次。
    let existing = curve.knots.iter().filter(|knot| **knot == t).count();
    if existing + times > degree + 1 {
        return Err(format!(
            "t={t} 上已经有 {existing} 重节点，再插 {times} 次就超过 {} 了",
            degree + 1
        ));
    }

    // ⚠ **在齐次坐标里插**（`A_i = (w·x, w·y, w·z, w)`），插完再除权回控制点：
    //   有理曲线的 Böhm 角切是对**齐次**控制点做的。直接对"控制点位置"加权平均是
    //   另一回事 —— 它在非有理（权全 1）时恰好一致，一有理就漂（实测：单位圆插一个
    //   节点之后半径变成 1.0009，`t` 越靠端点偏得越多）。
    let rational = !curve.weights.is_empty();
    let mut points: Vec<[f64; 4]> = (0..curve.count())
        .map(|index| curve.homogeneous(index))
        .collect();
    let mut knots = curve.knots.clone();

    for _ in 0..times {
        // ⚠ 两个下标**不是一回事**（`partition_point` 给的是"第一个 > t 的节点"的下标）：
        //
        // * `read` = 新节点插进**新**数组的位置，也正是末一个**新**控制点 `Q_read` 的位置；
        // * `k` = **旧**数组里"`t` 所在那一段"的下标（`read − 1`，下界兜住 `t ≤ u_p`）。
        let read = knots.partition_point(|knot| *knot <= t).min(count);
        let k = read.saturating_sub(1).max(degree);
        let mut next = vec![[0.0_f64; 4]; count + 1];
        // 头一段（`Q_0 … Q_{k−p}` = `P_0 … P_{k−p}`）与尾一段（`Q_{read+1} … Q_n` =
        // `P_k … P_{n−1}`）**一个点都没参与混合**，原样搬（尾巴整体后移一格）。
        let head = read - degree;
        next[..head].copy_from_slice(&points[..head]);
        for index in k..count {
            next[index + 1] = points[index];
        }
        // 中间那 `p` 个按**角切**混：`Q_i = (1−a_i)·P_{i−1} + a_i·P_i`，
        // `a_i = (t − u_i) / (u_{i+p} − u_i)`（**旧**节点向量）。
        // ⚠ 系数是**反**的（`P_{i−1}` 拿 `1−a`、`P_i` 拿 `a`）：写反了曲线会被拉向另一边，
        //   而 `t` 正好落在一个节点上时更糟 —— `a = 1` 会让新点变成旧 `P_i` 的副本，
        //   那里本该是 `P_{i−1}`。
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

    // 除权回控制点（`w = 0` 是坏数据：`Curve::new` 的检查会拒掉它）。
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

/// **升阶**：次数 `p → p + r`，几何一个字不变（控制点与权跟着变）。
///
/// 走的是 Piegl & Tiller 那套"**按段升阶**"的算法（A5.9）：
///
/// 1. 先把内部节点的重数补到 `p` —— 那时每一段就是一段**有理 Bézier**；
/// 2. 每段在自己的齐次坐标上做一次**二项式升阶**（`Q_i = Σ C(r,j)C(p,i−j)/C(p+r,i)·P_j`）；
/// 3. 拼回去，并把节点向量的每个内部节点重数加 `r`。
///
/// ⚠ 齐次坐标是**唯一**能让有理曲线升阶保持几何不变的地方：先升阶再除权，
///   与"先除权、再对点与权各升一次"不是一回事（后者会漂）。
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
    // ① 内部节点补到 p 重 ⇒ 每一段都是 Bézier。
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
    // ⚠ 段数是 `(count − 1) / p`，**不是** `count / (p + 1)`：相邻两段**共享一个控制点**
    //   （那就是"内部节点重数 = p"的含义），所以 `count = 段数 · p + 1`。
    //   拿 `count / (p + 1)` 算：8 段的圆会算成 3 段（实测：`9 / 3 = 3`，而它其实有 4 段）。
    let segments = (count - 1) / p;
    let knots = knot::distinct(&work.knots, p);
    if count != segments * p + 1 || segments != knots.len() + 1 {
        return Err(format!(
            "Bézier 抽取对不上：{count} 个控制点 / {} 次 ⇒ {segments} 段，而内部节点有 {} 个",
            p,
            knots.len()
        ));
    }

    // ② 逐段升阶（第 `j` 段的控制点是 `j·p ..= j·p + p`）。
    //    段与段**共享一个端点**（节点重数 p 处两个控制点重合）⇒ 每段只收后 `target` 个。
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
                // `Q_i = a·P_{i−1} + (1−a)·P_i`，`a = i / (d + 1)`。
                // ⚠ 系数是**反**的（新点更靠近 `P_i`）：写反了曲线会被整段拉偏
                //   （实测：升到 4 次之后半径从 1 变成 0.9969）。
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

    // ③ 节点向量：内部节点重数加 r。
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

/// **有理二次整圆**：9 个控制点、权是 `1, √2/2, 1, …`、节点是 `{0,0,0, ¼,¼, ½,½, ¾,¾, 1,1,1}`。
///
/// ⚠ 教科书构造（Piegl & Tiller 例 7.1），而它是**精确**的：对角线上那四个控制点的权是
///   `cos(45°) = √2/2`，于是每个参数点上 `|P(t)| = R` 严丝合缝（判据量的就是这个）。
///   `f32` 装不下那个权 ⇒ 库里一律 `f64`（见文件头）。
///
/// `plane` 是 `xy` / `xz` / `yz` 之一（圆的朝向）；`center` 是圆心。
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
    // 八段 45°：轴上四个点的权是 1、四条对角线上是 √2/2（控制点相应地放到 √2）。
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

/// 一条曲线所在平面的法线。
///
/// 走的是 **Newell 那条求和**（把控制多边形当成闭合环，逐边叉积累加）：
/// 对平面曲线它给出那张平面的法线、并且对"控制点里有重合点、边很短"这类退化不敏感
/// （逐边单独比较长度会挑到随便一条边，实测：xy 平面上的圆会算出 `(0.45, 0.89, 0)`）。
///
/// ⚠ 它决定"这条曲线立在哪张平面里"：平面曲线（圆就是）拿到的就是那张平面的法线。
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
