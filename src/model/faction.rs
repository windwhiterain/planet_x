use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::model::{BodyId, FactionId, ResourceMap};

/// 一个势力的「思潮偏向」——4 条轴，每条取 `[-1,1]`，`0` = 均衡。负 = 左端，正 = 右端。
///
/// * `peace_military`：和平(-1) ⟷ 军国(+1)
/// * `science_tech`  ：科学(-1) ⟷ 技术(+1)
/// * `people_elite`  ：人民(-1) ⟷ 精英(+1)
/// * `nature_colony` ：自然(-1) ⟷ 殖民(+1)
///
/// 这是**可变化的当代思潮**（区别于文明的 `alignment`/`aggression` 身份）：它由
/// `sim::step_ideology` 逐回合按「变化因素」驱动——战争得失（和平↔军国）、飞船在 MOND
/// 区域 vs 开采 MOND 区资源（科学↔技术）、经济好坏（人民↔精英）、人均面积高低
/// （自然↔殖民），并钳到 `[-1,1]`。agent 可直读以判断一国的**当下倾向**。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, JsonSchema)]
pub struct Ideology {
    #[serde(default)]
    pub peace_military: f64,
    #[serde(default)]
    pub science_tech: f64,
    #[serde(default)]
    pub people_elite: f64,
    #[serde(default)]
    pub nature_colony: f64,
}

/// A faction (势力) with diplomatic stances toward every other faction.
///
/// The command-controlled budget lives in [`ControllableState::budget`], not
/// here.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct Faction {
    pub name: String,
    /// Marker glyph on the CLI ASCII map (e.g. 'U').
    pub symbol: char,
    /// Display colour (CSS hex string) shown in the web UI, defined directly
    /// on the faction itself.
    pub color: String,
    /// Stockpiled resources (key -> amount).
    pub resources: ResourceMap,
    /// Relation of this faction toward another faction. Negative means hostile.
    pub relations: BTreeMap<FactionId, f64>,
    /// 意识形态位置（约 -1..1；越负越「东方/教派」，越正越「西方/国际」）。
    /// 由它推算出两两之间的静息亲和（阵营亲缘），驱动外交漂移，令国际关系波动。
    pub alignment: f64,
    /// 好战度（0..1）：越高的势力越会加速与异己阵营走向敌对。
    pub aggression: f64,
    /// 本土防御半径（AU）：本方城市/舰在此半径（距有效首都，见
    /// [`State::capital_body`]）内获得本土防御。
    /// cult 的数值按 MOND 异常放大——它「掌握了正确的牛顿修正引力」，孤悬柯伊伯带，
    /// 被围攻时依靠此异常自保。
    #[serde(default = "default_home_radius")]
    pub home_radius: f64,
    /// 在本方本土区域内，敌方对其造成的伤害倍率（<1 = 削弱入侵者）。
    #[serde(default = "default_home_attack_mult")]
    pub home_attack_mult: f64,
    /// 在本方本土区域内，本方舰只的额外护甲再生（占最大护甲/回合）。
    #[serde(default = "default_home_regen_bonus")]
    pub home_regen_bonus: f64,
    /// 当前的思潮偏向（可变化）。见 [`Ideology`]。
    #[serde(default)]
    pub ideology: Ideology,
}

pub(crate) fn default_capital_body() -> BodyId {
    "地球".to_string() // 地球, the default anchor when a .ron state omits it.
}

fn default_home_radius() -> f64 {
    0.0
}

fn default_home_attack_mult() -> f64 {
    1.0
}

fn default_home_regen_bonus() -> f64 {
    0.0
}
