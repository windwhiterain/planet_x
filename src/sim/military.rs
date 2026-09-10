//! 军事回合：舰队行为与移动、战斗结算、轰炸、殖民。

use super::*;

pub fn step_military(state: &mut State, config: &GameConfig, rng: &mut Prng, flow: &mut RoundFlow) {
    // 进入本步进时**还活着**的舰：漏斗兜底的断言只对它们成立（见 `sweep_dead_ships`）。
    let alive_at_step_start: BTreeSet<ShipId> = state
        .ships
        .iter()
        .filter(|s| s.hull > 0.0)
        .map(|s| s.name.clone())
        .collect();
    let mut order: Vec<ShipId> = state.ships.iter().map(|s| s.name.clone()).collect();
    for i in (1..order.len()).rev() {
        let j = rng.range(i as u64 + 1) as usize;
        order.swap(i, j);
    }

    let mut next_building_id = state
        .cities
        .iter()
        .flat_map(|c| c.buildings.iter().map(|b| b.id))
        .max()
        .map_or(0, |m| m + 1);

    // 联盟军事协同的「集火目标」：每回合每个势力各算一次（O(势力 × 实体)，摊薄到全军）。
    // 结盟势力在霸权先动手时优先集火该霸权，而非各自就近乱打。
    let focus_of: BTreeMap<FactionId, Option<FactionId>> = state
        .factions
        .iter()
        .map(|f| (f.name.clone(), coalition_war_focus(state, config, &f.name)))
        .collect();

    // **集货定编**（本回合一次）：先把「谁是运输舰」定下来并写进第三条风格轴，再逐舰执行。
    // 放在这里而不是让每艘舰自己算，是为了**与舰的处理顺序无关**——下面的 `order` 是按 rng
    // 打乱的：若逐舰现算，前几艘舰这一回合装的货会改掉后面舰的名额，结论就依赖抽到的顺序了。
    autocontrol::freight::assign_roles(state, config);

    for ship_id in order {
        let Some(ship) = state.ship(&ship_id) else { continue };
        if ship.hull <= 0.0 {
            continue;
        }
        let owner = ship.faction_id.clone();
        let class = ship.class.clone();
        let pos = ship.position;

        let is_ai = state.ship_control(ship_id.clone()) == ControlMode::Auto;

        if !is_ai {
            // --- player-controlled: execute the commanded behavior literally ---
            let mut behavior = state.ship_behavior(ship_id.clone()).unwrap_or(ShipBehavior::Idle);
            // A stale Follow/DockCity/Colonize order (followed ship destroyed, target
            // city razed, or a body with no settlement) must not send the ship drifting
            // toward the origin ([0,0]); degrade it to Idle and record a StaleOrder
            // event so the agent knows to re-issue. Move/Idle are always valid.
            if !behavior_is_valid(state, config, behavior.clone(), &owner) {
                let reason = match behavior {
                    ShipBehavior::Follow { ship } => format!("followed ship {ship} gone"),
                    ShipBehavior::DockCity { city } => format!("target city {city} razed"),
                    ShipBehavior::Colonize { body } => format!("body {body} has no blank settlement"),
                    ShipBehavior::Haul { from, to } => format!("haul route {from} → {to} invalid"),
                    _ => "invalid".to_string(),
                };
                if let Some(c) = state.control_mut(owner.clone()) {
                    c.ship_orders.insert(ship_id.clone(), Control::player(ShipBehavior::Idle));
                }
                ev(state, GameEvent::StaleOrder { ship: ship_id.clone(), reason });
                behavior = ShipBehavior::Idle;
            }
            // 行为 = 纯移动/停泊指令：攻击与轰炸都不再是行为——射程内有敌舰/敌城即自动发生。
            // （待命 Idle = 原地保持「不移动」，但仍会落到下面的自动接战/轰炸。）
            match &behavior {
                ShipBehavior::Colonize { body } => {
                    let bpos = state.body_position(body);
                    if dist(pos, bpos) <= config.combat.arrival_eps {
                        colonize(state, config, rng, &ship_id, body, &mut next_building_id);
                        continue;
                    }
                    move_toward(state, config, &ship_id, &class, bpos);
                    let np = state.ship(&ship_id).map(|s| s.position).unwrap_or(pos);
                    if dist(np, bpos) <= config.combat.arrival_eps {
                        colonize(state, config, rng, &ship_id, body, &mut next_building_id);
                        continue;
                    }
                }
                ShipBehavior::Idle => {
                    // 待命：软目标——原地保持；但附近有敌舰时按 kiting 姿态自动软移动（风筝拉开/贴脸压近）。
                    let dest = autocontrol::kiting_dest(state, config, &ship_id).unwrap_or(pos);
                    move_toward(state, config, &ship_id, &class, dest);
                }
                // 运输：**整条路线本回合都在 `haul_step` 里执行**（择腿 + 移动 + 装卸），
                // 所以这里不再自己移动——落到下面的自动接战，运输舰在航线上照样开火/轰炸。
                ShipBehavior::Haul { from, to } => {
                    haul_step(state, config, &ship_id, &class, from, to);
                }
                _ => {
                    // Move / Follow / DockCity / Dock：驶向行为目的地（软目标）；附近有敌舰时由 kiting 姿态调整。
                    let base = behavior_dest(state, &behavior);
                    let dest = autocontrol::kiting_dest(state, config, &ship_id).unwrap_or(base);
                    move_toward(state, config, &ship_id, &class, dest);
                }
            }
            // --- 自动战斗：攻击与轰炸不需要行为（射程内自动发生）---
            autocontrol::auto_combat(state, config, &ship_id, &owner);
            continue;
        }

        // --- AI-controlled: the auto-control brain decides and executes ---
        autocontrol::ai_ship_turn(
            state,
            config,
            rng,
            &ship_id,
            &focus_of,
            &mut next_building_id,
            &mut flow.decisions.ships,
        );
        continue;
    }

    // **风格轴的执行者**（`Auto` 风格叶那一层的真写入者）：按**本回合的战况**重估
    // temper / lone_wolf / kiting 三条轴，并把结论写回逐舰叶（`Control::inherit` = 流水）。
    // 位置是刻意的：逐舰循环之后 ⇒ 本回合的接战/撤退/战沉都已发生（`events` 与 `decisions`
    // 是本回合的），而护甲再生之前 ⇒ 「被打残」这个信号还是新鲜的；写下的值从**下一回合**
    // 起生效 ⇒ 与舰的处理顺序无关（那个顺序是按 rng 打乱的），也不改变本回合任何判定。
    autocontrol::regulate_styles(state, config, &flow.decisions.ships, &mut flow.decisions.styles);

    // 护甲再生（%/时间）：每回合幸存舰只按舰级 hull_regen 恢复其最大护甲的一
    // 个比例（不消耗资源、不复活已毁舰）。处于本方本土防御半径内的舰获得额外
    // home_regen_bonus 再生（cult 的 MOND 异常使其圣所旁舰只极难被消耗）。
    // 先读出每艘舰的本土再生加成，再统一修改（避免与 ships 的可变借用冲突）。
    let (bonuses, friendly): (Vec<f64>, Vec<bool>) = state
        .ships
        .iter()
        .map(|s| {
            if s.hull > 0.0 {
                let dest = state.body_position(&state.capital_body(&s.faction_id));
                let f = state
                    .faction(&s.faction_id)
                    .map(|fac| dist(s.position, dest) <= fac.home_radius)
                    .unwrap_or(false);
                (home_regen_bonus(state, &s.faction_id, s.position), f)
            } else {
                (0.0, false)
            }
        })
        .unzip();
    let comp_repair = config.combat.component_repair;
    for (i, s) in state.ships.iter_mut().enumerate() {
        if s.hull > 0.0 {
            let panel = ship_panel(config, s);
            s.hull = (s.hull + panel.hull_max * (panel.hull_regen + bonuses[i])).min(panel.hull_max);
            // 能量护盾每回合再生（护盾组件）：护盾优先吸收、损毁后再生，是防御组件的关键。
            if panel.shield_max > 0.0 {
                s.shield = (s.shield + panel.shield_max * panel.shield_regen).min(panel.shield_max);
            }
            // 模块修复：受损组件在母港/友方本土修得更快（与自保撤退闭环：打残→撤→修→再来）。
            if comp_repair > 0.0 && !s.components.is_empty() {
                if s.component_hp.len() != s.components.len() {
                    s.component_hp = s.components.iter().map(|c| component_integrity(config, c)).collect();
                }
                let rate = comp_repair * if friendly[i] { 2.5 } else { 1.0 };
                for (j, c) in s.components.iter().enumerate() {
                    let max = component_integrity(config, c);
                    if s.component_hp[j] < max {
                        s.component_hp[j] = (s.component_hp[j] + max * rate).min(max);
                    }
                }
            }
        }
    }

    // 清扫本回合战沉的舰（漏斗**兜底**）：保证「从 state.ships 消失的舰都有死因事件」，
    // 并清掉它的指令。正常 0 艘需要兜底——不为 0 说明某条路径漏了 `kill_ship`，
    // `debug_assert` 会在测试里立刻炸出来。
    let invented = sweep_dead_ships(state, &alive_at_step_start);
    debug_assert_eq!(invented, 0, "有 {invented} 艘舰死亡却没有事件：某条路径漏了 kill_ship");
}

// --- 重建：只有一条路——派殖民舰去复垦 ---------------------------------------
//
// 这里原本是 `step_resurgence`（反僵尸重建）：一支势力一旦「无舰又无活城」，就**凭空**在自己的
// 残骸足迹 / 任何空白城 / 任何未占据定居点上重新立起一座城，甚至夺取城数最多者的活城，还白送
// 一艘种子舰。它保证了「回合末总有立足点」，但代价是**反科学**：城市凭空出现、货物凭空出现
// （补贴市场）、组件凭空装配——三处「免费午餐」互相掩护，于是「缺矿」既不更贵、也不致命。
//
// 已删除（D5）。现在世界只有一条重建路径，且它是物理的：
//   **造一艘殖民舰 → 把它开到一处空白定居点 → `colonize` 复垦。**
// 见 `ShipBehavior::Colonize`（自动控制在「无仗可打」时就会就近挑一处被夷平的定居点）
// 与 `sim::colonize`（`reseed_city` / `found_city` 两个漏斗）。
//
// 后果（有意接受的）：一支**既无舰又无活城**的势力确实再也回不来了——那是亡国，是合法的
// 结局，而不是需要被掩盖的状态。守卫因此改判为「亡国不许滚雪球」+「有舰的流亡势力必须
// 自己复垦回来」，见 `tests/longhorizon.rs`。

// --- governance (light-speed management) -------------------------------------

// --- 思潮优势端自平衡 debuff（平滑、无硬阈值断点） ------------------------

/// 确定性命中率：武器追踪能力 `tracking`（AU/月）越高，越能咬住高速目标。目标速度
/// `target_speed` 越高，对低追踪武器的规避越强——所以推进组件 = 生存能力和抢先战位。
/// 无 RNG：命中定义为「伤害折减」而非「命中/未命中」的随机判定，保持确定性。
pub fn hit_factor(tracking: f64, target_speed: f64) -> f64 {
    if tracking <= 0.0 {
        return 0.3;
    }
    let evade = (target_speed / (tracking + target_speed)).min(1.0) * 0.6;
    (1.0 - evade).clamp(0.2, 1.0)
}

/// 开火：本舰的每件武器**独立索敌**、逐发射击（`fire_rate` 发/时间）。`plan` 给出一发
/// 打向哪个目标（每发条目 = 武器下标）。每发独立结算（命中 × 防御 × 护盾/护甲），一回合
/// 内多发可打在**不同**目标上（火力分配）。伤害按目标**聚合**——每个目标累计伤害 > 0 才发
/// 一次 `Attack` 事件、调一次关系（避免逐发打、关系掉得过快）。本舰攻击某目标后把它在该舰
/// `attack_hist` 里的新鲜度刷到 1（供火力分配层读，跨回合记忆）。确定性（无 RNG）。
pub fn fire(state: &mut State, config: &GameConfig, attacker_id: &str, plan: &[(usize, ShipId)]) {
    let afac = state.ship(attacker_id).map(|a| a.faction_id.clone()).unwrap_or_default();
    let weapons = state.ship(attacker_id).map(|a| ship_weapons(config, a)).unwrap_or_default();
    let mut damage_acc: BTreeMap<ShipId, f64> = BTreeMap::new();
    // 被击毁的舰 + **补刀的那一发**（哪艘舰/哪个势力/什么弹种）。此前只记「被毁」不记凶手，
    // 只能靠同回合的 Attack 反推，集火时不可判。
    let mut destroyed: Vec<(ShipId, Killer)> = Vec::new();
    // 攻击历史新鲜度：本舰打过谁。逐发结算后刷新，使同回合后续发能按「越近越降权重」改选。
    let mut hist: BTreeMap<ShipId, f64> = BTreeMap::new();

    for (wi, target_id) in plan {
        let Some(w) = weapons.get(*wi) else { continue };
        if !state.ship(target_id).map(|s| s.hull > 0.0).unwrap_or(false) {
            continue; // 目标已被本回合其它发击毁——跳过这发（火力补刀不浪费）。
        }
        let dmg = resolve_shot(state, config, attacker_id, w, target_id);
        let dead = state.ship(target_id).map(|s| s.hull <= 0.0).unwrap_or(false);
        // 第一发把它打到 hull ≤ 0 的就是**补刀**（已在 0 的目标在循环开头被跳过，
        // 所以这里判真一定是本发致命）。记下这一发的来源作为凶手。
        if dead && !destroyed.iter().any(|(n, _)| n == target_id) {
            destroyed.push((target_id.clone(), Killer {
                ship: attacker_id.to_string(),
                faction: afac.clone(),
                weapon: weapon_kind_name(w.kind).to_string(),
            }));
        }
        // 本发是「攻击」：把该目标新鲜度刷到 1（哪怕全被护盾/拦截吃掉）。
        hist.entry(target_id.clone()).or_insert(0.0);
        *hist.get_mut(target_id).unwrap() = 1.0;
        *damage_acc.entry(target_id.clone()).or_insert(0.0) += dmg;
    }
    // 攻击历史写回（火力分配跨回合记忆）。
    if let Some(a) = state.ship_mut(attacker_id) {
        for (t, v) in hist {
            a.attack_hist.insert(t, v);
        }
    }
    for (t, dmg) in &damage_acc {
        if *dmg > 1e-9 {
            let tfac = state.ship(t).map(|s| s.faction_id.clone()).unwrap_or_default();
            ev(state, GameEvent::Attack { attacker: attacker_id.to_string(), target: t.clone(), damage: *dmg });
            adjust_relation(state, config, &afac, &tfac, config.diplomacy.attack_delta);
        }
    }
    for (t, by) in destroyed {
        kill_ship(state, &t, DeathCause::Combat, Some(by));
    }
}

/// 把本舰所有武器的一次齐射（每武器 `fire_rate` 发）全打向**一个**目标——集中火力的
/// 便捷入口（测试用；玩家/ AI 都走 [`fire`] 的统一基本权重 plan）。保留为引擎原语。
#[allow(dead_code)]
pub fn fire_concentrate(state: &mut State, config: &GameConfig, attacker_id: &str, target_id: &str) {
    let weapons = state.ship(attacker_id).map(|a| ship_weapons(config, a)).unwrap_or_default();
    let plan: Vec<(usize, ShipId)> = weapons
        .iter()
        .enumerate()
        .flat_map(|(i, w)| {
            let shots = (w.fire_rate.round()).max(1.0) as usize;
            (0..shots).map(move |_| (i, target_id.to_string()))
        })
        .collect();
    fire(state, config, attacker_id, &plan);
}

/// 结算单件武器的一发打击：对一个活目标应用「命中 × 本土防御 × 护盾/护甲」伤害并修改
/// 目标的 hull/shield/组件完整度，返回造成的伤害（护盾吸收 + 船体）。这是「每发独立结算」，
/// 让一回合内多发可打在**不同**目标上。确定性。
pub fn resolve_shot(state: &mut State, config: &GameConfig, attacker_id: &str, w: &Weapon, target_id: &str) -> f64 {
    let apos = state.ship(attacker_id).map(|a| a.position).unwrap_or([0.0, 0.0]);
    let (tfac, tpos, tspeed, tpanel) = {
        let t = state.ship(target_id).expect("target gone");
        let panel = ship_panel(config, t);
        (t.faction_id.clone(), t.position, panel.speed, panel)
    };
    let d = dist(apos, tpos);
    if d > w.range {
        return 0.0; // weapon out of range — positional, not a stat
    }
    let def_mult = home_defense_mult(state, &tfac, tpos);
    let hit = hit_factor(w.tracking, tspeed);
    let mut dmg = w.damage * hit * def_mult;
    // 导弹是制导的（对高速目标规避弱），但会被目标点防御**线性**拦截（自身 + 附近友舰。
    if w.kind == WEAPON_MISSILE {
        let pd = tpanel.intercept + cluster_pd_cover(state, config, target_id, &tfac, tpos);
        if pd > 0.0 {
            dmg = (dmg - pd).max(0.0);
        }
    }
    let (mut hull, mut shield) = {
        let t = state.ship(target_id).unwrap();
        (t.hull, t.shield)
    };
    let hull_before = hull;
    // 护盾优先吸收（按 shield_mult），溢出与 hull_mult 部分进船体；满护盾削弱船体伤害。
    let shield_dmg = dmg * w.shield_mult;
    let hull_dmg = dmg * w.hull_mult;
    let absorbed = shield.min(shield_dmg);
    shield -= absorbed;
    let soak = if shield_dmg > 1e-9 { absorbed / shield_dmg } else { 1.0 };
    // 护甲 = 让船体变硬：打向船体(护甲)的伤害被目标硬度按**反比例函数**削减。
    let hardness = tpanel.hardness;
    let hull_dmg_effective = hull_dmg * (1.0 - 0.5 * soak);
    let armor_soak = if hardness > 1e-9 {
        (hardness / (hardness + hull_dmg_effective.max(1e-9))).min(0.85)
    } else {
        0.0
    };
    let hull_pen = hull_dmg_effective * (1.0 - armor_soak);
    hull -= hull_pen;
    let destroyed = hull <= 0.0;
    let hull_damage_done = (hull_before - hull.max(0.0)).max(0.0);
    if let Some(t) = state.ship_mut(target_id) {
        t.hull = if destroyed { 0.0 } else { hull.max(0.0) };
        t.shield = shield.max(0.0);
        // 组件被击中后渐进丧失战力（武器被打掉、护盾被打掉），而不是满血抗到壳破。
        if !destroyed && !t.components.is_empty() && config.combat.component_spill > 0.0 {
            let spill = hull_damage_done * config.combat.component_spill;
            if spill > 1e-9 {
                if t.component_hp.len() != t.components.len() {
                    t.component_hp = t.components.iter().map(|c| component_integrity(config, c)).collect();
                }
                let mut rem = spill;
                let mut order: Vec<usize> = (0..t.components.len()).collect();
                order.sort_by(|&a, &b| t.component_hp[a].total_cmp(&t.component_hp[b]).then_with(|| a.cmp(&b)));
                for &i in &order {
                    if rem <= 1e-9 {
                        break;
                    }
                    if t.component_hp[i] <= 0.0 {
                        continue;
                    }
                    let take = rem.min(t.component_hp[i]);
                    t.component_hp[i] -= take;
                    rem -= take;
                }
            }
        }
    }
    dmg // 本次造成的总伤害（护盾 + 船体）；用于事件/关系/攻击历史。
}

/// 舰队防空（防空屏护）：目标（`target_faction` 阵营、`tpos` 处）附近 `pd_radius` 内的友舰，
/// 其点防御拦截能力会为它**替拦导弹**——随距离线性衰减、封顶。让有 PD 的舰组成防空圈，
/// 能护卫航母/友舰（与护航行为衔接：护航舰贴近旗舰时提供防空）。确定性（无 RNG）。
pub fn cluster_pd_cover(state: &State, config: &GameConfig, target_id: &str, target_faction: &str, tpos: [f64; 2]) -> f64 {
    let r = config.combat.pd_radius;
    if r <= 0.0 {
        return 0.0;
    }
    let mut cover = 0.0;
    for s in &state.ships {
        if s.name == target_id || s.faction_id != target_faction || s.hull <= 0.0 {
            continue;
        }
        let d = dist(s.position, tpos);
        if d > r {
            continue;
        }
        let p = ship_panel(config, s).intercept;
        if p > 0.0 {
            cover += p * (1.0 - d / r);
        }
    }
    cover.min(30.0)
}

/// 本土防御伤害倍率：`pos` 位于 `faction` 首都的 `home_radius` 之内时返回该势力的
/// `home_attack_mult`（<1 = 削弱入侵者），否则 1.0（无削弱）。
///
/// 这使得每个有首都的势力在自己的核心区难啃（超大国空降别人家里要付代价），而
/// cult 因 MOND 异常拥有超大半径/强削减，能够在被围攻的柯伊伯带圣所自保。
pub fn home_defense_mult(state: &State, faction: &str, pos: [f64; 2]) -> f64 {
    let Some(f) = state.faction(faction) else { return 1.0 };
    if f.home_radius <= 0.0 {
        return 1.0;
    }
    let cap = state.body_position(&state.capital_body(faction));
    if dist(pos, cap) <= f.home_radius {
        f.home_attack_mult
    } else {
        1.0
    }
}

/// 本土防御额外再生：`pos` 位于 `faction` 首都的 `home_radius` 之内时，返回该势力
/// 的 `home_regen_bonus`，否则 0.0。
pub fn home_regen_bonus(state: &State, faction: &str, pos: [f64; 2]) -> f64 {
    let Some(f) = state.faction(faction) else { return 0.0 };
    if f.home_radius <= 0.0 {
        return 0.0;
    }
    let cap = state.body_position(&state.capital_body(faction));
    if dist(pos, cap) <= f.home_radius {
        f.home_regen_bonus
    } else {
        0.0
    }
}

/// Bombard a city: damage is spread across its buildings by area share. When all
/// buildings are destroyed the city is razed to a blank (colonizable) settlement
/// — it is never captured.
pub fn bombard_city(state: &mut State, config: &GameConfig, ship_id: &str, cid: &str) {
    let (attacker, weapons) = {
        let s = state.ship(ship_id).expect("ship gone");
        (s.faction_id.clone(), ship_weapons(config, s))
    };
    let (old_owner, cpos) = {
        let c = state.city(cid).expect("city gone");
        (c.faction_id.clone(), state.body_position(&c.body_id))
    };
    // 城市是静止的大型目标（无护盾、只有建筑装甲），轰炸用「每件武器 × 对甲倍率」的总
    // 齐射——导弹/重炮拆城，近防炮对城伤害低。命中视为全中（城市不规避）。
    let hull_attack: f64 = weapons.iter().map(|w| w.damage * w.hull_mult).sum();
    // 本土防御（首都即强弩 + cult 的 MOND 异常）：城市位于其势力首都的本土防御
    // 半径内时，受到的轰炸伤害被削弱。
    let dmg = hull_attack * home_defense_mult(state, &old_owner, cpos);
    let razed = {
        let c = state.city_mut(cid).expect("city gone");
        let total_deployed: f64 = c.buildings.iter().map(|b| b.deployed).sum();
        for b in &mut c.buildings {
            let share = if total_deployed > 1e-9 { (b.deployed / total_deployed).min(1.0) } else { 0.0 };
            b.armor -= dmg * share;
        }
        c.buildings.retain(|b| b.armor > 1e-6);
        // 建筑清零 = 城失守；**状态改写交给 `raze_city` 漏斗**（它自己读夷平前人口并记事件）。
        c.buildings.is_empty()
    };
    adjust_relation(state, config, &attacker, &old_owner, config.diplomacy.attack_delta);
    ev(state, GameEvent::Siege { attacker: ship_id.to_string(), city: cid.to_string(), damage: dmg });
    if razed {
        adjust_relation(state, config, &attacker, &old_owner, config.diplomacy.capture_delta);
        // 漏斗记 `CityRazed`：`by_ship` = 拆掉这座城的那艘舰（此前只能去同回合的 Siege 里猜），
        // `pop_before`/`damage` = 这次毁灭的量级。
        raze_city(state, &cid.to_string(), RazeCause::Bombardment {
            by_ship: ship_id.to_string(),
            by_faction: attacker,
            damage: dmg,
        });
    }
}

/// Colonize a settlement site (定居点 ↔ 城市 1:1). If the body hosts a razed
/// (blank) city, re-seed it on its own settlement (re-colonize); otherwise found
/// a new city only on a 定居点 that no city occupies yet. A site already holding
/// a live city can never take a second one — the colony ship is spent (order
/// reset to idle) once a site is found, or stays put otherwise.
pub fn colonize(
    state: &mut State,
    config: &GameConfig,
    rng: &mut Prng,
    ship_id: &str,
    body: &str,
    next_building_id: &mut BuildingId,
) {
    let faction = state.ship(ship_id).map(|s| s.faction_id.clone()).expect("ship gone");
    // **机制不变量（反凭空造城）**：建城必须由**一艘此刻就在场的活舰**解释。这是
    // `step_resurgence` 删除（D5）之后世界唯一的建城路径——调用方只在舰已进入
    // `combat.arrival_eps` 时才调这里（见 `autocontrol::tactics`）。若将来有人再加一条
    // 「凭空变城」的路径，这两条断言会在任何 debug 测试里立刻炸掉。
    debug_assert!(
        state.ship(ship_id).map(|s| s.hull > 0.0).unwrap_or(false),
        "colonize 必须由一艘活着的舰触发（没有凭空变城）"
    );
    debug_assert!(
        state
            .ship(ship_id)
            .map(|s| dist(s.position, state.body_position(body)) <= config.combat.arrival_eps)
            .unwrap_or(false),
        "colonize 的舰必须已经在目标天体的 arrival_eps 之内"
    );
    let seeded_ship_class = autocontrol::choose_next_class(state, &faction, config, rng);

    // 1) A razed (blank) city keeps occupying its settlement: re-seed it there.
    let razed_cid = state.cities.iter().find(|c| c.body_id == body && c.razed).map(|c| c.name.clone());
    if let Some(cid) = razed_cid {
        // 漏斗：复垦空白城 + 记 `ColonyFounded { how: Refounded }`。旧主（空白城保留的
        // diaspora claim）由漏斗自己读，调用方漏不掉。
        if !reseed_city(state, config, &cid, &faction, &seeded_ship_class, next_building_id) {
            // 该城没有可用的定居点 —— 殖民舰就地待命（旧行为）。
            reset_order_keep_mode(state, &faction, ship_id);
            return;
        }
        reset_order_keep_mode(state, &faction, ship_id);
        return;
    }

    // 2) No blank city: found a new city only on a settlement no city occupies.
    let occupied: BTreeSet<String> = state.cities.iter().filter(|c| c.body_id == body).map(|c| c.settlement.clone()).collect();
    let Some(settlement) = state
        .body(body)
        .and_then(|b| b.settlements.iter().find(|s| !occupied.contains(&s.name)).cloned())
    else {
        // Every settlement is occupied by a live city — nothing to colonize.
        reset_order_keep_mode(state, &faction, ship_id);
        return;
    };
    let base = if settlement.name.is_empty() {
        state.body(body).map(|b| b.name.clone()).unwrap_or_else(|| format!("#{body}"))
    } else {
        settlement.name.clone()
    };
    let cname = format!("{}-殖民城", base);
    // 漏斗：新建城 + 记 `ColonyFounded { how: NewSite }`。
    found_city(state, config, &cname, &body.to_string(), &settlement, &faction, &seeded_ship_class, next_building_id);
    reset_order_keep_mode(state, &faction, ship_id);
}
