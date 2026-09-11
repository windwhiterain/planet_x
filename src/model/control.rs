use schemars::JsonSchema;
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;
use std::fmt;

use crate::model::{
    Blueprint, BlueprintId, BodyId, BuildingId, CityId, FactionId, ShipBehavior, ShipDoctrine,
    ShipId, ShipRole,
};

/// 沿作用域链（从具体到宽泛）取第一个**有意见**的层，即第一个不是
/// [`ControlMode::Inherit`] 的节点；全链都「继承」（没有说话）时，由系统自动决定
/// = [`ControlMode::Auto`]。
///
/// 这就是「总得有人做决定」的兜底：`Inherit` 不是第三种决策者，而是**没有说话**。
pub(crate) fn resolve_chain(chain: &[ControlMode]) -> ControlMode {
    chain
        .iter()
        .copied()
        .find(|m| *m != ControlMode::Inherit)
        .unwrap_or(ControlMode::Auto)
}

impl ControlScope {
    /// 把一份**作用域补丁**按节点叠加到 `self` 上：只覆盖补丁里出现的节点
    /// （`Some`）与键（`Vec` 里出现的键）；没出现的层原样不动。
    ///
    /// 键的值就是三态之一：`Auto`/`Player` = 在这一层表态；`Inherit` = 撤销这一层的
    /// 表态（与「这个键不存在」等价，只是读面上更明确）。
    pub fn overlay(&mut self, other: &ControlScopePatch) {
        if let Some(g) = other.global {
            self.global = g;
        }
        for (k, v) in &other.factions {
            self.factions.insert(k.clone(), *v);
        }
        for (k, v) in &other.bodies {
            self.bodies.insert(k.clone(), *v);
        }
        for (k, v) in &other.cities {
            self.cities.insert(k.clone(), *v);
        }
    }
}

// --- 可控状态 (controllable / command-controlled state) --------------------

/// 建筑「建设投资权重」定位键：(城市, 建筑)。同一座城的每栋建筑一个值。
pub type InvestKey = (CityId, BuildingId);
/// 建造区「建造投资权重」定位键：(城市, 建造区建筑)。每座城的每个建造区一个值。
pub type BuildKey = (CityId, BuildingId);

/// 一个可控叶子的**归属**：三态。
///
/// * [`Inherit`](ControlMode::Inherit) —— **继承**：这一层没有说话，沿作用域链上溯
///   （舰/建筑/预算 → 城市 → 天体 → 势力 → 全局）。这是**缺省**：不表态就继承。
/// * [`Auto`](ControlMode::Auto) —— **自动**：由系统（`autocontrol`）每回合决定并改写。
/// * [`Player`](ControlMode::Player) —— **玩家**：玩家的指令，系统只读不改写。
///
/// 「继承」不是第三种决策者：它只是把决定权让给更宽的那一层，一路无话则落到 `Auto`。
///
/// 旧档兼容：改名前这里叫 `Ai`、第三态寄生在 `Option::None` 里。线格式与读写见下面手写的
/// `Serialize`/`Deserialize`——旧 `.ron` 存档**逐值无损**地读进三态
/// （`None`≡`Inherit`、`Some(Ai)`≡`Auto`、`Some(Player)`≡`Player`）。
#[derive(Clone, Copy, Debug, Default, PartialEq, JsonSchema)]
pub enum ControlMode {
    /// 继承更宽的一层；全链继承则落到 [`ControlMode::Auto`]。
    #[default]
    Inherit,
    /// 系统自动决策（原来的 `Ai`）。
    Auto,
    /// 玩家下达指令，系统只用该值。
    Player,
}

impl ControlMode {
    /// 三态 → 线上名字：JSON 读面就是它（`"mode":"Player"`），RON 里当
    /// `Some(Player)` 的载荷。读面即写面的那套拼写，唯一的权威来源。
    pub fn name(self) -> &'static str {
        match self {
            ControlMode::Inherit => "Inherit",
            ControlMode::Auto => "Auto",
            ControlMode::Player => "Player",
        }
    }

    /// 判定结果是否**归玩家**——读判定链（[`resolve_chain`]）的结果时用这个，
    /// 而不是 match 三态：链上判定只剩两态（`Inherit` 已被消化成 `Auto`），
    /// 每个调用点再写一条永不发生的 `Inherit` 分支只会变成噪声。
    /// 兜底也安全：万一有人把中间的 `Inherit` 当结果用，控制权不会误判给玩家。
    pub fn is_player(self) -> bool {
        matches!(self, ControlMode::Player)
    }
}

const MODE_NAMES: [&str; 3] = ["Inherit", "Auto", "Player"];

/// 只为反序列化存在的**旧拼写接纳器**：派生实现能同时吃 RON 的裸标识符
/// （`Player`）与 JSON 的字符串（`"Player"`），所以名字解析交给它，我们只负责
/// 把 `Ai` 映射成 `Auto`。
#[derive(Deserialize)]
enum ModeName {
    Inherit,
    Auto,
    /// `Auto` 改名前的拼写（旧 `.ron` 存档里写的是 `Some(Ai)`）。
    Ai,
    Player,
}

/// 线格式的**载荷**：一枚只装名字的「单元变体」。
///
/// 必须走 `serialize_unit_variant` 而不是 `serialize_str`：前者让 RON 写裸标识符
/// （`Some(Player)`，与旧存档同形）、JSON 写字符串（`"Player"`，读面干净）；后者在 RON 里
/// 会写成带引号的字符串，而派生枚举（[`ModeName`]）在 RON 里只认标识符——于是自己写的
/// 存档会读不回来（实测 `ExpectedIdentifier`）。
struct ModeNameSer(ControlMode);

impl Serialize for ModeNameSer {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            ControlMode::Inherit => s.serialize_unit_variant("ControlMode", 0, "Inherit"),
            ControlMode::Auto => s.serialize_unit_variant("ControlMode", 1, "Auto"),
            ControlMode::Player => s.serialize_unit_variant("ControlMode", 2, "Player"),
        }
    }
}

/// `ControlMode` 的**线格式**：一个「总是有载荷的 Option」——RON 里写作
/// `Some(Player)`、JSON 里就是 `"Player"`。
///
/// 为什么绕这一下：旧存档里这个字段是 `Option<ControlMode>`（`None` = 没有说话、
/// `Some(Ai)` = 那时的自动），而 RON 的 `deserialize_any` **看不见裸标识符的名字**
/// （实测会退化成无载荷的 unit，名字直接丢掉），只有 `deserialize_option` + 派生枚举
/// 这一条路能同时认出 `None`、`Some(Player)`、`Some(Ai)` 与裸的 `Player`。所以线格式选
/// 「Option 外壳 + 名字载荷」而不是裸枚举：旧档零损失读入，新档也不会把「归谁」静默
/// 降级成继承。
///
/// 注意**只写 `Some(..)`、永不写 `None`**：`Inherit` 也是一个显式载荷
/// （JSON 读面因此是 `"mode":"Inherit"`，而不是含糊的 `null`）。
impl Serialize for ControlMode {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_some(&ModeNameSer(*self))
    }
}

impl<'de> Deserialize<'de> for ControlMode {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct ModeVisitor;

        impl<'de> Visitor<'de> for ModeVisitor {
            type Value = ControlMode;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "{}（旧档的 null / None 也接受）", MODE_NAMES.join(" | "))
            }

            /// `None` / `null` = 旧档的「这一层没有说话」。
            fn visit_none<E: de::Error>(self) -> Result<ControlMode, E> {
                Ok(ControlMode::Inherit)
            }

            fn visit_unit<E: de::Error>(self) -> Result<ControlMode, E> {
                Ok(ControlMode::Inherit)
            }

            /// 载荷就是名字：`Some(Player)` / `"Player"` / `Some(Ai)`（旧拼写）。
            fn visit_some<D2: Deserializer<'de>>(self, d: D2) -> Result<ControlMode, D2::Error> {
                let name = ModeName::deserialize(d)?;
                Ok(match name {
                    ModeName::Inherit => ControlMode::Inherit,
                    ModeName::Auto | ModeName::Ai => ControlMode::Auto,
                    ModeName::Player => ControlMode::Player,
                })
            }
        }

        d.deserialize_option(ModeVisitor)
    }
}

/// 一个可控叶子：值 + 由谁决定。
///
/// `mode = Inherit`（缺省）表示**这一层没有说话**，沿作用域链上溯
/// （舰/建筑/预算 → 城市 → 天体 → 势力 → 全局），全链 `Inherit` 时落到 `Auto`。
#[derive(Serialize, Deserialize, Clone, Debug, Default, schemars::JsonSchema)]
pub struct Control<T> {
    pub value: T,
    #[serde(default)]
    pub mode: ControlMode,
}

impl<T> Control<T> {
    /// 一个由系统自动决策的可控值。
    pub fn auto(value: T) -> Self {
        Self {
            value,
            mode: ControlMode::Auto,
        }
    }

    /// 一个由玩家指令决定的可控值。
    pub fn player(value: T) -> Self {
        Self {
            value,
            mode: ControlMode::Player,
        }
    }

    /// 一个继承上层（本层没有说话）的可控值。
    pub fn inherit(value: T) -> Self {
        Self {
            value,
            mode: ControlMode::Inherit,
        }
    }
}

/// 城市/天体/势力/全局 的控制作用域。这些不是 [`ControllableState`] 的字段，
/// 单独建一棵作用域树；判定可控叶子的自动/玩家边界时沿链上溯。
///
/// 节点值 [`ControlMode::Inherit`] 表示这一层没有说话（与「这个键不存在」等价）：
/// 作用域节点**只表态「谁负责」**，不携带值——值属于叶子。
#[derive(Serialize, Deserialize, Clone, Debug, Default, schemars::JsonSchema)]
pub struct ControlScope {
    #[serde(default)]
    /// **全局那一档**（这个世界的默认归属）：链上谁都没说话时按它。
    pub global: ControlMode,
    #[serde(default)]
    /// **已表态的势力**（势力名 → 三态）。只存表过态的：`Inherit` 等于「这一层没有说话」，不占条目。
    pub factions: BTreeMap<FactionId, ControlMode>,
    #[serde(default)]
    /// **已表态的天体**（天体名 → 三态）。比势力具体、比城粗。
    pub bodies: BTreeMap<BodyId, ControlMode>,
    #[serde(default)]
    /// **已表态的城**（城名 → 三态）。链上最具体的一档。
    pub cities: BTreeMap<CityId, ControlMode>,
}

/// 作用域树的**补丁**（写面 `scope` 字段）：只覆盖出现的节点/键。
///
/// * `global: None` = 不动全局层；`Some(Inherit)` = 撤销全局层的表态。
/// * 三个 `Vec` 里**出现的键**就是被触碰的键，其值同样是三态（`Inherit` = 撤销该键）。
///
/// 同一个类型也是作用域树的**读面**（`--control` / web 的 `scope` 字段）：读面只列出
/// 有意见的节点（`Inherit` 不列），于是「读出来 → 改 → 回传」天然安全——没列出过的层
/// 不会被静默清掉。
#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema)]
pub struct ControlScopePatch {
    #[serde(default)]
    /// **全局那一档**：「链上谁都没说话时按谁」——`Inherit` = 全局也不说话（那就按引擎的默认），`Auto` = 自动控制，`Player` = 玩家。它是归属链的**最后一站**。
    pub global: Option<ControlMode>,
    #[serde(default)]
    /// **势力档**：`[(势力名, 三态)]`。比全局具体 ⇒ 势力表态了就轮不到全局说话。
    pub factions: Vec<(FactionId, ControlMode)>,
    #[serde(default)]
    /// **天体档**：`[(天体名, 三态)]`。比势力具体（“这个星球我亲自管”），但比城粗。
    pub bodies: Vec<(BodyId, ControlMode)>,
    #[serde(default)]
    /// **城档**：`[(城名, 三态)]`。链上**最具体**的一档 ⇒ 它一表态，上面几档都不算数。
    pub cities: Vec<(CityId, ControlMode)>,
}

/// 单个势力的可控状态：所有「指令控制」的量的集合。
///
/// 实体演化结果（资源量、耐久、人口、防御…）不在其中；本结构体是被指令
/// 直接改写的状态。指令 = 对它的修改（diff）。
///
/// 每个叶子用 [`Control`] 包裹：值 + 谁决定它。舰/资源/建筑的粒度在各自的
/// `mode`；城市/天体/势力/全局这些更粗的作用域在 [`State::scope`](crate::model::State::scope)。
#[derive(Serialize, Deserialize, Clone, Debug, Default, schemars::JsonSchema)]
pub struct ControllableState {
    /// 本方各飞船的当前指令（每艘舰一个 Control）。
    pub ship_orders: BTreeMap<ShipId, Control<ShipBehavior>>,
    /// 本方各舰的**行为风格**叶片（值 + 三态归属）。
    ///
    /// 这是 `ship_doctrine` 这一轴的**活层**：`Ship.doctrine` 降级成"记录值"（出厂快照 +
    /// AI 流水），有效值走 [`State::ship_doctrine`](crate::model::State::ship_doctrine) 的
    /// 叶 → 舰队默认 → 记录值 链。AI **从不写**这片叶（它只读有效值），所以"AI 覆盖玩家风格"
    /// 这种问题在这条轴上不存在。
    #[serde(default)]
    pub ship_doctrine: BTreeMap<ShipId, Control<ShipDoctrine>>,
    /// 本方各舰的**风筝<->贴脸姿态**叶片（值 + 三态归属），与 `ship_doctrine` 同形的另一条轴。
    #[serde(default)]
    pub ship_kiting: BTreeMap<ShipId, Control<f64>>,
    /// 本方各舰的**角色**叶片（值 + 三态归属）：[`ShipRole`] = 打仗 / 运输 / **观测**。
    /// 这是第三条风格轴，取值规则与 `ship_doctrine`/`ship_kiting` 完全同形
    /// （叶 → 舰队默认 → 舰上记录值，见 [`State::ship_role`](crate::model::State::ship_role)）。
    ///
    /// **与前两条轴的唯一差别：这条轴 AI 会写**（前两条 AI 只读）。因为「这艘舰干哪种活」是
    /// 自动控制**每回合要做的判断**（按积压定编 + 按观测缺口定编，见 `autocontrol::freight`
    /// 与 `autocontrol::knowledge`），它需要把结论落在某处才稳定。三态语义照旧：玩家把这片叶
    /// 设成 `Player`，AI 就不再改写它。
    #[serde(default)]
    pub ship_role: BTreeMap<ShipId, Control<ShipRole>>,
    /// **舰队默认行为风格**（势力级）：叶 Inherit 的舰取它的值。"全舰队风筝、战列舰贴脸"
    /// 这类意图 = 一片默认叶 + 几片特例叶，不必逐舰点名。
    ///
    /// ⚠ 舰队级**只有长期倾向**这三片（风格两轴 + 角色）；**没有**"舰队默认指令"那一片了
    /// （2026-10 删除）：指令是即时操作，写一片全舰队默认实测是**全舰队接管开关**，
    /// 名字与作用不符——见 [`State::ship_behavior`](crate::model::State::ship_behavior)。
    #[serde(default)]
    pub default_doctrine: Option<Control<ShipDoctrine>>,
    /// **舰队默认风筝<->贴脸姿态**（势力级），与 `default_doctrine` 同形的另一片。
    #[serde(default)]
    pub default_kiting: Option<Control<f64>>,
    /// **舰队默认角色**（势力级，第三条风格轴）：叶 Inherit 的舰取它的值。
    /// 「全舰队转运输、只有两艘战列留作战舰」这类意图 = 一片默认叶 + 几片特例叶。
    #[serde(default)]
    pub default_role: Option<Control<ShipRole>>,
    /// **设计图库**（势力级）：图名 → 图纸。
    ///
    /// 设计图是**「还不存在的舰」的出厂规格**：建造区指向一张图
    /// （[`Building::blueprint`](crate::model::Building::blueprint)），下水那一刻把图**印成**
    /// 一艘舰（`Ship.components` 是**快照**，之后改图不影响已有的舰）。
    ///
    /// 三态语义（照 [`Control`] 的通用规则，但这一片有自己的链——图是**势力的库**，
    /// **没有**「舰队默认」那一档）：
    /// * `Player` = 系统**不许重估**这张图：出厂按图装配（`components` 非空时就是它），
    ///   图上写了倾向（风格/姿态/角色）时那几条轴也归玩家（AI 不再改写它的叶片）；
    /// * `Auto` = 系统可重估这张图（`retool_shipyards` 会把它改到战局需要的舰级）；
    ///   出厂选装仍走 [`crate::autocontrol::choose_loadout`] 现算（= 今天的行为）；
    /// * `Inherit` = 这一层没有说话 ⇒ 沿 `scope` 链上溯（通常落到 `Auto`）。
    ///
    /// 有效归属走 [`State::blueprint_control`](crate::model::State::blueprint_control)。
    ///
    /// ⚠ **图能表态的是倾向（风格/姿态/角色），不是指令**；每条轴 `None` = 本图对该轴沉默
    /// （建图 ≠ 表态）。只有图上真写了某条轴，这一层才可能在那条轴的链上遮住舰队默认
    /// （用户裁决 Q1(c) + 2026-10 的倾向裁决）。
    #[serde(default)]
    pub blueprints: BTreeMap<BlueprintId, Control<Blueprint>>,
    /// 投资预算（资源/时间）：决定拿出多少资源用于「建设（建筑）」，按各建筑
    /// 建设投资权重竞争（每资源一个 Control）。
    pub investment_budget: BTreeMap<String, Control<f64>>,
    /// 建造预算（资源/时间）：决定拿出多少资源用于「造舰」，按各建造区建造
    /// 投资权重竞争（每资源一个 Control）。
    pub construction_budget: BTreeMap<String, Control<f64>>,
    /// 本方各建筑的「建设投资权重」（每建筑一个 Control）。
    #[serde(with = "crate::json::key2")]
    #[schemars(with = "std::collections::BTreeMap<String, Control<f64>>")]
    pub invest_weights: BTreeMap<InvestKey, Control<f64>>,
    /// 本方各建造区的「建造投资权重」（每建造区一个 Control）。
    #[serde(with = "crate::json::key2")]
    #[schemars(with = "std::collections::BTreeMap<String, Control<f64>>")]
    pub build_weights: BTreeMap<BuildKey, Control<f64>>,
    /// 本方各城的**福利权重**（每城一个 Control）：势力级
    /// [`welfare_budget`](Self::welfare_budget) 按这些权重分给城市，再按市场价值
    /// 折算成忠诚目标里的娱乐项。旧字段名保留为 `loyalty_budget`，但语义已从
    /// 「每城市场价值预算」改为「福利权重」。见 `.agents/notes/domestic-market.md`。
    #[serde(default)]
    pub loyalty_budget: BTreeMap<CityId, Control<f64>>,
    /// **逐城开发货币预算**（市场价值/回合）：国内市场开启时，城市拿它去买开发资源。
    /// `Inherit`/缺叶 = 系统按该城开发权重自动折算；`Player` = 玩家钉死。
    #[serde(default)]
    pub development_money: BTreeMap<CityId, Control<f64>>,
    /// **逐城建造货币预算**（市场价值/回合）：国内市场开启时，城市拿它去买造舰资源。
    #[serde(default)]
    pub construction_money: BTreeMap<CityId, Control<f64>>,
    /// **势力级福利预算**（资源/时间）：每资源一个 Control，按城市福利权重分给城市，
    /// 用于娱乐/忠诚。这是 `spec.md` 里「各类资源福利预算」的落点。
    #[serde(default)]
    pub welfare_budget: BTreeMap<String, Control<f64>>,
    /// 迁都（首都被命控制）：本势力当前希望的首都天体。`mode=Player` 时玩家说了算、
    /// 系统不改写（除非首都亡城——硬规则先于一切）；`mode=Auto`/`Inherit` 时由 sim 的
    /// 周期迁都步骤决定。`Inherit` = 这一层没有说话（回落到
    /// [`default_capital_body`](crate::model::default_capital_body) 兜底）。
    /// 「有效首都」的唯一事实来源就是这里，用 [`State::capital_body`](crate::model::State::capital_body)
    /// 解析——无 shadow 双状态。
    #[serde(default)]
    pub capital: Option<Control<BodyId>>,
}
