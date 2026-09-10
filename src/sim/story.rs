//! 剧情编年史：数据驱动的叙事弧、触发条件与参与者。

use super::*;

// --- story / chronicle -------------------------------------------------------
/// 剧情步进：评估 config 的 `story` 表，把满足触发条件的剧情事件火出，写入
/// [`State::chronicle`] 编年史并记一条 [`GameEvent::Story`]，同时应用可选的小幅
/// 机械后果（关系/资源）。确定性：无 RNG，同一种子触发完全一致。
///
/// 每个事件默认只触发一次（id 已入编年史则跳过）。触发条件见 [`StoryTrigger`]；
/// 事件型条件（`FirstWar`/`FirstRaze`/`FirstColony`/`WarBetween`/`FactionAtWar`）
/// 依据本回合已产生的事件（含开战/停战/夷平/殖民）判定——因此这些剧情节拍正好落在
/// 对应历史事件发生的那个回合，形成「剧情与局势同步」的叙事弧。
pub fn step_story(state: &mut State, config: &GameConfig) {
    for spec in &config.story {
        // 每个剧情事件只触发一次：已进编年史则跳过。
        if state.chronicle.iter().any(|c| c.id == spec.id) {
            continue;
        }
        if !story_trigger_fired(state, &spec.trigger) {
            continue;
        }
        // 机械后果（小幅、确定性）。
        for effect in &spec.effects {
            match effect {
                StoryEffect::Relations { a, b, delta } => {
                    adjust_relation(state, config, a, b, *delta);
                }
                StoryEffect::GrantResources { faction, resource, amount } => {
                    if let Some(f) = state.faction_mut(faction) {
                        *f.resources.entry(resource.clone()).or_insert(0.0) += *amount;
                    }
                }
                StoryEffect::GrantShip { faction, class, body } => {
                    grant_story_ship(state, config, faction.clone(), class, body.clone());
                }
            }
        }
        // 记入编年史 + 本回合故事事件。参与方由静态模板 + 本次事件的具体对象合成
        // （事件型触发把「实际是谁」写进编年史，让剧情真正反应该回合发生的事情）。
        let participants = story_participants(state, spec);
        let entry = ChronicleEntry {
            round: state.round,
            id: spec.id.clone(),
            title: spec.title.clone(),
            body: spec.body.clone(),
            participants: participants.clone(),
        };
        ev(state, GameEvent::Story { id: spec.id.clone(), title: spec.title.clone(), participants });
        state.chronicle.push(entry);
    }
}

/// 剧情事件的参与方：模板里写的静态可读名，加上事件型触发从本回合事件里提炼出的
/// 具体对象（哪两方开战 / 哪座城被夷平 / 谁建立了殖民地）。保证编年史「自描述」——
/// agent 无需反推就能知道这条剧情发生在谁身上。确定性：取自本回合事件流水。
pub fn story_participants(state: &State, spec: &StoryEvent) -> Vec<String> {
    let mut parts: Vec<String> = spec.participants.clone();
    let add = |parts: &mut Vec<String>, name: Option<String>| {
        if let Some(n) = name {
            if !n.is_empty() && !parts.iter().any(|p| p == &n) {
                parts.push(n);
            }
        }
    };
    let find_war = |state: &State, faction: Option<FactionId>| -> Option<(FactionId, FactionId)> {
        state.events.iter().find_map(|e| match e {
            GameEvent::WarStarted { a, b } => match &faction {
                Some(f) if a == f || b == f => Some((a.clone(), b.clone())),
                Some(_) => None,
                None => Some((a.clone(), b.clone())),
            },
            _ => None,
        })
    };
    match &spec.trigger {
        StoryTrigger::FirstWar => {
            if let Some((a, b)) = find_war(state, None) {
                add(&mut parts, state.faction(&a).map(|f| f.name.clone()));
                add(&mut parts, state.faction(&b).map(|f| f.name.clone()));
            }
        }
        StoryTrigger::FactionAtWar { faction } => {
            add(&mut parts, state.faction(faction).map(|f| f.name.clone()));
            if let Some((a, b)) = find_war(state, Some(faction.clone())) {
                let other = if a == *faction { b } else { a };
                add(&mut parts, state.faction(&other).map(|f| f.name.clone()));
            }
        }
        StoryTrigger::FirstRaze => {
            if let Some((city, fallen)) = state.events.iter().find_map(|e| match e {
                GameEvent::CityRazed { city, fallen_to, .. } => Some((city.clone(), fallen_to.clone())),
                _ => None,
            }) {
                add(&mut parts, state.city(&city).map(|c| c.name.clone()));
                add(&mut parts, state.faction(&fallen).map(|f| f.name.clone()));
            }
        }
        StoryTrigger::FirstColony => {
            if let Some((owner, body)) = state.events.iter().find_map(|e| match e {
                GameEvent::ColonyFounded { owner, body, .. } => Some((owner.clone(), body.clone())),
                _ => None,
            }) {
                add(&mut parts, state.faction(&owner).map(|f| f.name.clone()));
                add(&mut parts, state.body(&body).map(|b| b.name.clone()));
            }
        }
        StoryTrigger::WarBetween { a, b } => {
            add(&mut parts, state.faction(a).map(|f| f.name.clone()));
            add(&mut parts, state.faction(b).map(|f| f.name.clone()));
        }
        _ => {}
    }
    parts
}

/// 剧情：把一个舰级「出厂」给某势力，位置在天体当前位置附近（小幅确定性偏移）。
/// 舰 id 按当前最大 id 连续分配，/并配一条 `Idle` 指令；无 RNG，确定性复现。
pub fn grant_story_ship(state: &mut State, config: &GameConfig, faction: FactionId, class: &str, body: BodyId) {
    if !config.ships.contains_key(class) {
        return;
    }
    let pos = state.body_position(&body);
    if state.body(&body).is_none() {
        return;
    }
    // 剧情赠舰此前**完全不发事件**——一艘舰凭空出现。走 `spawn_ship` 漏斗补上，
    // 让它进可查的历史（`via = story` 与船坞出厂区分开）；赠舰不付组件成本
    // （是剧情送的），也没有出厂城（在天体附近下水）。
    spawn_ship(state, config, ShipSpawn {
        owner: faction,
        class,
        position: [pos[0] + 0.05, pos[1] + 0.05],
        city: None,
        via: SpawnVia::Story,
        pay_components: false,
        // 剧情赠舰**没有图**（它不是任何建造区印出来的）。
        blueprint: None,
    });
}

/// 判断一条剧情触发条件是否已满足。
pub fn story_trigger_fired(state: &State, trigger: &StoryTrigger) -> bool {
    match trigger {
        StoryTrigger::RoundAt { round } => state.round >= *round,
        StoryTrigger::FirstWar => state.events.iter().any(|e| matches!(e, GameEvent::WarStarted { .. })),
        StoryTrigger::FirstRaze => state.events.iter().any(|e| matches!(e, GameEvent::CityRazed { .. })),
        StoryTrigger::FirstColony => state.events.iter().any(|e| matches!(e, GameEvent::ColonyFounded { .. })),
        StoryTrigger::WarBetween { a, b } => state.events.iter().any(|e| match e {
            GameEvent::WarStarted { a: x, b: y } => {
                let (lo, hi) = (x.min(y), x.max(y));
                let (plo, phi) = (a.min(b), a.max(b));
                lo == plo && hi == phi
            }
            _ => false,
        }),
        StoryTrigger::FactionAtWar { faction } => state.events.iter().any(|e| match e {
            GameEvent::WarStarted { a, b } => *a == *faction || *b == *faction,
            _ => false,
        }),
        StoryTrigger::RelationBelow { a, b, value } => relation(state, a, b) < *value,
    }
}
