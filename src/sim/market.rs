//! 市场与承包：价格、封锁、挂单/成交、雇佣运力市场、供需撮合。

use super::*;

/// 军工需要的资源集合：所有舰级的 `build_cost` ∪ 所有组件的 `cost`。
/// 这是买方想常备的目标集合（一个势力有船坞，就想备齐这些料）。
pub fn military_need(config: &GameConfig) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for cls in config.ships.keys() {
        for rt in config.ship_spec(cls).build_cost.keys() {
            out.insert(rt.clone());
        }
    }
    for id in config.components.keys() {
        for rt in config.component_spec(id).cost.keys() {
            out.insert(rt.clone());
        }
    }
    out
}

/// **关系即价格**：卖方 `seller` 卖给买方 `buyer` 时的实际成交价倍率。
///
/// * 关系为负 → 加价（越冷越贵），到交战边缘封顶 `hostile_price_markup`（默认 2.5×）。
/// * 关系为正 → 折扣（越暖越便宜），到 `friendly_relation` 封顶 `friendly_price_discount`
///   （默认 85 折）。
///
/// 同一个矿，向朋友买和向敌人买不是一个价——这是「不同关系不同价格」（D4）。
pub fn relation_price_mult(state: &State, config: &GameConfig, seller: &str, buyer: &str) -> f64 {
    let m = &config.market;
    let rel = relation(state, seller, buyer);
    if rel < 0.0 {
        // 跨度 = 交战阈值的绝对值再多一点（越过它就已经开战/禁运了）。
        let span = (-config.combat.war_threshold).max(1.0) + 1.0;
        1.0 + m.hostile_price_markup * ((-rel) / span).clamp(0.0, 1.0)
    } else {
        let f = m.friendly_relation.max(1.0);
        1.0 - m.friendly_price_discount * (rel / f).clamp(0.0, 1.0)
    }
}

/// **全面禁运**：某一方「根本不卖给你」——**所有资源**都断供（D4：全面）。
///
/// 三档判据（任一档成立即封锁）：
/// 1. **交战**：关系 ≤ `war_threshold`（任一方这样看对方即可）。
/// 2. **反制联盟封锁**：已倒向联盟的弱者 ↔ 被锁定的霸权（取代旧的 `sanction_trade_mult`
///    ——那个只是「少卖一点」，这个是真的「不卖」）。
/// 3. **冷到断供**：关系 ≤ `embargo_relation`（比交战阈值更早，还没开打就断货）。
///
/// 判据是**对称**的：一方不卖，买卖就做不成。公开给测试/观测（`--schema` 之外的控制面
/// 也可用它回答「他到底卖不卖我」）。
pub fn trade_blocked(state: &State, config: &GameConfig, a: &str, b: &str) -> bool {
    trade_block_cause(state, config, a, b).is_some()
}

/// [`trade_blocked`] 的**原因**（`None` = 没封锁）。把「为什么断供」区分开，才能对账
/// ——否则「禁运太多」无法判断是战争、联盟还是阈值太严。
pub fn trade_block_cause(state: &State, config: &GameConfig, a: &str, b: &str) -> Option<&'static str> {
    if a == b {
        return None;
    }
    if hostile(state, config, a, b) || hostile(state, config, b, a) {
        return Some("war");
    }
    let cold = config.market.embargo_relation;
    if relation(state, a, b) <= cold || relation(state, b, a) <= cold {
        return Some("cold");
    }
    // 反制联盟对霸权的封锁：谁「倒向联盟」谁就不跟霸权做生意。
    if config.balance.hegemon_power <= 1.0 {
        if let Some(h) = sanctioned_hegemon(state, config) {
            let estranged = |x: &str| x != h && relation(state, x, &h) <= config.balance.coalition_estrange;
            if (a == h && estranged(b)) || (b == h && estranged(a)) {
                return Some("coalition");
            }
        }
    }
    None
}

/// 星际市场（真实交换所）。每回合三步，全部确定性、无 RNG：
///
/// 1. **价格发现**：`mult = (coverage_rounds / 覆盖回合数)^price_alpha`，其中
///    「覆盖回合数」= 世界总库存 ÷ 全球消费率（滑窗）。稀缺 → 高价（顶到
///    [`MarketConfig::price_ceiling`]），过剩 → 折价（[`MarketConfig::price_floor`]）。
///    消费率由「上回合市场时刻的库存 + 本回合产出 − 本回合市场时刻的库存」实测——
///    不猜需求。
/// 2. **挂单**：供给来自**各势力真实的富余**（库存扣掉自己要留的部分），挂单**记名卖家**
///    （禁运判据在卖家身上，见 [`step_market`] 的可见性过滤）。
/// 3. **结算（配给）**：买方按购买力（自己可出口富余的价值）从别人的挂单里买，
///    **仓里有多少卖多少**；买不到就是买不到。付款=把自己可出口的实物交给卖家，
///    另按 `spread` 烧掉一笔手续费（真实的价值 sink）。
///
/// 与旧实现的根本差别：旧版是**常数价的无限贩卖机**（没有卖家、没有仓、没有价格），
/// 所以「缺某种矿」不可能更贵、也不可能「不卖给你」。见
/// `.agents/notes/trade-and-sanctions.md` 的实测基线。
pub fn step_market(state: &mut State, config: &GameConfig, flow: &mut RoundSink) {
    let m = &config.market;
    if m.auto_trade_limit <= 0.0 {
        return;
    }
    let value_of = |rt: &str| config.resources.get(rt).map(|r| r.value).unwrap_or(1.0);
    let need = military_need(config);

    // --- 1) 观测：世界总库存 + 本回合产出 → 消费率（滑窗）→ 价格 ----------------
    let mut world_stock: ResourceMap = ResourceMap::new();
    for f in &state.factions {
        for (rt, v) in &f.resources {
            *world_stock.entry(rt.clone()).or_insert(0.0) += *v;
        }
    }
    let mut produced: ResourceMap = ResourceMap::new();
    for prod in flow.faction_production.values() {
        for (rt, v) in prod {
            *produced.entry(rt.clone()).or_insert(0.0) += *v;
        }
    }
    let last = state.market.last_stock.clone();
    let first_round = last.is_empty();
    let mut price: ResourceMap = ResourceMap::new();
    let mut avg_demand: ResourceMap = ResourceMap::new();
    for rt in config.resources.keys() {
        let have = world_stock.get(rt).copied().unwrap_or(0.0);
        let made = produced.get(rt).copied().unwrap_or(0.0);
        // 消费 = 上回合市场时刻库存 + 本回合产出 − 本回合市场时刻库存。
        // 中间发生的支出：上回合的建设/治理 + 本回合的维护 + 市场手续费。
        let consumed = if first_round {
            0.0
        } else {
            (last.get(rt).copied().unwrap_or(0.0) + made - have).max(0.0)
        };
        let prev = state.market.avg_demand.get(rt).copied().unwrap_or(0.0);
        let demand = if first_round || prev <= 0.0 {
            consumed
        } else {
            prev * (1.0 - m.demand_smoothing) + consumed * m.demand_smoothing
        };
        avg_demand.insert(rt.clone(), demand);
        let mult = if demand <= m.demand_min {
            // 几乎没人消费它 → 没有稀缺信号，按基价（否则没人用的矿会被永久顶成天价）。
            1.0
        } else {
            let cover = (have / demand).max(m.cover_floor);
            (m.coverage_rounds / cover)
                .powf(m.price_alpha)
                .clamp(m.price_floor, m.price_ceiling)
        };
        price.insert(rt.clone(), value_of(rt) * mult);
    }

    // --- 2) 挂单：真实供给（谁卖、卖什么、多少、什么价） ------------------------
    let faction_ids: Vec<FactionId> = state.factions.iter().map(|f| f.name.clone()).collect();
    let mut offers: Vec<Offer> = Vec::new();
    // 挂单簿：卖方还能交出什么（结算期间唯一权威的「可给出去的实物」账本）。
    let mut remaining: BTreeMap<(FactionId, String), f64> = BTreeMap::new();
    for fid in &faction_ids {
        let Some(f) = state.faction(fid) else { continue };
        for (rt, amt) in &f.resources {
            // 自己需要的资源至少留 `working_buffer`；其余按 `reserve_fraction` 留一半。
            let keep = if need.contains(rt) {
                m.working_buffer.max(amt * m.reserve_fraction)
            } else {
                amt * m.reserve_fraction
            };
            let over = amt - keep;
            if over > 1e-6 {
                offers.push(Offer {
                    seller: fid.clone(),
                    resource: rt.clone(),
                    amount: over,
                    ask: price.get(rt).copied().unwrap_or_else(|| value_of(rt)),
                });
                remaining.insert((fid.clone(), rt.clone()), over);
            }
        }
    }

    // --- 3) 结算：买方按购买力从别人的挂单里买（配给） --------------------------
    // 买方顺序＝购买力降序（「钱多的人先买」，确定性：同额按名字排序）。
    let mut buyers: Vec<(FactionId, f64)> = faction_ids
        .iter()
        .map(|fid| (fid.clone(), listed_value(&remaining, &price, &value_of, fid)))
        .collect();
    buyers.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let mut settled: ResourceMap = ResourceMap::new();
    let mut net_import: BTreeMap<FactionId, f64> = BTreeMap::new();
    let mut spent: BTreeMap<FactionId, f64> = BTreeMap::new();
    let mut freight_paid: BTreeMap<FactionId, f64> = BTreeMap::new();
    let mut carrier_income: BTreeMap<FactionId, f64> = BTreeMap::new();
    // **买方队列本身就是读面数据**（B3）：「为什么有货在卖我却没买到」的答案有一半在这里
    // ——购买力决定谁先挑。名次按**引擎自己排好的顺序**记（别让读者拿购买力重排一遍：
    // 同额时的名字序是这里的 tie-break）。
    for (rank, (fid, power)) in buyers.iter().enumerate() {
        flow.market_power.insert(fid.clone(), *power);
        flow.market_rank.insert(fid.clone(), rank);
    }
    // 本回合**真的成交**的每一对买卖方（B3 的中间量）：键是 (买方, 卖方)，值是那一对的价格
    // 分解与丢货率。**粒度选「一对一行」而不是「一对 × 一资源一行」**：下面这些数只由这一对
    // 决定（距离/深度/关系），与买的是哪种矿无关；「各买了多少件」收在 `moved` 里。
    let mut trades: BTreeMap<(FactionId, FactionId), MarketTrade> = BTreeMap::new();
    for (buyer, _) in buyers {
        // 想买的：军工需要、且低于目标库存的资源，**越贵越先买**（先抢最稀缺的）。
        let stock = state.faction(&buyer).map(|f| f.resources.clone()).unwrap_or_default();
        let mut want: Vec<(String, f64, f64)> = need
            .iter()
            .map(|rt| {
                let have = stock.get(rt).copied().unwrap_or(0.0);
                let p = price.get(rt).copied().unwrap_or_else(|| value_of(rt));
                (rt.clone(), (m.working_buffer - have).max(0.0), p)
            })
            .filter(|(_, w, _)| *w > 1e-9)
            .collect();
        want.sort_by(|a, b| b.2.total_cmp(&a.2).then_with(|| a.0.cmp(&b.0)));

        for (rt, mut short, p) in want {
            if short <= 1e-9 || p <= 1e-9 {
                continue;
            }
            // 别的势力此刻还剩多少这种资源在卖（按卖家名排序 → 确定性）。
            // **禁运过滤**：不卖给你的卖家，其挂单对你根本不存在。
            let sellers: Vec<(FactionId, f64)> = remaining
                .iter()
                .filter(|((s, r), amt)| {
                    r == &rt && s != &buyer && **amt > 1e-9 && !trade_blocked(state, config, s, &buyer)
                })
                .map(|((s, _), amt)| (s.clone(), *amt))
                .collect();
            for (seller, avail) in sellers {
                if short <= 1e-9 {
                    break;
                }
                // 购买力每次现算：自己的货可能已经被别人买走了。
                let spendable = listed_value(&remaining, &price, &value_of, &buyer);
                let limit_left = (m.auto_trade_limit - spent.get(&buyer).copied().unwrap_or(0.0)).max(0.0);
                let max_purchase = (spendable / (1.0 + m.spread).max(1e-9)).min(limit_left);

                // --- 路线：距离 + 引力异常带浸入深度（M6）---------------------------
                // 货物不是瞬移的：运得越远越贵；要穿越异常带就更贵——而只有掌握了 MOND 的
                // 势力能可靠走那条线（其余人要么付溢价，要么**丢货**）。
                let anchor_b = trade_anchor(state, &buyer);
                let anchor_s = trade_anchor(state, &seller);
                let depth = route_depth(config, anchor_b, anchor_s);
                let dist_au = dist(anchor_b, anchor_s);
                let mond_extra = if depth > 0.0 {
                    m.mond_freight_mult * (depth / (depth + 1.0))
                } else {
                    0.0
                };
                let freight_rate = m.freight_per_au * dist_au * (1.0 + mond_extra);
                // 成交价 = 市场价 ×（关系倍率 + 运费率）：向敌人买、运得远、要过异常带都更贵。
                let rel_mult = relation_price_mult(state, config, &seller, &buyer);
                let p_eff = p * (rel_mult + freight_rate);
                let take = short.min(avail).min(max_purchase / p_eff.max(1e-9));
                if take <= 1e-9 {
                    continue;
                }
                let give_market = take * p;               // 货值（按市场价）
                let pay_seller = give_market * rel_mult;  // 卖方实收（含关系溢价/折扣）
                let freight = give_market * freight_rate; // 运费
                let cost = pay_seller + freight;          // 贸易额（不含手续费）
                let fee = cost * m.spread;                // 市场手续费（烧掉）
                // 丢货：货走异常带会**部分失联**，比例随「这条线上最好的掌握度」连续下降
                // （`1 − max(买卖双方掌握度)`：任一方到顶就是 0 损失，与旧版「有一个 master
                // 就不丢」同口径）。确定性比例，不是掷骰——掷骰会污染 `Prng` 流。
                let route_mastery =
                    mond_control(state, &buyer).max(mond_control(state, &seller));
                let loss = if depth > 0.0 {
                    (m.mond_loss_per_au * depth * (1.0 - route_mastery))
                        .clamp(0.0, m.mond_loss_cap)
                } else {
                    0.0
                };
                let lost_units = take * loss;
                let received = give_market * (1.0 - loss);

                // 实物交割：卖家的货 → 买家；途中损失的那部分直接消失。
                if let Some(f) = state.faction_mut(&seller) {
                    let e = f.resources.entry(rt.clone()).or_insert(0.0);
                    *e = (*e - take).max(0.0);
                }
                if let Some(f) = state.faction_mut(&buyer) {
                    *f.resources.entry(rt.clone()).or_insert(0.0) += take - lost_units;
                }
                if let Some(r) = remaining.get_mut(&(seller.clone(), rt.clone())) {
                    *r -= take;
                }
                *settled.entry(rt.clone()).or_insert(0.0) += take - lost_units;

                // 记这一对（买方 × 卖方）的本回合结算：价格分解 + 丢货（B3 的中间量）。
                // 同一对可能买好几种矿 —— 那些数**只由这一对决定**，所以只写一次
                // （`or_insert_with`：拿第一个成交的那个资源时刻的值，其余资源同值）。
                let entry = trades
                    .entry((buyer.clone(), seller.clone()))
                    .or_insert_with(|| MarketTrade {
                        buyer: buyer.clone(),
                        seller: seller.clone(),
                        moved: ResourceMap::new(),
                        dist_au,
                        depth,
                        mond_extra,
                        freight_rate,
                        rel_mult,
                        mastery: route_mastery,
                        loss,
                    });
                *entry.moved.entry(rt.clone()).or_insert(0.0) += take;

                // 付款：卖方（货款）+ 承运人（异常带那一段运费）+ 市场（手续费，烧掉）。
                pay_with_surplus(state, &mut remaining, &price, &value_of, &buyer, Some(&seller), pay_seller);
                let carrier = if depth > 0.0 && m.carrier_share > 0.0 {
                    // 承运人 = **掌握度最高**、且不是买卖双方的势力（并列取 `state.factions`
                    // 里靠前的那个，保证确定性）。掌握度连续化之后「谁在承运」不再读配置名单，
                    // 而是读世界里的实际掌握度；它的**抽成按掌握度折算**（见下）——所以
                    // 「名单」那一步特权路径到此为止，留下的是一条连续的斜坡。
                    let mut best: Option<&crate::model::Faction> = None;
                    for f in &state.factions {
                        if f.name == buyer || f.name == seller || f.mond_control <= 0.0 {
                            continue;
                        }
                        if best.map_or(true, |b| f.mond_control > b.mond_control) {
                            best = Some(f);
                        }
                    }
                    best.map(|f| (f.name.clone(), f.mond_control))
                } else {
                    None
                };
                match &carrier {
                    Some((c, mastery)) => {
                        // 只有掌握 MOND 的人能可靠穿越异常带 → 它对这条线上的贸易**抽税**；
                        // 抽得多狠 = 它的掌握度（`control = 1` 时与旧版逐位相同）。
                        let carrier_fee = freight * m.carrier_share * mastery;
                        pay_with_surplus(state, &mut remaining, &price, &value_of, &buyer, Some(c), carrier_fee);
                        pay_with_surplus(state, &mut remaining, &price, &value_of, &buyer, None, freight - carrier_fee);
                        *carrier_income.entry(c.clone()).or_insert(0.0) += carrier_fee;
                    }
                    None => {
                        // 没有承运人（或承运人就是买卖双方之一）：那一段运费直接烧掉。
                        pay_with_surplus(state, &mut remaining, &price, &value_of, &buyer, None, freight);
                    }
                }
                pay_with_surplus(state, &mut remaining, &price, &value_of, &buyer, None, fee);
                *freight_paid.entry(buyer.clone()).or_insert(0.0) += freight;

                // 净进口 = 实收货值 − 全部流出（关系溢价是老账，运费与丢货是新账）。
                *net_import.entry(buyer.clone()).or_insert(0.0) += received - cost - fee;
                *net_import.entry(seller.clone()).or_insert(0.0) += pay_seller - give_market;
                *spent.entry(buyer.clone()).or_insert(0.0) += cost;
                short -= take;
            }
        }
    }

    // --- 4) 写回：挂单（原始清单，供观测）/ 价格 / 成交 / 需求滑窗 ---------------
    state.market = MarketState {
        offers,
        price,
        settled,
        avg_demand,
        last_stock: world_stock,
    };
    for (fid, v) in net_import {
        flow.market_net.insert(fid, v);
    }
    for (fid, v) in freight_paid {
        flow.market_freight.insert(fid, v);
    }
    for (fid, v) in carrier_income {
        flow.market_carrier_income.insert(fid, v);
    }
    // 成交清单（按买卖双方名排序 ⇒ 确定性的行序）。
    flow.market_trades = trades.into_values().collect();
}

// --- 雇佣运力市场（集货腿的第二条路：雇人来运）---------------------------------
//
// 集货腿有两条路：**自己派船**（`step_ships` 里的定编 + 抽签派单）与**雇人来运**。
// 本步管后者：雇主挂单 + 受雇方接单/派工 + 雇主的考核与续约/换人。
//
// 为什么它不是「又一个市场步」而是独立的一步：**雇佣的标的是运力，不是货**。
// 商品市场撮合的是「谁卖什么、多少钱」，成交即完成（货瞬移）；雇佣撮合的是
// 「谁替谁跑哪条线、跑得够不够」，成交只是**开始**——后面要真的有船去装、去运、去卸
// （复用 `Haul` 的常驻路线），而且**每个考核期还要验一次货**，所以它是运输行为的一部分。
//
// 依据与裁决见 `.agents/notes/freight-collection.md` §4（Q1(b) 只扣信誉 / Q2 挂单制 /
// Q4 禁运同样挡雇佣 / Q10 抽成制 / 雇佣形态：单子要求运力、派几条船都无所谓、
// 周期考核出信誉、船沉没不管）。
pub fn step_contracts(state: &mut State, config: &GameConfig, flow: &mut RoundSink) {
    // 1) 雇主挂单（内含**加价**：一个考核周期没人接就抬一档，见 `freight::escalate_open_contracts`）。
    autocontrol::freight::post_contracts(state, config, flow);
    // 2) 挂完就撮合：看得见、又愿意接的受雇方**按信誉加权抽签**接下（`carrier` 落定、
    //    雇佣期起算）。**不押船**——派几条船是受雇方自己的事。
    autocontrol::contract::match_carriers(state, config);
    // 3) 受雇方派工：把自己的空闲船按**缺口**补到手上的单上；自己缺船时又收回它们
    //    （缺船也提前结束手上的雇佣）。路线与角色叶**不在这里写**——`step_ships` 的运输舰
    //    分支与 `assign_roles` 会照常处理（它们都认识派工记录），每个叶子只有一个写者。
    autocontrol::contract::assign_hired_ships(state, config);
    // 4) 巡检：记考核分母 → 到点**考核**（信誉的唯一来源）→ 固定期到期**续约或换人**。
    //    交付不在这里——它发生在 `haul_unload` 那一刻（船真的靠了泊位）。
    autocontrol::contract::settle_contracts(state, config);
    // 5) 单子没了 ⇒ 清掉指向它的派工，免得有舰永远钉在一张不存在的单上。
    state.contracts.drop_dangling_assignments();
}

/// 某势力此刻挂单簿上的**总价值**（= 它的购买力：能拿出来交换的实物值多少）。
pub fn listed_value(
    remaining: &BTreeMap<(FactionId, String), f64>,
    price: &ResourceMap,
    value_of: &impl Fn(&str) -> f64,
    fid: &FactionId,
) -> f64 {
    remaining
        .iter()
        .filter(|((s, _), _)| s == fid)
        .map(|((_, rt), amt)| amt * price.get(rt).copied().unwrap_or_else(|| value_of(rt)))
        .sum()
}

/// 从 `fid` 的挂单簿里取出价值 `value` 的实物：
/// `to = Some(卖家)` 时交割给对方（付款），`to = None` 时实物消失（市场手续费 sink）。
/// 按资源名确定性顺序取，因此整条结算链无 RNG、可复现。
pub fn pay_with_surplus(
    state: &mut State,
    remaining: &mut BTreeMap<(FactionId, String), f64>,
    price: &ResourceMap,
    value_of: &impl Fn(&str) -> f64,
    fid: &FactionId,
    to: Option<&FactionId>,
    value: f64,
) {
    if value <= 1e-9 {
        return;
    }
    let keys: Vec<String> = remaining
        .keys()
        .filter(|(s, _)| s == fid)
        .map(|(_, rt)| rt.clone())
        .collect();
    let mut left = value;
    for rt in keys {
        if left <= 1e-9 {
            break;
        }
        let avail = remaining.get(&(fid.clone(), rt.clone())).copied().unwrap_or(0.0);
        if avail <= 1e-9 {
            continue;
        }
        let p = price.get(&rt).copied().unwrap_or_else(|| value_of(&rt));
        if p <= 1e-9 {
            continue;
        }
        let units = (left / p).min(avail);
        if units <= 1e-9 {
            continue;
        }
        *remaining.entry((fid.clone(), rt.clone())).or_insert(0.0) -= units;
        if let Some(f) = state.faction_mut(fid) {
            let e = f.resources.entry(rt.clone()).or_insert(0.0);
            *e = (*e - units).max(0.0);
        }
        if let Some(to) = to {
            if let Some(f) = state.faction_mut(to) {
                *f.resources.entry(rt.clone()).or_insert(0.0) += units;
            }
        }
        left -= units * p;
    }
}


// --- construction (dual budgets) ---------------------------------------------

/// 本回合按资源汇总的挂单量（供给侧观察）。
pub fn offered_by_resource(market: &MarketState) -> ResourceMap {
    let mut out: ResourceMap = ResourceMap::new();
    for o in &market.offers {
        *out.entry(o.resource.clone()).or_insert(0.0) += o.amount;
    }
    out
}

