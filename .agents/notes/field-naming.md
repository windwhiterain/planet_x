# 字段顺序 + **序列化中文名**（设计待裁决）

> 用户原话（2026-10）：
> 「我们先调整 rust 字段顺序，同时给字段加序列化中文名字（避免用单独的翻译表），名字参考 spec，
> 把不准的名词问我。**所有 UI 都用名词，鼠标移上去弹窗显示注释/解释**」
>
> **状态：`[x]` 已落地**（`[ ]` 那行是本文写下时的状态，保留作记录）。批 A→D 的改名已进 main，
> 本刀（第 5 步，`feature/kind-nouns`）把最后一块拼上：`--index` 的 `control` 表 `kind` 词表
> 也统一到中文叶名、并且**从 `LEAVES` 一处派生**。落地记录见 §7；**哪些字符串有意保持英文**见 §8。

## 0. 一句话

把「界面上该叫什么」这件事**从两份手抄表（engine 英文键 + `views.json` 的 `label`）收回引擎**：
`#[serde(rename = "中文名")]` 是**唯一的翻译处**，`///` 文档注释经 `schemars` 变成
`description`，就是**鼠标悬停的弹窗**。于是 UI 直接用键名当名词、用 description 当解释，
**没有任何第三种东西**（不查表、不映射）。

## 1. 机制（三条链）

| 想要的 | 机制 | 现状 |
| --- | --- | --- |
| UI 显示名词 | `#[serde(rename = "忠诚度")] pub loyalty: f64` ⇒ dump / `--index` / `--control-schema` / web 的 JSON 键**都是中文** | 现在是英文键 + `views.json` 里再写一遍 `label` |
| 悬停弹注释 | `schemars` 把 `///` 收进 `description` ⇒ 前端 hover 弹 | **已可用**：`--schema` 实测已发 **295** 条 description；web 只要加一个 `GET /api/schema`（与 §11 那个 `/api/control-schema` 同一路数） |
| 字段顺序 = 读面列序 | 结构体字段顺序就是 JSON 键序（`json::to_value` 保序）⇒ 读面默认列序 = 声明顺序 | `views.json` 的列序是手写的；重排后**默认就对**，`views.json` 只在「要覆盖」时写 |

**因此这一轮的验收有一条硬判据**：改完之后，`views.json` 里的 `label` 与
`leaf_ui.*.label` **应当只剩「需要覆盖引擎名」的那几条**（理想是 0 条），
而 `web/static/` 里**不该再有任何英文领域词**。这一条可以写进 `g4_spec.py`。

## 2. 现状盘点（实测）

* `src/model/*.rs`：**80 个 `pub struct`、627 个 `pub` 字段**；另有 `src/projection.rs`
  （派生读面的表）与 `src/agent.rs`（agent 视图）——三处都要改。
* `///` 文档覆盖：**536/627 = 85%**（缺的 91 个要补，否则那 91 个悬停是空的）。
* `--schema` 已发 295 条 `description`（说明机制不用新建）。
* 消费者（全都要跟着改，否则就是"失败看起来像成功"）：
  * `play/planet_xq`（投影 reader）、`play/planet_x_ctl`（kit 的配方 API 与 patch 键）、
    `play/tests/g1–g3`（断言里到处是字段名）；
  * `web/static/views.json`（**每条路径都是字段名**）、`app.js`/`controls.js` 里的领域词；
  * `.agents/*.md`（手册、spec 例子、笔记里的示例 JSON）。
* **digest 会变**（故事板的 JSON 键变了）⇒ 必须重测基线并记录，并附一份
  「**只有键名/键序变了、值逐值相同**」的规范化证明（映射回旧名后逐字节相同）。

## 3. 分批（每批一次「改 → 三端跟 → 两层门 → digest 换基线」）

| 批 | 范围 | 为什么这个顺序 |
| --- | --- | --- |
| **A（建议本轮）** | 核心实体 + 控制叶：`Ship` / `City` / `Body` / `Settlement` / `Orbit` / `Faction` / `Ideology` / `Building` / `Blueprint` / `Contract` / `Control` / `State` + `ControllableState` 的 14 片叶 | UI 读得最多、spec 覆盖最全；先把**规则**定下来 |
| B | `events` / 编年史 / 决策（`EventRow`/`StoryEvent`/`ChronicleEntry`/`ShipDecision`/…） | spec 基本没有，得靠问 |
| C | 派生读面（`projection.rs`：`RoundView`/`FactionRow`/`CityRow`/`MarketTrade`/`GovernanceFlow`/…） | 名字多是引擎造的复合词（"治理覆盖""没付上的维护"），要一起对齐 |
| D | 配置 `config/game.ron`（`ShipSpec`/`ComponentSpec`/`EconomyConfig`/…）+ **改配置文件本身的键** | 最大且最痛（500 行 `.json` 重写）；也可以选择**只改 dump 出去的名字、不动文件键** |

## 4. 样例名词表（批 A，❓ = 我不确定的，按用户要求列出来问）

**规则**（我提议，待确认）：用 spec 的原词；成对上下限用「X / X上限」；引用用实体名本身
（引擎里势力/城/舰的键就是名字，没有 numeric id）；复合派生态用一句话而不是缩写。

### 4.1 舰船 `Ship`

| 现在 | 我提议 | 依据 |
| --- | --- | --- |
| `name` | 舰名 | spec「舰船」 |
| `class` | 舰级 | spec 配置·舰船（护卫舰/驱逐舰/…） |
| `faction_id` | 势力 ❓ | 它就是势力名 |
| `position` | 坐标 | spec 导出属性「当前位置」⇒ 我这用了短一点的「坐标」❓ |
| `hull` / `hull_max` | 船体 / 船体上限 | spec「船体」 |
| `shield` / `shield_max` | 护盾 / 护盾上限 | spec「护盾：伤害吸收池」 |
| `components` | 组件 | spec「组件」 |
| `component_hp` | 组件耐久 ❓ | spec 只说「组件」 |
| `velocity` | 速度 | spec「速度上限」的当下值 |
| `doctrine` | 行为风格 ❓ | spec「行为风格：自动控制行为」；界面现在只叫「风格」 |
| `kiting` | 风筝姿态 ❓ | spec「警惕<->激进：风筝<->贴脸」——字段名取哪个？ |
| `role` | 角色 ❓ | 值是 战舰/运输舰/观测舰（既有 UI）；spec 只写「战斗/运输」 |
| `attack_hist` | 攻击历史 | spec「注意力（根据攻击历史）」 |
| `cargo` | 载货 | spec「货舱容量」 |
| `blueprint` | 出厂图 ❓ | spec 无「设计图」这个词，是我们自己起的 |
| `spawned_round` | 下水回合 | — |

### 4.2 城市 `City` / 天体 `Body` / 定居点 `Settlement` / 轨道 `Orbit`

| 现在 | 我提议 | 依据 |
| --- | --- | --- |
| `City.name` | 城名 ❓（还是「城市」？） | spec「城市」 |
| `City.body_id` | 所在天体 | — |
| `City.settlement` | 定居点 | spec「定居点：可以容纳一个城市」 |
| `City.faction_id` | 势力 | — |
| `City.population` | 人口 | spec「人口：限制生产效率」 |
| `City.buildings` | 建筑 | spec「建筑：任意数量」 |
| `City.ship_progress` | 造舰进度 ❓ | — |
| `City.razed` | 已焚毁 | 事件里有「城被夷平」 |
| `City.space_station` | 空间站 | — |
| `City.loyalty` | 忠诚度 | spec 原词（**不是**「忠诚」） |
| `Body.name` | 天体名 ❓ | spec「天体」 |
| `Body.orbit` / `position` / `kind` / `ring` / `settlements` | 轨道 / 位置 / 类型 / 星环 / 定居点 | spec |
| `Settlement.total_area` | 总面积 | spec 原词 |
| `Settlement.ecological_capacity` | 生态容量 | spec 原词 |
| `Settlement.construction_speed_mod` | 建设速度修正 | spec 原词 |
| `Settlement.construction_resource_mod` | 建设资源修正 | spec 原词 |
| `Settlement.resources` | 资源 | spec「资源：类型 / 面积」 |
| `Orbit.perihelion_distance` | 近日点距离 | spec 原词 |
| `Orbit.aphelion_distance` | 远日点距离 | spec 原词 |
| `Orbit.aphelion_direction` | 远日点方向 | spec 原词 |
| `Orbit.period` | 公转周期 | spec 原词 |
| `Orbit.parent` | 母星 ❓ | spec 没有（只有「轨道（2D，圆心为太阳）」） |

### 4.3 势力 `Faction` / 思潮 `Ideology`

| 现在 | 我提议 | 依据 |
| --- | --- | --- |
| `name` / `symbol` / `color` | 势力 / 符号 / 颜色 | — |
| `resources` | 资源（首都库存）❓ | 现在界面写「首都库存」——它是**首都的**资源池 |
| `relations` / `reputation` | 关系 / 名声 | — |
| `alignment` | 阵营倾向 ❓ | spec 无 |
| `aggression` / `home_radius` / `home_attack_mult` / `home_regen_bonus` | 侵略性 / 本土半径 / 本土攻击倍率 / 本土再生加成 ❓ | spec 无（本土防御那套是后来加的） |
| `ideology` | 思潮 | spec「各类思潮偏向」 |
| `Ideology.peace_military` | 和平↔军国 | spec 原词 |
| `Ideology.science_tech` | 科学↔技术 | spec 原词 |
| `Ideology.people_elite` | 人民↔精英 | spec 原词 |
| `Ideology.nature_colony` | 自然↔殖民 | spec 原词 |
| `mond_control` | MOND 掌握度 | 笔记里的既有用词 |

### 4.4 建筑 `Building` / 设计图 `Blueprint` / 合同 `Contract`

| 现在 | 我提议 | 依据 |
| --- | --- | --- |
| `Building.id` | 建筑编号 ❓ | 城内的 u32 下标（只在城内部唯一） |
| `Building.kind` | 类型（居住区/开采区/建造区） | spec 原词 |
| `Building.resource` | 开采资源 ❓ | spec 开采区「资源类型」 |
| `Building.ship_type` | 建造舰级 ❓ | spec 建造区「建造舰船类型」 |
| `Building.blueprint` | 挂的设计图 ❓ | spec 无 |
| `Building.structure` | 结构（混凝土/钢结构） | spec「建筑·混凝土/钢结构」 |
| `Building.area` / `armor` | 面积 / 护甲 | spec 原词 |
| `Building.deployed` | 已启用 ❓ | spec 无 |
| `Blueprint.class` / `components` | 舰级 / 选装 ❓ | spec 无「设计图」章；「槽位：装配的组件数上限」里有"装配" |
| `Blueprint.doctrine` / `kiting` / `role` | 同 4.1 | — |
| `Contract.id` | 合同号 ❓ | — |
| `Contract.shipper` / `carrier` | 货主 / 承运方 ❓ | 既有 UI 用「承运」 |
| `Contract.resource` / `capacity` | 货 / 运力 ❓ | — |
| `Contract.from` / `to` | 起点 / 终点 ❓ | （`from`/`to` 是 Rust 关键字避让，中文名没这问题） |
| `Contract.share` / `min_reputation` | **抽成**（第 10b 步订正，原提议「分成」）/ 最低名声 ❓ | — |
| `Contract.posted_round` / `accepted_round` / `expires_round` / `review_round` | 挂单回合 / 接单回合 / 到期回合 / 考核回合 | 既有 UI 用词 |
| `Contract.delivered` / `served_rounds` | 已交付 / 已服务回合 ❓ | — |

### 4.5 控制叶（14 + 1）

| 现在 | 我提议 | 依据 |
| --- | --- | --- |
| `capital` | 首都 | spec 控制属性「首都」 |
| `ship_orders` | 指令 ❓ | spec 叫「行为」；我们上一轮定「指令 = 即时操作」 |
| `ship_doctrine` / `default_doctrine` | 风格 / 舰队默认风格 ❓ | spec「行为风格」 |
| `ship_kiting` / `default_kiting` | 风筝姿态 / 舰队默认风筝姿态 ❓ | spec「警惕<->激进：风筝<->贴脸」 |
| `ship_role` / `default_role` | 角色 / 舰队默认角色 ❓ | 值 = 战舰/运输舰/观测舰 |
| `investment_budget` | 开发预算 ❓ | spec 控制属性「各类资源开发预算」（现在界面叫「投资预算」） |
| `construction_budget` | 建造预算 | spec 原词 |
| `loyalty_budget` | 福利预算 ❓ | spec 城市控制属性「福利权重」；现在界面叫「娱乐/福利预算」 |
| `invest_weights` | 开发权重 ❓ | spec 建筑控制属性「开发权重」（现在叫「建设权重」） |
| `build_weights` | 建造权重 | spec 原词 |
| `blueprints` | 设计图库 | — |
| `buildings`（命令，不是叶） | 建筑 | spec「建筑」 |

## 5. 要你拍的（我按这些问题动手，其余按同一规则自动生成）

1. **总口径**：名词用 spec 原词（「忠诚度」「总面积」「近日点距离」）还是更短（「忠诚」「面积」）？
   我倾向 **spec 原词**——它已经写在那儿了，短名反而要另一张对照表。
2. **引用字段**（`faction_id`/`body_id`/`ship_type`…）：用实体名本身（「势力」「所在天体」「建造舰级」），
   还是保留 `_id` 感（「势力名」）？我倾向**实体名本身**（引擎里键就是名字，无 numeric id）。
3. **上下限成对**：`hull`/`hull_max` → 「船体」/「船体上限」？
4. **`role` 的三值**：战舰/运输舰/观测舰（既有 UI）还是 spec 的 战斗/运输（观测舰是后加的，spec 没有）？
5. **`doctrine` / `kiting` 的字段名**：叫「行为风格」/「风筝姿态」，还是就「风格」/「姿态」？
6. **`investment_budget`**：跟 spec 叫「开发预算」，还是保留既有 UI 的「投资预算」？
7. **`loyalty_budget`**：spec 的城市控制属性写的是「福利权重」，引擎这片叶实际是**按城拨的预算** ⇒
   叫「福利预算」？还是连 spec 一起改成「福利预算」？
8. **`invest_weights`**：spec 叫「开发权重」，既有 UI 叫「建设权重」——跟哪个？
9. **`ship_orders`**：spec 叫「行为」，我们上一轮裁定「指令 = 即时操作、倾向 = 长期」⇒ 叫「指令」还是「行为」？
10. **批 D（配置）**：`config/game.ron` 的**文件键**要不要也中文化（会重写 500 行配置 + `--meta` + 文档）？
    还是只让 dump 出去的名字是中文、文件键保持英文？（我倾向**也改**，否则 UI 的配置页还是英文键）
11. **字段顺序的规则**：按 `web-human-views.md` §4.1 的重要性序（身份 → 此刻结论 → 原因/来源 →
    量与资源 → 引用/位置 → 内部元数据），并把它当成**读面默认列序**——同意吗？
12. **不接受向前兼容**：`SCHEMA_VERSION` → 23、旧 `.json` 存档直接读不了、digest 换基线（附映射回旧名的
    逐值等价证明）——确认？

## 6. 风险

1. **627 个字段 + 三端消费者**：这是本仓最大的一次改名。分批是为了每批都能过两层门；
   若一次全改，中间任何一处漏改都会以「某个字段静默变成 null」的形式出现。
2. **`from`/`to`/`type`/`kind` 这类通用词**在中文名里会撞车（多个结构体都有「类型」）：JSON 里没问题
   （各自在自己的对象里），但**读面的路径拼接**要小心，`views.json` 的路径得逐条核对。
3. **文档注释就是产品文案**了：85% 有注释，缺的 91 个要补，而且既有注释里有不少是写给实现者的
   （"⚠ `null` 是第四种情况"），悬停弹窗里要能读懂 ⇒ 可能要**把注释分成「给实现的」与「给玩家的」**。

## 7. 落地记录

### 7.1 裁决（2026-10）

* 除「权重/预算那一块」外**全部按推荐**：spec 原词、引用用实体名、成对上下限用「X / X上限」、
  `role` 三值 = 战舰/运输舰/观测舰、`doctrine`/`kiting` 叫「风格/姿态」、`ship_orders` 叫「指令」、
  配置 `game.ron` 的键也一起中文化（批 D）、字段顺序按重要性并当作读面默认列序、
  `SCHEMA_VERSION` 23 + 换 digest 基线。
* **权重/预算那 5 片叶暂缓**（用户「先不管权重这一块，我在仔细考虑考虑」）：
  `investment_budget` / `construction_budget` / `loyalty_budget` / `invest_weights` / `build_weights`
  的**名字与语义**都还没定，先不动。

### 7.2 第 1 步（已落地）：字段顺序 + 存档改 JSON

**① 存档格式 RON → JSON**（前提，用户已批「可以接受不用 ron」）

* 为什么必须换：**RON 要求结构体字段名是合法标识符**。实测 `ron 0.8` 连 `天体名` 都不收
  （`ERR_SAVE: Invalid identifier`），`ron 0.10` 收纯中文但**连未改名的模型都读不回来**
  （`missing field round in State`，与改名无关）⇒ 这条路上没有出路。
* 改法：`json::to_string_pretty` / `json::from_str`（`src/json.rs`），`save_state` /
  `save_checkpoint` / `load_checkpoint` / `load_state` 四个入口换掉。
  `to_value` 早就把**元组键**写成 `"城|7"`，反方向由 `#[serde(deserialize_with = "…::de_keys_ss/de_keys_su")]`
  在这三处还原（`State.depots` / `invest_weights` / `build_weights`）。
* **忠实性实测**：`--seed 7 --round 10 --save X` 与「跑 3 → 存档 → 再跑 7 → 存 Y」的
  X/Y **逐字节相同** ✓（这是「分段讲故事」的全部基础）。
* 仍留在 RON 的只有 `config/game.ron`（手写配置，批 D 再议）。

**② 实体字段按重要性重排**（11 个结构体、约 90 个字段）

顺序规则：**身份 → 此刻结论 → 倾向与意图 → 量与资源 → 引用与位置 → 内部与溯源**。
实测（`--seed 7 --round 1`）：`ships[0]` 键序 = `name, class, faction_id, hull, hull_max, shield,
shield_max, velocity, doctrine, kiting, role, components, component_hp, cargo, position, blueprint,
attack_hist, spawned_round` ✓ 顺序真的出现在读面上（`main` 已开 `serde_json/preserve_order`）。

**③ 行为中性**：键序变了 ⇒ **digest 换基线**

* 新基线：`748B4AA66FE169E8F316169D61CB5239057D0F3515CF59A50C629E76BF489603`
  （旧 `C928C3F19AFE3BA9D36A70DF8E340E3849271574663920D544AE62AFF70B06A9`）。
* **等价性证明**：把两边的 digest 行递归按键排序归一后再哈希，**完全相同**
  （`AA8240A5F16964E8DB20BBB87BFB…`）⇒ 只有键序不同、**值逐值相同**。

**④ 两层门**：Rust `213/213`（31 skipped）+ Python 四组 `111/111`。

### 7.3 踩过的坑（写下来，别再踩）

1. **`#[serde(rename)]` 撤回脚本删过头**：它按正则删掉 `src/model/**` 里所有 `rename`，
   把仓库里**原有的** `#[serde(rename = "type")]`（`EventRow.kind`）也删了 ⇒ 事件表少一列、
   两条投影守卫红。**教训：批量脚本只该删"自己加的"那些**（用白名单/上下文锚定）。
2. **`preserve_order` 已由 main 打开**（另一个会话加过）：我曾"为了排查"把它关掉，那实际上
   是**相对 main 的功能倒退**；现在恢复成与 main 一致。
3. **依赖键序的测试**：v9 档那条测试原来用「逐行删键」造旧档，`preserve_order` 一开后
   `spawned_round` 排在对象末尾 ⇒ 删完留一个悬空逗号（`trailing comma`）。已改成**结构化**
   （解析成 `Value` → 递归删键 → 再序列化），与键序无关。

### 7.4 下一步

* **第 2 步**：加 `#[serde(rename = "中文名")]`（现在 RON 不再是障碍）+ 三端消费者跟随。
  名词表见本文 §4（`❓` 已按 §7.1 定名）。
* **第 3 步**：`--schema` 的 `description` 接到 web（`GET /api/schema`）+ 悬停弹窗，
  `views.json` 的 `label` 逐条退掉。
* 权重/预算那 5 片叶：等用户裁决。

### 7.5 第 2 步（批 A 实体切片）：中文名落地

**做了**：11 个实体结构体、**83 个字段**加 `#[serde(rename = "中文名")]`
（`grep -n 'serde(rename' src/model/*.rs` 是唯一来源）。实测读面：
* 舰船 → `舰名, 舰级, 势力, 船体, 船体上限, 护盾, 护盾上限, 速度, 风格, 姿态, 角色, 组件, 组件耐久, 载货, 坐标, 出厂图, 攻击历史, 下水回合`
* 城市 → `城名, 所在天体, 定居点, 势力, 人口, 忠诚度, 已焚毁, 轨道空间站, 建筑, 造舰进度`
* 天体 → `天体名, 类型, 星环, 轨道, 位置, 定居点`；势力 → `势力, 符号, 颜色, 阵营倾向, 好战度, 思潮, 名声, MOND 掌握度, 资源, 关系, 本土半径, 本土攻击倍率, 本土再生加成`
* 思潮四轴 → `和平↔军国 / 科学↔技术 / 人民↔精英 / 自然↔殖民`（**带 ↔ 的名字现在合法了**——存档是 JSON）

**digest 不变**：改名前后都是 `748B4AA66FE169E8F316169D61CB5239057D0F3515CF59A50C629E76BF489603`
——digest 行里**不带字段名**（它压的是"故事值"），所以这一批改名**连 digest 基线都不用换**；
另外把新行的键逐条映回旧名 + 排序归一后与 main 逐字节相同（`AA8240A5F16964E8DB20BBB87BFBE496…`）。

**跟随改动**：`src/projection.rs` 7 张实体表 + `projection_schema()` 的 `columns`/`column_docs`
（157 键）、`src/agent.rs` 的三轴有效值注入键、`src/tests/**` 的读面断言、
`web/static/**`（views.json 路径 + JS 引用）、`play/**`（三个包 + 四组测试）。
**已验证「值没变」**：把新 digest 行的键逐条映回旧名 + 按键排序归一后，与 main **逐字节相同**
（`AA8240A5F16964E8DB20BBB87BFBE496…`，见 `scratch/prove_values_unchanged.py`）。

**`agent.rs` 抓到一个"失败看起来像成功"的坑**：`state_json` 在序列化**之后**用键名就地覆盖
三轴的有效值（`row["doctrine"] = 有效值`）。模型改名后这些赋值会在**中文键旁边另加三个英文键**
（中文键留记录值、英文键拿有效值）——界面上看着有值，实际是两份不同的东西。已跟着改名。

**存档写侧收紧**：`save_state` / `save_checkpoint` 现在**只写 JSON**，路径不是 `.json` 当场拒
（`存档只写 JSON：请把路径写成 …json`）——按扩展名选格式的话，一个 `.ron` 路径要么写失败、
要么写出一份**读不回来**的档。读侧仍按扩展名（老 .ron 档还能读）。

**这一步明确"留到下一批"的读面残留**（都是有意的，不是漏）：

| 残留 | 为什么先不动 |
| --- | --- |
| 连接键 `ship_id`/`city_id`/`faction_id`/`body_id`/`blueprint_id`… | 它们同时是 Python 全套 `join_on` 的连接键**与控制面 patch 的 `faction_id`**，要和权重那一批一起收口 |
| 建筑地址 `id`（城行内联 `buildings[].id`） | 与控制面 `invest_weights` 的键 `城\|下标` 是同一套编号，改了会断交叉引用 |
| 派生表的判别式 `kind`、事件表 `type` | 不是实体字段（`Body.kind`/`Building.kind` 已改「类型」） |
| decisions 表的 detail（`kiting`/`from`/`to`/`components`…） | 决策结构体不在这一批（批 B/C） |
| 事件载荷键（`loyalty`/`shipper`/`class`… 约 20 处 `json!`） | 事件词汇（批 B） |
| `src/control/leaves.rs` 的 `keys`/`values`/`carries` | 控制面，与权重一起（且权重还没裁决） |
| `agent.rs::meta_value` 的配置段键 | 配置（批 D） |
| ships 行的 `x`/`y`（`position` 的摊平） | 派生量（批 C） |
| `column_docs` 正文里的 `Ship.doctrine` 之类 | 那是 **Rust 标识符**，本来就该这么写 ✓ |

### 7.6 第 3a 步：悬停弹窗（注释即解释）

用户原话：「所有 UI 都用名词，**鼠标移上去弹窗显示注释/解释**」。落地成三件事：

**① 引擎：名词与解释是一份数据，两个出口**

* `planet_x --nouns` 与 web 的 `GET /api/schema` 是**同一个实现**
  （`agent::noun_schema_value()`），发 **四半**：
  `state`（实体字段的 `///`）、`view`（回合视图字段的 `///`）、
  `projection`（每张表的 `columns`/`column_docs`，含 `derived.*`）、
  `control`（叶/命令/作用域键）。
  四半各自覆盖不同的名词，合起来 = **界面上能出现的所有名词**。
* 为此给 `State` 及其内嵌类型补了 `schemars::JsonSchema`（`State`/`ControllableState`/
  `ControlScope`/`MarketState`/`Milestones`/`Notables`/`Roll`… 一串），元组键字段
  （`depots`/`invest_weights`/`build_weights`）另挂 `#[schemars(with = "BTreeMap<String, _>")]`
  ——schemars 不认 `#[serde(with = "模块")]`。
* **补的文档**：实体表缺的列解释（`舰名`/`城名`/`势力`/`所在天体`/`天体名`/`faction_id`、
  `decisions.ship`）、`ControlScope(/Patch)` 那四个作用域键的 `///`。

**② 前端：`web/static/tip.js`——它不认识任何领域词**

* 启动时 `GET /api/schema` 拉一次，建「名词 → 解释」表；此后 `Tip.attach(node, 名词, 字段名)`
  纯查表。查词顺序：**显示的那个词 → 字段名 → 控制面字段表**（叶的键名现在还是英文）。
* 挂点**只有名词**：表头（含身份列 `sv-th-key`）、控制行标签、卡片里的字段名。
  **不给每个单元格挂**（太吵）。
* 弹窗是**自定义 div**（不是原生 `title`）：~250ms 延迟、跟手、`max-width: 420px` /
  `max-height: 46vh` 可滚、`pointer-events: none`（鼠标穿过它，否则贴着鼠标的框会自己
  把自己 mouseleave 掉而闪烁）、离开/滚动/`Esc`/`blur` 即收、触摸不弹。
* **已有原生 `title` 的节点不挂**（归属那类 `<select>` 自带长解释，叠两层会两个框一起冒）。
* 求值器仍然不认识领域词：它只把「这条列声明」交给宿主（`ctx.tip`），由 `app.js` 决定名词与兜底字段。

**③ 判据（g4，静态、带防空转）**

* 名词覆盖率：**当名词显示的列**（裸字段列 + 控制行）必须能在 `--nouns` 里查到词条；
  实测 **105 个界面名词全命中**（语料 310 个），下限 40 防空转。
* `label` 不许与引擎键名**逐字重复**（"该退的都退了"）；`label` 只在要换词/加格式时才写，
  那时它同时是弹窗的查词键。
* 反向验证：新增 2 个注入错（裸字段列改成不认识的名词、控制行键改成不存在的键），
  `_g4_negative.py` 共 **21 个注入错全部咬住**。

**已知缺口（下一步）**：**表达式列**的表头（如 `@post.power_share.${势力}` 的「实力占比」）
不在这条判据的口径里——它的表头是**标题**不是引擎名词，所以不弹。两条路：
(a) 给这类列加一个声明字段 `noun: "power_share"`（一句"这一列说的是哪个名词"）；
(b) 让前端从表达式里挑第一个在语料里的裸段（`power_share` ✓，但 `@state.ships[…].势力`
会挑到 `ships` ⇒ **会挑错**）。倾向 (a)，但那是声明语言的新字段，等裁决。

**另一条待办**：少数实体字段的 `///` 还是**英文**（`Ship.hull` = "Current hull (armor) …"、
`Faction.资源` = "Stockpiled resources …"）——弹窗照实显示。要中文解释就得把那批 `///`
中文化（属于"用词统一"那一趟）。

### 7.7 digest 变了：是**世界分岔**，不是改名（含一条旧声明更正）

**观测**（口径：取 `^{` 行、去 `
`、按 `
` 拼接、sha256、大写）：
* `main`（`2f6cc7a`）= `748B4AA66FE169E8F316169D61CB5239057D0F3515CF59A50C629E76BF489603`
* 本分支（`feature/control-nouns`，第 4 步改名后）= `3CA2A8019BF81879824D4C6B50D31FDC027CDB9B2FB7EE6C2929196F4400D598`

**为什么变：世界分岔，与改名无关。证据（都不是推测，是实测）**

1. **digest 里没有我们改过的任何一个键**。它是专门的摘要结构（词表 60 个键：`from/to/rounds/world/factions/power_share/hegemon/…/events/top_events/story`）。
   实测 `舰名`/`城名`/`势力`/`投资预算`/`开发货币预算`/`归属`/`值`/`建筑` 在两边 digest 里**全部 = False**（一次都没出现）。
   ⇒ 改名**结构上不可能**影响 digest。
2. **全局唯一的"只在一侧出现"的键是 `coalition_ended`**，且所有单侧键都落在 `/events/` 下 —— 那是**事件类型直方图的桶**，
   意思是"这一局没发生过这个事件"，不是"键被改名"（`attack`/`siege`/`city_razed`/`colony_founded`/`story` 这些事件名两边都是英文、都没改）。
3. 差异**全在值上**：`world/ships` 20 → 16、`world/fleet_value` 492.0 → 358.82、`world/population` 11237 → 11226、
   `top_events/total` 376 → 384、`top_events/events[*]/headline`（谁和谁开战、谁迁都）成片不同。
   这些都是"世界怎么走"的量，改名碰不到。

**等价性证明的正确形态（这批不适用"键映回旧名"）**：digest 里没有可映的键，所以照批 A 那样写映射脚本**没有意义**。
真正的等价性证据是三条合起来：(a) 词表里没有任何改名键（上面第 1 条）；(b) **同一棵树内改前=改后** ——
批 A 实测过（`748B4AA6…` 前后一致，§7.5），本批 fork 也在同一棵树内实测前后一致（它自己的哈希口径 `feb672692c…`）；
(c) 差异只出现在事件计数与数值上。

**⚠ 旧声明更正**：提交 `a11c2ad` 的信息里写「digest 逐字节不变」**是错的**（分支与 main 实测不同）。
准确说法是：**本刀（改名）不改 digest；digest 的差异来自并发 feature 工作让世界分岔**。
前一位 agent 的"改前=改后"测量本身没错（它只在自己那棵树内比），错的是把它写成了"与 main 一致"。

**新基线（仅对本树本点有效）**：`3CA2A8019BF81879824D4C6B50D31FDC027CDB9B2FB7EE6C2929196F4400D598`。
⚠ 现在**多个 session 在并发加 feature，世界行为不稳定**（用户裁决原文：「世界行为本来就不 stable」），
所以这个值只对 `feature/control-nouns` 这个点成立；**功能风暴平息后必须重测**，别把它当长期基线。

### 7.8 一条行为层的红：与本刀无关（如实记录，不改断言凑绿）

* 判据名：`合成场景（预算）：批满 ⇒ 顶到产能上限（钱管够，是船坞的产能封顶）`
* 报错原文：`长三角 的 corvette（第一回合）：[(1, 10.0, 0.0)]`（`increment = 0`，即船坞这回合一点进度都没产出）
* **已排除的两点**：① 该判据的断言代码两棵树**逐字相同**（唯一差异是 patch 键名英文→中文与过滤列 `city_id`→`城名`）；
  ② main 上 `rm -rf target/test-fixtures/scenario/bp_budget_rich` 后重跑**仍 79/79 全绿** ⇒ 不是缓存撑着。
* **旁证**：同一张表里配对的那条"批 0 ⇒ increment = 0"**也是 0**（两边输出都是 `(1, 'corvette', 10.0, 0.0)`）
  ⇒ 在这个世界里"钱管够"与"钱为零"**产出一模一样**，即新并入的预算/国内市场功能在当前世界下**没有产生建造进度**。
* **结论**：**属行为层，由并发 feature 工作造成，与本刀（改名）无关**。
  待行为稳定后，由**预算/国内市场功能的那位作者**判定这是预期（设计如此）还是 bug。
  **本刀没有、也不会**为了变绿去改断言/改数字/改引擎。

### 7.9 第 5 步（已落地）：`control` 表的 `kind` 词表 = 叶的中文名词

* **病**：`--index` 的 `control` 派生表里 `kind` 是发射器**手写的英文串**
  （`ship_order`/`investment_budget`/`invest_weight`/`capital`…），而同一片叶在
  `--control-schema`、`--control`、`--apply` 里叫**中文**
  （`指令`/`投资预算`/`建设权重`/`首都`…）。于是 `planet_xq` 按英文 kind 筛、kit 与 apply 用中文键
  ——**同一个概念两套名**，两边写错都**不会红**，只会静默查不到（"看起来有值"）。
* **修**：名字只在 `src/control/leaves.rs` 的 `LEAVES[].field` 里写一次；`LeafSpec` 新增
  `state`（= `ControllableState` 的 **Rust 字段名**，投影据此选叶）与
  `not_in_index`（`Some(理由)` = 这片叶**不在** `control` 表里，理由必须非空）。
  投影发射器改调 `leaves::kind_of("ship_orders")` ⇒ 中文名不再有第二个字面量。
  `--index` 的 `control` 表 `kind` 列文档也由 `index_kinds()` **拼**出来（不再抄一份清单）。
* **纪律**：`play/tests/g4_spec.py` 第 7 条拿真跑出来的 `kind` 集合与
  `leaves[].field ∪ actions[].field − {自报 not_in_index}` **逐字双向对账**；
  `_g4_negative.py` 的 ⑳㉑㉒ 三个注入错（真文件里换个声明里没有的 kind / 把真在表里的叶标成不在 /
  把例外的理由改成空白）都要求**第 7 条自己红**。
* **例外只有两条**（写在声明里、带理由）：`设计图库`（结构叶，住 `derived.blueprints`，避免两份表示
  漂移）与 `建筑`（命令列表，不是叶、没有持久状态）。
* `decisions` 表的 `kind`（`ship_order`/`retool`/`style_retune`/`capital`/`blueprint`）
  **不在本步**：那是**决策类型**，另一个轴。

## 8. **有意保持英文的字符串**（动不得 / 不该动）

改名的边界不是"看到英文就换"。下面这些字符串**故意**保持英文，理由都归于同一条：
**它们是哈希输入或版本号/词汇枚举，不是给用户看的名词**——改了要么让同种子的世界变样，
要么让"可复现"这句话变成假的。

| 字符串 | 住在哪 | 为什么必须保持英文 |
| --- | --- | --- |
| `derived_roll` 的**盐**（`"accept"` / `"gate"` / `"observe_body"` / `"blueprint_theme"` / `"route"` / `"role"` / `"style_chance"` / `"nav"` / `"retool"` / `"blueprint_intent"` …） | `sim::derived_roll(faction, subject, round, purpose)` 的 `purpose` | 它**直接进哈希**（`(势力, 对象, 回合, 用途)`）。改一个字 ⇒ **同一个种子的世界完全变样**，而且和"改名"这件事毫无关系。它们**在读面上不出现**（`round_inputs.rolls[].purpose` 里能看到，那是**记录**不是名词）。 |
| `SCHEMA_VERSION` | `model/state.rs` | 存档/投影的**版本号**：它变 = "旧档要迁"这个信号。跟着 UI 词表跳舞会让迁移逻辑误判。 |
| `temper` / `lone_wolf` 的**盐用法** | `autocontrol::style` 的 `derived_roll` 调用 | 序列化名**可以**中文（`风格` 叶的**值字段**就叫 `temper`/`lone_wolf`，见 `LEAVES`），但**拿去当哈希盐的那两个词必须逐字是 `temper`/`lone_wolf`**——同一个词在两处承担不同职责时，**哈希那一处不动**。 |
| `ShipBehavior` 的变体名与参数字段名（`Idle`/`Move`/`Follow`/`DockCity`/`Dock`/`Colonize`，`position`/`ship`/`city`/`body`） | `model/ship.rs` | 它是**行为词汇**（枚举判别式），不是叶名；`指令` 叶本身的值字段叫 `行为`。改它 = 存档与 `--apply` 的兼容面整体翻一遍，收益只是"看起来更中文"。 |
| 投影**表名**与 `derived.*` 的键（`ships` / `cities` / `blueprints` / `control` / `derived.blueprints`…） | `projection.rs` 的 `LAZY`/`DERIVED` | 它们是 **API 路径**（`q.blueprints()` / `idx/blueprints.jsonl`），不是显示名词；表**内**的列名才是名词（`图名`/`舰级`…）。 |
| 建筑/结构的**配置键**（`residential` / `mining` / `construction` / `concrete` / `steel`）与资源**市场**列名 | `config/game.ron` / `Building.kind` 等 | 它们是**配置表的键**（数据驱动的那一端），不是 UI 名词；`--nouns` 里有逐条解释。 |

> **一句话判据**：**同一个字符串如果进了哈希、进了版本号、或是"配置/代码里的键"，
> 它就不是"名词"，不参与本轮改名**；反过来说，**凡是用户在读面上当名词看到的东西，
> 必须只有一处声明**（`serde(rename)` / `LEAVES` / `IDENTITY` / `LAZY`），别处从它派生。

### 8.1 顺手修掉的悬停弹窗英文（`///`）

`--nouns` 是**悬停弹窗的全部文字来源**，所以英文的 `///` 会**照实弹英文**给用户。
本轮把 `--nouns` 里**纯英文的属性描述**从 **36 条清到 0 条**（`Ship.船体` / `Faction.资源` /
`Faction.关系` / `Faction.符号` / `Faction.颜色` / `Building.护甲` / `Ship.坐标` /
`BuildingPatch` 的 7 个字段 / `ShipBehavior` / `BuildingPatch` 类型 / `Orbit` / `Trajectory` …），
复现口径：把 `--nouns` 走一遍，收集**没有任何 CJK 字符**的 `description`（改前 36 / 改后 0）。

### 8.2 顺带发现的缺陷（**未修**，留给下一刀）

* **单行 `/// **强调**` 会丢一个 `*`**：实测 `--nouns` 里 **57** 条描述以**单个 `*`** 开头
  （源里是 `**`），另有 94 条以 `**` 开头且**完好**。把 57 条逐一回溯源文件（把描述首行前面补一个
  `*` 再去找那行 `///`）：**57/57 命中，且全部是单行 doc comment**；完好的那 94 条里能对上源行的
  24 条**全部是多行**。于是悬停弹窗里会看到 `*库存资源**（…` 这种坏 markdown。
  这不影响任何判据（`g4` 只查"查不查得到解释"），但**是缺陷**。
  修法两选一：① 别让 doc 行以 `*` 开头（本轮新写的行已经这么做了）；② 在 `--nouns` 出境处做一次
  归一化。**本轮不动**：它跨 201 处声明、属于另一刀，而本 worktree 有并发的 session 在改同一批文件。

### 7.10 第 10 步（已落地）：`GameEvent` 载荷字段的中文名 + 补注释（批 B 的事件词汇）

* **病**（第 8 步如实记账的缺口）：`events` 读面表的追加列里 **11 个弹不出解释、列头是 ASCII**
  （`attacker` / `shipper` / `a` / `b` / `cargo` / `carrier` / `faction` / `how` / `prev_owner` /
  `reason` / `seeded_ship_class`）。根因：`GameEvent` 是 `#[serde(tag = "type")]` 的枚举，
  **变体字段既没有 `///` 也没有中文序列化名**（`///` 只写在变体级）。
* **改**：`GameEvent` 的**每一个**变体字段（23 个变体）+ 载荷类型 `Shot` / `Killer` 全部加
  `#[serde(rename = "中文名")]` + `///`（人话：这个字段是什么、单位/取值、什么时候有值）。
  `--nouns` 的 `state.definitions.GameEvent` 下 **98 个载荷字段**（121 个属性 − 23 个 `type`
  判别键）现在**条条有中文名、条条有解释**。`Shot` 的 17 个字段与 `Killer` 的 3 个也一并改了
  （它们会**逐字印在**「逐发」「凶手」那两格的文本里：`武器 0 战前船体 120 …`）。
* **判别键 `type` 与变体标签逐字不动**（`attack`/`city_razed`…）——它们是线上格式与词表枚举，
  按 §8 的判据不参与改名。
* **投影 `data` 载荷键同步**：`GameEvent::history_row` 是**手抄**的一份子集，键名逐条改成同一批
  中文名（`src/projection.rs` 的 `events.column_docs.data` 正文也跟）。新增 Rust 守卫
  `config::tests::event_payload_keys_are_the_serde_names`：`data` 的每个键都必须是 serde 发射
  出来的键、且**必须是中文名词**（手抄漂了 ⇒ 红）。
* **`SCHEMA_VERSION` 25 → 26**（事件住在 state 里 ⇒ 键名变了 = 旧档对不上，按「不考虑向前兼容」直接升）。
* **三端跟随**：`play/planet_xq`（`fates`/`cause`/`salvos` 的 `data` 取键）、`play/tests/g1_contract.py`
  （`--call military_deltas` 的合成事件 JSON）、`play/tests/g2_mid.py`（逐发分解与归因的取键）、
  `src/tests/projection`。**`web/static/**` 一个字节都不用改**：`events` 表在 `views.json` 里只声明
  了 `type`，其余全靠第 8 步的**自动追加**列 —— 名字一改，列头自己就全中文了（实机实测 25 个追加列
  全中文、0 个 ASCII）。
* **判据**：`g4_spec.py` **§8c 收紧**（从前 ASCII 名一律"如实记账"，现在**一个都不许弹不出解释**，
  确实还剩下的必须逐条自报理由：实测只剩 1 个 `faction`，是批 C 的决策结构体字段）；
  **新增 §8e「读面自检（数据级）」**——接手第 9 步随「未组织」页删掉的运行时自检 `renderSpecCheck`：
  **声明过的读列在真世界里必须至少取到过一次非空值**（口径照抄那段自检，求值走 `specview.js` 自己
  的 `evalPath`）。实测 **14 张表 / 130 条读列全部取到过非空值**，另有 1 张 `source: null` 的 3 条
  作用域账按更松的口径查「解析得到东西」。反向验证 40 个注入错全部咬住（新增 ㉞ 裸字段列 `path`
  改坏、㉟ **表达式列** `path` 改坏——后者**只有 §8e 看得见**）。
* **一个已知的口径限制（不是缺陷，是「名词→解释」这套机制的本性）**：语料是 **名词 → 解释**的
  一张表，同名名词**先到先得**。事件载荷里的 `载货`/`势力`/`舰级`/`城` 与实体字段同名，所以
  hover 弹出的是**先注册的那一条**（实测 `载货` 弹的是 `Ship.载货` 的解释）。真要按路径区分，
  得把语料从「名词 → 解释」改成「路径 → 解释」——那是另一刀（见 §7.6 的「表达式列表头」同类问题）。

### 7.11 第 10b 步（已落地）：按用户裁决订正事件载荷名（`feature/event-names-2`）

用户审了第 10 步那批名字，给了四条裁决。这一步**只改名字与注释**（`derived_roll` 的盐、
`type` 判别键、三个单元枚举的值、世界行为与断言数字**一个字没动**；`--digest` 逐字节不变已实测）。

* **裁决 1（通用规则）：按「这个字段存的是什么主体」命名**——存 `ShipId` 就叫「…舰」、
  存 `FactionId` 就叫「…方/…势力」。据此改三处：
  `Attack.attacker`「攻击方」→「攻击舰」、`Attack.target`「目标」→「目标舰」、
  `Siege.attacker`「攻击方」→「攻击舰」（三处都是 `ShipId`）。
  其余载荷字段逐条核过类型，**没有再需要改的**：`CityRazed.by_ship`「拆城舰」（`ShipId`）本来就带「舰」；
  「船东」「货主」「舰主」「新主」「旧主」「托运方」「承运方」「势力甲/乙」都是 `FactionId` 且已经带出主体角色；
  `Withdraw.to_body`「撤退目标」/`CapitalRelocated.from/to`「原首都/新首都」（`BodyId`）在变体语境里无歧义；
  `Shot`/`Killer` 内字段（`舰`/`势力`/`弹种`）已被外层「凶手」「逐发」限定，且 `play/planet_xq`
  按 `凶手.舰`/`凶手.势力` 取键 ⇒ 不动。
* **裁决 2：`share` → 「抽成」**（原「分成」）。**同物同名**落到**三处**（不止用户点名的两处）：
  ① 事件载荷 `ContractPosted.share`；② 投影 `contracts` 表的列 + `column_docs`（`src/projection.rs`）；
  ③ **`Contract.share` 自己的 `#[serde(rename)]`**（`src/model/contract.rs`，原「分成」）——不跟就是
  「同物不同名」没消灭干净（`--nouns` 的 `state.definitions.Contract` 与原始 JSON 视图读的是它）。
  `web/static/**` 与 `play/**` 一处都不用改（grep「分成」在两边都没有载荷取键）。
  ⚠ ③ 让 `Contract` 的档形状变了（该字段没有 `#[serde(default)]` ⇒ 旧档缺键读不回来）
  ⇒ **`SCHEMA_VERSION` 26 → 27**。
* **裁决 3：`prev_owner` 保留「旧主」，另三处语义相同的统一成「旧主」**——
  `CityRazed.owner`、`CityDefected.from`、`Revolt.faction`（原都是「失城方」）。
  **语义已逐条核过生产代码**：`raze_city` 的 `owner` = 夷平这一刻城的 `faction_id`（`sim/cities.rs`）；
  `defect_city` 的 `from` = 移出控制面的那一方（`sim/governance.rs`）；`Revolt.faction` 是
  `step_governance` 里那个城当前所属的 `fid`（同一个 `defect_city` 的另一分支）——
  三处**都真的是「上一任主人 / 失去它的那一方」**，没有一个是「发起方/叛乱方/新主」，故全部套用。
* **裁决 4：`shots` 保持「逐发」**；`ShipSpawned.via` 的读面名「来路」→「造舰路径」（与它自己的 `///` 一致）。
  上一轮报告把它挂到 `ColonyFounded` 上是**串行误会**（`via` 只住在 `ShipSpawned` 上）。
  全仓 grep「来路」，除这一处读面键名外只剩 `src/projection.rs` 一句散文（MOND 那一列的「来路与结论」）——
  **没有别的字段叫「来路」**。
* **三端跟随**：`src/tests/projection/mod.rs`（events 的 forbidden 列名 + `data["旧主"]` 取键）、
  `play/tests/g1_contract.py`（`--call military_deltas` 的合成事件 JSON 三处）、
  `play/tests/g2_mid.py`（拆平/复垦对账取 `data.旧主`）。`play/planet_xq` 与 `web/static/**` 不用改。
