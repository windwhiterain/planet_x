//! 治理与忠诚：光速治理成本、人均面积、忠诚度、思潮扣分、城市叛变。

use super::*;

/// 全星系**活城总人口**（各势力所有未夷平城市的人口之和）。
pub fn total_live_pop(state: &State) -> u64 {
    state
        .cities
        .iter()
        .filter(|c| !c.razed)
        .map(|c| c.population as u64)
        .sum()
}

/// 该势力的**军事实力占比**（舰队引擎数值之和 / 全星系舰队引擎数值之和），0..1。
pub fn faction_military_share(state: &State, config: &GameConfig, fid: &str) -> f64 {
    let total: f64 = state
        .ships
        .iter()
        .map(|s| ship_panel(config, s).hull_max)
        .sum();
    if total <= 1e-9 {
        return 0.0;
    }
    let mine: f64 = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid)
        .map(|s| ship_panel(config, s).hull_max)
        .sum();
    mine / total
}

/// 该势力**舰在 MOND 异常区的占比**：距太阳 > `mond.radius` 的舰数 / 该势力总舰数（0..1）。
/// 无舰 → 0（即完全没在探索异常区）。
pub fn faction_mond_ship_share(state: &State, config: &GameConfig, fid: &str) -> f64 {
    let r = config.mond.radius;
    let ships: Vec<&Ship> = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid && s.hull > 0.0)
        .collect();
    if ships.is_empty() {
        return 0.0;
    }
    let in_mond = ships
        .iter()
        .filter(|s| dist(s.position, [0.0, 0.0]) > r)
        .count() as f64;
    in_mond / ships.len() as f64
}

/// 该势力**活城人口占全星系比例**（0..1）。
pub fn faction_pop_share(state: &State, fid: &str, p_total: f64) -> f64 {
    let pop: u64 = state
        .cities
        .iter()
        .filter(|c| c.faction_id == fid && !c.razed)
        .map(|c| c.population as u64)
        .sum();
    if p_total <= 1e-9 {
        0.0
    } else {
        pop as f64 / p_total
    }
}

/// 该势力**殖民活动强度**（0..1）：正在执行 `Colonize`（殖民）行为的舰数，用 `smoothstep`
/// 连续成形（0 艘 → 0；≥2 艘 → 1），无突然跳变。无舰 → 0（完全不殖民）。
pub fn faction_colonizing(state: &State, fid: &str) -> f64 {
    let ships: Vec<&Ship> = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid && s.hull > 0.0)
        .collect();
    if ships.is_empty() {
        return 0.0;
    }
    let colonize = ships
        .iter()
        .filter(|s| {
            state
                .ship_behavior(s.name.clone())
                .map(|b| matches!(b, ShipBehavior::Colonize { .. }))
                .unwrap_or(false)
        })
        .count() as f64;
    smoothstep(0.0, 2.0, colonize)
}

/// 该势力**思潮优势端自平衡 debuff** —— 返回全国忠诚度惩罚（0..`max_loyalty_penalty`）。
///
/// 打击强度由**该势力自身在星系中的体量**（活城人口占比 `dom`）驱动——越大越该被打，因此
/// 随单极化持续、不随「行为一致化」消退。再乘上各轴「思潮 vs 行为不符」的连续度：
///
/// `penalty = max × clamp01(dom × Σ_轴 w_轴 × violate_轴)`：
///   * `dom` = 该势力活城人口 / 全星系活城人口的 `smoothstep(gate_lo..gate_hi)`；
///   * `violate_轴` = 该势力「优势端思潮 vs 行为不符」的连续度（见 [`IdeologyDebuffConfig`]）。
///
/// 全 C¹ 平滑、无硬阈值；只对优势端思潮本身生效（自指向，不误伤中立/对立端）。
pub fn ideology_loyalty_debuff(state: &State, config: &GameConfig, fid: &str, p_total: f64) -> f64 {
    let d = &config.ideology.debuff;
    let id = state.faction(fid).map(|f| f.ideology).unwrap_or_default();
    // 打击强度 = 该势力自身体量（单极化越坐大越该被打）。
    let dom = smoothstep(d.gate_lo, d.gate_hi, faction_pop_share(state, fid, p_total));

    // 轴1 和平↔军国，优势端=军国(+)：军国 且（不战争 且 低军事实力占比）。
    let w = war_strength(state, config, fid);
    let ml = faction_military_share(state, config, fid);
    let viol_mil = smoothstep(0.0, 1.0, id.peace_military)
        * (1.0 - w)
        * (1.0 - smoothstep(0.0, d.mil_share_ref, ml));
    // 轴2 科学↔技术，优势端=科学(−)：科学 且 舰在 MOND 区占比低。
    let ms = faction_mond_ship_share(state, config, fid);
    let viol_sci = smoothstep(0.0, 1.0, -id.science_tech) * (1.0 - smoothstep(0.0, 1.0, ms));
    // 轴3 人民↔精英，优势端=精英(+)：精英 且 人口占全星系比例高（体量由 dom 承担，这里只看精英度）。
    let viol_elite = smoothstep(0.0, 1.0, id.people_elite);
    // 轴4 自然↔殖民，优势端=殖民(+)：殖民 且 不殖民。
    let col = faction_colonizing(state, fid);
    let viol_col = smoothstep(0.0, 1.0, id.nature_colony) * (1.0 - smoothstep(0.0, 1.0, col));

    let raw = d.w_military * viol_mil
        + d.w_science * viol_sci
        + d.w_elite * viol_elite
        + d.w_colony * viol_col;
    d.max_loyalty_penalty * (dom * raw).clamp(0.0, 1.0)
}

/// 本回合各势力的「思潮优势端自平衡 debuff」忠诚度惩罚（0..`max_loyalty_penalty`）。
/// 纯观测（不推进世界），供 agent 视图与调参。与 `step_governance` 同源（同一公式）。
pub fn faction_ideology_debuffs(state: &State, config: &GameConfig) -> BTreeMap<FactionId, f64> {
    let p_total = total_live_pop(state) as f64;
    state
        .factions
        .iter()
        .map(|f| {
            (
                f.name.clone(),
                ideology_loyalty_debuff(state, config, &f.name, p_total),
            )
        })
        .collect()
}

/// 光速治理：每座城按其与统治势力首都的距离产生一笔治理开销（距离越远、管辖越难）。
/// 势力从库存按价值支付；付得起时城市忠诚度向距离目标恢复（远则低），付不起（欠费）
/// 时忠诚度暴跌。忠诚度跌破 [`GovernanceConfig::loyalty_revolt`] 即爆发离心叛乱，城市
/// 被夷平为空白（可再殖民）。这给超大帝国一个自然上限——既能管的领地有限，遥远的
/// 殖民地在治理失败时丢失，使世界在上千回合后保持多方参与。
pub fn step_governance(state: &mut State, config: &GameConfig, flow: &mut RoundSink) {
    let g = &config.governance;
    let faction_ids: Vec<FactionId> = state.factions.iter().map(|f| f.name.clone()).collect();
    let value_of = |rt: &str| config.resources.get(rt).map(|r| r.value).unwrap_or(1.0);

    // 思潮优势端自平衡 debuff 的输入：全星系活城人口（用于算「该势力体量」占比）。
    let p_total = total_live_pop(state) as f64;

    for fid in faction_ids {
        let capital = state.capital_body(&fid);
        let cap_pos = state.body_position(&capital);

        // 该势力所有活城 + 每城到首都的距离 + 每城想投入的娱乐/福利预算。
        let mut cities: Vec<(CityId, f64, f64)> = Vec::new(); // (name, distance, ent_budget)
        let mut total_pop = 0u64;
        for c in &state.cities {
            if c.faction_id != fid || c.razed {
                continue;
            }
            total_pop += c.population as u64;
            let d = dist(state.body_position(&c.body_id), cap_pos);
            let ent = city_loyalty_budget(state, config, fid.clone(), c.name.clone());
            cities.push((c.name.clone(), d, ent));
        }
        if cities.is_empty() {
            continue;
        }
        // 人口越多，管理能力越分散——人口超载放大远距离治理难度（与距离叠加）。
        let overload = (total_pop as f64 / g.population_capacity.max(1e-6) - 1.0).max(0.0);
        let scale = 1.0 + overload;
        let mut total_admin = 0.0;
        let mut ent_total = 0.0;
        for (_, d, ent) in &cities {
            let a = (d - g.admin_range).max(0.0);
            total_admin += (g.admin_base + g.admin_per_au * a) * scale;
            ent_total += ent;
        }
        let governance_total = (total_admin + ent_total) * sanction_cost_mult(state, config, &fid);

        // 用库存（按价值加权）支付治理 + 娱乐开销（与舰队维护同源）。覆盖率决定
        // 治理是否到位以及娱乐投入是否真正落地。
        let stock = state
            .faction(&fid)
            .map(|f| f.resources.clone())
            .unwrap_or_default();
        let total_value: f64 = stock.iter().map(|(k, v)| v * value_of(k)).sum();
        let pay = governance_total.min(total_value);
        if pay > 1e-9 {
            let ratio = (pay / total_value).min(1.0);
            if let Some(f) = state.faction_mut(&fid) {
                for (k, v) in stock.iter() {
                    let new = (*v - *v * ratio).max(0.0);
                    f.resources.insert(k.clone(), new);
                }
            }
        }
        let coverage = if governance_total > 1e-9 {
            (total_value / governance_total).min(1.0)
        } else {
            1.0
        };
        // 忠诚度向「距离目标 + 娱乐加成 + 首都人口占比 buff」恢复/下降，并标记叛乱。
        // 首都人口占全势力的比例越高，全国向心力越强（每城目标忠诚更高）。用占比：把
        // 首都放在人口中心有真实收益，而不是无脑堆绝对人口。
        let cap_bonus = faction_capital_share(state, &fid) * g.capital_share_loyalty_buff;
        // 思潮优势端自平衡 debuff：该势力若身处「垄断」的优势端思潮又「言行不符」，扣全国忠诚。
        let ideo_penalty = ideology_loyalty_debuff(state, config, &fid, p_total);
        // **B1 深空治理**（用户裁决：`tech-system.md` §10 七条 MOND 红利里**只做这一条**）：
        // 距离那条忠诚衰减 **× (1 − MOND 掌握度)** —— 掌握度 1.0（指哪打哪）的势力，
        // **深处不再因为「离首都太远」而离心**。
        //
        // 为什么挂在「距离」这一项上、而不是给个独立加成：光速治理这条机制的全部内容就是
        // 「指令从首都传到边陲要时间」；MOND 掌握的正是**在异常区里把坐标算准**这件事，
        // 所以它读起来是「同一个物理量的两个读数」，不是外挂的一层 buff。
        //
        // 为什么只动忠诚、不动开销：裁决的原话是「**不按距离付忠诚衰减**」。开销那一半
        // （`admin_per_au`）留在原处，于是这条红利买到的是**守得住**，不是**管得起**——
        // 付不出治理费时城市照样掉忠诚（覆盖率那条支路与距离无关）。想要「管得起」是
        // 另一个提案（§10 的 B2…B7 里没有它，留作未来）。
        //
        // 连续、无断崖：掌握度每涨一点，深处的离心压力就小一点（凡人 → 指哪打哪是渐变的）。
        let mond_distance_relief = 1.0 - mond_control(state, &fid);
        // 记录本回合治理流（step_governance 的「中间量」）：总开销 + 覆盖率 + 行政/娱乐拆分 +
        // 人口超载倍率 + 思潮惩罚。**写入点从上面挪到这里**，是因为后两项此刻才算出来——
        // 捕获的仍是同一批局部变量，只是等它们齐了再写（纯追加：不参与任何计算）。
        flow.governance.insert(
            fid.clone(),
            GovernanceFlow {
                total: governance_total,
                coverage,
                admin: total_admin,
                entertainment: ent_total,
                scale,
                ideology_penalty: ideo_penalty,
                capital_bonus: cap_bonus,
            },
        );
        let mut to_revolt = Vec::new();
        for (cid, d, ent) in &cities {
            let a = (d - g.loyalty_range).max(0.0);
            let target_base =
                (1.0 - g.loyalty_distance * a * scale * mond_distance_relief).clamp(0.0, 1.0);
            let ent_bonus = (ent * coverage) / g.entertainment_cost.max(1e-6);
            let target_eff = (target_base + ent_bonus + cap_bonus - ideo_penalty).clamp(0.0, 1.0);
            // 捕获这一城的忠诚目标值分项（纯追加）——「这座城的忠诚为什么在掉」的分解。
            // **只放逐城不同的项**：首都向心项与思潮惩罚按势力算一次，已经进了 `GovernanceFlow`
            // （→ `FactionRow`），在这里抄一遍就是「同一个数两个位置」。
            flow.city_loyalty.insert(
                cid.clone(),
                LoyaltyTarget {
                    distance: target_base,
                    entertainment: ent_bonus,
                    effective: target_eff,
                },
            );
            let cur = state.city(cid).map(|c| c.loyalty).unwrap_or(1.0);
            let new = if coverage >= 1.0 - 1e-6 {
                (cur + (target_eff - cur) * g.loyalty_recover).clamp(0.0, 1.0)
            } else {
                (cur - g.loyalty_penalty * (1.0 - coverage)).max(0.0)
            };
            if let Some(c) = state.city_mut(cid) {
                c.loyalty = new;
            }
            if new < g.loyalty_revolt {
                to_revolt.push(cid.clone());
            }
        }

        // 离心叛乱：低忠诚城市**改旗易帜**——不夷为荒地，而是倒戈到「思潮与旧主最对立」
        // 的势力（见 [`most_ideologically_distant_faction`]/[`defect_city`]）。这既让过度扩张
        // 的大帝国体量回落，又让旁观/小势力能接盘城市、成长为多极棋子。找不到可倒戈目标
        // （世界只剩一家）时兜底夷为空白（可再殖民，旧行为）。
        for cid in to_revolt {
            // 先读爆发时的忠诚度：下面两条分支都会把它改写（倒戈重置为 1.0、夷平清零）。
            let loyalty = state.city(&cid).map(|c| c.loyalty).unwrap_or(0.0);
            let target = most_ideologically_distant_faction(state, &fid).filter(|to| to != &fid);
            if let Some(to) = target {
                // 漏斗：改归属 + 记 `CityDefected`（在同一处，漏不掉）。
                defect_city(state, config, &cid, &fid, &to, loyalty);
            } else {
                // 漏斗：夷平为空白 + 记 `Revolt`。
                raze_city(
                    state,
                    &cid,
                    RazeCause::Revolt {
                        faction: fid.clone(),
                        loyalty,
                    },
                );
            }
        }
    }
}

// --- 离心「改旗易帜」(loyalty-driven defection) -----------------------------

/// 把一座城从 `from` 倒戈给 `to`（漏斗）：城市换主、居民重燃对新主的认同（忠诚重置为 1.0，
/// 不再立刻叛变）、人口/建筑/船坞/空间站全部保留（这是一次**改旗易帜**，不是夷平）。
/// 同时把旧主对该城建筑/娱乐预算的控制叶子迁到新主名下，使新主的 AI 确实能治理这座
/// 城；并对旧主↔新主施加「夺城」级的关系打击（倒戈在旧主眼中几近叛国）。
/// `loyalty` 是爆发时的忠诚度（换主后会被重置，故由调用方先读好传入）。
pub fn defect_city(
    state: &mut State,
    config: &GameConfig,
    city: &str,
    from: &str,
    to: &str,
    loyalty: f64,
) {
    // 先收集该城建筑 id（避免在可变借权时再读 state.city）。
    let building_ids: Vec<BuildingId> = state
        .city(city)
        .map(|c| c.buildings.iter().map(|b| b.id).collect())
        .unwrap_or_default();

    // 换主 + 忠诚重置。
    if let Some(c) = state.city_mut(city) {
        c.faction_id = to.to_string();
        c.loyalty = 1.0;
        c.razed = false;
        // **图纸不跟着城走**（实测逼出来的修复）：设计图是**势力设计库**里的东西
        // （[`ControllableState::blueprints`]），城换了主，新主的库里没有那些名字 ⇒ 建造区
        // 立刻变成**悬空指针**，而 `build_city` 明写「悬空 ⇒ 停产」、`autocontrol::blueprints`
        // 又明写「悬空是玩家/agent 删图造成的，AI 不替他收拾」——两条规则叠起来就是
        // **永久停产**：这座城从此再也造不出一艘船。
        //
        // 实测（seed 7 / 200 回合，全面改成「消耗只吃本地库存」之后）：两个幸存势力 20 个
        // 建造区里 **18 个悬空**（都是打下来的城），全世界造不出船 ⇒ 没有船 ⇒ 运不来料 ⇒
        // 造不出船，**0 舰的冻结态**。这不是那条「悬空」规则的原意（它防的是"玩家删了图却
        // 看起来像成功"），而是**易主**顺手带出来的。
        //
        // 清掉指针 ⇒ 新主下一回合由 [`crate::autocontrol`] 按自己的舰级重新建图，回到
        // 「无图 ⇒ 出厂现算」那条正式路径。**玩家自己删图**那种悬空照旧停产语义不变
        // （那条路不经过这里）。
        for b in &mut c.buildings {
            b.blueprint = None;
        }
    }

    // 控制转移：把旧主控制面里 keyed-by-(city, building) 的叶子搬到新主名下。
    let mut moved_invest: Vec<(InvestKey, Control<f64>)> = Vec::new();
    let mut moved_build: Vec<(BuildKey, Control<f64>)> = Vec::new();
    let mut moved_loyalty: Option<(CityId, Control<f64>)> = None;
    if let Some(o) = state.control_mut(from.to_string()) {
        for bid in &building_ids {
            let ikey = (city.to_string(), *bid);
            if let Some(v) = o.invest_weights.remove(&ikey) {
                moved_invest.push((ikey, v));
            }
            let bkey = (city.to_string(), *bid);
            if let Some(v) = o.build_weights.remove(&bkey) {
                moved_build.push((bkey, v));
            }
        }
        if let Some(v) = o.loyalty_budget.remove(city) {
            moved_loyalty = Some((city.to_string(), v));
        }
    }
    if let Some(n) = state.control_mut(to.to_string()) {
        for (k, v) in moved_invest {
            n.invest_weights.insert(k, v);
        }
        for (k, v) in moved_build {
            n.build_weights.insert(k, v);
        }
        if let Some((k, v)) = moved_loyalty {
            n.loyalty_budget.insert(k, v);
        }
    }

    // 外交：倒戈 = 夺城级的关系下压（旧主视新主为敌）。
    let cur = relation(state, from, to);
    set_relation_sym(
        state,
        from.to_string(),
        to.to_string(),
        cur + config.diplomacy.capture_delta,
        config,
    );
    // 漏斗负责记事件：改归属与记 `CityDefected` 在同一处，**忘记记在结构上不可能**。
    ev(
        state,
        GameEvent::CityDefected {
            city: city.to_string(),
            from: from.to_string(),
            to: to.to_string(),
            loyalty,
        },
    );
}

// --- 迁都 (capital relocation) ----------------------------------------------
