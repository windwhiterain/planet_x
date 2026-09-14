pub mod probe;

mod settlement;
mod step;

#[cfg(test)]
mod tests;

use fastrand::Rng;

use crate::market::Market;
use crate::warehouse::{Book, Warehouses};

pub struct Departments {
    pub departments: Vec<Department>,
    /// index with [`Self::departments`]
    pub grants: Vec<f32>,
    pub treasury: f32,
    /// 消费结算规则，默认走 [`Rationing::Interior`]
    pub rationing: Rationing,
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
    pub fn new(departments: Vec<Department>) -> Self {
        let grants = vec![0.0; departments.len()];
        Self {
            departments,
            grants,
            treasury: 0.0,
            rationing: Rationing::default(),
        }
    }

    pub fn with_grants(mut self, grants: Vec<f32>) -> Self {
        self.grants = grants;
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

    pub fn step(&mut self, warehouses: &mut Warehouses, market: &mut Market, rng: &mut Rng) {
        self.grant(warehouses);
        self.plan(warehouses, market);
        warehouses.step(market, rng);
        self.settle(warehouses, market);
        self.reclaim(warehouses);
    }

    /// 国内循环：回收上一轮的结余，再统一重新发放
    pub fn step_redistributing(
        &mut self,
        warehouses: &mut Warehouses,
        market: &mut Market,
        rng: &mut Rng,
    ) {
        self.redistribute(warehouses);
        self.plan(warehouses, market);
        warehouses.step(market, rng);
        self.settle(warehouses, market);
    }

    /// 国际循环：不收回、不发放，各凭手里的货币交易
    pub fn step_free(&mut self, warehouses: &mut Warehouses, market: &mut Market, rng: &mut Rng) {
        self.plan(warehouses, market);
        warehouses.step(market, rng);
        self.settle(warehouses, market);
    }

    /// 中央统筹：把全国的货币收回国库，再按部门平均发放
    pub fn redistribute(&mut self, warehouses: &mut Warehouses) {
        self.reclaim(warehouses);
        let count = warehouses.warehouses.len();
        if count == 0 {
            return;
        }
        let share = self.treasury / count as f32;
        for warehouse in &mut warehouses.warehouses {
            warehouse.currency += share;
        }
        self.treasury -= share * count as f32;
    }

    /// 中央拨款：每轮把 [`Self::grants`] 拨进各部门的仓库
    pub fn grant(&mut self, warehouses: &mut Warehouses) {
        for (i, warehouse) in warehouses.warehouses.iter_mut().enumerate() {
            let grant = self.grants.get(i).copied().unwrap_or(0.0);
            warehouse.currency += grant.max(0.0);
        }
    }

    /// 生产与政策：增产，选中政策，再从仓库提资源
    pub fn plan(&mut self, warehouses: &mut Warehouses, market: &Market) {
        self.debug_assert_aligned(warehouses, market);
        let Warehouses {
            warehouses: stocks,
            books,
            ..
        } = warehouses;
        for (i, department) in self.departments.iter_mut().enumerate() {
            let locality = stocks[i].locality;
            // 这个部门所在地方的账本；还没成形就是空表，plan 会回退到指数
            let book: &[Book] = books.get(locality).map(|row| row.as_slice()).unwrap_or(&[]);
            step::plan(department, &mut stocks[i], market, book, self.rationing);
        }
    }

    /// 结算：成交收入进仓库
    pub fn settle(&mut self, warehouses: &mut Warehouses, market: &Market) {
        for (i, warehouse) in warehouses.warehouses.iter_mut().enumerate() {
            warehouse.currency += step::revenue(market, i);
        }
    }

    /// 收回：花不完的拨款收回国库
    pub fn reclaim(&mut self, warehouses: &mut Warehouses) {
        for warehouse in &mut warehouses.warehouses {
            self.treasury += warehouse.currency;
            warehouse.currency = 0.0;
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
