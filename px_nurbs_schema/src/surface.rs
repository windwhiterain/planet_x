use crate::knot;

#[derive(Debug, Clone, PartialEq)]
pub struct Surface {
    pub degree: (usize, usize),
    pub nu: usize,
    pub nv: usize,
    pub control: Vec<f64>,
    pub weights: Vec<f64>,
    pub knots_u: Vec<f64>,
    pub knots_v: Vec<f64>,
}

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

    fn homogeneous_derivative(
        &self,
        du: usize,
        dv: usize,
        u: f64,
        v: f64,
    ) -> Result<[f64; 4], String> {
        let (p, q) = self.degree;
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

    pub fn derivative(&self, du: usize, dv: usize, u: f64, v: f64) -> Result<[f64; 3], String> {
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

    pub fn point(&self, u: f64, v: f64) -> Result<[f64; 3], String> {
        self.derivative(0, 0, u, v)
    }

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Along {
    U,
    V,
}

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

    let flat = |step: usize, index: usize| -> usize {
        match along {
            Along::U => step * surface.nv + index,
            Along::V => index * surface.nv + step,
        }
    };
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

pub fn sphere(radius: f64, center: [f64; 3], rings: u32) -> Result<Surface, String> {
    if !(radius > 0.0) || !radius.is_finite() {
        return Err(format!("半径必须是个正数，给的是 {radius}"));
    }
    if rings == 0 || rings > 8 {
        return Err(format!(
            "纬向片数要在 1..=8 之间（每片 90°），给的是 {rings}"
        ));
    }
    let patches_u = 4_usize;
    let patches_v = rings as usize;
    let nu = 2 * patches_u + 1;
    let nv = 2 * patches_v + 1;

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
        let rho_u = 1.0 / weight_u;
        let (sin_u, cos_u) = angle_u.sin_cos();
        for j in 0..nv {
            let (weight_v, angle_v) = pattern_v[j];
            let rho_v = 1.0 / weight_v;
            let (sin_v, cos_v) = angle_v.sin_cos();
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
