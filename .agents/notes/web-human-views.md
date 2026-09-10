# WebUI 面向人类的读面：**声明式组织点 + 通用兜底**（设计 + 实现）

> 状态 `[x]` **已实现**（`feature/web-human-views`；读面组织层 + 铁律 R + 通用层的三处静默上限；
> **引擎一行未动**）｜ 索引：[notes.md](../notes.md)
> 工作树：`C:\resource\planet_x-webviews`（主工作树 `C:\resource\planet_x` 上只做了只读勘察 + 截图）
>
> 下一步（用户已定）：**D8 = Rust 结构体字段按重要性重排**（见 §10.6）；写面 spec 化（D5）另开。

> 用户的题面（2026-10，本轮）：
> **「优化 web 端人类玩家的游玩体验。第一步：重新审视 WebUI。web ui 的理念本来是接受任意结构的数据
> 输入。但对于人类来说，许多数据需要一定的组织方式。因此需要指定一个设计，即 WebUI 以某种方式
> procedurally 地重新组织数据，但重组点以外的部分被扩展时依然能 generic 地被 WebUI 处理。」**
>
> 本篇 = 第一步的交付物：**核实现状 → 量出缺口 → 给出设计（含一条可检验的铁律）→ 列出要裁决的点**。
> 实机勘察（round 20，本会话起在 `http://127.0.0.1:3000`）的截图留在主工作树
> `scratch/shots/00-default.png` … `06-earth-zoom.png`（未跟踪）。

---

## 0. 一句话

现状是**「读面通用但无组织，写面有组织但硬编码」**：通用 widget 能把任意 JSON 铺成表，人类却读不懂
（118 KB/帧、英文蛇形字段名、没有标签单位、没有"为什么"）；而唯一真的按人的方式组织的界面
（左侧控制树 ~1200 行）是**手写死**的，引擎每加一条控制轴就要手改三处。

设计：**把"组织"从代码里抽成数据**——一份声明式的 `views.json`（**组织点 view spec**），由一个
通用求值器渲染；并且定一条可检验的铁律：

> **R（只能整理，不能隐藏）**：展示 = **认领（整理形态）⊎ 残差（通用形态）**，
> 而**残差是渲染时用集合差算出来的**（`该记录实际字段 ∖ 认领字段`），不是声明的。
> 于是引擎加字段 ⇒ 它自动落进残差、自动可见；引擎加新根 ⇒ 它自动落进"未组织索引"、自动可见。
> 唯一能把数据藏起来的方式是**显式 `omit`（带理由）**，且界面必须把这处省略**明说出来**。

这就是题面那句「重组点以外的部分被扩展时依然能 generic 地被 WebUI 处理」的结构性保证：不是靠人
记得去更新前端，而是靠**集合差**这条恒等式。

---

## 1. 现状核实（带 `文件:行号`）

### 1.1 后端：严格的**数据平面**（这一半是对的，不动）

`web/src/lib.rs:101-136` 的注释把理念写死了：`/api/state` 刻意**只有两样东西**——

* **写面** `control` / `scope`：`planet_x::control::control_view` / `scope_view` 的语义化模板
  （`Control<T>` 展开成 `{value, mode}`、`(城, 建筑)` 元组键展开成 `{city, building, kind}`）。
* **读面** `info`：`info_roots()`（`lib.rs:173-203`）里**每个根 = 模型的整份 JSON dump**
  （`state` / `pre` / `post` / `config` / `session`），走 `planet_x::json::to_value`（RON 里非字符串
  map key 也能落 JSON）。

注释原话：「**没有**任何『给前端的拍平视图』：以前那些 `bodies`/`cities`/`ships`/`FactionView`/
`MetaView` 字段全是手工挑选的投影（会漂移、会漏字段、加字段要改两处）」。

> 结论 1：**引擎侧不许再长出"给前端拍好的视图"**。任何组织逻辑落在引擎里，就是这篇文档要避免的
> 「手工投影」回魂。这是 §9 D1 的判据。

### 1.2 前端读面：一个 schema-agnostic widget，铁律写在文件头上

`web/static/jsonview.js:1-27` 的自述：「本文件**不认识任何领域字段名**……只认 JSON 的形状，并且
『形状 → 布局』的决策只有一处（`layoutOf`，`:45`）」；`app.js:10-13` 同样声明「本文件不写任何字段名，
只决定『渲染哪个根』」。

调用点只有三处（`app.js`）：

| 位置 | 是什么 |
| --- | --- |
| `renderInfo()` `app.js:1892-1917` | 右侧「状态」面板：五个根各一个 tab，整份 dump 交给 widget |
| `renderSelection()` `app.js:658-684` | 底部「选中读面」：在 `state` 顶层数组里按 `name` 找到那条记录，整份交给 widget |
| `renderDiff()` `app.js:1946-1958` | 底部那行"上回合"：**只比数组长度** |

### 1.3 前端写面：**1200 行手写领域代码**

左侧「控制层级」树 + 全部编辑器是 `app.js:686-1886`（约 1200 行）：

* `KIND` 注册表 `app.js:241-275`：34 种节点（`global`/`faction`/`group`/`body`/`city`/`ship`/
  `shiporder`/`shipdoctrine`/`shipkiting`/`shiprole`/`fleetorder`/…/`blueprint`），每种带
  `childMode` / `scope` / `editor` / `decorateLabel`。
* `LEAF_SPEC` `app.js:366-386`（叶的身份键与值字段，回传 diff 用）、`RAW_LEAF`/`DEFAULT_LEAF`、
  8 个编辑器（`shipEditor` `:1246`、`doctrineEditor` `:1200`、`kitingEditor` `:1208`、
  `roleEditor` `:1222`、`value` `:1324`、`buildingEditor` `:1779`、`blueprintEditor` `:1474`、
  `orderEditor` `:1573`）。

`control-live-layers.md` §12.1 记着这条的代价：角色轴（第三条风格轴）落地时，
「web 面板不认识它：`LEAF_SPEC` / `KIND` / `RAW_LEAF` / `DEFAULT_LEAF` 只有指令 + 风格两轴」
⇒ 引擎加一条轴，前端要手改四处才不瞎。

> 结论 2：写面的"组织"是**手写的 procedural**，读面的"组织"是**没有**。题面说的"需要一定的组织
> 方式"，缺的正是读面这一半；而写面那一半的病是**同一个病**（组织逻辑写成了代码，不是数据）。

---

## 2. 实机量出来的缺口

### 2.1 一帧给人类的是 118 KB JSON，铺成一张没有重点的网

round 20（22 城 / 18 舰），`GET /api/state` 的 `info` 各根字节数（实测）：

| 根 | 字节 |
| --- | --- |
| `state` | 69 146 |
| `post` | 22 256 |
| `config` | 15 874 |
| `pre` | 10 762 |
| `session` | 53 |

实机一次渲染 `state` 根，DOM 里出现 **119 个 `<th>`**（多张自动表的表头合计）；`state` 顶层 17 个键
（`bodies`/`chronicle`/`cities`/`contracts`/`control`/`depots`/`events`/`factions`/`market`/
`milestones`/`notables`/`round`/`schema_version`/`scope`/`ship_name_seq`/`ships`/`time_month`）
被**平铺**成同一层的一堆表——没有分组、没有优先级、没有"先看这个"。

### 2.2 ⚠ 通用兜底自己会**静默丢数据**（这是本轮最该记的一条）

* `jsonview.js:31` `MAX_COLS = 16`；`colsOf()` `:228-238` 一旦凑够 16 列就 `break outer`。
* **实机验证**：`state.ships` 的 18 条记录键并集 = 18 列
  （`attack_hist, blueprint, cargo, class, component_hp, components, doctrine, faction_id, hull,
  hull_max, kiting, name, position, role, shield, shield_max, spawned_round, velocity`），
  而渲染出的表头**正好 16 列 ⇒ `spawned_round` 与 `velocity` 被默默扔掉了**，
  界面上**一个字的提示都没有**。
* 同类静默上限还有：`PAGE = 50`（分页按钮会说"还有 N 项"，这条是好的）、`PREVIEW = 72`
  （折叠预览串截断，无提示）、`MAX_DEPTH = 14`（无提示）。

> 这条正好是题面的反例：**一次纯扩展（引擎加两个字段）就把兜底撑破了，而现象是"数据不见了"**。
> 所以「重组点以外的部分依然 generic 可用」不能只靠"残差自动出现"，还必须**先把通用层的静默上限
> 改成明说的上限**。

### 2.3 推进 20 回合，人类得到的反馈是零

实机：点「▶ 推进（20）」后，状态行只有 `已推进 20 回合`；底部那行是
`上回合: 0 → 20  chronicle 0→8  events 0→26  ships 21→18`（`renderDiff` 只数数组长度）。

而同一帧里**明明有**（实测取值，全在 `post` / `state` 里躺着）：

| 数据 | 实测内容 |
| --- | --- |
| `post.wars` | 6 对交战国（`[中国,欧盟] [俄罗斯,无国界科学组织] …`） |
| `post.hegemon` / `post.power_share` | 霸权 = 中国；占比 中国 .32 / 欧盟 .31 / 俄罗斯 .30 |
| `post.decisions.blueprints` | AI 每回合在建/改/回收设计图（`action: created/retuned/reaped` + `theme` + `city`） |
| `post.haul_steps` | 逐舰"此刻在哪一步"（`loaded` / `en_route` / `waiting` / `delivered`） |
| `post.market_trades` | 逐笔成交（`buyer/seller/resource/moved/freight_rate/loss/rel_mult`） |
| `state.events` | 本回合事件（`contract_posted` …，懒表里还带 `headline`/`salience`/`magnitude`） |

**数据全在，界面一个字都不说。** 这正是"人类需要组织方式"的最直接证据：不是缺数据，是缺**排列**。

### 2.4 没有语义单位：标签、单位、格式化、"为什么"

* 字段是引擎的英文蛇形名：`upkeep_unpaid` / `governance_coverage` / `loyalty_target_effective` /
  `freighter_quota_share`；数字是裸浮点（`0.054586936677831224`、`1.1521`）。
* `config` 里**本来就有**中文标签（建筑 `label`、舰级 `label`、资源名已是中文 WYSIWYG key——
  `wysiwyg-resource-keys.md`），但只有控制树用了它们（`app.js:557-560` 三个 `*Name()` 助手）。
* 引擎已经在 B1/B2/B3 三批里**捕获了过程量**（`step-intermediates.md`：忠诚三分项、钱去哪了、
  市场与运输），也就是"**为什么**"的原料都在读面里——但读面从不把它们摆到对应实体的旁边。

> 结论 3：人类要的不是"更多数据"，而是**（i）把散在不同根里的同一条事实接在一起（join）、
> （ii）给它中文名与单位、（iii）把原因摆在结果旁边**。这三件事都是**排列**，不是新数据。

---

## 3. 设计：三个层次 + 一条铁律

### 3.1 术语

* **数据（data）**：引擎 dump 的任意 JSON（§1.1，不改）。
* **组织点（view spec / mount）**：一份**声明**——「把哪些记录、按什么键、取哪些字段、按什么顺序、
  摆成什么形状」。**是数据，不是代码。**
* **通用兜底（generic fallback）**：`jsonview.js` 原样渲染**没有被任何组织点认领**的部分。
* **求值器（spec renderer）**：一个通用函数，**只认 spec 的形状，不认领域字段名**——与 `jsonview.js`
  同一条铁律。于是「新增一个组织点 = 写一段数据」，前端一行代码都不用改。

### 3.2 铁律 R（只能整理，不能隐藏）——可检验

对任一组织点 `s`（认领域 `D(s)` = 它声明的路径集合）与任一帧数据 `W`：

```
展示(W)  =  认领(W)  ⊎  残差(W)
认领(W)  =  { w ∈ W | w ∈ D(s) }          整理形态（表格行 / 卡片字段 / 时间线项…）
残差(W)  =  W ∖ D(s)                       通用形态（jsonview 渲染的"其余字段"）
```

* **残差在渲染时用集合差算出来**（不是声明出来的）⇒ 恒等式**由构造保证**，不会漂移。
* **推论 1（字段级扩展）**：引擎给已认领的记录加字段 ⇒ 落进残差 ⇒ **下一帧就看得见**。
* **推论 2（集合级扩展）**：引擎加一个新根 / 新顶层数组 ⇒ 没有任何 spec 认领它 ⇒ 落进
  **「未组织」索引**（一个通用的根列表）⇒ 同样看得见。
* **推论 3（不许静默截断）**：任何上限（列数 / 行数 / top-N / 预览长度）必须把"藏了多少"**写出来**
  （§7 就是拿这条去修 §2.2 的两个洞）。
* **唯一例外**：显式 `omit: [{path, why}]`——界面把省略**明说**成
  「（声明省略：配置表；理由 …）」，而不是让它凭空消失。

> 为什么这条铁律值得写进文档而不是"注意一下"：本仓反复拉黑「失败看起来像成功」
> （`agent-play.md`、§2.2 的静默丢列就是它的一个实例）。**"数据看起来没有"和"数据被藏了"
> 必须长得不一样**，这是同一件事在读面上的版本。

### 3.3 三个层次

| 层 | 是什么 | 改不改 |
| --- | --- | --- |
| **数据面** | `lib.rs` 的 `control`/`scope` + `info`（整份 dump） | **不改**（可选：多发一段 schema，见 §9 D1） |
| **组织层** | `web/static/views.json`：一批 view spec（数据） | **新增**（本轮的主角） |
| **渲染层** | `jsonview.js`（通用，已有）+ `specview.js`（新，通用求值器） | 求值器新增；`jsonview.js` 只修 §7 的上限 |

三层之间的唯一契约是 **JSON 形状 + 路径**，没有第四种东西（不新增手写投影、不新增第二套 schema）。

---

## 4. 组织点的原语（少而通用）

> ⚠ 下面这一节是**落地后的实际语法**（与 `web/static/views.json` 逐字对应）。
> 设计初稿曾用 `from: {root, by, then, match}` 那种"join 对象"，落地时被**一条路径表达式**取代：
> join 只是"路径里可以跨根"，见 §4.2。少一个原语、少一处要学的东西。

### 4.1 列序即优先级（用户追加的裁决 D7）

`columns` **数组的顺序就是信息重要性的顺序**——不是 JSON 顺序、不是字母序。编排原则：

1. **身份**（`name`）——人靠它锚定"这是哪一条"，放第一列；
2. **此刻的结论**（在干什么 / 忠诚 / 船体 / 战和）——回答"现在怎么样"；
3. **原因与来源**（忠诚三分项、指令是谁说的、钱花在哪、买不起还是没产能）——**紧挨结论**；
4. **量与资源**（产出、维护费、购买力、载货）；
5. **引用与位置**（天体、坐标、首都）；
6. **内部 / 元数据**（`spawned_round`、`attack_hist`、`velocity`…）——多数**不进列**，
   落进残差（它们不消失，只是不占视线）。

行序同理，也按重要性：**城市按忠诚升序**（最危险的在前）、**势力按实力占比降序**、挂单按量降序、
价格按价降序。首列若就是 `key`，就把它当身份格，**不再单开一列**（否则名字出现两次）。

### 4.2 路径表达式（唯一的"取数"语言）

```
expr  := seg ('.' seg)*
seg   := name | name '[' pick ']' | name '[' '*' ']' | '@key'
pick  := index | '?' field '=' value          （value 可用 ${…} 模板）
name / value 里可嵌 ${字段} 模板（相对当前记录求值；${@key} = 映射键）
首段以 '@' 开头 = 绝对路径（@state/@pre/@post/@config/@session/@control/@scope）；
否则相对当前记录。整条路径**可空**：任何一步取不到 ⇒ 该格显示 missing 文案，**列不消失**。
```

**join 不是原语，是绝对路径**——这正是"重新组织"的核心，人要按"舰 → 它的指令 → 这话谁说的"读：

```jsonc
"@post.factions.${name}.upkeep"                                     // 势力行 ⋈ 本回合过程量
"@control[?faction_id=${faction_id}].ship_orders[?ship=${name}].behavior"   // 舰 ⋈ 它的**有效**指令
"@post.haul_steps.${name}.step"                                     // 舰 ⋈ 此刻运输到哪一步
"@state.ships[?name=${@key}].faction_id"                            // 映射表的一行 ⋈ 那条记录
```

⚠ 实现期踩过的坑（两端各一次，见 §10.5）：**每一段必须"先取名字、再施加方括号"**——
`chronicle[*]` 要先取到 `chronicle` 再展开，`ship_orders[?ship=x]` 要先取到那个数组再挑。

### 4.3 原语清单

| 原语 | 作用 | 为什么必须有 |
| --- | --- | --- |
| `mount` | 挂在哪：`panel` / `inline` / `select` | §5，决定"通用"与"整理"如何共处 |
| `source` | 记录集合（路径；数组 / 映射表 / 单条记录三种形状） | 组织的输入 |
| `key` | 身份字段（`@key` = 映射键） | 选中联动、join、残差的行身份 |
| `columns[]` | `path` + `label`（中文名）+ `fmt`（格式化器）+ `missing`（取不到时说什么）+ `dot` / `bar` / `click` | 最基本的"排列"，也是"人类看不懂 `upkeep_unpaid`"的解药 |
| **`@根` 跨根路径** | 把别的根 / 别的集合的记录接进来 | 这才是"重新组织"：数据本来散在 `state`/`post`/`control` 三个根里 |
| `group` / `order` / `limit` | 分组（带色点）/ 行序 / 只留前 N（N **必须明说**） | 重要性排序；否则人要在 50 行里找 |
| `layout` | `table` / `sheet` / `cards` / `timeline` / `pairs` | 表看多数、sheet 看一条、cards 看几张、timeline 看事情、pairs 看映射 |
| `omit: [{path, why}]` | 显式不显示 + **理由必填** | 铁律 R 的唯一出口，且界面上必须说出来 |
| `use_at` / `use` | `inline` 的「路径 → 哪条视图」，以及视图之间的复用 | 同一份列定义服务面板与原始树，不抄第二遍 |

格式化器（`fmt`）一共 18 个，全是**通用**的：`text/int/num/pct/vec/bool/enum/count/list/map/sum/top/
pairs/progress/tagged/fields/rows/owner/ratio`。其中几个值得点名：

* `tagged`：serde 的单键对象（`{Haul:{from,to}}`）用**声明的模板**渲染（`"Haul": "跑运输 {from}→{to}"`）。
  没声明的变体 ⇒ **如实显示** `标签 + 载荷`，**不回落成"待命"那种假话**（`behaviorType` 以前正好会这么干）。
* `enum` + `label_from: "config.ships"`：舰级 / 资源 / 建筑的中文名**回落到 config 的 label**——
  引擎里已有的那份中文名，不再抄第二份。
* 事件类型的英文 key → 中文标签也写在声明里（`map`），并且**未声明的类型会显示成 `? 原名`**。

**失败语义**（照样要"响亮"）：

* 某列这帧取不到 ⇒ 显示声明的 `missing`（`—` / `空` / `没人说话` / `没在造`），**列不消失**
  ——"这局没有"与"没人摆它"必须长得不一样；
* `source` 整段落空 ⇒ 视图里明说「本回合没有成交。」这类话（`empty` 字段），不是空白；
* 声明自己写错（路径永远解析不了 / 引用了不存在的视图）⇒ §8 的 Rust 测试在 CI 里**红**；
* 这一帧某列**全帧取不到值** ⇒ 「未组织」页顶部的**运行时自检**明说（用真的求值器跑一遍）。

---

## 5. 挂载：重组点与通用兜底**在同一棵树里**共处

三种 `mount`，共用同一个求值器（都已落地）：

1. **`panel`（面板级重组）**：左栏「读面」的 5 个页（本回合 / 势力 / 舰队 / 城市 / 市场·运输）
   + 自动生成的「未组织」页。人类的主界面。
2. **`inline`（原位重组）**：右栏那棵**原始树**里，被认领的集合（`state.ships`、`state.cities`、
   `state.factions`、`state.events`、`post.market_trades`、`post.haul_steps`、
   `post.decisions.ships`、`post.decisions.blueprints`）**就地**换成整理后的表，顶部一行说明
   "此处由组织点「ship-table」整理"，每行仍带「其余 ▸N」；没被认领的部分照旧通用渲染。
   —— 这条是题面那句话的正面表达。
3. **`select`（选中读面）**：底部那条从「整份记录 dump」升级成「整理卡片 + 其余字段」；
   这个类型没有组织点就**明说**并退回通用 widget（老行为一条也不少）。

> 反过来说：**如果只做 `panel`**，"重组点以外的部分"就无从体现（面板与通用树彻底分家）。
> `inline` 是铁律 R 的最小可验证形态，也是这次扩展实验的观测点。

---

## 6. 第一批组织点（**引擎一行不用动**）

选它们不是审美，而是**逐条对着 §2.3 那张"数据全在、界面不说"的表**：

| # | 视图 | 数据来源（全部已存在） | 对人类的用处 |
| --- | --- | --- | --- |
| ① | **本回合**（页） | `state.chronicle`（正文）、`post`（霸权/交战对/被制裁/实力占比/总量）、`post.decisions.blueprints[*]`、`post.decisions.ships[*]`、`state.events[*]` | 推进之后的**反馈**：发生了什么、谁打了谁、AI 在干什么 |
| ② | **势力**（页） | `state.factions[*]` × `post.power_share.${name}` × `post.factions.${name}.*`（产出/维护/没付上的维护/治理覆盖/购买力/名次/运力缺口） | 一眼看清自己在哪个位置、钱花在哪 |
| ③ | **舰队**（页） | `state.ships[*]` × `@control[?…].ship_orders[?…]`（**有效**指令 + 叶的表态）× `post.haul_steps`、`post.decisions.ships` | 「我这些船在干什么、这句话是谁说的」 |
| ④ | **城市**（页） | `state.cities[*]` × `post.cities.${name}.*`（**忠诚三分项** / 在造进度 / 产出 / 劳动力 / 住房 / 枢纽） | 「这座城为什么在离心、为什么没在造」——原因就摆在结果旁边 |
| ⑤ | **选中对象卡片**（`select`×4） | 该实体记录 + 跨根 join（舰→指令→图；城→势力→前因；天体→定居点） | 从"读一份 dump"变成"读一张卡片" |
| ⑥ | **市场·运输**（页） | `post.market_trades`（逐笔含运费/丢货/关系倍率/MOND 附加）、`post.haul_steps`（每艘船此刻哪一步）、`post.market_price`、`state.market.offers` | 贸易/运输本来是 WIP 但已可玩，以前完全看不见 |

**六个之外的一切**（`config` 全表、`state.contracts`、`state.depots`、`notables`/`milestones`、
`scope`、`pre`…）照旧由通用 widget 处理，并且**自动出现在「未组织」页**上：
每个根 × 每个顶层键标「已整理 / 未组织」，被认领的集合还标出**每条记录里未被认领的字段**。

> 注意力路由（异常高亮：`revolt_risk`、被围、买不起、船卡住）**本轮不做**（D4）；
> 顶栏只加了一行**事实**摘要（"本回合 16 件事（接单 ×5 · 单结束 ×5 · 到货 ×4 · 单已交付）｜新故事：焚城之痛"）
> ——没有阈值、没有高亮、没有自动展开。

---

## 7. 顺带必修：通用兜底自己的三处静默

按铁律 R 的推论 3（不许静默截断），这些是**同一批活**里要修的，不能留到以后：

| 位置 | 现在 | 改成 |
| --- | --- | --- |
| `jsonview.js:31` `MAX_COLS=16` | 超出的列**直接消失**（实机已丢 `spawned_round`/`velocity`） | 表尾一列/一个按钮：「还有 N 列未显示（显示）」；或按展开状态分批出列 |
| `jsonview.js:32` `PREVIEW=72` | 折叠预览是 `.slice(0, PREVIEW)`，**不加省略号**（读源码核过：`preview()` `:83-88`），被截短看不出 | 截断处补 `…`，全文进 `title` |
| `jsonview.js:33` `MAX_DEPTH=14` | 超过即不产节点 | 到达上限的那一层显示「已达显示深度上限」 |
| `app.js:1946` `renderDiff` | 只比数组长度 | 至少把"本回合发生了 N 件事"接上视图 ①（本质是同一件事的弱化版） |

---

## 8. 测试与验收（前端没有测试设施，这条必须现在定）

前端是静态 JS，`cargo nextest` 摸不到它。建议：

1. **web crate 的 Rust 测试**（`web/src/tests.rs` 已有 `world()` 这套夹具）读 `web/static/views.json`，
   对**若干真实世界**（多 seed × 多回合，避免"这局恰好为空"）断言：
   * 每条 spec 的 `source` / `columns[].path` / `from.root` 都能解析（解析不了的**红**）；
   * `omit` 与 `columns` 不相交（自相矛盾的声明**红**）；
   * 打印一份**覆盖率报告**：认领字段数 / 残差字段数（**只打印不判红**——`config` 那种全表本来就不该被认领）。
2. **实机验收**（像 `web-blueprint-editor` 那次一样，记在实现篇里）：逐个视图点通 + 截图；
   专门做一次**"扩展实验"**：临时给 `Ship` 加一个字段（不碰前端），确认它**立刻**出现在
   §5 的残差里 —— 这是铁律 R 的直接验收。
3. **行为中性**：本轮**引擎一行不改** ⇒ 同 seed `--digest 20 --round 240` 必须逐字不变
   （实测两棵树同值，见 §10.4），并跑 `cargo nextest run -P full`。

---

## 9. 裁决（✅ 用户已确认 2026-10）

| # | 问题 | 裁决 |
| --- | --- | --- |
| **D1** | spec 住在哪 | **(a)** 前端数据文件 `web/static/views.json` + web crate 的 Rust 校验测试（引擎不碰）。用户原话：「就按你的推荐来，spec 在前端加校验测试」 |
| **D2** | 挂载方式 | **三种都做**（`panel` + `inline` + `select`），已落地 |
| **D3** | 第一批视图 | §6 的六个（落地为 5 个面板页 + 1 个自动生成的「未组织」页） |
| **D4** | 注意力路由 | **不做**（但顶栏加了一行"本回合 N 件事"的**事实**摘要——没有阈值、没有高亮） |
| **D5** | 写面 spec 化 | **分两步**：本轮只做读面组织层（控制树原样保留） |
| **D6** | 验收口径 | Rust 校验测试 + 扩展实验 + digest 中性 |
| **D7** | **列序 = 信息重要性的顺序**（用户追加） | ✅ 已落地：`columns` 数组顺序即优先级；编排原则见 §4.1；行序也按重要性（城市按忠诚升序、势力按实力占比降序） |
| **D8** | **字段顺序可以改 Rust 结构体定义**（用户追加） | **下一步再做**（用户原话：「struct 顺序字段我们下一步再改」）——本轮引擎一行未动，这条另开一步（要连带换 `--digest` 基线 + 世界中性的规范化证明） |

---

## 10. 实现记录（`feature/web-human-views`）

### 10.1 交付物

| 文件 | 是什么 |
| --- | --- |
| `web/static/views.json`（新，约 380 行） | **组织点声明**：5 个面板页（本回合 / 势力 / 舰队 / 城市 / 市场·运输）+ 4 个 `select` 卡片（舰/城/天体/势力）+ 1 张 `inline` 映射表（8 条路径 → 视图）。全是数据 |
| `web/static/specview.js`（新，约 700 行） | **通用求值器**：路径表达式 / join / 分组 / 行序 / 上限披露 / 残差 / 5 种布局 / 18 个格式化器。**一个领域词都没有**（与 `jsonview.js` 同一条铁律） |
| `web/static/jsonview.js` | 修 §7 的三处**静默上限**：`MAX_COLS` 静默丢列 → 表头 + 按钮明说「还有 N 列未显示（点开）」；`PREVIEW` 截断补 `…`；`MAX_DEPTH` 到顶说人话。新增 `inline` 钩子（调用方给 path→节点，widget 依然不认识领域字段）与 `rerender` |
| `web/static/app.js` | 左栏两个模式（**读面** / 控制）；读面渲染 + 「未组织」审计 + **运行时自检**；底部读面走 `select` 组织点；顶栏「本回合」一行；`jsonview` 的 `inline` 钩子接上（原始树里被认领的集合就地换成整理后的表） |
| `web/static/index.html` / `style.css` | 左栏 tab（读面/控制）+ 读面页 tab + 视图样式（约 90 行） |
| `web/src/views_tests.rs`（新） | **声明是数据 ⇒ 纪律要单独守**：静态校验（id 唯一 / 引用完整 / `omit` 与列不许重叠 / 路径合文法 / `@根` 已知）+ **对真实世界**（2 个种子 × 2 个回合数）检查每条相对列的首段真的存在 + 覆盖率报告 |

### 10.2 实机验收（round 20/40/60/100，本会话）

* **① 本回合**：故事（编年史正文 + 回合 + 参与方）、战局（霸权/交战对/被制裁/实力占比 top4 + 「还有 5 家未显示」/总人口/城/舰/舰队价值/本回合成交）、AI 在改设计图（新建/重估/回收 + 主题 + 在哪造）、AI 给舰下的判定（跑运输/机动/接战/轰炸 + 目标）、本回合事件明细（7 条，类型有中文标签）。
* **③ 舰队**：按势力分组（中国 12 条 / 欧盟 2 条 / 俄罗斯 4 条，带阵营色点），一行一舰：舰级 / 角色 / 船体（12/12 带比例条）/ **指令（有效）**「跑运输 地球→火星」/ **这话谁说的**「继承（上层说话）」/ 运输此刻 / 意志 / 目标 / 载货。跨根 join 全部由 `@control[?faction_id=…].ship_orders[?ship=…]` 这类路径表达式完成，前端没有为"舰指令"写一行代码。
* **④ 城市**：按势力分组、**按忠诚升序**（最危险的在前），忠诚 + 忠诚目标 + ↳距离项 + ↳娱乐项 + 在造（进度/速率）+ 产出 + 人口 + 天体。一眼能回答"这座城为什么离心"。
* **⑥ 市场·运输**：逐笔成交（买卖双方 + 货 + 距离 + 运费率 + 丢货率 + 关系倍率 + MOND 附加）、每艘货船此刻（地点/这一步/件数/货进哪里）、价格（按价格降序的 chips）、挂单簿（按量降序）。
* **`select` 卡片**：选中一艘舰 → 17 行中文标签的键值表（含"指令（有效）/ 这话谁说的 / 风格 / 姿态 / 运输此刻 / 载货 / 坐标 / 下水回合 / 组件"）+ 「其余字段 4 项」。
* **`inline`（原始树里）**：右栏 `state` 根展开 `ships` ⇒ 那一处**就地**换成整理后的表（顶部一行说明"此处由组织点「ship-table」整理"），每行仍带「其余 ▸12」。没被认领的集合（`depots`/`contracts`/`milestones`/`notables`/`config` 全表…）照旧由通用 widget 渲染。
* **「未组织」页**：`round`、`depots`、`notables`、`ship_name_seq`… 逐个标「未组织」并可就地展开原始 JSON；被认领的集合还标出**每条记录里未被认领的字段**（如 `cities ← city-table、sel-city · 按「city-table」每条记录里未被认领：buildings、settlement、ship_progress、space_station`）；`@control` 更细：`← faction-table 整理了 capital · 仍未组织：中国、俄罗斯、…`。
* **运行时自检**（同一页顶部，用**真的求值器**跑一遍）：`✓ ship-table 9 条记录 / 10 列全部取到了值`、`⚠ trades 这一帧没有记录（@post.market_trades[*]）`——引擎改字段名 ⇒ 这里立刻明说，而不是在表格里留一排「·」。

### 10.3 扩展实验（铁律 R 的直接验收）

临时给 `Ship` 加一个字段 `dsh_probe: f64`（`src/model/ship.rs` + 两处构造器），**前端一个字符都没改**，重编译后实机：

```
hasProbeInData: true
residualKeys: [attack_hist, blueprint, component_hp, components, doctrine,
               dsh_probe,            ← 新字段自己落进残差
               hull_max, kiting, position, shield, shield_max, spawned_round, velocity]
residualCount: 13          （实验前是 12）
```

`dsh_probe` 出现在「其余字段」里、带它的值（`dsh_probe 0`）。**实验后已把字段删回**（三处 `git checkout`），引擎逐字回到原状（见 10.4）。

### 10.4 验证

* **引擎一行未动**：`git diff --stat src/` 空；同 seed `--seed 42 --round 240 --digest 20`（取 `^{` 行、`\n` 连接、UTF-8 无 BOM、12 行）在两棵树上**实测相同**：
  `C928C3F19AFE3BA9D36A70DF8E340E3849271574663920D544AE62AFF70B06A9`
  （主工作树 `C:\resource\planet_x` = 本分支 = 同一个值）。
  ⚠ **顺带查实一条文档漂移**：`notes.md` 末尾记的基线 `81A19749…1811` **不是 `a06354f` 的值**——那是更早的换代点；本轮实测的 `C928C3F1…06A9` 才是当前 `main`（`a06354f`）的值。本轮没改引擎，所以这条漂移与本轮无关，但**别再拿旧值当基线**。
* `cargo nextest run -p planet_x_web`：**27 绿**（含 3 条新的 `views_tests`）。
* **全档**：`cargo nextest run -P full` = **230 通过 / 0 失败 / 34 跳过**（27.3 s）。
* 覆盖率报告（`cargo test … --nocapture`）：
  `state.ships，seed 7 / 40 回合：字段 18 个 = 认领 5 + 残差 13；残差 = attack_hist、blueprint、component_hp、components、doctrine、faction_id、hull_max、kiting、position、shield、shield_max、spawned_round、velocity`
  ——**残差永远非空**（那条断言本身也是一条守门人：全被认领说明有人把字段写死在别处了）。

### 10.5 实现期踩的坑（写给下一个 agent）

1. **路径段必须"先取名字、再施加方括号"**。第一版的 `step` 直接在父对象上展开/挑选，于是
   `chronicle[*]` 展开的是整个 `state`（`chron` 的键成了 `0..17`），整页"值全是 ·"。
   JS 修了之后，**Rust 侧那 60 行存在性检查又把同一个坑踩了一遍**——两处都加了注释。
2. `ctx.maps` 被 `bind({maps:{}})` **覆盖**过一次 ⇒ `tagged` 变体全退回原始 JSON
   （表格里出现 `Haul {"from":…}`）。改成合并。
3. 格式化器返回「没什么可显示」时，`String(null)` 会把字面量 `null` 印进单元格（实机看到
   「载货 null」）。现在：`''` = 空格子、`null` = 声明的 `missing` 文案。
4. `mount: select` 的 source 是**整个集合**，第一版照渲染 ⇒ 选中一艘舰却列出全部 21 艘。
   现在 `renderSelect` 按 `key` 挑出那一条，**挑不到就明说**。
5. app.js 的 `el(tag, attrs, html)` 第二参是**属性**不是 class：`el('span','sel-kind')` 一直
   只是设了个同名属性（`.sel-kind` 样式从来没生效过）。顺带修了（三处，选中读面的标签行）。
6. 分组表名与首列重复（"北辰 北辰"）：首列若就是 `key`，就把它当身份格，不再单开一列。

### 10.6 未做 / 下一步

* **D8：Rust 结构体字段按重要性重排**（用户已许可，明确放到下一步）：它换 `--digest` 基线，
  所以要连带一份「世界中性」的规范化证明（同 seed 的世界状态逐值相同、只是序列化字节序变了）。
* **D4：注意力路由**（异常高亮 / 推进后自动展开）——刻意不做，`flag` 原语留着口子。
* **D5：控制树（写面 1200 行）的 spec 化**——单独一步；本轮控制树原样保留。
* 读面还缺一条：引擎 `GameEvent::headline()`（唯一渲染器 + 守卫测试）**没有进 web 的读面**
  （`--index` 的 `events` 懒表才有 `headline` 列）。所以本轮的「事件明细」只有结构化字段 +
  声明式类型标签；若要让 web 也能说人话，最小改动是把 `EventRow` 那一路加进 `info` 根
  （引擎已有，属"派生读面"，不是手工投影）。**留给下一步裁决**。
* 字宽的取舍：左栏 430px，宽表（舰队 10 列）要横向滚。可考虑「读面面板可拖宽」。

