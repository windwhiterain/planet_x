use super::SettlementReport;

pub(super) const BARRIER: f64 = 0.1;

pub(super) const CURVATURE: f64 = 0.5;

const NEWTON_MAX: usize = 80;
const NEWTON_TOTAL: usize = 400;
const NEWTON_TOL: f64 = 1e-9;
const REDUCTION: f64 = 0.2;
const BOUNDARY: f64 = 0.99;
const BACKTRACK: usize = 60;
const SUFFICIENT: f64 = 1e-4;
const TINY: f64 = 1e-300;
const BOX_WEIGHT: f64 = 1e-4;

pub(super) struct Outcome {
    pub x: Vec<f64>,
    pub eaten: Vec<f64>,
    pub delivered: Vec<f64>,
    pub report: SettlementReport,
}

#[derive(Clone)]
struct Point {
    x: Vec<f64>,
    s: Vec<f64>,
    lambda: Vec<f64>,
    y: Vec<f64>,
}

impl Point {
    fn zeros(policies: usize, goods: usize) -> Self {
        Self {
            x: vec![0.0; policies],
            s: vec![0.0; goods],
            lambda: vec![0.0; goods],
            y: vec![0.0; policies],
        }
    }

    fn apply(&mut self, step: &Point, alpha_p: f64, alpha_d: f64) {
        for i in 0..self.x.len() {
            self.x[i] += alpha_p * step.x[i];
        }
        for i in 0..self.s.len() {
            self.s[i] += alpha_p * step.s[i];
        }
        for i in 0..self.lambda.len() {
            self.lambda[i] += alpha_d * step.lambda[i];
        }
        for i in 0..self.y.len() {
            self.y[i] += alpha_d * step.y[i];
        }
    }
}

struct System {
    active: Vec<usize>,
    used: Vec<usize>,
    rows: usize,
    box_weight: f64,
    c: Vec<Vec<f64>>,
    w: Vec<f64>,
    supply: Vec<f64>,
    curvature: f64,
}

impl System {
    fn marginal(&self, p: usize, x: f64) -> f64 {
        self.curvature * self.w[p] * x.max(TINY).powf(self.curvature - 1.0)
    }

    fn marginal_slope(&self, p: usize, x: f64) -> f64 {
        self.curvature * (self.curvature - 1.0) * self.w[p] * x.max(TINY).powf(self.curvature - 2.0)
    }

    fn shadow(&self, p: usize, lambda: &[f64]) -> f64 {
        let mut total = 0.0f64;
        for k in 0..self.used.len() {
            total += self.c[p][k] * lambda[k];
        }
        total
    }

    fn norm(&self, point: &Point, step: &Point, alpha_p: f64, alpha_d: f64, mu_bar: f64) -> f64 {
        let mut worst = 0.0f64;
        for p in 0..self.w.len() {
            let x = point.x[p] + alpha_p * step.x[p];
            let y = point.y[p] + alpha_d * step.y[p];
            let mut shadow = 0.0f64;
            for k in 0..self.used.len() {
                shadow += self.c[p][k] * (point.lambda[k] + alpha_d * step.lambda[k]);
            }
            let marginal = self.marginal(p, x);
            let stationarity = marginal - shadow + y;
            let magnitude = marginal + shadow.abs() + y.abs();
            worst = worst.max((stationarity / magnitude.max(TINY)).abs());
            worst = worst.max(((x * y - mu_bar) / mu_bar).abs());
        }
        for k in 0..self.used.len() {
            let s = point.s[k] + alpha_p * step.s[k];
            let lambda = point.lambda[k] + alpha_d * step.lambda[k];
            let mut consumed = 0.0f64;
            for p in 0..self.w.len() {
                consumed += self.c[p][k] * (point.x[p] + alpha_p * step.x[p]);
            }
            let feasibility = s + consumed - 1.0;
            worst = worst.max((feasibility / (1.0 + s.abs() + consumed.abs())).abs());
            let target = self.row_weight(k) * mu_bar;
            worst = worst.max(((s * lambda - target) / target).abs());
        }
        worst
    }

    fn gap(&self, point: &Point) -> f64 {
        let mut gap = 0.0f64;
        for k in 0..self.used.len() {
            gap += self.row_weight(k) * point.s[k] * point.lambda[k];
        }
        for p in 0..self.w.len() {
            gap += point.x[p] * point.y[p];
        }
        gap
    }

    fn consumed(&self, x: &[f64], k: usize) -> f64 {
        let mut total = 0.0f64;
        for p in 0..x.len() {
            total += self.c[p][k] * x[p];
        }
        total
    }

    fn row_weight(&self, k: usize) -> f64 {
        if k < self.rows { 1.0 } else { self.box_weight }
    }

    fn start(&self, mu: f64) -> Point {
        let policies = self.w.len();
        let goods = self.used.len();
        let mut point = Point::zeros(policies, goods);
        let mut base = vec![1.0f64; policies];
        for p in 0..policies {
            let mut demand = f64::INFINITY;
            for k in 0..goods {
                let c = self.c[p][k];
                if c > 0.0 {
                    let by_utility = (self.curvature * self.w[p] / (2.0 * c * mu))
                        .powf(1.0 / (1.0 - self.curvature));
                    let by_barrier = 1.0 / (2.0 * c);
                    let branch = if by_utility > by_barrier {
                        by_utility
                    } else {
                        by_barrier
                    };
                    if branch > 0.0 && branch.is_finite() {
                        demand = demand.min(branch);
                    }
                }
            }
            if demand.is_finite() && demand > 0.0 {
                base[p] = demand;
            }
        }
        for _ in 0..8 {
            let mut changed = false;
            for k in 0..goods {
                let column: f64 = (0..policies).map(|p| self.c[p][k] * base[p]).sum();
                let limit = 0.98 * self.supply[k];
                if column > limit && column > 0.0 {
                    let factor = limit / column;
                    for p in 0..policies {
                        if self.c[p][k] > 0.0 {
                            base[p] *= factor;
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        for p in 0..policies {
            point.x[p] = base[p].max(TINY);
        }
        for k in 0..goods {
            let consumed = self.consumed(&point.x, k);
            let slack = (self.supply[k] - consumed).max(TINY);
            point.s[k] = slack;
            point.lambda[k] = mu / slack;
        }
        for p in 0..policies {
            point.y[p] = mu / point.x[p];
        }
        point
    }

    fn direction(&self, point: &Point, mu_bar: f64) -> (Point, bool) {
        let policies = self.w.len();
        let goods = self.used.len();

        let mut diagonal = vec![0.0f64; policies];
        let mut bound_e = vec![0.0f64; policies];
        let mut bound_b = vec![0.0f64; policies];
        let mut rhs = vec![0.0f64; policies];
        for p in 0..policies {
            let x = point.x[p];
            let y = point.y[p];
            let marginal = self.marginal(p, x);
            let r = marginal - self.shadow(p, &point.lambda) + y;
            let e = y / x;
            let b = (mu_bar - x * y) / x;
            diagonal[p] = e - self.marginal_slope(p, x);
            bound_e[p] = e;
            bound_b[p] = b;
            rhs[p] = r + b;
        }

        let mut inverse_slack = vec![0.0f64; goods];
        let mut slack_step = vec![0.0f64; goods];
        let mut slack_rhs = vec![0.0f64; goods];
        for k in 0..goods {
            let s = point.s[k];
            let lambda = point.lambda[k].max(TINY);
            let consumed = self.consumed(&point.x, k);
            let feasibility = s + consumed - self.supply[k];
            let a = (self.row_weight(k) * mu_bar - s * lambda) / lambda;
            inverse_slack[k] = lambda / s;
            slack_step[k] = a;
            slack_rhs[k] = feasibility + a;
        }

        let mut schur = vec![vec![0.0f64; policies]; policies];
        for p in 0..policies {
            schur[p][p] = diagonal[p];
        }
        for k in 0..goods {
            let inverse = inverse_slack[k];
            for p in 0..policies {
                if self.c[p][k] == 0.0 {
                    continue;
                }
                for q in 0..policies {
                    schur[p][q] += self.c[p][k] * self.c[q][k] * inverse;
                }
            }
            for p in 0..policies {
                rhs[p] -= self.c[p][k] * slack_rhs[k] * inverse;
            }
        }

        let scale: Vec<f64> = (0..policies)
            .map(|p| schur[p][p].max(TINY).sqrt())
            .collect();
        let mut balanced = vec![vec![0.0f64; policies]; policies];
        for p in 0..policies {
            for q in 0..policies {
                balanced[p][q] = schur[p][q] / (scale[p] * scale[q]);
            }
        }
        let balanced_rhs: Vec<f64> = (0..policies).map(|p| rhs[p] / scale[p]).collect();
        let (solution, ok) = match cholesky_solve(&balanced, &balanced_rhs) {
            Some(z) => (
                (0..policies).map(|p| z[p] / scale[p]).collect::<Vec<f64>>(),
                true,
            ),
            None => (
                (0..policies)
                    .map(|p| rhs[p] / schur[p][p])
                    .collect::<Vec<f64>>(),
                false,
            ),
        };

        let mut step = Point::zeros(policies, goods);
        for p in 0..policies {
            step.x[p] = solution[p];
            step.y[p] = bound_b[p] - bound_e[p] * solution[p];
        }
        for k in 0..goods {
            let mut consumed = 0.0f64;
            for p in 0..policies {
                consumed += self.c[p][k] * solution[p];
            }
            step.lambda[k] = (consumed + slack_rhs[k]) * inverse_slack[k];
            step.s[k] = slack_step[k] - step.lambda[k] / inverse_slack[k];
        }
        (step, ok)
    }

    fn boundary(&self, point: &Point, step: &Point) -> (f64, f64) {
        let mut primal = 1.0f64;
        let mut dual = 1.0f64;
        for p in 0..self.w.len() {
            let dx = step.x[p];
            if dx < 0.0 {
                primal = primal.min(-point.x[p] / dx);
            }
            if step.y[p] < 0.0 {
                dual = dual.min(-point.y[p] / step.y[p]);
            }
        }
        for k in 0..self.used.len() {
            if step.s[k] < 0.0 {
                primal = primal.min(-point.s[k] / step.s[k]);
            }
            if step.lambda[k] < 0.0 {
                dual = dual.min(-point.lambda[k] / step.lambda[k]);
            }
        }
        ((BOUNDARY * primal).min(1.0), (BOUNDARY * dual).min(1.0))
    }
}

fn cholesky_solve(a: &[Vec<f64>], b: &[f64]) -> Option<Vec<f64>> {
    let n = b.len();
    let mut l = vec![vec![0.0f64; n]; n];
    for i in 0..n {
        for j in 0..=i {
            let mut sum = a[i][j];
            for k in 0..j {
                sum -= l[i][k] * l[j][k];
            }
            if i == j {
                if !(sum > 0.0) || !sum.is_finite() {
                    return None;
                }
                l[i][j] = sum.sqrt();
            } else {
                l[i][j] = sum / l[j][j];
            }
        }
    }
    let mut y = vec![0.0f64; n];
    for i in 0..n {
        let mut sum = b[i];
        for k in 0..i {
            sum -= l[i][k] * y[k];
        }
        y[i] = sum / l[i][i];
    }
    let mut x = vec![0.0f64; n];
    for i in (0..n).rev() {
        let mut sum = y[i];
        for k in (i + 1)..n {
            sum -= l[k][i] * x[k];
        }
        x[i] = sum / l[i][i];
    }
    Some(x)
}

pub(super) struct Budget<'a> {
    pub coefficient: &'a [f64],
    pub limit: f64,
}

pub(super) fn solve(
    w: &[f64],
    plans: &[Vec<f64>],
    outputs: &[Vec<f64>],
    supply: &[f64],
    budgets: &[Budget<'_>],
    barrier: f64,
    curvature: f64,
) -> Outcome {
    let curvature = curvature.clamp(0.01, 1.0);
    let policies = w.len();
    let goods = supply.len();
    let mut x = vec![0.0f64; policies];
    let mut eaten = vec![0.0f64; goods];
    let mut delivered = vec![0.0f64; goods];

    let mut blocked = Vec::new();
    let mut active = Vec::new();
    for p in 0..policies {
        if !(w[p] > 0.0) {
            continue;
        }
        let missing = (0..goods).any(|k| plans[p][k] > 0.0 && !(supply[k] > 0.0));
        if missing {
            blocked.push(p);
        } else {
            active.push(p);
        }
    }
    let mut used: Vec<usize> = (0..goods).filter(|k| supply[*k] > 0.0).collect();
    let rows = used.len();
    let active_budgets: Vec<usize> = (0..budgets.len())
        .filter(|i| budgets[*i].limit.is_finite() && budgets[*i].limit > 0.0)
        .collect();
    for (slot, _index) in active_budgets.iter().enumerate() {
        used.push(goods + slot);
    }

    if active.is_empty() || used.is_empty() {
        return Outcome {
            x,
            eaten,
            delivered,
            report: SettlementReport {
                gap: 0.0,
                mu: 0.0,
                iterations: 0,
                converged: true,
                residual: 0.0,
                phases: 0,
                degraded: false,
                blocked: blocked.len(),
                utilization: 0.0,
            },
        };
    }

    let c: Vec<Vec<f64>> = active
        .iter()
        .map(|p| {
            let mut row: Vec<f64> = used[..rows]
                .iter()
                .map(|k| (plans[*p][*k] - outputs[*p][*k]) / supply[*k])
                .collect();
            for index in active_budgets.iter() {
                let budget = &budgets[*index];
                row.push(budget.coefficient[*p] / budget.limit);
            }
            row
        })
        .collect();

    let system = System {
        c,
        w: active.iter().map(|p| w[*p]).collect(),
        supply: vec![1.0; used.len()],
        curvature,
        active,
        used,
        rows,
        box_weight: BOX_WEIGHT,
    };

    let mean_motive = system.w.iter().sum::<f64>() / system.w.len() as f64;
    let scale = (curvature * mean_motive).max(TINY);
    let target = (barrier * scale).max(TINY);
    let mut mu_bar = scale.max(target);
    let mut point = system.start(mu_bar);
    let mut iterations = 0usize;
    let mut phases = 0usize;
    let mut residual;
    let converged;
    let mut degraded = false;

    loop {
        phases += 1;
        let zero = Point::zeros(system.w.len(), system.used.len());
        let mut settled = false;
        residual = system.norm(&point, &zero, 0.0, 0.0, mu_bar);
        for _ in 0..NEWTON_MAX {
            if residual < NEWTON_TOL {
                settled = true;
                break;
            }
            if iterations >= NEWTON_TOTAL {
                break;
            }
            let (step, ok) = system.direction(&point, mu_bar);
            degraded |= !ok;
            let (limit_p, limit_d) = system.boundary(&point, &step);
            let mut theta = 1.0f64;
            let mut accepted = false;
            for _ in 0..BACKTRACK {
                let trial = system.norm(&point, &step, theta * limit_p, theta * limit_d, mu_bar);
                if trial <= (1.0 - SUFFICIENT * theta) * residual {
                    accepted = true;
                    break;
                }
                theta *= 0.5;
            }
            if !accepted {
                degraded = true;
                break;
            }
            point.apply(&step, theta * limit_p, theta * limit_d);
            iterations += 1;
            residual = system.norm(&point, &zero, 0.0, 0.0, mu_bar);
        }
        degraded |= !settled;
        if mu_bar <= target || iterations >= NEWTON_TOTAL {
            converged = settled;
            break;
        }
        mu_bar = (mu_bar * REDUCTION).max(target);
    }

    for (i, p) in system.active.iter().enumerate() {
        x[*p] = point.x[i];
    }
    for p in 0..policies {
        if x[p] > 0.0 {
            for k in 0..goods {
                eaten[k] += plans[p][k] * x[p];
                delivered[k] += outputs[p][k] * x[p];
            }
        }
    }
    let utilization = system
        .used
        .iter()
        .take(system.rows)
        .map(|k| {
            if supply[*k] > 0.0 {
                eaten[*k] / supply[*k]
            } else {
                0.0
            }
        })
        .fold(0.0f64, f64::max);

    Outcome {
        x,
        eaten,
        delivered,
        report: SettlementReport {
            gap: system.gap(&point),
            mu: mu_bar,
            iterations,
            converged,
            residual,
            phases,
            degraded,
            blocked: blocked.len(),
            utilization,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 11) as f64) / ((1u64 << 53) as f64)
        }

        fn range(&mut self, low: f64, high: f64) -> f64 {
            low + (high - low) * self.next()
        }
    }

    fn run(w: &[f64], plans: &[Vec<f64>], supply: &[f64]) -> Outcome {
        let outputs = vec![vec![0.0; supply.len()]; w.len()];
        solve(w, plans, &outputs, supply, &[], BARRIER, CURVATURE)
    }

    fn consumed(plans: &[Vec<f64>], x: &[f64], k: usize) -> f64 {
        (0..plans.len()).map(|p| plans[p][k] * x[p]).sum()
    }

    fn demand_root(motive: f64, mu: f64, supply: f64) -> f64 {
        let g = |x: f64| CURVATURE * motive * x.powf(CURVATURE - 1.0) + mu / x - mu / (supply - x);
        let (mut low, mut high) = (supply * 1e-12, supply * (1.0 - 1e-12));
        for _ in 0..200 {
            let mid = 0.5 * (low + high);
            if g(mid) > 0.0 {
                low = mid;
            } else {
                high = mid;
            }
        }
        0.5 * (low + high)
    }

    fn single_mu(motive: f64) -> f64 {
        BARRIER * CURVATURE * motive
    }

    #[test]
    fn the_demand_curve_matches_the_analytic_root() {
        let plans = vec![vec![1.0]];
        let mu = single_mu(1.0);
        for supply in [1.0, 1e2, 1e4, 1e6] {
            let outcome = run(&[1.0], &plans, &[supply]);
            assert!(outcome.report.converged, "{:?}", outcome.report);
            let expected = demand_root(1.0, mu, supply);
            assert!(
                (outcome.x[0] - expected).abs() <= 1e-6 * expected.abs().max(1.0),
                "存量 {supply}：数值 {} vs 解析根 {expected}",
                outcome.x[0],
            );
            assert!(
                outcome.x[0] > 0.0 && outcome.x[0] <= supply * (1.0 + 1e-9),
                "超取"
            );
        }
    }

    #[test]
    fn a_bigger_warehouse_is_eaten_deeper() {
        let plans = vec![vec![1.0]];
        let small = run(&[1.0], &plans, &[1e2]);
        let large = run(&[1.0], &plans, &[1e4]);
        assert!(
            large.x[0] > 10.0 * small.x[0],
            "存量涨 100 倍，篮子数应当大幅跟着涨：{} → {}",
            small.x[0],
            large.x[0],
        );
        assert!(
            large.report.utilization > small.report.utilization,
            "存量越大吃得越干净：{} vs {}",
            small.report.utilization,
            large.report.utilization,
        );
    }

    #[test]
    fn the_duality_gap_matches_the_theory() {
        let mut rng = Lcg(20260913);
        let policies = 4;
        let goods = 5;
        let w: Vec<f64> = (0..policies).map(|_| rng.range(0.05, 1.0)).collect();
        let plans: Vec<Vec<f64>> = (0..policies)
            .map(|_| (0..goods).map(|_| rng.range(0.0, 2.0)).collect())
            .collect();
        let supply: Vec<f64> = (0..goods).map(|_| rng.range(0.5, 50.0)).collect();
        let outcome = run(&w, &plans, &supply);
        let report = &outcome.report;
        assert!(report.converged, "未收敛：{report:?}");
        let expected = (goods + policies) as f64 * report.mu;
        assert!(
            (report.gap - expected).abs() <= 1e-6 * expected,
            "间隙 {} 与理论 {expected} 不符",
            report.gap,
        );
    }

    #[test]
    fn no_good_is_over_drawn() {
        let mut rng = Lcg(7);
        for trial in 0..200 {
            let policies = 1 + (trial % 4);
            let goods = 1 + (trial % 5);
            let w: Vec<f64> = (0..policies).map(|_| rng.range(0.01, 1.0)).collect();
            let plans: Vec<Vec<f64>> = (0..policies)
                .map(|_| (0..goods).map(|_| rng.range(0.0, 3.0)).collect())
                .collect();
            let supply: Vec<f64> = (0..goods).map(|_| rng.range(1e-9, 1e3)).collect();
            let outcome = run(&w, &plans, &supply);
            assert!(outcome.report.converged, "第 {trial} 例未收敛");
            for k in 0..goods {
                let taken = consumed(&plans, &outcome.x, k);
                assert!(
                    taken <= supply[k] * (1.0 + 1e-9),
                    "第 {trial} 例商品 {k} 超取：{taken} > {}",
                    supply[k],
                );
            }
            assert!(
                outcome.report.utilization <= 1.0 + 1e-9,
                "第 {trial} 例利用率 {} > 1",
                outcome.report.utilization,
            );
        }
    }

    #[test]
    fn a_shortage_in_one_policy_does_not_starve_another() {
        let plans = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
        let outcome = run(&[1.0, 1.0], &plans, &[0.02, 1e6]);
        assert!(outcome.report.converged, "{:?}", outcome.report);
        let mu = single_mu(1.0);
        let starved = demand_root(1.0, mu, 0.02);
        assert!(
            (outcome.x[0] - starved).abs() <= 1e-6 * starved,
            "缺货的政策应当落在自己的需求曲线上 {starved}，实际 {}",
            outcome.x[0],
        );
        assert!(outcome.x[0] < 0.05, "缺货的政策应当被压得很低");
        let rich = demand_root(1.0, mu, 1e6);
        assert!(
            (outcome.x[1] - rich).abs() <= 1e-6 * rich,
            "不缺货的政策应当落在 {rich}，实际 {}",
            outcome.x[1],
        );
    }

    #[test]
    fn a_basket_is_atomic() {
        let plans = vec![vec![1.0, 1.0]];
        let outcome = run(&[1.0], &plans, &[1e6, 0.1]);
        let eaten = &outcome.eaten;
        assert!(
            (eaten[0] - eaten[1]).abs() <= 1e-9 * eaten[0].max(1.0),
            "篮子必须按同一比例缩放，实际 {eaten:?}",
        );
        assert!(eaten[1] <= 0.1 * (1.0 + 1e-9), "超取：{eaten:?}");
        assert!(eaten[1] > 0.0, "不该整条判死：{eaten:?}");
    }

    #[test]
    fn a_basket_needing_an_absent_good_is_blocked_outright() {
        let plans = vec![vec![1.0, 1.0], vec![0.0, 1.0]];
        let outcome = run(&[1.0, 1.0], &plans, &[0.0, 1e6]);
        assert_eq!(outcome.x[0], 0.0, "缺货的篮子不该执行");
        assert_eq!(outcome.report.blocked, 1);
        assert!(outcome.x[1] > 0.0, "另一条政策不该被连坐");
    }

    #[test]
    fn a_vanishing_good_still_converges() {
        let plans = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
        for exponent in -30..=0 {
            let supply = 10f64.powi(exponent);
            let outcome = run(&[1.0, 1.0], &plans, &[supply, 1e6]);
            assert!(
                outcome.report.converged,
                "供给 1e{exponent}：残差 {:e}、{} 步、{} 档",
                outcome.report.residual, outcome.report.iterations, outcome.report.phases,
            );
        }
    }
    #[test]
    fn the_solution_is_continuous_in_the_supply() {
        let plans = vec![vec![1.0, 0.5], vec![0.25, 1.0]];
        let w = [0.7, 0.3];
        let base = run(&w, &plans, &[1.0, 1.0]);
        for step in [1e-6f64, -1e-6, 1e-3, -1e-3] {
            let bumped = run(&w, &plans, &[1.0 + step, 1.0]);
            for p in 0..2 {
                let jump = (bumped.x[p] - base.x[p]).abs();
                assert!(
                    jump < 1e-3,
                    "扰动 {step} 让政策 {p} 跳了 {jump}（{} → {}）",
                    base.x[p],
                    bumped.x[p],
                );
            }
        }
    }

    #[test]
    fn a_scarce_good_is_rationed_in_proportion_to_the_willingness() {
        let plans = vec![vec![1.0], vec![1.0]];
        let outcome = run(&[3.0, 1.0], &plans, &[0.4]);
        assert!(outcome.report.converged, "{:?}", outcome.report);
        let total = outcome.eaten[0];
        assert!(total <= 0.4 * (1.0 + 1e-9), "超取：{total}");
        assert!(
            outcome.x[0] > outcome.x[1],
            "意愿更高的政策执行率应当更高：{:?}",
            outcome.x,
        );
        let outputs = vec![vec![0.0; 1]; 2];
        let sharp = solve(
            &[3.0, 1.0],
            &plans,
            &outputs,
            &[0.4],
            &[],
            BARRIER * 1e-3,
            CURVATURE,
        );
        assert!(
            sharp.report.converged && sharp.report.utilization > outcome.report.utilization,
            "障碍调小必须吃得更干净：{} vs {}",
            sharp.report.utilization,
            outcome.report.utilization,
        );
    }
}
