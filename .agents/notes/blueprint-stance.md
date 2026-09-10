# 设计图带**倾向**、指令只剩**逐舰叶**（2026-10 裁决与实现）

**一句话**：**指令（ShipBehavior）是即时操作，只写逐舰叶**；**设计图带的是长期倾向**
（风格 / 风筝姿态 / 角色）——它是"这型舰是什么"，不是"这艘舰现在去干什么"。
势力级那片 **`default_ship_order` 整片删除**（实测它不是"默认值"而是**全舰队接管开关**）。

相关：`ship-blueprint.md`（图的原始设计）、`ship-blueprint-spec.md`（实现规格，本文档改写了它的
§「图的意图轴」）、`control-live-layers.md`（四条 / 三条控制轴）、`agent-play.md`（agent 手册）。

---

## 1. 用户裁决（原文与含义）

| # | 裁决原文 | 含义 |
|---|---|---|
| A1 | 「舰船默认行为没用啊，行为是即时操作，风格/角色才是长期控制项」 | 舰队级**不该**有"默认指令"；长期控制项 = 风格/角色 |
| A2 | 「不过也可以加上行为」 | 指令**仍然**要能表达（逐舰批量下令），只是不进舰队级/图级 |
| A3 | 「蓝图不需要指定指令，可以指定 风格/角色」 | `Blueprint.order` 删除；`Blueprint` 加 `doctrine` / `kiting` / `role` |
| A4 | 「现在控制面板没法写 风格/角色」 | Web 的设计图编辑器必须能写这三条轴（原来只有"意图"那一格） |
| A5 | 「删掉」（问：舰队级 `default_ship_order` 怎么办） | **删除**该叶（连同 wire/view/apply/normalize/projection/三端/文档） |

## 2. 实现前的实测：那片叶到底是什么（这是裁决的依据）

用真实引擎跑（seed 42，中国，写一片 Player 的「停泊地球」）：

| 实验 | `order_source` 分布（中国，末回合） |
|---|---|
| 开局就写，跑到 40 回合 | `fleet_default` 14/14 |
| **第 20 回合才写**（AI 已写了 20 回合逐舰指令），再跑到 40 | `fleet_default` 14/14 |

第二条是关键：AI 每回合都在写逐舰指令，**为什么全被盖掉**？因为
`autocontrol::tactics` 写的是 `Control::inherit(behavior)`（`ControlMode::Inherit`，不是 `Auto`），
而取值链是「叶 Inherit ⇒ 先看图、再看舰队默认（Player 时）」⇒ **舰队默认一写，全舰队的 AI 指令
在取值上同时失效**。

再叠一层：`State::ship_control` 对"没叶 / 叶是 Inherit"的舰**继承舰队默认的归属** ⇒ 全舰队解析成
`Player` ⇒ `autocontrol::{style,freight,contract}` 与 `sim/military.rs` 的 `is_ai` 闸门**全部跳过
这些舰**：不再重估风格、不再定编跑运输、不再接单，连 `tactics` 里的**自保撤退**都不生效
（移动读的是 `ship_behavior()` 有效值）。

**结论**：那一叶不是"默认值"，而是**一次点击把整个舰队从 AI 手里拿走、钉死在同一条站桩指令上**，
而且会给几十回合后下水的新舰继承一条过期命令。名字与作用不符 ⇒ 按用户裁决**删除**。

## 3. 新模型

### 3.1 指令：只有逐舰叶

```rust
// State::ship_behavior —— 唯一供值者
c.ship_orders.get(&ship_id).map(|l| l.value.clone())   // 叶不存在 ⇒ None（调用方按 Idle 兜底）
```

* 归属链：`叶 → 势力 scope → 全局 scope`（`State::ship_control`）。
  ⚠ **图不参与**：钉死选装（图设为 `Player`）**不该**连带把整支舰队的指令权收走（旧的 Q5 守卫，
  现在由"图根本不表态指令"从结构上保证）。
* `OrderSource` 只剩 `Leaf`（`Scope`/`Record` 留在取值域里，链上不会出现）。
* 新舰下水时船坞写一片**沉默的** `Idle` 叶 ⇒ 归属 `Auto`、归 AI 决定。

### 3.2 倾向三轴：叶 → **图** → 舰队默认 → 记录值

```rust
// State::ship_doctrine / ship_kiting / ship_role（三条同形）
if leaf_mode(leaf) == Inherit {
    if let Some(v) = self.blueprint_stance_role(s) { return v; }      // ① 出厂图（本舰那张、图上写了这条轴、图归 Player）
    if let Some(d) = &c.default_role { if d.mode.is_player() { return d.value } }   // ② 舰队默认
}
leaf.map(|l| l.value).unwrap_or(record)                                // ③ 叶里的值 / 舰上记录值
```

* **图插在舰队默认之前**：与原来行为链的 ① ② 同序——"这型舰"比"全势力默认"更具体。
* 三个前置条件（缺一即"这一层没有说话"）：有出厂图 / 图上写了**这条轴** / 图归属解析为 `Player`
  （`blueprint_stance_*` 与 `blueprint_speaks`，见 `src/model/state.rs`）。
* 逐轴独立：图上写了 `role` 不影响 `doctrine`。
* 归属链（`ship_style_chain`）同样插入图层，且**只在图上写了该轴时**才参与
  （保证"钉死选装"不会连带收走三条倾向轴的归属权——与 Q5 同一条纪律）。

### 3.3 数据结构

```rust
pub struct Blueprint {
    pub class: String,
    pub components: Vec<String>,          // 空 = 交给生成器
    pub doctrine: Option<ShipDoctrine>,   // 新增：风格两轴（一片）
    pub kiting: Option<f64>,              // 新增：风筝↔贴脸
    pub role: Option<ShipRole>,           // 新增：战舰 / 运输舰 / 观测舰
    // order: Option<ShipBehavior>        // ← 删除
}
pub struct ControllableState {
    // default_ship_order: Option<Control<ShipBehavior>>,   // ← 删除
    pub default_doctrine: Option<Control<ShipDoctrine>>,    // 保留（长期倾向）
    pub default_kiting: Option<Control<f64>>,
    pub default_role: Option<Control<ShipRole>>,
    ...
}
```

`SCHEMA_VERSION` 21 → **22**。**不迁移**：两片删除的叶没有等价物（正是被裁决删掉的东西），
旧档那两个键被 serde 忽略 ⇒ 旧局面的指令归属回到作用域链（`Auto`）。要复原就**逐舰重写指令**；
把"舰队默认"摊到每艘舰的显式叶上**不做**——那会把一条早已过期的站桩令伪装成"玩家逐舰下的命令"。

## 4. 三端

| 端 | 改动 |
|---|---|
| 引擎 | 上面 §3；`projection.rs` 删 `order_default_mode` / `order_blueprint_mode` 两列、蓝图表 `order` → `doctrine`/`kiting`/`role`；`control` 表不再发 `default_ship_order` 行 |
| kit | `set_default_ship_order` / `remove_default_ship_order` / `silence_blueprint_order` 保留名字但**抛 `AttributeError`**（响亮过时）；`set_blueprint(..., doctrine=, kiting=, role=)`；新增 `silence_blueprint_stance(..., doctrine=/kiting=/role=)`；`_BLUEPRINT_FIELDS` 换三轴；`ships()` 的 `default_ship_order_*` 列改成 `default_role_*` |
| web | 设计图编辑器把「意图」那一格换成**倾向三行**（角色下拉 + 风格两轴 + 风筝姿态，各带"不表态"）；舰队分组里删掉「舰队默认指令」那一行；`DEFAULT_LEAF` 去掉 `shiporder`；`views.json` 的读面列不变（指令的有效值仍然来自叶） |

## 5. 测试与验证

* 引擎新增/改写：`control::tests::ship::orders_come_only_from_the_per_ship_leaf`、
  `the_fleet_default_order_leaf_is_gone`（写它必须**报错**）；`sim::tests::blueprints::
  blueprint_role_governs_new_ships` / `fleet_default_still_covers_blueprintless_ships` /
  `order_source_separates_a_missing_leaf_from_a_silent_one`；`sim::tests::fleet::
  newly_built_ships_have_no_order_of_their_own`；`control::tests::view` 与 `projection` 的
  读面断言同步。
* `cargo nextest run -P full`：**240 passed / 0 failed / 34 skipped**（原 238 ⇒ 净增 2）。
* kit：`python demo.py` 全部断言通过（§4d 改写成"叶里的值 = 有效值"这条新不变式）。
* **digest = 中性**（最强的一条证据）：`--seed 42 --round 240 --digest 20` 的 12 行 JSON 与
  `main`（`76753c0`）**逐字节相同** = `975DC8A988F9846330DDCD3845B9C2D37C28D8E6E9893F26AB11DCC912C2E41B`。
  为什么应当相同（不是巧合）：默认局里**没人写过**那两片被删的叶（没有 `--apply`、没有舰队默认、
  AI 建的图三轴全 `None`），所以
  「`resolve_chain([leaf, Inherit, Inherit, faction, global])`」≡「`resolve_chain([leaf, faction, global])`」、
  「叶 Inherit ⇒ 图（沉默）⇒ 默认（不存在）⇒ 叶值」≡「叶值」。**变化只发生在有人真的写了那些叶的局面上**，
  而那正是裁决要改的东西。（顺带回填了 `main` 的基线：`C928C3F1…06A9` 是 B5 之前的，
  B5 各批改过行为但没记；新基线见 `notes.md`。）

## 6. 没做 / 待办

* **`order_source` 没进 web 的 control 读面**（这是上一轮 `web-human-views.md` §10.6 那条"图层的
  来源读不出来"的姊妹问题）：倾向三轴的**供值者**（叶 / 图 / 舰队默认 / 记录值）在 web 里看不见。
  引擎侧已经有 `State::ship_behavior_source`，但倾向三轴还没有 `ship_*_source`。建议下轮补。
* **AI 不写图上的倾向**（`autocontrol::blueprints` 建的图三轴全 `None`）：AI 继续逐舰写叶。
  "让 AI 通过改图来定全型舰的倾向"是一条**平衡改动**，另开裁决。
* `default_ship_order` 删除后，agent 的"一次说清全舰队"只能**逐舰点名**（N 片叶）。
  若要找回那个手感，可以做**批量下令动作**（一次点击 = 给当前每一艘舰写一条 Player 指令叶），
  但它是**动作**而不是长期默认——本轮没做，因为它牵动 web 的写面交互（另开裁决）。
