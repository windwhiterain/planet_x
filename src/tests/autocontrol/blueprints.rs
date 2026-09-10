//! AI 建图的单元测试。

use super::*;
use crate::config::load_config;
use crate::control::apply_patch;
use crate::world::default_state;

fn fresh(seed: u64) -> (GameConfig, State) {
    let config = load_config();
    let state = default_state(&config, seed);
    (config, state)
}

/// 本势力所有建造区（城名、建筑 id、当前图）。
fn yards(state: &State, fid: &str) -> Vec<(CityId, BuildingId, Option<BlueprintId>)> {
    state
        .cities
        .iter()
        .filter(|c| c.faction_id == fid)
        .flat_map(|c| {
            c.buildings
                .iter()
                .filter(|b| b.is_shipyard())
                .map(|b| (c.name.clone(), b.id, b.blueprint.clone()))
        })
        .collect()
}

fn lib(state: &State, fid: &str) -> BTreeMap<BlueprintId, Control<Blueprint>> {
    state
        .control(fid.to_string())
        .map(|c| c.blueprints.clone())
        .unwrap_or_default()
}

/// **AI 会自己建图**（`Auto` 图那一层的执行者）：跑一趟之后，归它管的每个建造区都指着一张
/// 图库里真实存在的图，舰级与建造区相等（口径 A），图的归属是 `Inherit`（流水）、
/// 意图轴**沉默**（建图 ≠ 表态，Q1(c)）。
#[test]
fn the_ai_creates_a_design_for_every_yard_it_owns() {
    let (config, mut state) = fresh(42);
    let fid = "中国".to_string();
    let mut out = Vec::new();
    design_fleets(&mut state, &config, &mut out, &mut crate::model::RoundInputs::default());
    assert!(!out.is_empty(), "AI 该给归它管的建造区建图");
    let l = lib(&state, &fid);
    assert!(!l.is_empty(), "{fid} 的图库该有图");
    for (city, bid, ptr) in yards(&state, &fid) {
        let name = ptr.unwrap_or_else(|| panic!("{city}#{bid} 没有被指到任何设计图"));
        let leaf = l
            .get(&name)
            .unwrap_or_else(|| panic!("{city}#{bid} 指着一张不存在的图 {name}"));
        let class = state
            .city(&city)
            .unwrap()
            .buildings
            .iter()
            .find(|b| b.id == bid)
            .and_then(|b| b.ship_type.clone())
            .unwrap();
        assert_eq!(leaf.value.class, class, "口径 A：图的舰级必须与建造区相等");
        assert_eq!(
            leaf.mode,
            ControlMode::Inherit,
            "AI 写的是流水（这一层没有说话）"
        );
        assert!(
            leaf.value.doctrine.is_none() && leaf.value.kiting.is_none() && leaf.value.role.is_none(),
            "建图 ≠ 表态：AI 不写任何倾向轴"
        );
        assert!(
            name.starts_with(DESIGN_PREFIX),
            "自建图的名字该带 AI 前缀（回收只碰自己造的东西）：{name}"
        );
    }
    // 建图是**纯状态改写**：它不消费主 Prng（函数签名里根本没有 rng）。
}

/// **去重**（`ship-blueprint.md` §5 的"设计图爆炸"）：同一 `(舰级, 选装签名)` 只留一张，
/// 而且再跑一趟**幂等**（不会每回合新造一张）。
#[test]
fn designs_are_deduped_by_class_and_signature() {
    let (config, mut state) = fresh(7);
    let fid = "中国".to_string();
    let mut out = Vec::new();
    design_fleets(&mut state, &config, &mut out, &mut crate::model::RoundInputs::default());
    let first = lib(&state, &fid);
    assert!(!first.is_empty());
    // 幂等：同样的状态再跑一趟，图库一个字节都不长。
    let mut out2 = Vec::new();
    design_fleets(&mut state, &config, &mut out2, &mut crate::model::RoundInputs::default());
    let second = lib(&state, &fid);
    assert_eq!(
        first.keys().collect::<Vec<_>>(),
        second.keys().collect::<Vec<_>>(),
        "同一状态再跑一趟不该新造图（去重 + 幂等）"
    );
    // 没有两张图是同一个 (舰级, 选装签名)；同舰级的建造区共用一个名字。
    let mut seen: BTreeMap<(String, Vec<String>), String> = BTreeMap::new();
    for (name, leaf) in &second {
        let key = (leaf.value.class.clone(), leaf.value.components.clone());
        if let Some(other) = seen.get(&key) {
            // 「同签名不同名」只允许在"玩家自己也插了一张"的情形，这里没有玩家 ⇒ 不许出现。
            panic!("图 {name} 与 {other} 是同一个 (舰级, 选装签名)，应当归并");
        }
        seen.insert(key, name.clone());
    }
    let mut by_class: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (_, _, ptr) in yards(&state, &fid) {
        if let Some(ptr) = ptr {
            let class = second
                .get(&ptr)
                .map(|l| l.value.class.clone())
                .unwrap_or_default();
            by_class.entry(class).or_default().insert(ptr);
        }
    }
    for (class, names) in by_class {
        assert_eq!(
            names.len(),
            1,
            "同舰级的建造区该共用一张图（{class} 有 {names:?}）"
        );
    }
}

/// **玩家的图一个字都不许动**：玩家把图钉成 `Player` 并让建造区指着它 ⇒ 那个建造区
/// 整个被跳过（不改图、不改指针），回收也不碰它。
#[test]
fn a_player_pinned_design_and_its_yard_are_left_alone() {
    let (config, mut state) = fresh(42);
    let fid = "中国".to_string();
    let (city, bid, _) = yards(&state, &fid)
        .into_iter()
        .next()
        .expect("中国有建造区");
    let class = state
        .city(&city)
        .unwrap()
        .buildings
        .iter()
        .find(|b| b.id == bid)
        .and_then(|b| b.ship_type.clone())
        .unwrap();
    let diff = serde_json::json!({"control": [{"faction_id": "中国",
        "blueprints": [{"name": "玩家的守卫图", "class": class, "components": ["kinetic", "ion_drive"]}]}]});
    apply_patch(&mut state, &config, &diff).expect("player design applies");
    // 钉成 Player + 把建造区指过去。
    let diff = serde_json::json!({"control": [
        {"faction_id": "中国", "blueprints": [{"name": "玩家的守卫图", "mode": "Player"}]},
        {"faction_id": "中国", "buildings": [{"city": city, "building": bid, "blueprint": "玩家的守卫图"}]}
    ]});
    apply_patch(&mut state, &config, &diff).expect("pin + pointer applies");
    let mut out = Vec::new();
    design_fleets(&mut state, &config, &mut out, &mut crate::model::RoundInputs::default());
    let leaf = lib(&state, &fid)
        .get("玩家的守卫图")
        .cloned()
        .expect("玩家的图还在");
    assert_eq!(leaf.mode, ControlMode::Player, "玩家的图归玩家");
    assert_eq!(
        leaf.value.components,
        vec!["kinetic", "ion_drive"],
        "玩家的选装一个字都不许动"
    );
    let ptr = yards(&state, &fid)
        .into_iter()
        .find(|(c, b, _)| *c == city && *b == bid)
        .and_then(|(_, _, p)| p);
    assert_eq!(
        ptr.as_deref(),
        Some("玩家的守卫图"),
        "玩家指过去的建造区不许被改派"
    );
    assert!(
        !out.iter().any(|d| d.blueprint == "玩家的守卫图"),
        "回收/重估都不许碰玩家的图"
    );
}

/// **悬空指针 ⇒ 跳过**：删图是玩家/agent 的动作，那个区停产本身就是可见的后果，
/// AI 不去替他收拾（不建新图、不改指针）。
#[test]
fn a_dangling_pointer_is_left_dangling() {
    let (config, mut state) = fresh(42);
    let fid = "中国".to_string();
    let (city, bid, _) = yards(&state, &fid)
        .into_iter()
        .next()
        .expect("中国有建造区");
    if let Some(c) = state.city_mut(&city) {
        for b in &mut c.buildings {
            if b.id == bid {
                b.blueprint = Some("已经删掉的图".to_string());
            }
        }
    }
    let mut out = Vec::new();
    design_fleets(&mut state, &config, &mut out, &mut crate::model::RoundInputs::default());
    let ptr = yards(&state, &fid)
        .into_iter()
        .find(|(c, b, _)| *c == city && *b == bid)
        .and_then(|(_, _, p)| p);
    assert_eq!(
        ptr.as_deref(),
        Some("已经删掉的图"),
        "悬空指针原样保留（那个区停产是可见后果，不是让 AI 悄悄修好）"
    );
}

/// **回收**：没人指向的**自建**图删掉；玩家钉的、以及不是 AI 命名的图一个都不碰。
#[test]
fn only_unreferenced_selfmade_designs_are_reaped() {
    let (config, mut state) = fresh(42);
    let fid = "中国".to_string();
    // 三张没人指向的图：AI 造的（`mode: Inherit` = 流水，该被回收）、玩家起的名的、
    // 以及玩家**钉住**的 AI 命名图。
    // ⚠ 这里必须显式写 `mode: "Inherit"`：`--apply` 的「写值即接管」会把没写 mode 的图
    // 钉成 `Player`（= 玩家写的），而 AI 自己写叶走的是直接状态改写、写的是 `Inherit`
    // ——"回收只碰自己造的"这条安全属性正是靠这个区分成立的。
    let diff = serde_json::json!({"control": [{"faction_id": "中国", "blueprints": [
        {"name": "自动强袭·陈图", "class": "corvette", "components": ["kinetic"], "mode": "Inherit"},
        {"name": "玩家自己的图", "class": "corvette", "components": ["kinetic"]}
    ]}]});
    apply_patch(&mut state, &config, &diff).expect("designs apply");
    state.control_mut(fid.clone()).unwrap().blueprints.insert(
        "自动堡垒·玩家钉的".to_string(),
        Control::player(Blueprint {
            class: "corvette".to_string(),
            components: vec![],
            doctrine: None,
            kiting: None,
            role: None,
        }),
    );
    let mut out = Vec::new();
    design_fleets(&mut state, &config, &mut out, &mut crate::model::RoundInputs::default());
    let l = lib(&state, &fid);
    assert!(!l.contains_key("自动强袭·陈图"), "没人指向的自建图该被回收");
    assert!(l.contains_key("玩家自己的图"), "不是 AI 命名的图不许碰");
    assert!(l.contains_key("自动堡垒·玩家钉的"), "玩家钉住的图不许回收");
    assert!(
        out.iter()
            .any(|d| d.action == "reaped" && d.blueprint == "自动强袭·陈图")
    );
}

/// **改图不影响已有的舰**（快照语义）：设计图重估了选装，已经下水的舰一个字节都不变，
/// 而**之后**下水的舰按新图装配。
#[test]
fn retuning_a_design_never_touches_ships_already_in_space() {
    let (config, mut state) = fresh(42);
    let fid = "中国".to_string();
    let (city, bid, _) = yards(&state, &fid)
        .into_iter()
        .next()
        .expect("中国有建造区");
    let class = state
        .city(&city)
        .unwrap()
        .buildings
        .iter()
        .find(|b| b.id == bid)
        .and_then(|b| b.ship_type.clone())
        .unwrap();
    // 先让 AI 建图并指过去，然后照这张图造一艘舰。
    let mut out = Vec::new();
    design_fleets(&mut state, &config, &mut out, &mut crate::model::RoundInputs::default());
    let ptr = yards(&state, &fid)
        .into_iter()
        .find(|(c, b, _)| *c == city && *b == bid)
        .and_then(|(_, _, p)| p)
        .expect("建造区该被指到一张图");
    let spec = crate::sim::ShipSpawn {
        owner: fid.clone(),
        class: &class,
        position: state.body_position(&state.capital_body(&fid)),
        city: Some(city.clone()),
        via: crate::model::SpawnVia::Shipyard,
        pay_components: false,
        blueprint: Some(&ptr),
    };
    let ship = crate::sim::spawn_ship(&mut state, &config, spec);
    let before = state.ship(&ship).cloned().unwrap();
    assert_eq!(
        before.blueprint.as_deref(),
        Some(ptr.as_str()),
        "出厂归因：这艘舰出自那张图"
    );
    assert_eq!(
        before.components,
        lib(&state, &fid)[&ptr].value.components,
        "按图装配（`Auto` 图的选装**真的生效**——本轮改掉的旧语义是「静默忽略」）"
    );
    // 把库存掏空 ⇒ 下一次重估必然给出不同的选装（甚至空），于是图会被改写。
    if let Some(f) = state.faction_mut(&fid) {
        for v in f.resources.values_mut() {
            *v = 0.0;
        }
    }
    let mut config2 = config.clone();
    config2.autocontrol.blueprint_chance = 1.0; // 让"这一回合重估"变成确定事件
    let mut out2 = Vec::new();
    design_fleets(&mut state, &config2, &mut out2, &mut crate::model::RoundInputs::default());
    let after = state.ship(&ship).cloned().unwrap();
    assert_eq!(
        after.components, before.components,
        "**已下水的舰是快照**：改图不许追溯改它"
    );
    assert_eq!(after.hull_max, before.hull_max);
    assert_eq!(after.component_hp, before.component_hp);
}

/// **舰级对不上就修**（口径 A）：`retool` 改了建造区的舰级之后，设计图那一趟要把图与区
/// 重新对齐（换名字/换图），而不是留下"图说造 A、区说造 B"的悬空口径。
#[test]
fn a_class_drift_between_the_yard_and_its_design_is_reconciled() {
    let (config, mut state) = fresh(42);
    let fid = "中国".to_string();
    let (city, bid, _) = yards(&state, &fid)
        .into_iter()
        .next()
        .expect("中国有建造区");
    let mut out = Vec::new();
    design_fleets(&mut state, &config, &mut out, &mut crate::model::RoundInputs::default());
    let old = yards(&state, &fid)
        .into_iter()
        .find(|(c, b, _)| *c == city && *b == bid)
        .and_then(|(_, _, p)| p)
        .unwrap();
    // 把建造区改造成另一级（就像 `retool_shipyards` 做的那样）。
    let new_class = if lib(&state, &fid)[&old].value.class == "cruiser" {
        "corvette"
    } else {
        "cruiser"
    };
    if let Some(c) = state.city_mut(&city) {
        for b in &mut c.buildings {
            if b.id == bid {
                b.ship_type = Some(new_class.to_string());
            }
        }
    }
    let mut out2 = Vec::new();
    design_fleets(&mut state, &config, &mut out2, &mut crate::model::RoundInputs::default());
    let ptr = yards(&state, &fid)
        .into_iter()
        .find(|(c, b, _)| *c == city && *b == bid)
        .and_then(|(_, _, p)| p)
        .unwrap();
    let l = lib(&state, &fid);
    assert_eq!(
        l[&ptr].value.class, new_class,
        "图与建造区必须重新对齐（口径 A）"
    );
    assert_ne!(
        ptr, old,
        "舰级变了 ⇒ 换一张对应舰级的图（名字里就写着舰级）"
    );
}
