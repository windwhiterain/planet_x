pub mod probe;

mod step;

#[cfg(test)]
mod tests;

use fastrand::Rng;

use crate::market::Market;
use crate::warehouse::Warehouses;

pub struct Departments {
    pub departments: Vec<Department>,
    /// index with [`Self::departments`]
    pub grants: Vec<f32>,
    pub treasury: f32,
}

/// index with [`crate::warehouse::Warehouse`]
pub struct Department {
    /// index with [`crate::warehouse::Stock`]
    pub productions: Vec<f32>,
    pub policies: Vec<Policy>,
    /// index [`Self::policies`]，份额最大的政策
    policy_choice: usize,
    /// 本轮按分布实际提货的比例
    policy_execution: f32,
}

pub struct Policy {
    /// index with [`crate::warehouse::Stock`]
    pub consumptions: Vec<f32>,
    /// index with [`crate::warehouse::Stock`]，转换政策的产出
    pub outputs: Vec<f32>,
    pub motive: f32,
    /// 意愿除以资源价格
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
        }
    }

    pub fn with_grants(mut self, grants: Vec<f32>) -> Self {
        self.grants = grants;
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
        for (i, department) in self.departments.iter_mut().enumerate() {
            step::plan(department, &mut warehouses.warehouses[i], market);
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
            debug_assert_eq!(
                department.productions.len(),
                goods,
                "部门 {i} 的产出表必须与仓库库存表等长且同序",
            );
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
    pub fn new(productions: Vec<f32>, policies: Vec<Policy>) -> Self {
        Self {
            productions,
            policies,
            policy_choice: 0,
            policy_execution: 0.0,
        }
    }

    pub fn policy_choice(&self) -> usize {
        self.policy_choice
    }

    pub fn policy_execution(&self) -> f32 {
        self.policy_execution
    }
}

impl Policy {
    pub fn new(consumptions: Vec<f32>, motive: f32) -> Self {
        let outputs = vec![0.0; consumptions.len()];
        Self {
            consumptions,
            outputs,
            motive,
            price_potential: 0.0,
            distribution: 0.0,
        }
    }

    /// 转换政策：吃进 inputs、产出 outputs，吸引力是市价下的利润率
    pub fn transform(inputs: Vec<f32>, outputs: Vec<f32>) -> Self {
        Self {
            consumptions: inputs,
            outputs,
            motive: 0.0,
            price_potential: 0.0,
            distribution: 0.0,
        }
    }

    pub fn is_transform(&self) -> bool {
        self.outputs.iter().any(|output| *output > 0.0)
    }

    pub fn price_potential(&self) -> f32 {
        self.price_potential
    }

    pub fn distribution(&self) -> f32 {
        self.distribution
    }
}
