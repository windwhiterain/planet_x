//! **集货派单**：自动控制怎么给运输舰排路线、又该让几艘舰去跑运输。
//!
//! 三件事，都是**纯函数**（只读 `State`，不掷骰、不写状态）：
//!
//! 1. [`needed_freighters`] / [`freighter_quota`]：本势力**该有多少艘运输舰**——一处有
//!    积压的货栈配一条船（需求），再乘上**思潮倾向**（[`freight_lean`]：军国少投、
//!    和平/殖民多投）；
//! 2. [`should_be_role`]：**这艘舰**是不是该跑运输——按「目标头数 − 现状头数」
//!    这个**缺口抽签**（概率分布 = 想要的比例 ⇒ 期望入伙数 = 缺口），与遍历顺序无关；
//! 3. [`route_for`]：这艘舰**该跑哪条线**——`from` 按**积压占比抽签**、`to` 永远是首都。
//!
//! # 为什么「选哪处积压」用抽签而不是贪心
//!
//! 用户裁决：**每艘船去哪个积压点的伪随机概率分布 = 积压占比**。这条规则一条话就说完，
//! 却自带三个好处：
//!
//! * **分配自动按积压成比例**：积压 100 的货栈期望分到 10 倍于积压 10 的那处的运力，
//!   不必维护「已派了几条船」的账（那本账还得考虑船在路上、船被击沉、船改行……）；
//! * **不需要协商**：每艘舰自己掷一次骰子就走，天然无中心、无顺序依赖；
//! * **仍然完全确定**：骰子是 `(势力, 舰名, 回合, "route")` 派生出来的（[`sim::derived_roll`]），
//!   同种子同回合逐字复现，且**不消费主 `Prng` 流**（否则「多派一艘船」会改变整个世界
//!   后续的掷骰，同种子可复现就退化成了「舰队数量一变后面全变」）。
//!
//! 抽签只在**需要选一条新线**时掷：`route_for` 优先续用现有路线（货栈还有货、或舱里载着货），
//! 所以船不会每回合在几处积压之间反复改道——「一票货永远运不回家」的抖动没有落点。
//!
//! # 与「角色」的分工
//!
//! 这一层**只给运输舰排线**。谁是运输舰是第三条风格轴
//! （[`State::ship_role`](crate::model::State::ship_role)）说了算，
//! 而「该有几艘」是这里的 [`needed_freighters`]——自动控制把结论写回那片叶
//! （`Player` 的叶不碰），于是玩家能覆写、AI 也不必每回合重新发明结论。
//!
//! 那片叶同时是**现状的记忆**：角色判据读的正是它（上一回合的结论），所以算的是
//! 「**还缺/超了几条腿**」而不是「我是谁」——于是头数正好等于目标时**谁都不动**，
//! 不会出现「每回合重掷身份 ⇒ 路线反复作废」。

use crate::model::*;
use crate::sim;
use std::collections::{BTreeMap, BTreeSet};

/// 一批货栈存货的**总件数**（不折算价值：「把东西搬回来」与「值多少钱」是两件事，
/// 后者交给市场）。
fn units_of(map: &ResourceMap) -> f64 {
    map.values().sum()
}

/// `a − b` 的**逐资源正部**：每种货各自 `max(0, ·)`（不是整表相减）。
fn positive_part(a: &ResourceMap, b: &ResourceMap) -> ResourceMap {
    let mut out = ResourceMap::new();
    for (rt, av) in a {
        let d = *av - b.get(rt).copied().unwrap_or(0.0);
        if d > 1e-9 {
            out.insert(rt.clone(), d);
        }
    }
    out
}

// --- 站点手上的货：建楼要花什么、还缺什么、剩下多少能运走（单向运输 → 双向）-------------
//
// 用户裁决「**完全禁止瞬移**」：非首都天体上的建造（建楼 / 造舰 / 装模块）只能花**那处
// 货栈里的货**（首都天体照旧花池子）。于是每一处站点同时是两条腿的两端：
//
// * **出口**：本地现货**扣掉自己建设要用的**，剩下的才等船运回首都（净额，见下）；
// * **进口**：自己建设要用的**减去本地现货**，缺口从首都运过来。
//
// 两条腿的货量都从**同一把尺子**（[`site_build_need`]）与**同一处库存**
//（[`State::stock_at`]）算出来 —— 与 `sim::build_city` 实际花钱的方式同源，不另编估算。

/// 一处站点（某势力在**某天体**上的城）**计划要干的活**还差哪些料（逐资源）。
///
/// 三项相加，全部是「**计划**」而不是「速率」——需求必须是**存量**才有终点（速率永远开着）：
///
/// 1. **在建的建筑**：`(计划面积 − 已建成面积) × 单位面积成本`，单位成本走
///    [`sim::per_area_cost`]（与 `build_city` 同一条算术，含 `construction_resource_mod`
///    与结构 `cost_mult`）；
/// 2. **建造区要下的下一艘舰的船体料**：`build_cost × (1 − 进度 ÷ build_points)`
///    ——进度已经攒了一部分，剩下的料还得运过来；
/// 3. **那艘舰的最低可用选装**（[`crate::autocontrol::minimum_loadout`]：最便宜的武器 +
///    最便宜的推进）——少了它，船坞下水不了一艘能动的船。
///
/// ⚠ 第 2、3 项是**实测补上的**：只算第 1 项时，一个「楼全建完了、只剩船坞在干活」的天体
/// 需求恒为 0 ⇒ 一条补给腿都不会开 ⇒ 船坞干等本地那点矿 ⇒ **全世界 0 舰**（seed 7 / 200 回合
/// 实测）。建筑、船体、模块是同一件事的三段：**这座城市要花的实物都在这个天体上**。
///
/// 它是**需求的存量**（一个计划还差多少），与「积压」同量纲——于是进出口两条腿的抽签
/// （概率 = 量占比）在两边的尺度一致，不需要另一套「需求怎么估」的模型。
pub fn site_build_need(state: &State, config: &GameConfig, fid: &str, body: &str) -> ResourceMap {
    let mut out = ResourceMap::new();
    let loadout = super::minimum_loadout(config);
    for c in state
        .cities
        .iter()
        .filter(|c| c.faction_id == fid && c.body_id == body && !c.razed)
    {
        let res_mod = state
            .city_settlement(&c.name)
            .map(|s| s.construction_resource_mod)
            .unwrap_or(1.0);
        for b in &c.buildings {
            let spec = config.building_spec(&b.kind);
            if b.under_construction() {
                let per_area = sim::per_area_cost(config, spec, res_mod, b);
                let left = b.area - b.deployed;
                for (rt, cost) in per_area {
                    *out.entry(rt).or_insert(0.0) += cost * left;
                }
            }
            // 建造区：这一艘还没下水的舰要的料（船体余量 + 一套最低可用选装）。
            if !b.is_shipyard() {
                continue;
            }
            let Some(cls) = b.ship_type.as_deref() else {
                continue;
            };
            let Some(ship) = config.ships.get(cls) else {
                continue;
            };
            let bp = ship.build_points.max(1e-9);
            let progress = c.ship_progress.get(cls).copied().unwrap_or(0.0);
            let left = (1.0 - progress / bp).clamp(0.0, 1.0);
            for (rt, cost) in &ship.build_cost {
                *out.entry(rt.clone()).or_insert(0.0) += cost * left;
            }
            for (rt, cost) in &loadout {
                *out.entry(rt.clone()).or_insert(0.0) += cost;
            }
        }
    }
    out
}

/// 一处站点**每回合要吃多少料**（逐资源）——**产能速率 × 单位成本**，与 `build_city`
/// 那两处 `max_affordable_inc(...)` 的**速率上限**同一把尺子：
///
/// * **在建建筑**：`min(剩余面积, construction_speed × speed_mod × productivity × labor)
///   × 单位面积成本`；
/// * **建造区**：`min(船体余量, rate ÷ build_points) × 船体成本`，其中
///   `rate = deployed × productivity × labor` 就是 `build_city` 里那个 `class_rate`。
///
/// 为什么需要它：站点要留多少料**不是「它还欠多少活」**（那是计划的总量，永远很大 ⇒ 什么
/// 都不往外运），而是「**下一班船来之前它会烧掉多少**」。实测：按计划总量当保留量时，
/// 站点把料全留下、首都永远收不到货，而首都自己也要料（seed 7 / r150：星系矿业@地球的船坞
/// 只差 **铁 3.8** 就卡了 50 回合，全世界的船坞相继停产、最终 **0 舰**）。
pub fn site_burn(state: &State, config: &GameConfig, fid: &str, body: &str) -> ResourceMap {
    let mut out = ResourceMap::new();
    let yard_spec = config.building_spec("construction");
    for c in state
        .cities
        .iter()
        .filter(|c| c.faction_id == fid && c.body_id == body && !c.razed)
    {
        let speed_mod = state
            .city_settlement(&c.name)
            .map(|s| s.construction_speed_mod)
            .unwrap_or(1.0);
        let res_mod = state
            .city_settlement(&c.name)
            .map(|s| s.construction_resource_mod)
            .unwrap_or(1.0);
        let labor = sim::labor_ratio(state, config, &c.name);
        for b in &c.buildings {
            let spec = config.building_spec(&b.kind);
            if b.under_construction() {
                let speed = spec.construction_speed * speed_mod * spec.productivity * labor;
                let inc = (b.area - b.deployed).min(speed);
                for (rt, cost) in sim::per_area_cost(config, spec, res_mod, b) {
                    *out.entry(rt).or_insert(0.0) += cost * inc;
                }
            }
            if !b.is_shipyard() {
                continue;
            }
            let Some(cls) = b.ship_type.as_deref() else {
                continue;
            };
            let Some(ship) = config.ships.get(cls) else {
                continue;
            };
            let bp = ship.build_points.max(1e-9);
            let progress = c.ship_progress.get(cls).copied().unwrap_or(0.0);
            let left = (1.0 - progress / bp).clamp(0.0, 1.0);
            let rate = b.deployed * yard_spec.productivity * labor; // = `class_rate`
            let frac = (rate / bp).min(left);
            for (rt, cost) in &ship.build_cost {
                *out.entry(rt.clone()).or_insert(0.0) += cost * frac;
            }
        }
    }
    out
}

/// 一处站点**要常备的一次性料**：每个建造区一套**最低可用选装**
/// （[`crate::autocontrol::minimum_loadout`]）。
///
/// 它是「下水那一回合要一次性付掉」的钱，不属于「每回合烧掉多少」，所以不进
/// [`site_burn`] 而单独常备——一个船坞手里**始终**该有装得出一艘能动的船的模块料。
pub fn site_standing(state: &State, config: &GameConfig, fid: &str, body: &str) -> ResourceMap {
    let mut out = ResourceMap::new();
    let loadout = super::minimum_loadout(config);
    let yards = state
        .cities
        .iter()
        .filter(|c| c.faction_id == fid && c.body_id == body && !c.razed)
        .flat_map(|c| c.buildings.iter())
        .filter(|b| b.is_shipyard() && b.ship_type.is_some())
        .count();
    for (rt, cost) in &loadout {
        out.insert(rt.clone(), cost * yards as f64);
    }
    out
}

/// 一处站点**本回合该留多少料**（lead-time 库存）——进出口两侧**共用这一个量**。
///
/// ```text
/// 保留量 = min(计划总量, 每回合消耗速率 × 这条线一个往返的回合数) + 常备（最低选装）
/// ```
///
/// * **速率 × 往返回合数** = 「下一班船到达之前我会烧掉多少」，这是库存论里最朴素的那条
///   口径，而这里的「一个往返」不是新编的数——它就是全作**唯一**的那把尺子
///   [`lane_rounds`](crate::model::lane_rounds)（合同市场的考核周期、要求运力都用它）。
///   深处的站点往返几十回合 ⇒ 自然要多囤一点；近地两个回合 ⇒ 几乎不囤。
/// * **上限是计划总量**：楼快建完时不必囤满一整套。
/// * **常备量加在外面**：下水是一锤子买卖，不该被「这回合烧多少」抹掉。
///
/// 于是**同一件货不可能同时出现在两条腿上**：存量超过保留量的那部分才是出口，低于保留量的
/// 那部分是缺口——两条腿互为镜像，中间没有缝（否则刚送到的货下一回合就被原路运回去）。
pub fn site_reserve(state: &State, config: &GameConfig, fid: &str, body: &str) -> ResourceMap {
    let cap = state.capital_body(fid);
    let rounds = crate::model::lane_rounds(state, config, body, &cap).max(1.0);
    let plan = site_build_need(state, config, fid, body);
    let burn = site_burn(state, config, fid, body);
    let standing = site_standing(state, config, fid, body);
    let mut out = ResourceMap::new();
    for (rt, want) in &plan {
        let lead = burn.get(rt).copied().unwrap_or(0.0) * rounds;
        out.insert(rt.clone(), want.min(lead));
    }
    for (rt, extra) in &standing {
        *out.entry(rt.clone()).or_insert(0.0) += extra;
    }
    out
}

/// 一处站点**还缺**的资源 = `本回合该留的量 − 本地现货`（逐资源正部）。
///
/// 这就是**补给腿**的需求信号：首都派船去那儿，就是去补这个缺口。
pub fn site_deficit(state: &State, config: &GameConfig, fid: &str, body: &str) -> ResourceMap {
    let need = site_reserve(state, config, fid, body);
    let stock = state.stock_at(fid, body).cloned().unwrap_or_default();
    positive_part(&need, &stock)
}

/// 一处站点**能运走的净剩余** = `本地现货 − 本回合该留的量`（逐资源正部）。
///
/// **为什么是净额而不是全部现货**（实测逼出来的口径）：货栈现在**一份库存两用**（自己建设
/// 花 + 等船运走）。若出口按「全部现货」算，刚给殖民地送上门的建材下一回合就会被集货签抽中
/// **原路运回首都**——两条腿自己和自己打架。扣掉本地该留的量之后，同一件货**不可能同时**
/// 出现在出口和进口两侧：站点还缺的留在原地，站点用不完的才往外运。
pub fn exportable_at(state: &State, config: &GameConfig, fid: &str, body: &str) -> ResourceMap {
    let need = site_reserve(state, config, fid, body);
    let stock = state.stock_at(fid, body).cloned().unwrap_or_default();
    positive_part(&stock, &need)
}

/// 一条**运输腿**：从 `from` 搬到 `to`，`units` = 此刻这条腿上**有多少货要动**
/// （出口腿 = 净剩余，进口腿 = 缺口与首都现货的交集）。抽签按 `units` 占比 ⇒ 期望运力
/// 自动按「哪儿的货多」成比例。
#[derive(Clone, Debug, PartialEq)]
pub struct Lane {
    pub from: BodyId,
    pub to: BodyId,
    pub units: f64,
}

/// 本势力此刻**要动的货**（两个方向合成一张表，按 `(from, to)` 天体名序 ⇒ 确定性）。
///
/// * **出口腿** `站点 → 首都`：该处的**净剩余**（[`exportable_at`]）> 0；
/// * **进口腿** `首都 → 站点`：该处的**缺口**（[`site_deficit`]）与**首都池现货**的交集 > 0
///   ——池子里没有的货，派船去也是空跑（「有货才派」与出口侧「有积压才派」是同一条纪律）。
///
/// 两端的量都是**存量**（件），所以两边的抽签权重同量纲、可以直接混在一张表里按占比抽。
pub fn lanes(state: &State, config: &GameConfig, fid: &str) -> Vec<Lane> {
    let cap = state.capital_body(fid);
    if cap.is_empty() || state.body(&cap).is_none() {
        return Vec::new();
    }
    let hub: ResourceMap = state.stock_at(fid, &cap).cloned().unwrap_or_default();
    let mut out: BTreeMap<(BodyId, BodyId), f64> = BTreeMap::new();
    // 出口：有货栈的地方（货栈按 `(势力, 天体)` 存，所以键集合就是「哪里攒着货」）。
    let bodies: Vec<BodyId> = state
        .depots
        .keys()
        .filter(|(f, b)| f == fid && *b != cap)
        .map(|(_, b)| b.clone())
        .collect();
    // 进口：有活城的地方（缺口不需要货栈已经存在——缺口正是「这里什么都没有」）。
    let mut sites: BTreeSet<BodyId> = bodies.iter().cloned().collect();
    sites.extend(
        state
            .cities
            .iter()
            .filter(|c| c.faction_id == fid && !c.razed && c.body_id != cap)
            .map(|c| c.body_id.clone()),
    );
    for b in &bodies {
        let out_units = units_of(&exportable_at(state, config, fid, b));
        if out_units > 1e-9 {
            *out.entry((b.clone(), cap.clone())).or_insert(0.0) += out_units;
        }
    }
    for b in &sites {
        let deficit = site_deficit(state, config, fid, b);
        // 只能运**首都真的有的**那部分：逐资源取交集（缺口 30 硅、池子 5 硅 ⇒ 这条腿此刻 5 件）。
        let movable: f64 = deficit
            .iter()
            .map(|(rt, want)| want.min(hub.get(rt).copied().unwrap_or(0.0)))
            .sum();
        if movable > 1e-9 {
            *out.entry((cap.clone(), b.clone())).or_insert(0.0) += movable;
        }
    }
    out.into_iter()
        .map(|((from, to), units)| Lane { from, to, units })
        .collect()
}

/// 本势力**该有多少艘运输舰**（**连续量**）：每条腿按**它有多少活**分到船的头数，
/// **一条腿最多算一艘**（一条线同时只跑一趟）。
///
/// ⚠ 这是**头数定编口径，不含航程**：远距大积压腿也最多算一艘。实际能搬多少要走
/// [`trip_throughput`] / 合同市场的运力账；这里只回答「该派几条船过去」。
///
/// ```text
/// 头数 = Σ_腿 min(1, 这条腿的货量 ÷ 一个货舱)
/// ```
///
/// **为什么不能「一条腿 = 一艘船」**（量出来的修正）：改成两条腿之后，**每个建造区**都会
/// 常备一套最低选装（[`site_standing`]），于是一个 12 城的势力平白多出十几条**涓流腿**
/// （每条一两件货），而按「一条腿配一条船」它们会**把整支舰队吃光**——实测 r1 就有 11/21
/// 艘舰被定成运输舰，战争舰队被抽空 ⇒ 更容易被打光 ⇒ 更造不出船。
/// 按活量折算之后，涓流腿各分到几分之一艘，**期望上仍然自动等于「谁活多谁多拿」**
/// （与抽签那条纪律同源：概率分布 = 想要的比例），只是不再让一条 1 件的腿独占一艘船。
///
/// （旧口径「一条腿配一艘船」（`stocked_depots().len()`）就是被这条实测顶掉的；
/// 「积压清空 ⇒ 那一份名额自然收回」这条性质不变，见 [`should_be_role`]。）
pub fn needed_haulers(state: &State, config: &GameConfig, fid: &str) -> f64 {
    let hold = config.freight.nominal_hold.max(1e-9);
    lanes(state, config, fid)
        .iter()
        .map(|l| (l.units / hold).min(1.0))
        .sum()
}

/// [`needed_haulers`] 取整（读面与守卫要一个「几条船」的整数）。
pub fn needed_freighters(state: &State, config: &GameConfig, fid: &str) -> usize {
    needed_haulers(state, config, fid).ceil() as usize
}

/// 一艘舰的**集货运力**（定编的排序键）：`有效舱容 × 巡航速度 ÷ 维护费`。
///
/// * **舱容**只说明「一趟能装多少」；单位时间的运力还要乘**速度**——航程一定时，跑得快就是
///   跑得勤（来回时间 ≈ `2 × 航程 ÷ 巡航速度`）。只看舱容会把「装得多但慢」的船排错，
///   而在本作里速度**完全来自推进模块**：*没有推进模块的船速度是 0*，派它去运货等于派一尊
///   雕像——所以 0 速的舰**根本不该出现在运力名单里**（本函数返回 0，调用处据此剔除）。
/// * 两项都取**有效值**：舱容乘战损折算（[`cargo_capacity`]），速度取 [`ship_panel`] 的巡航速度
///   ——推进模块的**完整度**已经折在里面了。于是打残的船自动让位给完好的船。
/// * 除以**维护费**：运货的成本是养船，同样的钱能搬多少货才是舰级的运输效率；
///   这也顺手把「主力舰（战列）别去拉货」变成排序的自然结果，而不是一条特判。
pub fn freight_tonnage(config: &GameConfig, ship: &Ship) -> f64 {
    let panel = ship_panel(config, ship);
    if panel.speed <= 0.0 {
        return 0.0; // 动不了 ⇒ 运力是**零**，不是「很小」。
    }
    cargo_capacity(config, ship) * panel.speed / panel.upkeep.max(1e-6)
}

// --- 思潮 → 角色（用户裁决：「**由国家思潮决定自动控制下舰船倾向于运输还是战斗**」）----
//
// 「谁是运输舰」从**硬定编**（按运力排名取前 N、一刀切）改成**思潮驱动的概率**：
// 需求（[`needed_freighters`]：一条腿 = 一条船）仍然说**要多少条腿**，
// 思潮说**本势力愿意投多少条腿**。
//
// 系数**写死在这里、不进 config**（用户裁决：「不要配置了，直接耦合思潮写死」）：
// 改行为就改下面这几个带量纲注释的常数。

/// 思潮轴权重（**和平端 −1 ⟷ 军国端 +1**）：军国 ⇒ 尚武 ⇒ **少**跑运输。
const LEAN_MILITARY: f64 = 1.0;
/// 思潮轴权重（**自然端 −1 ⟷ 殖民端 +1**）：殖民 ⇒ 要给远方殖民地送补给 ⇒ **多**跑运输
/// ⇒ 对「尚武度」是**负**贡献。
///
/// 两轴**同权反号**（用户裁决：就这两轴），于是「既军国又殖民」的势力两股力量互相抵消
/// （`+1, +1 ⇒ 0`）——扩张既要打仗也要补给，它们撞在同一个标量上，不是巧合。
const LEAN_COLONY: f64 = -1.0;
/// 尚武度 → 头数倍数的斜率。**中庸（尚武度 0）⇒ 倍数正好 1.0** = 旧硬定编的行为，
/// 于是这条改动在世界的中位上**行为中性**：思潮只负责把它往两边推。
const LEAN_GAIN: f64 = 1.5;
/// **岗位轮换率**（用户裁决：「运输/战斗是**动态调整**的，而非固定」）：即使头数正好等于
/// 配额，也按 `它 × 现状头数` 的期望换手——**入伙与退伍两侧的期望相等**，所以**头数不动、
/// 换的只是「谁来干」**。岗位平均任期 ≈ `1 ÷ 它`（0.05 ⇒ 约 20 个回合，够跑几趟来回）。
///
/// 没有它，配额处两侧概率都恰好是 0 ⇒ 谁去运货**一次定终身**（那是「固定」而不是「动态」）。
const ROLE_ROTATION: f64 = 0.05;
/// **效率票的温度**：一张票 = `e^(效率加成 ÷ 它)`（见 [`should_be_role`] 的抽签）。
/// 越小越接近「只让最好的船去运」（断崖就在那个极限里），越大越是「谁去都行」。
/// 取 0.5 时最好的船与最差的船票数之比 = `e^(ROLE_EFF_GAIN ÷ 0.5)` ≈ 20 倍。
const ROLE_WIDTH: f64 = 0.5;
/// **运力效率偏好**：票数按 `运力 ÷ 队内最大运力 − 1 ∈ [−1, 0]` 加成。
///
/// 它是旧「按运力排名取前 N」的**软版本**：最好的船加成 0、最差的 `−ROLE_EFF_GAIN`，
/// 两者入伙概率之比 = `e^(ROLE_EFF_GAIN ÷ ROLE_WIDTH)` ≈ 20 倍——**偏好很硬、但没有断崖**
///（遵 `AGENTS.md`：不设进不去的目标——真没人运货时，战列舰照样会去跑）。
const ROLE_EFF_GAIN: f64 = 1.5;

// --- 三个角色怎么瓜分一支舰队（用户裁决：不许加阈值，要自然）---------------------------
//
// 角色轴上有三支力量在抢同一批船，各自有一个**主张**（头数，连续量）：
//   战舰：`威胁`（被强敌压的程度）—— 压得越狠越要多留人打仗；
//   运输：`积压 × 思潮倾向`（[`freighter_quota`]）—— 货堆得越多越想派人去搬；
//   观测：`离学满的缺口 × 思潮倾向`（`knowledge::observe_claim`）—— 想学的人才会派人去蹲。
//
// 配给规则是**水位**（water-filling），**没有任何角色上限**：
//   1. 战舰那一份先按威胁定：`war_share = WAR_BASE + WAR_THREAT_GAIN × threat_motive`，
//      剩下的 `预算 = 舰队 × (1 − war_share)` 留给运输与观测；
//   2. 两支主张都装得进预算 ⇒ **各得其所**（想要多少给多少，剩下的船留在战位上）；
//   3. 加起来超了预算 ⇒ **按主张的相对大小成比例缩水**（谁的主张大谁少挨刀）。
//
// 为什么不是「每个角色一条上限」（第一版给观测写死「最多占一半」，用户当场否掉：
// 「加硬阈值只能说明动机设计的不够好，把资源堆积的运输动机和战争威胁动机覆盖了，
// 不能加阈值要自然」）：上限会**越过**另外两个动机——积压堆成山、大军压境都压不动它，
// 因为那个数是写死的。水位配给里三支力量**互相挤压**：积压涨 ⇒ 运输的主张涨 ⇒ 观测分到的少；
// 威胁涨 ⇒ 战舰那一份涨 ⇒ 可分的余量小 ⇒ 运输与观测一起缩。这就是「自然」。
//
// 威胁读的是 [`super::shipbuilding::threat_motive`]——实测它**确实是情境量、不是常量**：
// 长局里当霸权的中国/俄罗斯 ≈ 0.01（没人威胁得了它），被压着打的星系矿业/无国界科学组织
// ≈ 0.8–0.9。
const WAR_BASE: f64 = 0.25;
/// 威胁 → 战舰份额的斜率。威胁 1.0 ⇒ `0.25 + 0.6 = 0.85`：**极端威胁下几乎全留作战舰，
/// 观测与运输一起被挤到边上**——那正是「要被打死了谁还去搞科研、谁还去搬货」。
const WAR_THREAT_GAIN: f64 = 0.6;

/// **三支力量抢舰队的结果**：`(战舰, 运输, 观测)` 的目标头数（连续量；差额留在战位上）。
///
/// 纯函数、只读 `State`（[`should_be_role`] 每艘舰都会调它，所以它**必须与调用顺序无关**）。
pub fn role_quotas(state: &State, config: &GameConfig, fid: &str) -> (f64, f64, f64) {
    let fleet = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid && s.hull > 0.0)
        .count() as f64;
    if fleet <= 0.0 {
        return (0.0, 0.0, 0.0);
    }
    let threat = super::shipbuilding::threat_motive(state, config, fid);
    let war_share = (WAR_BASE + WAR_THREAT_GAIN * threat).clamp(0.0, 1.0);
    let war = fleet * war_share;
    let budget = fleet - war;
    let freight = freighter_quota(state, config, fid);
    let observe = super::knowledge::observe_claim(state, config, fid);
    let claims = freight + observe;
    if claims <= 1e-9 {
        // 两支都没主张（没有积压、也学满了）⇒ 全军留在战位上。
        return (fleet, 0.0, 0.0);
    }
    if claims <= budget {
        // 装得下 ⇒ 各得其所；余下的船留在战位（没人主张就不该派活，而不是「补齐给谁」）。
        return (fleet - claims, freight, observe);
    }
    // 装不下 ⇒ 按相对主张成比例缩水。
    let scale = budget / claims;
    (war, freight * scale, observe * scale)
}

/// 本势力本回合的**观测配额**（= [`role_quotas`] 里那一支）——观察面与调用方用它。
pub fn observer_quota(state: &State, config: &GameConfig, fid: &str) -> f64 {
    role_quotas(state, config, fid).2
}

/// 本势力本回合的**运输配额**（水位配给**之后**的那一支）。注意与 [`freighter_quota`]
/// 那个**主张**不同：主张是「想派多少」，这里是「抢完舰队之后真的能派多少」。
pub fn freighter_quota_share(state: &State, config: &GameConfig, fid: &str) -> f64 {
    role_quotas(state, config, fid).1
}

/// 本势力此刻的**尚武度**（两轴加权和，权重见上面两个常数）。
fn martial(state: &State, fid: &str) -> f64 {
    let Some(f) = state.faction(fid) else {
        return 0.0;
    };
    LEAN_MILITARY * f.ideology.peace_military + LEAN_COLONY * f.ideology.nature_colony
}

/// 思潮 → **集货倾向**：本势力愿意投在集货上的**头数倍数**（`目标头数 = 需求 × 它`）。
///
/// `2σ(−LEAN_GAIN × 尚武度)`：中庸 ⇒ 1.0；军国 ⇒ **< 1**（宁可缺货、宁可雇人，也要把船留在
/// 战线上）；和平 / 殖民 ⇒ **> 1**（囤运力，多出来的船正好去做承运人）。
pub fn freight_lean(state: &State, fid: &str) -> f64 {
    2.0 * super::contract::sigmoid(-LEAN_GAIN * martial(state, fid))
}

/// 本回合的**目标头数**（连续量，不取整）：`需求 × 思潮倾向`。
///
/// 需求是 [`needed_freighters`]（一条腿 = 一条船）——**倾向乘在需求上**，
/// 所以「**没有货要动 ⇒ 目标 0 ⇒ 谁都不去跑运输**」这条不变量不会被思潮冲掉。
pub fn freighter_quota(state: &State, config: &GameConfig, fid: &str) -> f64 {
    needed_haulers(state, config, fid) * freight_lean(state, fid)
}

/// 本势力此刻**已经是运输舰**的舰数（不含 `except`）——抽签的**现状项**。
///
/// 取的是**有效角色**（叶 → 舰队默认 → 记录值）= **上一回合定下来的那个结论**，这就是迟滞
/// 的来源：船不是每回合从头掷「我是谁」，而是掷「**要不要换岗**」。玩家的钉子与舱里有货的舰
/// 都算进来——它们**确实在跑运输**，占着运力的名额。
fn hauler_headcount(state: &State, fid: &str, except: &str) -> f64 {
    state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid && s.hull > 0.0 && s.name != except)
        .filter(|s| state.ship_role(s.name.clone()) == ShipRole::Freight)
        .count() as f64
}

/// **这艘舰本回合的角色**（`true` = 运输舰）。**纯函数**：同一回合对同一艘舰多次调用结果
/// 一致（骰子由 `(势力, 舰名, 回合, "role")` 派生 ⇒ `assign_roles` 与更早的 `step_contracts`
/// 拿到同一个答案），且**不消费主 `Prng`**。
///
/// 四层，从硬到软：
/// 1. **硬承诺**：正在执行承包单的舰、舱里有货的舰 ⇒ **必须是**运输舰。角色一改，那条线就
///    再没人跑、那票货就烂在舱里（所以这两条压过抽签）；
/// 2. **玩家表态**：这条轴归属解析为 `Player` ⇒ 用玩家的值，AI 一个骰子都不掷；
/// 3. **物理**：运力为 0 的舰（没有推进模块 ⇒ 速度 0）**运不了货**——派它去等于派一尊雕像；
/// 4. **思潮驱动的配额 → 按票抽签**：目标头数 = `需求 × 思潮倾向`（[`freighter_quota`]），
///    而**每个候选舰的入伙概率 = 缺口 × 它的票 ÷ 同侧总票数**——与 [`route_for`] 的
///    「按积压占比抽签」是同一条纪律（**概率分布 = 想要的比例**），于是：
///    * **期望入伙数正好 = 缺口**，不多不少（不是「每人各掷一次身份」，那样缺口大时
///      全舰队会**一起**入伙、下一回合又一起退伍——实测 12 艘舰配额 4 时会在 0 与 12
///      之间两极震荡）；
///    * **头数钉在目标上、但人员是流动的**：目标处净变化为 0（判据里不出现被这个动作本身
///      改变的量），再叠一层**轮换**（[`ROLE_ROTATION`]）——走一个、来一个，头数不动而
///      「谁来干」每回合都在动（用户裁决：「运输/战斗是**动态调整**的，而非固定」）；
///    * **票**把旧的「按运力排名取前 N」变成软的：票 = `e^(效率加成 ÷ ROLE_WIDTH)`，
///      最好的船票最重（≈ 最差船的 20 倍）⇒ **偏好很硬、但没有断崖**
///      （`AGENTS.md`：不设进不去的目标——真没人运货时，战列舰照样会被抽中）；
///    * **退伍的票按低效率**：超额时先走的是运力最差的那些（于是名单会自己换成好船）。
///
/// 名单（**同侧总票数**的分母）只算**掷得动的船**：玩家钉住的、舱里有货的、正在执行承包单的
/// 舰都不在名单上——票不该投给动不了的人，否则期望入伙数会凭空少掉。
pub fn should_be_role(state: &State, config: &GameConfig, fid: &str, ship_id: &str) -> ShipRole {
    let roll = sim::derived_roll(fid, ship_id, state.round, "role");
    role_with_roll(state, config, fid, ship_id, roll).0
}

/// [`should_be_role`] 的**拍板**入口（B5）：同一套判据，但**把这次抽签记进输入面**。
///
/// ⚠ 只有这里会记账，`should_be_role`（估算路，例如「挂单时估我还有几条腿」）不记——
/// 同一枚骰子会被问两次，记的是**决定**（谁被定编成什么），不是「谁算过」。
pub(crate) fn decide_role(
    state: &State,
    config: &GameConfig,
    fid: &str,
    ship_id: &str,
    inputs: &mut RoundInputs,
) -> (ShipRole, RoleOdds) {
    let roll = sim::derived_roll(fid, ship_id, state.round, "role");
    let (role, odds) = role_with_roll(state, config, fid, ship_id, roll);
    // **记账只在这里**：闸门记「谁做了什么决定」（拍板那条路），估算那条路（`should_be_role`）
    // 不问就不记。用途与机会值都来自 `RoleOdds`，所以观测/运输两支共用一段。
    let fired = match odds.purpose {
        Some("observe_role") => odds.observe.map(|(p, _)| p),
        _ => odds.freight.map(|(p, _)| p),
    };
    if let (Some(purpose), Some(p)) = (odds.purpose, fired) {
        inputs
            .record_gate(
                purpose,
                fid,
                ship_id,
                roll,
                p,
                match role {
                    ShipRole::Freight => "freight",
                    ShipRole::War => "war",
                    ShipRole::Observe => "observe",
                },
            )
            // **候选池（B5c）**：同侧每艘候选舰各持多少票——「为什么是它被定编」就在这张表里。
            .pool = odds.pool.clone();
    }
    (role, odds)
}

/// 定编的**判据**（纯函数，吃骰子）：返回角色 + **这次抽签的机会值与候选池**。
///
/// 第二个值是 `Some((p, pool))` 当且仅当**这枚骰子真的被用到了**（判据是 `roll < p`）；
/// 早退的那几档（硬承诺 / 玩家表态 / 观测优先 / 运力为 0）返回 `None`——
/// 于是记账那边不必复制一遍早退逻辑（**判据只有一处**）。
/// 取某一支那份账的可变引用（`assign_roles` 记账用）。
fn share_mut<'a>(
    d: &'a mut crate::model::RoleDistribution,
    r: ShipRole,
) -> &'a mut crate::model::RoleShare {
    match r {
        ShipRole::War => &mut d.war,
        ShipRole::Freight => &mut d.freight,
        ShipRole::Observe => &mut d.observe,
    }
}

/// 一次定编抽签的**全部机会值**（给 `RoundInputs::role_distribution` 对账用）。
///
/// 定编是**级联**的：先观测（优先级 1）、再运输（优先级 2）、都没中 ⇒ 战舰。所以一艘舰
/// 落到三支的概率是 `p_obs` / `(1−p_obs)·p_frt` / `(1−p_obs)(1−p_frt)`——这里把**两个门槛**
/// 都交出来（而不是只交落中的那一支的 p，那会让"期望"少一项）。
#[derive(Clone, Debug, Default)]
pub(crate) struct RoleOdds {
    /// 观测那一支的 `(机会值, 引擎自己算的 flow)`（`None` = 那枚骰子没被用到）。
    /// **两枚骰子各带各的 `flow`**：同一艘舰会先后掷两枚，共用一个字段会被后一枚覆盖。
    pub observe: Option<(f64, f64)>,
    /// 运输那一支的 `(机会值, flow)`（`None` = 没走到）。
    pub freight: Option<(f64, f64)>,
    /// 命中的那一支的候选池（B5c 记账用）。
    pub pool: Vec<crate::model::PoolEntry>,
    /// 记账的用途名（`observe_role` / `role`）——只有真的掷了的档才记。
    pub purpose: Option<&'static str>,
    /// 不掷骰就定性的那一档（硬承诺 / 有货 / 玩家表态 / 动不了 / 没配额 / 没这条舰）。
    pub fixed: Option<&'static str>,
}

fn role_with_roll(
    state: &State,
    config: &GameConfig,
    fid: &str,
    ship_id: &str,
    roll: f64,
) -> (ShipRole, RoleOdds) {
    // **记账不在这里**：`decide_role`（拍板那条路）拿 `RoleOdds` 统一记；`should_be_role`
    // （估算那条路）不问就不记。判据仍然只有一处。
    let fixed = |why: &'static str| RoleOdds { fixed: Some(why), ..Default::default() };
    // 1) 硬承诺（见上）。
    if state.contracts.assignment_of(ship_id).is_some() {
        return (ShipRole::Freight, fixed("承包单硬承诺"));
    }
    let Some(ship) = state.ship(ship_id) else {
        return (ShipRole::War, fixed("没有这条舰"));
    };
    if !ship.cargo.is_empty() {
        return (ShipRole::Freight, fixed("舱里有货"));
    }
    // 2) 玩家表态：AI 不掷骰，直接用玩家的值（`Player` 的逐舰叶或舰队默认）。
    let role = state.ship_role(ship_id.to_string());
    if state.ship_role_control(ship_id.to_string()).is_player() {
        return (role, fixed("玩家表态"));
    }
    // 3) **三个动机抢舰队**（水位配给，见 [`role_quotas`]）：先算出本回合观测与运输各自的
    //    配额。观测**先挑**（优先级，见下一条），但**挑几条**由配给说了算——所以一处积压
    //    成山（运输主张大）或一支大军压境（战舰那一份大）都会真的把观测挤小。
    let (_, freighter_quota_share, observe_quota) = role_quotas(state, config, fid);
    // 4) **观测优先**（用户裁决：观测 > 运输 > 战斗）：观测是**唯一没有替代品**的角色——
    //    渠道空转就是零掌握度，而运输缺一条船还能雇人（承包市场就是干这个的）。选靶与抽签
    //    在 `autocontrol::knowledge`（与这里**同形**的缺口抽签）；它自己读 `state.ship_role`
    //    判断「我现在是不是观测舰」，所以入伙与退伍都在那一个函数里定。
    //
    //    **B5**：观测那一支的骰子由 `recorder` 决定记不记——拍板那条路（`decide_role`）
    //    传进来的是 `Some`，把这次抽签落进输入面；估算那条路传 `None`（同一枚骰子不记两遍）。
    let observe_roll = sim::derived_roll(fid, ship_id, state.round, "observe_role");
    let (observe, observe_p, observe_flow) = super::knowledge::observe_with_roll(
        state,
        config,
        fid,
        ship_id,
        observe_quota,
        observe_roll,
    );
    if observe {
        let (obs_p, _clamped, pool) = match observe_p {
            Some((p, pool)) => (
                Some(p),
                p >= 1.0 - 1e-12,
                pool.into_iter()
                    .map(|(n, w)| crate::model::PoolEntry { name: n, weight: w })
                    .collect(),
            ),
            None => (None, false, Vec::new()),
        };
        return (
            ShipRole::Observe,
            RoleOdds {
                observe: obs_p.map(|p| (p, observe_flow)),
                pool,
                purpose: Some("observe_role"),
                fixed: None,
                freight: None,
            },
        );
    }
    // 5) 当前角色不是运输舰 ⇒ 归零成「战舰」基线再掷运输的骰子。
    let cur = role == ShipRole::Freight;
    // 6) 物理：动不了的舰运不了货（不是阈值，是「没有推进模块就没有速度」）。
    if freight_tonnage(config, ship) <= 0.0 {
        return (ShipRole::War, fixed("动不了"));
    }
    // 7) 配额 → 抽签（用**水位配给之后**的那一支，不是主张）。
    let quota = freighter_quota_share;
    let others = hauler_headcount(state, fid, ship_id);
    let temp = ROLE_WIDTH.max(1e-9);
    // 运力效率加成（以**队内最大运力**为基准，尺度无关）：最好的船 = 0、最差的 = −gain。
    let best = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid && s.hull > 0.0)
        .map(|s| freight_tonnage(config, s))
        .fold(0.0f64, f64::max);
    let tonnage = |s: &Ship| freight_tonnage(config, s);
    // 一张票：**入伙**按高效率（最好的船票最重）、**退伍**按低效率（最差的船先走）。
    let ticket = |s: &Ship| -> f64 {
        let eff = if best > 0.0 {
            ROLE_EFF_GAIN * (tonnage(s) / best - 1.0)
        } else {
            0.0
        };
        if cur {
            (-eff / temp).exp()
        } else {
            (eff / temp).exp()
        }
    };
    let mut mine = 0.0;
    let mut tickets = 0.0;
    // **候选池（B5c）**：同侧每个候选舰各持多少票——「为什么是这艘被定编」= 票重 × 缺口。
    let mut pool: Vec<crate::model::PoolEntry> = Vec::new();
    for s in state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid && s.hull > 0.0)
    {
        // ⚠ **观测那一支是另一本账**：它**先挑**（优先级 观测 > 运输 > 战斗），挑走的船这一回合
        // 不再参与集货的抽签。不排掉它们的话，分母里会一直挂着「永远不加入」的观测舰，
        // 于是集货的期望入伙数被稀释、头数系统性低于配额（实测 4.30 的配额只跑到 3.65）。
        // 被观测那一支释放出来的船**下一回合**才回到这本账上——一轮的延迟，换一本干净的账。
        let s_role = state.ship_role(s.name.clone());
        if tonnage(s) <= 0.0 || s_role == ShipRole::Observe || (s_role == ShipRole::Freight) != cur
        {
            continue;
        }
        if s.name != ship_id {
            // 钉住的舰不在这张名单上（玩家表态 / 舱里有货 / 正在执行承包单）。
            if state.ship_role_control(s.name.clone()).is_player()
                || state.contracts.assignment_of(&s.name).is_some()
                || !s.cargo.is_empty()
            {
                continue;
            }
        }
        let t = ticket(s);
        tickets += t;
        pool.push(crate::model::PoolEntry {
            name: s.name.clone(),
            weight: t,
        });
        if s.name == ship_id {
            mine = t;
        }
    }
    let tickets = tickets.max(1e-9);
    // 缺口（我入伙时是「还缺几条腿」，我退伍时是「带上我超了几条腿」）——两者都由同一个
    // `others` 算出，所以这个动作**不改变判据本身**。
    let gap = if cur {
        (others + 1.0 - quota).max(0.0)
    } else {
        (quota - others).max(0.0)
    };
    // **轮换**：配额处也要换手（用户裁决：角色是动态调整的）。两侧都是 `ROLE_ROTATION × h`
    // ⇒ 期望「走的」与「来的」一样多 ⇒ **头数不动，换的只是谁来干**（效率票决定换谁：
    // 低效率的先走、高效率的先上）。
    let headcount = others + if cur { 1.0 } else { 0.0 };
    let flow = gap + ROLE_ROTATION * headcount;
    let p = (flow * mine / tickets).min(1.0);
    // 骰子**由调用方掷进来**（B5）：拍板那条路要把它记进输入面，估算那条路不必。
    let flip = roll < p;
    let stay = if cur { !flip } else { flip };
    let role = if stay {
        ShipRole::Freight
    } else {
        ShipRole::War
    };
    (
        role,
        RoleOdds {
            freight: Some((p, flow)),
            pool,
            purpose: Some("role"),
            fixed: None,
            // **观测那一支的机会值也要留着**：这艘舰没被观测挑走，但它**确实掷过**观测的骰子
            // （`observe_with_roll` 总是被调），所以落在"观测"那一支的概率就是 `p_obs`。
            // 只记落中的那一支（`rolls` 的闸门就是这么记的）时，`Σp` 会永远缺这一项。
            observe: observe_p.as_ref().map(|(p, _)| (*p, observe_flow)),
        },
    )
}

/// **本回合的定编**：把「谁是运输舰」一次性写进第三条风格轴
/// （[`State::ship_role`](crate::model::State::ship_role) 那片叶）。
///
/// 每回合跑一次，且**只看本回合开始时的状态**（在 `step_ships` 的逐舰循环**之前**调用）
/// ——逐舰现算会让结论依赖舰的处理顺序，而那个顺序是按 `rng` 打乱的。
///
/// 写叶有两道闸，缺一不可：
/// 1. **只写自动控制自己开的舰**（`ship_control == Auto`）：玩家开的舰一个字节都不碰；
/// 2. **归属链判定是 `Player` 就不写**：玩家在叶上或舰队默认上表过态 ⇒ 这条轴归玩家。
///
/// 值没变就不重写：控制面的 diff 是给人读的，把同一个值每回合重写一遍只会制造噪声。
pub(crate) fn assign_roles(
    state: &mut State,
    config: &GameConfig,
    inputs: &mut RoundInputs,
) {
    let mut fids: Vec<String> = state.factions.iter().map(|f| f.name.clone()).collect();
    fids.sort();
    for fid in fids {
        // 本回合**开始**时三支各有多少头（在任 + 硬承诺）——配给与两支抽签都以它为基准。
        let held = |r: ShipRole| -> f64 {
            state
                .ships
                .iter()
                .filter(|s| s.faction_id == fid && s.hull > 0.0 && state.ship_role(s.name.clone()) == r)
                .count() as f64
        };
        let (qw, qf, qo) = role_quotas(state, config, &fid);
        let mk = |q: f64, h: f64, rot: f64| {
            crate::model::RoleShare {
                quota: q,
                held: h,
                gap_join: (q - h).max(0.0),
                gap_leave: (h - q).max(0.0),
                rotation: rot,
                ..Default::default()
            }
        };
        let mut dist = crate::model::RoleDistribution {
            war: mk(qw, held(ShipRole::War), 0.0),
            freight: mk(qf, held(ShipRole::Freight), ROLE_ROTATION),
            observe: mk(qo, held(ShipRole::Observe), super::knowledge::OBSERVER_ROTATION),
        };
        // 先算完整个势力的名单再写：同一回合内几个势力的结论互不影响（也更好推理）。
        let mut plan: Vec<(String, ShipRole)> = Vec::new();
        for s in state
            .ships
            .iter()
            .filter(|s| s.faction_id == fid && s.hull > 0.0)
        {
            let name = s.name.clone();
            let was = state.ship_role(name.clone());
            // 轮不到自动控制决定的（玩家的叶钉死 / 订单叶不是 Auto）算**外生**：配额算的是
            // 全舰队，所以对账时要单独摆出来，别混进抽签那本账。
            if state.ship_control(name.clone()) != ControlMode::Auto
                || state.ship_role_control(name.clone()).is_player()
            {
                share_mut(&mut dist, was).exogenous += 1.0;
                continue;
            }
            let (role, odds) = decide_role(state, config, &fid, &name, inputs);
            // 级联的两个门槛：观测那一支的机会值归观测那份账，运输的归运输那份；
            // **入伙/退伍**按「这只舰原先是哪一支」分两侧。
            // **每枚骰子各记各的账**（观测没中 ⇒ 接着掷运输 ⇒ 两枚都要入账，各用各的 `flow`）。
            for (r, lot) in [
                (ShipRole::Observe, odds.observe),
                (ShipRole::Freight, odds.freight),
            ] {
                let Some((p, flow)) = lot else { continue };
                let sh = share_mut(&mut dist, r);
                if was == r {
                    sh.flow_leave = flow;
                    sh.sum_p_leave += p;
                    if p >= 1.0 - 1e-12 {
                        sh.clamped_leave += 1.0;
                    }
                } else {
                    sh.flow_join = flow;
                    sh.sum_p_join += p;
                    if p >= 1.0 - 1e-12 {
                        sh.clamped_join += 1.0;
                    }
                }
            }
            if odds.fixed.is_some() {
                share_mut(&mut dist, role).fixed += 1.0;
            }
            share_mut(&mut dist, role).actual += 1.0;
            plan.push((name, role));
        }
        inputs.role_distribution.insert(fid.clone(), dist);
        for (name, role) in plan {
            let unchanged = state
                .control(fid.clone())
                .and_then(|c| c.ship_role.get(&name))
                .map(|l| l.value == role && l.mode == ControlMode::Inherit)
                .unwrap_or(false);
            if unchanged {
                continue;
            }
            if let Some(c) = state.control_mut(fid.clone()) {
                c.ship_role.insert(name, Control::inherit(role));
            }
        }
    }
}

/// 给某舰挑一条路线（**两个方向共用一张抽签表**，见 [`lanes`]）。
///
/// * **出口腿** `站点 → 首都`：把产地用不完的货运回集散地（公理：首都即集散地）；
/// * **进口腿** `首都 → 站点`：把站点建设缺的货从首都送过去。
///
/// `None` = 没有货要动（或势力连首都都没有）。
///
/// **优先续用现有路线**——常驻路线不该每回合重掷：舱里有货 ⇒ 一定续（那票货得送到）；
/// 空舱 ⇒ 看**这条腿还有没有活**（出口腿看起点还有没有净剩余、进口腿看终点还有没有缺口）。
/// 抽签细节见本模块的文档。
///
/// **B5**：只有**真的抽了签**那一档（末尾）会往 `inputs` 记一条（早退的那几档没掷骰子——
/// 「执行承包单的舰跑的是单据上的路线」，那不是抽出来的）。
pub fn route_for(
    state: &State,
    config: &GameConfig,
    fid: &str,
    ship_id: &str,
    inputs: &mut RoundInputs,
) -> Option<(BodyId, BodyId)> {
    // **执行承包单的舰**跑的是那张单的路线（接单时立的承诺，不是抽签抽出来的）：
    // 起运在**托运方**那里（可能是它的货栈、也可能是它的首都池）、目的在托运方那一端——
    // 与自有运输的目标完全不同，所以这条要压在最前面，不能让它去抽自己的签。
    if let Some(id) = state.contracts.assignment_of(ship_id) {
        if let Some(c) = state.contracts.get(id) {
            return Some((c.from.clone(), c.to.clone()));
        }
    }
    let cap = state.capital_body(fid);
    if cap.is_empty() || state.body(&cap).is_none() {
        return None;
    }
    let holding = state
        .ship(ship_id)
        .map(|s| !s.cargo.is_empty())
        .unwrap_or(false);
    let cands = lanes(state, config, fid);
    // 续用现有路线：舱里有货就送完它；空舱则看这条腿还有没有活。
    if let Some(ShipBehavior::Haul { from, to }) = state.ship_behavior(ship_id.to_string()) {
        if state.body(&from).is_some() && state.body(&to).is_some() {
            let live = cands.iter().any(|l| l.from == from && l.to == to);
            if holding || live {
                return Some((from, to));
            }
        }
    }
    // 舱里有货但**没有**路线（例如玩家把指令清掉了）：先把货送回首都再说。
    // `from = to = 首都` 是合法的「只卸不装」路线——`haul_step` 只看 `to`（舱里有货时腿别就是 `to`）。
    if holding {
        return Some((cap.clone(), cap.clone()));
    }
    if cands.is_empty() {
        return None;
    }
    // 抽签：把 [0, 总量) 按每条腿的量切成区间，落在哪段就跑哪条 ⇒ 概率 = 量占比
    // （期望运力自动按「哪儿的货多」成比例，与派单纪律同源）。
    let total: f64 = cands.iter().map(|l| l.units).sum();
    if total <= 0.0 {
        return None;
    }
    let roll = sim::derived_roll(fid, ship_id, state.round, "route");
    let mut x = roll * total;
    let mut picked = cands.last().map(|l| (l.from.clone(), l.to.clone()))?; // 浮点兜底：落到末尾之外就取最后一条
    for l in &cands {
        if x < l.units {
            picked = (l.from.clone(), l.to.clone());
            break;
        }
        x -= l.units;
    }
    // **输入面（B5）**：记的是「掷出的那一枚」+ 池子的总权重 + **每条腿各有多少货**（B5c）
    // + 抽中的腿——于是「为什么它去了那个货栈」= 概率 ∝ 该腿的积压占比，而这里给出的是
    // 那一次的实况与全部对手。
    let pool: Vec<crate::model::PoolEntry> = cands
        .iter()
        .map(|l| crate::model::PoolEntry {
            name: format!("{}→{}", l.from, l.to),
            weight: l.units,
        })
        .collect();
    inputs
        .record_draw(
            "route",
            fid,
            ship_id,
            roll,
            total,
            &format!("{}→{}", picked.0, picked.1),
        )
        .pool = pool;
    Some(picked)
}

/// **一条腿上此刻的货**（挂单的主货种、考核的「还有没有活」都用它）。
///
/// 起点是首都 ⇒ **进口腿**：终点还缺的（[`site_deficit`]）；
/// 否则 ⇒ **出口腿**：起点用不完的净剩余（[`exportable_at`]）。
pub fn lane_cargo(
    state: &State,
    config: &GameConfig,
    fid: &str,
    from: &str,
    to: &str,
) -> ResourceMap {
    if from == state.capital_body(fid) {
        site_deficit(state, config, fid, to)
    } else {
        exportable_at(state, config, fid, from)
    }
}

/// **一条腿还有没有活**（考核分母与「续用路线」共用这一把尺子）。
///
/// * **出口腿**：起点还有净剩余就是有活；
/// * **进口腿**：终点还缺**且首都真的拿得出**那几种货——首都池空着的回合不该算在受雇方头上
///   （与出口腿「产地当期还没产出」是同一条宽免的理由）。
pub fn lane_has_work(state: &State, config: &GameConfig, fid: &str, from: &str, to: &str) -> bool {
    let cargo = lane_cargo(state, config, fid, from, to);
    if from != state.capital_body(fid) {
        return units_of(&cargo) > 1e-9;
    }
    let hub = state.stock_at(fid, from).cloned().unwrap_or_default();
    cargo
        .iter()
        .any(|(rt, want)| *want > 1e-9 && hub.get(rt).copied().unwrap_or(0.0) > 1e-9)
}

// --- 雇佣运力市场：雇主侧（挂单）-----------------------------------------------
//
// 集货腿有**两条路**：自己派船（上面那套定编 + 抽签派单），或**雇人来运**。
// 这一层负责第二条路里**雇主**的那一半：把自己派不出船的**运力缺口**挂出去。
//
// 用户裁决把这条腿定成**雇佣**：单子要求的是**运力**（单位/回合），不是一票货；
// 雇主挂单的逻辑与派自己的船**同源**（一处积压配一条船的运力，缺多少挂多少）；
// 受雇方自己派船（派几条都无所谓）。机制依据见 `.agents/notes/freight-collection.md` §4。
// **接单/派工/考核/续约/解约**（受雇方那一半 + 雇主的验货）在 `autocontrol::contract`。

/// 一艘运输舰跑**某条具体航线**的吞吐（单位/回合）：`舱容 × 每回合能跑几趟`。
///
/// 每回合的趟数 = `巡航速度 ÷ 往返航程`（往返 = `2 × 距离`）——**距离必然要进来**：
/// 同样的船，跑 0.3 AU 的金星和跑 30 AU 的柯伊伯带，单位时间的运力差两个数量级。
/// 距离为 0（起终点同一天体，例如迁都留下的旧中转货栈）时按**一回合一趟**算。
///
/// 与 [`freight_tonnage`] 的分工：那个是**定编**用的排序键（跨舰比较，不含航程——
/// 比的是船本身的运输效率），这个是**某条航线**上的实际吞吐（含航程）。两者不可互换。
pub fn trip_throughput(
    state: &State,
    config: &GameConfig,
    ship: &Ship,
    from: &str,
    to: &str,
    mond_control: f64,
) -> f64 {
    let panel = ship_panel(config, ship);
    if panel.speed <= 0.0 {
        return 0.0; // 动不了 ⇒ 吞吐是零（与定编同一个判据）。
    }
    // 与 [`trip_rounds`](crate::model::trip_rounds) **同源**：非 master 走深空要试多次、
    // 装卸还有离散回合，吞吐必须和合同考核用同一把尺子，不能再按「直线速度」高估。
    let rounds = trip_rounds(state, config, from, to, panel.speed, mond_control);
    if !rounds.is_finite() || rounds <= 0.0 {
        return 0.0;
    }
    cargo_capacity(config, ship) / rounds
}

/// 本势力**此刻能去跑运输的舰**（[`should_be_role`] 的名单，**扣掉正在替别人跑的**）。
///
/// 挂单发生在 `assign_roles` **之前**（见 `sim::step_contracts` 的注解），所以这里不能读
/// 角色叶——那片叶还是上一回合的结论。`should_be_role` 是**纯函数**，拿它算出来的
/// 正是本回合稍后会写进叶子、并据此派单的那批舰，因此估算与实际派单同口径。
///
/// **受雇在外的舰不算我的集货运力**：`should_be_role` 的第 0 条说「替别人跑的舰也是
/// 运输舰」（它得跑完那条线），但那是**别人的**线——把它算进「我自己能搬多少」会让雇主
/// 以为积压有着落了，从而少雇人（旧形态里这条估算还不会露馅，因为一张单只押一艘舰）。
fn serving_freighters<'a>(state: &'a State, config: &GameConfig, fid: &str) -> Vec<&'a Ship> {
    state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid && s.hull > 0.0)
        .filter(|s| state.contracts.assignment_of(&s.name).is_none())
        .filter(|s| should_be_role(state, config, fid, &s.name) == ShipRole::Freight)
        .collect()
}

/// 一处货栈里**积压最多的那种货**（同量按名字序 ⇒ 确定性）。`None` = 空了。
///
/// 它只是合同的**主货种**（用来折算货值、给人看）：受雇的船到了货栈装的是**当时有什么**
/// ——与雇主自己的运输舰完全一样（用户：「单主派发单的逻辑和派发自己运输船的逻辑一样」）。
/// 所以一处货栈**只有一张单**（旧形态是「一处货栈 × 一种货」各一张）。
fn principal_resource(map: &ResourceMap) -> Option<String> {
    let mut best: Option<(String, f64)> = None;
    for (r, v) in map {
        if *v > 1e-9 && best.as_ref().map(|(_, bv)| *v > *bv).unwrap_or(true) {
            best = Some((r.clone(), *v));
        }
    }
    best.map(|(r, _)| r)
}

/// **没人接的单不收回，而是「加价」**（用户裁决：价格做成**动态平衡**）。
///
/// * 挂单的**开叫价**统一（`freight.share`，人人都从 15% 起叫）；
/// * 一个**考核周期**（= 这条线的一个往返，与受雇方的验货节拍同一把尺子）没人接
///   ⇒ **抽成抬一档**，并把叫价起点挪到本回合（下一档要再等一个周期）；
/// * 抬到 `freight.share_max` 就不再加（雇主宁可让货烂在产地，也不会把大半货送人）。
///
/// 于是**深空/战区的价格是市场自己抬上去的**，而不是设计者用一个难度公式猜出来的。
/// **已有人接的单一个字都不动**：那时抽成是**承诺**（一份合同的条件下不该因结算顺序而变）。
fn escalate_open_contracts(state: &mut State, config: &GameConfig) {
    let f = &config.freight;
    let factor = f.share_escalation.max(1.0);
    let cap = f.share_max.max(f.share);
    let round = state.round;
    // 先算完再写（同一回合内几个势力的结论互不影响；也免得边遍历边借用）。
    let plan: Vec<(u64, f64)> = state
        .contracts
        .contracts
        .iter()
        .filter(|c| c.is_open())
        .filter(|c| {
            let interval = crate::model::hire_terms(state, config, &c.from, &c.to).interval;
            round >= c.posted_round.saturating_add(interval)
        })
        .map(|c| (c.id, (c.share * factor).min(cap)))
        .collect();
    for (id, share) in plan {
        if let Some(c) = state.contracts.get_mut(id) {
            c.share = share;
            c.posted_round = round; // 重新起叫：下一档要再等一个完整周期
        }
    }
}

/// 雇主这一回合要动的一张单（先只读算完，再一次性写状态 ⇒ 同回合内几个势力互不影响）。
enum Plan {
    /// 改一张**未接单**的缺口（需求信号跟着现实走）。
    Revise {
        id: u64,
        resource: String,
        capacity: f64,
    },
    /// 撤回一张**没人接**的单（这条线不再缺运力，或货栈空了）。
    Drop { id: u64 },
    /// 挂一张新单。
    Post {
        shipper: FactionId,
        resource: String,
        capacity: f64,
        from: BodyId,
        to: BodyId,
    },
}

/// 本势力**每一条腿的运力账**：`(起点, 终点, 要求运力, 自有运力, 已雇运力, 缺口)`。
///
/// * **自有运力**落到每条腿的那一份是**期望值**：派单是**按货量占比抽签**的（[`route_for`]），
///   所以「期望落到这条腿的那一份」= `Σ(各运输舰在这条线上的吞吐) × (这条腿的货量 ÷ 总货量)`
///   ——与真实派单**同口径**，不是另编一个模型；
/// * **已雇运力**按**已接单合同的 `capacity`** 算（接单就是承诺），不看此刻有几条船在跑；
/// * **缺口** = `max(0, 要求 − 自有 − 已雇)`：**连续量、无阈值**，雇够了自己归零。
///
/// 出口腿与进口腿**同一本账**：两条腿的缺口都是「我搬不动的那部分」，所以「该雇人还是该自己
/// 造船」在两边是同一条判据（用户裁决：**进承包商**）。
///
/// 一本账供两处用：雇主挂单（[`post_contracts`]）与「该不该腾个船坞去造货船」
/// （[`crate::autocontrol::shipbuilding::retool_haulers`]）。
pub fn capacity_ledger(
    state: &State,
    config: &GameConfig,
    fid: &str,
) -> Vec<(BodyId, BodyId, f64, f64, f64, f64)> {
    let lns = lanes(state, config, fid);
    let total: f64 = lns.iter().map(|l| l.units).sum();
    if total <= 0.0 {
        return Vec::new();
    }
    let serve = serving_freighters(state, config, fid);
    let mut committed: BTreeMap<(BodyId, BodyId), f64> = BTreeMap::new();
    for c in state
        .contracts
        .contracts
        .iter()
        .filter(|c| c.shipper == fid && c.is_hired())
    {
        *committed
            .entry((c.from.clone(), c.to.clone()))
            .or_insert(0.0) += c.capacity;
    }
    lns.iter()
        .map(|l| {
            let control = state
                .faction(fid)
                .map(|f| f.mond_control)
                .unwrap_or(0.0);
            let rate: f64 = serve
                .iter()
                .map(|s| trip_throughput(state, config, s, &l.from, &l.to, control))
                .sum();
            let own = rate * (l.units / total);
            let need = crate::model::required_throughput(state, config, &l.from, &l.to);
            let hired = committed
                .get(&(l.from.clone(), l.to.clone()))
                .copied()
                .unwrap_or(0.0);
            (
                l.from.clone(),
                l.to.clone(),
                need,
                own,
                hired,
                (need - own - hired).max(0.0),
            )
        })
        .collect()
}

/// 本势力**搬不动的比例** = `Σ缺口 ÷ Σ要求运力` ∈ [0, 1]（0 = 自己的船够，1 = 一件也搬不动）。
///
/// 这是**造货船的需求信号**（[`crate::autocontrol::shipbuilding::retool_haulers`]）：
/// **已经雇到人**的那部分不算缺口——雇佣市场本来就该顶掉它。所以「一直雇不到人、或雇到了
/// 也不够」才会推动船坞改产货船；而「雇得到」的势力本来就不必自己造船（分工，而不是重复建设）。
/// 没有货要动（或没有首都）⇒ 0。
pub fn haul_gap(state: &State, config: &GameConfig, fid: &str) -> f64 {
    let ledger = capacity_ledger(state, config, fid);
    let need: f64 = ledger.iter().map(|(_, _, n, _, _, _)| n).sum();
    let uncovered: f64 = ledger.iter().map(|(_, _, _, _, _, u)| u).sum();
    if need <= 0.0 {
        0.0
    } else {
        (uncovered / need).clamp(0.0, 1.0)
    }
}

/// **挂单**：把「自己派不出船的运力缺口」挂到雇佣市场上（一处货栈一张）。
///
/// # 派单逻辑与派自己的船**完全同源**（用户裁决）
///
/// 雇主先按**已有的定编规则**把自己的船派出去：一处有积压的货栈要**一条船的运力**
/// （`needed_freighters` 的那条口径），而「这条线需要多少运力」正是
/// [`crate::model::required_throughput`]（= 一条参考船在这条线上的吞吐）。
/// 两者**同尺度**，所以「我缺多少」不需要另编一套估算：
///
/// ```text
/// 缺口 = 要求运力 − 自有运力（落到这处的期望份额） − 已雇到的运力（已接单的 capacity）
/// ```
///
/// * **自有运力**那一份为什么是期望值：派单是**按积压占比抽签**的（[`route_for`]），
///   所以「期望落到这处的那一份」= `Σ(各运输舰在这条线上的吞吐) × (这处积压 ÷ 总积压)`。
///   这与真实派单**同口径**——不是另编一个模型。
/// * **已雇到的运力**按**已接单合同的 `capacity`** 算，不看此刻有几条船在跑：接下就是承诺，
///   接单那一刻它就该顶掉缺口（否则雇主要在收到第一条船之前反复挂单）。
///
/// 于是「连续量、无阈值」这条纪律自动成立：运力缺口为 0 的线**一件不挂**（`max(0, ·)`），
/// 缺口大的线挂得多——挂的是**吞吐**而不是「几条船」，所以受雇方派几条船都行。
///
/// # 需求信号跟着现实走，接单后冻结
///
/// 同一处货栈**只有一张未接单**，且它**每回合被改成此刻的缺口**（[`ContractState::open_mut`]）：
/// * 积压涨了、自己的船少了 ⇒ 缺口变大；自己的船补上了 ⇒ 缺口变小甚至**撤单**
///   （这条线不缺运力了，没必要继续请人——这是**内生的撤单**，不是「挂出去就等人接」）；
/// * 货栈被自己的船搬空 ⇒ 那张未接单直接撤回（需求信号必须跟着现实走，**哪怕现实是「没货了」**，
///   否则受雇方会照着一条不存在的需求派船过来）。
///
/// **一旦有人接了** ⇒ `capacity` 冻结（`open_mut` 只找 `carrier.is_none()` 的单）：那时它已经
/// 不是需求而是**承诺**了。
pub(crate) fn post_contracts(state: &mut State, config: &GameConfig, flow: &mut RoundSink) {
    // 先加价：一个考核周期没人接的单子，**抬一档抽成并重新起叫**。
    escalate_open_contracts(state, config);
    let mut fids: Vec<FactionId> = state.factions.iter().map(|f| f.name.clone()).collect();
    fids.sort(); // 确定性：写状态的顺序不依赖势力表的排列
    let mut plan: Vec<Plan> = Vec::new();
    for fid in &fids {
        // 照公理：集散地是自己的首都。没有首都（或首都天体不存在）就没有「集散地」，
        // 也就无从挂单——两条腿（出口/进口）都以它为另一端。
        let cap = state.capital_body(fid);
        if cap.is_empty() || state.body(&cap).is_none() {
            continue;
        }
        let lns = lanes(state, config, fid);
        let total: f64 = lns.iter().map(|l| l.units).sum();
        // 本势力**未接单**的单子（按**整条腿** `(起点, 终点)` 索引）——下面要么改它、要么撤它，
        // 不会堆成一片。⚠ 从前只按 `from` 索引，两个方向的腿都以首都为起点时（两条进口腿）
        // 会互相认错，所以键必须是整条腿。
        let open: BTreeMap<(BodyId, BodyId), u64> = state
            .contracts
            .contracts
            .iter()
            .filter(|c| c.shipper == *fid && c.is_open())
            .map(|c| ((c.from.clone(), c.to.clone()), c.id))
            .collect();
        // **这条腿没活了**的未接单：撤回（`lanes` 只列出此刻真有货要动的腿，所以没被列出的
        // 就是「没活了」——需求信号必须跟着现实走，**哪怕现实是「货搬完了」**，
        // 否则受雇方会照着一条不存在的需求派船过来）。
        let alive: BTreeSet<(BodyId, BodyId)> =
            lns.iter().map(|l| (l.from.clone(), l.to.clone())).collect();
        for (lane, id) in &open {
            if !alive.contains(lane) {
                plan.push(Plan::Drop { id: *id });
            }
        }
        if total <= 0.0 {
            continue; // 没有货要动 ⇒ 没有需求（上面的清扫已经把旧单撤掉了）
        }
        // **每一条腿的运力账**（要求 / 自有期望份额 / 已雇 / 缺口），一本账供两处用：
        // 这里挂单，[`crate::autocontrol::shipbuilding::retool_haulers`] 据此决定要不要
        // 腾个船坞去造货船——各算一份必然漂移。
        let ledger = capacity_ledger(state, config, fid);
        // 记这本账（B3 的中间量）：**挂单用的就是它**，而它此前只以势力级的 `haul_gap`
        // （`Σ缺口 ÷ Σ要求`）露出来——「哪一处货栈在积压、缺口多少」没有读法。
        // ⚠ 记的是**这一步算出来的**那份：回合末重算会得到另一个数（那时船已经动过、货已经装卸过）。
        //
        // ⚠ **键 = 站点（那条腿的「非首都端」），两个方向合并**：改成两条腿之后，同一个站点
        // 可能**同时**有出口腿与进口腿（逐资源可以一边多、一边缺），而读面那一列问的是
        // 「这处站点还差多少运力」——所以按站点聚合（同站点的两个方向相加）。
        // 每条腿**恰有一个**非首都端 ⇒ 聚合不丢账，`Σneed`/`Σuncovered` 与 `haul_gap` 逐字对得上
        // （`src/tests/sim/trade.rs` 那条守卫仍然成立）。
        let cap_body = state.capital_body(fid);
        let mut by_site: BTreeMap<BodyId, FreightGap> = BTreeMap::new();
        for (from, to, need, own, hired, uncovered) in &ledger {
            if *need <= 1e-9 {
                continue;
            }
            let site = if from == &cap_body { to } else { from };
            let e = by_site.entry(site.clone()).or_insert(FreightGap {
                need: 0.0,
                own: 0.0,
                hired: 0.0,
                uncovered: 0.0,
            });
            e.need += need;
            e.own += own;
            e.hired += hired;
            e.uncovered += uncovered;
        }
        flow.freight_gap.insert(fid.clone(), by_site);
        // 挂单按**整条腿**索引（`(起点, 终点)`）：两条腿都可能以首都为起点，只按一端会互相认错。
        let by_lane: BTreeMap<(BodyId, BodyId), (f64, f64, f64, f64)> = ledger
            .iter()
            .map(|(f, t, n, o, h, u)| ((f.clone(), t.clone()), (*n, *o, *h, *u)))
            .collect();
        for l in &lns {
            let key = (l.from.clone(), l.to.clone());
            let Some((_need, _own, _hired, uncovered)) = by_lane.get(&key).copied() else {
                continue;
            };
            // 主货种 = 这条腿上**此刻的货**里最多的那种（出口 = 净剩余，进口 = 缺口）：
            // 它只用来折算货值（门槛与自评闸）与给人看，不约束承运人装什么。
            let Some(resource) =
                principal_resource(&lane_cargo(state, config, fid, &l.from, &l.to))
            else {
                continue;
            };
            match open.get(&key) {
                // 已有的未接单：改成此刻的缺口；不缺了就撤回。
                Some(id) => {
                    if uncovered <= 1e-9 {
                        plan.push(Plan::Drop { id: *id });
                    } else if let Some(c) = state.contracts.get(*id) {
                        if (c.capacity - uncovered).abs() > 1e-9 || c.resource != resource {
                            plan.push(Plan::Revise {
                                id: *id,
                                resource,
                                capacity: uncovered,
                            });
                        }
                    }
                }
                None => {
                    if uncovered > 1e-9 {
                        plan.push(Plan::Post {
                            shipper: fid.clone(),
                            resource,
                            capacity: uncovered,
                            from: l.from.clone(),
                            to: l.to.clone(),
                        });
                    }
                }
            }
        }
    }
    for p in plan {
        match p {
            // 改数/撤单不发事件：它们只是「需求变了」，不是一件**发生的事**（只有新单才是）。
            Plan::Revise {
                id,
                resource,
                capacity,
            } => {
                if let Some(c) = state.contracts.get_mut(id) {
                    c.resource = resource;
                    c.capacity = capacity;
                }
            }
            Plan::Drop { id } => {
                state.contracts.release(id); // 未接单的本就没有船，收尾而已
                state.contracts.remove(id);
            }
            Plan::Post {
                shipper,
                resource,
                capacity,
                from,
                to,
            } => {
                // 条款在**挂单这一刻**算好并冻结（门槛）：天体在动，每回合重算会让
                // 同一张单的条件漂移，而合同一旦挂出去，条件就该是固定的。
                let min_reputation =
                    crate::model::required_reputation(state, config, &resource, &from, &to);
                let share = config.freight.share;
                let id = state.contracts.post(
                    shipper.clone(),
                    resource.clone(),
                    capacity,
                    from.clone(),
                    to.clone(),
                    share,
                    state.round,
                    min_reputation,
                );
                sim::ev(
                    state,
                    GameEvent::ContractPosted {
                        contract: id,
                        shipper,
                        resource,
                        capacity,
                        from,
                        to,
                        share,
                    },
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/autocontrol/freight.rs"]
mod tests;
