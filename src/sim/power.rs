//! 权力/霸权/联盟/制裁与均势外交（反制联盟是概率化的，不是硬阈值）。

use super::*;

/// 该势力**战争强度**（0..1）：与任何其他势力的最低关系相对交战阈值越深越贴近 1（平滑）。
/// 关系远在交战阈值之上 → 0（没在打仗）；跌到阈值之下越深 → 趋近 1（在交战）。
///
/// `pub(crate)`：`autocontrol::style` 用它当**风格重估的战况输入**（「在打」是热血/独狼/贴脸
/// 这三条轴共同的驱动力之一）——那与思潮 debuff 的用法同源，所以共用一份实现，不另写一份。
pub fn war_strength(state: &State, config: &GameConfig, fid: &str) -> f64 {
    let wt = config.combat.war_threshold;
    let mut worst: f64 = 0.0;
    for o in &state.factions {
        if o.name == fid {
            continue;
        }
        worst = worst.min(relation(state, fid, &o.name));
    }
    let h = (wt - worst).max(0.0);
    smoothstep(0.0, config.ideology.debuff.war_band, h)
}

/// 各势力**综合实力**（幂：`city_weight×城市份额 + fleet_weight×舰队份额`，未除以两权重之和）。
/// 这是“谁最强”的**单一权威**统计：`faction_power_share` 由它归一化而来，观测
/// （[`observe`] 的 `faction_power`）与游戏逻辑（`step_balance_of_power`/
/// `sanction_cost_mult`）都读同一份。城市份额 = 活城数/总活城数，舰队份额 = 舰艇引擎数值
/// 之和/总引擎数值之和。无活城且无舰时全 0。
pub fn faction_power(state: &State, config: &GameConfig) -> BTreeMap<FactionId, f64> {
    let b = &config.balance;
    let total_cities = state.cities.iter().filter(|c| !c.razed).count() as f64;
    let total_fleet: f64 = state.ships.iter().map(|s| ship_panel(config, s).hull_max).sum();
    let mut powers = BTreeMap::new();
    if total_cities <= 0.0 && total_fleet <= 0.0 {
        return state.factions.iter().map(|f| (f.name.clone(), 0.0)).collect();
    }
    for f in &state.factions {
        let cities = state.cities.iter().filter(|c| c.faction_id == f.name && !c.razed).count() as f64;
        let fleet: f64 = state
            .ships
            .iter()
            .filter(|s| s.faction_id == f.name)
            .map(|s| ship_panel(config, s).hull_max)
            .sum();
        let city_share = if total_cities > 0.0 { cities / total_cities } else { 0.0 };
        let fleet_share = if total_fleet > 0.0 { fleet / total_fleet } else { 0.0 };
        powers.insert(f.name.clone(), b.power_city_weight * city_share + b.power_fleet_weight * fleet_share);
    }
    powers
}

/// 综合实力占比：`power = faction_power / (city_weight + fleet_weight)`。
/// 两份额各自在 [0,1] 且对全势力求和为 1，故 power 也是合法的占比（0..1）。
pub fn faction_power_share(state: &State, config: &GameConfig) -> BTreeMap<FactionId, f64> {
    let b = &config.balance;
    let wp = b.power_city_weight + b.power_fleet_weight;
    if wp <= 0.0 {
        return state.factions.iter().map(|f| (f.name.clone(), 0.0)).collect();
    }
    faction_power(state, config)
        .into_iter()
        .map(|(k, v)| (k, v / wp))
        .collect()
}

/// 当前的反制联盟成员：非霸权势力中，对霸权的**疏远**达到 [`BalanceOfPowerConfig::coalition_estrange`]
/// （关系 ≤ 该值，即被遏制/疏远了霸权）、且彼此相互和平（互不交战）的一方。若 ≥
/// [`BalanceOfPowerConfig::min_members`] 即视为联盟成立。遏制是冷战式的——成员未必与
/// 霸权开战，但已脱离其影响、转而与弱国抱团。
pub fn coalition_of(state: &State, config: &GameConfig, hegemon: &str, members: &[FactionId]) -> Vec<FactionId> {
    let estrange = config.balance.coalition_estrange;
    let estranged: Vec<FactionId> = members
        .iter()
        .cloned()
        .filter(|m| relation(state, m, hegemon) <= estrange)
        .collect();
    estranged
        .iter()
        .cloned()
        .filter(|m| estranged.iter().all(|o| o == m || !hostile(state, config, m, o)))
        .collect()
}

/// 当前综合实力占比最高的「霸权」及其已倒向联盟的成员（关系 ≤ `coalition_estrange`）。
/// 只负责判定「谁是最强、谁在抱团」，实力占比未达 [`BalanceOfPowerConfig::hegemon_power`]
/// 时返回 `None`。
pub fn dominant_hegemon(state: &State, config: &GameConfig) -> Option<(FactionId, Vec<FactionId>)> {
    let b = &config.balance;
    if b.hegemon_power > 1.0 {
        return None;
    }
    let ids: Vec<FactionId> = state.factions.iter().map(|f| f.name.clone()).collect();
    if ids.len() < 2 {
        return None;
    }
    let powers = faction_power_share(state, config);
    let (hegemon, max_power) = powers
        .iter()
        .max_by(|x, y| x.1.partial_cmp(y.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(k, v)| (k.clone(), *v))
        .unwrap_or((String::new(), 0.0));
    if max_power < b.hegemon_power {
        return None;
    }
    let members: Vec<FactionId> = ids.iter().cloned().filter(|x| *x != hegemon).collect();
    let estranged = coalition_of(state, config, &hegemon, &members);
    Some((hegemon, estranged))
}

/// 当前一个活跃反制联盟（≥ [`BalanceOfPowerConfig::min_members`] 个疏远成员）针对的
/// 「霸权」；用于政治上报（`coalition` 字段）与联盟跃迁事件。
pub fn active_coalition_hegemon(state: &State, config: &GameConfig) -> Option<FactionId> {
    dominant_hegemon(state, config)
        .filter(|(_, m)| m.len() >= config.balance.min_members)
        .map(|(h, _)| h)
}

/// 经济制裁针对的「霸权」：只要势力**已称霸（实力占比达标）且至少有一个弱者倒向联盟**
/// 就实施封锁——不必等联盟完全成形。这使「人缘好但已坐大」的紧凑区域帝国也能被压缩
/// （否则它不招人恨就没人封锁它），且比 war 更拟真、不引发夷平/僵尸。
pub fn sanctioned_hegemon(state: &State, config: &GameConfig) -> Option<FactionId> {
    dominant_hegemon(state, config)
        .filter(|(_, m)| !m.is_empty())
        .map(|(h, _)| h)
}

/// 经济制裁的「治理代价」倍率：若 `fid` 正是被经济封锁的霸权，则其维持帝国
/// （行政 + 娱乐）的成本按 `sanction_cost_mult` 放大；否则 1.0（不碰别国）。这使被
/// 多国封锁的大国要花更多资源维持领地与治安——边缘殖民地更难养、更易离心。
pub fn sanction_cost_mult(state: &State, config: &GameConfig, fid: &str) -> f64 {
    if sanctioned_hegemon(state, config).as_deref() == Some(fid) {
        config.balance.sanction_cost_mult
    } else {
        1.0
    }
}

/// 联盟军事协同的「集火目标」：若 `owner` 属于针对霸权 H 的活跃反制联盟（已倒向联盟、
/// 关系 ≤ `coalition_estrange`），且 H 正与联盟内某一弱者交战（集体安全已触发——霸权
/// 先动手了），则返回 Some(H)。这使结盟势力的舰只**优先集火 H**、而非各自就近乱打——
/// 给「攻其一方、集体制衡」真正的军事牙齿。否则返回 None（不改变普通行为）。
pub fn coalition_war_focus(state: &State, config: &GameConfig, owner: &str) -> Option<FactionId> {
    let b = &config.balance;
    if b.hegemon_power > 1.0 {
        return None;
    }
    let Some(hegemon) = active_coalition_hegemon(state, config) else { return None };
    if owner == hegemon.as_str() {
        return None;
    }
    // 该弱者是否已倒向联盟（疏远霸权）。未倒向则不集火。
    if relation(state, owner, &hegemon) > b.coalition_estrange {
        return None;
    }
    // 霸权是否正与任一弱者交战（集体防御触发）——注意霸权自己对它与他人开战不作集火。
    let war_on = state
        .factions
        .iter()
        .any(|f| f.name != hegemon && hostile(state, config, &f.name, &hegemon));
    if war_on {
        Some(hegemon)
    } else {
        None
    }
}

/// 合纵连横 / 均势外交：当一方被判定为「霸权」时，其余较弱势力被共同威胁推向彼此——
/// 弱者-弱者向 [`BalanceOfPowerConfig::coalition_affinity`] 靠拢（合纵），弱者对霸权向
/// [`BalanceOfPowerConfig::hegemon_affinity`] 靠拢（均势/疏远）。霸权对任一弱者开战时，
/// 其余弱者对霸权关系骤降（集体安全）。全部确定性、无 RNG。
pub fn step_balance_of_power(state: &mut State, config: &GameConfig) {
    let b = &config.balance;
    if b.hegemon_power > 1.0 {
        return; // 关闭
    }
    let ids: Vec<FactionId> = state.factions.iter().map(|f| f.name.clone()).collect();
    if ids.len() < 2 {
        return;
    }

    // 找综合实力占比最高的「霸权」；未达阈值则不触发机制。
    let powers = faction_power_share(state, config);
    let (hegemon, max_power) = powers
        .iter()
        .max_by(|x, y| x.1.partial_cmp(y.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(k, v)| (k.clone(), *v))
        .unwrap_or((String::new(), 0.0));
    if max_power < b.hegemon_power {
        return;
    }

    // 威胁强度：霸权超越阈值越多，弱者靠拢得越急（scale ∈ [1, 2] 附近）。
    let dom = (max_power - b.hegemon_power).max(0.0);
    let scale = 1.0 + dom / (1.0 - b.hegemon_power).max(1e-9);

    let members: Vec<FactionId> = ids.iter().cloned().filter(|x| *x != hegemon).collect();
    if members.is_empty() {
        return;
    }

    // 步骤前后联盟成员、及与霸权交战成员（用于跃迁/集体安全判定）。
    let coalition_before = coalition_of(state, config, &hegemon, &members);
    let was_at_war: BTreeSet<FactionId> =
        members.iter().cloned().filter(|m| hostile(state, config, m, &hegemon)).collect();

    // 合纵：弱者-弱者相互靠拢（共同威胁把他们推向彼此）。
    for i in 0..members.len() {
        for j in (i + 1)..members.len() {
            let (m1, m2) = (members[i].clone(), members[j].clone());
            let rel = relation(state, &m1, &m2);
            let nv = rel + b.coalition_rate * scale * (b.coalition_affinity - rel);
            set_relation_sym(state, m1, m2, nv, config);
        }
    }

    // 均势：「冷处理/遏制」——弱者对霸权的关系向 hegemon_affinity 下压，但**只在它比
    // 该目标更暖时才往下压**，绝不自动把它推到交战阈值之下（不「无脑宣战」）。这模拟
    // 现实中的遏制：弱国不再争相讨好霸权、甚至疏远它，但井水不犯河水，真正的共同军事
    // 行动留给「集体安全」（霸权一旦动手打某弱者，其余弱者才群起而攻之）。
    for m in &members {
        let rel = relation(state, m, &hegemon);
        if rel > b.hegemon_affinity {
            let nv = rel + b.hegemon_rate * scale * (b.hegemon_affinity - rel);
            set_relation_sym(state, m.clone(), hegemon.clone(), nv, config);
        }
    }

    // 集体安全：任一弱者与霸权进入交战（本回合新跨入），其余尚未交战的弱者对霸权关系
    // 骤降——「攻其一方 = 与全体为敌」的防御协定：霸权一旦开打，弱者联盟群起而攻之。
    let now_at_war: BTreeSet<FactionId> =
        members.iter().cloned().filter(|m| hostile(state, config, m, &hegemon)).collect();
    if now_at_war.difference(&was_at_war).next().is_some() {
        for m in &members {
            if !now_at_war.contains(m) {
                let rel = relation(state, m, &hegemon);
                set_relation_sym(state, m.clone(), hegemon.clone(), rel + b.collective_defense_delta, config);
            }
        }
    }

    // 联盟跃迁事件（只在成立/解体的当回合记一条，供 agent 直读政治格局）。
    let coalition_after = coalition_of(state, config, &hegemon, &members);
    let before_active = coalition_before.len() >= b.min_members;
    let after_active = coalition_after.len() >= b.min_members;
    if before_active && !after_active {
        ev(state, GameEvent::CoalitionEnded { hegemon, members: coalition_before });
    } else if !before_active && after_active {
        ev(state, GameEvent::CoalitionFormed { hegemon, members: coalition_after });
    }
}

// --- 思潮 (ideology) ----------------------------------------------------------
