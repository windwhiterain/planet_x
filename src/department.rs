mod settlement;
mod step;

#[cfg(test)]
mod tests;

use crate::market::Market;
use crate::warehouse::Warehouses;

pub struct Departments {
    pub departments: Vec<Department>,
    /// 消费结算规则
    pub rationing: Rationing,
    /// 转移支付速率：每轮把余额按这个比例拉向均值（0 = 不转移）
    pub transfer_rate: f32,
    /// 货币总量的目标值（0 = 不控制）。结算会净创造货币（部门把产出卖给自己的仓库，
    /// 而仓库不持钱），所以存量货币必须有一个数量控制，否则购买力无界增长。
    /// 按比例缩放保持相对份额，所以"存量"仍然成立。
    pub money_target: f32,
}

/// 默认障碍强度：`μ = barrier × θ × 平均 motive`，读作**留货值多少**
/// （`λ = μ/s`，越大留的余量越多、市场上越有货可卖）
pub const DEFAULT_BARRIER: f32 = settlement::BARRIER as f32;

/// 默认边际效应曲率：`u(x) = x^θ`，`θ = 1` 退回线性（需求对价格完全无弹性）
pub const DEFAULT_CURVATURE: f32 = settlement::CURVATURE as f32;

/// 消费怎么在彼此独立的政策之间分摊仓库存量
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Rationing {
    /// 原始-对偶内点法，见 [`settlement`]。
    ///
    /// `barrier` 是障碍强度，`curvature` 是 `u(x) = x^θ` 的 `θ`：θ 越小边际效用掉得越快、
    /// 需求曲线越平；θ → 1 就是旧版的线性目标（要多少吃多少，量对价格无弹性）。
    Interior { barrier: f32, curvature: f32 },
    /// 旧的硬配给 `x_p = min(1, min_k 存量_k / 该商品的总意愿)`，只留作 A/B
    Hard,
}

impl Default for Rationing {
    fn default() -> Self {
        Self::Interior {
            barrier: settlement::BARRIER as f32,
            curvature: settlement::CURVATURE as f32,
        }
    }
}

/// 消费结算的诊断读数，见 [`settlement`]
#[derive(Clone, Copy, Debug, Default)]
pub struct SettlementReport {
    /// 对偶间隙——收敛证书，理论上等于 `(商品数 + 2×政策数) × μ`
    pub gap: f64,
    /// 收敛时的障碍参数
    pub mu: f64,
    /// 牛顿步总数
    pub iterations: usize,
    /// 残差是否压到容差；没压到就是**解没解出来**，不许静默继续
    pub converged: bool,
    /// 收尾时的相对残差（缩放无穷范数）——`converged` 为假时用来看**离得多远**
    pub residual: f64,
    /// 走了几档 μ
    pub phases: usize,
    /// 是否出现过退化（Cholesky 失败或回溯失败）
    pub degraded: bool,
    /// 因配方里有零存量商品被整条判死的政策数
    pub blocked: usize,
    /// 实际用掉的最紧那样货的比例，`≤ 1` 即不超取
    pub utilization: f64,
}

/// index with [`crate::warehouse::Warehouse`]
pub struct Department {
    pub policies: Vec<Policy>,
    /// 每轮可用的产能，无穷表示不设限
    pub capacity: f32,
    /// index [`Self::policies`]，份额最大的政策
    policy_choice: usize,
    /// 本轮**实际提货量**（逐政策篮子数已经打进去了）
    intake: Vec<f32>,
    /// 本轮实际入账的产出（计划产出 × 产能缩放）
    delivery: Vec<f32>,
    /// 本轮产能缩放系数
    capacity_scale: f32,
    /// 本轮消费结算的收敛情况
    settlement: SettlementReport,
    /// **部门手里的现金**。存量、跨轮累积：卖产出给自己的仓库收钱、买消费付钱。
    /// 它同时是消费的预算约束（结算里的一条现金行）。
    pub currency: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PolicyKind {
    /// 按意愿吃进商品
    Consumption,
    /// 吃进投入、产出成品，主生产也是一个消耗产能的转换
    Production,
}

pub struct Policy {
    pub kind: PolicyKind,
    /// index with [`crate::warehouse::Stock`]
    pub consumptions: Vec<f32>,
    /// index with [`crate::warehouse::Stock`]
    pub outputs: Vec<f32>,
    /// 每单位经手物资占用的产能
    pub capacity_cost: f32,
    pub motive: f32,
    /// 意愿除以资源价格，或约束下的纯策略收益
    price_potential: f32,
    /// normalize to 1
    distribution: f32,
}

impl Departments {
    pub const DEFAULT_TRANSFER: f32 = 0.5;

    pub fn new(departments: Vec<Department>) -> Self {
        Self {
            departments,
            rationing: Rationing::default(),
            transfer_rate: Self::DEFAULT_TRANSFER,
            money_target: 0.0,
        }
    }

    pub fn with_transfer(mut self, rate: f32) -> Self {
        self.transfer_rate = rate.clamp(0.0, 1.0);
        self
    }

    /// 换一套消费结算规则，见 [`Rationing`]
    pub fn with_rationing(mut self, rationing: Rationing) -> Self {
        self.rationing = rationing;
        self
    }

    pub fn len(&self) -> usize {
        self.departments.len()
    }

    pub fn is_empty(&self) -> bool {
        self.departments.is_empty()
    }

    pub fn step(&mut self, warehouses: &mut Warehouses, market: &mut Market) {
        self.plan(warehouses, market);
        warehouses.step(market);
        self.settle(warehouses);
        self.normalize();
        self.transfer(self.transfer_rate);
    }

    /// 货币数量控制：把总量按比例拉回 [`Self::money_target`]。相对份额不变。
    pub fn normalize(&mut self) {
        if !(self.money_target > 0.0) {
            return;
        }
        let total: f32 = self
            .departments
            .iter()
            .map(|department| department.currency)
            .sum();
        if !total.is_finite() || !(total > 0.0) {
            return;
        }
        let scale = self.money_target / total;
        if !scale.is_finite() || !(scale > 0.0) {
            return;
        }
        for department in self.departments.iter_mut() {
            department.currency *= scale;
        }
    }

    /// 转移支付：把余额按 `rate` 的比例拉向均值。**总量守恒**（纯粹的再分配），
    /// 所以它不会自己制造通货膨胀；它管的是"纯消费部门手里有没有钱"。
    pub fn transfer(&mut self, rate: f32) {
        let count = self.departments.len();
        if count == 0 || !(rate > 0.0) {
            return;
        }
        let rate = rate.min(1.0);
        let mean = self
            .departments
            .iter()
            .map(|department| department.currency)
            .sum::<f32>()
            / count as f32;
        for department in self.departments.iter_mut() {
            if department.currency.is_finite() {
                department.currency += rate * (mean - department.currency);
            } else {
                department.currency = mean;
            }
        }
    }

    /// 生产与政策：增产，选中政策，再从仓库提资源
    pub fn plan(&mut self, warehouses: &mut Warehouses, market: &Market) {
        self.debug_assert_aligned(warehouses, market);
        let Warehouses {
            warehouses: stocks, ..
        } = warehouses;
        for (i, department) in self.departments.iter_mut().enumerate() {
            step::plan(department, &mut stocks[i], self.rationing);
        }
    }

    /// 结算：部门**直接与自己的仓库**交易。买走消费按仓库挂价付钱，
    /// 交出产出按同一个挂价收钱，净额进余额。仓库不持钱，所以这里就是货币
    /// 唯一的出入口——净产出为正是发行、净消费为正是回笼。
    pub fn settle(&mut self, warehouses: &Warehouses) {
        for (i, department) in self.departments.iter_mut().enumerate() {
            let Some(warehouse) = warehouses.warehouses.get(i) else {
                continue;
            };
            let mut cost = 0.0f32;
            let mut revenue = 0.0f32;
            for (k, stock) in warehouse.stocks.iter().enumerate() {
                let price = if stock.price.is_finite() && stock.price > 0.0 {
                    stock.price
                } else {
                    0.0
                };
                cost += price * department.intake.get(k).copied().unwrap_or(0.0);
                revenue += price * department.delivery.get(k).copied().unwrap_or(0.0);
            }
            if cost.is_finite() && revenue.is_finite() {
                department.currency += revenue - cost;
            }
        }
    }

    fn debug_assert_aligned(&self, warehouses: &Warehouses, market: &Market) {
        debug_assert_eq!(
            self.departments.len(),
            warehouses.warehouses.len(),
            "一个部门必须对应一个仓库",
        );
        debug_assert_eq!(
            self.departments.len(),
            market.traders.len(),
            "一个部门必须对应一个交易者",
        );
        for (i, department) in self.departments.iter().enumerate() {
            let goods = warehouses.warehouses[i].stocks.len();
            for policy in &department.policies {
                debug_assert_eq!(
                    policy.consumptions.len(),
                    goods,
                    "部门 {i} 的政策消耗表必须与仓库库存表等长且同序",
                );
                debug_assert_eq!(
                    policy.outputs.len(),
                    goods,
                    "部门 {i} 的政策产出表必须与仓库库存表等长且同序",
                );
            }
        }
    }
}

impl Department {
    pub fn new(policies: Vec<Policy>) -> Self {
        let goods = policies
            .first()
            .map(|policy| policy.consumptions.len())
            .unwrap_or(0);
        Self {
            policies,
            capacity: f32::INFINITY,
            policy_choice: 0,
            intake: vec![0.0; goods],
            delivery: vec![0.0; goods],
            capacity_scale: 0.0,
            settlement: SettlementReport::default(),
            currency: 0.0,
        }
    }

    pub fn with_capacity(mut self, capacity: f32) -> Self {
        self.capacity = capacity.max(0.0);
        self
    }

    pub fn policy_choice(&self) -> usize {
        self.policy_choice
    }

    /// 本轮**实际提货量**（逐政策的篮子数已经打进去了）
    pub fn intake(&self) -> &[f32] {
        &self.intake
    }

    /// 本轮消费结算的收敛情况，见 [`SettlementReport`]
    pub fn settlement(&self) -> &SettlementReport {
        &self.settlement
    }

    /// 本轮实际入账的产出
    pub fn delivery(&self) -> &[f32] {
        &self.delivery
    }

    pub fn capacity_scale(&self) -> f32 {
        self.capacity_scale
    }
}

impl Policy {
    /// 消费政策：按意愿吃进商品
    pub fn consumption(consumptions: Vec<f32>, motive: f32) -> Self {
        let outputs = vec![0.0; consumptions.len()];
        Self {
            kind: PolicyKind::Consumption,
            consumptions,
            outputs,
            capacity_cost: 0.0,
            motive,
            price_potential: 0.0,
            distribution: 0.0,
        }
    }

    /// 生产政策：吃进 inputs、产出 outputs，只看市价下的利润；主生产就是投入全为零的那种
    pub fn production(inputs: Vec<f32>, outputs: Vec<f32>) -> Self {
        Self {
            kind: PolicyKind::Production,
            consumptions: inputs,
            outputs,
            capacity_cost: 0.0,
            motive: 0.0,
            price_potential: 0.0,
            distribution: 0.0,
        }
    }

    pub fn with_capacity_cost(mut self, capacity_cost: f32) -> Self {
        self.capacity_cost = capacity_cost.max(0.0);
        self
    }

    /// 每单位产出占用的产能
    pub fn capacity_use(&self) -> f32 {
        let handled: f32 = self
            .consumptions
            .iter()
            .map(|consumption| consumption.max(0.0))
            .sum::<f32>()
            + self
                .outputs
                .iter()
                .map(|output| output.max(0.0))
                .sum::<f32>();
        self.capacity_cost * handled
    }

    pub fn is_production(&self) -> bool {
        self.kind == PolicyKind::Production
    }

    pub fn is_consumption(&self) -> bool {
        self.kind == PolicyKind::Consumption
    }

    pub fn price_potential(&self) -> f32 {
        self.price_potential
    }

    pub fn distribution(&self) -> f32 {
        self.distribution
    }
}
