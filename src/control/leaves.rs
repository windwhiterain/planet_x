//! 控制叶的**结构事实**声明（`--control-schema` 里 `leaves` / `actions` 两段的来源）。
//!
//! ## 为什么需要这一份
//!
//! 写面（web 的控制行、kit 的 `set_*` / `silence_*` / `remove_*`）必须知道三件事：
//! **这片叶靠哪几个字段定位**（身份键）、**值写在哪几个字段**（值字段）、
//! **归属字段叫什么**（三态 + 删叶）。这些事实**只**存在于本模块旁边那些 patch / view
//! 结构体里（`wire.rs`）——但以前 web 与 kit 各自**手抄**了一份：
//!
//! * `web/static/app.js` 的 `LEAF_SPEC` / `LEAF_OPTIONS`
//! * `play/planet_x_ctl/planet_x_ctl/__init__.py` 的 `LEAF_KINDS` / `_VALUE_FIELD` /
//!   `_TWO_AXIS_KINDS` / `_COMPOSITE_KINDS` / `_BLUEPRINT_FIELDS`
//!
//! 于是「加一个叶」要改三处，而**漏改的那一端不会报错**——它只会静静地不认识这个叶
//! （读面里看不见、写面里写不了）。`role-axis-parity`（角色轴补三端）与
//! `blueprint-stance`（kit 的 `LEAF_KINDS` 缺项）两次踩的都是这个缺口。
//!
//! ## 分工（2026-10 裁决 S1）
//!
//! | 事实 | 住哪 |
//! | --- | --- |
//! | 键名 / 身份键 / 值字段 / 只读派生列 / 归属与删叶字段名 | **引擎**（本模块） |
//! | 编辑器、中文标签、单位、行序、和哪些读行相邻、在哪一页 | **前端**（`web/static/views.json`） |
//!
//! ## 纪律（由数据检查强制，不靠自觉）
//!
//! `play/tests/g4_spec.py` 会断言：
//!
//! 1. 本表的 `field` 集合 ∪ `{faction_id}` ∪ actions 的 `field` **双向等于** `schemars`
//!    从 [`FactionControlPatch`](super::FactionControlPatch) 派生出来的属性集合
//!    （⇒ 加字段不写声明 = 红；写了一个不存在的叶 = 红）；
//! 2. 每条声明的 `keys` / `values` / `carries` / `read_only` 与**真实读面**里该叶条目的
//!    字段集合**对齐**（⇒ 改结构体却忘了改声明 = 红）；
//! 3. `--index` 的 `control` 派生表里**出现过的 `kind` 取值集合**，与本表的
//!    `{field | not_in_index 为空} ∪ {actions 里 not_in_index 为空的}` **逐字相等**
//!    （⇒ 投影里少发一片叶、或写了个声明之外的名 = 红）。这条把「`kind` 词表」也钉住了：
//!    以前它是发射器里手写的英文串，与 `--control-schema` 的中文 `field` 是**同一个概念
//!    的两套名**（Python 按英文 kind 筛、写面用中文键），现在只有 [`LeafSpec::field`]
//!    一处声明，[`kind_of`] 是唯一翻译点。
//!
//! ⚠ 一条**实测**事实（2026-10，315 个读面条目里 0 次）：读面**从不发 `remove`** ——
//! `DefaultDoctrine` / `DefaultKiting` / `DefaultShipRole` 构造时写死 `remove: false`，
//! 而该字段带 `skip_serializing_if = "is_false"`；`capital` 是 [`Control<BodyId>`]，根本
//! 没有这个字段。`remove` 只活在**写面**（删叶）。所以那条对账把「条目里出现 `remove`」
//! 实现成**宽容侧**（出现即允许），并在细节里如实报「实测 0 次」，而不是假装它该有。
//!
//! [`Control<BodyId>`]: crate::model::Control
//!
//! 这两条合起来就是「一份事实、三端共用」的兑现处：web 与 kit 都读这一份，
//! 不再各抄各的。

use serde::Serialize;

/// 一片控制叶的结构事实。字段来源见每个常量的注释（`wire.rs` 的行号）。
#[derive(Serialize)]
pub struct LeafSpec {
    /// 这片叶在 patch / 读面里的**键名**（= `FactionControlPatch` 的字段名），
    /// 也是 `--index` 的 `control` 表里 `kind` 列的取值。
    ///
    /// ⚠ **这个中文名全仓只在这里写一次**（`--control-schema` / `--index` / web / kit /
    /// `planet_xq` 都是它的消费者）。投影发射器不写第二遍：它按 [`Self::state`] 选叶、
    /// 用 [`kind_of`] 换回这个名字。
    pub field: &'static str,
    /// 这片叶在**持久状态**（[`ControllableState`](crate::model::ControllableState)）里的
    /// **Rust 字段名**（`"ship_orders"` 这种，不是读面上的词）。
    ///
    /// 它是「叶标识」：投影发射器（`projection::write_round` 的 `control` 表）按它取数，
    /// 发出去的 `kind` 则是 [`Self::field`]。这样「同一个概念两套名」不可能再发生——
    /// 中文名只有一处声明，[`kind_of`] 是唯一的翻译点。
    ///
    /// ⚠ **不发进 `--control-schema`**：它是 Rust 侧的连接，不是读面事实。
    #[serde(skip_serializing)]
    pub state: &'static str,
    /// 这片叶**在不在** `--index` 的 `control` 派生表里。
    ///
    /// * `None` = 在：每回合每个元素一行（没有元素的回合零行——稀疏是**元素**的稀疏，
    ///   不是种类的稀疏）；
    /// * `Some(why)` = **不在**，`why` 必须写明为什么。
    ///
    /// 为什么要有这个字段：读面缺一片叶从前是**没人发现的省略**——发射器里少写一个
    /// `for` 循环，Python 侧只是查不到，不会红。现在「不在」也是一种**声明**：
    /// `play/tests/g4_spec.py` 拿真跑出来的 `control` 表的 `kind` 集合与
    /// `{field | not_in_index 为空}` **逐字对账**（多一个 / 少一个都红）。
    pub not_in_index: Option<&'static str>,
    /// **身份键**：定位一片叶靠哪几个字段。
    ///
    /// * 空 = **势力级单叶**（一个对象，不是列表）⇒ 界面据此决定「能不能当表格里的一格」；
    /// * 非空 = 列表叶，键一起构成这片叶的身份（界面拿它当行标签、也是回传差异时的定位键）。
    pub keys: &'static [&'static str],
    /// **值字段**：这片叶的「值」写在哪几个字段里（身份键、[`Self::owner_field`] 的
    /// `mode`、`remove` 都不算值）。
    pub values: &'static [&'static str],
    /// **随行属性**：读面条目里顺带带着的 **state 属性**（不是控制面的一部分）。
    ///
    /// 它们是引擎在 [`control_view`](super::control_view) 里 join 进来的（例如
    /// `invest_weights` 的 `kind` / `structure`），**写面回传时必须忽略**——
    /// 以前的 `app.js` 靠一句注释提醒自己「不能靠 `for k in leaf` 猜」，现在它是数据。
    pub carries: &'static [&'static str],
    /// **只读派生列**：读面里有、写面**收下但不写回**（引擎现算的派生量）。
    pub read_only: &'static [&'static str],
}

/// 一条**命令列表**的结构事实。
///
/// 与叶的区别是**存在性**：叶的「存在 / 不存在」本身是一种状态（`remove` 能删掉它），
/// 而命令列表里的每一条本来就只该执行一次（`buildings` 是「新建 / 拆掉 / 改属性」的意图）。
#[derive(Serialize)]
pub struct ActionSpec {
    /// 在 patch 里的键名（= `FactionControlPatch` 的字段名）。
    pub field: &'static str,
    /// 这条命令靠哪几个字段指向它的对象（新建时可以缺，例如「在这个城建一座矿场」）。
    pub keys: &'static [&'static str],
    /// 一句话说明它为什么不是叶（给读文档的人看，也进 `--control-schema`）。
    pub note: &'static str,
    /// 见 [`LeafSpec::not_in_index`]。命令列表**永远是 `Some(...)`**：它不是叶、没有持久状态，
    /// `--index` 的 `control` 表（一行一片叶）**从不**给它行。写死在这里是为了让
    /// 「命令列表出现在控制面表里」也变成一条能被对账出来的事实。
    pub not_in_index: Option<&'static str>,
}

/// 归属三态写在哪个字段里（`"Inherit" | "Auto" | "Player"`）。
pub const OWNER_FIELD: &str = "归属";
/// 删叶（「恢复出厂值」）写在哪个字段里。
pub const REMOVE_FIELD: &str = "删叶";

/// 全部控制叶的结构事实。
///
/// 顺序**没有语义**（消费者按 `field` 查表）；这里按「势力级单叶 → 逐舰 → 逐城/逐资源」
/// 排，只为读起来顺。
pub const LEAVES: &[LeafSpec] = &[
    // --- 势力级单叶（`wire.rs:523-574` 里那几个 `Option<...>` 字段）-----------------
    LeafSpec {
        // `wire.rs` 的 `CapitalPatch`：`{值, 归属, 删叶}`；
        // 读面给的是 `Control<BodyId>` = `{值, 归属}`（`model/control.rs`，没有 `删叶`）。
        field: "首都",
        state: "capital",
        not_in_index: None,
        keys: &[],
        values: &["值"],
        carries: &[],
        read_only: &[],
    },
    LeafSpec {
        // `DefaultDoctrine`：两个轴一起给（单轴新建会被引擎响亮拒绝）。
        field: "舰队默认风格",
        state: "default_doctrine",
        not_in_index: None,
        keys: &[],
        values: &["temper", "lone_wolf"],
        carries: &[],
        read_only: &[],
    },
    LeafSpec {
        field: "舰队默认姿态",
        state: "default_kiting",
        not_in_index: None,
        keys: &[],
        values: &["姿态"],
        carries: &[],
        read_only: &[],
    },
    LeafSpec {
        field: "舰队默认角色",
        state: "default_role",
        not_in_index: None,
        keys: &[],
        values: &["角色"],
        carries: &[],
        read_only: &[],
    },
    // --- 逐舰四叶（`ShipId` 定位）-------------------------------------------------
    LeafSpec {
        // 指令**没有**势力级那一片（2026-10 删除）：它是即时操作，只写逐舰叶。
        field: "指令",
        state: "ship_orders",
        not_in_index: None,
        keys: &["舰"],
        values: &["行为"],
        carries: &[],
        read_only: &[],
    },
    LeafSpec {
        field: "风格",
        state: "ship_doctrine",
        not_in_index: None,
        keys: &["舰"],
        values: &["temper", "lone_wolf"],
        carries: &[],
        read_only: &[],
    },
    LeafSpec {
        field: "姿态",
        state: "ship_kiting",
        not_in_index: None,
        keys: &["舰"],
        values: &["姿态"],
        carries: &[],
        read_only: &[],
    },
    LeafSpec {
        field: "角色",
        state: "ship_role",
        not_in_index: None,
        keys: &["舰"],
        values: &["角色"],
        carries: &[],
        read_only: &[],
    },
    // --- 逐资源预算 ---------------------------------------------------------------
    LeafSpec {
        field: "投资预算",
        state: "investment_budget",
        not_in_index: None,
        keys: &["资源"],
        values: &["值"],
        carries: &[],
        read_only: &[],
    },
    LeafSpec {
        field: "建造预算",
        state: "construction_budget",
        not_in_index: None,
        keys: &["资源"],
        values: &["值"],
        carries: &[],
        read_only: &[],
    },
    LeafSpec {
        field: "福利预算",
        state: "welfare_budget",
        not_in_index: None,
        keys: &["资源"],
        values: &["值"],
        carries: &[],
        read_only: &[],
    },
    // --- 逐建筑权重（身份 = 城 + 建筑；读面顺带带着建筑的 state 属性）-----------------
    LeafSpec {
        field: "建设权重",
        state: "invest_weights",
        not_in_index: None,
        keys: &["城", "建筑"],
        values: &["值"],
        // `InvestWeightEntry` 从 `state` 里 join 进来的四列（`view.rs` 的
        // `invest_weights` 映射）：写面不看它们，只用来显示「这是座什么楼、在造什么」。
        carries: &["类型", "资源", "建造舰级", "结构"],
        read_only: &[],
    },
    LeafSpec {
        field: "建造权重",
        state: "build_weights",
        not_in_index: None,
        keys: &["城", "建筑"],
        values: &["值"],
        carries: &["建造舰级"],
        read_only: &[],
    },
    // --- 逐城的娱乐/福利预算 --------------------------------------------------------
    LeafSpec {
        field: "城市福利预算",
        state: "loyalty_budget",
        not_in_index: None,
        keys: &["城"],
        values: &["值"],
        carries: &[],
        read_only: &[],
    },
    LeafSpec {
        field: "开发货币预算",
        state: "development_money",
        not_in_index: None,
        keys: &["城"],
        values: &["值"],
        carries: &[],
        read_only: &[],
    },
    LeafSpec {
        field: "建造货币预算",
        state: "construction_money",
        not_in_index: None,
        keys: &["城"],
        values: &["值"],
        carries: &[],
        read_only: &[],
    },
    // --- 设计图库（复合值叶：舰级 + 选装 + 倾向三轴，逐轴可空 = 该轴沉默）--------------
    LeafSpec {
        field: "设计图库",
        state: "blueprints",
        // 设计图是**结构叶**（`{舰级, 选装[], 倾向三轴}`），塞进 `control` 表的 `value: any`
        // 列会让列类型不稳、Python 侧还要二次解析；而且它与 `derived.blueprints`
        // （有类型列的专用表）会成为**同一份事实的两份表示** = 漂移风险。所以只住那张表。
        not_in_index: Some("结构叶：住在 `derived.blueprints`（有类型列的专用表），不重复发到 `derived.control`"),
        keys: &["图名"],
        values: &["舰级", "选装", "风格", "姿态", "角色"],
        carries: &[],
        // `BlueprintPatch` 收下这两列但**不写回状态**（`view.rs`）：
        // 「这张图在等钱」与「有几艘舰在用」是玩家做决定要看的东西，引擎现算。
        read_only: &["ship_count", "launch_waiting"],
    },
];

/// 命令列表（不是叶）：见 [`ActionSpec`]。
pub const ACTIONS: &[ActionSpec] = &[ActionSpec {
    field: "建筑",
    keys: &["城", "建筑"],
    note: "结构性建筑命令（新建 / 拆掉 / 改属性）：每一条只该执行一次，不是状态",
    not_in_index: Some("命令列表（不是叶）：`--apply` 里执行一次就完了，没有每回合可读的持久状态"),
}];

/// **叶标识 → 叶字段名**：投影 `control` 表的 `kind` 列取值的**唯一**翻译点。
///
/// `state` 是 [`ControllableState`](crate::model::ControllableState) 的 **Rust 字段名**
/// （`"ship_orders"` 这种）——投影发射器手里只有它（它在遍历 `c.ship_orders`），
/// 而读面要的是 [`LeafSpec::field`] 那个中文名。名字只在 [`LEAVES`] 里写一次，
/// 这里只是查表，**不引入第二个字面量**。
///
/// ⚠ 查不到就 **panic**：发射器里出现一个声明里没有的字段 = 声明与实现脱钩，
/// 那是缺陷，不是可以静默跳过的情形（静默跳过就是「失败看起来像成功」）。
/// 它的数据级对偶是 `play/tests/g4_spec.py` 的 kind 对账，进程级对偶是仓库里的
/// `kind_of` 调用与 `LEAVES` 一起改。
pub fn kind_of(state: &str) -> &'static str {
    match LEAVES.iter().find(|l| l.state == state) {
        Some(l) => l.field,
        None => panic!(
            "投影里出现了 `LEAVES` 声明之外的控制叶 `{state}`（加了字段忘了写声明？）"
        ),
    }
}

/// `--index` 的 `control` 派生表里**应该有**的 `kind` 取值（声明侧）。
///
/// = 所有 [`LeafSpec::not_in_index`] 为空的叶的 `field`，**按 [`LEAVES`] 的顺序**。
/// 两个用途：
/// * `--index` 的 `control` 表列文档由它拼出来（省得文档里再抄一份种类清单）；
/// * `play/tests/g4_spec.py` 拿它与真跑出来的表的 kind 集合逐字对账。
pub fn index_kinds() -> Vec<&'static str> {
    LEAVES
        .iter()
        .filter(|l| l.not_in_index.is_none())
        .map(|l| l.field)
        .collect()
}

/// `--control-schema` 里那几段的总和。
#[derive(Serialize)]
pub struct ControlFacts {
    /// 归属三态写在这个字段里。
    pub owner_field: &'static str,
    /// 删叶（「恢复出厂值」）写在这个字段里。
    pub remove_field: &'static str,
    /// 控制叶。
    pub leaves: &'static [LeafSpec],
    /// 命令列表。
    pub actions: &'static [ActionSpec],
}

/// 交给消费者（web / kit / 文档）的那一份事实。
pub fn facts() -> ControlFacts {
    ControlFacts {
        owner_field: OWNER_FIELD,
        remove_field: REMOVE_FIELD,
        leaves: LEAVES,
        actions: ACTIONS,
    }
}
