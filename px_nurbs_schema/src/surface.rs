//! **NURBS 曲面**：张量积有理 B 样条 + 求值 / 偏导 / 法线 / 节点插入 / 升阶。
//!
//! 与曲线逐条同构（`curve.rs` 的口径原样搬过来），只是**两向**：
//! 次数 `(p, q)`、控制点 `(nu × nv)`、节点 `u: nu + p + 1` / `v: nv + q + 1`。
//!
//! ⚠ 控制点按**行主序**（`index = i · nv + j`，`i` 沿 u、`j` 沿 v）。
//!   行主序不是"顺手"：它让"沿 v 的一行"是**连续一段**（`insert_knot` 按行处理时
//!   不用跳着走），而沿 u 是跨行等距取。

use crate::knot;

/// 一张 NURBS 曲面。
#[derive(Debug, Clone, PartialEq)]
pub struct Surface {
    /// `(u 向次数, v 向次数)`。
    pub degree: (usize, usize),
    /// u 向的控制点数。
    pub nu: usize,
    /// v 向的控制点数。
    pub nv: usize,
    /// 控制点，扁平 `[x, y, z, …]`，**行主序**（`i · nv + j`）。
    pub control: Vec<f64>,
    /// 有理权（空 = 非有理）。
    pub weights: Vec<f64>,
    pub knots_u: Vec<f64>,
    pub knots_v: Vec<f64>,
}

/// 曲面上的一格读数：点 + 两个偏导 + 单位法线（偏导退化时是 `None`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Patch {
    pub point: [f64; 3],
    pub du: [f64; 3],
    pub dv: [f64; 3],
    pub normal: Option<[f64; 3]>,
}

impl Surface {
    pub fn new(
        degree: (usize, usize),
        nu: usize,
        nv: usize,
        control: Vec<f64>,
        weights: Vec<f64>,
        knots_u: Vec<f64>,
        knots_v: Vec<f64>,
    ) -> Result<Self, String> {
        let surface = Self {
            degree,
            nu,
            nv,
            control,
            weights,
            knots_u,
            knots_v,
        };
        surface.check()?;
        Ok(surface)
    }

    pub fn check(&self) -> Result<(), String> {
        let (p, q) = self.degree;
        let want = self.nu * self.nv;
        if self.control.len() != want * 3 {
            return Err(format!(
                "曲面的控制点是 {} 个数，而 {nu}×{nv} 要 {} 个（扁平 [x, y, z, …]）",
                self.control.len(),
                want * 3,
                nu = self.nu,
                nv = self.nv,
            ));
        }
        if !self.weights.is_empty() && self.weights.len() != want {
            return Err(format!(
                "曲面有 {want} 个控制点、{} 个权 —— 两者必须一样多（空权 = 非有理）",
                self.weights.len()
            ));
        }
        if self.control.iter().any(|value| !value.is_finite()) {
            return Err("曲面的控制点里有非有限值".to_string());
        }
        knot::check(&self.knots_u, p, self.nu, "曲面 u 向")?;
        knot::check(&self.knots_v, q, self.nv, "曲面 v 向")
    }

    pub fn domain(&self) -> ((f64, f64), (f64, f64)) {
        (
            (self.knots_u[self.degree.0], self.knots_u[self.nu]),
            (self.knots_v[self.degree.1], self.knots_v[self.nv]),
        )
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

    /// **齐次曲面的偏导** `∂^(du+dv) / ∂u^du ∂v^dv`。
    ///
    /// 张量积曲面 `A(u, v) = Σ_i Σ_j N_{i,p}(u)·N_{j,q}(v)·P_ij` 逐项求偏导 ⇒
    /// 把两边的基函数各自换成第 `du` / 第 `dv` 阶导再乘起来即可（没有交叉项的麻烦）。
    fn homogeneous_derivative(
        &self,
        du: usize,
        dv: usize,
        u: f64,
        v: f64,
    ) -> Result<[f64; 4], String> {
        let (p, q) = self.degree;
        // ⚠ 次数之上偏导就是 0：`basis` 只会算到 `min(order, degree)` 阶，不先挡住
        //   就会把"低一阶的导"当成"高一阶的导"交出去。
        if du > p || dv > q {
            return Ok([0.0; 4]);
        }
        let basis_u = knot::basis(&self.knots_u, p, u, du);
        let basis_v = knot::basis(&self.knots_v, q, v, dv);
        let mut value = [0.0_f64; 4];
        for i in 0..self.nu {
            let coefficient_u = basis_u[du][i];
            if coefficient_u == 0.0 {
                continue;
            }
            for j in 0..self.nv {
                let coefficient = coefficient_u * basis_v[dv][j];
                if coefficient == 0.0 {
                    continue;
                }
                let point = self.homogeneous(i * self.nv + j);
                for lane in 0..4 {
                    value[lane] += coefficient * point[lane];
                }
            }
        }
        Ok(value)
    }

    /// **有理偏导**：先算齐次的，再用商法则把权除掉。
    ///
    /// 用的是那张递推表（`A = w·S`）：
    /// `S⁽ᵏ⁾ = (A⁽ᵏ⁾ − Σ_{i=1..k} C(k,i)·w⁽ⁱ⁾·S⁽ᵏ⁻ⁱ⁾) / w`。
    pub fn derivative(&self, du: usize, dv: usize, u: f64, v: f64) -> Result<[f64; 3], String> {
        // ⚠ 次数之上的导数**就是 0**（那张递推表只在 `du ≤ p` 且 `dv ≤ q` 时成立：
        //   齐次曲面在那一档上恒为零，而"零除以权"会被误当成一个真导数）。
        if du > self.degree.0 || dv > self.degree.1 {
            return Ok([0.0; 3]);
        }
        let order = du + dv;
        if order == 0 {
            let point = self.homogeneous_derivative(0, 0, u, v)?;
            if point[3].abs() <= f64::EPSILON {
                return Err(format!(
                    "曲面在 ({u}, {v}) 处权的和为 0（这份数据不是一张曲面）"
                ));
            }
            return Ok([
                point[0] / point[3],
                point[1] / point[3],
                point[2] / point[3],
            ]);
        }
        // 递推表：每一格是 (i, j) 阶的**有理**导（点）。
        let mut table: Vec<Vec<[f64; 3]>> = vec![vec![[0.0; 3]; dv + 1]; du + 1];
        let mut weight: Vec<Vec<f64>> = vec![vec![0.0; dv + 1]; du + 1];
        for i in 0..=du {
            for j in 0..=dv {
                let a = self.homogeneous_derivative(i, j, u, v)?;
                weight[i][j] = a[3];
                if i == 0 && j == 0 {
                    if a[3].abs() <= f64::EPSILON {
                        return Err(format!(
                            "曲面在 ({u}, {v}) 处权的和为 0（这份数据不是一张曲面）"
                        ));
                    }
                    table[0][0] = [a[0] / a[3], a[1] / a[3], a[2] / a[3]];
                    continue;
                }
                let mut sum = [0.0_f64; 3];
                for k in 0..=i {
                    for l in 0..=j {
                        if k == 0 && l == 0 {
                            continue;
                        }
                        if k > i || l > j {
                            continue;
                        }
                        let coefficient = binomial(i, k) as f64 * binomial(j, l) as f64;
                        let w = weight[k][l];
                        for lane in 0..3 {
                            sum[lane] += coefficient * w * table[i - k][j - l][lane];
                        }
                    }
                }
                let denominator = weight[0][0];
                if denominator.abs() <= f64::EPSILON {
                    return Err(format!("曲面在 ({u}, {v}) 处权的和为 0，导数无从谈起"));
                }
                let mut value = [0.0_f64; 3];
                for lane in 0..3 {
                    value[lane] = (a[lane] - sum[lane]) / denominator;
                }
                table[i][j] = value;
            }
        }
        Ok(table[du][dv])
    }

    /// 点。
    pub fn point(&self, u: f64, v: f64) -> Result<[f64; 3], String> {
        self.derivative(0, 0, u, v)
    }

    /// 点 + 两个偏导 + 单位法线。
    pub fn patch(&self, u: f64, v: f64) -> Result<Patch, String> {
        let point = self.point(u, v)?;
        let du = self.derivative(1, 0, u, v)?;
        let dv = self.derivative(0, 1, u, v)?;
        let cross = [
            du[1] * dv[2] - du[2] * dv[1],
            du[2] * dv[0] - du[0] * dv[2],
            du[0] * dv[1] - du[1] * dv[0],
        ];
        let length = (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
        let normal = if length <= 1e-14 {
            None
        } else {
            Some([cross[0] / length, cross[1] / length, cross[2] / length])
        };
        Ok(Patch {
            point,
            du,
            dv,
            normal,
        })
    }
}

/// `C(n, k)`（升阶与商法则都要它）。
pub fn binomial(n: usize, k: usize) -> u64 {
    if k > n {
        return 0;
    }
    let k = k.min(n - k);
    let mut value = 1_u64;
    for step in 0..k {
        value = value * (n - step) as u64 / (step + 1) as u64;
    }
    value
}

/// 插入的**方向**。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Along {
    U,
    V,
}

/// **插入一个节点**：与曲线同一条算法（Böhm），只是逐行/逐列各做一遍。
///
/// ⚠ 它**保持张量积**：沿 u 插时，`v` 的每一个下标各是一条曲线（沿 u 取点），
///   各自插完再填回去 —— 不是"把曲面当一条长曲线"（那会换掉基）。
pub fn insert_knot(
    surface: &Surface,
    along: Along,
    t: f64,
    times: usize,
) -> Result<Surface, String> {
    let (p, q) = surface.degree;
    let target = match along {
        Along::U => p,
        Along::V => q,
    };
    if times > target {
        return Err(format!("插入重数 {times} 超过 {along:?} 向的次数 {target}"));
    }
    if times == 0 {
        return Ok(surface.clone());
    }

    let rational = !surface.weights.is_empty();
    // 三个数一起定下来，配错一个就是"读出来的不是一条线"（实测踩过两次）：
    //
    // * `lines`  = **有几条线**（沿 u 插时每个 `v` 一条 ⇒ `nv` 条；沿 v 插时 `nu` 条）；
    // * `stride` = 行主序里**沿插入方向走一步**跨过几个点（沿 u 是 `nv`、沿 v 是 `1`）；
    // * `count`  = 一条线**有几个控制点**（沿 u 是 `nu`、沿 v 是 `nv`）—— 节点向量的形状由它定。
    let (lines, count, knot_vector) = match along {
        Along::U => (surface.nv, surface.nu, surface.knots_u.clone()),
        Along::V => (surface.nu, surface.nv, surface.knots_v.clone()),
    };
    let knot = knot_vector.clone();
    let (low, high) = (knot[target], knot[count]);
    let t = t.clamp(low, high);
    let existing = knot.iter().filter(|value| **value == t).count();
    if existing + times > target + 1 {
        return Err(format!(
            "t={t} 上已经有 {existing} 重节点，再插 {times} 次就超过 {} 了",
            target + 1
        ));
    }

    // 控制网是行主序（`flat = u·nv + v`）⇒ 两个方向取线的下标公式**不一样**
    // （沿 u 是"步长乘 `nv`"、沿 v 是"下标乘 `nv`"）。写成"统一的 `step·stride + index`"
    // 看起来更整齐，但那个 `stride` 在沿 v 时要取 1、而 `index` 又要乘回 `nv`，
    // 于是必然有一处错（实测：整条线取出来是错的，插完曲面在 v 方向漂掉）。
    let flat = |step: usize, index: usize| -> usize {
        match along {
            Along::U => step * surface.nv + index,
            Along::V => index * surface.nv + step,
        }
    };
    // 一条**沿插入方向**的曲线：控制点按上面那个公式取出来。
    let line = |index: usize| -> crate::curve::Curve {
        let mut control = Vec::with_capacity(count * 3);
        let mut weights = Vec::with_capacity(count);
        for step in 0..count {
            let at = flat(step, index) * 3;
            control.extend_from_slice(&[
                surface.control[at],
                surface.control[at + 1],
                surface.control[at + 2],
            ]);
            if rational {
                weights.push(surface.weights[flat(step, index)]);
            }
        }
        crate::curve::Curve {
            degree: target,
            control,
            weights,
            knots: knot_vector.clone(),
        }
    };

    // 逐行/逐列插完之后，新的控制点数（每一条都一样）。
    let grown = count + times;
    let (nu, nv) = match along {
        Along::U => (grown, surface.nv),
        Along::V => (surface.nu, grown),
    };
    let mut control = vec![0.0_f64; nu * nv * 3];
    let mut weights = if rational {
        vec![0.0_f64; nu * nv]
    } else {
        Vec::new()
    };
    for index in 0..lines {
        let inserted = crate::curve::insert_knot(&line(index), t, times)?;
        if inserted.count() != grown {
            return Err(format!(
                "逐行插节点之后每条线的控制点是 {} 个，而应当有 {grown} 个",
                inserted.count()
            ));
        }
        for step in 0..grown {
            // 写回用**新网格**的那个下标公式：沿 u 时行宽没变（还是 `nv`），
            // 沿 v 时行宽已经变成 `grown` ⇒ 这里复用同一个 `flat` 形状的公式。
            let to = match along {
                Along::U => step * nv + index,
                Along::V => index * nv + step,
            };
            control[to * 3..to * 3 + 3].copy_from_slice(&inserted.control[step * 3..step * 3 + 3]);
            if rational {
                weights[to] = inserted.weights[step];
            }
        }
    }

    let (degree, knots_u, knots_v) = match along {
        Along::U => {
            let mut knots = surface.knots_u.clone();
            let at = knots.partition_point(|knot| *knot <= t);
            for _ in 0..times {
                knots.insert(at, t);
            }
            ((p, q), knots, surface.knots_v.clone())
        }
        Along::V => {
            let mut knots = surface.knots_v.clone();
            let at = knots.partition_point(|knot| *knot <= t);
            for _ in 0..times {
                knots.insert(at, t);
            }
            ((p, q), surface.knots_u.clone(), knots)
        }
    };

    Surface::new(degree, nu, nv, control, weights, knots_u, knots_v)
}

/// **有理二次球面**：一张张量积曲面片，u 向绕一圈（四个 90° 片）、v 向从北极到南极
/// （`rings` 个 90° 片）。
///
/// 一维那一半就是**有理二次圆弧**的经典写法（与 `curve::circle` 同一份）：
/// `n = 2p + 1` 个控制点、`p` 段 90°，权是 `1, √2/2, 1, …`，控制点在 `ρ = 1/w` 倍处
/// （偶数档 `ρ = 1` 在圆上、奇数档 `ρ = √2` 落在两条切线的交点上）。
///
/// 曲面这一档取**两个一维模式的外积**：把"经线"那一半当作半平面里的坐标
/// `(r, z)`、把"纬线"那一半当作平面里的 `(X, Y)`，于是
///
/// ```text
/// 控制点 = center + R · (r_j · X_i, r_j · Y_i, z_j)
/// 权     = w_u_i · w_v_j
/// ```
///
/// ⚠ `r_j` 与 `X_i` 是**各自**那一半的（已经含了各自的 `ρ`）—— 不能把两个权揉成一个
///   再除一次：极点上 `r = 0`，可那一行的**权仍然是 `w_u_i`**，揉起来除会把极点那一行
///   拉成 `(0, 0, R/w_u)`（实测：极点附近半径算出来 3.5 而不是 3）。
///
/// ⚠ 缝上（90° 那条线）节点重数写成 2 ⇒ 只有 `C⁰` 连续，但**几何是精确的**：
///   `|P(u, v) − center| = R` 在每个参数上成立。判据量的正是后者。
pub fn sphere(radius: f64, center: [f64; 3], rings: u32) -> Result<Surface, String> {
    if !(radius > 0.0) || !radius.is_finite() {
        return Err(format!("半径必须是个正数，给的是 {radius}"));
    }
    if rings == 0 || rings > 8 {
        return Err(format!(
            "纬向片数要在 1..=8 之间（每片 90°），给的是 {rings}"
        ));
    }
    // u 向绕一圈（四个 90° 片）；v 向 `rings` 个 90° 片（北极 → 南极）。
    let patches_u = 4_usize;
    let patches_v = rings as usize;
    let nu = 2 * patches_u + 1;
    let nv = 2 * patches_v + 1;

    // 一维模式：`2p + 1` 个控制点的有理二次圆弧，每段 90°。
    let arc = |patches: usize| -> Vec<(f64, f64)> {
        let root = std::f64::consts::FRAC_1_SQRT_2;
        (0..=2 * patches)
            .map(|index| {
                let angle = std::f64::consts::FRAC_PI_2 * index as f64 / 2.0;
                if index % 2 == 0 {
                    (1.0, angle)
                } else {
                    (root, angle)
                }
            })
            .collect()
    };
    let pattern_u = arc(patches_u);
    // v 向只走 180°（北极 → 南极）：角度是 `π · j / (2·patches_v)`；权与 u 向同一串。
    let pattern_v: Vec<(f64, f64)> = (0..=2 * patches_v)
        .map(|index| {
            let (weight, _) = pattern_u[index.min(pattern_u.len() - 1)];
            let angle = std::f64::consts::PI * index as f64 / (2.0 * patches_v as f64);
            (weight, angle)
        })
        .collect();

    let mut control = vec![0.0_f64; nu * nv * 3];
    let mut weights = vec![0.0_f64; nu * nv];
    for i in 0..nu {
        let (weight_u, angle_u) = pattern_u[i];
        // `ρ = 1/w`：偶数档 1、奇数档 √2（切线交点）。
        let rho_u = 1.0 / weight_u;
        let (sin_u, cos_u) = angle_u.sin_cos();
        for j in 0..nv {
            let (weight_v, angle_v) = pattern_v[j];
            let rho_v = 1.0 / weight_v;
            let (sin_v, cos_v) = angle_v.sin_cos();
            // 经线那一半：`r` 是离轴的距离、`z` 是高度。极点上 `r = 0`（整行同一个点）。
            let r = rho_v * sin_v;
            let z = rho_v * cos_v;
            let direction = [r * rho_u * cos_u, r * rho_u * sin_u, z];
            let at = (i * nv + j) * 3;
            for lane in 0..3 {
                control[at + lane] = center[lane] + radius * direction[lane];
            }
            weights[i * nv + j] = weight_u * weight_v;
        }
    }

    let u_breaks: Vec<f64> = (1..patches_u)
        .map(|index| index as f64 / patches_u as f64)
        .flat_map(|value| [value, value])
        .collect();
    let v_breaks: Vec<f64> = (1..patches_v)
        .map(|index| index as f64 / patches_v as f64)
        .flat_map(|value| [value, value])
        .collect();
    let knots_u = knot::clamped(2, nu, &u_breaks)?;
    let knots_v = knot::clamped(2, nv, &v_breaks)?;
    Surface::new((2, 2), nu, nv, control, weights, knots_u, knots_v)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **球是精确的**：`|P(u, v)| = R` 在整个参数域上成立。
    #[test]
    fn the_rational_sphere_stays_on_the_radius() {
        let ball = sphere(3.0, [1.0, 0.0, 0.0], 2).expect("造球");
        for i in 0..=8 {
            for j in 0..=8 {
                let u = f64::from(i) / 8.0;
                let v = f64::from(j) / 8.0;
                let point = ball.point(u, v).expect("求值");
                let dx = point[0] - 1.0;
                let radius = (dx * dx + point[1] * point[1] + point[2] * point[2]).sqrt();
                assert!(
                    (radius - 3.0).abs() < 1e-12,
                    "(u={u}, v={v}) 处的半径是 {radius}"
                );
            }
        }
    }

    /// **法线是径向的**（曲面法线与位置同向）。
    #[test]
    fn the_normal_of_the_sphere_points_along_the_radius() {
        let ball = sphere(1.0, [0.0; 3], 2).expect("造球");
        for i in 1..8 {
            for j in 1..8 {
                let u = f64::from(i) / 8.0;
                let v = f64::from(j) / 8.0;
                let patch = ball.patch(u, v).expect("求值");
                let normal = patch.normal.expect("非退化");
                let point = patch.point;
                let length =
                    (point[0] * point[0] + point[1] * point[1] + point[2] * point[2]).sqrt();
                let dot =
                    (normal[0] * point[0] + normal[1] * point[1] + normal[2] * point[2]) / length;
                assert!(
                    (dot.abs() - 1.0).abs() < 1e-9,
                    "(u={u}, v={v}) 处法线与半径的夹角余弦是 {dot}（应当是 ±1）"
                );
            }
        }
    }

    /// **插入节点不换几何**（曲面那一档：沿 u 与沿 v 都试）。
    #[test]
    fn inserting_a_knot_keeps_the_surface_where_it_was() {
        let ball = sphere(1.0, [0.0; 3], 2).expect("造球");
        for along in [Along::U, Along::V] {
            let fine = insert_knot(&ball, along, 0.3, 1).expect("插节点");
            for i in 0..=6 {
                for j in 0..=6 {
                    let u = f64::from(i) / 6.0;
                    let v = f64::from(j) / 6.0;
                    let before = ball.point(u, v).expect("原曲面");
                    let after = fine.point(u, v).expect("插过的曲面");
                    for lane in 0..3 {
                        assert!(
                            (before[lane] - after[lane]).abs() < 1e-12,
                            "{along:?} (u={u}, v={v}) 第 {lane} 栏动了"
                        );
                    }
                }
            }
        }
    }
}

/// **升阶**（沿一个方向）：张量积曲面沿 u 升阶 = **每一列**（固定 v）各升一次，
/// 沿 v 升阶 = **每一行**（固定 u）各升一次 —— 与 `insert_knot` 同一条分解。
///
/// ⚠ 两个方向可交换（各自只动自己那一维的基），所以"升到 `(p, q)`"就是两次调用；
///   而**不能**只升一个方向却改两个节点向量（基变了，另一个方向的曲线就对不上了）。
pub fn elevate(surface: &Surface, along: Along, target: usize) -> Result<Surface, String> {
    let (p, q) = surface.degree;
    if target
        == match along {
            Along::U => p,
            Along::V => q,
        }
    {
        return Ok(surface.clone());
    }
    let rational = !surface.weights.is_empty();
    // ⚠ `fixed` = 那一条线**有几条**（另一个方向的下标数）；`stride` = 它内部的**步长**
    //   （沿 u 取一条 v = 常数的线：u 每 +1 跨过一整行 `nv` 个点 ⇒ 步长 `nv`；
    //    沿 v 取一条 u = 常数的线：v 每 +1 就是下一个点 ⇒ 步长 1）。
    //   这一对在下面三处（取线 / 写回）必须一致，配错一次就是"读出来的不是一条线"。
    let (fixed, stride) = match along {
        Along::U => (surface.nv, surface.nv),
        Along::V => (surface.nu, 1),
    };
    let (degree, knot_vector) = match along {
        Along::U => (p, surface.knots_u.clone()),
        Along::V => (q, surface.knots_v.clone()),
    };

    let line = |index: usize| -> crate::curve::Curve {
        let mut control = Vec::with_capacity(stride * 3);
        let mut weights = Vec::with_capacity(stride);
        for step in 0..stride {
            let flat = match along {
                Along::U => step * fixed + index,
                Along::V => index * fixed + step,
            };
            let at = flat * 3;
            control.extend_from_slice(&[
                surface.control[at],
                surface.control[at + 1],
                surface.control[at + 2],
            ]);
            if rational {
                weights.push(surface.weights[flat]);
            }
        }
        crate::curve::Curve {
            degree,
            control,
            weights,
            knots: knot_vector.clone(),
        }
    };

    let mut grown: Option<usize> = None;
    let mut control: Vec<f64> = Vec::new();
    let mut weights: Vec<f64> = Vec::new();
    let mut new_knots: Vec<f64> = Vec::new();
    for index in 0..fixed {
        let raised = crate::curve::elevate_degree(&line(index), target)?;
        grown = Some(raised.count());
        if control.is_empty() {
            control = vec![0.0_f64; raised.count() * fixed * 3];
            if rational {
                weights = vec![0.0_f64; raised.count() * fixed];
            }
            new_knots = raised.knots.clone();
        }
        for step in 0..raised.count() {
            let to = match along {
                Along::U => step * fixed + index,
                Along::V => index * fixed + step,
            };
            control[to * 3..to * 3 + 3].copy_from_slice(&raised.control[step * 3..step * 3 + 3]);
            if rational {
                weights[to] = raised.weights[step];
            }
        }
    }
    let grown = grown.ok_or_else(|| "升阶一份空曲面".to_string())?;

    let (degree, nu, nv, knots_u, knots_v) = match along {
        Along::U => (
            (target, q),
            grown,
            surface.nv,
            new_knots,
            surface.knots_v.clone(),
        ),
        Along::V => (
            (p, target),
            surface.nu,
            grown,
            surface.knots_u.clone(),
            new_knots,
        ),
    };
    Surface::new(degree, nu, nv, control, weights, knots_u, knots_v)
}
