//! AI 建图的单元测试。

//! ## 2026-10：能只看数据的那几条搬去了 `play/tests/g2_mid.py`
//!
//! | 原用例 | 现在住 | 为什么能搬 |
//! | --- | --- | --- |
//! | `editing_a_blueprint_does_not_touch_existing_ships` / `retuning_a_design_never_touches_ships_already_in_space` | g2「**出厂快照不随时间变**」（一条舰的选装一生恒定，400 条舰零漂移） | `ships.components` 与 `blueprints.components` 都在读面上 |
//! | `ship_spawned_event_carries_the_blueprint_only_when_there_is_one` | g2「造舰事件的图归因与舰表一致」（374 条事件） | 事件层 `ship_spawned.data.blueprint` ↔ 舰表 `blueprint` |
//! | `designs_are_deduped_by_class_and_signature` | g2「图按 `(舰级, 选装)` 去重」（18,691 个签名） | 图库表逐回合可查 |
//! | `a_class_drift_between_the_yard_and_its_design_is_reconciled` | g2「建造区挂了图就必须挂到存在的图上」+「舰级不符只是**滞后**、会自己收敛」（16,709 个建造区·回合；实测 3 行不符、最长滞后 1 回合） | `cities.buildings[].{ship_type,blueprint}` + 图库表 |
//! | `a_player_pinned_design_and_its_yard_are_left_alone` / `a_dangling_pointer_is_left_dangling` / `only_unreferenced_selfmade_designs_are_reaped` | g2 **合成场景 · 拨控制叶**（施工图 §5.6 第 6 批）：「玩家钉住的图一个字都不许动」「悬空指针原样保留（AI 不替你修）」「回收只碰自己造的」 | 三条都是「**先在世界上做一件事**（钉图 / 把指针改成悬空 / 造几张没人指向的图），再看 AI 那一趟怎么反应」——现在 `h.scenario(patch=…)` 改状态字段、`h.scenario_apply(diffs=…)` 走引擎自己的 `--apply` 拨控制叶，断言仍只读 `blueprints`/`cities`/`decisions` 三张表 |
//!
//! **留在这里的**：`the_ai_creates_a_design_for_every_yard_it_owns`——它要「每个区的**有效**
//! 归属」（读面只有逐个区自己的 `blueprint` 指针，判不出「AI 该不该给它建图」）。

use super::*;
use crate::config::load_config;
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
