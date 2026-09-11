use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::model::{ShipDoctrine, ShipRole};

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
/// # 图能表态的是**倾向**，不是**指令**（用户裁决 2026-10）
///
/// 图上可以写的是**长期倾向**三片——行为风格（[`ShipDoctrine`]）、风筝↔贴脸姿态
/// （`kiting`）、角色（[`ShipRole`]）；**不能**写"指令"（[`crate::model::ShipBehavior`]）：
/// 指令是**即时操作**（去那里 / 跟随那艘船），不是"这型舰是什么"，没有"出厂默认"可言。
/// 原来那一片 `order` 已删除；"新舰出厂就有倾向"的家 = 图上的**角色**（战舰图 / 运输舰图 /
/// 观测舰图），而具体去哪、跟随谁，由自动控制或玩家**逐舰**现给。
///
/// 每个轴 `None` = **本图对该轴没有说话**（Q1(c)：图的意图轴默认沉默——建图 ≠ 表态），
/// 于是链继续往下降到舰队默认。写了它，这一层才可能遮住舰队默认（还要看图的归属是否解析为
/// `Player`，见 [`crate::model::State::ship_doctrine`]）。
///
/// 与组件不同，倾向是**活层**（Q2=(b) 的延伸）：舰上只记图名，取值时现查图——改图的倾向会
/// 立刻对这张图的所有舰（对应轴的叶沉默者）生效，而面板/组件仍是下水那一刻的快照。
///
/// 三态归属不在这里：图作为一片叶存在 [`crate::model::ControllableState::blueprints`] 里
/// （`Control<Blueprint>`），`Player` = 系统不许重估这张图（舰级/选装/倾向全归玩家）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Blueprint {
    /// 舰级：`ShipSpec` 的 key（`config.ships`）。
    ///
    /// ⚠ 口径 A（用户裁决 Q3）：`Building.ship_type` 仍是「这个区造哪一级」的**唯一真相**，
    /// 图的 `class` 必须与它**相等**（不等在 apply 时报 `blueprint_class_mismatch`）。
    #[serde(rename = "舰级")]
    pub class: String,
    /// 选装表（组件 id，顺序 = 槽位顺序）。**空 = 「交给生成器」**（= 今天的
    /// `autocontrol::choose_loadout`），与 `mode` 无关——所以一张只钉倾向/舰级的图
    /// 不必把选装也抄一遍。
    #[serde(default)]
    #[serde(rename = "选装")]
    pub components: Vec<String>,
    /// 本图给这型舰的**行为风格**（理智↔热血 + 护航↔独狼）。`None` = 本图对该轴没有说话。
    #[serde(default)]
    #[serde(rename = "风格")]
    pub doctrine: Option<ShipDoctrine>,
    /// 本图给这型舰的**风筝↔贴脸姿态**（`[-1,1]`）。`None` = 本图对该轴没有说话。
    #[serde(default)]
    #[serde(rename = "姿态")]
    pub kiting: Option<f64>,
    /// 本图给这型舰的**角色**（战舰 / 运输舰 / 观测舰）。`None` = 本图对该轴没有说话。
    ///
    /// 这是"新舰出厂就有的倾向"里最有用的一片：**运输舰图**会让这型舰一造出来就被自动控制
    /// 派去跑集货路线（`autocontrol::freight` 按积压派活），**观测舰图**会派它们去引力异常区。
    #[serde(default)]
    #[serde(rename = "角色")]
    pub role: Option<ShipRole>,
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
    #[serde(rename = "图名")]
    pub name: BlueprintId,
    /// 舰级。
    #[serde(rename = "舰级")]
    pub class: String,
    /// 选装（空 = 交给生成器）。
    #[serde(default)]
    #[serde(rename = "选装")]
    pub components: Vec<String>,
    /// 行为风格（省略 = 本图对该轴没有说话）。
    #[serde(default)]
    #[serde(rename = "风格")]
    pub doctrine: Option<ShipDoctrine>,
    /// 风筝↔贴脸姿态（省略 = 本图对该轴没有说话）。
    #[serde(default)]
    #[serde(rename = "姿态")]
    pub kiting: Option<f64>,
    /// 角色（省略 = 本图对该轴没有说话）。
    #[serde(default)]
    #[serde(rename = "角色")]
    pub role: Option<ShipRole>,
}

impl BlueprintSeed {
    /// 种子 → 图（丢掉名字，名字是它在库里的键）。
    pub fn blueprint(&self) -> Blueprint {
        Blueprint {
            class: self.class.clone(),
            components: self.components.clone(),
            doctrine: self.doctrine,
            kiting: self.kiting,
            role: self.role,
        }
    }
}
