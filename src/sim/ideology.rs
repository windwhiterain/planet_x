//! 思潮：相似度/距离、驱动量（战争得失 / MOND / 经济）与每回合演化。

use super::*;

/// 两方思潮的**相似度**（[0,1]）：`1 − 四轴平均 |Δ|/2`。每轴取 `[-1,1]`，故单轴归一化距离
/// 为 `|Δ|/2`（同极=0、对极=1），再对 [`Ideology`] 的 4 条轴取平均。相似度越高两方思潮越像。
/// 供外交静息亲和修正使用（思潮可变因子），与静态 alignment 叠加。
pub fn ideology_similarity(a: &Ideology, b: &Ideology) -> f64 {
    let dist = (0.5 * (a.peace_military - b.peace_military).abs()
        + 0.5 * (a.science_tech - b.science_tech).abs()
        + 0.5 * (a.people_elite - b.people_elite).abs()
        + 0.5 * (a.nature_colony - b.nature_colony).abs())
        / 4.0;
    (1.0 - dist).clamp(0.0, 1.0)
}

/// 两股思潮的**对立度**：4 条轴上的 L1 距离（`|Δ|` 之和），范围 [0,8]。
/// 越大代表两国的当代思潮越对立。
pub fn ideology_distance(a: &Ideology, b: &Ideology) -> f64 {
    (a.peace_military - b.peace_military).abs()
        + (a.science_tech - b.science_tech).abs()
        + (a.people_elite - b.people_elite).abs()
        + (a.nature_colony - b.nature_colony).abs()
}

/// 与 `owner` **思潮最对立**的势力（取其当前 [`Faction::ideology`]）：这是低忠诚城市
/// 「改旗易帜」的倒戈目标——居民不认同旧主的思潮，投向与其最对立的强权。确定性、无
/// RNG：距离最大者胜，并列取名字序最小。`owner` 自身排除；世界只剩一家时返回 `None`。
pub fn most_ideologically_distant_faction(state: &State, owner: &str) -> Option<FactionId> {
    let owner_ideo = state.faction(owner).map(|f| f.ideology)?;
    let mut best: Option<(f64, FactionId)> = None;
    for f in &state.factions {
        if f.name == owner {
            continue;
        }
        let d = ideology_distance(&owner_ideo, &f.ideology);
        let better = match &best {
            None => true,
            Some((bd, bn)) => d > *bd || (d == *bd && f.name < *bn),
        };
        if better {
            best = Some((d, f.name.clone()));
        }
    }
    best.map(|(_, n)| n)
}

/// 按「变化因素」驱动各势力 4 条思潮轴。每条轴先算**本回合的信号 target**（[-1,1]），
/// 再把当前值按 `drift_rate` 向 target 靠拢并钳到 [-1,1]。确定性、无 RNG。
///
/// 变化因素（见 spec「势力.各类思潮偏向」）：
///   * 和平↔军国：战争得失——敌舰被击毁+夷平敌城（得利→军国）减 我舰被击毁+城损失（失利→和平）。
///   * 科学↔技术：飞船在 MOND 异常区（→科学）vs 开采 MOND 区资源（→技术，按异常区城数计）。
///   * 人民↔精英：经济好坏——净流（产出−维护−治理）为正→精英，为负→人民。
///   * 自然↔殖民：人均面积——拥挤（低于参考）→殖民，宽敞→自然。
pub fn step_ideology(state: &mut State, config: &GameConfig, flow: &RoundSink) {
    let ic = &config.ideology;
    let r = config.mond.radius;
    // --- 军事信号：完全由**本回合的事件历史**推出，不再回读回合末的 state ---------------
    //
    // 这里以前有**两处「事后回读」**，都是错的——它们都在回合末去读一个回合内已经变过的世界，
    // 于是把「当时发生了什么」记到了「现在还剩什么」的头上：
    //
    // * **凶手**：曾用「同回合最后一条 `Attack` 的势力」近似。那要 `state.ship(attacker)`
    //   才知道攻击者属于谁，而**互杀**（凶手本回合也被打沉）时那艘舰已经不在 `state.ships`
    //   里 → 这次击杀**领不到功**。`ShipDestroyed.by` 是补刀那一刻记下的权威事实，不受影响。
    // * **失城方**：曾用 `state.city(city).faction_id` 判断「谁丢了这座城」。但夷平**不改归属**
    //   （空白城保留最后主人的 diaspora claim），而同一回合稍后的复垦/重建会把它改成新主——
    //   于是读到的是**新主**。活体样本 seed 7 r24：大红斑科学站被欧盟夷平、同回合被无国界
    //   科学组织复垦，这次战功被记到了**抢城的人**头上。`CityRazed.owner` 在夷平那一刻记下
    //   真正的失主。
    //
    // 口径（与模块文档声明一致）：「我丢了一城 / 沉了一舰 → −1；我夺了一城 / 击沉敌舰 → +1」。
    // 「一座活城易主」是一个**现象**、有两条实现分支（`city_defected` 主路 / `revolt` 兜底），
    // 因此二者必须同分——旧代码让兜底分支 −1 而主路 0 分，等于「分数取决于有没有可倒戈目标」
    // 这个无关的偶然。同理 `city_overrun` 也是「活城易主」，一并同分。
    // **`colony_founded` 刻意不计**：新建/复垦是**殖民**行为，归 `nature_colony` 轴管；把它记成
    // 军事得分会让殖民者集体漂向军国，两轴打架。
    let mil_delta = military_deltas(&state.events);
    let value_of = |rt: &str| config.resources.get(rt).map(|r| r.value).unwrap_or(1.0);

    // Pass 1（只读 state/flow）：算出每势力的信号 target。
    let mut targets: BTreeMap<FactionId, Ideology> = BTreeMap::new();
    for f in &state.factions {
        let name = f.name.clone();
        // 战争得失（见上：全部来自本回合事件，无 state 回读）。
        let mil = mil_delta.get(&name).copied().unwrap_or(0.0);
        // MOND 接触：到访异常区舰数（→科学） vs 开采异常区资源（→技术，按异常区城数计）。
        let mut sci_ships = 0u32;
        let mut tech_cities = 0u32;
        for s in &state.ships {
            if s.faction_id == name && s.hull > 0.0 && dist(s.position, [0.0, 0.0]) > r {
                sci_ships += 1;
            }
        }
        for c in &state.cities {
            if c.faction_id == name && !c.razed && dist(state.body_position(&c.body_id), [0.0, 0.0]) > r {
                tech_cities += 1;
            }
        }
        // 经济净流
        let prod: f64 = flow
            .faction_production
            .get(&name)
            .map(|m| m.iter().map(|(k, v)| v * value_of(k)).sum())
            .unwrap_or(0.0);
        let upkeep = flow.upkeep.get(&name).map(|u| u.total).unwrap_or(0.0);
        let gov = flow.governance.get(&name).map(|g| g.total).unwrap_or(0.0);
        let net = prod - upkeep - gov;
        // 人均面积（全部定居点面积 / 总人口）
        let area: f64 = state
            .cities
            .iter()
            .filter(|c| c.faction_id == name && !c.razed)
            .filter_map(|c| state.city_settlement(&c.name).map(|s| s.total_area))
            .sum();
        let pop: u64 = state
            .cities
            .iter()
            .filter(|c| c.faction_id == name && !c.razed)
            .map(|c| c.population as u64)
            .sum();
        let pca = if pop > 0 { area / pop as f64 } else { 0.0 };

        let t = Ideology {
            peace_military: (mil * ic.military_scale).clamp(-1.0, 1.0),
            science_tech: ((tech_cities as f64 - sci_ships as f64) * ic.mond_scale).clamp(-1.0, 1.0),
            people_elite: (net / ic.economy_scale.max(1e-6)).clamp(-1.0, 1.0),
            nature_colony: ((ic.area_ref - pca) * ic.area_scale).clamp(-1.0, 1.0),
        };
        targets.insert(name, t);
    }

    // Pass 2（可变 state）：把各势力思潮向 target 靠拢（确定性；钳 [-1,1]）。
    for f in &mut state.factions {
        let Some(target) = targets.get(&f.name) else { continue };
        let dr = ic.drift_rate;
        let converge = |cur: f64, tgt: f64| (cur + dr * (tgt - cur)).clamp(-1.0, 1.0);
        f.ideology.peace_military = converge(f.ideology.peace_military, target.peace_military);
        f.ideology.science_tech = converge(f.ideology.science_tech, target.science_tech);
        f.ideology.people_elite = converge(f.ideology.people_elite, target.people_elite);
        f.ideology.nature_colony = converge(f.ideology.nature_colony, target.nature_colony);
    }
}
