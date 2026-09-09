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

/// A deposit of a single resource on a settlement. `area` bounds how much
/// mining may be carved out of this deposit.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct ResourceDeposit {
    pub resource: String,
    pub area: f64,
}
