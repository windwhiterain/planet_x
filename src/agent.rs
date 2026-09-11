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

/// **面向 agent 的一回合世界视图**：只放**权威的世界字段**，直接复用模型自己的实体类型
/// （`Body`/`City`/`Faction`/`Ship`，以及 `GameEvent`/`ChronicleEntry` 那套叙事类型）。
/// 这里**没有**手写的镜像投影——JSON Schema 就是从同一批类型派生的，所以 schema 与吐出来的
/// JSON 同源、不可能漂移。可控制/操舵面（`control`/`scope`）**刻意不在这里**：
/// agent 在这里读世界，另走 `--apply` 操舵。
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
    /// 本回合的**视图**（[`RoundView`]）——世界观测（世界总量/政治/市场/每势力一行/每城一行）
    /// **加上本回合的过程量**（产出、舰队维护费、治理开销与覆盖率、市场运费/承运费/净进口，
    /// 以及 AI 的判定流水 [`RoundView::decisions`]）。这些过程量由步进时捕获，与模拟逐回合一致；
    /// 观测部分与直接状态同源，故与本节实体字段**严格一致**。
    pub view: RoundView,
}

/// The canonical, atomic per-round agent view as a `serde_json::Value` — one line
/// of the **trajectory** an agent queries over time with external `jq`. It is
/// deliberately lean (it carries only this chapter's `events`, not the cumulative
/// `story` chronicle) so a trajectory of thousands of rounds doesn't drag along a
/// copy of the narrative in every snapshot; the chronicle is delivered separately
/// as `--story` / the `story` field of the `--traj` pack. Floats are rounded to 2
/// decimals for token-noise reduction.
pub fn state_json(state: &State, view: &RoundView) -> serde_json::Value {
    let t = Trajectory {
        round: state.round,
        time_month: state.time_month,
        bodies: state.bodies.clone(),
        cities: state.cities.clone(),
        factions: state.factions.clone(),
        ships: state.ships.clone(),
        events: state.events.clone(),
        chronicle: state.chronicle.clone(),
        // 视图直接取自 `advance` 已算好的那一份（单一来源），不再重算一遍。
        view: view.clone(),
    };
    let mut v = serde_json::to_value(t).expect("trajectory is serializable");
    // 舰的**风格**在这里给**有效值**（叶 → 舰队默认 → 舰上记录值），不是 `Ship` 上那份记录：
    // 风格现在是活层，`--apply` 写的是叶片，所以直接序列化 `Ship` 只能读到出厂快照——agent
    // 会看到 `姿态: 0.0` 而以为是基线，实际上舰队默认早把它推成 -1.0 了。这类
    // 「读数不反映真实行为」正是本项目最忌讳的那种坑，所以在唯一的 agent 视图上就地改掉
    // （与 `round_value` 同一手法：序列化后统一修字段）。记录值仍完整地存在 checkpoint 里。
    // 放在 `round_value` **之前**，让这些值也一起按两位小数规整（避免 token 噪声）。
    if let Some(ships) = v.get_mut("ships").and_then(|s| s.as_array_mut()) {
        for (row, s) in ships.iter_mut().zip(state.ships.iter()) {
            row["风格"] = serde_json::to_value(state.ship_doctrine(s.name.clone()))
                .expect("doctrine is serializable");
            row["姿态"] = json!(state.ship_kiting(s.name.clone()));
            // 第三条风格轴（角色，三态 `War`/`Freight`/`Observe`）：同样给**有效值**——
            // 自动控制每回合会写这片叶（按积压 + 观测需求定编），所以 `Ship.role` 那份
            // 记录值常常不是它此刻的活。归属（谁说了算）在 `role_mode`。
            row["角色"] = json!(state.ship_role(s.name.clone()));
            row["role_mode"] = json!(state.ship_role_control(s.name.clone()).name());
        }
    }
    round_value(&mut v);
    v
}

/// A JSON Schema for the agent's per-round view (`Trajectory`), derived from the
/// same authoritative model types the state is rendered from — so it stays in sync
/// and self-describes the keys/types an agent may query (instead of memorising
/// them). Exposed via `--schema`.
pub fn schema_value() -> serde_json::Value {
    crate::schema::of::<Trajectory>()
}

/// A JSON Schema for the **canonical world** ([`State`] and every entity it embeds) ——
/// 与 [`schema_value`] 同一个道理，只是低一层：卡片与实体表上那些名词（`舰名`/`忠诚度`/
/// `所在天体`/`要价`…）的 `///` 注释就是它们在读面上的 `description`。
///
/// 为什么值得多这一份：悬停弹窗（`web/static/tip.js`）**只认名词**，不认来源表——
/// 于是「模型 `///` 就是唯一的产品文案」这件事成立，前端与 kit 都不必再维护翻译。
/// （代价：`State` 及其内嵌类型都要 `schemars::JsonSchema`；元组键字段另挂 `#[schemars(with)]`。）
pub fn state_schema_value() -> serde_json::Value {
    crate::schema::of::<State>()
}

/// **名词与解释**（悬停弹窗的语料）：三份 schema 合成一个值。
///
/// * `state`：规范世界（实体字段：`舰名`/`忠诚度`/`要价`…）；
/// * `view`：回合视图（`产出`/`维护`/`治理`…）；
/// * `projection`：投影每张表的列与 `column_docs`（含 `derived.*` 派生平铺表）；
/// * `control`：控制面（叶/命令/作用域键）。
///
/// 四份都要，因为它们各自覆盖**不同的名词**；合起来才是「界面上能出现的所有名词」。
/// 一份实现、两个出口：`--nouns`（CLI，数据级判据用它）与 `GET /api/schema`（web）。
/// 见 `web/static/tip.js`（前端只做「拿名词查解释」）与 `play/tests/g4_spec.py`（覆盖率判据）。
pub fn noun_schema_value() -> serde_json::Value {
    serde_json::json!({
        "state": state_schema_value(),
        "view": schema_value(),
        "projection": crate::projection::projection_schema(),
        // 控制面（写面）也要：控制行的名词（`首都`/`开发预算`… 的字段名，以及 `global`
        // 这类作用域键）住在它的 schema 里。**四半合起来**才是"界面上能出现的所有名词"。
        "control": crate::control::control_schema_value(),
        // **谁靠哪个字段认人**：也一并发出去，三端（Python 测试 / kit / 前端）都来问这里，
        // 不许各自维护镜像小表。`structs` 的唯一真值是 `model::IDENTITY`，
        // `tables` 的是投影自己的 `LAZY`（见 `projection::table_identity_keys`）。
        "identity": {
            "structs": crate::model::IDENTITY.iter()
                .map(|(s, f)| (s.to_string(), serde_json::Value::from(*f)))
                .collect::<serde_json::Map<String, serde_json::Value>>(),
            "tables": crate::projection::table_identity_keys(),
        },
    })
}

/// Zero-noise rendering of one state as a single-line JSON object.
pub fn render_state(state: &State, derived: &RoundView) -> String {
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
            "集货 collection（**非首都产出必须靠船运**）：首都天体的产出免运直接进池，其余落到**产地货栈**（`depots`：`(势力, 天体) → 库存`），必须有人开船把它运回首都才变成可用库存。谁跑运输是**第三条风格轴** `ships[].role`（三态 `War`/`Freight`/`Observe`；叶 → 舰队默认 → 舰上记录值，与前两条风格轴同形：AI 每回合按积压 + 观测需求定编并写叶、玩家把叶或舰队默认设成 Player 即可压住；它**只管派哪种活**——任何角色的舰照样自动开火、照样按 kiting 软移动）。观测那一态去引力异常区蹲着，喂 MOND 掌握度那条知识渠道（`autocontrol::knowledge`，优先级 观测 > 运输 > 战斗）。派单是**按积压占比抽签**（不是派去积压最大的那处），骰子由 (势力, 舰名, 回合) 派生、不消费主随机流。观察面：ships[].role/role_mode、factions[].observer_quota/observer_count/observer_target、cities[].depot_value、事件 cargo_loaded / cargo_delivered（货在舰上被击沉则随舰消失，没有单独事件）。",
            "承包 contracting（集货的**第二条路**：请人来运）：自己运力不够的势力（或**一艘舰都没有**的亡国残部）把「一个回合搬不动的积压」挂到承包市场（`contracts` 表：托运方 / 承运方 / 资源 / 数量 / 从哪到哪 / 抽成 share / 截止期；`carrier=null` 表示还没人接）。**报酬是抽成**：承运人交付时从货里自留 share，其余进托运方首都池——**没有货币转移**，所以没有汇率、没有通胀，也不会递归收费。**砸单不赔货值**（只扣信誉）：于是 `factions[].reputation`（势力级、公开）是这条腿上**唯一的抵押品**——托运方靠它决定敢不敢把货交给一个陌生人，低信誉者结构上接不到贵单/难单。超期**不作废**（只扣一次信誉，货照运、抽成照拿）；没人接的过期单会被托运方收回（没有任何承诺，不扣信誉）。禁运同样挡承包（不给你运货）。",
            "运费与 MOND 承运 freight/mond：**注意这与上面的集货/承包是两件事**。这里是**抽象市场运费**——成交价再乘运费率 = freight_per_au × 买卖双方首都距离，**要穿越 28 AU 引力异常带**再按浸入深度加 mond_freight_mult 倍；非 master 的货走那条线会**按深度丢货**（确定性比例，见 mond_loss_per_au），只有掌握了 MOND 的 master 能可靠承运、并对这条线上的运费**抽税**（carrier_share）。它发生在**成交瞬间**（货是瞬移的），而集货/承包是**真实的舰船航线**（货真的在路上，会被拦、会随舰沉没）。观察面：metrics.factions[].freight_paid / carrier_income——后者只有 master 会 >0，那是柯伊伯带贸易的垄断租金。",
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
        "freight": config_json(&config.freight),
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
    let bpos = state
        .body(body_id)
        .map(|b| b.position)
        .unwrap_or([0.0, 0.0]);
    let cpos = state
        .body(&capital)
        .map(|b| b.position)
        .unwrap_or([0.0, 0.0]);
    ((bpos[0] - cpos[0]).powi(2) + (bpos[1] - cpos[1]).powi(2)).sqrt()
}

#[cfg(test)]
#[path = "tests/agent.rs"]
mod tests;
