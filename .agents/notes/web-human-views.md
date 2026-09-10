# WebUI 面向人类的读面：**声明式组织点 + 通用兜底**（设计长文）

> 状态 `[ ]` 未实现（**等裁决**，见 §9） ｜ 索引：[notes.md](../notes.md) ｜ 分支 `feature/web-human-views`
> 工作树：`C:\resource\planet_x-webviews`（主工作树 `C:\resource\planet_x` 上只做了只读勘察 + 截图）

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

```jsonc
{
  "version": 1,
  "views": [
    {
      "id": "fleet",                       // 稳定 id（展开状态 / 深链 / 测试都按它引用）
      "title": "舰队",
      "mount": "panel",                    // panel | inline | select（见 §5）
      "source": "state.ships[*]",          // 记录集合：路径模式（[*] 数组展开、.* 映射展开）
      "key": "name",                       // 身份字段：选中联动 / join 的键
      "group": { "by": "faction_id",       // 分组（可选）；组标题的染色回落到 state.factions
                 "color_from": "state.factions[*].color" },
      "columns": [
        { "path": "name",  "label": "舰" },
        { "path": "class", "label": "舰级", "from": { "root": "config.ships", "by": "key", "as": "label" } },
        { "path": "hull",  "label": "船体", "fmt": "ratio", "with": "hull_max", "digits": 0 },
        { "path": "role",  "label": "角色", "fmt": "enum",
          "map": { "War": "战舰", "Freight": "运输舰", "Observe": "观测舰" } },
        { "label": "指令（有效）", "fmt": "behavior",
          "from": { "root": "state.control", "by": "faction_id", "then": "ship_orders[*]", "match": "name" },
          "path": "behavior" },
        { "label": "这话是谁说的",
          "from": { "root": "state.control", "by": "faction_id", "then": "ship_orders[*]", "match": "name" },
          "path": "order_source" },
        { "label": "此刻", "only_if": { "role": "Freight" },
          "from": { "root": "post.haul_steps", "by": "name" }, "path": "step" }
      ],
      "residual": { "title": "其余字段" }   // 默认开；渲染时算集合差（铁律 R）
    }
  ]
}
```

**原语清单**（就这九个，多一个都要在文档里给理由）：

| 原语 | 作用 | 为什么必须有 |
| --- | --- | --- |
| `mount` | 挂在哪（面板 / 树里原位 / 选中卡片） | §5，决定"通用"与"整理"如何共处 |
| `source` | 记录集合（路径模式） | 组织的输入 |
| `key` | 身份字段 | 选中联动、join、diff 对齐 |
| `columns[].path` | 取哪个字段 | 最基本的"排列" |
| `columns[].label/unit/fmt/map/digits` | 中文名、单位、格式化、枚举译名 | §2.4：人类看不懂 `upkeep_unpaid` |
| `from`（**join**） | **把别的根的记录按 key 接进来** | 这才是"重新组织"的核心：人是按"舰→它的指令→这话谁说的"读的，不是按根读的 |
| `group` / `order` / `limit` | 分组 / 排序 / 只留前 N（N 必须明说） | 重要性排序；否则人要在 50 行里找 |
| `layout` | `table` / `cards` / `sheet` / `timeline` | 四种够表达"人需要的组织"（表看多数、卡片看单条、sheet 看一条记录、timeline 看事情） |
| `omit` | 显式不显示 + 理由（界面上明说） | 铁律 R 的唯一出口，且必须可见 |

**求值器的失败语义**（照样要"响亮"）：

* 某列的路径这帧**不存在** ⇒ 单元格显示 `·`，**列不消失**（"这局没有"与"没人摆它"必须区分）；
* spec 引用的**根**整份不存在 ⇒ 该视图显示「这条视图的数据本帧没有（`post.haul_steps`）」，不是空白；
* spec 自己写错了（路径永远解析不了）⇒ 由 §8 的 Rust 测试在 CI 里红，而不是靠人肉发现。

---

## 5. 挂载：重组点与通用兜底**在同一棵树里**共处

三种 `mount`，共用同一个求值器：

1. **`panel`（面板级重组）**：把一批记录摆成一个新面板（如「舰队」「本回合」）。
   —— 人类的主界面。
2. **`inline`（原位重组）**：通用树里**某个路径的节点**被 spec 接管——渲染成整理后的卡片，
   卡片底部就是**残差**（"其余字段"）。
   —— 这条是题面那句话的正面表达：**同一棵通用树里，有组织点的地方是整理过的，
   没有的地方照旧由 widget 通用处理**；扩展落进残差，看得见。
3. **`select`（选中读面）**：底部那条从「整份记录 dump」升级成「整理卡片 + 其余字段」——
   即 `renderSelection()` 从"扔给 widget"改成"先问 spec 有没有接管这个 kind"。

> 反过来说：**如果只做 `panel`**，"重组点以外的部分"就无从体现（面板与通用树彻底分家，
> 人类只会在面板之间迷路）。所以我建议三种都做，而 `inline` 是最小的可验证形态。

---

## 6. 第一批组织点（六个；**引擎一行不用动**）

选它们不是审美，而是**逐条对着 §2.3 那张"数据全在、界面不说"的表**：

| # | 视图 | 数据来源（全部已存在） | 对人类的用处 |
| --- | --- | --- | --- |
| ① | **本回合发生了什么** | `state.events`（+ 懒表 `headline`/`salience`）、`post.decisions.*`、`post.market_trades`、`post.wars`、`post.haul_steps` | 推进之后的**反馈**：发生了什么、谁打了谁、AI 在干什么 |
| ② | **势力概况**（一行一势力） | 城/舰计数 + `post.view.factions[<f>].*`（产出 / 维护 / 治理覆盖 / 购买力 / 名次 / 战和）+ `post.power_share` | 一眼看清自己在哪个位置 |
| ③ | **舰队表** | `state.ships[*]` × `state.control[f].ship_orders[*]`（有效指令 + `order_source`）× 出厂图/风格 | 「我这些船在干什么、为什么不干」（`order_source` 已经在读面里，只是没人摆） |
| ④ | **城市表** | `state.cities[*]` × `post.cities[*]`（**忠诚三分项** / 离心风险 / 治理距离 / `build` 增量）/ × `post.view.factions[f]`（`upkeep_unpaid`/`investment_spent`） | 「这座城为什么在离心、为什么没在造」——原因就摆在结果旁边 |
| ⑤ | **选中对象卡片** | 该实体记录 + 跨根 join（舰→指令→图；城→势力→治理；天体→定居点→矿藏） | 从"读一份 dump"变成"读一张卡片" |
| ⑥ | **市场与运输** | `post.market_trades`（逐笔含运费/丢货/关系倍率）、`post.haul_steps`（每艘船此刻哪一步）、`post.market_settled`/`market_offered`/`market_price` | 贸易/运输本来是 WIP 但已可玩（`freight-collection.md`），现在完全看不见 |

每个视图的"其余字段"就是 §3.2 的残差；六个视图之外的一切（`config` 全表、`control` 写面、
`session`、`pre`）**照旧由通用 widget 处理**，并且会出现在「未组织」索引里。

> 注意力路由（异常高亮：`revolt_risk`、被围、买不起、船卡住）与"推进后自动展开什么"**先不做**，
> 但 `columns[].flag` 这个原语先留好接口（§9 D4）。

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
   （基线 `81A19749…1811`，`notes.md` 末尾），并跑 `cargo nextest run -P full`。

---

## 9. 需要裁决的点（**请逐条拍**）

| # | 问题 | 我的建议 | 备选 |
| --- | --- | --- | --- |
| **D1** | **spec 住在哪？** | **(a) 前端数据文件** `web/static/views.json` + web crate 的 Rust 测试校验（引擎一行不动，符合"引擎 = 数据平面"与"纯前端改动当天可验"的先例） | (b) 夹进引擎 schema（`x-ui`，`schema-query-architecture.md` §9.3）——单一来源、agent 也能读，但引擎要长出 UI 概念；(c) 引擎直接算好视图 = **不建议**，那是被本仓杀过的"手工投影" |
| **D2** | **挂载方式** | **三种都做**（`panel` + `inline` + `select`）；`inline` 是铁律 R 的最小可验证形态 | 只做 `panel`（更快，但题面那句"重组点以外"就落不了地） |
| **D3** | **第一批做哪几个视图** | §6 的六个（①③④ 优先，它们是每回合都要看的） | 你来挑（比如先只做①+③） |
| **D4** | **注意力路由**（异常高亮 / 推进后自动展开摘要） | **先不做**，但留 `flag` 接口 | 一起做（人味更足，但会掺进"什么算异常"的平衡判断） |
| **D5** | **写面（控制树 1200 行）要不要一起 spec 化？** | **分两步**：本轮只做读面组织层 + 让 `inline` 也能挂在控制树上做**只读装饰**（显示有效值/来源/"当前跟随"）；控制树的重构单独立项 | 一起做（风险：控制树涉及三态归属/删叶/只回传差异这些**语义**，不是纯展示） |
| **D6** | **验收口径** | §8 那三条（Rust 校验测试 + 一次"加字段"扩展实验 + digest 中性） | 加一条：**覆盖率阈值**（我不建议——`config` 全表不该被认领） |

---

## 10. 若裁决通过，实现顺序（草案）

| 步 | 做什么 | 验收 |
| --- | --- | --- |
| M1 | `specview.js` 求值器（九原语）+ 铁律 R 的残差机制 + `view.json` 骨架 + 修 §7 三处静默 | Rust 校验测试绿；实机点通 `inline` 一条；同 seed `--digest` 逐字不变 |
| M2 | 六个视图（§6），先 ①③④ | 每回合能在 3 秒内回答"发生了什么 / 我的船在干什么 / 这座城为什么离心" |
| M3 | 「未组织」索引 + 覆盖率报告上界面（一个 dev 角标）+ 扩展实验写进实现篇 | 临时加一个引擎字段 ⇒ 残差里立刻出现 |
| M4（可选） | 注意力路由（D4）；写面 spec 化（D5） | 另开一篇 |

---

## 附：本轮改到的文件

* 本篇（新）+ `notes.md` 索引一行。
* 只读勘察与截图：主工作树 `scratch/shots/*.png`（未跟踪，未提交）。
* **代码一行未动**（等 §9 裁决）。
