//! Machine-readable ("zero-noise") rendering for an LLM agent player.
//!
//! The CLI's default human output (ASCII star map, comfy-table with Unicode
//! box-drawing characters, colored legend, Chinese prose) is optimised for a
//! terminal and is hostile to a machine. This module is the exact opposite:
//! it renders a single [`State`] as one compact, stable JSON object with no
//! colour, no tables, no decoration, and values rounded to kill token noise.
//!
//! Emit one object per line (JSON Lines): `round 0` first, then one per round
//! as the simulation advances. Field order is fixed and stable; floats are
//! rounded to 2 decimals; empty resource buckets and sub-threshold values are
//! dropped so the document stays small.

use crate::model::*;
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeMap;

/// Round a float to 2 decimals (token-noise reduction). `+ 0.0` normalizes the IEEE
/// `-0.0` that `f64::round` preserves (negative zero reads as `-0.0` to an agent).
fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0 + 0.0
}

/// The agent-facing view of one world state: the **authoritative world fields**,
/// reusing the model's own entity types (`Body`/`City`/`Faction`/`Ship` and the
/// `GameEvent`/`ChronicleEntry` narrative types). There is no hand-written mirror
/// projection — the JSON Schema is derived straight from these same types, so the
/// schema and the emitted JSON are one sourced and can never drift. The
/// controllable/steering surface (`control`/`scope`) is deliberately omitted: an
/// agent reads the world here, and steers it separately via `--apply`.
#[derive(Serialize, JsonSchema)]
pub struct Trajectory {
    /// 当前回合（从 0 起：回合 0 = 初始世界，之后每步 +1）。
    pub round: u32,
    /// 已流逝的总月数（= round，浮点，便于与时间序列计算）。
    pub time_month: f64,
    /// 天体：轨道 + 定居点 + 已建城市（每个 body 一对象）。
    pub bodies: Vec<Body>,
    /// 城市：位置/人口/建筑/所属（每个 city 一对象）。
    pub cities: Vec<City>,
    /// 势力：资源库存/外交关系/投资与建造预算（每个 faction 一对象）。
    pub factions: Vec<Faction>,
    /// 飞船：坐标/舰级/耐久/阵营/当前命中与目标（每艘 ship 一对象）。
    /// `doctrine`/`kiting` 是**有效值**（叶 → 舰队默认 → 舰上记录值，引擎解析），
    /// 不是舰上那份出厂快照——见 `state_json` 的注释。
    pub ships: Vec<Ship>,
    /// 本回合事件（谁开火/被毁/城被夷平/殖民/战争/剧情…）。
    pub events: Vec<GameEvent>,
    /// 剧情编年史：整段已展开的叙事弧。
    pub chronicle: Vec<ChronicleEntry>,
    /// 本回合的**总结指标**（[`crate::sim::round_metrics`] 计算的中间聚合量）：存量/政治
    /// （实力占比/霸权/联盟/制裁/交战/世界总量）+ **流量**（各势力·各城的开采产出、舰队
    /// 维护费、治理开销与覆盖率——由步进时捕获的中间量，与模拟逐回合一致）。与直接状态
    /// 同源自步进函数，故与本节实体字段**严格一致**。
    pub metrics: RoundMetrics,
}

/// The canonical, atomic per-round agent view as a `serde_json::Value` — one line
/// of the **trajectory** an agent queries over time with external `jq`. It is
/// deliberately lean (it carries only this chapter's `events`, not the cumulative
/// `story` chronicle) so a trajectory of thousands of rounds doesn't drag along a
/// copy of the narrative in every snapshot; the chronicle is delivered separately
/// as `--story` / the `story` field of the `--traj` pack. Floats are rounded to 2
/// decimals for token-noise reduction.
pub fn state_json(state: &State, derived: &Derived) -> serde_json::Value {
    let t = Trajectory {
        round: state.round,
        time_month: state.time_month,
        bodies: state.bodies.clone(),
        cities: state.cities.clone(),
        factions: state.factions.clone(),
        ships: state.ships.clone(),
        events: state.events.clone(),
        chronicle: state.chronicle.clone(),
        // `metrics` 直接取自 `advance` 已算好的 `Derived::metrics`（单一来源），不再重算一遍。
        metrics: derived.metrics.clone(),
    };
    let mut v = serde_json::to_value(t).expect("trajectory is serializable");
    // 舰的**风格**在这里给**有效值**（叶 → 舰队默认 → 舰上记录值），不是 `Ship` 上那份记录：
    // 风格现在是活层，`--apply` 写的是叶片，所以直接序列化 `Ship` 只能读到出厂快照——agent
    // 会看到 `kiting: 0.0` 而以为是基线，实际上舰队默认早把它推成 -1.0 了。这类
    // 「读数不反映真实行为」正是本项目最忌讳的那种坑，所以在唯一的 agent 视图上就地改掉
    // （与 `round_value` 同一手法：序列化后统一修字段）。记录值仍完整地存在 checkpoint 里。
    // 放在 `round_value` **之前**，让这些值也一起按两位小数规整（避免 token 噪声）。
    if let Some(ships) = v.get_mut("ships").and_then(|s| s.as_array_mut()) {
        for (row, s) in ships.iter_mut().zip(state.ships.iter()) {
            row["doctrine"] = serde_json::to_value(state.ship_doctrine(s.name.clone())).expect("doctrine is serializable");
            row["kiting"] = json!(state.ship_kiting(s.name.clone()));
            // 第三条风格轴（角色）：`true` = 运输舰。同样给**有效值**——自动控制每回合会写
            // 这片叶（按积压定编），所以 `Ship.freighter` 那份记录值常常不是它此刻的活。
            row["freighter"] = json!(state.ship_freighter(s.name.clone()));
            row["freighter_mode"] = json!(state.ship_freighter_control(s.name.clone()).name());
        }
    }
    round_value(&mut v);
    v
}

/// The story chronicle (`State::chronicle`) as a JSON array, for `story` /
/// `.story` queries. This is the full, growing narrative arc of the run.
pub fn story_value(state: &State) -> serde_json::Value {
    serde_json::to_value(&state.chronicle).expect("chronicle is serializable")
}

/// A JSON Schema for the agent's per-round view (`Trajectory`), derived from the
/// same authoritative model types the state is rendered from — so it stays in sync
/// and self-describes the keys/types an agent may query (instead of memorising
/// them). Exposed via `--schema`.
pub fn schema_value() -> serde_json::Value {
    let schema = schemars::schema_for!(Trajectory);
    serde_json::to_value(schema).expect("schema is serializable")
}

/// Zero-noise rendering of one state as a single-line JSON object.
pub fn render_state(state: &State, derived: &Derived) -> String {
    state_json(state, derived).to_string()
}

/// The game's full tunable configuration, rendered as one JSON object for the
/// `meta` command / `--meta` flag. This is the agent's "rules dictionary":
///
/// * `structures`  structure *key* → full spec (混凝土/钢结构).
/// * `buildings`  building *kind* → full spec (role, construction speed/cost,
///                staffing, productivity, default invest weight).
/// * `ships`      ship *class* → full spec (hull, hull_regen, attack, speed,
///                range, build points/cost). An agent needs these to decide
///                what to build.
/// * `economy` / `combat` / `diplomacy`  the numeric tuning constants
///                (`invest_fraction`, `war_threshold`, `production_rate`, …).
///
/// Keys are WYSIWYG: the resource key *is* its readable Chinese name, so what an
/// agent sees in the state view is exactly the key it writes in a `--apply` diff.
/// There is no raw-key ↔ 中文 translation table (that was `resources`, now gone);
/// `market.resource_value` lists every resource name → value for discovery.
///
/// Everything except `story` is rendered by [`config_json`], i.e. derived straight
/// from the config structs — so the rules dictionary can never drift from
/// `config/game.ron`. `story` maps trigger/effects into a flat readable shape.
///
/// Round every float in a JSON tree to 2 decimals (agent token-noise reduction).
/// Integer numbers (ids, `slots`, `min_members`, …) are left untouched so they
/// don't render as `3.0`.
fn round_value(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Number(n) if n.is_f64() => {
            if let Some(f) = n.as_f64() {
                // + 0.0 规整 -0.0 → +0.0（round 保留负零）。
                *v = serde_json::Value::from((f * 100.0).round() / 100.0 + 0.0);
            }
        }
        serde_json::Value::Array(a) => a.iter_mut().for_each(round_value),
        serde_json::Value::Object(m) => m.values_mut().for_each(round_value),
        _ => {}
    }
}

/// Serialize any serializable config value to JSON with floats already rounded
/// to 2 decimals. This is the single-source renderer for `meta_value`'s config
/// sections: derive from the struct, never transcribe a field list.
fn config_json<T: serde::Serialize>(value: &T) -> serde_json::Value {
    let mut v = serde_json::to_value(value).expect("config value is serializable");
    round_value(&mut v);
    v
}

pub fn meta_value(config: &GameConfig) -> serde_json::Value {
    // `market` = 当前配置结构 + 由资源定义派生的每资源价值表（key = 资源的可读名）。
    // 资源 key 本身就是可读名（WYSIWYG），所以不再需要 raw→中文 翻译表。
    let mut market = config_json(&config.market);
    if let serde_json::Value::Object(m) = &mut market {
        m.insert(
            "resource_value".to_string(),
            config_json(
                &config
                    .resources
                    .iter()
                    .map(|(k, r)| (k.clone(), r.value))
                    .collect::<BTreeMap<_, _>>(),
            ),
        );
    }

    json!({
        "notes": [
            "budget：造舰预算 construction_budget = 库存×invest_fraction，但**先留维护底线**：从库存里预留 upkeep×upkeep_reserve_mult 的市场价值，只把超出部分用于造舰（'把海军养在经济能承受的规模'）。投资预算 investment_budget = 库存×invest_fraction，不受该保留约束。只有叶子的 mode=Player 时命令的 value 才被采用；mode=Auto 时系统每回合按上式重算；mode=Inherit 时沿作用域链上溯（全链没人表态则落到 Auto）。",
            "每回合净流 ≈ 产出 production_value − 舰队维护 upkeep − 治理开销 governance_cost。为负则库存持续下降（清算），最终舰队生锈（护甲扣到 0 报废）、城市治理不到位而降忠诚→叛乱夷平。用 --control-plan [faction] 看该势力的剖面（净流/可养舰队上限/清算前剩余回合）。",
            "生产 production：采矿建筑按面积×labor×productivity×production_rate 出矿；人口限制劳动效率（min_efficiency 下限）。治理 governance：行政成本 = (admin_base + admin_per_au×距首都距离)×人口超载倍率 + 娱乐预算，用库存按价值加权支付，覆盖率<1 则忠诚下跌。",
            "迁都（capital，controllable）：`capital` 叶子带 mode（Player=你说的算，Auto=系统周期性重估，Inherit=沿作用域链上溯）。首都=治理/本土防御锚点；首都人口占全势力人口的比例越高，全国每城目标忠诚加成越大（capital_share_loyalty_buff）；迁都则按「旧首都人口占比」扣全国忠诚（capital_share_relocate_cost）——迁都是为了省治理距离成本，却是以全国忠诚为赌注的豪赌，不是免费优化。首都亡城（其上已无本势力活城）会被立即强迁到人口最高的活城。",
            "市场 market：**真实交换所**，不是常数价贩卖机。价格 = 基价 × (coverage_rounds/覆盖回合数)^price_alpha，其中覆盖回合数 = 世界总库存 ÷ 实测消费率（由库存差量出来）——稀缺顶到 price_ceiling（默认 8×）、过剩折到 price_floor。供给是**别人真的拿出来卖的富余**（挂单记名卖家），卖光就买不到；成交价再乘**关系倍率**（对敌最多 1+hostile_price_markup 倍，对友打 friendly_price_discount 折）。**关系冷到 embargo_relation、交战、或「已倒向联盟的弱者 ↔ 被锁定的霸权」即全面禁运**——那个卖家的所有资源对你都不存在（见 metrics.factions[<你>].trade_blocked_by）。付款=把自己可出口的实物交割给对方，另按 spread 烧掉一笔（真实价值 sink）。",
            "运费与 MOND 承运 freight/mond：货物**不是瞬移**——成交价再乘运费率 = freight_per_au × 买卖双方首都距离，**要穿越 28 AU 引力异常带**再按浸入深度加 mond_freight_mult 倍；非 master 的货走那条线会**按深度丢货**（确定性比例，见 mond_loss_per_au），只有掌握了 MOND 的 master 能可靠承运、并对这条线上的运费**抽税**（carrier_share）。观察面：metrics.factions[].freight_paid / carrier_income——后者只有 master 会 >0，那是柯伊伯带贸易的垄断租金。",
            "实体身份=唯一名字（WYSIWYG 资源 key 即可读中文名），无 numeric shadow id；schema 由同一批结构体派生（--schema / --control-schema 自描述）。"
        ],
        "structures": config_json(&config.structures),
        "buildings": config_json(&config.buildings),
        "ships": config_json(&config.ships),
        "components": config_json(&config.components),
        "economy": config_json(&config.economy),
        "combat": config_json(&config.combat),
        "diplomacy": config_json(&config.diplomacy),
        "market": market,
        "governance": config_json(&config.governance),
        "mond": config_json(&config.mond),
        "balance": config_json(&config.balance),
        "story": config
            .story
            .iter()
            .map(|s| {
                let trigger = match &s.trigger {
                    StoryTrigger::RoundAt { round } => json!({"kind": "round_at", "round": round}),
                    StoryTrigger::FirstWar => json!({"kind": "first_war"}),
                    StoryTrigger::FirstRaze => json!({"kind": "first_raze"}),
                    StoryTrigger::FirstColony => json!({"kind": "first_colony"}),
                    StoryTrigger::WarBetween { a, b } => json!({"kind": "war_between", "a": a, "b": b}),
                    StoryTrigger::FactionAtWar { faction } => json!({"kind": "faction_at_war", "faction": faction}),
                    StoryTrigger::RelationBelow { a, b, value } => {
                        json!({"kind": "relation_below", "a": a, "b": b, "value": r2(*value)})
                    }
                };
                let effects = s
                    .effects
                    .iter()
                    .map(|e| match e {
                        StoryEffect::Relations { a, b, delta } => {
                            json!({"kind": "relations", "a": a, "b": b, "delta": r2(*delta)})
                        }
                        StoryEffect::GrantResources { faction, resource, amount } => {
                            json!({"kind": "grant_resources", "faction": faction, "resource": resource, "amount": r2(*amount)})
                        }
                        StoryEffect::GrantShip { faction, class, body } => {
                            json!({"kind": "grant_ship", "faction": faction, "class": class, "body": body})
                        }
                    })
                    .collect::<Vec<_>>();
                json!({"id": s.id, "title": s.title, "trigger": trigger, "effects": effects})
            })
            .collect::<Vec<_>>(),
    })
}

/// 一座城（其宿主天体 `body_id`）到其统治势力首都天体的距离（AU）——可读的治理压力
/// 信号：越远，管理越难、忠诚越易跌破叛变阈值。无主/首都缺失时返回 0。
///
/// 身份即名字：`owner`/`body_id` 都是势力的名字/天体名（唯一 key）。
pub fn governance_distance(state: &State, owner: &str, body_id: &str) -> f64 {
    let capital = state.capital_body(owner);
    let bpos = state.body(body_id).map(|b| b.position).unwrap_or([0.0, 0.0]);
    let cpos = state.body(&capital).map(|b| b.position).unwrap_or([0.0, 0.0]);
    ((bpos[0] - cpos[0]).powi(2) + (bpos[1] - cpos[1]).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    /// 风格是**活层**：agent 视图（`state_json`）必须给**有效值**，而不是 `Ship` 上那份
    /// 出厂快照——否则 agent 读到 `kiting: 0.0`、以为还是基线，实际舰队默认早已把它推成
    /// `-1.0`（又一次「读数不反映真实行为」）。
    #[test]
    fn state_view_reports_effective_doctrine_and_kiting() {
        let cfg = config::load_config();
        let mut state = crate::world::default_state(&cfg, 7);
        let (fid, name) = state
            .ships
            .iter()
            .find(|s| s.faction_id == "中国")
            .map(|s| (s.faction_id.clone(), s.name.clone()))
            .expect("中国 has a starting ship");
        let record_kiting = state.ship(&name).unwrap().kiting;
        let record_doctrine = state.ship(&name).unwrap().doctrine;

        let diff = serde_json::json!({
            "control": [{"faction_id": fid,
                "default_kiting": {"kiting": -1.0, "mode": "Player"},
                // `default_doctrine` 是**两轴一片叶**：这片叶还不存在时必须两条轴一起给
                // （只给一条 ⇒ `partial_doctrine_leaf` 拒绝，见 control.rs）。
                "default_doctrine": {"temper": 0.5, "lone_wolf": 0.0, "mode": "Player"}}]
        });
        crate::control::apply_patch(&mut state, &cfg, &diff).expect("默认风格 applies");

        let v = state_json(&state, &crate::sim::derived_from_state(&state, &cfg));
        let row = v["ships"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["name"] == serde_json::json!(name))
            .expect("该舰在视图里");
        assert_eq!(row["kiting"], serde_json::json!(-1.0), "视图必须给有效姿态（舰队默认）");
        assert_eq!(row["doctrine"]["temper"], serde_json::json!(0.5), "视图必须给有效风格");
        // 记录值不动（它仍是出厂快照）——这正是"视图不能直接序列化 Ship"的原因。
        assert_eq!(state.ship(&name).unwrap().kiting, record_kiting);
        assert_eq!(state.ship(&name).unwrap().doctrine, record_doctrine);
    }

    /// `meta_value` 是让 agent 读的「规则字典」。它必须从 config 结构体**派生**，
    /// 而不是手写字段清单——此守卫确保新增的 config 字段（尤其是 `combat` 那些）
    /// 一定出现在 meta 里，防止再次像 `component_spill` 那样「配置有、meta 无」。
    #[test]
    fn meta_derives_all_combat_fields_and_keeps_ints() {
        let cfg = config::load_config();
        let m = meta_value(&cfg);
        let combat = m.get("combat").and_then(|v| v.as_object()).expect("combat section");
        for f in [
            "component_spill",
            "component_repair",
            "escort_range",
            "pursuit_range",
            "pd_radius",
        ] {
            assert!(combat.contains_key(f), "meta.combat 缺少 {f}（曾被手写清单漏掉）");
        }
        // 整数型配置字段必须保持整数，不能因圆整变成 `2.0`。
        let slots = &m["ships"]["corvette"]["slots"];
        assert!(slots.is_i64() || slots.is_u64(), "slots 应为整数，实为 {slots}");
    }

    /// 规则字典 `meta` 必须**覆盖每一个配置段**（它从 config 派生，不手写）。任何 config
    /// 字段都必须出现在对应的 meta 段里——这直接拦住「配置加了字段、meta 忘了加」这类漂移
    /// （正是当初 `combat.component_spill` 消失的那种 bug）。
    #[test]
    fn meta_value_covers_every_config_section() {
        fn key_set(v: &serde_json::Value) -> std::collections::BTreeSet<String> {
            v.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default()
        }
        fn assert_covers(m: &serde_json::Value, key: &str, cfgv: &serde_json::Value) {
            let meta_sec = m.get(key).unwrap_or_else(|| panic!("meta 缺 {key} 段"));
            let cfg_keys = key_set(cfgv);
            let meta_keys = key_set(meta_sec);
            let missing: Vec<_> = cfg_keys.difference(&meta_keys).cloned().collect();
            assert!(missing.is_empty(), "meta.{key} 遗漏 config 字段: {missing:?}");
        }
        let cfg = config::load_config();
        let m = meta_value(&cfg);
        // 结构体段：meta 用 config_json 全量派生，key 集合必须覆盖 config 的集合。
        assert_covers(&m, "economy", &serde_json::to_value(&cfg.economy).unwrap());
        assert_covers(&m, "combat", &serde_json::to_value(&cfg.combat).unwrap());
        assert_covers(&m, "diplomacy", &serde_json::to_value(&cfg.diplomacy).unwrap());
        assert_covers(&m, "market", &serde_json::to_value(&cfg.market).unwrap());
        assert_covers(&m, "governance", &serde_json::to_value(&cfg.governance).unwrap());
        assert_covers(&m, "mond", &serde_json::to_value(&cfg.mond).unwrap());
        assert_covers(&m, "balance", &serde_json::to_value(&cfg.balance).unwrap());
        // 字符串键表段：meta 的段 key == config 表的 key。
        assert_covers(&m, "structures", &serde_json::to_value(&cfg.structures).unwrap());
        assert_covers(&m, "buildings", &serde_json::to_value(&cfg.buildings).unwrap());
        assert_covers(&m, "ships", &serde_json::to_value(&cfg.ships).unwrap());
        assert_covers(&m, "components", &serde_json::to_value(&cfg.components).unwrap());
    }

    /// agent 视图（`state_json` 发射的 JSON）必须被 `schema_value()` **自描述**：发射的
    /// 顶层 key 都出现在 schema 的 `properties` 里，且实体数组被声明。因为 schema 与 JSON
    /// 都由同一批权威类型（`Trajectory` + 模型类型）派生，这个守卫把「schema↔发射不一致」
    /// 显式化，防止未来有人用另一个生产者替换掉 `state_json`。
    #[test]
    fn agent_view_is_self_described_by_schema() {
        let cfg = config::load_config();
        let state = crate::world::default_state(&cfg, 42);
        let v = state_json(&state, &crate::sim::derived_from_state(&state, &cfg));
        let schema = schema_value();
        let props = schema.get("properties").and_then(|p| p.as_object()).expect("schema.properties");
        let emit_keys = v
            .as_object()
            .map(|o| o.keys().cloned().collect::<std::collections::BTreeSet<_>>())
            .unwrap_or_default();
        let schema_keys = props.keys().cloned().collect::<std::collections::BTreeSet<_>>();
        let missing: Vec<_> = emit_keys.difference(&schema_keys).cloned().collect();
        assert!(missing.is_empty(), "agent 视图发射了 schema 未描述的字段: {missing:?}");
        for k in ["bodies", "cities", "factions", "ships", "events", "chronicle", "metrics"] {
            assert!(props.contains_key(k), "schema 缺字段 {k}");
        }
        // 总结指标 `metrics` 必须真的出现在发射的 JSON 里（不是空壳），且是实体视图的一部分。
        assert!(
            v.get("metrics").is_some_and(|m| m.get("factions").is_some()),
            "agent 视图必须携带 metrics 总结（含各势力聚合）"
        );
        // metrics 必须携带新增的**流量**字段（产出/维护/治理 + 每城产出）。
        let m = &v["metrics"];
        let sample_fac = m
            .get("factions")
            .and_then(|f| f.as_object())
            .and_then(|o| o.values().next())
            .cloned()
            .unwrap_or_default();
        for k in ["production_value", "production", "upkeep", "governance_cost", "governance_coverage"] {
            assert!(sample_fac.get(k).is_some(), "metrics.factions 缺流量字段 {k}");
        }
        assert!(m.get("city_production").is_some(), "metrics 缺每城产出 city_production");
    }
}
