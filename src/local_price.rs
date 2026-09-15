#[cfg(test)]
mod tests;


use crate::department::{Department, Departments, Policy, Rationing};
use crate::market::{Market, Merchandise, Trader, TraderMerchandise};
use crate::warehouse::{Stock, Warehouse, Warehouses};

pub const GOODS: usize = 3;
pub const UNITS: usize = 3;

/// 每个单元拆成两类部门：**生产部门**（只留生产政策，仓库里是投入品与产出品）
/// 与**消费部门**（只留消费政策，仓库里是口粮，靠市场与转移支付过日子）。
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

/// 专精：把"这个部门自己那一层"的产出乘上 `factor`。
///
/// 只动**产出**，不动投入、不动产能占用——所以它改的是绝对效率，而每个部门
/// 在别的层上仍然和别人一样，于是"谁该干什么"由相对效率决定（比较优势），
/// 而不是由"谁能干什么"决定（绝对壁垒）。
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
    /// 专精：第 `u` 号部门生产**自己那一层**（商品 `u`）的工艺产出乘数。
    ///
    /// `1.0` = 同构（历史行为），`> 1` = 有比较优势。**异质性从这里来**：
    /// 九个部门同构时，"自己造"永远不比"买"贵，于是每个部门在自己内部把整条链跑完，
    /// 成交恒为 0（§13.5 第 2 条）。让每个部门各自擅长一层，交易才有理由发生。
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

    /// 每个部门擅长自己那一层：第 `u` 号部门生产商品 `u` 的工艺产出乘 `factor`。
    ///
    /// 这是"比较优势"的最小实现：各层仍然**人人可开**（wildcard 没动），
    /// 所以交易是划算而不是必须——异质性负责让买比造便宜，不负责强迫分工。
    /// 这一层的专精强度
    pub fn specialty_of(&self, good: usize) -> f32 {
        self.specialty.get(good).copied().unwrap_or(1.0)
    }

    pub fn with_specialty(mut self, factor: f32) -> Self {
        let factor = if factor.is_finite() && factor > 0.0 { factor } else { 1.0 };
        self.specialty = vec![factor; GOODS];
        self
    }

    /// 只抬某一层的专精强度（`good` 是那一层的商品序号）
    pub fn with_specialty_at(mut self, good: usize, factor: f32) -> Self {
        if let Some(slot) = self.specialty.get_mut(good) {
            if factor.is_finite() && factor > 0.0 {
                *slot = factor;
            }
        }
        self
    }

    /// 人人可搞一产、人人消耗三种产物、人人都能开各种工业。
    /// 一产产能便宜 ⇒ 人人搞 ⇒ 过剩 ⇒ 贱；工业占产能 ⇒ 稀缺 ⇒ 贵
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

    /// 三层投入产出：一产零投入、二产吃一产、三产吃二产，产能占用逐层变高
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

    /// 同一个部门挂两个工艺：省料但慢、费料但快。切换点在 工业品价 ÷ 粮食价 = 1
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
        self.transforms
            .iter()
            .filter(move |transform| {
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

/// 一种商品在某一轮的全场状态
#[derive(Clone, Copy, Default)]
pub struct GoodState {
    /// 各仓库挂价的几何平均
    pub index: f32,
    /// 成交价（按成交量加权），没有成交记 0
    pub deal_price: f32,
    /// 各地方挂价区间的平均
    pub bid: f32,
    pub ask: f32,
    /// 卖方加权挂价与买方加权挂价
    pub quote_sell: f32,
    pub quote_buy: f32,
    /// 各仓库自适应目标的合计
    pub target: f32,
    /// 成交量（只数一次）
    pub dealt: f32,
    /// 卖方与买方各自成交的量
    pub sold: f32,
    pub bought: f32,
    /// 全场库存合计
    pub stock: f32,
    /// 本轮部门想取走的量
    pub wanted: f32,
    /// 本轮部门实际取走的量
    pub consumed: f32,
    /// 本轮实际入账的产出
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
    /// 消费结算**没解出来**的次数（逐部门逐轮计）。不许静默继续：解不出来必须在读数里看得见。
    pub settlement_failures: usize,
    /// 结算过程中出现过退化（Cholesky 或回溯失败）但最终仍然收敛的次数。
    /// 与 `settlement_failures` 分开报：前者是"没解出来"，后者只是"路上磕了一下"。
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
                        // 自有商品是 0：生产者的自有商品本来就该全卖，消费者不吃自己那样。
                        // 其余给一场战役的量——生产者的**投入品**和消费者的**口粮**都从
                        // 这里来，所以这条规则拆前拆后都成立。目标从第二轮起被
                        // "三倍取货量"覆盖，初值只在第一轮起作用。
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
                        // 生产部门：只有生产政策。它**不吃东西**——两族之间只剩市场这一条
                        // 通道，价格才有活干。
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
                        // 消费部门：只有消费政策，产出为零，靠每轮的拨款过日子。
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
        // 货币是**存量**：开局每部门发一份，之后靠"卖产出收钱、买消费付钱"自己滚动，
        // 再由转移支付在部门之间调剂。仓库不持钱。
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

    /// 势流的价差尺度，见 [`crate::market::Market::with_flow_scale`]
    pub fn with_flow_scale(mut self, scale: f32) -> Self {
        self.market = self.market.with_flow_scale(scale);
        self
    }

    /// 消费结算规则，见 [`crate::department::Rationing`]
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

    /// 价格律：`price = price_base · exp(−κ·tanh(ln(volume/target)))`，先过一阶低通
    pub fn with_price_law(mut self, curvature: f32, inertia: f32, target_rate: f32) -> Self {
        self.warehouses
            .set_price_law(curvature, inertia, target_rate);
        self
    }

    /// 转移支付速率：每轮把部门余额按这个比例拉向均值
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
        let traders = self.market.traders.len();
        self.market
            .set_relations(&vec![vec![1.0; traders]; traders]);
    }

    /// 某个政权、某个单元里的一类部门（生产或消费）
    pub fn department_of(&self, polity: usize, unit: usize, kind: Kind) -> usize {
        polity * 2 * UNITS + unit * 2 + if kind == Kind::Producer { 0 } else { 1 }
    }

    /// 某个部门的各个转换工艺的（份额，单位产能利润，产能占用）
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

    /// 每种商品这一轮的全场状态。报价与指数分开给：报价是挂出来的，指数是成交出来的
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
            state.quote_sell = if sell > 0.0 { state.quote_sell / sell } else { 0.0 };
            state.quote_buy = if buy > 0.0 { state.quote_buy / buy } else { 0.0 };
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
                state.stock += warehouse.stocks.get(k).map(|stock| stock.volume).unwrap_or(0.0);
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

    /// 某个地方的**挂价**相对银河指数的偏离，正数表示它比别人挂得贵。
    /// 纯挂价口径：不依赖成交，也不掺成交价。
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
        self.departments.step(&mut self.warehouses, &mut self.market);
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
