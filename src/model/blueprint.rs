use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::model::ShipBehavior;

/// 设计图 id = **名字**（势力内的唯一 key）。
///
/// 与项目其它实体一致：名字就是身份（`.agents/notes/name-as-unique-key.md`）。图名会
/// **换代**（玩家改名 = 删旧建新），所以任何按名字写的指针下一回合都可能指向不存在的东西
/// ——那种情况必须**响亮**报 `no_such_blueprint`，绝不静默回落到生成器（见
/// `.agents/notes/ship-blueprint-spec.md` §9.1 与 Q10(a)）。
pub type BlueprintId = String;

/// 一份设计图：**「还不存在的舰」的出厂规格**，不是舰。
///
/// 建造区**指向**一张图（[`crate::model::Building::blueprint`]），下水那一刻把图**印成**
/// 一艘舰：`Ship.components` 是**快照**，之后改图**不影响**已有的舰。
///
/// 它**刻意不携带任何数值**（面板/造价/维护/槽位）：那些只有 `config/game.ron` 一份真值
/// （`ShipSpec`/`ComponentSpec`），图里再抄一份就是第二个真相源（且会随 config 漂移）。
/// 「这张图造一艘要多少钱」是 `build_cost + Σ 组件 cost` 的**派生量**，引擎在读面/投影里
/// 算给 agent，不落状态。
///
/// 三态归属不在这里：图作为一片叶存在 [`crate::model::ControllableState::blueprints`] 里
/// （`Control<Blueprint>`），`Player` = 系统不许重估这张图（舰级/选装/意图全归玩家）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Blueprint {
    /// 舰级：`ShipSpec` 的 key（`config.ships`）。
    ///
    /// ⚠ 口径 A（用户裁决 Q3）：`Building.ship_type` 仍是「这个区造哪一级」的**唯一真相**，
    /// 图的 `class` 必须与它**相等**（不等在 apply 时报 `blueprint_class_mismatch`）。
    pub class: String,
    /// 选装表（组件 id，顺序 = 槽位顺序）。**空 = 「交给生成器」**（= 今天的
    /// `autocontrol::choose_loadout`），与 `mode` 无关——所以一张只钉意图/舰级的图
    /// 不必把选装也抄一遍。
    #[serde(default)]
    pub components: Vec<String>,
    /// 本图给**新舰**的**舰级默认指令**（「新造的护卫舰默认守家」的落点）。
    ///
    /// `None` = **本图对意图没有说话**（Q1(c)：图的意图轴默认 `Inherit`——建图 ≠ 表态），
    /// 于是链继续往下降到舰队默认。写了它，这一层才可能遮住舰队默认（还要看图的归属是否
    /// 解析为 `Player`，见 [`crate::model::State::ship_behavior`]）。
    #[serde(default)]
    pub order: Option<ShipBehavior>,
}

/// 设计图的**开局种子**（`config/game.ron` 的 `blueprints:` 表里的一项）。
///
/// 只在**世界生成**时被读一次（[`crate::world::default_state`]）：把它塞进对应势力的
/// 设计图库，并且**一律以 `Inherit`（这一层没有说话）写入**——归属由 `scope` 链解析
/// （默认落到 `Auto`）。想让某张图开局就归玩家，用 `--apply` 钉。
///
/// 用户裁决 Q8：**不预置标准图**（`config/game.ron` 里这张表是空的）——预置会让「新旧
/// 行为一致」从**结构性**退化成「要额外证明的」。这张表留着是为了「容器做好」，而不是
/// 让谁偷偷塞一张图进去。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct BlueprintSeed {
    /// 图名（势力内唯一）。
    pub name: BlueprintId,
    /// 舰级。
    pub class: String,
    /// 选装（空 = 交给生成器）。
    #[serde(default)]
    pub components: Vec<String>,
    /// 舰级默认指令（省略 = 本图对意图没有说话）。
    #[serde(default)]
    pub order: Option<ShipBehavior>,
}

impl BlueprintSeed {
    /// 种子 → 图（丢掉名字，名字是它在库里的键）。
    pub fn blueprint(&self) -> Blueprint {
        Blueprint {
            class: self.class.clone(),
            components: self.components.clone(),
            order: self.order.clone(),
        }
    }
}
