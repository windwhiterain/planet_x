//! 造舰决策的单元测试。

use super::*;
use crate::config::load_config;
use crate::prng::Prng;
use crate::world::default_state;
use std::collections::BTreeSet;

/// Build the config + a fresh deterministic world (round 0).
fn fresh_world(seed: u64) -> (GameConfig, State) {
    let config = load_config();
    let state = default_state(&config, seed);
    (config, state)
}

/// 造舰选装必须**确定**并且**总量可负担**（资源→组件 的确定性链接）。
#[test]
fn choose_loadout_is_deterministic_and_affordable() {
    let (config, mut state) = fresh_world(42);
    // Give China (3) a fat rare-mineral stack so it can afford a real loadout.
    if let Some(f) = state.faction_mut("中国") {
        for (r, amt) in [
            ("铀", 200.0),
            ("金", 200.0),
            ("氦-3", 200.0),
            ("铂", 200.0),
            ("氢", 200.0),
            ("钍", 200.0),
            ("铁", 200.0),
            ("碳", 200.0),
            ("硅", 200.0),
        ] {
            *f.resources.entry(r.to_string()).or_insert(0.0) += amt;
        }
    }
    let slots = config.ship_spec("battleship").slots as usize;
    let a = choose_loadout(&state, &config, "中国".to_string(), "battleship");
    let b = choose_loadout(&state, &config, "中国".to_string(), "battleship");
    assert_eq!(a, b, "loadout must be deterministic");
    assert!(a.len() <= slots, "must not exceed slot cap ({slots})");
    // The whole chosen set must be cumulatively affordable out of the stockpile.
    let mut pool = state.faction("中国").unwrap().resources.clone();
    for c in &a {
        for (r, amt) in &config.component_spec(c).cost {
            assert!(
                pool.get(r).copied().unwrap_or(0.0) >= *amt,
                "loadout {c} must be affordable for resource {r}"
            );
            *pool.entry(r.clone()).or_insert(0.0) -= *amt;
        }
    }
    // A resource-rich faction should fill more than a token slot.
    assert!(
        a.len() >= 2,
        "rich faction should field a real loadout, got {a:?}"
    );
}

/// 拟人指挥官：海军**混编**——一支富有的、近乎全护卫的势力，`choose_next_class` 会被
/// 「去重加分」拉去建其它舰型（不只堆护卫），形成更像真实海军的混编。
#[test]
fn choose_next_class_diversifies_toward_a_mix() {
    let (config, mut state) = fresh_world(42);
    if let Some(f) = state.faction_mut("中国") {
        for (r, amt) in [
            ("铀", 300.0),
            ("金", 300.0),
            ("氦-3", 300.0),
            ("铂", 300.0),
            ("氢", 300.0),
            ("钍", 300.0),
            ("铁", 300.0),
            ("碳", 300.0),
            ("硅", 300.0),
        ] {
            *f.resources.entry(r.to_string()).or_insert(0.0) += amt;
        }
    }
    // 强制这支势力的现役舰队全部是护卫舰——其余舰型因此「欠份额」，得到去重加分。
    for s in state.ships.iter_mut() {
        if s.faction_id == "中国" {
            s.class = "corvette".to_string();
        }
    }
    let mut rng = Prng::new(7);
    let mut got = BTreeSet::new();
    for _ in 0..60 {
        got.insert(choose_next_class(&state, "中国", &config, &mut rng));
    }
    assert!(
        got.len() >= 3,
        "an all-corvette navy should be pulled into a mix, got {got:?}"
    );
}

/// 威胁响应造舰（拟人「战时多造重舰」）：交战中，AI 会比和平时更倾向造重型战斗舰
/// （攻击力高的舰型加分），而不是只堆轻护卫。
#[test]
fn choose_next_class_builds_heavier_navy_at_war() {
    let (config, mut state) = fresh_world(42);
    if let Some(f) = state.faction_mut("中国") {
        for (r, amt) in [
            ("铀", 300.0),
            ("金", 300.0),
            ("氦-3", 300.0),
            ("铂", 300.0),
            ("氢", 300.0),
            ("钍", 300.0),
            ("铁", 300.0),
            ("碳", 300.0),
            ("硅", 300.0),
        ] {
            *f.resources.entry(r.to_string()).or_insert(0.0) += amt;
        }
    }
    // 舰队全部护卫舰，让去重加分对各舰型一视同仁。
    for s in state.ships.iter_mut() {
        if s.faction_id == "中国" {
            s.class = "corvette".to_string();
        }
    }
    let sample_heavy = |st: &State| -> usize {
        let mut rng = Prng::new(99);
        let mut heavy = 0;
        for _ in 0..240 {
            let c = choose_next_class(st, "中国", &config, &mut rng);
            if c == "battleship" || c == "carrier" {
                heavy += 1;
            }
        }
        heavy
    };
    let peace = sample_heavy(&state);
    state
        .faction_mut("中国")
        .unwrap()
        .relations
        .insert("美国".to_string(), -35.0);
    state
        .faction_mut("美国")
        .unwrap()
        .relations
        .insert("中国".to_string(), -35.0);
    let war = sample_heavy(&state);
    assert!(
        war > peace,
        "at war the AI should build more heavy hulls (war {war} > peace {peace})"
    );
}

/// 威胁响应（海军随威胁重构）：交战中，被单一舰型过度统治的势力会把一个船坞重定向到
/// 战局感知的新舰型（多造重舰），让威胁响应作用于整支舰队而不仅是新建舰厂。
#[test]
fn war_retools_over_abundant_shipyard_toward_a_war_class() {
    let (config, mut state) = fresh_world(42);
    let shipyard_types = |st: &State, f: FactionId| -> BTreeSet<(CityId, String)> {
        st.cities
            .iter()
            .filter(|c| c.faction_id == f)
            .flat_map(|c| {
                c.buildings
                    .iter()
                    .filter(|b| b.is_shipyard() && b.ship_type.is_some())
                    .map(|b| (c.name.clone(), b.ship_type.clone().unwrap()))
            })
            .collect()
    };
    // China (3) 舰队全护卫（过度单一），并让其与 US (1) 交战。
    for s in state.ships.iter_mut() {
        if s.faction_id == "中国" {
            s.class = "corvette".to_string();
        }
    }
    state
        .faction_mut("中国")
        .unwrap()
        .relations
        .insert("美国".to_string(), -35.0);
    state
        .faction_mut("美国")
        .unwrap()
        .relations
        .insert("中国".to_string(), -35.0);
    let before = shipyard_types(&state, "中国".to_string());
    let mut rng = Prng::new(7);
    let mut retools = Vec::new();
    retool_shipyards(&mut state, &config, "中国", &mut rng, &mut retools, &mut crate::model::RoundInputs::default());
    let after = shipyard_types(&state, "中国".to_string());
    assert!(
        after.iter().any(|(_, t)| t != "corvette"),
        "a corvette-dominated wartime fleet should retool a shipyard into a war class; before={before:?} after={after:?}"
    );
    // 改装决策必须**被记下来**（它不发事件，只有这里能留下"什么时候改成什么的"）。
    let rec = retools
        .iter()
        .find(|r| r.faction == "中国")
        .expect("改装要留一条判定");
    assert_eq!(rec.from, "corvette", "改装前后舰级要对得上：{rec:?}");
    assert_eq!(
        after
            .iter()
            .find(|(c, _)| *c == rec.city)
            .map(|(_, t)| t.clone()),
        Some(rec.to.clone()),
        "判定里记的新舰级必须就是状态里改成的那个：{rec:?}"
    );
}

/// 拟人指挥官：军舰选装要「又能打、又能扛」（…）；战局感知也在此测试。
#[test]
fn choose_loadout_is_balanced_and_threat_aware() {
    let (config, mut state) = fresh_world(42);
    if let Some(f) = state.faction_mut("中国") {
        for (r, amt) in [
            ("铀", 300.0),
            ("金", 300.0),
            ("氦-3", 300.0),
            ("铂", 300.0),
            ("氢", 300.0),
            ("钍", 300.0),
            ("铁", 300.0),
            ("碳", 300.0),
            ("硅", 300.0),
        ] {
            *f.resources.entry(r.to_string()).or_insert(0.0) += amt;
        }
    }
    // 和平：一艘巡洋舰（slot≥2）应至少各有一件武器与防御。
    let peace = choose_loadout(&state, &config, "中国".to_string(), "cruiser");
    assert!(
        peace
            .iter()
            .any(|c| config.component_spec(c).category == "weapon"),
        "a ship should field a weapon (got {peace:?})"
    );
    assert!(
        peace
            .iter()
            .any(|c| config.component_spec(c).category == "defense"),
        "a ship should field a defense (got {peace:?})"
    );
    // 开战：武器数不应比和平少（战时要火力的偏置）。
    state
        .faction_mut("中国")
        .unwrap()
        .relations
        .insert("美国".to_string(), -35.0);
    state
        .faction_mut("美国")
        .unwrap()
        .relations
        .insert("中国".to_string(), -35.0);
    let war = choose_loadout(&state, &config, "中国".to_string(), "cruiser");
    let peace_w = peace
        .iter()
        .filter(|c| config.component_spec(c).category == "weapon")
        .count();
    let war_w = war
        .iter()
        .filter(|c| config.component_spec(c).category == "weapon")
        .count();
    assert!(
        war_w >= peace_w,
        "at war the AI should field at least as many weapons (war {war_w} >= peace {peace_w}); war={war:?} peace={peace:?}"
    );
}

// ---- 设计图的归属 gate（spec §4.7：AI 重估不许绕过玩家的图）-------------------

/// 让中国进入「被单一舰型统治 + 交战」的状态，并把它的**前两座建造区**都设成 corvette。
///
/// 返回 `[(城名, 建筑下标); 2]`（按 `state.cities` 的顺序，与 `retool_shipyards` 的
/// 挑选顺序一致）。少于两座就直接 panic——用例需要「一个可以另选」的候选。
fn two_corvette_yards(state: &mut State) -> [(CityId, BuildingId); 2] {
    for s in state.ships.iter_mut() {
        if s.faction_id == "中国" {
            s.class = "corvette".to_string();
        }
    }
    state
        .faction_mut("中国")
        .unwrap()
        .relations
        .insert("美国".to_string(), -35.0);
    state
        .faction_mut("美国")
        .unwrap()
        .relations
        .insert("中国".to_string(), -35.0);
    let mut yards: Vec<(CityId, BuildingId)> = Vec::new();
    for c in state.cities.iter().filter(|c| c.faction_id == "中国") {
        for b in c.buildings.iter().filter(|b| b.is_shipyard()) {
            yards.push((c.name.clone(), b.id));
        }
    }
    assert!(
        yards.len() >= 2,
        "中国的建造区要 ≥2 座，实际 {}",
        yards.len()
    );
    let picked = [yards[0].clone(), yards[1].clone()];
    for (cid, bid) in &picked {
        if let Some(city) = state.city_mut(cid) {
            for b in city.buildings.iter_mut() {
                if b.id == *bid {
                    b.ship_type = Some("corvette".to_string());
                }
            }
        }
    }
    picked
}

/// **玩家钉住的图，AI 不许重估**（§7.2-5）：目标舰坞挂 `Player` 图 ⇒ 跳过它、另选一个；
/// 挂 `Auto` 图 ⇒ **改图**（`class`）并保持 `ship_type` 与它一致（口径 A）。
#[test]
fn player_pinned_blueprint_is_not_retooled() {
    let (config, mut state) = fresh_world(42);
    let [first, second] = two_corvette_yards(&mut state);
    state
        .control
        .entry("中国".to_string())
        .or_default()
        .blueprints
        .insert(
            "玩家钉的护卫".to_string(),
            Control::player(Blueprint {
                class: "corvette".to_string(),
                components: vec!["kinetic".to_string(), "ion_drive".to_string()],
                doctrine: None,
                kiting: None,
                role: None,
            }),
        );
    if let Some(city) = state.city_mut(&first.0) {
        for b in city.buildings.iter_mut() {
            if b.id == first.1 {
                b.blueprint = Some("玩家钉的护卫".to_string());
            }
        }
    }
    let mut rng = Prng::new(7);
    let mut retools = Vec::new();
    retool_shipyards(&mut state, &config, "中国", &mut rng, &mut retools, &mut crate::model::RoundInputs::default());

    let rec = retools
        .iter()
        .find(|r| r.faction == "中国")
        .expect("战时过度单一 ⇒ 必须有一次改装");
    assert_eq!(
        (rec.city.clone(), rec.building),
        second.clone(),
        "挂了 Player 图的舰坞必须被**跳过**，改装落在另一个区上"
    );
    // 被钉住的那座：舰级、图的舰级、图的选装**都没变**。
    let bp = state.control["中国"].blueprints["玩家钉的护卫"].clone();
    assert_eq!(bp.value.class, "corvette", "玩家钉的图的舰级不许被 AI 改");
    assert_eq!(
        bp.value.components,
        vec!["kinetic".to_string(), "ion_drive".to_string()],
        "选装不许被改"
    );
    let pinned = state
        .city(&first.0)
        .unwrap()
        .buildings
        .iter()
        .find(|b| b.id == first.1)
        .unwrap()
        .ship_type
        .clone();
    assert_eq!(
        pinned.as_deref(),
        Some("corvette"),
        "被钉住的建造区不许被改装"
    );
    // 另一个区照旧被重定向到战局需要的舰级。
    let other = state
        .city(&second.0)
        .unwrap()
        .buildings
        .iter()
        .find(|b| b.id == second.1)
        .unwrap()
        .ship_type
        .clone();
    assert_ne!(
        other.as_deref(),
        Some("corvette"),
        "没挂图的那个区照旧被改装：{other:?}"
    );
    assert_eq!(other, Some(rec.to.clone()));
}

/// 挂 **`Auto`** 图的舰坞：改装要**改图**（`class`），并让图的舰级与区的舰级保持一致。
#[test]
fn an_auto_blueprint_is_retooled_as_a_blueprint() {
    let (config, mut state) = fresh_world(42);
    let [first, _second] = two_corvette_yards(&mut state);
    state
        .control
        .entry("中国".to_string())
        .or_default()
        .blueprints
        .insert(
            "auto:corvette".to_string(),
            Control::auto(Blueprint {
                class: "corvette".to_string(),
                components: Vec::new(),
                doctrine: None,
                kiting: None,
                role: None,
            }),
        );
    if let Some(city) = state.city_mut(&first.0) {
        for b in city.buildings.iter_mut() {
            if b.id == first.1 {
                b.blueprint = Some("auto:corvette".to_string());
            }
        }
    }
    let mut rng = Prng::new(7);
    let mut retools = Vec::new();
    retool_shipyards(&mut state, &config, "中国", &mut rng, &mut retools, &mut crate::model::RoundInputs::default());
    let rec = retools
        .iter()
        .find(|r| r.faction == "中国")
        .expect("必须有一次改装");
    assert_eq!(
        (rec.city.clone(), rec.building),
        first,
        "Auto 图不挡改装（第一个候选就是它）"
    );
    let bp = state.control["中国"].blueprints["auto:corvette"].clone();
    assert_eq!(bp.value.class, rec.to, "改的是**图**的舰级");
    assert_eq!(bp.mode, ControlMode::Auto, "归属不许被改装动作改掉");
    assert!(
        bp.value.components.is_empty(),
        "选装**不预生成**（出厂那一刻由生成器现算）"
    );
    let yard = state
        .city(&first.0)
        .unwrap()
        .buildings
        .iter()
        .find(|b| b.id == first.1)
        .unwrap()
        .ship_type
        .clone();
    assert_eq!(
        yard,
        Some(rec.to.clone()),
        "口径 A：区的 ship_type 必须与图的 class 相等"
    );
}
