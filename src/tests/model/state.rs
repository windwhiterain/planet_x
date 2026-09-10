//! State 的单元测试（checkpoint 迁移）。

use super::*;
use crate::config::load_config;

/// **旧档加载**（§6 的 `v9 → v10` / §7.5-20）：v9 档里根本没有设计图那一轮的字段。
///
/// 这里拿一份**真的旧格式文本**来测，而不是「把字段设成 None 再存一遍」：把当前状态
/// 序列化成 RON，然后**逐字删掉** v9→v10 新增的四个字段
/// （`ControllableState.blueprints` / `Building.blueprint` / `Ship.blueprint` /
/// `Ship.spawned_round`）并把版本号写回 9 —— 那就是一份 v9 档（同一代里没有这些键）。
///
/// 断言三件事：
/// 1. 它**能读进来**（四个字段都是 `#[serde(default)]`）；
/// 2. `migrate` **只推版本号**（`Ok`，且落到 `SCHEMA_VERSION`）——这一档零信息损失；
/// 3. 补齐的值就是「空库 / 无指针 / 未知回合」，且实体一个不少（旧档行为逐值不变）。
#[test]
fn a_v9_checkpoint_loads_with_empty_blueprints_and_no_pointers() {
    let config = load_config();
    let mut state = crate::world::default_state(&config, 7);
    let mut rng = crate::prng::Prng::new(7);
    for _ in 0..8 {
        crate::sim::advance(&mut state, &config, &mut rng);
    }
    let ships = state.ships.len();
    let cities = state.cities.len();
    let buildings: usize = state.cities.iter().map(|c| c.buildings.len()).sum();
    // **先把这个新层清干净**：8 回合之后 AI（`autocontrol::blueprints`）已经建了自己的设计图、
    // 并把建造区指了过去（那是 v10 之后才有的东西）。本测试的文本手术要造的是**一份 v9 档**
    // ——v9 那一代里根本没有设计图这一层，所以得先把它按 v9 的样子清空（空库 / 无指针），
    // 否则序列化里既没有 `blueprints:{}` 也没有 `blueprint:None` 可删。
    for c in state.control.values_mut() {
        c.blueprints.clear();
    }
    for c in state.cities.iter_mut() {
        for b in c.buildings.iter_mut() {
            b.blueprint = None;
        }
    }
    // 同时把 `spawned_round` 归零：v9 档里**没有这个键**，而紧凑 RON 里 `Some(12)` 没法用
    // 一次字符串替换干净地删掉（`None` 可以）。这不妨碍本测试的目的——它测的是
    // 「文件里没有那个键时会发生什么」。
    for s in state.ships.iter_mut() {
        s.blueprint = None;
        s.spawned_round = None;
    }
    // **事件日志也清掉**：`ShipSpawned` 那类事件同样带 `blueprint` 字段（v9 之后才有的），
    // 而它的值是个**名字**（`Some("自动平价·护卫舰")`），字符串手术删不干净。本用例测的是
    // **状态结构**的迁移，不是事件日志的形状——而「这一回合出过哪些事件」本来就随世界走向变
    //（改一条经济机制就可能让它出现），留着它会让这条守卫变成一条**行为**断言。
    state.events.clear();
    let text = ron::to_string(&state).expect("serialize the state");
    for needle in ["blueprint:None", "blueprints:{}"] {
        assert!(
            text.contains(needle),
            "序列化里应当出现 `{needle}`（改过 RON 形状的话，这条手术要先更新）"
        );
    }
    let old_text = text
        .replace(",blueprint:None", "")
        .replace(",blueprints:{}", "")
        .replace(",spawned_round:None", "")
        // 用 `SCHEMA_VERSION` 拼针脚，而不是写死当时那个号：这条手术的目的是**造一份真的
        // v9 档**，而版本号每升一档都会变（合并设计图/承包市场两条分支时就已经撞过一次：
        // 写死 `10` 的针脚在 v13 上什么都不替换，于是「v9 档」里写着 13）。
        .replace(&format!("schema_version:{SCHEMA_VERSION}"), "schema_version:9");
    assert!(
        !old_text.contains("blueprint") && !old_text.contains("spawned_round"),
        "手术没做干净：v9 档里不该出现设计图那四个字段 —— 残留处：{:?}",
        old_text
            .find("blueprint")
            .or_else(|| old_text.find("spawned_round"))
            .map(|i| {
                let head: String = old_text[..i].chars().rev().take(80).collect::<Vec<_>>()
                    .into_iter().rev().collect();
                let tail: String = old_text[i..].chars().take(120).collect();
                format!("{head}{tail}")
            })
    );

    let mut restored: State = ron::from_str(&old_text).expect("v9 档必须能读进来（serde default 补齐）");
    assert_eq!(restored.schema_version, 9, "档里写的就是 9");
    migrate(&mut restored).expect("v9 必须能迁到当前版本");
    assert_eq!(restored.schema_version, SCHEMA_VERSION);
    assert_eq!(restored.ships.len(), ships, "实体不许在迁移里丢");
    assert_eq!(restored.cities.len(), cities);
    assert_eq!(restored.cities.iter().map(|c| c.buildings.len()).sum::<usize>(), buildings);
    for (fid, c) in &restored.control {
        assert!(c.blueprints.is_empty(), "{fid} 的设计图库在旧档里必须是空的");
    }
    for s in &restored.ships {
        assert_eq!(s.blueprint, None, "旧档的舰没有出厂图");
        assert_eq!(s.spawned_round, None, "旧档缺 spawned_round ⇒ 未知（不是第 0 回合）");
    }
    for c in &restored.cities {
        for b in &c.buildings {
            assert_eq!(b.blueprint, None, "旧档的建造区没有图指针 ⇒ 走 choose_loadout");
        }
    }
    // 没有图 ⇒ 指令链上新增的那一层恒为空 ⇒ 取值与旧档一致：舰上的叶（出厂时那条
    // `Inherit` 的 `Idle`）就是它的答案，归属仍是「系统自动」（除非 scope 另有表态）。
    if let Some(ship) = restored.ships.first().map(|s| s.name.clone()) {
        assert_eq!(restored.ship_behavior_source(ship.clone()), Some(OrderSource::Leaf));
        assert_eq!(restored.ship_behavior(ship.clone()), Some(ShipBehavior::Idle));
        assert_eq!(restored.ship_control(ship), ControlMode::Auto);
    }
}
