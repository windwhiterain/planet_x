//! 读面字段的**中性值**（= 缺省值）：**一处声明**。
//!
//! # 为什么必须只有一处
//!
//! 历史上 `flow.jsonl` 说「治理覆盖 0%」而同回合的 `metrics` 说「100%」（见
//! `notes/pre-post-unify.md` §1）——根因**不是**「流里缺了键」，而是**缺省值由每个读者自己编**：
//! 读流的人补 0，读汇总的人按约定补 1.0，于是同一回合两个读面各说各话。
//!
//! 所以修法不是「哪儿都别缺键」，而是**把缺省的所有权收归一处**：
//!
//! 1. 本模块 [`READ_FACE_NEUTRALS`] 声明读面（[`RoundView`](crate::model::RoundView)）**每一个叶子
//!    字段**的中性值；
//! 2. 引擎的运行时缺省**用这里的具名常量**（[`value`]），不在别处再写那些字面量；
//! 3. `schema.json` 的 `neutral` 段把它发给**所有**外部读者（kit / web / 别的语言）；
//! 4. 三条测试钉住它（见 `src/tests/model/neutral.rs`）：
//!    * `every_read_face_field_declares_a_neutral`——用 schemars 遍历读面结构，
//!      **新加字段必须同时声明中性值**（两边集合相等，且类型相容）；
//!    * `declared_neutral_matches_the_engine_pre_face`——拿一个真实世界做实证：
//!      「这一步还没跑」时引擎吐出来的值必须**逐字段等于**声明；
//!    * `schema_publishes_the_neutral_table`——发出去的 schema 段等于本表（发布路径不许漂）。
//!
//! # 语义
//!
//! 字段处于**中性值** ⇔ 「这一步还没跑 / 这件事没发生」，**不是**「它的值是零」。
//! 所以 `governance_scale` 的中性值是 **1.0**（未超载）而不是 0——写 0 会被读成「治理能力归零」；
//! `Option` 的中性值是 `null`（表达「**没算**」），而不是拿 0 冒充「算了，得 0」。
//!
//! ⚠ **数组的条目不做叶子级声明**：`decisions.capital` 这样的稀疏数组，整条存在或整条缺席，
//! 没有「条目里的某个字段缺了」这种读法——所以它的中性值是 `[]`，条目内部不逐字段声明。

/// 一个字段的中性值（缺省值）。
///
/// 只有这几种：够表达读面全部叶子字段，且每一种都对应 schemars 里的一种类型。
/// （**不**在这里表达「嵌套结构的整体中性值」——中性值是**叶子级**的：`factions[].capital`
/// 的中性值由它七个叶子各自的声明决定，不需要第八条声明。）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Neutral {
    /// 整数 `0`——**计数类**字段（城数/舰数/人口/禁运家数）。
    /// 与 [`Neutral::Zero`] 分开是因为 `serde_json` 区分 `0` 与 `0.0`，而 Python 读者拿它当
    /// 缺键替补时，整数列补 `0.0` 会把那一列变成 float（lazy 表给 `production` 补 `{}` 是同一条理由）。
    ZeroInt,
    /// 浮点 `0.0`——价值/比率类字段。
    Zero,
    /// 数值 `1`：中性**倍率/覆盖率**——「没有账」而不是「能力归零」。
    One,
    /// `false`。
    False,
    /// `null`（`Option::None`）：**没算**，区别于「算了得 0」。
    Null,
    /// `{}`。
    EmptyMap,
    /// `[]`。
    EmptyArray,
}

impl Neutral {
    /// 序列化成 JSON（就是「缺键时该读到的值」）。
    pub fn to_json(self) -> serde_json::Value {
        match self {
            Neutral::ZeroInt => serde_json::json!(0),
            Neutral::Zero => serde_json::json!(0.0),
            Neutral::One => serde_json::json!(1.0),
            Neutral::False => serde_json::json!(false),
            Neutral::Null => serde_json::Value::Null,
            Neutral::EmptyMap => serde_json::json!({}),
            Neutral::EmptyArray => serde_json::json!([]),
        }
    }

    /// 这一档在 schema 里对应哪一类 JSON 值：`number` / `boolean` / `null` / `object` / `array`。
    pub fn kind(self) -> &'static str {
        match self {
            Neutral::ZeroInt => "integer",
            Neutral::Zero | Neutral::One => "number",
            Neutral::False => "boolean",
            Neutral::Null => "null",
            Neutral::EmptyMap => "object",
            Neutral::EmptyArray => "array",
        }
    }

    /// 是不是数值档。
    pub fn is_numeric(self) -> bool {
        matches!(self, Neutral::ZeroInt | Neutral::Zero | Neutral::One)
    }
}

/// 引擎运行时缺省用的**具名常量**——值全部来自本模块，别在别处再写这些字面量。
///
/// 它们与 [`READ_FACE_NEUTRALS`] 的一致性由测试钉住（`value_consts_match_the_table`）：
/// 常量是给 Rust 代码读的（编译期、零成本），表是给 schema 与外部读者读的，**两者必须同值**。
pub mod value {
    /// 治理覆盖率的中性值：没有账要付 ⇒ 覆盖得住（**不是**「没覆盖」）。
    pub const GOVERNANCE_COVERAGE: f64 = 1.0;
    /// 人口超载倍率的中性值：未超载。
    pub const GOVERNANCE_SCALE: f64 = 1.0;
    /// 用工系数的中性值：**不缺人手**（没有用工缺口 ⇒ 不打折）。同上面两个 1.0 的道理——
    /// 写 0 会被读成「全城没人上工」，那是另一回事。
    pub const CITY_LABOR: f64 = 1.0;
}

/// 读面（`RoundView`）每个**叶子字段**的中性值。
///
/// 路径相对读面根（= `main.jsonl` 每行的 `view`、`--derived` 的 `pre`/`post`）；
/// map / 数组的值用 `[]` 表示，例如 `factions[].governance_scale` 指「每个势力那一行里
/// 的 `governance_scale`」。
///
/// ⚠ 加字段就要加一行——`every_read_face_field_declares_a_neutral` 会红。
pub const READ_FACE_NEUTRALS: &[(&str, Neutral)] = &[
    // ── 世界总量（观测）──
    ("city_count", Neutral::ZeroInt),
    ("ship_count", Neutral::ZeroInt),
    ("fleet_value", Neutral::Zero),
    ("population", Neutral::ZeroInt),
    // ── 政治（观测）──
    ("power_share", Neutral::EmptyMap),
    ("faction_power", Neutral::EmptyMap),
    ("hegemon", Neutral::Null),
    ("coalition_members", Neutral::EmptyArray),
    ("sanctioned", Neutral::Null),
    ("wars", Neutral::EmptyArray),
    // ── 市场（观测）──
    ("market_price", Neutral::EmptyMap),
    ("market_settled", Neutral::EmptyMap),
    ("market_offered", Neutral::EmptyMap),
    // ── 每势力一行 / 每城一行 ──
    ("factions", Neutral::EmptyMap),
    ("cities", Neutral::EmptyMap),
    // ── 本回合的结算事实（过程；`pre` 里为空）──
    // 一笔成交一行 / 一艘在跑运输的舰一行——两者都是**稀疏**的：没成交、没跑运输就是空的。
    ("market_trades", Neutral::EmptyArray),
    ("haul_steps", Neutral::EmptyMap),
    // ── AI 的判定（过程；`pre` 里为空）──
    ("decisions.ships", Neutral::EmptyArray),
    ("decisions.retools", Neutral::EmptyArray),
    ("decisions.styles", Neutral::EmptyArray),
    ("decisions.blueprints", Neutral::EmptyArray),
    // 首都评估/迁都是**稀疏**的判定：大多数回合是空数组（条目内部字段不需要声明——
    // 「整条存在 / 整条缺席」，不存在「条目里的字段缺了」这种读法）。
    ("decisions.capital", Neutral::EmptyArray),
    // ── FactionRow：观测 ──
    ("factions[].city_count", Neutral::ZeroInt),
    ("factions[].ship_count", Neutral::ZeroInt),
    ("factions[].fleet_value", Neutral::Zero),
    ("factions[].population", Neutral::ZeroInt),
    ("factions[].market_value", Neutral::Zero),
    ("factions[].at_war", Neutral::False),
    // B3：这列从「计数」升级成「名单 + 原因」（`{对方势力: war|cold|coalition}`）；
    // 空 map = 谁都跟我做生意（不是「没算过」——它本来就是从 state 现算的观测）。
    ("factions[].trade_blocked_by", Neutral::EmptyMap),
    // ── FactionRow：过程 ──
    ("factions[].production", Neutral::EmptyMap),
    ("factions[].production_value", Neutral::Zero),
    ("factions[].upkeep", Neutral::Zero),
    ("factions[].governance_cost", Neutral::Zero),
    // 覆盖率的中性值是 **1.0**：零城势力「无账可付」，不是「付不起」。
    ("factions[].governance_coverage", Neutral::One),
    ("factions[].governance_admin", Neutral::Zero),
    ("factions[].governance_entertainment", Neutral::Zero),
    // 超载倍率的中性值是 **1.0**（未超载），不是 0。
    ("factions[].governance_scale", Neutral::One),
    ("factions[].ideology_loyalty_penalty", Neutral::Zero),
    ("factions[].capital_loyalty_bonus", Neutral::Zero),
    // ── FactionRow：贸易 ──
    ("factions[].freight_paid", Neutral::Zero),
    ("factions[].carrier_income", Neutral::Zero),
    ("factions[].net_import", Neutral::Zero),
    // ── FactionRow：钱去哪了（B2）──
    // 「批了多少」是控制面的持久叶（`control` 的 investment_budget/construction_budget），
    // 不在读面里；这里只有「真花掉的」，所以它的中性值是空 map（不是「没批」）。
    ("factions[].investment_spent", Neutral::EmptyMap),
    ("factions[].construction_spent", Neutral::EmptyMap),
    ("factions[].upkeep_unpaid", Neutral::Zero),
    ("factions[].fleet_rust", Neutral::Zero),
    // ── FactionRow：市场里的位置 + 集货运力账（B3）──
    ("factions[].purchasing_power", Neutral::Zero),
    // **名次的中性值是 `null`**：`None` = 这一回合没排过队（`pre` 面）。写 0 会被读成
    // 「第一个挑」——那是实打实的一个名次，不是「还没排队」（同 `hegemon: Option` 的约定）。
    ("factions[].market_rank", Neutral::Null),
    // 运力账是稀疏的（没积压的货栈不占键）；条目内部**逐字段**声明，因为一旦有键，
    // 四个数就是完整的（不存在「条目里某个字段缺了」的读法）。
    ("factions[].freight_gap", Neutral::EmptyMap),
    ("factions[].freight_gap[].need", Neutral::Zero),
    ("factions[].freight_gap[].own", Neutral::Zero),
    ("factions[].freight_gap[].hired", Neutral::Zero),
    ("factions[].freight_gap[].uncovered", Neutral::Zero),
    // ── CityRow ──
    ("cities[].population", Neutral::ZeroInt),
    ("cities[].loyalty", Neutral::Zero),
    ("cities[].production", Neutral::EmptyMap),
    ("cities[].production_value", Neutral::Zero),
    ("cities[].loyalty_target.distance", Neutral::Zero),
    ("cities[].loyalty_target.entertainment", Neutral::Zero),

    ("cities[].loyalty_target.effective", Neutral::Zero),
    // ── CityRow：产出与建造的中间量（B2）──
    // 用工系数的中性值是 **1.0**（不缺人手），不是 0——写 0 会被读成「全城没人上工」。
    ("cities[].labor", Neutral::One),
    ("cities[].housing_capacity", Neutral::Zero),
    ("cities[].is_hub", Neutral::False),
    ("cities[].build", Neutral::EmptyMap),
    ("cities[].build[].rate", Neutral::Zero),
    ("cities[].build[].increment", Neutral::Zero),
];

/// 查一个路径的中性值（路径口径见 [`READ_FACE_NEUTRALS`]）。
pub fn neutral_for(path: &str) -> Option<Neutral> {
    READ_FACE_NEUTRALS
        .iter()
        .find(|(p, _)| *p == path)
        .map(|(_, n)| *n)
}

/// 整张表序列化成 `path → 中性值`——这就是 `schema.json` 的 `neutral.fields` 段。
pub fn neutral_table_json() -> serde_json::Map<String, serde_json::Value> {
    READ_FACE_NEUTRALS
        .iter()
        .map(|(path, n)| ((*path).to_string(), n.to_json()))
        .collect()
}

/// `schema.json` 的 `neutral` 段（含一段说明，告诉外部读者「缺键时按这里补，别自己编缺省」）。
pub fn schema_section() -> serde_json::Value {
    serde_json::json!({
        "root": "view",
        "description": "读面（`view`；= 主流每行的 `view`、`--derived` 的 `pre`/`post`）**每个叶子字段的中性值（缺省值）**。\n· 语义：字段处于中性值 ⇔「这一步还没跑 / 这件事没发生」，**不是**「它的值是零」——例如 `factions[].governance_scale` 的中性值是 1.0（未超载）；`decisions.capital` 那种稀疏数组的中性值是 `[]`（这一回合没有那条判定），条目内部不再逐字段声明。\n· **一处声明**：这张表由 Rust `model::neutral::READ_FACE_NEUTRALS` 生成，引擎自己的运行时缺省读同一批常量，所以「引擎怎么补」与「这里怎么写」不可能不一致（有三条测试钉住）。\n· 路径相对 `view` 根；map / 数组的值用 `[]` 表示。\n· ⚠ **缺键时按这里的值补装，不要自己编缺省**——历史上 `flow.jsonl` 补 0、`metrics` 补 1.0，同一回合两个读面各说各话，就是这么来的。",
        "fields": neutral_table_json(),
    })
}

#[cfg(test)]
#[path = "../tests/model/neutral.rs"]
mod tests;
