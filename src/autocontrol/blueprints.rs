//! **设计图的执行者**：AI 势力自己**建图 / 重估 / 回收**。
//!
//! # 为什么需要它（另一半空头支票）
//!
//! `Auto` 的设计图承诺「系统可重估它」，而今天只有半个执行者：
//! [`super::shipbuilding::retool_shipyards`] 会重估**已有图**的舰级，但 AI **从不自己建图**
//! （`.agents/notes/ship-blueprint.md` §6「未做项」）。于是世界的图库要么空着（所有建造区
//! 走「出厂那一刻现算选装」），要么只有玩家手写的图——`Auto` 那一层等于没有执行者。
//! 这个模块把 AI 的建图补齐：**按自己的资源优势与战况**给每个归它管的建造区生成一张设计图。
//!
//! # 设计图是什么形状的（三个决定 + 理由）
//!
//! 1. **一张图 = 一个 `(舰级, 设计主题)`**。主题是「这一型舰是干什么的」（`强袭`/`堡垒`/
//!    `远洋`/`平价`，见 `config.autocontrol.blueprint_themes`），它决定选装评分里各模块分类的
//!    权重与造价惩罚——**这就是"图库为什么会爆炸"的答案**：图名里有主题，同主题同舰级永远
//!    复用同一张（而不是每算出一份新选装就新造一张）。长局里每势力的图库因此是
//!    `O(主题数 × 舰级数)`，而不是 `O(回合数)`。
//!    * 主题按**抽签**选（权重 = `weight + 战争强度 × war_weight`）⇒ 在打仗的势力自然画出
//!      进攻型设计；主题换掉 = 换一张图（旧图没人指向就被回收）。
//!    * 另有**按签名归并**：`(舰级, 选装签名)` 相同的自建图直接复用（去重的那一条）。
//! 2. **建图 ≠ 表态**：AI 写的图 `order: None`（意图轴留空，用户裁决 Q1(c)），叶片一律
//!    [`Control::inherit`]（「这一层没有说话」）——与指令轴/风格轴同一条规矩。
//! 3. **选装是"当时的快照"，且只挑当时买得起的**：组件由
//!    [`super::shipbuilding::choose_loadout_themed`] 生成——它与出厂现算用的是**同一套**
//!    可得性检查，所以 AI 画出来的图不会要一批势力供不起的模块。库存会变，所以设计**按概率
//!    重估**（跟着库存走），而**已下水的舰不受影响**（`Ship.components` 是出厂快照）。
//!
//! # 闸门（哪些建造区/哪些图归 AI）
//!
//! * **玩家钉住的图**（[`State::blueprint_control`](crate::model::State::blueprint_control)
//!   `== Player`）⇒ 那个建造区**整个跳过**（既不改图，也不改指针）；
//! * **悬空指针**（图已被删）⇒ 跳过：删图是玩家/agent 的动作，那个区停产本身就是可见的后果，
//!   AI 不去替玩家收拾（"失败看起来像成功"的反面教材就在这儿）；
//! * 想建/想用的那张图**此刻的归属**是 `Player`（叶自己的表态，或势力作用域/全局说了算）⇒ 不碰。
//!
//! # 回收（长局里图库不该只增不减）
//!
//! 没人指向、且**名字带 AI 前缀**（`自动…`）、且不是玩家钉的图 ⇒ 删掉叶子。
//! 三个条件缺一不可：前缀保证"回收只碰自己造的东西"（对照
//! `agent-control-long-game.md` §7「幽灵权重」那条教训：条目只会单调增长）。
//! 已下水的舰不受影响——它们各自带着出厂快照，只是**出处**（`Ship.blueprint`）可能指向一张
//! 后来被回收的图（读面原样输出那个名字；`ship_blueprint_leaf` 查不到就当"这一层没有说话"）。

use crate::model::*;
use crate::sim;
use std::collections::{BTreeMap, BTreeSet};

use super::shipbuilding::choose_loadout_themed;

/// AI 自建设计图的**名字前缀**。回收只碰带这个前缀的图（不是"按归属猜"，而是"只收拾自己造的"）。
const DESIGN_PREFIX: &str = "自动";

/// 一个归 AI 管的建造区（它此刻指向哪张图，决定这个舰级的"设计意图"）。
struct Yard {
    city: CityId,
    building: BuildingId,
    blueprint: Option<BlueprintId>,
}

/// **每回合一次**：给归 AI 管的建造区建图 / 重估 / 复用 / 回收。
///
/// 调用点：`sim::step_construction` 的末尾（`retool_shipyards` **之后**）——那时本回合的舰级
/// 重估已经落定，设计图这一趟就能把"图与建造区对得上"顺手收敛（`retool` 改了舰级 ⇒ 图的名字
/// 与舰级要跟着换）。下一回合的出厂（`build_city` → `spawn_ship`）用的就是这些图。
pub(crate) fn design_fleets(state: &mut State, config: &GameConfig, out: &mut Vec<BlueprintDecision>) {
    if config.autocontrol.blueprint_themes.is_empty() {
        return; // 空表 = 不建图（退回「所有建造区无图、出厂现算」）。
    }
    let mut fids: Vec<FactionId> = state.factions.iter().map(|f| f.name.clone()).collect();
    fids.sort();
    for fid in fids {
        design_one_faction(state, config, &fid, out);
    }
}

fn design_one_faction(
    state: &mut State,
    config: &GameConfig,
    fid: &str,
    out: &mut Vec<BlueprintDecision>,
) {
    let round = state.round;
    // ① 归 AI 管的建造区，按**舰级**分组（图是舰级的函数 ⇒ 同舰级的区共用一张图）。
    let mut by_class: BTreeMap<String, Vec<Yard>> = BTreeMap::new();
    for c in &state.cities {
        if c.faction_id != fid {
            continue;
        }
        for b in &c.buildings {
            if !b.is_shipyard() {
                continue;
            }
            let Some(class) = b.ship_type.clone() else { continue };
            match b.blueprint.as_ref() {
                // 悬空指针：那个区已停产（Q10(a)）；删图是玩家/agent 的动作 ⇒ AI 不替他收拾。
                Some(ptr) if !blueprint_known(state, fid, ptr) => continue,
                // 玩家钉住的图 ⇒ 这个区不归 AI。
                Some(ptr) if state.blueprint_control(&fid.to_string(), ptr).is_player() => continue,
                _ => {}
            }
            by_class.entry(class).or_default().push(Yard {
                city: c.name.clone(),
                building: b.id,
                blueprint: b.blueprint.clone(),
            });
        }
    }
    if by_class.is_empty() {
        return;
    }
    let war = sim::war_strength(state, config, fid);
    let lib: BTreeMap<BlueprintId, Control<Blueprint>> = state
        .control(fid.to_string())
        .map(|c| c.blueprints.clone())
        .unwrap_or_default();

    for (class, yards) in by_class {
        // ② 本舰级此刻的**设计意图**：图的身份就写在名字里（`自动{主题}·{舰级}`），
        //    而建造区指着哪张图就是"上一次抽到了什么主题"——所以这条意图不需要额外状态。
        let current = current_theme(&lib, config, &class, &yards);
        let redraw = sim::derived_roll(fid, &class, round, "blueprint_intent")
            < config.autocontrol.blueprint_intent_chance;
        let theme = match current {
            Some(t) if !redraw => t,
            _ => draw_theme(config, fid, &class, round, war),
        };
        let intended = design_name(&theme.name, &class, config);
        // 想用的名字被玩家的图占了 ⇒ 这一轮对这个舰级什么都不做（宁可不动，也不改名/抢名字）。
        if lib
            .get(&intended)
            .map(|_| state.blueprint_control(&fid.to_string(), &intended).is_player())
            .unwrap_or(false)
        {
            continue;
        }
        // ③ 保持这张图（按概率重估选装）/ 复用同签名的一张 / 新建。
        let mut target = intended.clone();
        let mut action = "kept";
        let mut components = lib
            .get(&intended)
            .map(|l| l.value.components.clone())
            .unwrap_or_default();
        if let Some(existing) = lib.get(&intended) {
            let retune = sim::derived_roll(fid, &class, round, "blueprint_retune")
                < config.autocontrol.blueprint_chance;
            // 舰级对不上（`retool` 改了区、或别的路径改过）⇒ 无论如何都要修（口径 A）。
            let mut write = existing.value.class != class;
            if retune {
                let fresh = choose_loadout_themed(state, config, fid.to_string(), &class, theme);
                // 生成器在"库存全空"时只给一件免费兜底的推进器；那种答案不入图（否则图上
                // 写着它、出厂时又付不起，等于把"只挑买得起的"这条承诺作废）。
                if !fresh.is_empty() && fresh != components {
                    components = fresh;
                    write = true;
                }
            }
            if write {
                if let Some(leaf) = state
                    .control_mut(fid.to_string())
                    .and_then(|c| c.blueprints.get_mut(&target))
                {
                    // 只改该改的：舰级保持与建造区相等（口径 A），`order` 一个字不动
                    // （AI 从不写意图轴）。**已下水的舰不受影响**——它们的选装是出厂快照。
                    leaf.value.class = class.clone();
                    leaf.value.components = components.clone();
                }
                action = "retuned";
            }
        } else {
            let fresh = choose_loadout_themed(state, config, fid.to_string(), &class, theme);
            match signature_match(&lib, &class, &fresh) {
                // 同 (舰级, 选装签名) 的自建图已经存在 ⇒ 复用它（这就是"设计图去重"）。
                Some(name) => {
                    target = name;
                    action = "reused";
                }
                None => {
                    action = "created";
                    if let Some(c) = state.control_mut(fid.to_string()) {
                        c.blueprints.insert(
                            target.clone(),
                            // ⚠ **建图 ≠ 表态**（Q1(c)）：图上不写意图轴，叶子的归属是
                            // 「这一层没有说话」（链继续上升到势力作用域/全局）。
                            Control::inherit(Blueprint {
                                class: class.clone(),
                                components: fresh.clone(),
                                order: None,
                            }),
                        );
                    }
                }
            }
            components = fresh;
        }
        if action != "kept" {
            out.push(BlueprintDecision {
                faction: fid.to_string(),
                blueprint: target.clone(),
                action: action.to_string(),
                class: class.clone(),
                theme: theme.name.clone(),
                components: components.clone(),
                city: yards.first().map(|y| y.city.clone()),
                building: yards.first().map(|y| y.building),
            });
        }
        // ④ 把本舰级的建造区指过去（值没变就不写：存档/读面的 diff 是给人读的）。
        for y in &yards {
            if y.blueprint.as_deref() == Some(target.as_str()) {
                continue;
            }
            if let Some(c) = state.city_mut(&y.city) {
                for b in &mut c.buildings {
                    if b.id == y.building {
                        b.blueprint = Some(target.clone());
                    }
                }
            }
        }
    }

    // ⑤ 回收：没人指向、名字带 AI 前缀、且不是玩家钉的图（见模块文档）。
    if config.autocontrol.blueprint_reap {
        let referenced: BTreeSet<BlueprintId> = state
            .cities
            .iter()
            .filter(|c| c.faction_id == fid)
            .flat_map(|c| c.buildings.iter().filter_map(|b| b.blueprint.clone()))
            .collect();
        let reap: Vec<BlueprintId> = lib
            .iter()
            .filter(|(name, leaf)| {
                name.starts_with(DESIGN_PREFIX)
                    && !referenced.contains(name.as_str())
                    && !leaf.mode.is_player()
            })
            .map(|(name, _)| name.clone())
            .collect();
        for name in reap {
            let leaf = lib.get(&name).cloned();
            if let Some(c) = state.control_mut(fid.to_string()) {
                c.blueprints.remove(&name);
            }
            out.push(BlueprintDecision {
                faction: fid.to_string(),
                blueprint: name,
                action: "reaped".to_string(),
                class: leaf.as_ref().map(|l| l.value.class.clone()).unwrap_or_default(),
                theme: String::new(),
                components: leaf.map(|l| l.value.components).unwrap_or_default(),
                city: None,
                building: None,
            });
        }
    }
}

/// 这个势力库里**有没有**这张图（`Building.blueprint` 是悬空指针吗）。
fn blueprint_known(state: &State, fid: &str, bp: &str) -> bool {
    state
        .control(fid.to_string())
        .map(|c| c.blueprints.contains_key(bp))
        .unwrap_or(false)
}

/// 本舰级**此刻的设计意图**：从建造区指向的那张自建图的名字里读回来。
///
/// 名字就是图的身份（`自动{主题}·{舰级}`），所以"上一次抽到的是哪个主题"不需要额外状态——
/// 这也是"改主题 = 换一张图"能自己收敛的原因。认不出（主题名已不在配置表里、指向玩家自己
/// 起的名、或根本没有图）⇒ `None` = 重新抽。
fn current_theme<'a>(
    lib: &BTreeMap<BlueprintId, Control<Blueprint>>,
    config: &'a GameConfig,
    class: &str,
    yards: &[Yard],
) -> Option<&'a DesignTheme> {
    for y in yards {
        let Some(name) = y.blueprint.as_ref() else { continue };
        let Some(leaf) = lib.get(name) else { continue };
        if leaf.mode.is_player() || leaf.value.class != class {
            continue;
        }
        let Some(theme) = theme_of_name(name) else { continue };
        if let Some(t) = config
            .autocontrol
            .blueprint_themes
            .iter()
            .find(|t| t.name == theme)
        {
            return Some(t);
        }
    }
    None
}

/// 图名 → 主题名（`自动强袭·护卫舰` → `强袭`）。
fn theme_of_name(name: &str) -> Option<String> {
    let rest = name.strip_prefix(DESIGN_PREFIX)?;
    let theme = rest.split('·').next()?;
    if theme.is_empty() {
        None
    } else {
        Some(theme.to_string())
    }
}

/// 主题名 + 舰级 → 图名（图的**唯一身份**就是这个名字）。
fn design_name(theme: &str, class: &str, config: &GameConfig) -> BlueprintId {
    let label = config
        .ships
        .get(class)
        .map(|s| s.label.clone())
        .unwrap_or_else(|| class.to_string());
    format!("{DESIGN_PREFIX}{theme}·{label}")
}

/// 抽一个设计主题：权重 = `weight + 战争强度 × war_weight`（在打仗 → 更可能画进攻型）。
///
/// 骰子走 `(势力, 舰级, 回合, "blueprint_theme")` 派生，**不消费主 `Prng` 流**。
fn draw_theme<'a>(
    config: &'a GameConfig,
    fid: &str,
    class: &str,
    round: u32,
    war: f64,
) -> &'a DesignTheme {
    let themes = &config.autocontrol.blueprint_themes;
    let total: f64 = themes.iter().map(|t| (t.weight + war * t.war_weight).max(0.0)).sum();
    if total <= 1e-9 {
        return &themes[0];
    }
    let mut x = sim::derived_roll(fid, class, round, "blueprint_theme") * total;
    let mut last = &themes[0];
    for t in themes {
        last = t;
        x -= (t.weight + war * t.war_weight).max(0.0);
        if x <= 0.0 {
            return t;
        }
    }
    last
}

/// **设计图去重**：库里有没有一张同 `(舰级, 选装签名)` 的**自建**图可以复用？
///
/// 玩家钉的图不参与复用：那会让 AI 的建造区悄悄按玩家的设计生产，而玩家并没有要求这件事
/// （要让 AI 用玩家的图，玩家把建造区的指针指过去就行）。
fn signature_match(
    lib: &BTreeMap<BlueprintId, Control<Blueprint>>,
    class: &str,
    components: &[String],
) -> Option<BlueprintId> {
    if components.is_empty() {
        return None;
    }
    lib.iter()
        .find(|(_, leaf)| {
            !leaf.mode.is_player() && leaf.value.class == class && leaf.value.components == components
        })
        .map(|(name, _)| name.clone())
}
#[cfg(test)]
mod tests {
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
        state.control(fid.to_string()).map(|c| c.blueprints.clone()).unwrap_or_default()
    }

    /// **AI 会自己建图**（`Auto` 图那一层的执行者）：跑一趟之后，归它管的每个建造区都指着一张
    /// 图库里真实存在的图，舰级与建造区相等（口径 A），图的归属是 `Inherit`（流水）、
    /// 意图轴**沉默**（建图 ≠ 表态，Q1(c)）。
    #[test]
    fn the_ai_creates_a_design_for_every_yard_it_owns() {
        let (config, mut state) = fresh(42);
        let fid = "中国".to_string();
        let mut out = Vec::new();
        design_fleets(&mut state, &config, &mut out);
        assert!(!out.is_empty(), "AI 该给归它管的建造区建图");
        let l = lib(&state, &fid);
        assert!(!l.is_empty(), "{fid} 的图库该有图");
        for (city, bid, ptr) in yards(&state, &fid) {
            let name = ptr.unwrap_or_else(|| panic!("{city}#{bid} 没有被指到任何设计图"));
            let leaf = l.get(&name).unwrap_or_else(|| panic!("{city}#{bid} 指着一张不存在的图 {name}"));
            let class = state
                .city(&city)
                .unwrap()
                .buildings
                .iter()
                .find(|b| b.id == bid)
                .and_then(|b| b.ship_type.clone())
                .unwrap();
            assert_eq!(leaf.value.class, class, "口径 A：图的舰级必须与建造区相等");
            assert_eq!(leaf.mode, ControlMode::Inherit, "AI 写的是流水（这一层没有说话）");
            assert!(leaf.value.order.is_none(), "建图 ≠ 表态：AI 不写意图轴");
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
        design_fleets(&mut state, &config, &mut out);
        let first = lib(&state, &fid);
        assert!(!first.is_empty());
        // 幂等：同样的状态再跑一趟，图库一个字节都不长。
        let mut out2 = Vec::new();
        design_fleets(&mut state, &config, &mut out2);
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
                let class = second.get(&ptr).map(|l| l.value.class.clone()).unwrap_or_default();
                by_class.entry(class).or_default().insert(ptr);
            }
        }
        for (class, names) in by_class {
            assert_eq!(names.len(), 1, "同舰级的建造区该共用一张图（{class} 有 {names:?}）");
        }
    }

    /// **玩家的图一个字都不许动**：玩家把图钉成 `Player` 并让建造区指着它 ⇒ 那个建造区
    /// 整个被跳过（不改图、不改指针），回收也不碰它。
    #[test]
    fn a_player_pinned_design_and_its_yard_are_left_alone() {
        let (config, mut state) = fresh(42);
        let fid = "中国".to_string();
        let (city, bid, _) = yards(&state, &fid).into_iter().next().expect("中国有建造区");
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
        design_fleets(&mut state, &config, &mut out);
        let leaf = lib(&state, &fid).get("玩家的守卫图").cloned().expect("玩家的图还在");
        assert_eq!(leaf.mode, ControlMode::Player, "玩家的图归玩家");
        assert_eq!(leaf.value.components, vec!["kinetic", "ion_drive"], "玩家的选装一个字都不许动");
        let ptr = yards(&state, &fid)
            .into_iter()
            .find(|(c, b, _)| *c == city && *b == bid)
            .and_then(|(_, _, p)| p);
        assert_eq!(ptr.as_deref(), Some("玩家的守卫图"), "玩家指过去的建造区不许被改派");
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
        let (city, bid, _) = yards(&state, &fid).into_iter().next().expect("中国有建造区");
        if let Some(c) = state.city_mut(&city) {
            for b in &mut c.buildings {
                if b.id == bid {
                    b.blueprint = Some("已经删掉的图".to_string());
                }
            }
        }
        let mut out = Vec::new();
        design_fleets(&mut state, &config, &mut out);
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
            Control::player(Blueprint { class: "corvette".to_string(), components: vec![], order: None }),
        );
        let mut out = Vec::new();
        design_fleets(&mut state, &config, &mut out);
        let l = lib(&state, &fid);
        assert!(!l.contains_key("自动强袭·陈图"), "没人指向的自建图该被回收");
        assert!(l.contains_key("玩家自己的图"), "不是 AI 命名的图不许碰");
        assert!(l.contains_key("自动堡垒·玩家钉的"), "玩家钉住的图不许回收");
        assert!(out.iter().any(|d| d.action == "reaped" && d.blueprint == "自动强袭·陈图"));
    }

    /// **改图不影响已有的舰**（快照语义）：设计图重估了选装，已经下水的舰一个字节都不变，
    /// 而**之后**下水的舰按新图装配。
    #[test]
    fn retuning_a_design_never_touches_ships_already_in_space() {
        let (config, mut state) = fresh(42);
        let fid = "中国".to_string();
        let (city, bid, _) = yards(&state, &fid).into_iter().next().expect("中国有建造区");
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
        design_fleets(&mut state, &config, &mut out);
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
        design_fleets(&mut state, &config2, &mut out2);
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
        let (city, bid, _) = yards(&state, &fid).into_iter().next().expect("中国有建造区");
        let mut out = Vec::new();
        design_fleets(&mut state, &config, &mut out);
        let old = yards(&state, &fid)
            .into_iter()
            .find(|(c, b, _)| *c == city && *b == bid)
            .and_then(|(_, _, p)| p)
            .unwrap();
        // 把建造区改造成另一级（就像 `retool_shipyards` 做的那样）。
        let new_class = if lib(&state, &fid)[&old].value.class == "cruiser" { "corvette" } else { "cruiser" };
        if let Some(c) = state.city_mut(&city) {
            for b in &mut c.buildings {
                if b.id == bid {
                    b.ship_type = Some(new_class.to_string());
                }
            }
        }
        let mut out2 = Vec::new();
        design_fleets(&mut state, &config, &mut out2);
        let ptr = yards(&state, &fid)
            .into_iter()
            .find(|(c, b, _)| *c == city && *b == bid)
            .and_then(|(_, _, p)| p)
            .unwrap();
        let l = lib(&state, &fid);
        assert_eq!(
            l[&ptr].value.class,
            new_class,
            "图与建造区必须重新对齐（口径 A）"
        );
        assert_ne!(ptr, old, "舰级变了 ⇒ 换一张对应舰级的图（名字里就写着舰级）");
    }
}
