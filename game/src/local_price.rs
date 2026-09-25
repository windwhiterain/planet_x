#[cfg(test)]
mod tests;

use crate::department::{Department, Departments, Policy, Rationing};
use crate::market::{Market, Merchandise, Trader, TraderMerchandise};
use crate::warehouse::{Stock, Warehouse, Warehouses};

pub const GOODS: usize = 3;
pub const UNITS: usize = 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Producer,
    Consumer,
}
pub const BASE: f32 = 4.0;
pub const CAMPAIGN: f32 = 4.0;
pub const MOTIVE: f32 = 20.0;
pub const GRANT: f32 = 1000.0;
pub const BASE_PRICE: f32 = 1.0;

pub const NAMES: [&str; 6] = ["甲", "乙", "丙", "丁", "戊", "己"];

pub struct Transform {
    pub polity: usize,
    pub unit: usize,
    pub inputs: Vec<f32>,
    pub outputs: Vec<f32>,
    pub capacity_cost: f32,
}

pub const LADDER_CAPACITY: f32 = 8.0;
pub const MODERN_CAPACITY: f32 = 24.0;
pub const PRIMARY_CAPACITY_COST: f32 = 0.25;
pub const LADDER_THRIFTY: (f32, f32, f32) = (0.2, 4.0, 2.0);
pub const LADDER_FAST: (f32, f32, f32) = (0.8, 12.0, 0.2);
pub const SECTOR_SCALE: f32 = 8.0;
pub const SECTOR_INPUT: f32 = 2.0;
pub const SECTOR_CAPACITY: [f32; 3] = [0.1, 0.2, 0.3];
pub const SECTOR_MOTIVE: [f32; 3] = [1.0, 1.6, 2.4];

fn specialize(outputs: &mut [f32], unit: usize, factor: f32) {
    if !(factor > 0.0) || (factor - 1.0).abs() < f32::EPSILON {
        return;
    }
    if let Some(output) = outputs.get_mut(unit) {
        if *output > 0.0 {
            *output *= factor;
        }
    }
}

pub struct Spec {
    pub polities: usize,
    pub supply: Vec<Vec<f32>>,
    pub motive: Vec<Vec<f32>>,
    pub transforms: Vec<Transform>,
    pub capacity: f32,
    pub self_capacity: bool,
    pub primary_free: bool,
    pub all_consume: bool,
    pub motive_ladder: Vec<f32>,
    pub specialty: Vec<f32>,
}

impl Spec {
    pub fn symmetric(polities: usize) -> Self {
        Self {
            polities,
            supply: vec![vec![1.0; GOODS]; polities],
            motive: vec![vec![1.0; GOODS]; polities],
            transforms: Vec::new(),
            capacity: f32::INFINITY,
            self_capacity: false,
            primary_free: true,
            all_consume: false,
            motive_ladder: vec![1.0; GOODS],
            specialty: vec![1.0; GOODS],
        }
    }

    pub fn specialty_of(&self, good: usize) -> f32 {
        self.specialty.get(good).copied().unwrap_or(1.0)
    }

    pub fn with_specialty(mut self, factor: f32) -> Self {
        let factor = if factor.is_finite() && factor > 0.0 {
            factor
        } else {
            1.0
        };
        self.specialty = vec![factor; GOODS];
        self
    }

    pub fn with_specialty_at(mut self, good: usize, factor: f32) -> Self {
        if let Some(slot) = self.specialty.get_mut(good) {
            if factor.is_finite() && factor > 0.0 {
                *slot = factor;
            }
        }
        self
    }

    pub fn modern(polities: usize) -> Self {
        let mut spec = Self::symmetric(polities);
        spec.all_consume = true;
        spec.primary_free = false;
        spec.capacity = MODERN_CAPACITY;
        for unit in 0..GOODS {
            let mut inputs = vec![0.0; GOODS];
            let mut outputs = vec![0.0; GOODS];
            outputs[unit] = SECTOR_SCALE;
            if unit > 0 {
                inputs[unit - 1] = SECTOR_INPUT * SECTOR_SCALE;
            }
            spec.transforms.push(Transform {
                polity: polities + 1,
                unit: GOODS,
                inputs,
                outputs,
                capacity_cost: SECTOR_CAPACITY[unit],
            });
        }
        spec
    }

    pub fn sectors(polities: usize) -> Self {
        let mut spec = Self::symmetric(polities);
        spec.self_capacity = true;
        spec.primary_free = true;
        for unit in 0..GOODS {
            let mut inputs = vec![0.0; GOODS];
            let mut outputs = vec![0.0; GOODS];
            outputs[unit] = SECTOR_SCALE;
            if unit > 0 {
                inputs[unit - 1] = SECTOR_INPUT * SECTOR_SCALE;
            }
            for polity in 0..polities {
                spec.transforms.push(Transform {
                    polity,
                    unit,
                    inputs: inputs.clone(),
                    outputs: outputs.clone(),
                    capacity_cost: SECTOR_CAPACITY[unit],
                });
            }
        }
        spec
    }

    pub fn scarce(polities: usize, seat: usize, good: usize) -> Self {
        let mut spec = Self::symmetric(polities);
        spec.supply[seat][good] = 0.5;
        spec.motive[seat][good] = 1.5;
        for other in 0..polities {
            if other != seat {
                spec.supply[other][good] = 1.25;
            }
        }
        spec
    }

    pub fn ladder(polities: usize, food_supply: f32) -> Self {
        let mut spec = Self::scarce(polities, 0, 0);
        spec.supply[0][0] = food_supply;
        spec.capacity = LADDER_CAPACITY;
        for (rate, scale, capacity_cost) in [LADDER_THRIFTY, LADDER_FAST] {
            let mut inputs = vec![0.0; GOODS];
            let mut outputs = vec![0.0; GOODS];
            inputs[1] = rate * scale;
            outputs[0] = scale;
            spec.transforms.push(Transform {
                polity: 0,
                unit: 0,
                inputs,
                outputs,
                capacity_cost,
            });
        }
        spec
    }

    pub fn with_transform(
        mut self,
        polity: usize,
        unit: usize,
        inputs: Vec<f32>,
        outputs: Vec<f32>,
    ) -> Self {
        self.transforms.push(Transform {
            polity,
            unit,
            inputs,
            outputs,
            capacity_cost: 0.0,
        });
        self
    }

    pub fn with_process(
        mut self,
        polity: usize,
        unit: usize,
        inputs: Vec<f32>,
        outputs: Vec<f32>,
        capacity_cost: f32,
    ) -> Self {
        self.transforms.push(Transform {
            polity,
            unit,
            inputs,
            outputs,
            capacity_cost,
        });
        self
    }

    pub fn with_capacity(mut self, capacity: f32) -> Self {
        self.capacity = capacity;
        self
    }

    fn transforms(&self, polity: usize, unit: usize) -> impl Iterator<Item = &Transform> {
        self.transforms.iter().filter(move |transform| {
            (transform.polity == polity || transform.polity > self.polities)
                && (transform.unit == unit || transform.unit >= GOODS)
        })
    }

    pub fn supply(&self, polity: usize, good: usize) -> f32 {
        self.supply
            .get(polity)
            .and_then(|row| row.get(good))
            .copied()
            .unwrap_or(1.0)
    }

    pub fn motive(&self, polity: usize, good: usize) -> f32 {
        self.motive
            .get(polity)
            .and_then(|row| row.get(good))
            .copied()
            .unwrap_or(1.0)
    }
}

pub struct Polity {
    pub name: &'static str,
    pub seat: usize,
}

impl Polity {
    pub fn span(&self) -> std::ops::Range<usize> {
        self.seat..self.seat + 2 * UNITS
    }
}

#[derive(Clone, Copy, Default)]
pub struct GoodState {
    pub index: f32,
    pub deal_price: f32,
    pub bid: f32,
    pub ask: f32,
    pub quote_sell: f32,
    pub quote_buy: f32,
    pub target: f32,
    pub dealt: f32,
    pub sold: f32,
    pub bought: f32,
    pub stock: f32,
    pub wanted: f32,
    pub consumed: f32,
    pub delivery: f32,
}

pub struct Snapshot {
    pub round: usize,
    pub prices: Vec<f32>,
    pub local_ratios: Vec<Vec<f32>>,
    pub transform_share: f32,
    pub transform_potential: f32,
    pub money: f32,
}

pub struct Lab {
    pub departments: Departments,
    pub warehouses: Warehouses,
    pub market: Market,
    pub polities: Vec<Polity>,
    pub round: usize,
    pub settlement_failures: usize,
    pub settlement_degraded: usize,
    pub history: Vec<Snapshot>,
}

impl Lab {
    pub fn new(spec: &Spec) -> Self {
        let count = spec.polities * 2 * UNITS;
        let market = Market::new(
            (0..GOODS)
                .map(|_| Merchandise { price: BASE_PRICE })
                .collect(),
            (0..count)
                .map(|_| Trader {
                    merchandises: (0..GOODS)
                        .map(|_| TraderMerchandise::new(0.0, 0.0))
                        .collect(),
                })
                .collect(),
        );
        let mut warehouses = Vec::with_capacity(count);
        let mut departments = Vec::with_capacity(count);
        let mut polities = Vec::with_capacity(spec.polities);
        for polity in 0..spec.polities {
            for unit in 0..UNITS {
                for kind in [Kind::Producer, Kind::Consumer] {
                    let mut stocks = Vec::with_capacity(GOODS);
                    for good in 0..GOODS {
                        let (volume, target) = if good == unit {
                            (0.0, 0.0)
                        } else {
                            (CAMPAIGN, CAMPAIGN / 2.0)
                        };
                        stocks.push(Stock::new(volume, target));
                    }
                    warehouses.push(
                        Warehouse::new(stocks)
                            .with_reference(vec![BASE_PRICE; GOODS])
                            .with_locality(polity),
                    );
                    let policies: Vec<Policy> = match kind {
                        Kind::Producer => {
                            let mut policies: Vec<Policy> = Vec::new();
                            for transform in spec.transforms(polity, unit) {
                                let mut outputs = transform.outputs.clone();
                                specialize(&mut outputs, unit, spec.specialty_of(unit));
                                policies.push(
                                    Policy::production(transform.inputs.clone(), outputs)
                                        .with_capacity_cost(transform.capacity_cost),
                                );
                            }
                            if spec.primary_free || unit == 0 {
                                let mut outputs = vec![0.0; GOODS];
                                outputs[unit] = BASE * spec.supply(polity, unit);
                                specialize(&mut outputs, unit, spec.specialty_of(unit));
                                policies.push(
                                    Policy::production(vec![0.0; GOODS], outputs)
                                        .with_capacity_cost(PRIMARY_CAPACITY_COST),
                                );
                            }
                            policies
                        }
                        Kind::Consumer => (0..GOODS)
                            .filter(|good| spec.all_consume || *good != unit)
                            .map(|good| {
                                let mut consumptions = vec![0.0; GOODS];
                                consumptions[good] = CAMPAIGN;
                                Policy::consumption(
                                    consumptions,
                                    MOTIVE * spec.motive(polity, good) * spec.motive_ladder[good],
                                )
                            })
                            .collect(),
                    };
                    let capacity = if spec.self_capacity {
                        policies
                            .iter()
                            .filter(|policy| policy.is_production())
                            .map(|policy| policy.capacity_use())
                            .sum::<f32>()
                    } else {
                        spec.capacity
                    };
                    departments.push(Department::new(policies).with_capacity(capacity));
                }
            }
            polities.push(Polity {
                name: NAMES[polity % NAMES.len()],
                seat: polity * 2 * UNITS,
            });
        }
        let mut departments = Departments::new(departments);
        for department in departments.departments.iter_mut() {
            department.currency = GRANT;
        }
        departments.money_target = GRANT * count as f32;
        let mut lab = Self {
            departments,
            warehouses: Warehouses::new(warehouses),
            market,
            polities,
            round: 0,
            settlement_failures: 0,
            settlement_degraded: 0,
            history: Vec::new(),
        };
        let snapshot = lab.snapshot();
        lab.history.push(snapshot);
        lab
    }

    pub fn with_flow_scale(mut self, scale: f32) -> Self {
        self.market = self.market.with_flow_scale(scale);
        self
    }

    pub fn with_rationing(mut self, rationing: Rationing) -> Self {
        self.departments.rationing = rationing;
        self
    }

    pub fn with_grant(mut self, grant: f32) -> Self {
        let grant = grant.max(0.0);
        let count = self.departments.departments.len();
        for department in self.departments.departments.iter_mut() {
            department.currency = grant;
        }
        self.departments.money_target = grant * count as f32;
        self
    }

    pub fn with_relations(mut self, relations: &[Vec<f32>]) -> Self {
        self.market.set_relations(relations);
        self
    }

    pub fn with_price_law(mut self, curvature: f32, inertia: f32, target_rate: f32) -> Self {
        self.warehouses
            .set_price_law(curvature, inertia, target_rate);
        self
    }

    pub fn with_transfer(mut self, rate: f32) -> Self {
        self.departments.transfer_rate = rate.clamp(0.0, 1.0);
        self
    }

    pub fn block(&mut self, among: &[usize], relation: f32) {
        let traders = self.market.traders.len();
        let mut relations = self.market.relations.clone();
        for i in 0..traders {
            for j in 0..traders {
                let left = self.polity_of(i);
                let right = self.polity_of(j);
                if left == right {
                    continue;
                }
                let blocked = among.contains(&left) || among.contains(&right);
                if blocked {
                    relations[i][j] = relation;
                }
            }
        }
        self.market.set_relations(&relations);
    }

    pub fn sanction(&mut self, departments: &[usize], relation: f32) {
        let traders = self.market.traders.len();
        let mut relations = self.market.relations.clone();
        for i in 0..traders {
            for j in 0..traders {
                if self.polity_of(i) == self.polity_of(j) {
                    continue;
                }
                if departments.contains(&i) || departments.contains(&j) {
                    relations[i][j] = relation;
                }
            }
        }
        self.market.set_relations(&relations);
    }

    pub fn unsanction(&mut self) {
        let traders = self.market.traders.len();
        self.market
            .set_relations(&vec![vec![1.0; traders]; traders]);
    }

    pub fn department_of(&self, polity: usize, unit: usize, kind: Kind) -> usize {
        polity * 2 * UNITS + unit * 2 + if kind == Kind::Producer { 0 } else { 1 }
    }

    pub fn process_state(&self, department: usize) -> Vec<(f32, f32, f32)> {
        self.departments
            .departments
            .get(department)
            .map(|department| {
                department
                    .policies
                    .iter()
                    .filter(|policy| policy.is_production())
                    .map(|policy| {
                        (
                            policy.distribution(),
                            policy.price_potential(),
                            policy.capacity_use(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn department_prices(&self) -> Vec<Vec<f32>> {
        self.market
            .traders
            .iter()
            .map(|trader| {
                trader
                    .merchandises
                    .iter()
                    .enumerate()
                    .map(|(k, merchandise)| {
                        let price = self.market.merchandises[k].price.max(1e-9);
                        if merchandise.deal_volume() != 0.0 && merchandise.deal_price() > 0.0 {
                            merchandise.deal_price() / price
                        } else {
                            0.0
                        }
                    })
                    .collect()
            })
            .collect()
    }

    pub fn department_external(&self) -> Vec<f32> {
        let traders = self.market.traders.len();
        (0..traders)
            .map(|i| {
                (0..traders)
                    .filter(|j| self.polity_of(*j) != self.polity_of(i))
                    .map(|j| {
                        (0..GOODS)
                            .map(|k| self.market.deals[i][j][k].volume.abs())
                            .sum::<f32>()
                    })
                    .sum()
            })
            .collect()
    }

    pub fn department_fill(&self) -> Vec<Vec<f32>> {
        self.market
            .traders
            .iter()
            .map(|trader| {
                trader
                    .merchandises
                    .iter()
                    .map(|merchandise| {
                        if merchandise.volume != 0.0 {
                            merchandise.deal_volume().abs() / merchandise.volume.abs()
                        } else {
                            0.0
                        }
                    })
                    .collect()
            })
            .collect()
    }

    pub fn polity_of(&self, warehouse: usize) -> usize {
        warehouse_polity(warehouse)
    }

    pub fn good_states(&self) -> Vec<GoodState> {
        let goods = self.market.merchandises.len();
        let traders = self.market.traders.len();
        let index = self.warehouses.quoted_index(goods);
        let mut states = vec![GoodState::default(); goods];
        for (k, state) in states.iter_mut().enumerate() {
            state.index = index.get(k).copied().unwrap_or(0.0);
            let mut formed = 0.0;
            for (locality, row) in self.warehouses.ask.iter().enumerate() {
                let ask = row.get(k).copied().unwrap_or(0.0);
                let bid = self
                    .warehouses
                    .bid
                    .get(locality)
                    .and_then(|row| row.get(k))
                    .copied()
                    .unwrap_or(0.0);
                if !(ask > 0.0) || !(bid > 0.0) {
                    continue;
                }
                state.bid += bid;
                state.ask += ask;
                formed += 1.0;
            }
            if formed > 0.0 {
                state.bid /= formed;
                state.ask /= formed;
            }
            let mut sell = 0.0;
            let mut buy = 0.0;
            for i in 0..traders {
                state.target += self.warehouses.warehouses[i].stocks[k].target_volume;
                let merchandise = &self.market.traders[i].merchandises[k];
                let net = merchandise.deal_volume();
                if net > 0.0 {
                    state.sold += net;
                    sell += net;
                    state.quote_sell += net * merchandise.price;
                } else if net < 0.0 {
                    state.bought += -net;
                    buy += -net;
                    state.quote_buy += -net * merchandise.price;
                }
            }
            state.quote_sell = if sell > 0.0 {
                state.quote_sell / sell
            } else {
                0.0
            };
            state.quote_buy = if buy > 0.0 {
                state.quote_buy / buy
            } else {
                0.0
            };
            let mut volume = 0.0;
            let mut value = 0.0;
            for i in 0..traders {
                for j in (i + 1)..traders {
                    let deal = &self.market.deals[i][j][k];
                    let size = deal.volume.abs();
                    volume += size;
                    value += size * deal.price;
                }
            }
            state.dealt = volume;
            state.deal_price = if volume > 0.0 { value / volume } else { 0.0 };
            for warehouse in &self.warehouses.warehouses {
                state.stock += warehouse
                    .stocks
                    .get(k)
                    .map(|stock| stock.volume)
                    .unwrap_or(0.0);
            }
            for department in &self.departments.departments {
                state.delivery += department.delivery().get(k).copied().unwrap_or(0.0);
                state.consumed += department.intake().get(k).copied().unwrap_or(0.0);
            }
            for warehouse in &self.warehouses.warehouses {
                state.wanted += warehouse.stocks[k].wanted;
            }
        }
        states
    }

    pub fn spread(&self, polity: usize, good: usize) -> f32 {
        let level = |p: usize| -> f32 {
            let ask = self
                .warehouses
                .ask
                .get(p)
                .and_then(|row| row.get(good))
                .copied()
                .unwrap_or(0.0);
            let bid = self
                .warehouses
                .bid
                .get(p)
                .and_then(|row| row.get(good))
                .copied()
                .unwrap_or(0.0);
            let mid = crate::utils::geometric_average(bid, ask);
            let index = self
                .warehouses
                .quoted_index(self.market.merchandises.len())
                .get(good)
                .copied()
                .unwrap_or(0.0);
            if mid > 0.0 && index > 0.0 {
                mid / index
            } else {
                1.0
            }
        };
        let own = level(polity);
        let others: Vec<f32> = (0..self.polities.len())
            .filter(|p| *p != polity)
            .map(level)
            .collect();
        if others.is_empty() {
            return 0.0;
        }
        own - others.iter().sum::<f32>() / others.len() as f32
    }

    pub fn step(&mut self) {
        self.departments
            .step(&mut self.warehouses, &mut self.market);
        for department in &self.departments.departments {
            let report = department.settlement();
            if !report.converged {
                self.settlement_failures += 1;
            }
            if report.degraded {
                self.settlement_degraded += 1;
            }
        }
        self.round += 1;
        let snapshot = self.snapshot();
        self.history.push(snapshot);
    }

    pub fn run(&mut self, rounds: usize) {
        for _ in 0..rounds {
            self.step();
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        let transform = self
            .departments
            .departments
            .iter()
            .flat_map(|department| department.policies.iter())
            .find(|policy| policy.is_production());
        let index = self.warehouses.quoted_index(self.market.merchandises.len());
        Snapshot {
            round: self.round,
            prices: index.clone(),
            local_ratios: self.warehouses.local_ratios(&index),
            transform_share: transform.map_or(0.0, |policy| policy.distribution()),
            transform_potential: transform.map_or(0.0, |policy| policy.price_potential()),
            money: self
                .departments
                .departments
                .iter()
                .map(|department| department.currency)
                .sum(),
        }
    }
}

pub fn warehouse_polity(warehouse: usize) -> usize {
    warehouse / (2 * UNITS)
}

pub fn bloc_relations(polities: usize, outside: f32) -> Vec<Vec<f32>> {
    let traders = polities * 2 * UNITS;
    let mut relations = vec![vec![1.0; traders]; traders];
    for i in 0..traders {
        for j in 0..traders {
            if warehouse_polity(i) != warehouse_polity(j) {
                relations[i][j] = outside;
            }
        }
    }
    relations
}
