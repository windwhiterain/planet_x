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
//!    字段集合**对齐**（⇒ 改结构体却忘了改声明 = 红）。
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
    /// 这片叶在 patch / 读面里的**键名**（= `FactionControlPatch` 的字段名）。
    pub field: &'static str,
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
        keys: &[],
        values: &["值"],
        carries: &[],
        read_only: &[],
    },
    LeafSpec {
        // `DefaultDoctrine`：两个轴一起给（单轴新建会被引擎响亮拒绝）。
        field: "舰队默认风格",
        keys: &[],
        values: &["temper", "lone_wolf"],
        carries: &[],
        read_only: &[],
    },
    LeafSpec {
        field: "舰队默认姿态",
        keys: &[],
        values: &["姿态"],
        carries: &[],
        read_only: &[],
    },
    LeafSpec {
        field: "舰队默认角色",
        keys: &[],
        values: &["角色"],
        carries: &[],
        read_only: &[],
    },
    // --- 逐舰四叶（`ShipId` 定位）-------------------------------------------------
    LeafSpec {
        // 指令**没有**势力级那一片（2026-10 删除）：它是即时操作，只写逐舰叶。
        field: "指令",
        keys: &["舰"],
        values: &["行为"],
        carries: &[],
        read_only: &[],
    },
    LeafSpec {
        field: "风格",
        keys: &["舰"],
        values: &["temper", "lone_wolf"],
        carries: &[],
        read_only: &[],
    },
    LeafSpec {
        field: "姿态",
        keys: &["舰"],
        values: &["姿态"],
        carries: &[],
        read_only: &[],
    },
    LeafSpec {
        field: "角色",
        keys: &["舰"],
        values: &["角色"],
        carries: &[],
        read_only: &[],
    },
    // --- 逐资源预算 ---------------------------------------------------------------
    LeafSpec {
        field: "投资预算",
        keys: &["资源"],
        values: &["值"],
        carries: &[],
        read_only: &[],
    },
    LeafSpec {
        field: "建造预算",
        keys: &["资源"],
        values: &["值"],
        carries: &[],
        read_only: &[],
    },
    LeafSpec {
        field: "福利预算",
        keys: &["资源"],
        values: &["值"],
        carries: &[],
        read_only: &[],
    },
    // --- 逐建筑权重（身份 = 城 + 建筑；读面顺带带着建筑的 state 属性）-----------------
    LeafSpec {
        field: "建设权重",
        keys: &["城", "建筑"],
        values: &["值"],
        // `InvestWeightEntry` 从 `state` 里 join 进来的四列（`view.rs` 的
        // `invest_weights` 映射）：写面不看它们，只用来显示「这是座什么楼、在造什么」。
        carries: &["类型", "资源", "建造舰级", "结构"],
        read_only: &[],
    },
    LeafSpec {
        field: "建造权重",
        keys: &["城", "建筑"],
        values: &["值"],
        carries: &["建造舰级"],
        read_only: &[],
    },
    // --- 逐城的娱乐/福利预算 --------------------------------------------------------
    LeafSpec {
        field: "城市福利预算",
        keys: &["城"],
        values: &["值"],
        carries: &[],
        read_only: &[],
    },
    LeafSpec {
        field: "开发货币预算",
        keys: &["城"],
        values: &["值"],
        carries: &[],
        read_only: &[],
    },
    LeafSpec {
        field: "建造货币预算",
        keys: &["城"],
        values: &["值"],
        carries: &[],
        read_only: &[],
    },
    // --- 设计图库（复合值叶：舰级 + 选装 + 倾向三轴，逐轴可空 = 该轴沉默）--------------
    LeafSpec {
        field: "设计图库",
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
}];

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
