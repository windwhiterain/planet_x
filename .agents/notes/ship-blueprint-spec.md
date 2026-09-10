# 舰船设计图（blueprint）实现规格 —— **已实现**（`feature/ship-blueprint`）

> 状态 `[x]`（**十条裁决逐条落地，三端齐活**）｜ 实现记录 / 验收数据 / 未做项：
> [`ship-blueprint.md`](ship-blueprint.md) §6 ｜ 分支 `feature/ship-blueprint`，
> **实现提交 `fe534ff`**（本行随文档修订提交补记；`git log --oneline main..feature/ship-blueprint` 看全），
> `SCHEMA_VERSION` **9 → 10**（v7→v8 产地货栈、v8→v9 货舱 + 运输 Haul 已被
> `feature/freight-collection` 用掉）｜
> 索引：[notes.md](../notes.md) ｜ 关联：[`ship-blueprint.md`](ship-blueprint.md)（**设计/裁决篇**：
> 四条语义已拍板）、`control-live-layers.md`（控制属性=活层，本规格是它的对偶）、
> `engine-data-plane.md`（读面/投影契约）、`agent-control-long-game.md` §6（`ship_type` 被 AI 重估）
>
> **验收（2026-10）**：`--seed 7 --round 240 --digest 20` 的 12 行 JSON 与基线**逐字节相同**
> （sha256 `395E7D01…61DC8D`）；「v9 旧档 + 新二进制」与「v9 旧档 + 旧二进制」
> `--digest` 同一 sha256（`3FF7B191…72A9FD`）；`cargo test --workspace` 全绿
> （lib 139 / longhorizon 6 / projection_derived 4 / planet_x_web 19+2，ignored 11 不变）；
> `play/planet_x_ctl` 的 demo 全部断言通过。
>
> **实现时相对本规格的偏离（四条，都写在这里而不是藏着）**：
> 1. §4.3 的代码片段用 `bp.mode.is_player()`（图叶**自己**的表态）判定图层供值；实现改用
>    **§2.4/Q4 明说的「归属解析为 `Player`」**（`State::blueprint_control`：图叶 → 势力 scope →
>    全局）。理由：同一个仓库里「归属」一律指**解析后**的三态，读面给的也是 `effective_mode`；
>    用叶自己的表态会让 scope 层永远接管不了图这一层（与其它每一条叶都不对称）。
> 2. §4.7 的括注「改**图**（`class` 与重算选装）」只做了 `class`：**选装不预生成**
>    （§2.4 的硬约束：一旦在这里算，出厂成本就从「下水那一刻」变成「改装那一刻」，不是行为中立）。
> 3. §5.3 的蓝图表多了一列 `launch_waiting`（Q4(b) 要的**可见标记**）。它是**状态的可观察后果**
>    （回合末进度 ≥ `build_points` 却没下水），所以不落新状态、也不加事件——正常路径下进度
>    每下水一艘就减一次，因此那个不等式只在「有下水被卡住」时成立。
> 4. 新增两个丢弃码 `no_such_class` / `missing_class`（§4.6 的表里没有）：`config.ship_spec()`
>    对未知舰级是 **panic**，所以「建图时舰级写错」必须在写面响亮拒绝而不是让它进状态。
>
> 另有两处**规格内部冲突**按更硬的那条办：Q9 的目的（「解编制表 tie-break」）要求
> `spawned_round` **进投影**（§5.1 只列了三列、§9 还承认那条 engine gap 仍然成立）⇒
> `ships.jsonl` 加了 `spawned_round` 列 + kit 的 `DEFAULT_REFRESH_RULE` 用它；
> §5.1 的三列之外还加了 `order_source`（Q2=(b) 明说要「出处列」，而 Q2 正出自 §8.0 的裁决表）。
>
> 这一篇是**实现规格**：现状核实（带 `文件:行号`）/ 数据结构 / config 形状 / 控制面叶片与解析规则 /
> 读面投影 / 迁移 / 测试计划 / 风险，外加「附 A 改动地图」「附 B 验证命令」。设计动机与已拍板的
> 语义在 `ship-blueprint.md`，这里不重复论证。
>
> 出处：由子 agent 依 `ship-blueprint.md` §3 的四条裁决 + 控制面/投影的既有契约产出（2026-10）；
> 其中 §1.5、§1.3 更正了旧 note 的两处事实（见 `ship-blueprint.md` §1 的回填）。

---

# 舰船设计图（blueprint）实现规格

> 面向**下一个实现 agent**：照着这份规格可以直接动手，不需要再重新调研一遍现状。
>
> 来源裁决：`.agents/notes/ship-blueprint.md` §0/§2/§3（四条语义**已裁决**）、
> `.agents/notes/control-live-layers.md` §1/§3/§4.3（活层模型 + 「按舰级默认 = 舰船模板的功能」）、
> `.agents/notes/engine-data-plane.md` §1（引擎=数据平面 / Python=策略平面）。
>
> 约定：所有断言都带 `文件:行号`。写作时仓库在 `main` @ `c332676`，工作区干净。
> 标 **未核实** 的地方是真的没去看，不是「大概是这样」。
> 本文里凡标 `【待裁决 Qn】` 的，都是我**没有替用户拍板**的点，集中在 §8。

---

## 0. 一句话与范围

**设计图 = 「还不存在的舰」的出厂规格**：一张势力级的、有名字、有三态归属的模板
（舰级 + 选装 + 该舰级的默认意图）。建造区**指向**一张图，下水那一刻把图**印成**一艘舰
（`Ship.components` 是快照，之后改图不影响已有舰）。

本轮范围（照 `ship-blueprint.md` §3.4 的裁决）：

* **做**：数据结构 + 势力级设计图库 + 建造区指针 + 出厂快照 + `choose_loadout` 降级成
  `Auto` 图的默认生成器 + 控制面叶片（三态 + 写值即接管 + 丢弃报告）+ 读面/投影 + 迁移 + 守卫。
* **不做**：refit（把新图套到老舰上，回船坞付差额）——归 `military-combat.md:152` 的剩余项；
  时代门控（「什么时代能造什么图」）——归 `eras-technology.md:9` 的剩余项；
  面板修正（设计图自带装甲/加固加值）——见 §2.3 与 Q6。

---

## 1. 现状核实（今天「舰的非控制属性」到底住在哪）

### 1.1 舰级的静态属性 = `config/game.ron`（**全局、按舰级、只读**）

| 东西 | 位置 | 性质 |
| --- | --- | --- |
| `hull` / `hull_regen` / `build_points` / `build_cost` / `upkeep` / `slots` / 8 个 `*_mult` 修正系数 | `src/model/ship_combat.rs:8-65`（`ShipSpec`），装载于 `GameConfig.ships`（`src/model/game_config.rs:564`），数据在 `config/game.ron:301-387` | 配置表，**运行时无人改写** |
| 组件（伤害类型/射程/护盾/硬度/点防/推进/成本/维护） | `src/model/ship_combat.rs:92-145`（`ComponentSpec`），`GameConfig.components`（`game_config.rs:566-567`），数据 `config/game.ron:393+` | 配置表 |
| 「有效面板」 | `ship_panel(config, ship)`（`src/model/ship_combat.rs:243-303`）：**舰级提供 base 船体 + 修正系数，战斗力全部来自所装组件 × 舰级系数** | 纯函数：`(config, ship) → ShipPanel` |

⚠ **面板不是纯快照**（重要，实现时要选口径）：`Ship.hull_max` 在出厂时算一次存下来
（`src/sim.rs:362-366`），但 `ship_panel` 每次都从 config 重算 `base.hull`
（`ship_combat.rs:253`），投影每次现算（`src/projection.rs:312`）。所以**改 `config/game.ron`
会追溯改变已有舰的「有效面板」**，而 `Ship.hull_max` 不变——今天没人改 config，所以没暴露。
设计图落地时不要把这个洞挖大：**图里不存数值**（§2.3），「面板快照」= 船体字段 + 组件表快照，
数值真值仍只有 config 一份。

### 1.2 选装：**出厂时算一次，之后谁都改不了**

* `choose_loadout(state, config, fid, class) -> Vec<String>`：`src/autocontrol/shipbuilding.rs:98-253`。
  纯确定性、**无 RNG**（同文件注释 `shipbuilding.rs:94-97`）；按「势力库存的资源丰度」打分，
  硬保证至少一件 `weapon`（买不起就不装，`shipbuilding.rs:233-235` 的 M7 硬门槛）与至少一件
  `thrust`（平台件，付不起也白送，`shipbuilding.rs:210-231`），其余按分数填满 `slots`。
* 调用点只有两处（非测试）：`src/sim.rs:338`（`spawn_ship` 内，**每次造舰现场算**）与
  `src/world.rs:839`（开局预置舰队补装，一次性）。
* 全库 `grep '\.components'`：非测试代码里写 `Ship.components` 的**只有** `world.rs:841` 与
  `sim.rs:355`（`Ship{}` 构造）⇒ **`Ship.components` 是出厂快照，AI 从不重估已有舰的选装**
  （`component_hp` 只修不换：`sim.rs:1672-1680` 修复、`sim.rs:2435-2450` 战损）。
* 组件成本在出厂时从库存扣一次：`sim.rs:369-377`（`pay_components`；剧情赠舰传 `false`，
  `sim.rs:3489`）。扣款走 `commit_spend`（`sim.rs:1204-1212`），**把库存钳到 0、绝不拒绝**——
  所以「买不起却照造」在今天是一条静默路径（见 Q4）。

**结论：选装 = 出厂定死。** 今天没有任何「每回合重算选装」的逻辑。

### 1.3 出厂风格（doctrine/kiting）= 从 `ShipSpec` 拷一份记录值

`sim.rs:358-359` 把 `cspec.default_doctrine` / `cspec.default_kiting` 拷进 `Ship.doctrine/kiting`；
字段定义 `ship_combat.rs:57-64`。**但配置里根本没有这两个键**（`grep default_doctrine|default_kiting
config/*.ron` 零命中）⇒ 今天所有舰出厂都是 `{0,0}` / `0.0`。
有效值走活层 `State::ship_doctrine` / `ship_kiting`（`src/model/state.rs:275-312`：叶 → 舰队默认 →
**舰上记录值**），`Ship.doctrine/kiting` 只是记录值（`src/model/ship.rs:80-96`）。

### 1.4 「造什么舰级」= `Building.ship_type`——**唯一会被 AI 重估的非控制属性**

* 字段：`src/model/building.rs:28-29`；语义 = 这个建造区产哪一级舰。
* 产线读取：`sim.rs:1430-1438`——`ship_type: None` 的建造区**什么都不产**（不是「默认护卫舰」）。
* 写入者（穷举）：
  1. 世界生成：`world.rs:181`（每城一个 `construction`，带 class）；
  2. 殖民/复垦：`sim.rs:2626` 算 `choose_next_class` → `sim.rs:2716` 建区时写入；
  3. 玩家/agent：`control.rs:1185`（新建时默认 `corvette`）、`control.rs:1276-1288`（改已有区，
     非建造区报 `not_a_shipyard`）；
  4. **AI：`retool_shipyards`（`autocontrol/shipbuilding.rs:259-310`，每回合由
     `sim.rs:1250-1253` 调用）**。
* **`retool_shipyards` 完全不看归属**：没有 `Control`、没查过 `scope`（`shipbuilding.rs:259-310`
  通读）。它只在「交战中 + 某舰型占舰队 ≥0.60」时，把**产出该舰型的最小
  (city, building)** 的 `ship_type` 改写成 `choose_next_class` 的结果
  （`shipbuilding.rs:277-309`，`rng` 消耗在 `:280`）。
  ⇒ 这就是 `agent-control-long-game.md` §6 那条「r0 钉的巡洋舰到 r36 被改成战列舰」的
  **唯一机制来源**，也是 `ship-blueprint.md` §3.2 要解的那个旧账。
  ⚠ 它还**消耗一次 RNG**（`choose_next_class` 的 `rng.unit()`，`shipbuilding.rs:72`）：任何
  改动都必须保持这个调用次序与次数，否则 `--digest` 会变（§7 的确定性守卫）。

### 1.5 造舰漏斗 = `spawn_ship`，**今天只剩两条生产路径**（更正旧 note）

`spawn_ship(state, config, ShipSpawn{ owner, class, position, city, via, pay_components })`：
`sim.rs:316-393`。三条路径的说法已过时——`SpawnVia` 现在只有两个变体
（`src/model/event.rs:94-106`），生产调用点只有：

1. **船坞出厂**：`sim.rs:1483-1490`（`via: Shipyard`，带 `city`，`pay_components: true`）；
2. **剧情赠舰**：`grant_story_ship` → `sim.rs:3483-3490`（`via: Story`，无 `city`，不付钱）。

（第三条「反僵尸重建」随 `step_resurgence` 在提交 `4283dc2` 删除；复垦现在走
`colonize` → `reseed_city`，只建城不赠舰：`sim.rs:2602-2661`。）

漏斗内做五件事：选装 → 确定性取名（`ship.rs:137-151`）→ 算面板并把 `hull/shield` 拉满
（`sim.rs:362-366`）→ 装 `component_hp`（`sim.rs:368`）→ 扣组件成本 + 发 `ShipSpawned` 事件
→ **给新舰插一条 `Control::inherit(Idle)` 的指令叶**（`sim.rs:385-390`，`Inherit` = 没有说话，
于是归属上溯到舰队默认：`state.rs:237-244`）。这就是设计图的**唯一接入点**。

### 1.6 造舰进度是「城市 × 舰级」的池子（不是「城市 × 建造区」）

`City.ship_progress: BTreeMap<String /*class*/, f64>`（`src/model/city.rs:27-28`）；
`sim.rs:1442-1496`：同城所有建造区**按 class 合并速率**、按 `build_weights` 竞争建造预算，
攒够 `build_points` 就下水（`build_cost / build_points` 是每点进度的资源价，
`sim.rs:1466-1474`）。
⇒ **两张同舰级、不同选装的设计图挂在同一座城时，共用一个进度池**。实现时必须给一个明确语义
（§2.6 / §9）。

### 1.7 控制面/读面的现状（本轮要动的地方，先列清楚）

| 组件 | 位置 | 现状 |
| --- | --- | --- |
| 叶片集合 | `ControllableState`，`src/model/control.rs:251-308` | 11 片：`ship_orders`/`ship_doctrine`/`ship_kiting`/`default_ship_order`/`default_doctrine`/`default_kiting`/两类预算/两类权重/`loyalty_budget`/`capital` |
| 三态 | `ControlMode`，`control.rs:63-92`（线格式 `Some(Player)` / JSON `"Player"`，`control.rs:126-177`） | 已落定，新叶直接复用 |
| 归属链（舰指令） | `state.rs:248-262`（叶 → 舰队默认 → 势力 → 全局）；值规则 `state.rs:230-245` / `:275-292` | 新层要插进这里 |
| 读面即写面 | `FactionControlView`（`control.rs:90-108`）+ `control_view`（`control.rs:432-536`）+ `round_view`（`:575-635`，数值 `r2` 到 2 位小数） | 每加一片叶，读面/写面/round/web 四处都要动 |
| 写面 | `FactionControlPatch`（`control.rs:308-351`，`deny_unknown_fields`）+ `apply_diff`（`:791-1109`）+ `write_value_leaf`/`write_mode_leaf`（`:702-738`） | 写值即接管；丢弃必须 `report.skip` |
| 结构叶 | `BuildingPatch`（`control.rs:259-285`）+ `apply_building_patch`（`:1117-1323`） | `ship_type` 只能写在建造区上（`:1276-1288`） |
| schema | `control_schema_value()` 由 `CommandReq` 自动派生（`control.rs:654-657`） | 加字段自动跟随 |
| CLI | `--control`（`main.rs:360`）、`--apply` 回执（`main.rs:276-310`）、`--control-schema` | 自动跟随 |
| web | `StateView{control,scope,info}`（`web/src/lib.rs:99-110/172-181`）+ `buildingEditor`（`web/static/app.js:653-712`）、`ship_type` 下拉（`app.js:667-669/710`） | 建造区编辑器要加一行 |
| 投影 | `LAZY`（`projection.rs:64-74`）、`DERIVED`（`:92-97`）、ships 行（`:311-356`）、cities 内联 `buildings[]`（`:360-375`）、派生发射（`:494-527`）、schema（`:573-700`） | **加表要改三处**：`DERIVED` 声明 + 发射 + schema（注释 `:90-91`，守卫 `:1202-1235`） |
| 迁移 | `SCHEMA_VERSION = 9`（`state.rs:12`）、`migrate()`（`state.rs:488-500`，注意 `0..=8` 那一臂） | 升档要改这一臂（**下一档是 10**：v8 = 货舱、v9 = 运输 Haul 已被 `feature/freight-collection` 用掉） |

---

## 2. 数据结构提案

### 2.1 推荐形状（三件东西）

```rust
// --- 新文件 src/model/blueprint.rs，并在 model/mod.rs:43-55 注册 + `pub use blueprint::*;` ---

/// 设计图 id = **名字**（势力内的唯一 key，与「名字即主键」一致）。
pub type BlueprintId = String;

/// 一份设计图：**出厂规格**，不是舰。
///
/// 它**刻意不携带任何数值**（面板/造价/维护）：那些只有 `config/game.ron` 一份真值
/// （`ShipSpec`/`ComponentSpec`），图里再抄一份就是第二个真相源（且会随 config 漂移）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Blueprint {
    /// 舰级：`ShipSpec` 的 key（`config.ships`）。
    pub class: String,
    /// 选装表（组件 id，顺序 = 槽位顺序）。空 = 「交给生成器」（= 今天的 `choose_loadout`）。
    #[serde(default)]
    pub components: Vec<String>,
    /// 本图的**舰级默认指令**（「新造的护卫舰默认守家」的落点，见 §4.3）。
    /// `None` = 本图对意图没有说话 ⇒ 落到舰队默认。
    #[serde(default)]
    pub order: Option<ShipBehavior>,
}
```

```rust
// --- src/model/control.rs: ControllableState 新增一片叶 ---
    /// **设计图库**（势力级）：名字 → 图纸。`Player` = 系统不许重估这张图（舰级/选装/意图全归玩家）；
    /// `Auto` = `choose_loadout` 每回合/出厂时现算选装（今天的行为，见 §3.3 的裁决）；
    /// `Inherit` = 这一层没有说话 ⇒ 归属沿 `scope` 链上溯（通常落到 `Auto`）。
    #[serde(default)]
    pub blueprints: BTreeMap<BlueprintId, Control<Blueprint>>,
```

```rust
// --- src/model/building.rs: Building 新增一个字段（建造区指向一张图）---
    /// 本建造区的**设计图**（在所属势力的 `ControllableState::blueprints` 里按名字查）。
    /// `None` = 没有图 ⇒ 走 `ship_type` + `choose_loadout`（**与今天逐字节一致**）。
    #[serde(default)]
    pub blueprint: Option<BlueprintId>,
```

```rust
// --- src/model/ship.rs: Ship 新增一个字段（出厂归因）---
    /// 本舰**出厂所用**的设计图名（快照的溯源，也是「按舰级默认」那一层的查表键，见 §4.3）。
    /// `None` = 无图（旧档 / 开局预置舰队 / 剧情赠舰）。
    /// ⚠ 它**不**表示「本舰的选装可以随图变化」——`components` 是快照（§3.1 的裁决）。
    #[serde(default)]
    pub blueprint: Option<BlueprintId>,
```

### 2.2 为什么是「势力级设计图库 + 建造区指针」，不是别的

* `ship-blueprint.md:52-59` 已裁决选「设计图库」而不是「一坞一图」（后者在长局里退化成几十份
  重复图）。本规格照办：图的宿主是**势力**（`State.control[FactionId].blueprints`），
  建造区只存**指针**（`Building.blueprint`）。
* **不放进 `GameConfig`**：`GameConfig` 是「规则」（无状态、无归属），而设计图是**状态**
  （谁拥有、哪张图在哪个区用），且 `GameConfig` 里没有 `State` 就无法表达归属与 `Auto` 重估。
  config 只放**开局种子**（§3.1，可选）。
* **不新开一层 `BTreeMap<舰级, …>`**：`ship-blueprint.md:14-18` 明确禁止；「按舰级默认」落在
  **图上的 `order`**（§4.3），已经是「按舰级」，因为一张图恰好是一个舰级。

### 2.3 面板 / 造价 / 维护放在哪（**不进 `Blueprint`**）

| 量 | 真值来源 | 出厂行为 |
| --- | --- | --- |
| 船体 `hull` / `hull_regen` | `ShipSpec`（config） | `ship_panel` 现算（`ship_combat.rs:253-254`），存进 `Ship.hull_max`（`sim.rs:362-366`） |
| 攻击/射程/速度/护盾/硬度/点防 | 组件 × `ShipSpec` 的 `*_mult` | 同上（`ship_combat.rs:261-301`） |
| 「造价」（进度用的资源） | `ShipSpec.build_cost` / `build_points`（config） | 进度池按 `build_cost/build_points` 扣（`sim.rs:1466-1474`） |
| 「造价」（选装一次性成本） | `ComponentSpec.cost`（config） | 出厂一次扣（`sim.rs:369-377`） |
| 维护费 | `ShipSpec.upkeep` + Σ组件 `upkeep` | `ship_panel(...).upkeep`（`ship_combat.rs:259/299`），`step_upkeep` 每回合现算（`sim.rs:732-737`） |

⇒ 图只决定 **class + components（+ 意图）**；「这张图造一艘要多少钱」是
`build_cost + Σ components.cost` 的**派生量**，引擎可以在读面算给 agent（不要 agent 自己算：
`engine-data-plane.md` 的分工），但**不存进状态**。

### 2.4 `pick_loadout` 与 `choose_loadout` 的关系（§3.3 裁决的落地）

* 新增 `fn resolve_loadout(state, config, fid, blueprint: Option<&Control<Blueprint>>) -> Vec<String>`：
  * 图存在、且**该图的归属解析为 `Player`**（§4.2）⇒ 用 `bp.components`（原样，见 §4.6 校验）；
  * 图存在但归属 `Auto` ⇒ **现场调 `choose_loadout`**（与今天同一条代码路径、同一时点）；
  * 没有图 ⇒ 现场调 `choose_loadout`。
* **`choose_loadout` 不删、不改行为**（`ship-blueprint.md:73-76` 的裁决）——它是 `Auto` 图的默认生成器。
* ⚠ **不许在别处（回合步进）预生成 Auto 图的选装**：今天选装是在**出厂那一刻**按当时库存算的
  （`sim.rs:338`），提前算会改变成本时点与结果（行为不中性）。

### 2.5 键的选取与「设计图爆炸」的防法

* 键 = **图名**（`BlueprintId = String`），势力内唯一。名字进 diff、进读面、进投影，
  与项目「名字即唯一 key」一致（`name-as-unique-key.md`）。
* `Auto` 侧的命名规则**必须是 class 的确定函数**（不消耗 RNG！），例如
  `"auto:{class}"`（或中文化 `自动·护卫舰`），并**每势力每舰级至多一张**
  ⇒ 上限 = 5 × 势力数（`config/game.ron:301-387` 共 5 个舰级）。
  这直接解掉 `ship-blueprint.md:97-99` 的「几百份自动设计图」风险。
* 玩家可以随便起名（`"重甲巡洋"`、`"长城级"`…）；**改名 = 删除 + 新建**（键变了，
  所有指向旧名的建造区会变成悬空指针——必须响亮报错，见 §9）。

### 2.6 与 `City.ship_progress`（按 class 的池子）的口径

两条可选口径（**推荐 A**，但需要用户确认，见 Q3/Q9）：

* **A（最小）**：`ship_type` 仍是「这个区造哪一级」的唯一真相；`blueprint.class` 必须 ==
  `ship_type`（否则 apply 报 `blueprint_class_mismatch`）。进度池语义**不动**。
  ⇒ 同城两张同舰级不同选装的图共用进度池，谁先攒够谁先下水（产出顺序仍由
  `build_weights` + 建筑下标决定：`sim.rs:1429-1457`）。**这是今天的语义，零回归。**
* **B（彻底）**：class 搬进图，`ship_type` 降级为派生列；进度键从 class 改成 blueprint id。
  ⇒ 世界生成（`world.rs:181`）、殖民（`sim.rs:2716`）、`--apply`（`control.rs:1276-1288`）、
  `retool_shipyards`、旧档迁移（migrate 拿不到 config，造不出图）全都要改。

---

## 3. `config/game.ron` 的形状

### 3.1 需要新增的：**只有一张可选的种子表**（建议本轮就加，但可以为空）

```ron
    // ── 设计图种子表（可选）────────────────────────────────────────────────────
    // 开局把这几张图放进对应势力的设计图库（`State.control[势力].blueprints`）。
    // · 缺这一节 / 写空 = 一张图都没有 ⇒ 所有建造区走 `choose_loadout`（**与今天逐字节一致**）。
    // · 归属（mode）不在这里给：种子一律以 `Inherit`（这一层没有说话）写入，
    //   由 `scope` 链解析（默认落到 `Auto`）。想让某张图开局就归玩家，用 `--apply` 钉。
    // · `components` 空 = 交给生成器（Auto）；非空 = 这张图的选装就是它。
    // · `order` 省略 = 本图对意图没有说话（落到舰队默认）。
    blueprints: {
        "中国": [
            (name: "护卫-守家", class: "corvette",
             components: ["kinetic", "ion_drive"],
             order: Some(Dock(body: "地球"))),
            (name: "巡洋-殖民", class: "cruiser",
             components: ["kinetic", "shield", "ion_drive"],
             order: Some(Colonize(body: "火星"))),
        ],
    },
```

对应 Rust 侧（`src/model/game_config.rs`）：

```rust
    /// 设计图种子表：势力名 → 该势力开局的设计图。`#[serde(default)]` 容忍旧配置无此节。
    #[serde(default)]
    pub blueprints: BTreeMap<FactionId, Vec<BlueprintSeed>>,
```
（`BlueprintSeed` = `{ name, class, components, order }`，只用于世界生成
`world.rs::default_state` 往 `control[fid].blueprints` 里塞 `Control::inherit(..)`；
`GameConfig` 只在世界生成时被读一次。）

### 3.2 明确**不改**的东西（避免有人顺手加）

* `ships:`（`config/game.ron:301-387`）——不加 `default_blueprint`、不加 `components`：
  舰级的静态属性与图无关。
* `components:`（`393+`）——不动。
* **不加** `blueprints_era` / 解锁门控表：归 `eras-technology.md:9`。
* **不加** `ships.<class>.default_order`（那会是为了「按舰级默认」开的第二张表，
  与 `ship-blueprint.md:14-18` 的裁决冲突）。

### 3.3 备选：**完全不动 config**

如果本轮只想把容器做好（`ship-blueprint.md:101-103` 正是这个口径），可以只加状态字段
（§2.1），config 一个键都不加：设计图只能由 `--apply` 创建。代价是「开局每个势力都有一张
标准图」这件事没地方写；好处是**行为中性是结构性的**（新开局连一张图都没有）。
**推荐**：先按 3.1 加可选表但**留空**（`blueprints: {}`），把「要不要开局预置图」交给用户（Q8）。

---

## 4. 控制面：叶片形状与解析规则

### 4.1 叶片形状（读面 = 写面）

读面（`FactionControlView` 加一片，`control.rs:90-108`）：

```jsonc
{ "faction_id": "中国",
  "blueprints": [
    { "name": "护卫-守家",
      "class": "corvette",
      "components": ["kinetic", "ion_drive"],
      "order": { "type": "dock", "body": "地球" },   // 或 null
      "mode": "Player",
      "ship_count": 3 }                              // 读面附加：本图造了多少艘（引擎算）
  ]
}
```

* `mode` 语义与其它叶一致：`Inherit`（这一层没有说话）/ `Auto`（系统可重估）/ `Player`（系统不许动）。
* **`components` 必须全量输出**（不是 `null`）：读面即写面要能「dump → 改 → 回传」，
  按 presence-aware 规则，`components` 缺席 = 不动、`[]` = 清空（交给生成器）。少输出 = 回传时清空选装。
* 是否输出 `ship_count`：推荐输出（引擎算、Python 只筛，符合 `engine-data-plane.md:22`），
  但注意它是**派生量**：读面的值必须由 `state.ships` 现算，别落进状态（否则又一个影子状态）。
  若与 `round_view` 的「模板原样回传」冲突（回传时多了个只读字段 → `deny_unknown_fields` 会炸），
  则**把它排除在写面之外**——同 `ShipOrderEntry` 的做法：给 `BlueprintPatch` 单独的
  `deny_unknown_fields` 白名单，读面字段是写面的超集（现状如此：`control.rs:306-307` 的注释）。

### 4.2 三态语义与 `Auto` 生成器

| 归属（图叶 → 势力 scope → 全局） | 引擎行为 |
| --- | --- |
| `Player` | 系统**不许**改这张图（class/components/order 全是玩家写的）；出厂按图装配 |
| `Auto` | 出厂时按 `choose_loadout` 现算选装（= 今天的路径）；AI 可以改 class（若 Q3 选口径 B）与写回流水 |
| `Inherit` | 这一层没有说话 ⇒ 沿 `scope` 链上溯（`resolve_chain`，`control.rs:14-20`）；全链无话 ⇒ `Auto` |

新增解析函数（与 `investment_budget_control` 同形，`state.rs:351-357`）：

```rust
    /// 谁负责这张设计图：图叶 → 势力 scope → 全局（**没有**「舰队默认」这一档——设计图是
    /// 势力的库，不是某支舰队的指令）。
    pub fn blueprint_control(&self, fid: &FactionId, bp: &BlueprintId) -> ControlMode {
        let leaf = leaf_mode(self.control(fid.clone()).and_then(|c| c.blueprints.get(bp)));
        let faction = self.scope.factions.get(fid).copied().unwrap_or_default();
        resolve_chain(&[leaf, faction, self.scope.global])
    }
```

**AI 写回**：与预算同形（`autocontrol/budget.rs:82-107`）——`Auto` 图写
`Control::inherit(Blueprint { .. })`（值是流水，不是指令）；解析成 `Player` 的图**绝不覆盖**
（用 `or_insert_with` 那套）。写回点：`spawn_ship` 里选装定案之后（顺手记「AI 最近怎么配的」），
或者干脆不写回（`Auto` 图的 `components` 保持空 = 「由生成器现算」）。**推荐不写回**：
少一个写入点、少一次潜在的行为偏移；读面想看 AI 在造什么，看 `Ship.components` 更真。

### 4.3 「按舰级默认指令」落在链的哪一层

`ship-blueprint.md:14-18` + `control-live-layers.md:102-105` 的裁决：**按舰级默认 = 舰船模板的功能**，
控制面不另立一层。

提案（**要用户确认优先级，见 Q1**）：链变成

```
叶 → 【本舰出厂图的 order（图叶是 Player 时才取值）】 → 舰队默认 → 势力 scope → 全局 scope
```

实现（`state.rs:230-245` 的扩写，**新层插在舰队默认之前**）：

```rust
    pub fn ship_behavior(&self, ship_id: ShipId) -> Option<ShipBehavior> {
        let s = self.ship(&ship_id)?;
        let c = self.control(s.faction_id.clone())?;
        let leaf = c.ship_orders.get(&ship_id);
        if leaf.map(|l| l.mode).unwrap_or_default() == ControlMode::Inherit {
            // ① 舰级层：本舰出厂那张图的默认意图（**只在图叶是 Player 时取值**——
            //    Auto 图的 order 是流水，不能当指令，与舰队默认同一条规则）。
            if let Some(bp) = s.blueprint.as_ref().and_then(|id| c.blueprints.get(id)) {
                if bp.mode.is_player() {
                    if let Some(o) = &bp.value.order { return Some(o.clone()); }
                }
            }
            // ② 势力级舰队默认（现状不变）
            if let Some(d) = &c.default_ship_order {
                if d.mode.is_player() { return Some(d.value.clone()); }
            }
        }
        leaf.map(|l| l.value.clone())
    }
```

两条候选语义（**必须二选一**，见 Q2）：

* **(a) 快照式**：出厂时把图的 `order` **写进该舰的指令叶**（`mode` 照图叶），之后图改了老舰
  不跟随。⇒ 与「非控制属性 = 快照」一致；「按舰级默认」变成一次性动作。
* **(b) 活层式（推荐）**：舰上只记 `blueprint` 名，取值时查图（上面的代码）。
  ⇒ 「意图」本来就是控制属性（项目的第一性区分：控制属性=活层、非控制属性=快照，
  `control-live-layers.md:14-15`），所以改图立刻对该图的所有舰（叶沉默者）生效，
  而**面板/组件仍是快照**。
  ⚠ `Auto` 图的 `order` 永远不会被采用（值规则），所以「AI 决定舰级默认意图」这件事**不会**
  自动发生——AI 依然靠逐舰叶写流水（`tactics.rs:355-357/407-409/418-420`）；这符合
  `control-live-layers.md:54-56` 那条「AI 写回的是流水不是指令」的推论。

### 4.4 写面：diff 的四种形状（可以直接抄进 `--apply`）

**(1) 新建一张图（写值即接管 ⇒ `mode: Player`）并把一个建造区指过去**

```jsonc
{ "control": [
  { "faction_id": "中国",
    "blueprints": [
      { "name": "重甲巡洋", "class": "cruiser",
        "components": ["railgun", "shield", "ion_drive"],
        "order": { "type": "dock", "body": "地球" },
        "mode": "Player" }
    ],
    "buildings": [ { "city": "长三角", "building": 4, "blueprint": "重甲巡洋" } ]
  }
]}
```

**(2) 只把某个区的选装钉死，class 不动（`blueprint` 指针 + 图一次性建好）**

同上，只是 `components` 里放的是你想要的组合；`blueprint_class_mismatch` 会在
`class != ship_type` 时报出来。

**(3) 让 AI 用生成器造（`Auto`），但仍记录这张图**

```jsonc
{ "control": [ { "faction_id": "中国",
  "blueprints": [ { "name": "auto:cruiser", "class": "cruiser", "components": [], "mode": "Auto" } ] } ] }
```
`components: []` + `Auto` ⇒ 出厂现算（今天的行为）。

**(4) 只改「新下水的护卫舰守家」，不点任何一艘舰**

```jsonc
{ "control": [ { "faction_id": "中国",
  "blueprints": [ { "name": "护卫-守家", "class": "corvette",
                    "order": { "type": "dock", "body": "地球" }, "mode": "Player" } ] } ] }
```
（前提：护卫舰的建造区都指向这张图，或按 Q1 的优先级裁决让图压过舰队默认。）

**(5) 拆掉指针（回到今天的 `choose_loadout`）**：`{ "city": "长三角", "building": 4, "blueprint": null }`
——写面要把「不改」与「清空」区分开（`Option<Option<BlueprintId>>`；缺席 = 不动，`null` = 清空）。

### 4.5 「写什么会让新下水的舰跟着变」对照表

| 你写的东西 | 之后下水的舰 | 已下水的舰 |
| --- | --- | --- |
| 新建一张图 + 建造区指向它 | 面板/选装/归因 = 这张图 | **不变**（快照；`ship-blueprint.md:66-70` 的裁决） |
| 只改**已有图**的 `class`（口径 B）或 `components` | 之后下水的带新选装，**已下水的面板/选装不变** | 面板/选装不变 |
| 只改已有图的 `order`（Q2=(b) 活层） | 新舰按新意图 | **该图的所有舰（叶沉默者）一起跟**（意图是活层） |
| 只改已有图的 `order`（Q2=(a) 快照） | 新舰按新意图 | 不变 |
| 只写 `default_ship_order`（舰队默认） | **所有**「叶沉默且图沉默（或没有图）」的舰，含新舰与开局舰队 | 叶沉默者跟着变（`control.rs:73-76` 已实现） |
| 只写 `scope.factions["中国"] = "Player"` | **什么都不给**：新舰归你但仍 `Idle`（作用域节点不带值，`control-live-layers.md:40-41`） | 归属变了，值没变 |
| 逐舰 `ship_orders` 叶 | 不影响新舰 | 只影响那一艘（永远最具体） |
| 只改 `build_weights`（同城的建造区权重） | 不改选装/面板，只改**哪个区先吃预算**（`sim.rs:1440/1457`） | 不变 |

⚠ **`Building.ship_type` 仍然决定「产哪一级」**（口径 A，§2.6）。只改图不改 `ship_type`
不会让船坞改产另一级舰——想要那样，两处一起写（或选口径 B）。

### 4.6 写面校验与丢弃码（`ApplyReport.skipped`，`control.rs:375-385`）

| code | 何时 | 说明 |
| --- | --- | --- |
| `no_such_blueprint` | `buildings[].blueprint` 指向库里没有的名字 | **必须响亮**，绝不静默回落到生成器（那就是「失败看起来像成功」，`agent-play.md:27-31`） |
| `blueprint_class_mismatch` | 图 `class` != 该建造区 `ship_type`（口径 A） | `reason` 里列出该区的 `ship_type` 与图的 `class` |
| `no_such_component` | `components` 里有 config 里不存在的组件 id | `reason` 列出 `config.components` 的键（或提示 `--meta`） |
| `too_many_components` | `len(components) > ShipSpec.slots` | `ship-blueprint.md:100-102` 的硬约束：槽位上限必须仍然生效 |
| `not_a_shipyard` | 把 `blueprint` 写在非建造区上 | 与 `ship_type` 同一条守卫（`control.rs:1280-1287`） |
| `no_such_faction` / `no_such_city` / `no_such_building` | 复用现有校验（`control.rs:743-780`） | — |

其它必守的规则：

* **写值即接管**：只写 `components`/`class`/`order` 而不写 `mode` ⇒ 该图叶变 `Player`，
  并记 `NOTE_APPLY_TOOKOVER`（`control.rs:719-738` 的 `write_value_leaf`）。
* 只写 `mode` 合法（值不动）——`{"name":"重甲巡洋","mode":"Auto"}` = 交回系统重估。
* 图名不存在但只写了 `mode`：报 `no_such_blueprint`（不许凭空造图，同 `no_such_faction`
  防幽灵势力的理由，`control.rs:795-806`）。
* `components` 的**重复**与**可负担性**见 Q4/Q7（本轮建议：先按「不许重复」= 与
  `choose_loadout` 的 `!chosen.contains(id)` 一致，`shipbuilding.rs:241`）。

### 4.7 与结构性叶片（`BuildingPatch`）的关系：**`blueprint` 也是一片会被 AI 覆盖的叶**

`agent-control-long-game.md` §6 的原始痛点（`ship_type` 的归属不明）在设计图落地后**只解一半**：
`blueprint` 有 `mode`（因为它在 `blueprints` 库里是一片叶），但 `ship_type` 仍然没有
（`BuildingPatch` 无 `mode`，`control.rs:259-285`）。所以必须同时做这件事：

> **`retool_shipyards` 要看设计图的归属**（`shipbuilding.rs:259-310`）：
> * 目标舰坞若挂了**归属为 `Player`** 的图 ⇒ **跳过它**，另选一个（或不动）；
> * 若挂的是 `Auto` 图 ⇒ 改**图**（`class` 与重算选装）而不是直接改 `ship_type`，
>   并保持 `ship_type` 与图一致（口径 A）；
> * **RNG 消耗次序不变**（`choose_next_class` 的 `rng.unit()` 仍在同一位置调用，
>   `shipbuilding.rs:280`），否则 `--digest` 会变（§7）。
>
> 不加这条 gate，「玩家钉住的设计图 AI 不许重估」的守卫会因为**另一条路径**（改 `ship_type`
> 造成 `blueprint_class_mismatch`）而失效——这是本轮最容易漏的一处。

---

## 5. 读面 / 投影

### 5.1 `idx/ships.jsonl` 新增列

在 `projection.rs:316-353` 的 `json!` 里加三列：

| 列 | 含义 | 取值 |
| --- | --- | --- |
| `blueprint` | 本舰出厂所用图名 | `s.blueprint`（`string` / `null`） |
| `blueprint_mode` | 该图**在其势力库里的叶表态** | `state.control(fid).blueprints.get(id).map(|l| l.mode)`，缺 = `Inherit` |
| `order_blueprint_mode` | 该图的 `order` 层表态（链上新增的那一层） | 缺图 / `order: None` = `Inherit` |

既有四列不动：`order_leaf_mode` / `order_default_mode` / `order_effective_mode` / `order_effective`
（`projection.rs:340-348`）——`order_effective*` 是**引擎解析后的答案**，已经包含新层，
Python 不要自己重算（`engine-data-plane.md:52`）。

### 5.2 `cities` 表的内联 `buildings[]` 新增键

在 `projection.rs:360-375` 的 `json!` 里加 `"blueprint": b.blueprint`。
⇒ agent 能一眼看出「哪座城的哪个下标在造哪张图」，而这正是指标 `build_weights` 需要的上下文
（那两列是 `(city, building)` 下标键，`control.rs:47-49`）。

### 5.3 新增派生表 `blueprints`（**推荐**）

理由：`control` 派生表的列是标量形状（`value: any` + `sub: integer`，`projection.rs:660`），
把一张 `{class, components[], order{}}` 塞进它的 `value` 会让列类型不稳、Python 侧还要二次解析；
而设计图是**结构叶**，值得一张有类型列的专用表（`engine-data-plane.md:22` 的「引擎给答案、
Python 只筛」）。

`projection.rs:92-97` 的 `DERIVED` 加一行：

```rust
    DerivedTable { name: "blueprints", table: "idx/blueprints.jsonl", key: "blueprint_id", join_on: "faction_ids", round: true },
```

发射（在 `write_round` 的派生段，`projection.rs:494-527` 附近）——每回合 × 每势力 × 每张图一行：

```json
{"round": 12, "faction_id": "中国", "blueprint_id": "重甲巡洋", "class": "cruiser",
 "components": ["railgun", "shield", "ion_drive"],
 "order": {"type": "dock", "body": "地球"},
 "mode": "Player", "effective_mode": "Player",
 "ship_count": 3, "class_slots": 3, "component_cost": {"铁": 24.0, "碳": 12.0}}
```

* `mode` = 叶自己的表态（没有叶 = 不可能：表里的行就是叶）；`effective_mode` = `blueprint_control`
  的答案（`Player`/`Auto`，`Inherit` 已被链消化）。
* `ship_count` = `state.ships` 里 `blueprint == Some(id)` 的条数（引擎算，回答
  `ship-blueprint.md:57-58` 那句「能统计多少艘舰出自这张图」）。
* `class_slots`（`ShipSpec.slots`）与 `component_cost`（Σ组件 `cost`）是**派生量**，推荐一并给：
  否则每个 recipe 都要自己去 `meta.json` 里 join 配置表。**可选**（若嫌表宽就砍掉）。
* `order` 用 tagged 形式（`{"type":"dock","body":"地球"}`）还是默认枚举形式
  （`{"Dock":{"body":"地球"}}`）？**两处不一致**：`control` 派生表直接 `json!(leaf.value)`
  ⇒ 默认枚举形式（`projection.rs:504`）；而 `--control` 读面同样是默认枚举形式，但
  `--apply` **两种都收**（`normalize_behavior`，`control.rs:1400-1414` 的调用点）。
  推荐**照 `control` 表用默认枚举形式**（与既有一致，不引入第二种口味）。

`projection_schema()` 的 `derived` 段（`projection.rs:639-679`）加：

```rust
            "blueprints" => json!({
                "table": t.table, "key": t.key, "join_on": t.join_on, "round": t.round,
                "description": "**舰船设计图库**（势力级）：一行 = 一张图。设计图是「还不存在的舰」的出厂规格——建造区指向一张图，下水时把图印成一艘舰（`components` 是快照，改图不改已下水的舰）。`Auto` 图的选装由 `choose_loadout` 在出厂时现算；`Player` 图的选装就是 `components`。",
                "columns": {"round":"integer","faction_id":"string","blueprint_id":"string","class":"string","components":"array","order":"object","mode":"string","effective_mode":"string","ship_count":"integer"},
                "column_docs": {
                    "blueprint_id": "图名（势力内的唯一 key）。`ships` 表的 `blueprint` 列 join 它。",
                    "class": "舰级（= 该建造区的 `ship_type`；口径 A 下二者必须相等）。",
                    "components": "选装表（组件 id，顺序 = 槽位）。空数组 = 交给 `choose_loadout` 生成器。",
                    "order": "本图给**新舰**的默认意图（`ShipBehavior`，默认枚举形式；null = 本图对意图没有说话）。⚠ 链上只在该图的 `mode` 解析为 `Player` 时取值。",
                    "mode": "图叶自己的表态：Inherit（没有说话）/ Auto（系统可重估选装）/ Player（系统不许动）。",
                    "effective_mode": "**有效归属**（`State::blueprint_control`：图叶 → 势力 scope → 全局；全继承 ⇒ Auto）。引擎解析，别在 Python 里重算。",
                    "ship_count": "本回合世界上 `Ship.blueprint == blueprint_id` 的舰数（引擎算）。",
                },
            }),
```

⚠ **三处一起改**：`DERIVED` 声明 / `write_round` 的发射 / `projection_schema` 的 `derived` 段——
守卫 `derived_tables_are_written_and_declared`（`projection.rs:1202-1235`）会钉住前两者与
`join_on`，但**不会**钉 `column_docs` 的完整性（schema 里多写一列而实际不发射，守卫抓不到
⇒ 记得 §7 的自检）。

**要不要同时在 `control` 派生表里也发一行 `kind="blueprint"`？** 两份表示 = 漂移风险
（本项目最忌讳）。推荐**不发**，只改 `control` 表的 `description`/`kind` 文档说明
「设计图不进这张表，见 `blueprints`」。若消费方需要「整面叶子的完整清单」，再加，并补一条
跨表一致性守卫（同一回合两张表的行集必须逐条对应）。

### 5.4 Python 侧 join（`play/planet_xq`）

`planet_xq` 已经会读 `schema["derived"]` 并暴露 `q.derived(name, round)`（`planet_xq/__init__.py:99-170`）
⇒ **新表零改动就能读**（这正是 `engine-data-plane.md` §8.1 那条教训修好后的红利）：

```python
import planet_xq
q = planet_xq.load("out")
bp = q.blueprints(12)                    # 或 q.derived("blueprints", 12)
sh = q.ships(12)
m = sh.merge(bp, left_on=["round", "faction_id", "blueprint"],
             right_on=["round", "faction_id", "blueprint_id"], how="left")
m.groupby(["blueprint_id", "class"]).size()          # 每张图造了多少艘（也可直接看 ship_count）
q.cities(12).explode("buildings").assign(
    blueprint=lambda d: d["buildings"].apply(lambda b: b.get("blueprint"))
).query("blueprint == '重甲巡洋'")                    # 哪些区在造这张图
```

`planet_xq` 侧**无需新函数**（若想让 `q.blueprints()` 与 `q.derived(...)`/`q.control()`
那几个便利读法同待遇，加一个三行便利函数即可）。真正要改的是**写侧套件** `planet_x_ctl`（见 §9.6）。

### 5.5 `--control` / `--control-schema` / `--schema` / `--round` 的影响

* `--control`：`FactionControlView` 多一片 ⇒ JSON 多一个 `blueprints` 数组（`control.rs:511-535` 加一行）。
* `--control-schema`：`CommandReq` 派生 ⇒ 自动多出 `blueprints` 与 `BuildingPatch.blueprint`（`control.rs:654-657`）。
  注意 `Option<Option<String>>` 在 schema 里会显示成 `["string","null"]` + 默认缺省；
  若嫌含糊，用 `deserialize_with` 的双 Option 写法并手写 description（见 Q5 无关，纯实现细节）。
* `--schema`（agent 视图，`agent.rs:33-58`）：`Trajectory.ships: Vec<Ship>` 直接序列化
  ⇒ `Ship.blueprint` **自动出现在每艘舰的行里**（`agent.rs:74`），零改动。
  ⚠ 这会让 `--round`/`--traj` 的每一行多一个键（读面变化，不是模拟变化）。
* `--derived`：**不含蓝图表**（它读的是 `RoundState.post`，一张**视图** `RoundView`——当时的名字是
  `Derived`；蓝图表由 state 现算）
  ⇒ 与 `faction_process`/`city_process` 那类「同一回合两个读面必须一致」的约束**不适用**；文档里要写明这一点，
  免得有人以为漏了一张表。

---

## 6. 迁移

* `SCHEMA_VERSION` 9 → **10**（`state.rs:12`；⚠ v7→v8 是产地货栈、v8→v9 是货舱 + 运输 Haul，
  都已被 `feature/freight-collection` 用掉 —— 动手前先 `grep SCHEMA_VERSION src/model/state.rs` 确认）。
* `migrate()`（`state.rs:488-500`）：把 `0 | 1 | … | 8` 那一臂改成 `0..=9`，
  并在文档注释里加一段 `v9 → v10`。
* **零信息损失**（要写进注释，照 `state.rs:455-481` 的体例）：
  新增四个字段全部 `#[serde(default)]`——
  `ControllableState.blueprints`（旧档 ⇒ 空库）、`Building.blueprint`（旧档 ⇒ `None`）、
  `Ship.blueprint`（旧档 ⇒ `None`）、`GameConfig.blueprints`（旧配置 ⇒ 空种子表）。
  于是**每一条建造路径都走 `choose_loadout`，与旧档在旧二进制下的行为逐值一致**：
  * 选装：旧档没有图 ⇒ `resolve_loadout` 走生成器（`sim.rs:338` 今天的路径）；
  * 面板/成本：完全不变（config 一元真值，§2.3）；
  * 意图：`Ship.blueprint = None` ⇒ 链上新增的那一层恒为空 ⇒ `ship_behavior` 的取值与今天同；
  * `retool_shipyards`：旧档没有图 ⇒ 新的归属 gate 不触发，改的仍是 `ship_type`。
  ⇒ **「旧档 + 新二进制」与「旧档 + 旧二进制」在同一 seed 下 `--digest` 必须逐字相同**
  （§7 的验收命令）。
* 不做向前兼容（AGENTS.md 的「不考虑向前兼容」）：新档旧二进制读不了是**预期**的
  （`migrate` 会因版本过新而显式 `Err`，`state.rs:488-491`）。
* 要不要顺手加 `Ship.spawned_round`（`engine-data-plane.md` §8.5 的候选：解 kit 的编制表
  tie-break）？与本轮正交，但**共用一次升档**最省事——见 Q9，我不替你拍板。

---

## 7. 测试计划

命名照现有风格；`文件:行号` 指向可照抄的既有守卫。

### 7.1 出厂快照（核心语义）

1. `spawn_uses_the_yard_blueprint`（`src/sim.rs` 测试区，参照 `sim.rs:3616-3660`）：
   给一座城的建造区挂一张 `Player` 图（`components: ["kinetic","ion_drive"]`），
   `advance` 到下水 ⇒ 断言 `ship.components == 图.components`、
   `ship.blueprint == Some("…")`、`ship.hull_max == ship_panel(config,&ship).hull_max`、
   组件成本确实从库存扣过（对比同 seed 不挂图的基线库存）。
2. `editing_a_blueprint_does_not_touch_existing_ships`（**最重要的一条**）：
   下水一艘 → 改图的 `components` → 断言旧舰的 `components`/`hull_max`/`shield_max` **逐值不变**；
   再下水一艘 ⇒ 新舰带新选装。（这是 `ship-blueprint.md:66-70` 裁决的可检查形式。）
3. `blueprint_components_respect_slots`：`len(components) > slots` 的补丁被拒（`too_many_components`），
   且**状态未被污染**（库里不出现这张图）。
4. `auto_blueprint_uses_choose_loadout_at_launch`：`Auto` 图（`components: []`）下水后的
   `ship.components` == 当场调 `choose_loadout(state, config, fid, class)` 的结果
   （`shipbuilding.rs:98`）；且**同一时点**——把库存改掉再下水，结果跟着变（证明没有提前缓存）。

### 7.2 归属与 AI 冲突

5. `player_pinned_blueprint_is_not_retooled`（照抄 `shipbuilding.rs:436-466`）：
   战时、舰队被单一舰型统治、其中一个建造区挂着 `Player` 图 ⇒ `retool_shipyards` 之后
   该区的 `ship_type` **与**图的 `class`/`components` 都没变；另一个 `Auto` 区照旧被重定向。
6. `blueprint_default_order_governs_new_ships`（照抄 `sim.rs:3616-3660` 的
   `fleet_default_governs_newly_built_ships`）：图 `order = Dock{地球}`（`Player`）⇒
   新舰出厂后 `state.ship_control == Player`、`advance` 一回合后它驶向地球（AI 不许接管）。
7. `fleet_default_still_covers_blueprintless_ships`：没有图/图没有 `order` 的舰仍由舰队默认作答
   （回归守卫，防新层把旧行为吃掉）。
8. **Q1 的优先级守卫**（裁决后写死）：图 `order` 与舰队默认**同时** `Player` 且冲突时，
   有效值必须等于裁决的那一层——写成一条名字里带答案的测试，避免下次有人「顺手改回去」。

### 7.3 写面/读面契约

9. `blueprint_patch_reports_every_skip_code`：`no_such_blueprint` / `blueprint_class_mismatch` /
   `no_such_component` / `too_many_components` / `not_a_shipyard` 各一条，断言
   `report.skipped[0].code` 与 `path` 点名到叶（体例见 `control.rs:1847-1863`）。
10. `the_control_template_round_trips_back_through_apply`（**扩展现有守卫**，`control.rs:1950-1965`）：
    模板里多出的 `blueprints` 片必须原样合法回传、`report.is_clean()`、且 `applied` 计数更新。
    （反向陷阱：读面少输出一个字段 = 回传时静默清空；这条守卫就是抓它。）
11. `writing_a_blueprint_value_without_mode_takes_over`（体例见 `control-live-layers.md:124` 提到的
    `writing_a_value_without_mode_takes_over`）：只写 `components` ⇒ 图叶变 `Player` +
    `report.took_over` 里有它。
12. `a_dangling_blueprint_pointer_is_reported_not_silently_ignored`：建造区指向一个被删掉的图名 ⇒
    apply 报 `no_such_blueprint`；**并且**那一回合不下水（若按这个语义，见 Q10）。

### 7.4 投影契约

13. 扩展 `derived_tables_are_written_and_declared`（`projection.rs:1202-1235`）：
    `blueprints` 表被写出、schema 有 `derived.blueprints`、`join_on = "faction_ids"` 在 main 行里存在。
14. 扩展 `projection_writes_lean_main_and_indexed_tables`（`projection.rs:753-832`）：
    `ships` 行有 `blueprint`/`blueprint_mode`/`order_blueprint_mode` 三列；
    `cities.buildings[]` 有 `blueprint` 键。
15. `blueprints_table_matches_the_control_face`（新）：同一回合，
    `idx/blueprints.jsonl` 的行集 == `state.control[*].blueprints` 的键集，
    `mode`/`components` 逐值相等（防两张表各说各话）。
16. 跨进程一致性：`tests/projection_derived.rs` 那套**不需要**为蓝图表扩展（它比的是
    `--derived` 与 `--index` 共有的 `RoundView` 数据，而蓝图表不在 `RoundView` 里，见 §5.5）。
    在测试里加一行注释说明「为什么不在这里」，免得后人以为是漏的。

### 7.5 确定性 / 行为中性（**硬门槛，必须逐字**）

17. `cargo test --workspace` 全绿（含 `longhorizon` 6 条 + `projection_derived` 2 条）。
18. `same_seed_reproduces_identically`（`tests/longhorizon.rs:353-373`）仍绿。
19. **同 seed 的 240 回合 `--digest` 逐字不变**（改前/改后各跑一次，记 sha256）。
    这一条成立的前提要写进实现注释：
    * 新增代码里**不许出现任何新的 `rng` 调用**（唯一的 RNG 消费者是 `choose_next_class` 的
      `rng.unit()`，`shipbuilding.rs:72`，它只被 `retool_shipyards`/`colonize` 调用；
      `choose_loadout` 是纯确定性的，`shipbuilding.rs:94-97`）；
    * `Auto`/无图路径**必须调用同一个 `choose_loadout`**（不能重写一份）；
    * `retool_shipyards` 的 `choose_next_class` 调用位置与次数不变（`shipbuilding.rs:277-283`）。
20. **旧档 + 新二进制 == 旧档原行为**：用 `play/exp2/ckpt_r12.ron`（`control-live-layers.md:127-129`
    用过的那份）跑 `--control`（应见空的 `blueprints`、`buildings[].blueprint` 全 `null`）与
    `--digest`，与旧二进制的输出逐字比对。
21. 长局健康：`no_nonfinite_over_long_run`（`longhorizon.rs:217`）、`world_value_is_bounded`
    （`:253`）、`world_is_multipolar`（`:415`）不变——**若 AI 开始自己建图**（口径 B 或
    Auto 图的重估），这三条是回归网。

---

## 8. 开放问题（**需要用户裁决，我不拍板**）

> 每条给「选项 + 后果」，可直接拿去问。

### 8.0 裁决结果（✅ 用户已确认 2026-10，逐条按「建议/倾向」定案）

| 问 | 定案 | 一句话理由 |
| --- | --- | --- |
| **Q1** 舰级层 vs 舰队默认 | **(c) = (a) 的安全版**：链插一层 `叶 → 图 → 舰队默认 → 势力 → 全局`，但**图的意图轴默认 `Inherit`（沉默）**——建图 ≠ 表态，只有真写了图的 `order` 才遮住舰队默认；`Auto` 也算"有意见"，所以更要小心 | 语义上仍只有一个规则（最具体者胜），又不会出现"我设了舰队默认，却对新造的护卫舰无效" |
| **Q2** 按舰级默认意图：快照还是活层 | **(b) 活层** + 读面加**出处列**（建议 `order_source`，取值 `leaf` / `blueprint:<名>` / `fleet_default` / `scope` / `record`） | "控制属性=活层"的第一性延续；光加 mode 列不够，**必须能回答"这条意图是谁下的"** |
| **Q3** `class` 的真相 | **A**：`Building.ship_type` 仍是唯一真相，图的 `class` 必须相等（不等报 `blueprint_class_mismatch`） | 旧档行为中性**是结构性的**；B 的迁移成本全额、收益打折（`migrate()` 拿不到 config，造不出图） |
| **Q4** 玩家写的图买不起 | **(b) 买不起就不下水、进度继续攒**（`≥ build_points` 时不 spawn、不扣进度，下回合再试）+ **可见标记**（事件或读面一列） | 诚实且确定性；"进度满了却不出舰"看不见会被当成 bug。⚠ **只对 `Player` 归属的图生效**，`Auto` 图保持旧行为 ⇒ 长局基线不必重标 |
| **Q5** 一张图一个 mode 够不够 | **不拆**（接受"钉选装连带钉意图"的耦合），在文档里写死；真出现需求再拆 `blueprints` + `blueprint_orders` | 有了 Q1(c) 的"意图轴默认沉默"，耦合只在"想钉选装又想让意图跟随"这一种组合下才有害 |
| **Q6** 图里要不要带面板修正 | **不做** | config 是数值唯一真值；图里再叠数字=第二个真值源+新失衡入口。"装甲型/速度型"用**新增组件/选装组合**表达 |
| **Q7** 组件能否重复 | **不允许**（apply 报 `duplicate_component`） | 与今天 `choose_loadout` 一致；允许重复是新机制，要重审 `slots` 与平衡 |
| **Q8** 开局要不要预置标准图 | **不预置** | 预置会让"新旧一致"从**结构性**退化成"要额外证明的"；真要预置则只预置图、初始舰队仍 `blueprint: None` |
| **Q9** 顺带加 `Ship.spawned_round` | **加，与蓝图同一次升档**（省一次迁移） | 解 kit 编制表的确定性 tie-break（"同分取最老的"，今天只能用名字序近似）；旧档缺字段 ⇒ 记"未知"、回落名字序 |
| **Q10** 悬空图指针 | **(a) 该建造区停产**（进度停攒）+ `no_such_blueprint` + 读面原样输出指针 | (b) 静默回落生成器 = 「失败看起来像成功」；而图名会换代（玩家改名），这条**一定会被触发** |

**两条贯穿性要求**（不是问答，是上面结论的推论，实现时必须照办）：

1. **图的 `order` 轴默认 `Inherit`；`Auto` 必须有真实执行者，否则拒绝**。依据
   `control-live-layers.md` §3.2 的新事实：**风格/意图类叶片今天没有任何 AI 写入者**，
   所以"再加一个 `Auto`"若不实现执行者，就是给 agent 一个假承诺（读面说"系统会重估"，
   实际永远不动）。
2. **读面契约两端一起动**：新列（`order_source` 等）要同时进 `projection_schema()`、`planet_xq`、
   `planet_x_ctl` 的 `LEAF_KINDS`、`agent-play.md` 的链描述——这轮已经因为漏掉消费者而两次踩坑
   （`engine-data-plane.md` §8.1/§8.7），别犯第三次。
3. **⚠ `order_source` 必须把「叶不存在」与「叶写着 `Inherit`」分开报**（本轮新查实，
   `control-live-layers.md` §10.4/§11）：这两者在**归属**上等价、在**取值**上不等价——
   `State::ship_doctrine` 是 `leaf.map(|l| l.value).unwrap_or(record)`，所以"叶存在但没表态"
   照样用它叶里的值，只有叶**不存在**才回落到出厂记录值。蓝图那一步要给"回出厂快照"一个动作，
   而引擎刚从补丁层拿到了它（`remove: true` 删叶 + `NOTE_APPLY_REMOVED`，`control-live-layers.md` §11.1）：
   图层的意图轴若要表达"我不管"，得想清楚是写 `Inherit` 还是**删掉这片叶**——两者后果不同。

**动手顺序（用户已确认）**：① web 三条（`control-live-layers.md` §8）→ ② 两轴叶（同篇 §3.1）
→ ③ 本规格（连同 Q9 一次升 `SCHEMA_VERSION`，**9 → 10**：v7→v8 是产地货栈、v8→v9 是货舱 +
运输 Haul，都已被 `feature/freight-collection` 用掉）。

---

**Q1（最重要）舰级层 vs 势力舰队默认，谁更有权威？**
现状链是 `叶 → 舰队默认 → 势力 → 全局`；插入舰级层有两种放法：

* **(a) 图（舰级）压过舰队默认**（= 「最具体者胜」的直译：舰级 ⊂ 势力）。
  后果：「给护卫舰钉了守家的图」之后，玩家再写 `default_ship_order` 对新护卫舰**不再生效**
  （只对没有图的舰/图没表态的舰生效）。语义一致，但违反直觉：「我明明设了舰队默认」。
* **(b) 舰队默认压过图**（势力级 intent 优先，图只在舰队默认没表态时兜底）。
  后果：玩家写了舰队默认之后，「新造的护卫舰守家、巡洋舰殖民」这类按舰级编排**失效**，
  要按舰级编排就必须**不写**舰队默认（或者写舰队默认时接受它覆盖一切）。
* 我的倾向：(a)（链的语义最干净、与 `resolve_chain` 的单一规则一致），但这条会改变
  `agent-play.md:326-335` 教给 agent 的心智模型，**必须由用户拍**。

**Q2「按舰级默认意图」是快照还是活层？**（§4.3 (a)/(b)）
* **(a) 快照**：出厂时写进该舰的指令叶。后果：改图对老舰无效；「按舰级默认」是一次性动作；
  与「非控制属性=快照」的直觉一致，但意图明明是控制属性。
* **(b) 活层（我推荐）**：舰上记 `blueprint` 名，取值时查图。
  后果：改图立刻对该图的所有舰（叶沉默者）生效 —— 好处是「一处改，全级跟」；
  代价是「图」变成了一个**活的控制层**，agent 需要理解「这艘舰的意图来自哪张图」，
  而**读面必须把 `order_blueprint_mode` 给出来**（§5.1），否则又是一次「读数不反映行为」。

**Q3 舰级（class）的真相在哪？**（§2.6 A/B）
* **A（我推荐，最小）**：`Building.ship_type` 仍是唯一真相，图的 `class` 必须与它相等；
  进度池仍按 class。后果：改动只在选装/意图侧；同城两张同舰级图共用进度池（今天语义）。
* **B（彻底）**：class 搬进图，`ship_type` 变成派生列，进度键改成图名。
  后果：世界生成 / 殖民 / `--apply` / `retool` / 迁移全要动；`migrate()` 拿不到 config，
  旧档的 `ship_type` **造不出图**（只能保留 `ship_type` 兼容字段，于是 B 的收益打折）。

**Q4 玩家写的图「买不起」怎么办？**（今天 `commit_spend` 把库存钳到 0，
`sim.rs:1204-1212`；`choose_loadout` 的免费兜底只适用于生成器，`shipbuilding.rs:210-231`）
* **(a) 照造、库存钳零**：与今天 `pay_components` 的行为一致，但玩家能凭空造出超预算舰队
  ——`ship-blueprint.md:100-102` 明确担心的失衡入口。
* **(b) 买不起就不下水，进度继续攒**（确定性、诚实；需要一个"等钱"的读面/事件，否则玩家
  看到「进度满了却不出来」会困惑）。
* **(c) apply 时按**当时**库存校验并拒绝**：不成立——库存每回合变，写下合法、之后买不起，
  于是还是回到 (a)/(b)。
* 我的倾向：(b) + 一条 `ShipBlueprintWaiting` 事件或读面标记；但这是平衡决定，归用户。

**Q5 一张图一个 `mode` 够不够？** 单片叶 ⇒ 「钉死选装」会**连带**钉死该图的 `order`
（因为 `order` 是图值的一部分，图叶 `Player` 时它才被链采用）。
若用户想要「选装归我、意图交给舰队默认」，需要拆成两片叶
（`blueprints: Control<Blueprint>` + `blueprint_orders: BTreeMap<BlueprintId, Control<ShipBehavior>>`）。
拆的代价：多一片叶、读面多一段、kit 多一个 kind；不拆的代价：一个耦合。

**Q6 设计图要不要带「面板修正」？**（`ship-blueprint.md:41-50` 草案里的「可选面板修正」）
我建议**本轮不做**：config 是数值唯一真值（§2.3），图里加数值 = 第二个真值源 + 新的失衡入口。
若要给玩家「装甲型/速度型专精」（`eras-technology.md:9`），正确做法是**新增组件**或用
选装组合表达，而不是在图里叠数字。**请确认"不做"。**

**Q7 同一件组件能不能装两遍？**（今天 `choose_loadout` 不会重复，`shipbuilding.rs:241`）
允许重复 = 新机制（双主炮）且要重新审 `slots`/平衡；不允许 = 与今天一致。
建议**不允许**（apply 报 `duplicate_component`），但请确认。

**Q8 开局要不要预置「标准图」？**（§3.1/§3.3）
* 预置（config 种子表非空）：开局就有「护卫-守家」这类图，玩家一接管就能看懂；
  代价是**行为中性不再结构性成立**（要额外证明种子图不会改变基线 AI 的造舰）。
* 不预置（推荐）：新开局零张图 ⇒ 「新旧一致」是结构性的；代价是开局没有任何「按舰级默认」
  的示范，agent 得自己建图。
* 若选预置，还要定：初始舰队（`world.rs:832-850`）要不要挂到这些图上（挂了才让「按舰级默认」
  覆盖开局舰队；不挂则只覆盖新造的舰）。我建议**只预置图、初始舰队仍 `blueprint: None`**。

**Q9（顺带）要不要一起加 `Ship.spawned_round`？**（`engine-data-plane.md` §8.5）
解 kit 编制表的确定性 tie-break（今天只能用名字序当代理）。同一次升档最省事，但与本轮正交。

**Q10 悬空指针（图被删/改名，建造区还指着它）的语义？**
* **(a) 该区停产**（进度停攒），apply 报 `no_such_blueprint` + 读面把 `blueprint` 原样输出。
  ⇒ 响亮、无静默。
* **(b) 回落到 `choose_loadout`**：静默改变玩家意图 —— **我强烈反对**（「失败看起来像成功」）。
请确认 (a)。

---

## 9. 风险与陷阱

1. **名字即主键，图名同样会「换代」**。舰名换代是战沉后重建（`ship.rs:128-151`，`长城` → `长城2`）；
   图名换代是**玩家主动重命名**（= 删旧建新）。两者的共同后果：任何按名字写的 diff 下一回合
   可能指向不存在的东西 ⇒ **引用了不存在的图必须响亮报 `no_such_blueprint`**，
   绝不能静默回落到生成器。
2. **旧档加载**：四个新字段全部 `#[serde(default)]`；`migrate()` 的 `0..=8` 臂要改
   （`state.rs:488`）；别忘了 `RoundState` 也有自己的 `schema_version`（`state.rs:93-95`）。
   另一条：`SCHEMA_VERSION` 注释里要写清 **v9→v10** 的**零信息损失论证**（照 `state.rs:461-487` 的体例）。
3. **AI 与玩家的所有权冲突有两条路**，只堵一条没用：
   ① 图的归属（本规格）；② **`retool_shipyards` 改 `ship_type`**（`shipbuilding.rs:259-310`，
   今天不看任何归属）。必须按 §4.7 给 retool 加 gate，否则玩家钉的图会被
   `blueprint_class_mismatch` 式地「间接作废」。
4. **`write_value_leaf` 的接管语义与读面回传是一对**：读面若少输出 `components`，
   「dump → 改 → 回传」会静默清空选装（presence-aware 的经典坑，`control.rs:28-41` 的
   `ControlScope::overlay` 同一族）。守卫 7.3-10 就是抓它。
5. **只有 `choose_loadout` 一条生成路径**：不要在 `Auto` 图上另写一份选装逻辑；
   也不要提前（回合步进里）算——今天是在**出厂那一刻**算的（`sim.rs:338`）。
6. **kit（Python）侧的连带改动**（`engine-data-plane.md` §8 的教训：契约是发射端 + 消费者两处）：
   * `play/planet_x_ctl/planet_x_ctl/__init__.py:95-106` 的 `LEAF_KINDS` 要加 `blueprints: ("name",)`；
     顺手把**已经缺的** `default_doctrine` / `default_kiting` 补上（它们是 70e15e5 之后的叶子，
     kit 还没跟上——这是既有缺口，不是本轮引入）。
   * `_KEY_FIELDS`（`:109`）加 `"name"`；`_VALUE_FIELD`（`:113`）对 `blueprints` 无效——
     图的「值」是复合的（`class`+`components`+`order`），要像 `ship_doctrine` 那样给一个
     复合分支（`_leaf_value`，`:117-120`），否则 `verify()` 的字段比较会全错。
   * `_engine_path_to_leaf`（`:1645-1664`）靠 `_ENGINE_PATH_RE`（`:1592`）把
     `中国.blueprints[0].components` 映射到叶名 ⇒ 新 kind 走同一条正则即可，
     但要用**新 diff 里的 name**（`_entry_key`，`:449-455`）——加一条 demo 断言。
   * 新便利函数：`set_blueprint(faction, name, class=…, components=…, order=…, mode=…)`、
     `q.blueprints(round)`；README 的 API 表与「Engine gaps」一节要更新
     （`engine-data-plane.md` §8.5 那条「没有 spawned_round」仍然成立）。
7. **web**：`web/static/app.js` 的 `buildingEditor`（`:653-712`）要加「设计图」下拉
   （现有 `ship_type` 下拉 `:667-669/710` 是模板），并且要能显示「该图归谁」（
   照 `app.js:511` 的 `normMode(fc.default_ship_order.mode)` 写法）；`web/src/lib.rs` 的
   `StateView` 自动跟随（`:180`）。⚠ `control-live-layers.md:89/111` 的教训：web 改动**只跑
   crate 测试不算验证**，要按 `scripts/web.ps1` 实机点一遍。
8. **性能/规模**：蓝图表每回合每势力每图一行，量级与 `control` 表相同（那表现已是
   「每叶一行」，`projection.rs:494-527`）。主要风险是 `Auto` 侧不再按 class 归并
   （§2.5）⇒ 长局几百张图。**按 class 封顶**是硬要求。
9. **不要在图里存数值**（面板/造价/维护）：config 一元真值（§2.3）。若将来要做「装甲型专精」，
   走新增组件，不要在图里加 `+armor` 这类字段。
10. **`--derived` 与 `--index` 的一致性约束不适用于蓝图表**（它不在 `RoundView` 里——当时的名字是
    `Derived`；§5.5）
    ——写进文档，免得后人以为漏了一张表，或者反过来硬把它塞进 `RoundView`（那会让
    `--derived` 也依赖 state 的额外计算，破坏「post 是 state 的函数」的既有理由）。
11. **别忘 `agent-play.md` 要跟着改**（`control-live-layers.md:106-108` 已经欠着这一笔）：
    §3 的控制面表要加一行「设计图」；§4 的 diff 示例要加「按舰级定意图」；§2 的 lazy 表
    要加 `blueprints` 派生表与 ships 的三个新列。
12. **`--round`/`--traj` 的 JSON 每行多一个键**（`agent.rs:74` 直接序列化 `Ship`）：
    这是读面变化，不是行为变化；但如果有人在对比「改前/改后的 `--round` 输出」当作行为中性
    的证据，会误判 —— 行为中性的判据是 **`--digest`**（§7.5-19）与 `longhorizon` 的状态流
    sha256（`control-live-layers.md:92` 用过的手法），不是 `--round`。

---

## 附 A：改动地图（实现时按这个清单走）

| 文件 | 改动 |
| --- | --- |
| `src/model/blueprint.rs`（新） | `BlueprintId` / `Blueprint`；`mod.rs:43-68` 注册 + `pub use` |
| `src/model/control.rs` | `ControllableState.blueprints`（`:251-308`）；`BlueprintEntry` 读面；`FactionControlView.blueprints`（`:90-108`）；`control_view`（`:511-535`）；`round_view`（`:575-635`）；`BlueprintPatch` + `FactionControlPatch.blueprints`（`:308-351`）；`apply_diff` 的新分支（写值即接管 + skip 码）；`BuildingPatch.blueprint`（`:259-285`）+ `apply_building_patch`（`:1276-1288` 附近） |
| `src/model/building.rs` | `Building.blueprint`（`:22-36`） |
| `src/model/ship.rs` | `Ship.blueprint`（`:49-102`） |
| `src/model/state.rs` | `SCHEMA_VERSION 9→10`（`:12`）；`migrate()`（`:488-500`）+ 注释；`blueprint_control`；`ship_behavior` 插新层（`:230-245`） |
| `src/model/game_config.rs` | `GameConfig.blueprints`（可选种子表，`:534-578`） |
| `src/model/event.rs` | （可选）`ShipSpawned` 加 `blueprint: Option<String>`（`:171`）——**读面/事件归因**；加了要同步 `history_row`（`:395`）与 headline（`:573-578`） |
| `src/world.rs` | 种子表 → `control[fid].blueprints`；开局舰队 `blueprint: None`（`:832-850` 不变） |
| `src/sim.rs` | `ShipSpawn` 加 `blueprint: Option<&BlueprintId>`（`:316-326`）；`spawn_ship` 用 `resolve_loadout`（`:338`）并记 `Ship.blueprint`；船坞路径传该区的图（`:1483-1490`）；剧情路径传 `None`（`:3483-3490`） |
| `src/autocontrol/shipbuilding.rs` | `resolve_loadout`（新）；`retool_shipyards` 加归属 gate（`:259-310`，**保持 RNG 次序**） |
| `src/projection.rs` | ships 三列（`:311-356`）；`buildings[].blueprint`（`:360-375`）；`DERIVED`（`:92-97`）+ 发射 + schema（`:639-679`） |
| `src/control.rs` | 见上（读面/写面/schema 自动跟随） |
| `web/static/app.js` | `buildingEditor` 加设计图行（`:653-712`） |
| `play/planet_x_ctl/planet_x_ctl/__init__.py` | `LEAF_KINDS`/`_KEY_FIELDS`/`_leaf_value`/新 helper（`:95-120`、`:449-470`） |
| `.agents/notes/ship-blueprint.md` | 勾 `[x]`、补实现记录与「未做」 |
| `.agents/notes.md` | 索引行状态改 `[~]`/`[x]` |
| `.agents/agent-play.md` | §2 表 + §3/§4 示例（见 §9.11） |

## 附 B：验证命令（照抄）

```bash
cargo test --workspace                      # 单测 + 长局 + 投影派生的跨进程守卫
cargo test --test longhorizon               # 6 条快守卫
cargo run --bin planet_x -- --seed 7 --round 240 --digest 20 > /tmp/digest_after.txt
#   ↑ 与改前的同一命令输出**逐字比对**（§7.5-19）；这是行为中性的唯一硬判据
cargo run --bin planet_x -- --start play/exp2/ckpt_r12.ron --control    # 旧档：blueprints 应为空
cargo run --bin planet_x -- --seed 7 --round 12 --index out/
python -c "import planet_xq,sys;q=planet_xq.load('out');print(q.blueprints(12))"
```

---

*本规格由调研产出，未实现任何代码；`scratch/` 是 gitignored 目录（`.gitignore:12`），
仓库被跟踪文件未被改动。*
