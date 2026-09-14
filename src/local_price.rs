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
    /// 这些 `(政权, 单元)` 的**生产部门不发**免费一产政策。
    ///
    /// 免费一产是"人人可种地"那版的默认工艺（`primary_free`），但 `LADDER_CAPACITY`
    /// 只有 8：一个零投入、每篮只吃 1.0 产能的工艺会把窗口里所有产能都占掉，
    /// 于是"技术阶梯"的两条工艺只剩 ~2% 份额、换挡永远看不见（§21）。
    /// 要研究阶梯本身，就得把那条免费工艺从**这个**部门拿掉（其余部门照旧）。
    pub no_primary: Vec<(usize, usize)>,
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
            no_primary: Vec::new(),
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

    /// 同一个部门挂两个工艺：省料但慢、费料但快。产能受限时的切换点是
    /// 工业品价 ÷ 粮食价 = **1.104**（推导印在 `--scenario ladder` 的表头上；实测见 §21）
    pub fn ladder(polities: usize, food_supply: f32) -> Self {
        let mut spec = Self::scarce(polities, 0, 0);
        spec.capacity = LADDER_CAPACITY;
        // 只留阶梯的两条工艺：否则零投入的免费一产会把产能全占掉，阶梯份额掉到 2%。
        spec.no_primary.push((0, 0));
        // 粮食供给的杠杆放在**别的政权**的免费一产上。阶梯部门自己没有免费工艺，
        // 于是杠杆只动**价格**，不动这门技术本身的产出（`with_specialty` 那种杠杆会
        // 直接改产出，测的就不是价格反应了）。
        for other in 0..polities {
            if other != 0 {
                spec.supply[other][0] = food_supply;
            }
        }
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
    pub anchor: bool,
    pub gauge: Vec<f32>,
    pub round: usize,
    /// 消费结算**没解出来**的次数（逐部门逐轮计）。
    /// 存在的意义就是"不许静默继续"：数值出问题必须在读数里看得见。
    pub settlement_failures: usize,
    pub history: Vec<Snapshot>,
    rng: Rng,
}

/// 逐政权逐商品的**本地价读数**：该政权地方账本中间价
/// （由学习曲线产出的**绝对报价**聚合出来的几何平均）。
fn local_readout(warehouses: &Warehouses, locality: usize, goods: usize) -> Vec<f32> {
    (0..goods)
        .map(|k| {
            warehouses
                .books
                .get(locality)
                .and_then(|row| row.get(k))
                .filter(|book| book.is_formed())
                .map(|book| book.mid())
                .filter(|mid| mid.is_finite() && *mid > 0.0)
                .unwrap_or(0.0)
        })
        .collect()
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
                            let bare = spec.no_primary.contains(&(polity, unit));
                            if !bare && (spec.primary_free || unit == 0) {
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
            anchor: true,
            gauge: vec![0.0; GOODS],
            round: 0,
            settlement_failures: 0,
            history: Vec::new(),
            rng: Rng::with_seed(seed),
        };
        let snapshot = lab.snapshot();
        lab.history.push(snapshot);
        lab
    }

    /// 仓库内部的学习率：响应曲面、价格曲线、账本记忆。
    ///
    /// §19.5 点名"仓库内部那三个写死的学习率"是唯一没扫过的旋钮；实测（多种子、
    /// 5000 轮）只有**价格曲线**是承重的：冻结它整场就稳，只冻 `Response` 照样炸。
    pub fn with_learning_rates(mut self, response: f32, price: f32, fixed_slope: bool) -> Self {
        self.warehouses = self.warehouses.with_learning_rates(response, price, fixed_slope);
        self
    }

    /// 账本/本地比值的记忆（旧 `LOCAL_PRICE_FORGETTING` 常量）
    pub fn with_book_forgetting(mut self, forgetting: f32) -> Self {
        self.warehouses = self.warehouses.with_book_forgetting(forgetting);
        self
    }

    /// 报价搜索的对数半宽（中心 = 上一轮自己的成交价）。默认 = `LOG_LIMIT`，
    /// 即全 f32 范围（历史行为）。见 `docs/market-system.md` §7.3。
    pub fn with_quote_band(mut self, band: f32) -> Self {
        self.warehouses = self.warehouses.with_quote_band(band);
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
                let scale = self.warehouses.warehouses[i].stocks[k].marketing_price();
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
        // 本地价读数：逐政权 = 该政权地方账本中间价（由学习曲线产出的**绝对报价**聚合而来）。
        // 这里不再有"独立学一个水平"的状态：那条楔子/`apply_levels` 的路子已整条删掉（§20.7–§20.13）。
        let goods = self.market.merchandises.len();
        for (p, polity) in self.polities.iter_mut().enumerate() {
            let readout = local_readout(&self.warehouses, p, goods);
            for k in 0..goods {
                if readout[k] > 0.0 {
                    polity.level[k] = readout[k];
                }
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
        self.update_diagnostics();
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

    /// 每轮的**诊断读数**：逐政权的成交均价（`vwap`）、内部/跨境成交量、现金。
    ///
    /// 这里**没有任何价格水平状态**。曾经那条「独立学一个楔子、再把指数乘回去」的路子
    /// 已经整条退场（§20.7–§20.13）：价格水平由学习曲线产出的**绝对报价**承载，本函数
    /// 只把已经发生的事记成读数，不写回任何决策路径。
    fn update_diagnostics(&mut self) {
        let Self {
            polities,
            warehouses,
            market,
            ..
        } = self;
        let mut observations = vec![vec![(0.0f32, 0.0f32); GOODS]; polities.len()];
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
                let entry = &mut observations[polity][k];
                entry.0 += volume;
                entry.1 += volume * deal;
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
                let (volume, value) = observations[p][k];
                if volume > 0.0 {
                    polity.vwap[k] = value / volume;
                }
            }
            polity.internal = internal[p];
            polity.external = external[p];
            polity.cash = warehouses.warehouses[polity.span()]
                .iter()
                .map(|warehouse| warehouse.currency)
                .sum();
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
