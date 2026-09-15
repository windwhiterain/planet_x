//! 消费结算：把「谁吃多少」写成一个凸问题，用 **原始-对偶内点法** 解。
//!
//! 变量：`x_p > 0` 是政策 p 跑几篮，`s_k > 0` 是商品 k 结算后的剩余存量。
//! 约束：`s_k + Σ_p 配方_pk·x_p = S_k`。**约束就是仓库存量，上界只有它。**
//! 目标：`max Σ_p motive_p·x_p^θ + μ·[ Σ_k ln s_k + Σ_p ln x_p ]`，`θ = CURVATURE`。
//!
//! # 为什么没有上界盒子，也没有归一化的 distribution
//!
//! 旧版的目标是 `Σ_p 分布_p·x_p`（**线性**），`x_p ∈ (0,1)`。线性 + 正系数 ⇒ 只要那样货的
//! 影子价格低于 `分布_p`，最优就是 `x_p = 1`：**要多少吃多少，跟价格无关**。而且
//! `分布_p` 是先归一化到和为 1 再喂进来的，同一个数既当目标系数又当物质量
//! （`c_pk = 分布_p × 配方_pk`），于是 **motive 的绝对大小完全不起作用**：全乘 1000 结果逐位不变。
//! 实测：商品 0 从第 150 轮到第 2000 轮 `wanted_buy = 9` 不变、缺口恒定 `≈14.5`，
//! 而买价从 `0.166` 走到 `2.133e-21`——**价格动了 19 个数量级，需求量一点没动**。
//!
//! 现在：`motive` 直接进目标（绝对值），约束用**裸配方**，`x` 读作"几篮"，
//! 混合与规模都由解给出。`u(x) = x^θ` 是凹的 ⇒ 边际效用 `θ·motive·x^{θ−1}` 随 `x` 递减，
//! 与影子价格 `Σ_k 配方_pk·λ_k` 平衡 ⇒ **需求成了一条曲线**：库存多 ⇒ `λ = μ/s` 小 ⇒ 多吃几篮；
//! 库存紧 ⇒ `λ` 大 ⇒ 自动少吃。上界不再需要——`λ > 0` **恒成立**（只要还有存货，
//! `λ_k = μ/s_k > 0`），所以这个平衡点永远有限。
//!
//! `u' = θ·x^{θ−1}`：`x→0` 时无穷（每条政策总会吃一点，不会整条不执行），
//! `x→1` 时趋于 `θ`（有界，任何正影子价格都压得住）。`θ → 1` 退回线性（无边际效应）。
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
//! `x = 1` 处的边际效用是 `θ·motive`，所以障碍取 `μ = BARRIER × θ × 平均 motive`。
//! `BARRIER` 读作"障碍相对边际效用有多强"：它同时决定库存留多少余量（`s = μ/λ`）
//! 和解离边界多远。
//!
//! 单政策、单商品、`c = 1`、存量 `S` 时 KKT 是 `θ·motive·x^{θ−1} + μ/x = μ/(S − x)`：
//! 左边是边际效用加下界障碍，右边是影子价格。**这就是需求曲线**，测试里对它独立二分求根。
//!
//! # 为什么必须解**耦合**的 KKT，而不是交替迭代
//!
//! 上一版用 `λ_k = μ/s_k` 与 `x_p` 交替迭代（Gauss-Seidel）。那个 Jacobian 在 `s → 0`
//! 时爆炸：`λ` 巨大 ⇒ `x` 被顶到 0 ⇒ `s` 变大 ⇒ `λ` 变小 ⇒ `x` 跳回来 ⇒ 来回振荡。
//!
//! 这里解的是完整系统
//!
//! ```text
//! 定常性   f'(x_p) − Σ_k c_pk·λ_k + y_p = 0        f(x) = motive·x^θ
//! 原始可行 s_k + Σ_p c_pk·x_p − S_k     = 0
//! 中心化   s_k·λ_k = σμ,  x_p·y_p = σμ
//! ```
//!
//! **下界乘子取 `+y`**，判据是 `max f(x), x ≥ 0`：`f` 还在涨时 `y = 0`，涨不动了才 `y > 0`，
//! 所以定常性是 `f'(x) + y = Σcλ`。写成 `f'(x) − y = …` 会把解翻到障碍另一侧——
//! 上一版实测无约束政策收敛到 `x = 0.011` 而不是 `1 − μ/w = 0.9`，症状就是"三产不吃饭"。
//! 上界盒子已经删掉，所以那一侧的乘子 `z` 与 `(1−x)z = σμ` 整条消失。
//!
//! 牛顿步消去 `Δs, Δy` 之后是一个 `P×P` 的 **Schur 补**
//! `S = diag(y/x − f''(x)) + C·diag(λ/s)·Cᵀ`；`−f''(x) = θ(1−θ)·motive·x^{θ−2} > 0`，
//! 两项都正 ⇒ 对称正定 ⇒ 用对角缩放后的 Cholesky 解，不需要选主元。步长取分数步长
//! `α = 0.99·α_max`（把 `x, s, λ, y` 全部留在正侧），再按残差范数回溯。
//! `μ` 沿 `σ = 0.2` 的路径降到目标值，最后在目标 `μ` 上把残差压到 `1e-9`。
//!
//! 收敛证书是对偶间隙 `gap = Σ_k s_kλ_k + Σ_p x_p y_p`，理论上等于 `(K + P)·μ`。

use super::SettlementReport;

/// 障碍强度：目标是 `Σ motive·x^θ`，目标量纲取 `θ × 平均 motive`（`x = 1` 处的边际效用）
pub(super) const BARRIER: f64 = 0.1;

/// 边际效应的曲率：`u(x) = x^θ`。`θ = 1` 退回线性（需求对价格完全无弹性）
pub(super) const CURVATURE: f64 = 0.5;

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
/// 产能那一行的障碍权重。
///
/// 商品行是**存量**约束（`s_k` 是期末库存），障碍必须留出可观的一份，否则政策会把
/// 货吃干；产能行是**预算**约束（`s` 是没用掉的那部分预算），把它和存量行同一个权重
/// 会让每轮**系统性地少用约 10% 的产能**（实测经典三部门环里产量 4 → 3.605，
/// `modern` 里 `capacity_scale` 顶在 0.955）。§18.7 早就点名了这条出路：两类约束的
/// 障碍权重必须拆开。取 1e-3 等于"预算用到千分之一以内"。
const BOX_WEIGHT: f64 = 1e-4;

pub(super) struct Outcome {
    /// 逐政策**跑了几篮**，index 对齐调用方的政策表；未选中/被判死的政策是 0
    pub x: Vec<f64>,
    /// 逐商品实际消耗
    pub eaten: Vec<f64>,
    /// 逐商品实际产出
    pub delivered: Vec<f64>,
    pub report: SettlementReport,
}

/// 变量的一整套取值
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

/// 参与求解的那部分问题：活跃政策 × 有库存的商品
struct System {
    /// 活跃政策在完整政策表里的下标
    active: Vec<usize>,
    /// 进入约束的行在完整商品表里的下标（存量 > 0）；产能行用 `goods` 当哨兵
    used: Vec<usize>,
    /// 真实商品行数；`used[rows..]` 是产能那条虚拟行
    rows: usize,
    /// 产能行的障碍权重（商品行恒为 1），见 [`BOX_WEIGHT`]
    box_weight: f64,
    /// `c[p][k]`，局部下标
    c: Vec<Vec<f64>>,
    w: Vec<f64>,
    supply: Vec<f64>,
    /// `u(x) = x^θ` 的 θ，运行时给，方便扫参
    curvature: f64,
}

impl System {
    /// 边际效用 `f'(x) = θ·motive·x^{θ−1}`
    fn marginal(&self, p: usize, x: f64) -> f64 {
        self.curvature * self.w[p] * x.max(TINY).powf(self.curvature - 1.0)
    }

    /// 边际效用对 `x` 的导数 `f''(x) = θ(θ−1)·motive·x^{θ−2}`（`θ < 1` 时恒负）
    fn marginal_slope(&self, p: usize, x: f64) -> f64 {
        self.curvature * (self.curvature - 1.0) * self.w[p] * x.max(TINY).powf(self.curvature - 2.0)
    }

    /// 影子价格 `Σ_k c_pk·λ_k`
    fn shadow(&self, p: usize, lambda: &[f64]) -> f64 {
        let mut total = 0.0f64;
        for k in 0..self.used.len() {
            total += self.c[p][k] * lambda[k];
        }
        total
    }

    /// 残差的缩放无穷范数。`step` 可以为全零向量，用来评价当前点。
    ///
    /// **每一项都按自身的量纲相对化**，这是这套东西能不能收敛的关键。定常性方程是
    /// `f'(x_p) − Σ_k c_pk λ_k + y_p = 0`，所以它的自然量纲不是 `motive_p`，而是
    /// **这一行里最大的那一项**：某条政策几乎无货可吃时 `y_p = μ/x_p` 能到 `1e29`，
    /// 拿小额的分母就等于要求 38 位有效数字——f64 只有 16 位，任何实现都到不了，
    /// 实测牛顿会在 `残差 ≈ 1` 上打满 240 步（`--motive-ladder` 二产归零前的
    /// `converged = false` 就是这个）。
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
            // 行已经按存量归一，所以约束的两侧都是 O(1)
            let feasibility = s + consumed - 1.0;
            worst = worst.max((feasibility / (1.0 + s.abs() + consumed.abs())).abs());
            let target = self.row_weight(k) * mu_bar;
            worst = worst.max(((s * lambda - target) / target).abs());
        }
        worst
    }

    /// 当前点的对偶间隙：`Σ_k ω_k·s_kλ_k + Σ_p x_p y_p = (Σω + P)·μ`
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

    /// 这一行的障碍权重：商品行 1，产能行 [`BOX_WEIGHT`]
    fn row_weight(&self, k: usize) -> f64 {
        if k < self.rows {
            1.0
        } else {
            self.box_weight
        }
    }

    /// 严格可行的起点：每条政策先取**边际项 = 影子价格**的领头平衡，再逐样货压到可行。
    ///
    /// 左边有两项，取哪一项当领头要看 `x` 在哪一侧（`ŝ = 1/2` 时）：
    /// - 边际效用领先：`θ·m·x^{θ−1} = 2·ĉ·μ` ⇒ `x = (θ·m/(2·ĉ·μ))^{1/(1−θ)}`
    /// - 下界障碍领先：`μ/x = 2·ĉ·μ` ⇒ `x = 1/(2·ĉ)`
    ///
    /// 两者取**大**（和式由大的那项主导），再逐商品取小。
    ///
    /// **只取第一支是错的**：`x` 小时 `μ/x` 才是主导项，实测存量 `1e-30` 那条
    /// 起点被算成 `2.5e-61`，真解是 `5e-31`——差 30 个数量级，牛顿爬不回来。
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
        // 逐样货压：**只压用它的那些政策**。全局压一次是错的——最紧那样货（存量巨大、
        // 配方系数极小 ⇒ `ĉ` 极小 ⇒ 领头平衡极大）会把另一条根本不用它的政策一起压下去，
        // 实测把 `x₀` 从自己的平衡 `0.0185` 压到 `3.9e-10`，然后牛顿要爬 7 个数量级回来。
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

    /// 牛顿方向。返回 `(方向, Cholesky 是否成功)`。
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
            // `Δy = b − (y/x)·Δx`，所以 Schur 对角是 `y/x − f''(x)`；`θ < 1` 时
            // `−f''(x) = θ(1−θ)·motive·x^{θ−2} > 0`，两项都正 ⇒ 对称正定。
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
                (0..policies).map(|p| rhs[p] / schur[p][p]).collect::<Vec<f64>>(),
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

    /// 分数步长：把 `x, s` 与 `λ, y` 都留在正侧（上界盒子删掉之后 `1−x` 那一条没有了）
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

/// 一条**预算行**：`Σ_p coefficient[p]·x_p ≤ limit`。
///
/// 产能与现金都写成这种行——它们都不是商品存量，但同样该限制活动规模。
/// 行的障碍权重取 [`BOX_WEIGHT`]（预算不是存量），所以预算会被用到近乎满。
pub(super) struct Budget<'a> {
    pub coefficient: &'a [f64],
    pub limit: f64,
}

/// 结算一次。`w[p]` 是政策的目标权重（消费＝每篮效用、生产＝每篮净货币价值），
/// `plans[p][k]` 是**裸配方**，`supply[k]` 是本轮可用存量。返回的 `x[p]` 是**跑了几篮**。
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

    // 配方里要一样**存量已经是 0** 的货，整条政策就执行不了：篮子是不可分的。
    // （这和"缺货按比例少吃"不是一回事——按比例少吃是存量 > 0 时的连续配给。）
    let mut blocked = Vec::new();
    let mut active = Vec::new();
    for p in 0..policies {
        // **空投入的政策是合法的**：零投入的主生产就是一条。旧版这里还要求
        // `plans[p]` 里至少有一项 > 0，那是"结算只管消费"时代的残留；生产并入之后
        // 它会把主生产政策静默丢掉，于是那个商品从此再也没有产出。
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
    // 预算行放在早退判据**之前**：一个**什么东西都没有、却有产能（或现金）**的部门
    // 靠这些行才解得出来。放在后面会让这种部门直接返回零解。
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

    // **逐商品把约束行按存量归一**：`ŝ_k = s_k/S_k`、`ĉ_pk = c_pk/S_k`，于是约束是
    // `ŝ_k + Σ_p ĉ_pk x_p = 1`、`ŝ ∈ (0,1)`。这不是近似——障碍项
    // `μ·ln ŝ = μ·ln s − μ·ln S`，差一个与 `x` 无关的常数，`argmax` 逐字不变。
    //
    // 为什么非做不可：实测存量 `1e-30` 时（`--motive-ladder` 二产归零前那几轮）
    // 未归一版的 `s` 会被步长推到 f64 下溢，`λ/s` 溢出成 `inf`，Cholesky 直接失败，
    // 牛顿在 `残差 = 5.2e-2` 上打满 240 步也下不来。归一之后 `ŝ` 起点在
    // `[0.02, 1]`、解也在 `O(1)`，整个"存量跨 30 个数量级"的区间被压平。
    //
    // 系数用**净配方**（投入 − 产出）：产出也是解给出的，不能预先加进 `supply`，
    // 否则投入与产出的比例会碎。净系数为负时 `ŝ` 变大，障碍 `ln ŝ > 0` 直接就是
    // "期末库存为正"。
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

    // 障碍的量纲取 `x = 1` 处的边际效用 `θ × 平均 motive`，`BARRIER` 是它相对边际效用的倍数。
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
        let outputs = vec![vec![0.0; supply.len()]; w.len()];
        solve(
            w,
            plans,
            &outputs,
            supply,
            &[],
            BARRIER,
            CURVATURE,
        )
    }

    fn consumed(plans: &[Vec<f64>], x: &[f64], k: usize) -> f64 {
        (0..plans.len()).map(|p| plans[p][k] * x[p]).sum()
    }

    /// 单政策、单商品（配方系数 1）时的**需求曲线**：
    /// `θ·motive·x^{θ−1} + μ/x = μ/(S − x)`——左边是边际效用加下界障碍，右边是影子价格。
    /// 左边随 `x` 严格递减、右边严格递增，所以根唯一；这里独立二分求它当对照。
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

    /// 单政策时的障碍量纲：`μ = BARRIER × θ × motive`
    fn single_mu(motive: f64) -> f64 {
        BARRIER * CURVATURE * motive
    }

    /// 库存给得很大时，解必须落在需求曲线的解析根上。
    ///
    /// 这就是"给 motive 一个边际效应"买到的东西：**量对存量（也就是影子价格）有响应**。
    /// 旧版是线性目标 + `x ≤ 1` 的盒子，库存再大也只能吃到 1，需求完全无弹性。
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
            assert!(outcome.x[0] > 0.0 && outcome.x[0] <= supply * (1.0 + 1e-9), "超取");
        }
    }

    /// 存量越大影子价格越低 ⇒ 吃得越深（需求曲线单调，余量 `s ≈ μ/λ` 次线性增长）
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

    /// 收敛证书：对偶间隙 = (K + P)·μ（上界盒子删掉后少了 P 项）
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
        let mu = single_mu(1.0);
        let starved = demand_root(1.0, mu, 0.02);
        assert!(
            (outcome.x[0] - starved).abs() <= 1e-6 * starved,
            "缺货的政策应当落在自己的需求曲线上 {starved}，实际 {}",
            outcome.x[0],
        );
        assert!(outcome.x[0] < 0.05, "缺货的政策应当被压得很低");
        // 另一条不受连坐：它把自己那样货吃到只剩余量
        let rich = demand_root(1.0, mu, 1e6);
        assert!(
            (outcome.x[1] - rich).abs() <= 1e-6 * rich,
            "不缺货的政策应当落在 {rich}，实际 {}",
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
