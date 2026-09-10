# 写面 spec 化 + 读/控制**穿插**（设计待裁决）

> 前身：[`web-human-views.md`](web-human-views.md) §9 **D5**（写面 spec 化 = 分两步，本轮走第二步）
> 与 §10.6；上一轮的收尾清单在 [`blueprint-stance.md`](blueprint-stance.md) §6。
> **状态：`[ ]` 设计待用户裁决（2026-10）**——本文只写方案，代码一行未动。

## 0. 一句话

把「**谁能在哪儿写什么**」也变成**声明（数据）**，并且让它和读面**住在同一份声明、同一条行序**里——
于是「这个势力有多少库存（读）」与「我给它多少钱（控制）」是**相邻的两行**，
而不是分在两个 tab 里让人来回跳。

## 1. 题面（用户原话，2026-10）

> 继续 spec 化 WebUI。重点：**控制和读面要穿插在一起**，比如一个势力的资源信息和控制资源预算
> 应该放在一（起）。注意：目前为了提升测试速度，测试正在逐渐迁移到 python，
> 所以请你**只使用 python 测试**。

## 2. 现状核实（带 `文件:行号`）

### 2.1 读面已经通用（这一半不动）

`web/static/views.json`（407 行，纯数据）+ `web/static/specview.js`（940 行，**一个领域词都没有**）。
铁律 R「只能整理，不能隐藏」由**构造**保证（残差 = 渲染时集合差）。见 `web-human-views.md` §3.2。

### 2.2 写面是**手写的领域代码**（`web/static/app.js`，2454 行里约 1200 行）

| 位置 | 是什么 |
| --- | --- |
| `app.js:242-275` | `KIND` 注册表：`childMode` / `scope` / `editor` **逐叶种类手写** |
| `app.js:603-621` | `LEAF_SPEC`：每片叶的**身份键**与**值字段**（手抄自引擎 struct） |
| `app.js:623` | `LEAF_OPTIONS`：哪些是势力级单叶 |
| `app.js:977-1054` | `buildTree()`：全局→势力→天体→城→建筑 的层级，**逐层手写** |
| `app.js:1069-1096` | `shipNode()`：一条舰 = 四叶容器，**逐叶手写** |
| `app.js:1317-1441` | `RAW_LEAF` / `DEFAULT_LEAF` / `effectiveMode` / `modeToggleFor`：归属三态，**逐叶种类手写** |
| `app.js:692-761` | `diffLeaf` / `buildScopeDiff` / `buildCommandDiff`：差异回传（**这一段是语义，要保**） |
| `app.js:1487-1600` | 四个编辑器（`doctrineEditor`/`kitingEditor`/`roleEditor`/`blueprintEditor`/`shipEditor`），**逐叶种类手写** |

### 2.3 同一份「叶种类事实」现在有**四份**副本

1. **引擎**：`src/control/wire.rs`（667 行，patch 结构）+ `src/control/*.rs` 的 apply/view——**权威**；
2. `--control-schema`：`view.rs:251` 的 `schemars::schema_for!(CommandReq)`——**只有形状、没有语义**，
   但它是**自动派生**的（所以「字段集合」这一半永不漂移）；
3. **web**：`app.js` 的 `LEAF_SPEC` + `KIND` + `RAW_LEAF` + `DEFAULT_LEAF`（§2.2 那四张表）；
4. **kit**：`play/planet_x_ctl/planet_x_ctl/__init__.py:107-161` 的
   `LEAF_KINDS` / `_VALUE_FIELD` / `_TWO_AXIS_KINDS` / `_COMPOSITE_KINDS` / `_BLUEPRINT_FIELDS`。

⇒ 这就是上一轮「删一个叶改了 11 处」的结构性原因（`blueprint-stance.md` §6 的实测账）。
两个已经踩过的坑都是这个缺口的实例：`role-axis-parity`（角色轴补三端）、
`blueprint-stance`（kit 的 `LEAF_KINDS` 缺项）。

### 2.4 两个模式是**分家**的

`app.js:295` `sideMode = 'read' | 'control'`，`index.html` 里 `#readPanel` 与 `#treePanel`
互斥显示。于是「这势力的库存是多少 / 我给的预算又是多少」必须**切 tab** 才看得全——
正是用户这次钉住的那件事。

## 3. 目标形状

### 3.1 写面只需要**三种**行原语（少而通用）

| 原语 | 是什么 | 例子 |
| --- | --- | --- |
| `leaf` | 一片**控制叶**（状态：存在性本身有意义） | `investment_budget` / `ship_orders` / `blueprints` |
| `owner` | **作用域归属**（不是叶：`global`/`factions`/`bodies`/`cities` 上的三态） | 「这个势力的舰默认归谁」 |
| `action` | **命令**（不是状态：写了就执行一次） | 建楼 / 拆楼 / 新建设计图（今天的 `buildings`） |

三者的共同点：都要「写一次、给一个差异」，都要归属 + 回执。差别只在**存在性**语义
（叶能删、命令不能）。

### 3.2 卡片 = 读行与写行**同一个有序数组**

`rows[]`（或沿用 `columns[]`）里每一项要么是 `path`（读），要么是 `leaf`/`owner`/`action`（写）：

```jsonc
{ "id": "faction-card", "title": "势力", "mount": "select", "select_kind": "faction",
  "layout": "sheet", "source": "@state.factions[*]", "key": "name",
  "rows": [
    { "path": "name", "label": "势力", "dot": "color" },
    { "path": "resources",                                  "label": "首都库存",     "fmt": "map", "digits": 0 },
    { "path": "@post.factions.${name}.production",          "label": "产出/月",      "fmt": "map", "digits": 1 },
    { "leaf": "investment_budget",   "label": "投资预算/回合", "editor": "number" },   // ← 控制
    { "leaf": "construction_budget", "label": "建造预算/回合", "editor": "number" },   // ← 控制
    { "path": "@post.factions.${name}.upkeep",              "label": "舰队维护/月",  "fmt": "num", "digits": 1 },
    { "path": "@post.factions.${name}.upkeep_unpaid",       "label": "没付上的维护", "fmt": "num", "digits": 1 },
    { "leaf": "default_role",  "label": "舰队默认角色", "editor": "role" },
    { "leaf": "default_kiting","label": "舰队默认风筝姿态", "editor": "kiting" },
    { "owner": "factions",     "label": "这个势力的舰默认归谁" },
    { "path": "@state.control.${name}.capital.value",       "label": "首都", "missing": "—" }
  ] }
```

**行序仍然 = 信息重要性的顺序**（`web-human-views.md` §4.1 的裁决 D7）：身份 → 此刻的结论 →
**钱与量（读的库存/产出紧挨写的预算）** → 原因与来源 → 引用与位置。

### 3.3 表（多记录）里能放什么

* **读列**：照旧；
* **单值叶**（`keys: []`，每记录一片，如 `capital`/`default_*`）：**可以**当列——格子 = 值 + 归属 chip；
* **多键叶**（`investment_budget` 是「每资源一行」、`ship_orders` 每舰一行、`blueprints` 每图一行）：
  **不进表列**，进「**这条记录展开的卡片**」（行内展开 / 底部选中卡）。
  判据来自 manifest 的 `keys` 是不是空——**不是**前端猜的。

### 3.4 差异回传的语义**一条不改**

`app.js:592-761` 那一整段（只回传变过的字段 / 写值即接管 / 原地改回则撤回自己钉的 `Player` /
壳被碰过就整片发 / `remove` 与值不可同条 / 作用域只报变过的键）**逐条保留**，
只是「哪片叶有哪几个身份键、值字段叫什么」从**硬编码**改成**读 manifest**。

> ⚠ 配对基准必须仍来自**权威读面**（`/api/state` 的 `control` 段），不能是界面上显示的那个数：
> 读面里的风格叶给的是**有效值**，拿它当基准会把「没有叶 ⇒ 跟随上层」变成「叶钉住这个数」。

## 4. 叶种类注册表：**引擎发结构事实，前端发呈现**

### 4.1 分工

| 事实 | 住哪 | 为什么 |
| --- | --- | --- |
| patch/view 的键名（`investment_budget`…） | **引擎** | 它就是 struct 字段名，`schemars` 已经发了 |
| 身份键 `["resource"]` / `[]` | **引擎** | 「怎么定位一片叶」是协议，写面与 kit 都靠它 |
| 值字段 `["value"]` / `["temper","lone_wolf"]` / `["class","components",…]` | **引擎** | 同上；kit 现在手抄了这份 |
| 归属字段 `mode` + `remove` | **引擎** | 三态 + 删叶是协议，不是界面 |
| 只读派生列（`ship_count`/`launch_waiting`） | **引擎** | 写面收下但不写回（今天已经是这样） |
| 编辑器种类、中文标签、单位 | **前端** | 界面词汇 |
| 行序、和哪些读行相邻、在哪一页 | **前端** | 组织 = `views.json` 的活 |

### 4.2 形状（草案）

`--control-schema` 现在发的是 `schemars` 的 `CommandReq`。**在同一个 JSON 里**加一段
（不新增命令、不新增端点）：

```jsonc
"leaves": [
  { "field": "capital",             "keys": [],                 "values": ["value"] },
  { "field": "default_doctrine",    "keys": [],                 "values": ["temper", "lone_wolf"] },
  { "field": "default_kiting",      "keys": [],                 "values": ["kiting"] },
  { "field": "default_role",        "keys": [],                 "values": ["role"] },
  { "field": "ship_orders",         "keys": ["ship"],           "values": ["behavior"] },
  { "field": "ship_doctrine",       "keys": ["ship"],           "values": ["temper", "lone_wolf"] },
  { "field": "ship_kiting",         "keys": ["ship"],           "values": ["kiting"] },
  { "field": "ship_role",           "keys": ["ship"],           "values": ["role"] },
  { "field": "investment_budget",   "keys": ["resource"],       "values": ["value"] },
  { "field": "construction_budget", "keys": ["resource"],       "values": ["value"] },
  { "field": "invest_weights",      "keys": ["city", "building"], "values": ["value"] },
  { "field": "build_weights",       "keys": ["city", "building"], "values": ["value"] },
  { "field": "loyalty_budget",      "keys": ["city"],           "values": ["value"] },
  { "field": "blueprints",          "keys": ["name"],
    "values": ["class", "components", "doctrine", "kiting", "role"],
    "read_only": ["ship_count", "launch_waiting"] }
]
```

**「加一个叶就要加一行声明」这条纪律由数据检查强制**（§6 第 2/4 条）：
`schemars`（自动派生）的属性集合 **==** `leaves[].field` 集合，多一个少一个都红。
这是读面 `neutral.rs` 那条「双向集合相等」守卫在**写面**的对应物。

### 4.3 为什么值得动引擎（而不是再抄一份到前端）

1. 它是**唯一**能同时消灭 web 与 kit 那两套手抄副本的位置；
2. 有了它，写面才**可能** generic（前端不再逐叶种类 `switch`）——这才是「spec 化」的实质；
3. 「kit 与 web 谁先认识新叶」这个**反复出现**的缺口（`role-axis-parity`、`blueprint-stance` 都栽过）
   从结构上消失：两端读同一份。

## 5. 落地步骤（建议分两步，第一步就能看见穿插）

* **第一步（穿插可见）**
  1. 引擎：`leaves` 段进 `control_schema_value()`（`src/control/view.rs`），声明就放在 `view.rs`/`wire.rs` 旁边；
  2. `views.json` 支持四种行（`path` / `leaf` / `owner` / `action`）+ 卡片布局里的写行渲染；
  3. `app.js`：写行渲染 = 值 + 归属 chip + ⋯（恢复出厂值 / 只看这一层）；差异回传语义照旧，只把
     `LEAF_SPEC`/`KIND` 里**能从 manifest 推出**的部分换成运行时读取；
  4. **左栏两模式合并成一个**（同一份 spec、同一批页）：势力 / 舰 / 城 / 全局 + 未组织；
  5. Python 检查组 `play/tests/g4_spec.py`（§6）。
* **第二步（删手写）**：`buildTree`/`shipNode`/`renderNode`/`KIND`/编辑器 switch 整段删掉，改由 spec 驱动；
  kit 的三张手抄表删掉改读引擎。

## 6. 验证（**只用 Python**——用户裁决）

* 现成三组：`uv run --project play/planet_xq python play/tests/run.py all -j 7`
  （`g1_contract` / `g2_mid` / `g3_long`）全绿；
* **新组 `play/tests/g4_spec.py`**（快档：几十回合 + 单点 dump，不进长局）：
  1. **静态纪律**（**从 `web/src/views_tests.rs` 搬**）：id 唯一 / 引用完整 / `omit` 不与列重叠 /
     路径合文法 / `@根` 已知；
  2. **写面对账**：`leaves[].field` 集合 **==** `schemars` 里 `FactionControlPatch` 的属性集合（双向）；
  3. **读面对账**：每条 `leaves` 的 `keys`/`values` 都能在**真实读面**的该叶条目上找到
     （起一个世界、写几片叶、再读回来——不是拿声明自证）；
  4. **认领完整性（铁律 R 的写面对偶）**：`leaves` 里每个 `field` 要么被某条 `leaf` 行认领、
     要么在 `omit` 里写明理由——否则新加的叶在界面上**凭空消失**（= 静默藏）；
  5. **防空转**（本仓硬要求）：每条守卫配一句「这一局里真的发生过 N 片叶」。
* 引擎行为中性：`--seed 42 --round 240 --digest 20`（取 `^{` 行、`\n` 连接、UTF-8 无 BOM、12 行）
  = **`975DC8A988F9846330DDCD3845B9C2D37C28D8E6E9893F26AB11DCC912C2E41B`**
  （本轮只加读面/声明，不改行为；口径见 [`code-layout.md`](code-layout.md) §3）。
* 实机：`scripts/web.ps1` 起在独立端口（本 worktree 与 main 的实例互不干扰），人工点 5 页 +
  写一片叶并「应用」，看回执。
* ⚠ **不再跑 `cargo nextest` / `cargo test`**：`web/src/views_tests.rs`（488 行）**搬到 `g4_spec.py` 之后删掉**，
  原处留一行「搬到哪儿」的指针（与 `test-decoupled-suite` 的做法一致）。

## 7. 待裁决（请用户拍 —— ✅ **2026-10 已全部按「推荐」批准**）

| # | 问题 | 我的推荐 | 裁决 |
| --- | --- | --- | --- |
| **S1** | 叶种类注册表放**引擎**还是前端 `controls.json`？ | **引擎发结构事实**（§4），前端只发呈现——唯一能消灭两份手抄的位置 | ✅ 按推荐 |
| **S2** | 左栏「控制 / 读面」两个模式**合并成一个**？ | **合并**（穿插的前提）；作用域归属退成 `owner` 行，控制树不再单独存在 | ✅ 按推荐（旧控制树第一步先降级成一个页，第二步删） |
| **S3** | 多键叶（预算 / 权重 / 设计图）在表里怎么给？ | 表内只给**单值叶**；多键叶进「这条记录的卡片」（行内展开） | ✅ 按推荐 |
| **S4** | `web/src/views_tests.rs`（488 行 Rust 检查）怎么办？ | 搬到 `g4_spec.py` 后**删掉**，留指针 | ✅ 按推荐 |
| **S5** | 本轮做到哪一步？ | 先交**第一步**（穿插可见 + 引擎发 leaves + `g4` 全绿），第二步（删手写控制树）紧接着 | ✅ 按推荐 |

用户追加的口径（2026-10）：**「用词规范先不管，最后再来统一」**——本轮不动措辞，能复用旧文案就复用。

---

## 11. 实现记录（第一步，分支 `feature/web-control-spec`）

### 11.1 引擎：一份结构事实

| 文件 | 是什么 |
| --- | --- |
| `src/control/leaves.rs`（新） | `LEAVES`（14 条）/ `ACTIONS`（1 条）/ `OWNER_FIELD` / `REMOVE_FIELD` + `facts()`。每条声明 `field`（= patch 字段名）、`keys`（身份键，空 = 势力级单叶）、`values`（值字段）、`carries`（读面顺带带过来的 state 属性）、`read_only`（读面有、写面不写回） |
| `src/control/view.rs` | `control_schema_value()` 把 `facts()` **并进同一份 JSON**（不新增命令、不新增端点）；`schemars` 那边一个字节没动 |
| `web/src/lib.rs` | 新增 `GET /api/control-schema`（前端启动拉**一次**，不是每帧） |

**实测**（`--control-schema` vs `schemars`）：
`leaves(14) ∪ actions(1) ∪ {faction_id}` **≡** `FactionControlPatch.properties(16)`，双向相等。
⇒ 「加字段不写声明」与「写一个不存在的叶」都**当场红**，这是读面 `neutral.rs` 那条守卫在写面的对偶。

### 11.2 `web/static/views.json` v2：四种行住进同一个数组

* `path`（读）/ `leaf`（一片控制叶，路径就是它在 `@control` 上的读路径）/ `owner`（作用域归属）/
  `action`（命令列表）。**顺序 = 穿插的顺序**：势力页上「首都库存、产出/月（读）」紧挨
  「投资预算、建造预算（控制）」；舰队页上「船体（读）」紧挨「指令（控制）」；城市页上
  「忠诚三项（读）」紧挨「娱乐/福利预算（控制）」。
* 顶层三张**呈现**表：`leaf_ui`（编辑器/标签/`key_label_from`/`keys_from`/`hint`）、`action_ui`、
  `write_omit`（没被任何行认领的叶，**理由必填**）。
* `keys_from` 是给「**还没有这片叶**」用的：今天的界面根本没法给一个还没写过叶的资源设预算
  （旧树只列已存在的叶）——现在候选键来自 `config.resources` / 本势力的城，改它 = 新建这片叶。
* 实测（`seed 7 / r30` 的 `--control`）：`investment_budget` 每资源一行、`capital` 是**单叶且没有
  `remove`**、`loyalty_budget`/`blueprints` 在早期回合是**空数组** ⇒「没有叶也能建」是**主路径**。
* 认领账（`g4_spec` 实测）：`leaf` 行认领 11 种、`action` 行 1 种、`write_omit` 带理由地免掉 3 种
  （`invest_weights` / `build_weights` 住在建筑行里、`blueprints` 仍住在旧页），**14+1 一个不落**。

### 11.3 测试：纪律检查整段搬到 Python（用户裁决：本轮只用 Python 测试）

* **新组 `play/tests/g4_spec.py`**（14 条判据，**0.3–0.4 s**，不吃投影缓存）：静态纪律五条（从
  `views_tests.rs` 搬）+ 写面对账 + 读面对账 + 认领完整性 + 防空转。做法是
  `--seed 42 --round 40 --save` 起短局 → 用**哨兵值**把 14 片叶各写一次（`--apply`）→
  `--control` 读回来 → **315 个读面条目**逐条对字段集 == `keys ∪ values ∪ carries ∪ read_only ∪ {mode}`。
  防空转 = 每片叶按哨兵**从读面**认领回来（不信 apply 回执）。
* `play/tests/run.py`：注册组 4，`DEFAULT = ("1","4")` ⇒ 内循环 2.9 s；`_harness.report()` 多一个
  分支——**没跑投影的组不再打印「全部命中（0 份）」**（那是假话）。
* **删掉 `web/src/views_tests.rs`（488 行）**，`web/src/lib.rs` 原处留一张「三条原测试 → 现在住
  g4 的哪一族」的指针表；`cargo check -p planet_x_web --all-targets` 绿。
* **反向验证 `play/tests/_g4_negative.py`**（不是组，不进 `run.py`）：把 `views.json` 与
  `--control-schema` 的**副本**逐个改坏喂给 `g4_spec.run`，要求「该红的红、基线绿」。
  实测 **16 个注入错全部咬住**（第二步又加了两条 `source` 形态的，现为 18）。**一条不会红的守卫等于没有守卫**，这份就是那条判据的量具。
* ⚠ 与设计稿不符、以引擎实测为准的一条：**读面从不发 `remove`**（315 个条目里 0 次）。
  `DefaultDoctrine`/`DefaultKiting`/`DefaultShipRole` 构造时写死 `remove: false` 而该字段
  `skip_serializing_if = "is_false"`；`capital` 是 `Control<BodyId>`，根本没这个字段。
  所以 g4 把 `remove` 实现成**宽容侧**（出现即允许、但只在势力级单叶上），并在 detail 里如实报「实测 0 次」。

### 11.4 行为中性：判据改用「与 `main` 同机同口径」

* 本分支实测：`--seed 42 --round 240 --digest 20`（12 行）= **`C928C3F1…06A9`**，
  与 `main`（`e430532` 与 `0385025` 两棵树）× `release`/`debug` 两种档**四个组合都逐字节相同**。
* ⚠ 这同时暴露一件事：`notes.md` 里那条基线 `975DC8A9…C2E41B` 在本机**复现不出来**（详情与处置见
  [`notes.md`](../notes.md) 末尾那条警告）。所以本轮起行为中性的判据是
  **「与 `main` 同机、同 config、同口径逐字节相同」**——不依赖任何历史记录，可当场复现。

### 11.5 前端：见 §12。

---

## 12. 实现记录（前端那一半，同一个分支）

### 12.1 交付物

| 文件 | 是什么 |
| --- | --- |
| `web/static/controls.js`（**新**，541 行） | 写面那一半：启动拉**一次** `/api/control-schema` 建 manifest；`decorateDoc` 给控制行补 label/`field`；`controlNode(col, rec, recKey)` 渲染三种行（`leaf`/`owner`/`action`）；`keys_from` + **壳**（还没有叶的行也能写，写值 = 新建这片叶）；`compact` 格；`audit()` 写面自检 |
| `web/static/specview.js`（+128） | 控制行**不解释**，只向宿主要节点：`ctx.controlNode(...)`；`spec.card` 行内展开 + `renderCard`；`claimedPaths`/`claimedKeys` 覆盖控制行；导出 `doc()` / `isControlRow`。**「本文件一个领域词都没有」这条铁律保住了**（新增的领域词只在注释里） |
| `web/static/app.js`（+649/−214） | `LEAF_SPEC`/`LEAF_OPTIONS` 改成**从 manifest 建**（`rebuildLeafSpec`）；`RAW_LEAF`/`DEFAULT_LEAF`/`AUTO_*_KINDS` 三张手抄表**删掉**（改由 manifest 的 `keys` + `leaf_ui.<field>.follows` 驱动）；抽出 `renderLeafNode(node)` 让旧树与新控制行**共用**同一批编辑器；左栏两个模式**合并**，旧控制树降级成「控制树（旧）」页；新增迁都编辑器（`bodyEditor`） |
| `web/static/index.html` | 删 `#sideTabs`（两个模式）；挂 `controls.js`；右侧状态面板不再自称「只读」（见 §12.3 第 4 条） |
| `web/static/style.css`（+35） | 控制行 / 归属 chip / 行内卡片 |
| `web/static/views.json`（3 行 + 一次挪位） | 三条逐舰轴加 `follows`（`effectiveMode` 用）；势力表把「投资预算/建造预算」挪到「首都库存/产出」**紧后面** |

### 12.2 实机验收（`scripts/web.ps1`，本 worktree 实例，全程 `errs: []`）

* **列序（读紧挨控制）**：势力表 `… 首都 | 城 | 舰 | 舰队 | 舰队默认角色 | 首都库存 | 产出/月 | 产出价值 | 投资预算/回合 | 建造预算/回合 | 维护/月 | …`；舰队表 `… 船体 | **指令** | **角色** | 运输此刻 | …`；城市表 `… ↳ 娱乐项 | **娱乐/福利预算** | 在造 | …`。
* **势力卡片**：`首都（迁都）→ 首都库存 → 产出/月 → 产出价值 → 投资预算/回合 → 建造预算/回合 → 舰队维护/月 → 没付上的维护 → 娱乐/福利预算 → 城 → 舰 → 舰队 → 舰队默认角色/风格/风筝姿态 → … → 这个势力归谁`。
* **「还没有叶」也能建**（今天的界面本来**根本做不到**）：在投资预算格里给没有叶的 `氢` 写 `2.5` ⇒ 回传 diff 原文只有那一片
  `{"control":[{"faction_id":"联合国","investment_budget":[{"resource":"氢","value":2.5,"mode":"Player"}]}]}`
  ⇒ 点应用 ⇒ 回执 `✓ 全部落地：1 条（没有丢弃、没有隐含接管、没有删叶）` ⇒ **刷新页面后读回 `氢 2.5`**（表格 compact 格：`氢 2.5 · 氦-3 0.6 · 碳 1.2 …（共 4 项）`）。
* **指令列可改**（真事件 `browser_select`）：`北斗` 改成「殖民 + 海王星」⇒ diff
  `{"control":[{"faction_id":"中国","ship_orders":[{"ship":"北斗","behavior":{"Colonize":{"body":"海王星"}},"mode":"Player"}]}]}`
  （值 + `mode` 一起、只有这一片）。
* **幂等**：给一片「还没有叶」的势力级默认叶选「玩家」⇒ diff 发整片（`{"role":"War","mode":"Player"}`，新建叶必须带全值）；再撤回「继承」⇒ diff = `{"control":[]}`。
* **同一片叶在两个面上是同一个对象**：右栏（`state.cities` 的 inline 表）里改 `长三角` 的娱乐预算，左栏城市页同一格的输入框立刻是同一个数；`edControl` 里**只有一条**该叶，diff 也只发一条。
  ⇒ 顺手辟掉一个我担心的坑（两处各造一片壳 ⇒ diff 里出现两条同身份的叶）：**没有发生**。
* **未组织页**：`@control` 不再是「整份未组织」，改成 `9 项 × 对象；按键的并集逐项标（共 15 个键）`，逐键写 `已整理 capital ← sel-faction` / `未组织 invest_weights`（后者在 `write_omit` 里，如实显示成未组织）。
* **运行时自检**：读列与控制行分开报——`faction-table 9 条记录 / 18 个读列（含 4 条控制行，其中 1 条的叶这一帧还不存在 = 这一层没表态，界面给新建入口）`；**写面自检**把 14 片叶逐条标「被组织点认领 / 声明不看（理由）」。
* **旧页没坏**：`控制树（旧）` 页仍渲染 `全局 → 势力 tab → 设计图库/舰/预算 tab → …`；推进 1 回合正常（`已推进 1 回合`），写进去的叶不被冲掉。

### 12.3 有意的偏离（写下来，别让下一个人以为是 bug）

1. **新控制行的编辑器不再按「有效归属是玩家」上闸门**（旧控制树那一页的闸门**没动**）。两条理由：
   ① 没有闸门才可能在「还没有叶」的行上写值——那是新建这片叶的**唯一**入口；
   ② 舰队页的指令列必须可编辑。归属信息由 chip + 一句「这片叶没表态（继承）⇒ 现在按上层：Inherit；改一个值就归你」承担。
   ⚠ 这条是**产品取舍**，用户看过再说；要恢复闸门就得另给「新建」入口。
2. `rawLeafOf` 现在认**原始 state 的形状**：`state.control` 里的键叶是**映射**（`{"碳":{value,mode}}`），读面里才是数组。
3. 运行时自检把控制行与读列**分开判**：`leaf` 行的值是 `null`/空数组 = 「这一层还没表态」，报成"取不到值"是反方向的谎话。
4. 右侧「状态」面板不再自称**只读**：被组织点认领的集合会**就地**换成整理后的表，而那张表现在也有控制行（实测右侧面板里 **122 个**可编辑控制格，例如 `state.cities` 那张 inline 表里的娱乐/福利预算）。选择改标题而不是给 inline 单独走只读渲染——**因为它们本来就是同一批对象**（见 §12.2 那条），两边不一致的风险不存在。
5. 编辑器对 `<select>` 类控件靠 **real 事件**（`input`）触发；合成 `change` 不会触发（验收脚本要注意这一点，我踩过一次）。

### 12.4 未做（第二步及以后）

* **第二步**：`buildTree`/`shipNode`/`renderNode`/`KIND`/编辑器 switch 整段删掉，让「控制树（旧）」页消失。
* `blueprints`（设计图库）与 `invest_weights`/`build_weights` 搬进卡片（现在靠 `write_omit` + 旧页），
* **kit 的三张手抄表**（`LEAF_KINDS`/`_VALUE_FIELD`/`_TWO_AXIS_KINDS`/`_COMPOSITE_KINDS`/`_BLUEPRINT_FIELDS`）改读引擎那份 manifest——`--control-schema` 已经发了，删掉它们才是「一份事实、三端共用」的完全体。
* 用词统一（用户：**最后再来统一**）。



## 8. 风险

1. **这是界面重构，不是加一列**：两栏合并要动 `index.html` 的 DOM 与 `style.css`；写面 1200 行要拆。
2. **差异回传是语义，不能因为改渲染而丢**（§3.4 那五条）。→ `g4` 要把发出去的 **diff 原文**钉在数据上。
3. **声明写错不会编译报错**——所以 §6 的第 2/4 条必须是**双向集合相等**，不是「包含」。
4. 引擎那一改动要守住**行为中性**（只多发一段 JSON）；digest 是这条的判据。
