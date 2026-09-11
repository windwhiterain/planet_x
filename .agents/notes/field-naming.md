# 字段顺序 + **序列化中文名**（设计待裁决）

> 用户原话（2026-10）：
> 「我们先调整 rust 字段顺序，同时给字段加序列化中文名字（避免用单独的翻译表），名字参考 spec，
> 把不准的名词问我。**所有 UI 都用名词，鼠标移上去弹窗显示注释/解释**」
>
> **状态：`[ ]` 设计待裁决**——本文只写方案与名词表，**代码一行未动**。

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
| `Contract.share` / `min_reputation` | 分成 / 最低名声 ❓ | — |
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
