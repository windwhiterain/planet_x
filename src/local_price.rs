#[cfg(test)]
mod tests;

use fastrand::Rng;

use crate::department::{Department, Departments, Policy, Rationing};
use crate::market::{Market, Merchandise, Trader, TraderMerchandise};
use crate::warehouse::{Stock, Warehouse, Warehouses};

pub const GOODS: usize = 3;
pub const UNITS: usize = 3;

/// 默认申报涨落幅度：0.05 给出约 0.5–2 倍的乘性噪声
pub const DEFAULT_FLUCTUATION: f32 = 0.05;

/// 每个单元拆成两类部门：**生产部门**（只留生产政策，仓库里是投入品与产出品）
/// 与**消费部门**（只留消费政策，仓库里是口粮，靠拨款过日子）。
///
/// 拆的理由是量出来的：一个部门一个仓库、一个 LP 的时候，消费政策与生产政策在同一个
/// 目标里抢同一批库存，需求曲线会把生产者的投入品也吃光；而且部门既吃又产，
/// "谁卖"永远悬空——最后没人有货可卖，`dealt = 0`、指数冻在初值。
/// 拆开之后市场是两族之间**唯一**的通道，价格才有活干。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Producer,
    Consumer,
}
pub const BASE: f32 = 4.0;
pub const CAMPAIGN: f32 = 4.0;
pub const MOTIVE: f32 = 20.0;
pub const GRANT: f32 = 10.0;
pub const BASE_PRICE: f32 = 1.0;

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
    /// 申报量的乘性涨落幅度。**必须大于 0**：确定性市场里估计器没有激励，
    /// 一条退化的轨迹（学出 `1e-27` 的局部价之类）是吸收态，永远纠正不回来。
    /// `fluctuation_factor` 给出 `(u/(1−u))^amplitude`，`u ~ U(1e-6, 1−1e-6)`，
    /// 所以 0.05 ≈ 0.5–2 倍、0.1 ≈ 0.25–4 倍。
    pub fluctuation: f32,
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
            fluctuation: DEFAULT_FLUCTUATION,
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
    pub level: Vec<f32>,
    pub wedge: Vec<f32>,
    pub vwap: Vec<f32>,
    pub internal: f32,
    pub external: f32,
    pub cash: f32,
}

impl Polity {
    pub fn span(&self) -> std::ops::Range<usize> {
        self.seat..self.seat + 2 * UNITS
    }
}

/// 一种商品在某一轮的全场状态：申报、报价、成交、库存、投入产出
///
/// 「报价」是双方挂出来的价，「指数」是成交的加权平均——两者不是一回事：
/// 指数只在有成交的那一轮才动，所以申报（尤其是一侧没有对手盘的申报）不会推动任何东西。
#[derive(Clone, Copy, Default)]
pub struct GoodState {
    /// 银河指数，只有发生成交才会被改写
    pub price: f32,
    /// 本轮成交价（按成交量加权），没有成交记 0
    pub deal_price: f32,
    /// 卖侧申报量与量加权报价
    pub declared_sell: f32,
    pub quote_sell: f32,
    /// 买侧申报量与量加权报价
    pub declared_buy: f32,
    pub quote_buy: f32,
    /// 两侧的量加权报价尺度：>1 是加价，<1 是让价
    pub scale_sell: f32,
    pub scale_buy: f32,
    /// 各地方账本的边际买价/卖价的平均（**决策**用的就是这一对，逐地方取）
    pub bid: f32,
    pub ask: f32,
    /// 卖方/买方学习器**当前估的市场天花板** `share_max × depth_max` 的平均值
    pub sell_ceiling: f32,
    pub buy_ceiling: f32,
    /// 原始缺口为负（想买）的部门数
    pub wanted_buy: f32,
    /// 想买但被"买不起"清零的部门数
    pub blocked_buy: f32,
    /// 各仓库自适应目标的合计（诊断用）
    pub target: f32,
    /// 实际成交量
    pub dealt: f32,
    /// 全场库存合计
    pub stock: f32,
    /// 本轮想吃的量（计划投入）
    pub intake: f32,
    /// 本轮实际吃进的量（计划投入 × 执行率）
    pub consumed: f32,
    /// 本轮实际入账的产出
    pub delivery: f32,
}

pub struct Snapshot {
    pub round: usize,
    pub prices: Vec<f32>,
    pub wedges: Vec<Vec<f32>>,
    pub levels: Vec<Vec<f32>>,
    pub vwaps: Vec<Vec<f32>>,
    pub internal: f32,
    pub external: f32,
    pub uncleared: f32,
    pub treasury: f32,
    pub gauge: Vec<f32>,
    pub transform_share: f32,
    pub transform_potential: f32,
    pub local_ratios: Vec<Vec<f32>>,
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
    /// 消费结算**没解出来**的次数（逐部门逐轮计）。
    /// 存在的意义就是"不许静默继续"：数值出问题必须在读数里看得见。
    pub settlement_failures: usize,
    pub history: Vec<Snapshot>,
    rng: Rng,
}

impl Lab {
    pub fn new(spec: &Spec, seed: u64) -> Self {
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
                level: vec![BASE_PRICE; GOODS],
                wedge: vec![0.0; GOODS],
                vwap: vec![BASE_PRICE; GOODS],
                internal: 0.0,
                external: 0.0,
                cash: 0.0,
            });
        }
        let departments = Departments::new(departments).with_grants(vec![GRANT; count]);
        let mut lab = Self {
            departments,
            warehouses: Warehouses::new(warehouses).with_fluctuation(spec.fluctuation.max(0.0)),
            market,
            polities,
            rule: LevelRule::Counterparty,
            forgetting: 0.1,
            gain: 0.02,
            // **默认定规范。** `level = index × e^wedge` 在
            // `index → index·c, wedge → wedge − ln c` 下不变，所以楔子的**均值是
            // 一个规范自由度**：它不携带任何信息，只是把指数已经承载的水平又说了一遍。
            // 留着它自由，环路就会沿这个方向漂——实测三产楔子漂到 −35.344，把**所有**
            // 政体的三产水平一起压到指数的 4.5e-16，于是三产的买价变成 5.12e-17，
            // t2 毛利 = 16×5.12e-17 − 16×3.14e-2 < 0 ⇒ 停产 ⇒ execution 冻结 ⇒ 洪水无界。
            // 每轮跨政体中心化就把这个不可观测的自由度钉在 0 上，楔子只留相对信息。
            recenter: true,
            anchor: true,
            gauge: vec![0.0; GOODS],
            round: 0,
            settlement_failures: 0,
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

    /// 软成交容差，见 [`crate::market::Market::with_soft_eps`]
    pub fn with_soft_eps(mut self, eps: f32) -> Self {
        self.market = self.market.with_soft_eps(eps);
        self
    }

    /// 消费结算规则，见 [`crate::department::Rationing`]
    pub fn with_rationing(mut self, rationing: Rationing) -> Self {
        self.departments.rationing = rationing;
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
        let mut states = vec![GoodState::default(); goods];
        for (k, state) in states.iter_mut().enumerate() {
            state.price = self.market.merchandises[k].price;
            // 账本：逐地方取边际买卖价，这里报的是各地方的平均（对称场景下就是那一个值）
            let mut formed = 0.0;
            for row in &self.warehouses.books {
                let Some(book) = row.get(k) else { continue };
                if !book.is_formed() {
                    continue;
                }
                state.bid += book.bid;
                state.ask += book.ask;
                formed += 1.0;
            }
            if formed > 0.0 {
                state.bid /= formed;
                state.ask /= formed;
            }
            if traders > 0 {
                state.sell_ceiling /= traders as f32;
                state.buy_ceiling /= traders as f32;
            }
            for i in 0..traders {
                let stock = &self.warehouses.warehouses[i].stocks[k];
                state.sell_ceiling += stock.sell_response().ceiling();
                state.buy_ceiling += stock.buy_response().ceiling();
                if stock.declared_gap < 0.0 {
                    state.wanted_buy += 1.0;
                }
                if stock.purchase_blocked {
                    state.blocked_buy += 1.0;
                }
                state.target += stock.target_volume;
                let merchandise = &self.market.traders[i].merchandises[k];
                let scale = self.warehouses.warehouses[i].stocks[k].marketing_price_scale();
                let volume = merchandise.volume;
                if volume > 0.0 {
                    state.declared_sell += volume;
                    state.quote_sell += volume * merchandise.price;
                    state.scale_sell += volume * scale;
                } else if volume < 0.0 {
                    let size = -volume;
                    state.declared_buy += size;
                    state.quote_buy += size * merchandise.price;
                    state.scale_buy += size * scale;
                }
            }
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
                let intake = department.intake().get(k).copied().unwrap_or(0.0);
                state.intake += intake;
                // `intake` 已经是**结算后的实际提货量**（逐政策执行率已经打进去了），
                // 不能再乘一次执行率。
                state.consumed += intake;
                state.delivery += department.delivery().get(k).copied().unwrap_or(0.0);
            }
            state.quote_sell = if state.declared_sell > 0.0 {
                state.quote_sell / state.declared_sell
            } else {
                0.0
            };
            state.quote_buy = if state.declared_buy > 0.0 {
                state.quote_buy / state.declared_buy
            } else {
                0.0
            };
            state.scale_sell = if state.declared_sell > 0.0 {
                state.scale_sell / state.declared_sell
            } else {
                0.0
            };
            state.scale_buy = if state.declared_buy > 0.0 {
                state.scale_buy / state.declared_buy
            } else {
                0.0
            };
        }
        states
    }

    /// 与 [`Self::spread`] 同形，但用的是**账本**而不是成交。
    ///
    /// 路线 b 之后这才是"本地价"的正身：账本中间价 ÷ 指数。`spread` 用的是已实现的
    /// 成交价除以指数——而指数现在本身就是账本的聚合，两个口径混在一起读出来的数
    /// 既不是"本地 vs 全局"，也不是"挂价 vs 成交"。账本口径不依赖成交，也不掺口径。
    pub fn book_spread(&self, polity: usize, good: usize) -> f32 {
        let level = |p: usize| -> f32 {
            let mid = self
                .warehouses
                .book(p, good)
                .map(|book| book.mid())
                .unwrap_or(0.0);
            let index = self
                .market
                .merchandises
                .get(good)
                .map(|merchandise| merchandise.price)
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
        for department in &self.departments.departments {
            let report = department.settlement();
            if !report.converged || report.degraded {
                self.settlement_failures += 1;
            }
        }
        // 指数改由**账本聚合**导出（账本先验、指数导出）。放在决策之后、锚之前：
        // 锚要钉的就是这个由账本推出来的量。
        let goods = self.market.merchandises.len();
        for (k, price) in self
            .warehouses
            .aggregate_index(goods)
            .into_iter()
            .enumerate()
        {
            if price.is_finite() && price > 0.0 {
                self.market.merchandises[k].price = price;
            }
        }
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
                // 参照价 = 银河指数 × e^楔子。
                //
                // 这里**不能**用"该政权自己账本的中间价"：挂价 = 参照价 × 尺度，尺度又由
                // 账本推出来，于是 账本 → 参照价 → 挂价 → 账本 是一个没有锚的乘法环，
                // 实测会把挂价推到 f32 边界（$10^{19}$）。本地信息改由**决策价**承载
                // （`plan` 读逐地方的 bid/ask），不再由参照价承载。
                let level = prices[good] * polity.wedge[good].exp();
                polity.level[good] = if level.is_finite() && level > 0.0 {
                    level
                } else {
                    prices[good]
                };
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
                // 旧代码这里还有 `clamp(-MAX_WEDGE, MAX_WEDGE)`（±2.0）。那是个假天花板：
                // §3.4 表里 `counterparty` 那一列的 −200% 就是钳位本身，不是学出来的楔子。
                // 去掉之后发散会真的发走出去，代价是那个地方的参照价可能变成非有限——
                // 这由 apply_levels 的有限性回退接住，而不是由一个常数掩盖。
            }
            polity.internal = internal[p];
            polity.external = external[p];
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
            .find(|policy| policy.is_production());
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
            local_ratios: (0..self.polities.len())
                .map(|locality| {
                    (0..GOODS)
                        .map(|good| self.warehouses.local_ratio(locality, good).unwrap_or(1.0))
                        .collect()
                })
                .collect(),
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
