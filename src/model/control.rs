use schemars::JsonSchema;
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;
use std::fmt;

use crate::model::{BodyId, BuildingId, CityId, FactionId, ShipBehavior, ShipDoctrine, ShipId};

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
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Control<T> {
    pub value: T,
    #[serde(default)]
    pub mode: ControlMode,
}

impl<T> Control<T> {
    /// 一个由系统自动决策的可控值。
    pub fn auto(value: T) -> Self {
        Self { value, mode: ControlMode::Auto }
    }

    /// 一个由玩家指令决定的可控值。
    pub fn player(value: T) -> Self {
        Self { value, mode: ControlMode::Player }
    }

    /// 一个继承上层（本层没有说话）的可控值。
    pub fn inherit(value: T) -> Self {
        Self { value, mode: ControlMode::Inherit }
    }
}

/// 城市/天体/势力/全局 的控制作用域。这些不是 [`ControllableState`] 的字段，
/// 单独建一棵作用域树；判定可控叶子的自动/玩家边界时沿链上溯。
///
/// 节点值 [`ControlMode::Inherit`] 表示这一层没有说话（与「这个键不存在」等价）：
/// 作用域节点**只表态「谁负责」**，不携带值——值属于叶子。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ControlScope {
    #[serde(default)]
    pub global: ControlMode,
    #[serde(default)]
    pub factions: BTreeMap<FactionId, ControlMode>,
    #[serde(default)]
    pub bodies: BTreeMap<BodyId, ControlMode>,
    #[serde(default)]
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
    pub global: Option<ControlMode>,
    #[serde(default)]
    pub factions: Vec<(FactionId, ControlMode)>,
    #[serde(default)]
    pub bodies: Vec<(BodyId, ControlMode)>,
    #[serde(default)]
    pub cities: Vec<(CityId, ControlMode)>,
}

/// 单个势力的可控状态：所有「指令控制」的量的集合。
///
/// 实体演化结果（资源量、耐久、人口、防御…）不在其中；本结构体是被指令
/// 直接改写的状态。指令 = 对它的修改（diff）。
///
/// 每个叶子用 [`Control`] 包裹：值 + 谁决定它。舰/资源/建筑的粒度在各自的
/// `mode`；城市/天体/势力/全局这些更粗的作用域在 [`State::scope`](crate::model::State::scope)。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
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
    /// **舰队默认指令**（势力级的「没有别的指令时怎么办」）。
    ///
    /// 它是「新舰出生就有意图」和「一次性指令执行完回落到哪」的唯一答案，也是本势力
    /// 舰队指令的**高层值**：叶子 `mode = Inherit`（没有说话）的舰（包括**刚下水、还没有
    /// 任何叶片**的新舰）取它的值。
    ///
    /// 三态照旧：`Player` = 玩家给全舰队定的默认（新舰自动继承意图）；`Auto` = 明说交给
    /// 系统（叶子 Inherit 的舰也归 AI）；`Inherit` = 这一层没有说话，由作用域链决定。
    /// 它**只表态「谁负责 + 默认干什么」**，单舰的特例仍写在 `ship_orders` 的叶子上
    /// （更具体的层优先）。
    #[serde(default)]
    pub default_ship_order: Option<Control<ShipBehavior>>,
    /// **舰队默认行为风格**（势力级）：叶 Inherit 的舰取它的值。"全舰队风筝、战列舰贴脸"
    /// 这类意图 = 一片默认叶 + 几片特例叶，不必逐舰点名。
    #[serde(default)]
    pub default_doctrine: Option<Control<ShipDoctrine>>,
    /// **舰队默认风筝<->贴脸姿态**（势力级），与 `default_doctrine` 同形的另一片。
    #[serde(default)]
    pub default_kiting: Option<Control<f64>>,
    /// 投资预算（资源/时间）：决定拿出多少资源用于「建设（建筑）」，按各建筑
    /// 建设投资权重竞争（每资源一个 Control）。
    pub investment_budget: BTreeMap<String, Control<f64>>,
    /// 建造预算（资源/时间）：决定拿出多少资源用于「造舰」，按各建造区建造
    /// 投资权重竞争（每资源一个 Control）。
    pub construction_budget: BTreeMap<String, Control<f64>>,
    /// 本方各建筑的「建设投资权重」（每建筑一个 Control）。
    pub invest_weights: BTreeMap<InvestKey, Control<f64>>,
    /// 本方各建造区的「建造投资权重」（每建造区一个 Control）。
    pub build_weights: BTreeMap<BuildKey, Control<f64>>,
    /// 本方各城的「娱乐/福利预算」（每城一个 Control，市场价值/回合）：把资源投入
    /// 城市娱乐以提升忠诚度。这是「枪支与黄油」的现实权衡——花钱安抚居民，就少了
    /// 建设与造舰的预算；玩家/agent 用它来稳固对大/远城市的统治（见治理模型）。
    #[serde(default)]
    pub loyalty_budget: BTreeMap<CityId, Control<f64>>,
    /// 迁都（首都被命控制）：本势力当前希望的首都天体。`mode=Player` 时玩家说了算、
    /// 系统不改写（除非首都亡城——硬规则先于一切）；`mode=Auto`/`Inherit` 时由 sim 的
    /// 周期迁都步骤决定。`Inherit` = 这一层没有说话（回落到
    /// [`default_capital_body`](crate::model::default_capital_body) 兜底）。
    /// 「有效首都」的唯一事实来源就是这里，用 [`State::capital_body`](crate::model::State::capital_body)
    /// 解析——无 shadow 双状态。
    #[serde(default)]
    pub capital: Option<Control<BodyId>>,
}
