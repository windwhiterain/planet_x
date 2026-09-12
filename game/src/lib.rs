use fastrand::Rng;
use planet_x::department::{Department, Departments, Policy};
use planet_x::market::{Market, Merchandise, Trader, TraderMerchandise};
use planet_x::warehouse::{SellerRule, Stock, Warehouse, Warehouses};

pub const GOODS: usize = 3;
pub const DEPARTMENTS: usize = 3;

pub const GOOD_NAMES: [&str; GOODS] = ["粮食", "工业品", "服务"];
pub const DEPARTMENT_NAMES: [&str; DEPARTMENTS] = ["甲(农业)", "乙(工业)", "丙(服务)"];

/// 每个部门生产一种商品，另外两个部门用得上
const PRODUCTION: f32 = 4.0;
/// 一条政策满额消耗的商品量
const CAMPAIGN: f32 = 4.0;
/// 政策的意愿支付
const MOTIVE: f32 = 20.0;
/// 中央政府对每个部门的每轮拨款
const GRANT: f32 = 10.0;
const BASE_PRICE: f32 = 1.0;

pub fn production(department: usize) -> Vec<f32> {
    let mut productions = vec![0.0; GOODS];
    productions[department] = PRODUCTION;
    productions
}

pub fn campaign(good: usize) -> Vec<f32> {
    let mut consumptions = vec![0.0; GOODS];
    consumptions[good] = CAMPAIGN;
    consumptions
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub round: usize,
    pub prices: [f32; GOODS],
    pub cpi: f32,
    pub output: f32,
    pub consumption: f32,
    pub turnover: f32,
    pub granted: f32,
    pub treasury: f32,
    pub executions: [f32; DEPARTMENTS],
    pub revenues: [f32; DEPARTMENTS],
    pub payments: [f32; DEPARTMENTS],
    pub holdings: [[f32; GOODS]; DEPARTMENTS],
}

pub struct DomesticEconomy {
    pub departments: Departments,
    pub warehouses: Warehouses,
    pub market: Market,
    pub round: usize,
    pub history: Vec<Snapshot>,
    rng: Rng,
}

impl DomesticEconomy {
    pub fn new(seed: u64) -> Self {
        let market = Market::new(
            (0..GOODS)
                .map(|_| Merchandise { price: BASE_PRICE })
                .collect(),
            (0..DEPARTMENTS)
                .map(|_| Trader {
                    merchandises: (0..GOODS)
                        .map(|_| TraderMerchandise::new(0.0, 0.0))
                        .collect(),
                })
                .collect(),
        );

        let warehouses = Warehouses::new(
            (0..DEPARTMENTS)
                .map(|department| {
                    let mut stocks = Vec::with_capacity(GOODS);
                    for good in 0..GOODS {
                        let (volume, target) = if good == department {
                            (PRODUCTION, 2.0 * PRODUCTION)
                        } else {
                            (CAMPAIGN / 2.0, 0.0)
                        };
                        stocks.push(
                            Stock::new(volume, target).with_seller_rule(SellerRule::TargetVolume),
                        );
                    }
                    Warehouse::new(stocks)
                })
                .collect(),
        )
        .with_fluctuation(0.0);

        let departments = Departments::new(
            (0..DEPARTMENTS)
                .map(|department| {
                    Department::new(
                        production(department),
                        (0..GOODS)
                            .filter(|good| *good != department)
                            .map(|good| Policy::new(campaign(good), MOTIVE))
                            .collect(),
                    )
                })
                .collect(),
        )
        .with_grants(vec![GRANT; DEPARTMENTS]);

        let mut economy = Self {
            departments,
            warehouses,
            market,
            round: 0,
            history: Vec::new(),
            rng: Rng::with_seed(seed),
        };
        economy.history.push(economy.snapshot());
        economy
    }

    pub fn step(&mut self) {
        self.departments
            .step(&mut self.warehouses, &mut self.market, &mut self.rng);
        self.round += 1;
        let snapshot = self.snapshot();
        self.history.push(snapshot);
    }

    pub fn with_fluctuation(mut self, fluctuation: f32) -> Self {
        self.warehouses = self.warehouses.with_fluctuation(fluctuation);
        self
    }

    pub fn run(&mut self, rounds: usize) {
        for _ in 0..rounds {
            self.step();
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        let prices = self.prices();
        let base = self.history.first().map(|snapshot| snapshot.prices);
        let cpi = match base {
            Some(base) => 100.0 * weighted(&prices, &base),
            None => 100.0,
        };

        let mut executions = [0.0; DEPARTMENTS];
        let mut revenues = [0.0; DEPARTMENTS];
        let mut payments = [0.0; DEPARTMENTS];
        let mut holdings = [[0.0; GOODS]; DEPARTMENTS];
        let mut consumption = 0.0;
        for (i, department) in self.departments.departments.iter().enumerate() {
            executions[i] = department.policy_execution();
            let execution = department.policy_execution();
            for policy in &department.policies {
                let share = policy.distribution() * execution;
                for (k, intake) in policy.consumptions.iter().enumerate() {
                    consumption += share * intake.max(0.0) * prices[k];
                }
            }
            for (k, stock) in self.warehouses.warehouses[i].stocks.iter().enumerate() {
                holdings[i][k] = stock.volume;
            }
        }

        let mut turnover = 0.0;
        for (i, row) in self.market.deals.iter().enumerate() {
            for deals in row {
                for deal in deals {
                    turnover += deal.volume.abs() * deal.price;
                    if deal.volume > 0.0 {
                        revenues[i] += deal.volume * deal.price;
                    } else {
                        payments[i] += -deal.volume * deal.price;
                    }
                }
            }
        }
        turnover /= 2.0;

        let mut output = 0.0;
        for department in &self.departments.departments {
            for (k, productions) in department.productions.iter().enumerate() {
                output += productions.max(0.0) * prices[k];
            }
        }

        Snapshot {
            round: self.round,
            prices,
            cpi,
            output,
            consumption,
            turnover,
            granted: self.departments.grants.iter().sum(),
            treasury: self.departments.treasury,
            executions,
            revenues,
            payments,
            holdings,
        }
    }

    pub fn prices(&self) -> [f32; GOODS] {
        let mut prices = [0.0; GOODS];
        for (k, merchandise) in self.market.merchandises.iter().enumerate() {
            prices[k] = merchandise.price;
        }
        prices
    }
}

fn weighted(prices: &[f32; GOODS], base: &[f32; GOODS]) -> f32 {
    let mut numerator = 0.0;
    let mut denominator = 0.0;
    for k in 0..GOODS {
        numerator += prices[k];
        denominator += base[k];
    }
    if denominator > 0.0 {
        numerator / denominator
    } else {
        1.0
    }
}
