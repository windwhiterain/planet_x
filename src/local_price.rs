#[cfg(test)]
mod tests;

use fastrand::Rng;

use crate::department::{Department, Departments, Policy};
use crate::market::{Market, Merchandise, Trader, TraderMerchandise};
use crate::warehouse::{Stock, Warehouse, Warehouses};

pub const GOODS: usize = 3;
pub const UNITS: usize = 3;
pub const BASE: f32 = 4.0;
pub const CAMPAIGN: f32 = 4.0;
pub const MOTIVE: f32 = 20.0;
pub const GRANT: f32 = 10.0;
pub const BASE_PRICE: f32 = 1.0;
pub const MAX_WEDGE: f32 = 2.0;

pub const NAMES: [&str; 6] = ["甲", "乙", "丙", "丁", "戊", "己"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelRule {
    Fixed,
    OwnVwap,
    Counterparty,
    Shortfall,
    Pressure,
}

impl LevelRule {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "fixed" => Some(Self::Fixed),
            "vwap" => Some(Self::OwnVwap),
            "counterparty" => Some(Self::Counterparty),
            "shortfall" => Some(Self::Shortfall),
            "pressure" => Some(Self::Pressure),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Fixed => "fixed",
            Self::OwnVwap => "vwap",
            Self::Counterparty => "counterparty",
            Self::Shortfall => "shortfall",
            Self::Pressure => "pressure",
        }
    }
}

pub struct Transform {
    pub polity: usize,
    pub unit: usize,
    pub inputs: Vec<f32>,
    pub outputs: Vec<f32>,
}

pub struct Spec {
    pub polities: usize,
    pub supply: Vec<Vec<f32>>,
    pub motive: Vec<Vec<f32>>,
    pub transform: Option<Transform>,
}

impl Spec {
    pub fn symmetric(polities: usize) -> Self {
        Self {
            polities,
            supply: vec![vec![1.0; GOODS]; polities],
            motive: vec![vec![1.0; GOODS]; polities],
            transform: None,
        }
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

    pub fn with_transform(mut self, polity: usize, unit: usize, inputs: Vec<f32>, outputs: Vec<f32>) -> Self {
        self.transform = Some(Transform {
            polity,
            unit,
            inputs,
            outputs,
        });
        self
    }

    fn transforms(&self, polity: usize, unit: usize) -> Option<&Transform> {
        self.transform
            .as_ref()
            .filter(|transform| transform.polity == polity && transform.unit == unit)
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
    pub level: Vec<f32>,
    pub wedge: Vec<f32>,
    pub execution: f32,
    pub vwap: Vec<f32>,
    pub internal: f32,
    pub external: f32,
    pub cash: f32,
}

impl Polity {
    pub fn span(&self) -> std::ops::Range<usize> {
        self.seat..self.seat + UNITS
    }
}

pub struct Snapshot {
    pub round: usize,
    pub prices: Vec<f32>,
    pub wedges: Vec<Vec<f32>>,
    pub levels: Vec<Vec<f32>>,
    pub executions: Vec<f32>,
    pub vwaps: Vec<Vec<f32>>,
    pub internal: f32,
    pub external: f32,
    pub uncleared: f32,
    pub treasury: f32,
    pub gauge: Vec<f32>,
    pub transform_share: f32,
    pub transform_potential: f32,
}

pub struct Lab {
    pub departments: Departments,
    pub warehouses: Warehouses,
    pub market: Market,
    pub polities: Vec<Polity>,
    pub rule: LevelRule,
    pub forgetting: f32,
    pub gain: f32,
    pub recenter: bool,
    pub anchor: bool,
    pub gauge: Vec<f32>,
    pub round: usize,
    pub history: Vec<Snapshot>,
    rng: Rng,
}

impl Lab {
    pub fn new(spec: &Spec, seed: u64) -> Self {
        let count = spec.polities * UNITS;
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
                    Warehouse::new(stocks).with_reference(vec![BASE_PRICE; GOODS]),
                );
                let mut productions = vec![0.0; GOODS];
                productions[unit] = BASE * spec.supply(polity, unit);
                let policies = {
                    let mut policies: Vec<Policy> = (0..GOODS)
                        .filter(|good| *good != unit)
                        .map(|good| {
                            let mut consumptions = vec![0.0; GOODS];
                            consumptions[good] = CAMPAIGN;
                            Policy::new(consumptions, MOTIVE * spec.motive(polity, good))
                        })
                        .collect();
                    if let Some(transform) = spec.transforms(polity, unit) {
                        policies.push(Policy::transform(
                            transform.inputs.clone(),
                            transform.outputs.clone(),
                        ));
                    }
                    policies
                };
                departments.push(Department::new(productions, policies));
            }
            polities.push(Polity {
                name: NAMES[polity % NAMES.len()],
                seat: polity * UNITS,
                level: vec![BASE_PRICE; GOODS],
                wedge: vec![0.0; GOODS],
                execution: 1.0,
                vwap: vec![BASE_PRICE; GOODS],
                internal: 0.0,
                external: 0.0,
                cash: 0.0,
            });
        }
        let departments = Departments::new(departments).with_grants(vec![GRANT; count]);
        let mut lab = Self {
            departments,
            warehouses: Warehouses::new(warehouses).with_fluctuation(0.0),
            market,
            polities,
            rule: LevelRule::Counterparty,
            forgetting: 0.1,
            gain: 0.02,
            recenter: false,
            anchor: true,
            gauge: vec![0.0; GOODS],
            round: 0,
            history: Vec::new(),
            rng: Rng::with_seed(seed),
        };
        lab.apply_levels();
        let snapshot = lab.snapshot();
        lab.history.push(snapshot);
        lab
    }

    pub fn with_rule(mut self, rule: LevelRule) -> Self {
        self.rule = rule;
        self
    }

    pub fn with_forgetting(mut self, forgetting: f32) -> Self {
        self.forgetting = forgetting.clamp(0.0, 1.0);
        self
    }

    pub fn with_gain(mut self, gain: f32) -> Self {
        self.gain = gain.max(0.0);
        self
    }

    pub fn with_recenter(mut self, recenter: bool) -> Self {
        self.recenter = recenter;
        self
    }

    pub fn with_anchor(mut self, anchor: bool) -> Self {
        self.anchor = anchor;
        self
    }

    pub fn with_grant(mut self, grant: f32) -> Self {
        let count = self.departments.departments.len();
        self.departments.grants = vec![grant.max(0.0); count];
        self
    }

    pub fn with_relations(mut self, relations: &[Vec<f32>]) -> Self {
        self.market.set_relations(relations);
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

    /// 局部制裁：只掐掉这几个部门与政权外的配对，政权内部与其余部门照常
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
        self.block(&[], 1.0);
    }

    pub fn department_of(&self, polity: usize, unit: usize) -> usize {
        polity * UNITS + unit
    }

    /// 每个部门每种商品的成交均价除以银河指数，没有成交记 0
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

    /// 这个政权对某种商品的实际成交价相对银河指数的偏离，正数表示它比你别人买的贵
    pub fn spread(&self, polity: usize, good: usize) -> f32 {
        let own = self.polities[polity].vwap[good];
        let others: Vec<f32> = self
            .polities
            .iter()
            .enumerate()
            .filter(|(p, _)| *p != polity)
            .map(|(_, other)| other.vwap[good])
            .collect();
        if others.is_empty() {
            return 0.0;
        }
        own - others.iter().sum::<f32>() / others.len() as f32
    }

    pub fn step(&mut self) {
        self.apply_levels();
        let before: Vec<f32> = self
            .market
            .merchandises
            .iter()
            .map(|merchandise| merchandise.price.max(1e-9))
            .collect();
        self.departments
            .step(&mut self.warehouses, &mut self.market, &mut self.rng);
        self.gauge = self
            .market
            .merchandises
            .iter()
            .zip(before.iter())
            .map(|(merchandise, previous)| (merchandise.price.max(1e-9) / previous.max(1e-9)).ln())
            .collect();
        if self.anchor {
            self.anchor_prices();
        }
        self.update_levels();
        self.round += 1;
        let snapshot = self.snapshot();
        self.history.push(snapshot);
    }

    fn anchor_prices(&mut self) {
        let count = self.market.merchandises.len();
        if count == 0 {
            return;
        }
        let mean = self
            .market
            .merchandises
            .iter()
            .map(|merchandise| merchandise.price.max(1e-9).ln())
            .sum::<f32>()
            / count as f32;
        let scale = (BASE_PRICE.ln() - mean).exp();
        if !scale.is_finite() || scale <= 0.0 {
            return;
        }
        for merchandise in self.market.merchandises.iter_mut() {
            merchandise.price *= scale;
        }
    }

    pub fn run(&mut self, rounds: usize) {
        for _ in 0..rounds {
            self.step();
        }
    }

    fn apply_levels(&mut self) {
        let Self {
            polities,
            warehouses,
            market,
            ..
        } = self;
        let prices: Vec<f32> = market
            .merchandises
            .iter()
            .map(|merchandise| merchandise.price.max(1e-6))
            .collect();
        for polity in polities.iter_mut() {
            for good in 0..GOODS {
                polity.level[good] = prices[good] * polity.wedge[good].exp();
            }
            let level = polity.level.clone();
            for warehouse in &mut warehouses.warehouses[polity.span()] {
                warehouse.reference = level.clone();
            }
        }
    }

    fn update_levels(&mut self) {
        let rule = self.rule;
        let forgetting = self.forgetting;
        let gain = self.gain;
        let recenter = self.recenter;
        let Self {
            polities,
            warehouses,
            market,
            departments,
            ..
        } = self;
        let prices: Vec<f32> = market
            .merchandises
            .iter()
            .map(|merchandise| merchandise.price.max(1e-6))
            .collect();
        let mut observations = vec![vec![(0.0f32, 0.0f32, 0.0f32); GOODS]; polities.len()];
        let mut internal = vec![0.0f32; polities.len()];
        let mut external = vec![0.0f32; polities.len()];
        let traders = market.traders.len();
        for i in 0..traders {
            let polity = warehouse_polity(i);
            for k in 0..GOODS {
                let merchandise = &market.traders[i].merchandises[k];
                let volume = merchandise.deal_volume().abs();
                if volume <= 0.0 {
                    continue;
                }
                let deal = merchandise.deal_price();
                if !(deal > 0.0) {
                    continue;
                }
                let quote = merchandise.price.max(1e-6);
                let counterparty = 2.0 * (deal / prices[k]).ln() - (quote / prices[k]).ln();
                let entry = &mut observations[polity][k];
                entry.0 += volume;
                entry.1 += volume * deal;
                entry.2 += volume * counterparty;
            }
            for j in 0..traders {
                let other = warehouse_polity(j);
                for k in 0..GOODS {
                    let volume = market.deals[i][j][k].volume.abs();
                    if volume <= 0.0 {
                        continue;
                    }
                    if other == polity {
                        internal[polity] += volume;
                    } else {
                        external[polity] += volume;
                    }
                }
            }
        }
        for (p, polity) in polities.iter_mut().enumerate() {
            for k in 0..GOODS {
                let (volume, value, counterparty) = observations[p][k];
                let vwap = if volume > 0.0 {
                    value / volume / prices[k]
                } else {
                    polity.vwap[k]
                };
                polity.vwap[k] = vwap;
                match rule {
                    LevelRule::Fixed => {}
                    LevelRule::OwnVwap => {
                        if volume > 0.0 && vwap > 0.0 {
                            polity.wedge[k] += forgetting * (vwap.ln() - polity.wedge[k]);
                        }
                    }
                    LevelRule::Counterparty => {
                        if volume > 0.0 {
                            let observed = counterparty / volume;
                            polity.wedge[k] += forgetting * (observed - polity.wedge[k]);
                        }
                    }
                    LevelRule::Shortfall | LevelRule::Pressure => {
                        let mut need = 0.0;
                        let mut surplus = 0.0;
                        for warehouse in &warehouses.warehouses[polity.span()] {
                            let stock = &warehouse.stocks[k];
                            need += (stock.target_volume - stock.volume).max(0.0);
                            surplus += (stock.volume - stock.target_volume).max(0.0);
                        }
                        let pressure = (need - surplus) / (need + surplus + BASE);
                        polity.wedge[k] += gain * pressure;
                        if rule == LevelRule::Pressure {
                            polity.wedge[k] -= forgetting * polity.wedge[k];
                        }
                    }
                }
                polity.wedge[k] = polity.wedge[k].clamp(-MAX_WEDGE, MAX_WEDGE);
            }
            polity.internal = internal[p];
            polity.external = external[p];
            polity.execution = departments.departments[polity.span()]
                .iter()
                .map(|department| department.policy_execution())
                .sum::<f32>()
                / UNITS as f32;
            polity.cash = warehouses.warehouses[polity.span()]
                .iter()
                .map(|warehouse| warehouse.currency)
                .sum();
        }
        if recenter {
            for k in 0..GOODS {
                let mean = polities
                    .iter()
                    .map(|polity| polity.wedge[k])
                    .sum::<f32>()
                    / polities.len() as f32;
                for polity in polities.iter_mut() {
                    polity.wedge[k] -= mean;
                }
            }
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        let transform = self
            .departments
            .departments
            .iter()
            .flat_map(|department| department.policies.iter())
            .find(|policy| policy.is_transform());
        let mut declared = 0.0;
        let mut dealt = 0.0;
        for k in 0..GOODS {
            for trader in &self.market.traders {
                declared += trader.merchandises[k].volume.max(0.0);
            }
            for row in &self.market.deals {
                for deals in row {
                    dealt += deals[k].volume.max(0.0);
                }
            }
        }
        Snapshot {
            round: self.round,
            prices: self
                .market
                .merchandises
                .iter()
                .map(|merchandise| merchandise.price)
                .collect(),
            wedges: self
                .polities
                .iter()
                .map(|polity| polity.wedge.clone())
                .collect(),
            levels: self
                .polities
                .iter()
                .map(|polity| polity.level.clone())
                .collect(),
            executions: self
                .polities
                .iter()
                .map(|polity| polity.execution)
                .collect(),
            vwaps: self
                .polities
                .iter()
                .map(|polity| polity.vwap.clone())
                .collect(),
            internal: self.polities.iter().map(|polity| polity.internal).sum(),
            external: self.polities.iter().map(|polity| polity.external).sum(),
            uncleared: if declared > 0.0 {
                (declared - dealt) / declared
            } else {
                0.0
            },
            treasury: self.departments.treasury,
            gauge: self.gauge.clone(),
            transform_share: transform.map_or(0.0, |policy| policy.distribution()),
            transform_potential: transform.map_or(0.0, |policy| policy.price_potential()),
        }
    }

    pub fn levels(&self) -> Vec<Vec<f32>> {
        self.polities
            .iter()
            .map(|polity| polity.level.clone())
            .collect()
    }

    pub fn wedges(&self) -> Vec<Vec<f32>> {
        self.polities
            .iter()
            .map(|polity| polity.wedge.clone())
            .collect()
    }
}

pub fn warehouse_polity(warehouse: usize) -> usize {
    warehouse / UNITS
}

pub fn bloc_relations(polities: usize, outside: f32) -> Vec<Vec<f32>> {
    let traders = polities * UNITS;
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
