use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Entity identity = the entity's unique **name** (single authoritative schema
/// key; no numeric shadow id). References between entities use these names so the
/// world has one source of truth. Entities without a meaningful name (Building,
/// Settlement) keep an index within their parent and are NOT name-keyed.
pub type FactionId = String;
pub type BodyId = String;
pub type CityId = String;
/// The ship's identity is its unique, meaningful **name** — the single authoritative
/// key for the schema (no numeric shadow id).
pub type ShipId = String;
pub type BuildingId = u32;

/// A resource bundle: resource key -> amount. Resource keys are configurable
/// and resolved against [`GameConfig::resources`].
pub type ResourceMap = BTreeMap<String, f64>;

/// A resource type definition (display metadata for a resource key).
///
/// `value` is the resource's market price per unit (abstract credits): rare
/// minerals (氦-3/钍/铀/金/铂) are worth several times the common industrial
/// ones (铁/碳/硅/氢/甲烷/水冰). The automatic market trades surpluses for the
/// minerals a faction is short of, priced on these values.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ResourceDef {
    pub name: String,
    pub value: f64,
}

/// 定居点上的**单资源矿藏**：`area`（面积）决定这片矿**最多能开采出多少**
/// （矿场把它一点点刻出来，见 `ResourceDeposit` 的 `area`）。
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct ResourceDeposit {
    pub resource: String,
    pub area: f64,
}

/// **身份键的唯一声明**：结构体名 → 它在读面上的身份字段名（`serde` 名，即中文名词）。
///
/// 「一张表/一个结构体靠哪个字段认人」以前在 Python（`_harness._ID_KEY`）、kit、JS 三处
/// 各自维护**镜像小表**——引擎把字段改名或换身份键，它们**不会红**，只会悄悄用旧键
/// （典型的"失败看起来像成功"）。现在这一份是**唯一真值**：由 [`crate::agent::noun_schema_value`]
/// 随 `--nouns` / `GET /api/schema` 一起发出去，三端都来问它。
///
/// 由 `play/tests/g4_spec.py` 的守卫钉死：指名的字段必须在**真实世界**里存在**且唯一**
/// （拿一局真跑的投影逐轮分组验，不是自证声明）。
///
/// ⚠ 只列**身份就是自己某个字段**的结构体。两类不在这里：
/// * **以 map 键认人的**（`Blueprint` 的键是 `BlueprintId`，结构体里没有那个字段）——
///   它的身份由**表的键**（`projection` 的 `blueprints.blueprint_id`）承担；
/// * **复合身份的**（`Building` 的身份是 `(城, 城内下标)`）——同上，由表键承担。
pub const IDENTITY: &[(&str, &str)] = &[
    ("Ship", "舰名"),
    ("City", "城名"),
    ("Body", "天体名"),
    ("Settlement", "定居点"),
    ("Faction", "势力"),
    ("Contract", "合同号"),
];
