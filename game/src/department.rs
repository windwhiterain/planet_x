mod settlement;
mod step;

#[cfg(test)]
mod tests;

use crate::market::Market;
use crate::warehouse::Warehouses;

pub struct Departments {
    pub departments: Vec<Department>,
    pub rationing: Rationing,
    pub transfer_rate: f32,
    pub money_target: f32,
}

pub const DEFAULT_BARRIER: f32 = settlement::BARRIER as f32;

pub const DEFAULT_CURVATURE: f32 = settlement::CURVATURE as f32;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Rationing {
    Interior { barrier: f32, curvature: f32 },
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

#[derive(Clone, Copy, Debug, Default)]
pub struct SettlementReport {
    pub gap: f64,
    pub mu: f64,
    pub iterations: usize,
    pub converged: bool,
    pub residual: f64,
    pub phases: usize,
    pub degraded: bool,
    pub blocked: usize,
    pub utilization: f64,
}

pub struct Department {
    pub policies: Vec<Policy>,
    pub capacity: f32,
    policy_choice: usize,
    intake: Vec<f32>,
    delivery: Vec<f32>,
    capacity_scale: f32,
    settlement: SettlementReport,
    pub currency: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PolicyKind {
    Consumption,
    Production,
}

pub struct Policy {
    pub kind: PolicyKind,
    pub consumptions: Vec<f32>,
    pub outputs: Vec<f32>,
    pub capacity_cost: f32,
    pub motive: f32,
    price_potential: f32,
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

    pub fn plan(&mut self, warehouses: &mut Warehouses, market: &Market) {
        self.debug_assert_aligned(warehouses, market);
        let Warehouses {
            warehouses: stocks, ..
        } = warehouses;
        for (i, department) in self.departments.iter_mut().enumerate() {
            step::plan(department, &mut stocks[i], self.rationing);
        }
    }

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

    pub fn intake(&self) -> &[f32] {
        &self.intake
    }

    pub fn settlement(&self) -> &SettlementReport {
        &self.settlement
    }

    pub fn delivery(&self) -> &[f32] {
        &self.delivery
    }

    pub fn capacity_scale(&self) -> f32 {
        self.capacity_scale
    }
}

impl Policy {
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
