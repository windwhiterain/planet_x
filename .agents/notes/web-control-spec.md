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

## 7. 待裁决（请用户拍）

| # | 问题 | 我的推荐 |
| --- | --- | --- |
| **S1** | 叶种类注册表放**引擎**还是前端 `controls.json`？ | **引擎发结构事实**（§4），前端只发呈现——唯一能消灭两份手抄的位置 |
| **S2** | 左栏「控制 / 读面」两个模式**合并成一个**？ | **合并**（穿插的前提）；作用域归属退成 `owner` 行，控制树不再单独存在 |
| **S3** | 多键叶（预算 / 权重 / 设计图）在表里怎么给？ | 表内只给**单值叶**；多键叶进「这条记录的卡片」（行内展开） |
| **S4** | `web/src/views_tests.rs`（488 行 Rust 检查）怎么办？ | 搬到 `g4_spec.py` 后**删掉**，留指针 |
| **S5** | 本轮做到哪一步？ | 先交**第一步**（穿插可见 + 引擎发 leaves + `g4` 全绿），第二步（删手写控制树）紧接着 |

## 8. 风险

1. **这是界面重构，不是加一列**：两栏合并要动 `index.html` 的 DOM 与 `style.css`；写面 1200 行要拆。
2. **差异回传是语义，不能因为改渲染而丢**（§3.4 那五条）。→ `g4` 要把发出去的 **diff 原文**钉在数据上。
3. **声明写错不会编译报错**——所以 §6 的第 2/4 条必须是**双向集合相等**，不是「包含」。
4. 引擎那一改动要守住**行为中性**（只多发一段 JSON）；digest 是这条的判据。
