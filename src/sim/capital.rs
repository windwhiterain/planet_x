//! 首都：亡城强迁 + 周期性 AI 评估 + 迁都代价（贸易锚点/治理距离）。

use super::*;

/// 维护每个势力的「有效首都」的唯一事实来源（[`ControllableState::capital`]）。
///
/// 两个触发（都确定性、无 RNG）：
/// * **亡城强迁（硬规则，先于一切）**：只要有效首都天体上已无本势力的活城（被夷平或
///   被敌人殖民夺走），就把首都切到本势力**人口最高的活城**（并列取名字序）——不能让
///   首都钉在已死的天体上。即使 Player 设过首都也强迁（死首都无效）。
/// * **周期性 AI 评估**：非 Player 控制的首都（[`State::capital_control`] == Ai）每
///   [`GovernanceConfig::capital_review_every`] 回合重估一次：候选 = 人口最高的活城；
///   仅当它对全势力各城的「总治理距离成本」比当前首都低
///   [`GovernanceConfig::capital_relocate_threshold`] AU 以上时才迁（避免反复横跳）。
///
/// 放在 [`step_resurgence`] 之后：刚重建出立足点的势力也能当回合被认领一个新首都。
pub fn step_capital(state: &mut State, config: &GameConfig, flow: &mut RoundSink) {
    let faction_ids: Vec<FactionId> = state.factions.iter().map(|f| f.name.clone()).collect();
    let review_every = config.governance.capital_review_every.max(1);

    for fid in faction_ids {
        let cur = state.capital_body(&fid);
        let living: Vec<CityId> = state
            .cities
            .iter()
            .filter(|c| c.faction_id == fid && !c.razed)
            .map(|c| c.name.clone())
            .collect();
        if living.is_empty() {
            continue; // 无活城：resurgence 会在后续回合重建，届时再定首都。
        }

        let cur_owned = living.iter().any(|cid| {
            state.city(cid).map(|c| c.body_id == cur).unwrap_or(false)
        });

        let mut new_cap: Option<BodyId> = None;
        let mut reason = "";

        if !cur_owned {
            // 亡城强迁 → 人口最高的活城（并列取名字序）。
            new_cap = Some(highest_pop_city_body(state, &fid));
            reason = "destroyed";
        } else if state.capital_control(&fid) == ControlMode::Auto && state.round % review_every == 0 {
            let best = highest_pop_city_body(state, &fid);
            // 候选与两笔成本**无论最后迁不迁都算出来并记下**：判据本身就是读面要回答的问题
            // （「为什么没迁」= 候选没便宜够 `capital_relocate_threshold`）。两个函数都只读
            // 城市位置与人口 ⇒ 纯追加、不改行为。
            let cur_cost = capital_anchor_cost(state, config, &fid, &cur);
            let best_cost = capital_anchor_cost(state, config, &fid, &best);
            flow.capital.insert(
                fid.clone(),
                CapitalFlow {
                    reviewed: true,
                    candidate: Some(best.clone()),
                    current_cost: Some(cur_cost),
                    candidate_cost: Some(best_cost),
                    ..Default::default()
                },
            );
            if best != cur && best_cost + config.governance.capital_relocate_threshold < cur_cost {
                new_cap = Some(best);
                reason = "ai_review";
            }
        }

        if let Some(nc) = new_cap {
            let from = cur.clone();
            // 迁都的全国忠诚度代价：旧首都人口占比 × 系数 = 每座城忠诚下降。占比越高
            // 迁离越动荡（国本动摇）；亡城强迁时旧首都已失（占比=0）→ 应急无忠诚代价。
            let old_share = faction_capital_share(state, &fid);
            let loyalty_cost = old_share * config.governance.capital_share_relocate_cost;
            // 这次迁都本身与它的忠诚代价也进读面。亡城强迁那条路**没有评估**（`reviewed` 仍是
            // false、没有候选与成本），但「从哪迁到哪、付了多少忠诚」同样要看得见。
            let entry = flow.capital.entry(fid.clone()).or_default();
            entry.relocated_from = Some(from.clone());
            entry.relocated_to = Some(nc.clone());
            entry.relocate_loyalty_cost = loyalty_cost;
            // 保留原 mode 标记（Player 仍归玩家、Inherit 让作用域链决定）——迁都是换「值」，
            // 不改变「由谁决定」的层次化粒度。
            let prev_mode = state
                .control
                .get(&fid)
                .and_then(|c| c.capital.as_ref())
                .map(|c| c.mode)
                .unwrap_or_default();
            {
                let ctrl = state.control.entry(fid.clone()).or_default();
                ctrl.capital = Some(Control { value: nc.clone(), mode: prev_mode });
            }
            if loyalty_cost > 0.0 {
                for cid in &living {
                    if let Some(c) = state.city_mut(cid) {
                        c.loyalty = (c.loyalty - loyalty_cost).max(0.0);
                    }
                }
            }
            ev(state, GameEvent::CapitalRelocated {
                faction: fid.clone(),
                from,
                to: nc,
                reason: reason.to_string(),
            });
        }
    }
}

/// 该势力**有效首都**的人口占其全势力活城人口的比例（0..1）。首都占全势力人口的比例
/// 越高 → 全国忠诚度 buff 越强；迁离占比高的首都 → 全国忠诚度代价越大。
pub fn faction_capital_share(state: &State, fid: &str) -> f64 {
    let cap = state.capital_body(fid);
    let mut cap_pop = 0u64;
    let mut total_pop = 0u64;
    for c in &state.cities {
        if c.faction_id != fid || c.razed {
            continue;
        }
        let p = c.population as u64;
        total_pop += p;
        if c.body_id == cap {
            cap_pop += p;
        }
    }
    if total_pop == 0 {
        0.0
    } else {
        cap_pop as f64 / total_pop as f64
    }
}

/// 本势力**人口最高的活城**之天体（并列取名字序，确定性）。调用方保证该势力有活城。
pub fn highest_pop_city_body(state: &State, fid: &str) -> BodyId {
    state
        .cities
        .iter()
        .filter(|c| c.faction_id == fid && !c.razed)
        .min_by_key(|c| (std::cmp::Reverse(c.population), c.name.clone()))
        .map(|c| c.body_id.clone())
        .expect("caller guarantees a living city")
}

/// 该势力若以 `cap` 为首都，其全部活城到它的「总治理距离成本」（AU）：
/// Σ max(0, dist(body, cap) - admin_range)。用作迁都「是否更优」的判据（越小越好）。
pub fn capital_anchor_cost(state: &State, config: &GameConfig, fid: &str, cap: &str) -> f64 {
    let g = &config.governance;
    let cap_pos = state.body_position(cap);
    let mut total = 0.0;
    for c in &state.cities {
        if c.faction_id != fid || c.razed {
            continue;
        }
        let d = dist(state.body_position(&c.body_id), cap_pos);
        total += (d - g.admin_range).max(0.0);
    }
    total
}
