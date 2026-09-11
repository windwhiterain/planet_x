//! 设计图（`--apply` 的 blueprint 叶）与船坞下水：`Auto` 图的空选装现场调生成器。
//!
//! ## 2026-10（第 7 批）：五条搬去了 g2「合成场景 · 下水那艘舰长什么样」
//!
//! | 原用例 | 判据 |
//! | --- | --- |
//! | `spawn_uses_the_yard_blueprint` / `the_yard_launches_from_any_designs_components` | 图上写了选装 ⇒ 下水那艘就按它装配，**与图的归属无关**（两臂只差 `Player`/`Auto` 一个变量） |
//! | `blueprint_role_governs_new_ships` | 图上的 `角色` 比舰队默认更具体 ⇒ 图赢 |
//! | `fleet_default_still_covers_blueprintless_ships` | 图沉默的那条轴仍由舰队默认作答（那臂摆的是**非缺省**的 `Observe`，所以不是空转） |
//! | `order_source_separates_a_missing_leaf_from_a_silent_one` | 图写得再满也**不供指令**（`ships.order_source` 只会是 `leaf`），但倾向（风格/姿态）确实从图下来 |
//!
//! **留在这里的**：`auto_blueprint_uses_choose_loadout_at_launch` —— 它测的是「空选装的
//! `Auto` 图在**出厂那一刻现场**调 `choose_loadout`」，读面只看得见结果、看不见「现场」。
//! 夹具 `attach_blueprint`/`spawn_at`/`stock` 在 `super`。

//! ## 2026-10：能只看数据的那几条搬去了 `play/tests/g2_mid.py`
//!
//! | 原用例 | 现在住 | 为什么能搬 |
//! | --- | --- | --- |
//! | `editing_a_blueprint_does_not_touch_existing_ships` / `retuning_a_design_never_touches_ships_already_in_space` | g2「**出厂快照不随时间变**」（一条舰的选装一生恒定，400 条舰零漂移） | `ships.components` 与 `blueprints.components` 都在读面上 |
//! | `ship_spawned_event_carries_the_blueprint_only_when_there_is_one` | g2「造舰事件的图归因与舰表一致」（374 条事件） | 事件层 `ship_spawned.data.blueprint` ↔ 舰表 `blueprint` |
//! | `a_dangling_blueprint_pointer_stops_the_yard` | g2「指针悬空 ⇒ 那个建造区**连建造行都没有**」 | 停产在读面上就是 `city_process.build` 里**没有那一行**（与「缺钱」的 `increment = 0` 分得开）；防空转 = 同一座城批满时**有**建造行 |
//! | `a_player_blueprint_that_cannot_be_afforded_waits_for_money` | g2「买不起 ⇒ 不下水 + `launch_waiting` 亮着」/「垫厚国库 ⇒ 同一个图就下水」（**A/B 只差国库一个变量**） | `blueprints.launch_waiting` 就是那个可见标记；国库用 `_stock_patch` 拨（状态补丁），进度看 `city_process.build.increment` |
//! | `designs_are_deduped_by_class_and_signature` | g2「图按 `(舰级, 选装)` 去重」（18,691 个签名） | 图库表逐回合可查 |
//! | `a_class_drift_between_the_yard_and_its_design_is_reconciled` | g2「建造区挂了图就必须挂到存在的图上」+「舰级不符只是**滞后**、会自己收敛」（16,709 个建造区·回合；实测 3 行不符、最长滞后 1 回合） | `cities.buildings[].{ship_type,blueprint}` + 图库表 |
//!
//! **留在这里的**（这一族住在 `src/tests/autocontrol/blueprints.rs`，本文件是船坞下水那一侧）：
//! `the_ai_creates_a_design_for_every_yard_it_owns`——它要「每个区的**有效**归属」（读面只有
//! 逐个区自己的 `blueprint` 指针，判不出「AI 该不该给它建图」）。另外三条（玩家钉住的图 /
//! 悬空指针 / 回收只碰自己造的）已随第 6 批搬去 g2 的**合成场景 · 拨控制叶**（施工图 §5.6）。

use super::*;

/// `Auto` 图（`components: []`）⇒ 出厂那一刻**现场**调 `choose_loadout`（§7.1-4）。
///
/// ⚠ 本用例测的是**空选装**那条路（`components` 为空 = 交给生成器，与归属无关）。
/// 「`Auto` 图的选装被**静默忽略**」那条旧语义本轮已经改掉：图 = 出厂规格、`mode` = 谁能改图
/// ⇒ `components` 非空时**任何**归属都按图装配（见 `the_yard_launches_from_any_designs_components`）。
#[test]
fn auto_blueprint_uses_choose_loadout_at_launch() {
    let (config, mut state) = fresh_world(42);
    let fid = "中国".to_string();
    stock(&mut state, &config, &fid, 300.0);
    let (_, _, class) = attach_blueprint(
        &mut state,
        &config,
        &fid,
        "auto:corvette",
        "corvette",
        &[],
        (None, None, None),
        ControlMode::Auto,
    );
    // 同一时点的生成器答案（`spawn_ship` 内部就是走它——不许另写一份）。
    let site = state.stock_at(&fid, "地球").cloned().unwrap_or_default();
    let expected = crate::autocontrol::resolve_loadout(
        &state,
        &config,
        fid.clone(),
        &class,
        Some(&"auto:corvette".to_string()),
        &site,
    );
    assert_eq!(
        expected,
        crate::autocontrol::choose_loadout(&state, &config, fid.clone(), &class),
        "`Auto` 图的选装必须**就是** `choose_loadout` 的答案（不是第二份生成逻辑）"
    );
    let name = spawn_at(
        &mut state,
        &config,
        &fid,
        &class,
        "地球",
        Some("auto:corvette"),
    );
    assert_eq!(
        state.ship(&name).unwrap().components,
        expected,
        "出厂用的是当场算出来的选装"
    );

    // **没有提前缓存**：把库存掏空之后再算，生成器给不出完整选装——只剩**平台兜底**的那件
    // 推进器（「至少一件推进」是硬保证：没有推进器的舰速度 0、永远不能当运输舰，那是
    // 「完全禁止瞬移」下最容易踩的死亡螺旋，见 `choose_loadout_prefs` 的注释）。
    // 若选装是在回合步进里预生成的，这里就会拿到上一回合那份完整的选装。
    if let Some(f) = state.faction_mut(&fid) {
        for v in f.resources.values_mut() {
            *v = 0.0;
        }
    }
    let site = state.stock_at(&fid, "地球").cloned().unwrap_or_default();
    let empty_stock = crate::autocontrol::resolve_loadout(
        &state,
        &config,
        fid.clone(),
        &class,
        Some(&"auto:corvette".to_string()),
        &site,
    );
    assert!(
        empty_stock.len() < expected.len() && empty_stock.len() <= 1,
        "库存掏空 ⇒ 只剩平台兜底（证明它是**出厂那一刻**算的）：{empty_stock:?} vs {expected:?}"
    );
    assert!(
        empty_stock
            .iter()
            .all(|c| config.component_spec(c).category == "thrust"),
        "兜底只给**平台**（推进器）：买不起的军备绝不白送，实为 {empty_stock:?}"
    );
}

