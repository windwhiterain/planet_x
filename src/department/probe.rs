use std::f32::consts::{PI, TAU};

use fastrand::Rng;

use super::{Department, Departments, Policy};
use crate::estimator::Estimator;
use crate::market::{Market, Merchandise, Trader, TraderMerchandise};
use crate::warehouse::{SellerRule, Stock, Warehouse, Warehouses};

const GOODS: usize = 3;
const DEPARTMENTS: usize = 3;
const BASE: f32 = 4.0;
const MOTIVE: f32 = 1.0;
const SUPPLY: f32 = 30.0;
const WARMUP: usize = 12;
const ROUNDS: usize = 48;
const PERIODS: [f32; 3] = [3.0, 6.0, 12.0];
const AMPLITUDES: [f32; 5] = [0.0, 0.02, 0.05, 0.1, 0.25];
const PATTERNS: [(&str, [f32; DEPARTMENTS]); 3] = [
    ("同相", [0.0, 0.0, 0.0]),
    ("递相", [0.0, TAU / 3.0, 2.0 * TAU / 3.0]),
    ("反相", [0.0, PI, 0.0]),
];

#[derive(Clone, Copy, PartialEq)]
pub enum Treasury {
    /// 国内：回收货币，统一重新发放
    Redistributing,
    /// 国际：不收回，不发放
    Free,
}

pub struct Report {
    pub regime: &'static str,
    pub period: f32,
    pub amplitude: f32,
    pub pattern: &'static str,
    pub min_price: f32,
    pub max_price: f32,
    pub drift: f32,
    pub min_stock: f32,
    pub max_stock: f32,
    pub uncleared: f32,
    pub frozen: usize,
    pub overdrafts: usize,
    pub min_cash: f32,
    pub max_cash: f32,
    pub supply: f32,
}

impl Report {
    pub fn line(&self) -> String {
        format!(
            "{} 周期 {:>4} 幅度 {:>5.2} {}：价格 {:.3}~{:.3} 漂移 {:+.1}% 库存 {:.1}~{:.1} 未成交 {:>5.1}% 冻结 {} 透支 {} 现金 {:.2}~{:.2} 货币 {:.2}",
            self.regime,
            self.period,
            self.amplitude,
            self.pattern,
            self.min_price,
            self.max_price,
            100.0 * self.drift,
            self.min_stock,
            self.max_stock,
            self.uncleared,
            self.frozen,
            self.overdrafts,
            self.min_cash,
            self.max_cash,
            self.supply,
        )
    }

    pub fn verdict(&self) -> &'static str {
        if self.frozen > 0 || self.overdrafts > 0 || self.uncleared > 0.5 {
            "失稳"
        } else if self.drift.abs() > 0.05 || self.max_price > self.min_price * 4.0 {
            "偏"
        } else {
            "稳"
        }
    }
}

pub struct Round {
    pub round: usize,
    pub prices: [f32; GOODS],
    pub stocks: [[f32; GOODS]; DEPARTMENTS],
    pub targets: [[f32; GOODS]; DEPARTMENTS],
    pub declared: [[f32; GOODS]; DEPARTMENTS],
    pub deal_price: [[f32; GOODS]; DEPARTMENTS],
    pub sell_scale: [[f32; GOODS]; DEPARTMENTS],
    pub quote: [[f32; GOODS]; DEPARTMENTS],
    pub sell_slope: [[f32; GOODS]; DEPARTMENTS],
    pub sell_intercept: [[f32; GOODS]; DEPARTMENTS],
    pub cash: [f32; DEPARTMENTS],
    pub net: [[f32; GOODS]; DEPARTMENTS],
    pub execution: [f32; DEPARTMENTS],
}

/// 某个部门在它自产那种商品上的学习曲线与实况
pub struct Curve {
    pub department: usize,
    pub declared: f32,
    pub dealt: f32,
    pub learned: f32,
    pub realized: f32,
    pub slope: f32,
    pub intercept: f32,
}

impl Curve {
    pub fn line(&self) -> String {
        format!(
            "[{}] 申报 {:+.2} 成交 {:+.2} 学价 {:.3} 实价 {:.3} (斜率 {:+.2} 截距 {:+.2})",
            ["甲", "乙", "丙"][self.department],
            self.declared,
            self.dealt,
            self.learned,
            self.realized,
            self.slope,
            self.intercept,
        )
    }
}

impl Round {
    pub fn curve(&self, department: usize) -> Curve {
        let good = department;
        let market = self.prices[good];
        Curve {
            department,
            declared: self.declared[department][good],
            dealt: self.net[department][good],
            learned: self.sell_scale[department][good],
            realized: if market > 0.0 {
                self.deal_price[department][good] / market
            } else {
                0.0
            },
            slope: self.sell_slope[department][good],
            intercept: self.sell_intercept[department][good],
        }
    }

    /// 某种商品的报价交叉情况：谁卖、谁买、各自报价多少
    pub fn quotes(&self, good: usize) -> String {
        let mut parts = vec![format!("市价 {:.3}", self.prices[good])];
        for i in 0..DEPARTMENTS {
            let declared = self.declared[i][good];
            parts.push(format!(
                "{} {} {:+.2} 学价 {:.3} 报价 {:.3} 实价 {:.3} 成交 {:+.2}",
                ["甲", "乙", "丙"][i],
                if declared > 0.0 { "卖" } else { "买" },
                declared,
                self.sell_scale[i][good],
                self.quote[i][good],
                if self.prices[good] > 0.0 {
                    self.deal_price[i][good] / self.prices[good]
                } else {
                    0.0
                },
                self.net[i][good],
            ));
        }
        parts.join(" | ")
    }
}

struct Observation {
    prices: [f32; GOODS],
    stocks: [[f32; GOODS]; DEPARTMENTS],
    targets: [[f32; GOODS]; DEPARTMENTS],
    declared: [[f32; GOODS]; DEPARTMENTS],
    deal_price: [[f32; GOODS]; DEPARTMENTS],
    sell_scale: [[f32; GOODS]; DEPARTMENTS],
    quote: [[f32; GOODS]; DEPARTMENTS],
    sell_slope: [[f32; GOODS]; DEPARTMENTS],
    sell_intercept: [[f32; GOODS]; DEPARTMENTS],
    net: [[f32; GOODS]; DEPARTMENTS],
    execution: [f32; DEPARTMENTS],
    stock: f32,
    uncleared: f32,
    frozen: bool,
    cash: [f32; DEPARTMENTS],
}

struct Probe {
    departments: Departments,
    warehouses: Warehouses,
    market: Market,
    rng: Rng,
    period: f32,
    amplitude: f32,
    phases: [f32; DEPARTMENTS],
    treasury: Treasury,
    observations: Vec<Observation>,
}

fn market() -> Market {
    Market::new(
        (0..GOODS).map(|_| Merchandise { price: 1.0 }).collect(),
        (0..DEPARTMENTS)
            .map(|_| Trader {
                merchandises: (0..GOODS)
                    .map(|_| TraderMerchandise::new(0.0, 0.0))
                    .collect(),
            })
            .collect(),
    )
}

fn warehouses(treasury: Treasury) -> Warehouses {
    Warehouses::new(
        (0..DEPARTMENTS)
            .map(|department| {
                let mut stocks = Vec::with_capacity(GOODS);
                for _ in 0..GOODS {
                    stocks.push(
                        Stock::new(BASE / 2.0, BASE / 2.0).with_seller_rule(SellerRule::TargetVolume),
                    );
                }
                let warehouse = Warehouse::new(stocks);
                match treasury {
                    Treasury::Redistributing => warehouse,
                    Treasury::Free => warehouse.with_currency(SUPPLY / DEPARTMENTS as f32),
                }
            })
            .collect(),
    )
    .with_fluctuation(0.0)
}

fn departments(treasury: Treasury) -> Departments {
    let departments = (0..DEPARTMENTS)
        .map(|department| {
            let mut productions = vec![0.0; GOODS];
            productions[department] = BASE / 2.0;
            productions[(department + 1) % GOODS] = BASE / 2.0;
            Department::new(
                productions,
                (0..GOODS)
                    .filter(|good| *good != department)
                    .map(|good| {
                        let mut consumptions = vec![0.0; GOODS];
                        consumptions[good] = BASE;
                        Policy::new(consumptions, MOTIVE)
                    })
                    .collect(),
            )
        })
        .collect();
    match treasury {
        Treasury::Redistributing => {
            let mut departments = Departments::new(departments);
            departments.treasury = SUPPLY;
            departments
        }
        Treasury::Free => Departments::new(departments),
    }
}

fn factor(round: usize, period: f32, phase: f32, amplitude: f32) -> f32 {
    (1.0 + amplitude * (TAU * round as f32 / period + phase).sin()).max(0.05)
}

impl Probe {
    fn new(
        treasury: Treasury,
        period: f32,
        amplitude: f32,
        phases: [f32; DEPARTMENTS],
        seed: u64,
    ) -> Self {
        Self {
            departments: departments(treasury),
            warehouses: warehouses(treasury),
            market: market(),
            rng: Rng::with_seed(seed),
            period,
            amplitude,
            phases,
            treasury,
            observations: Vec::new(),
        }
    }

    fn regime(&self) -> &'static str {
        match self.treasury {
            Treasury::Redistributing => "国内",
            Treasury::Free => "国际",
        }
    }

    fn pattern(&self) -> &'static str {
        PATTERNS
            .iter()
            .find(|(_, phases)| *phases == self.phases)
            .map(|(name, _)| *name)
            .unwrap_or("自定")
    }

    fn step(&mut self, round: usize) {
        for (i, department) in self.departments.departments.iter_mut().enumerate() {
            for good in [i, (i + 1) % GOODS] {
                department.productions[good] =
                    BASE / 2.0 * factor(round, self.period, self.phases[i], self.amplitude);
            }
        }
        match self.treasury {
            Treasury::Redistributing => self.departments.step_redistributing(
                &mut self.warehouses,
                &mut self.market,
                &mut self.rng,
            ),
            Treasury::Free => {
                self.departments
                    .step_free(&mut self.warehouses, &mut self.market, &mut self.rng)
            }
        }

        let mut observation = Observation {
            prices: [0.0; GOODS],
            stocks: [[0.0; GOODS]; DEPARTMENTS],
            targets: [[0.0; GOODS]; DEPARTMENTS],
            declared: [[0.0; GOODS]; DEPARTMENTS],
            deal_price: [[0.0; GOODS]; DEPARTMENTS],
            sell_scale: [[0.0; GOODS]; DEPARTMENTS],
            quote: [[0.0; GOODS]; DEPARTMENTS],
            sell_slope: [[0.0; GOODS]; DEPARTMENTS],
            sell_intercept: [[0.0; GOODS]; DEPARTMENTS],
            net: [[0.0; GOODS]; DEPARTMENTS],
            execution: [0.0; DEPARTMENTS],
            stock: 0.0,
            uncleared: 0.0,
            frozen: false,
            cash: [0.0; DEPARTMENTS],
        };
        for (k, merchandise) in self.market.merchandises.iter().enumerate() {
            observation.prices[k] = merchandise.price;
        }
        for i in 0..DEPARTMENTS {
            observation.execution[i] = self.departments.departments[i].policy_execution();
            for (k, stock) in self.warehouses.warehouses[i].stocks.iter().enumerate() {
                observation.stocks[i][k] = stock.volume;
                observation.targets[i][k] = stock.target_volume;
                observation.stock += stock.volume;
                let declared = self.market.traders[i].merchandises[k].volume;
                let sell = stock.sell_volume2price_scale();
                let buy = stock.buy_volume2price_scale();
                let scale = if declared > 0.0 {
                    sell.get(declared)
                } else {
                    buy.get(declared.abs())
                };
                observation.declared[i][k] = declared;
                observation.sell_scale[i][k] = scale;
                observation.quote[i][k] = observation.prices[k] * scale;
                observation.sell_slope[i][k] = sell.slope();
                observation.sell_intercept[i][k] = sell.intercept();
                observation.deal_price[i][k] = self.market.traders[i].merchandises[k].deal_price();
            }
            observation.cash[i] = self.warehouses.warehouses[i].currency;
            for k in 0..GOODS {
                observation.net[i][k] = self.market.traders[i].merchandises[k].deal_volume();
            }
        }
        let mut declared = 0.0;
        let mut dealt = 0.0;
        for k in 0..GOODS {
            for i in 0..DEPARTMENTS {
                declared += self.market.traders[i].merchandises[k].volume.max(0.0);
            }
            for row in &self.market.deals {
                for deals in row {
                    dealt += deals[k].volume.max(0.0);
                }
            }
        }
        observation.uncleared = if declared > 0.0 {
            (declared - dealt) / declared
        } else {
            0.0
        };
        observation.frozen = dealt <= 0.0;
        self.observations.push(observation);
    }

    fn advance(&mut self, rounds: usize) {
        self.observations.clear();
        for round in 0..rounds {
            self.step(round);
        }
    }

    fn report(&self) -> Report {
        let settled = &self.observations[WARMUP..];
        let mut min_price = f32::INFINITY;
        let mut max_price = 0.0f32;
        let mut min_stock = f32::INFINITY;
        let mut max_stock = 0.0f32;
        let mut min_cash = f32::INFINITY;
        let mut max_cash = 0.0f32;
        let mut uncleared = 0.0;
        let mut frozen = 0;
        let mut overdrafts = 0;
        for observation in settled {
            for price in observation.prices {
                min_price = min_price.min(price);
                max_price = max_price.max(price);
            }
            min_stock = min_stock.min(observation.stock);
            max_stock = max_stock.max(observation.stock);
            uncleared += observation.uncleared;
            frozen += usize::from(observation.frozen);
            for cash in observation.cash {
                min_cash = min_cash.min(cash);
                max_cash = max_cash.max(cash);
                overdrafts += usize::from(cash < -1e-3);
            }
        }
        let half = settled.len() / 2;
        let early = mean_price(&settled[..half]);
        let late = mean_price(&settled[half..]);
        let supply: f32 = self
            .warehouses
            .warehouses
            .iter()
            .map(|warehouse| warehouse.currency)
            .sum::<f32>()
            + self.departments.treasury;
        Report {
            regime: self.regime(),
            period: self.period,
            amplitude: self.amplitude,
            pattern: self.pattern(),
            min_price,
            max_price,
            drift: if early > 0.0 { (late - early) / early } else { 0.0 },
            min_stock,
            max_stock,
            uncleared: uncleared / settled.len() as f32,
            frozen,
            overdrafts,
            min_cash,
            max_cash,
            supply,
        }
    }
}

fn mean_price(observations: &[Observation]) -> f32 {
    let mut total = 0.0;
    let mut count = 0.0;
    for observation in observations {
        for price in observation.prices {
            total += price;
            count += 1.0;
        }
    }
    if count > 0.0 { total / count } else { 0.0 }
}

pub fn amplitudes() -> &'static [f32] {
    &AMPLITUDES
}

/// 相位互补程度与周期的扫描参数
pub const SWEEP_PERIODS: [f32; 4] = [3.0, 6.0, 12.0, 24.0];
pub const SWEEP_PHASES: [f32; 7] = [
    0.0,
    PI / 6.0,
    PI / 3.0,
    PI / 2.0,
    2.0 * PI / 3.0,
    5.0 * PI / 6.0,
    PI,
];

/// 三个部门依次相差 delta 相位；delta = 0 完全同相，delta = π 最互补
pub fn sweep(treasury: Treasury, period: f32, delta: f32, amplitude: f32) -> Report {
    sweep_offset(treasury, period, delta, 0.0, amplitude)
}

/// offset = 0 为正正弦（涨在先），offset = π 为负正弦（跌在先）
pub fn sweep_offset(
    treasury: Treasury,
    period: f32,
    delta: f32,
    offset: f32,
    amplitude: f32,
) -> Report {
    let phases = [offset, offset + delta, offset + 2.0 * delta];
    Probe::new(treasury, period, amplitude, phases, 11).run_report()
}

pub fn cycles(treasury: Treasury, amplitude: f32) -> Vec<Report> {
    let mut reports = Vec::new();
    for period in PERIODS {
        for (_, phases) in PATTERNS {
            reports.push(Probe::new(treasury, period, amplitude, phases, 11).run_report());
        }
    }
    reports
}

/// 国内经济循环：回收货币，统一重新发放
pub fn domestic_cycle(amplitude: f32) -> Vec<Report> {
    cycles(Treasury::Redistributing, amplitude)
}

/// 国际经济循环：不收回，不发放
pub fn international_cycle(amplitude: f32) -> Vec<Report> {
    cycles(Treasury::Free, amplitude)
}

/// 逐轮明细：产量波动如何传进价格、库存与成交
pub fn trace(
    treasury: Treasury,
    period: f32,
    amplitude: f32,
    phases: [f32; DEPARTMENTS],
    rounds: usize,
) -> Vec<Round> {
    let mut probe = Probe::new(treasury, period, amplitude, phases, 11);
    probe.advance(rounds);
    probe
        .observations
        .into_iter()
        .enumerate()
        .map(|(round, observation)| Round {
            round,
            prices: observation.prices,
            stocks: observation.stocks,
            targets: observation.targets,
            declared: observation.declared,
            deal_price: observation.deal_price,
            sell_scale: observation.sell_scale,
            quote: observation.quote,
            sell_slope: observation.sell_slope,
            sell_intercept: observation.sell_intercept,
            cash: observation.cash,
            net: observation.net,
            execution: observation.execution,
        })
        .collect()
}

pub const TRACE_PHASES: [f32; DEPARTMENTS] = [0.0, 0.0, 0.0];

impl Probe {
    fn run_report(mut self) -> Report {
        self.advance(ROUNDS);
        self.report()
    }
}
