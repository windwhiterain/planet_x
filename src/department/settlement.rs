//! 消费结算：把「谁吃多少」写成一个凸问题，用 **原始-对偶内点法** 解。
//!
//! 变量：`x_p ∈ (0,1)` 是政策 p 的执行率，`s_k > 0` 是商品 k 结算后的剩余存量。
//! 约束：`s_k + Σ_p c_pk·x_p = S_k`，其中 `c_pk = 分布_p × 配方_pk`。**约束就是仓库存量。**
//! 目标：`max Σ_p w_p·x_p + μ·[ Σ_k ln s_k + Σ_p ln x_p + Σ_p ln(1 − x_p) ]`，`w_p = 分布_p`。
//!
//! # 为什么不是那个硬 `min`
//!
//! 纯 LP（去掉障碍）的解在**同构部门**下退化成一整个面：几条政策的配方一样、意愿一样时，
//! 谁吃多少完全由遍历顺序决定。更要命的是硬配给 `share = min(S_k/需求_k)` 是**不连续**的，
//! 而且过去是**跨所有商品取 min 再跨所有政策共用一个标量**——一条政策缺货，全部政策连坐。
//! 对数障碍让解严格落在内部、唯一、对数据连续；`s_k > 0` 还在**结构上**排除了超取。
//!
//! # μ 是什么量纲
//!
//! 单政策、库存无限时 KKT 给出 `w = μ(2x−1)/(x(1−x))`，也就是 `w·x² − (w−2μ)x − μ = 0`
//! 在 `(0,1)` 内的那个根；一阶近似是 `x ≈ 1 − μ/w`（`μ/w = 0.1` 时精确根 `0.909902`、
//! 近似 `0.9`）。所以 `μ/w` 读作**无约束时大致不吃的那部分**。取 `μ = BARRIER × w̄`，
//! `BARRIER` 就是"软化程度"本身，不是拟合出来的常数：它同时是"留多少余量"和"解离角点多远"。
//!
//! # 为什么必须解**耦合**的 KKT，而不是交替迭代
//!
//! 上一版用 `λ_k = μ/s_k` 与 `x_p` 交替迭代（Gauss-Seidel）。那个 Jacobian 在 `s → 0`
//! 时爆炸：`λ` 巨大 ⇒ `x` 被顶到 0 ⇒ `s` 变大 ⇒ `λ` 变小 ⇒ `x` 跳到 1 ⇒ 来回振荡。
//! 另外那一版的定常性方程符号是**反的**（写成 `w − μ/x + μ/(1−x) = shadow`，正确是
//! `w + μ/x − μ/(1−x) = shadow`），于是影子价格为零时解落在 `x < 0.5` 一侧、`μ→0` 时
//! `x → 0`——正是实测到的"三产摄入 0.025、execution 0.000"。正确符号下 `x → 1 − μ/w`。
//!
//! 这里解的是完整系统
//!
//! ```text
//! 定常性   w_p − Σ_k c_pk·λ_k + y_p − z_p = 0
//! 原始可行 s_k + Σ_p c_pk·x_p − S_k       = 0
//! 中心化   s_k·λ_k = σμ,  x_p·y_p = σμ,  (1−x_p)·z_p = σμ
//! ```
//!
//! **乘子的符号是这套东西里唯一容易写错的地方**，判据是角点：`max x, 0 ≤ x ≤ 1` 在
//! `x = 1` 处必须给出 `z = w > 0, y = 0`，在 `x = 0`（`w < 0`）处必须给出
//! `y = −w > 0, z = 0`。按这个判据推出来的是**下界乘子取 `+y`、上界取 `−z`**。
//! 写成 `−y + z` 会把解翻到障碍的另一侧——实测无约束政策会收敛到 `x = 0.011`
//! 而不是解析解 `1 − μ/w = 0.9`，症状和上一版那个"三产不吃饭"一模一样。
//!
//! 牛顿步消去 `Δs, Δy, Δz` 之后是一个 `P×P` 的 **Schur 补** `S = diag(x/y + z/(1−x)) + C·diag(λ/s)·Cᵀ`，
//! 对称正定 ⇒ 用对角缩放后的 Cholesky 解，不需要选主元。步长取分数步长
//! `α = 0.99·α_max`（把 `x, 1−x, s, λ, y, z` 全部留在正侧），再按残差范数回溯。
//! `μ` 沿 `σ = 0.2` 的路径降到目标值，最后在目标 `μ` 上把残差压到 `1e-9`。
//!
//! 收敛证书是对偶间隙 `gap = Σ_k s_kλ_k + Σ_p [x_p y_p + (1−x_p)z_p]`，
//! 理论上等于 `(K + 2P)·μ`——测试里就是这么验的。

use super::SettlementReport;

/// 障碍强度：无约束时每条政策只吃 `1 − BARRIER` 的篮子
pub(super) const BARRIER: f64 = 0.1;

/// 单个 μ 上的最大牛顿步
const NEWTON_MAX: usize = 80;
/// 一次结算的总牛顿步上限（超出即报告未收敛，不静默继续）
const NEWTON_TOTAL: usize = 400;
/// 相对残差容差
const NEWTON_TOL: f64 = 1e-9;
/// μ 的下降比
const REDUCTION: f64 = 0.2;
/// 分数步长：离边界留 1%
const BOUNDARY: f64 = 0.99;
/// 回溯次数
const BACKTRACK: usize = 60;
/// 回溯的充分下降系数
const SUFFICIENT: f64 = 1e-4;
/// 只用来挡住除零；f64 的最小正规数
const TINY: f64 = 1e-300;

pub(super) struct Outcome {
    /// 逐政策执行率，index 对齐调用方的政策表；未选中/被判死的政策是 0
    pub x: Vec<f64>,
    /// 逐商品实际消耗
    pub eaten: Vec<f64>,
    pub report: SettlementReport,
}

/// 变量的一整套取值
#[derive(Clone)]
struct Point {
    x: Vec<f64>,
    s: Vec<f64>,
    lambda: Vec<f64>,
    y: Vec<f64>,
    z: Vec<f64>,
}

impl Point {
    fn zeros(policies: usize, goods: usize) -> Self {
        Self {
            x: vec![0.0; policies],
            s: vec![0.0; goods],
            lambda: vec![0.0; goods],
            y: vec![0.0; policies],
            z: vec![0.0; policies],
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
            self.z[i] += alpha_d * step.z[i];
        }
    }
}

/// 参与求解的那部分问题：活跃政策 × 有库存的商品
struct System {
    /// 活跃政策在完整政策表里的下标
    active: Vec<usize>,
    /// 进入约束的商品在完整商品表里的下标（存量 > 0）
    used: Vec<usize>,
    /// `c[p][k]`，局部下标
    c: Vec<Vec<f64>>,
    w: Vec<f64>,
    supply: Vec<f64>,
}

impl System {
    /// 残差的缩放无穷范数。`step` 可以为全零向量，用来评价当前点。
    ///
    /// **每一项都按自身的量纲相对化**，这是这套东西能不能收敛的关键。定常性方程是
    /// `w_p − Σ_k c_pk λ_k + y_p − z_p = 0`，所以它的自然量纲不是 `w_p`，而是
    /// **这一行里最大的那一项**：某条政策几乎无货可吃时 `y_p = μ/x_p` 能到 `1e29`，
    /// 拿 `w_p = 1` 当分母就等于要求 38 位有效数字——f64 只有 16 位，任何实现都到不了，
    /// 实测牛顿会在 `残差 ≈ 1` 上打满 240 步（`--motive-ladder` 二产归零前的
    /// `converged = false` 就是这个）。
    fn norm(&self, point: &Point, step: &Point, alpha_p: f64, alpha_d: f64, mu_bar: f64) -> f64 {
        let mut worst = 0.0f64;
        for p in 0..self.w.len() {
            let x = point.x[p] + alpha_p * step.x[p];
            let y = point.y[p] + alpha_d * step.y[p];
            let z = point.z[p] + alpha_d * step.z[p];
            let mut shadow = 0.0f64;
            for k in 0..self.used.len() {
                shadow += self.c[p][k] * (point.lambda[k] + alpha_d * step.lambda[k]);
            }
            let stationarity = self.w[p] - shadow + y - z;
            let magnitude = self.w[p] + shadow.abs() + y.abs() + z.abs();
            worst = worst.max((stationarity / magnitude.max(TINY)).abs());
            worst = worst.max(((x * y - mu_bar) / mu_bar).abs());
            worst = worst.max((((1.0 - x) * z - mu_bar) / mu_bar).abs());
        }
        for k in 0..self.used.len() {
            let s = point.s[k] + alpha_p * step.s[k];
            let lambda = point.lambda[k] + alpha_d * step.lambda[k];
            let mut consumed = 0.0f64;
            for p in 0..self.w.len() {
                consumed += self.c[p][k] * (point.x[p] + alpha_p * step.x[p]);
            }
            // 行已经按存量归一，所以约束的两侧都是 O(1)
            let feasibility = s + consumed - 1.0;
            worst = worst.max((feasibility / (1.0 + s.abs() + consumed.abs())).abs());
            worst = worst.max(((s * lambda - mu_bar) / mu_bar).abs());
        }
        worst
    }

    /// 当前点的对偶间隙
    fn gap(&self, point: &Point) -> f64 {
        let mut gap = 0.0f64;
        for k in 0..self.used.len() {
            gap += point.s[k] * point.lambda[k];
        }
        for p in 0..self.w.len() {
            gap += point.x[p] * point.y[p] + (1.0 - point.x[p]) * point.z[p];
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

    /// 严格可行的起点：逐政策取自己最紧的那样货，再全局压一次保证联合可行
    fn start(&self, mu: f64) -> Point {
        let policies = self.w.len();
        let goods = self.used.len();
        let mut point = Point::zeros(policies, goods);
        let columns: Vec<f64> = (0..goods)
            .map(|k| (0..policies).map(|p| self.c[p][k]).sum::<f64>())
            .collect();
        for p in 0..policies {
            let mut share = 1.0f64;
            for k in 0..goods {
                if self.c[p][k] > 0.0 && columns[k] > 0.0 {
                    share = share.min(self.supply[k] / columns[k]);
                }
            }
            point.x[p] = (0.99 * share).clamp(TINY, 0.99);
        }
        let mut worst = 1.0f64;
        for k in 0..goods {
            let consumed = self.consumed(&point.x, k);
            if consumed > 0.0 {
                worst = worst.max(consumed / (0.98 * self.supply[k]));
            }
        }
        for p in 0..policies {
            point.x[p] = (point.x[p] / worst).max(TINY);
        }
        for k in 0..goods {
            let consumed = self.consumed(&point.x, k);
            let slack = (self.supply[k] - consumed).max(TINY);
            point.s[k] = slack;
            point.lambda[k] = mu / slack;
        }
        for p in 0..policies {
            point.y[p] = mu / point.x[p];
            point.z[p] = mu / (1.0 - point.x[p]).max(TINY);
        }
        point
    }

    /// 牛顿方向。返回 `(方向, Cholesky 是否成功)`。
    fn direction(&self, point: &Point, mu_bar: f64) -> (Point, bool) {
        let policies = self.w.len();
        let goods = self.used.len();

        let mut diagonal = vec![0.0f64; policies];
        let mut bound_e = vec![0.0f64; policies];
        let mut bound_h = vec![0.0f64; policies];
        let mut bound_b = vec![0.0f64; policies];
        let mut bound_g = vec![0.0f64; policies];
        let mut rhs = vec![0.0f64; policies];
        for p in 0..policies {
            let x = point.x[p];
            let y = point.y[p];
            let z = point.z[p];
            let mut shadow = 0.0f64;
            for k in 0..goods {
                shadow += self.c[p][k] * point.lambda[k];
            }
            let stationarity = self.w[p] - shadow + y - z;
            // `Δy = b − (y/x)·Δx`、`Δz = g + (z/(1−x))·Δx`，所以对角块是
            // `y/x + z/(1−x)`——它正是障碍的 `−Hessian`（`σμ = μ` 时与
            // "把障碍写进目标"那一版逐项相同，这是两套写法一致的判据）。
            let e = y / x;
            let h = z / (1.0 - x).max(TINY);
            let b = (mu_bar - x * y) / x;
            let g = (mu_bar - (1.0 - x) * z) / (1.0 - x).max(TINY);
            diagonal[p] = e + h;
            bound_e[p] = e;
            bound_h[p] = h;
            bound_b[p] = b;
            bound_g[p] = g;
            rhs[p] = stationarity + b - g;
        }

        let mut inverse_slack = vec![0.0f64; goods];
        let mut slack_step = vec![0.0f64; goods];
        let mut slack_rhs = vec![0.0f64; goods];
        for k in 0..goods {
            let s = point.s[k];
            let lambda = point.lambda[k].max(TINY);
            let consumed = self.consumed(&point.x, k);
            let feasibility = s + consumed - self.supply[k];
            let a = (mu_bar - s * lambda) / lambda;
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
                (0..policies).map(|p| rhs[p] / schur[p][p]).collect::<Vec<f64>>(),
                false,
            ),
        };

        let mut step = Point::zeros(policies, goods);
        for p in 0..policies {
            step.x[p] = solution[p];
            step.y[p] = bound_b[p] - bound_e[p] * solution[p];
            step.z[p] = bound_g[p] + bound_h[p] * solution[p];
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

    /// 分数步长：把 `x, 1−x, s` 与 `λ, y, z` 都留在正侧
    fn boundary(&self, point: &Point, step: &Point) -> (f64, f64) {
        let mut primal = 1.0f64;
        let mut dual = 1.0f64;
        for p in 0..self.w.len() {
            let dx = step.x[p];
            if dx < 0.0 {
                primal = primal.min(-point.x[p] / dx);
            } else if dx > 0.0 {
                primal = primal.min((1.0 - point.x[p]) / dx);
            }
            if step.y[p] < 0.0 {
                dual = dual.min(-point.y[p] / step.y[p]);
            }
            if step.z[p] < 0.0 {
                dual = dual.min(-point.z[p] / step.z[p]);
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
        (
            (BOUNDARY * primal).min(1.0),
            (BOUNDARY * dual).min(1.0),
        )
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

/// 结算一次。`w[p]` 是政策的意愿份额，`plans[p][k]` 是 `分布 × 配方`，`supply[k]` 是本轮可用存量。
pub(super) fn solve(w: &[f64], plans: &[Vec<f64>], supply: &[f64], barrier: f64) -> Outcome {
    let policies = w.len();
    let goods = supply.len();
    let mut x = vec![0.0f64; policies];
    let mut eaten = vec![0.0f64; goods];

    // 配方里要一样**存量已经是 0** 的货，整条政策就执行不了：篮子是不可分的。
    // （这和"缺货按比例少吃"不是一回事——按比例少吃是存量 > 0 时的连续配给。）
    let mut blocked = Vec::new();
    let mut active = Vec::new();
    for p in 0..policies {
        if !(w[p] > 0.0) || !plans[p].iter().any(|c| *c > 0.0) {
            continue;
        }
        let missing = (0..goods).any(|k| plans[p][k] > 0.0 && !(supply[k] > 0.0));
        if missing {
            blocked.push(p);
        } else {
            active.push(p);
        }
    }
    // 没有存量的商品不构成约束（`s_k = 0`，无货可留）
    let used: Vec<usize> = (0..goods).filter(|k| supply[*k] > 0.0).collect();

    if active.is_empty() || used.is_empty() {
        return Outcome {
            x,
            eaten,
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

    let system = System {
        // **逐商品把约束行按存量归一**：`ŝ_k = s_k/S_k`、`ĉ_pk = c_pk/S_k`，于是约束是
        // `ŝ_k + Σ_p ĉ_pk x_p = 1`、`ŝ ∈ (0,1)`。这不是近似——障碍项
        // `μ·ln ŝ = μ·ln s − μ·ln S`，差一个与 `x` 无关的常数，`argmax` 逐字不变。
        //
        // 为什么非做不可：实测存量 `1e-30` 时（`--motive-ladder` 二产归零前那几轮）
        // 未归一版的 `s` 会被步长推到 f64 下溢，`λ/s` 溢出成 `inf`，Cholesky 直接失败，
        // 牛顿在 `残差 = 5.2e-2` 上打满 240 步也下不来。归一之后 `ŝ` 起点在
        // `[0.02, 1]`、解也在 `O(1)`，整个"存量跨 30 个数量级"的区间被压平。
        c: active
            .iter()
            .map(|p| used.iter().map(|k| plans[*p][*k] / supply[*k]).collect())
            .collect(),
        w: active.iter().map(|p| w[*p]).collect(),
        supply: vec![1.0; used.len()],
        active,
        used,
    };

    let mean_w = system.w.iter().sum::<f64>() / system.w.len() as f64;
    let target = (barrier * mean_w).max(TINY);
    let mut mu_bar = mean_w.max(target);
    let mut point = system.start(mu_bar);
    let mut iterations = 0usize;
    let mut phases = 0usize;
    let mut residual = f64::INFINITY;
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
                let trial =
                    system.norm(&point, &step, theta * limit_p, theta * limit_d, mu_bar);
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
        // 只有**降到目标 μ** 才算解完：中途收敛只说明这一档走完了。
        // （第一版这里写反了——`settled` 一到就 break，于是解永远停在 `μ = 平均 w` 那一档：
        // 实测单政策无约束场景返回 `x = 0.618034 = 1/φ`，那正是 `μ = w` 时的解析解
        // `x(1−x) = 2x−1`。数值和解析对到 8 位，反倒验证了整套路子是对的。）
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
            }
        }
    }
    let utilization = system
        .used
        .iter()
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

    /// 固定序列的伪随机，测试要可复现
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> f64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((self.0 >> 11) as f64) / ((1u64 << 53) as f64)
        }

        fn range(&mut self, low: f64, high: f64) -> f64 {
            low + (high - low) * self.next()
        }
    }

    fn run(w: &[f64], plans: &[Vec<f64>], supply: &[f64]) -> Outcome {
        solve(w, plans, supply, BARRIER)
    }

    fn consumed(plans: &[Vec<f64>], x: &[f64], k: usize) -> f64 {
        (0..plans.len()).map(|p| plans[p][k] * x[p]).sum()
    }

    /// 单政策、库存无限时的**解析解**：`w = μ(2x−1)/(x(1−x))` 在 `(0,1)` 内的根，
    /// 即 `w·x² − (w − 2μ)·x − μ = 0`。注意它**不是** `1 − μ/w`——那只是一阶近似：
    /// `μ/w = 0.1` 时精确根是 `0.909902`，一阶近似给 `0.9`，差 1%。
    fn analytic_root(w: f64, mu: f64) -> f64 {
        let b = w - 2.0 * mu;
        ((b + (b * b + 4.0 * w * mu).sqrt()) / (2.0 * w)).clamp(0.0, 1.0)
    }

    /// 单政策、库存无限：解必须落在解析根上
    ///
    /// 库存取 `1e12` 是为了让影子价格 `λ = μ/S = 1e-13` 远小于容差——上面的解析根
    /// 是 `λ = 0` 的极限。库存只有 `1e6` 时影子价格是 `2e-7`，解会**正确地**偏开
    /// `1.6e-8`，那是模型的一部分，不是误差。
    #[test]
    fn an_unconstrained_policy_converges_to_the_analytic_root() {
        let plans = vec![vec![1.0, 1.0]];
        let outcome = run(&[1.0], &plans, &[1e12, 1e12]);
        assert!(outcome.report.converged, "{:?}", outcome.report);
        let expected = analytic_root(1.0, BARRIER);
        assert!(
            (outcome.x[0] - expected).abs() < 1e-8,
            "无约束执行率应为解析根 {expected}，实际 {}",
            outcome.x[0],
        );
        // `1 − μ/w` 只是一阶近似，别拿它当判据
        assert!(
            (expected - (1.0 - BARRIER)).abs() > 1e-3,
            "解析根不该等于一阶近似",
        );
    }

    /// 收敛证书：对偶间隙 = (K + 2P)·μ
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
        let expected = (goods + 2 * policies) as f64 * report.mu;
        assert!(
            (report.gap - expected).abs() <= 1e-6 * expected,
            "间隙 {} 与理论 {expected} 不符",
            report.gap,
        );
    }

    /// 性质一：不超取（结构上由 `s_k > 0` 保证）
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
            // 存量跨 12 个数量级，复现实际系统里的量纲
            let supply: Vec<f64> = (0..goods)
                .map(|_| rng.range(1e-9, 1e3))
                .collect();
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

    /// 性质二：政策彼此独立。一条政策缺货不得把另一条不缺货的政策一起按下去。
    #[test]
    fn a_shortage_in_one_policy_does_not_starve_another() {
        // 政策 0 要商品 0（只剩一点点），政策 1 只要商品 1（要多少有多少）
        let plans = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
        let outcome = run(&[1.0, 1.0], &plans, &[0.02, 1e6]);
        assert!(outcome.report.converged, "{:?}", outcome.report);
        assert!(
            outcome.x[0] < 0.05,
            "缺货的政策应当被压到很低，实际 {}",
            outcome.x[0],
        );
        assert!(
            (outcome.x[1] - analytic_root(1.0, BARRIER)).abs() < 1e-3,
            "不缺货的政策应当照常吃满，实际 {}",
            outcome.x[1],
        );
    }

    /// 性质三：篮子不可分——一条政策只有一个执行率，配方按同一个比例缩放
    #[test]
    fn a_basket_is_atomic() {
        // 政策要 (1, 1)，商品 1 只剩 0.1
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

    /// 性质三的边界：配方里要一样**存量恰好为 0** 的货，整条政策执行不了
    #[test]
    fn a_basket_needing_an_absent_good_is_blocked_outright() {
        let plans = vec![vec![1.0, 1.0], vec![0.0, 1.0]];
        let outcome = run(&[1.0, 1.0], &plans, &[0.0, 1e6]);
        assert_eq!(outcome.x[0], 0.0, "缺货的篮子不该执行");
        assert_eq!(outcome.report.blocked, 1);
        assert!(outcome.x[1] > 0.0, "另一条政策不该被连坐");
    }

    /// 存量小到"几乎没有"时也必须解得出来。
    ///
    /// 这是实测踩到的坑：`--motive-ladder` 在二产归零的那几轮里，牛顿步在
    /// `残差 = 5.2e-2` 上停住（240 步打满、`converged = false`），随后整个经济
    /// 掉进"二产为 0 ⇒ 两条政策整篮判死"的死态。供给越接近零，约束的两侧
    /// 越是 38 个数量级的悬殊（`x ~ S`，而另一条政策 `x ~ 1`），这套缩放要先看清
    /// 断点在哪一档。
    #[test]
    fn a_vanishing_good_still_converges() {
        let plans = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
        for exponent in -30..=0 {
            let supply = 10f64.powi(exponent);
            let outcome = run(&[1.0, 1.0], &plans, &[supply, 1e6]);
            assert!(
                outcome.report.converged,
                "供给 1e{exponent}：残差 {:e}、{} 步、{} 档",
                outcome.report.residual,
                outcome.report.iterations,
                outcome.report.phases,
            );
        }
    }
    /// 性质四：连续——存量微扰不得让执行率跳变
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

    /// 稀缺时的两条可检验性质：**意愿高的执行率高**，而且**障碍越小吃得越干净**。
    ///
    /// 注意这里不检查"存量被吃干"这种绝对量——障碍的解本来就留一个 `s ≈ μ/λ` 的余量，
    /// 余量随 `μ` 单调下降才是这套写法真正承诺的东西。
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
        let sharp = solve(&[3.0, 1.0], &plans, &[0.4], BARRIER * 1e-3);
        assert!(
            sharp.report.converged && sharp.report.utilization > outcome.report.utilization,
            "障碍调小必须吃得更干净：{} vs {}",
            sharp.report.utilization,
            outcome.report.utilization,
        );
    }
}
