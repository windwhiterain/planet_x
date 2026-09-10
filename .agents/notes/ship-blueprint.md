# 舰船设计图（非控制属性 = 建造单位上的模板）

> 状态 `[x]`（**已实现并提交**，`feature/ship-blueprint`：规格 §8.0 的十条逐条照办，
> `SCHEMA_VERSION` **9 → 10**，引擎 + `play/planet_x_ctl` + web 三端齐活，
> 同 seed `--digest` 与基线**逐字节相同**；实现记录 / 验收数据 / 未做项见本文 **§6**） ｜
> 索引：[notes.md](../notes.md) ｜ 关联：
> `ship-blueprint-spec.md`（**实现篇**：改动地图 / 叶片形状 / 测试计划）、
> `control-live-layers.md`（它的对偶：控制属性=活层）、`eras-technology.md`（「设计图分支」
> 剩余项就是它）、`military-combat.md`（换模块/再装配剩余项）、`agent-control-long-game.md`
> §6（结构性叶片的所有权不明）、`combat-behavior-doctrine.md`、`spawn_ship`（唯一的造舰漏斗）

## 0. 一句话

**控制属性是活层，非控制属性是快照。** 舰的面板/组件/造价这类「非控制属性」应当在
**建造单位（建造区建筑）** 上作为一份**设计图**存在：出厂那一刻把设计图**印成**一艘舰，
之后改设计图**不影响已有的舰**。

**用户裁决（2026-10）：这里就是「还不存在的实体的规则」的家。** 凡是"**新造的**舰该怎么
被造出来"的规则——出厂面板/选装/造价，**以及按舰级的默认指令**（"新造的护卫舰默认守家"）
——都属于舰船模板，**不要**在控制面上另立一层 `BTreeMap<舰级, …>`（那会分叉出两套概念，
见 `control-live-layers.md` §4.3 与 `engine-data-plane.md` §1.2）。对照：控制面管的是
**现存**舰的归属与值。

这条切分同时解掉三处旧账：

* `eras-technology.md` 的「设计图分支」；
* `agent-control-long-game.md` §6「`ship_type` 会被 AI 的舰队构成逻辑重估覆盖」——
  设计图是**控制属性（三态）**，AI 只重估 `Auto` 的设计图，玩家钉住的不会被覆盖；
* `military-combat.md` 的「换模块/再装配」= 把新设计图套到老舰上（回船坞花钱）。

## 1. 今天的非控制属性住在哪

| 东西 | 现在的位置 | 性质 |
| --- | --- | --- |
| 面板/造价/维护/槽位 | `ShipSpec`（`config/game.ron` 的 `ships:`，**按舰级**，全局） | 配置表 |
| 选装（模块表） | `Ship.components`，由 `autocontrol::choose_loadout(state, config, owner, class)` **确定性**地按势力资源优势选 | 出厂快照 |
| 出厂风格 | `ShipSpec.default_doctrine` / `default_kiting` → 拷进 `Ship.doctrine`/`Ship.kiting` | 出厂快照 |
| 造什么舰级 | `Building{kind:"construction", ship_type}`（建造区一个字段） | ⚠ **唯一**会被 AI 重估的非控制属性 |

两处**回填更正**（子 agent 逐行核实，见 `ship-blueprint-spec.md` §1.3/§1.5）：

* **出厂风格那两行在原 note 里写重了**：`ShipSpec.default_doctrine`/`default_kiting` 在
  `config/*.ron` 里**从来没有被填过**（grep 零命中），所以今天所有舰的出厂记录风格都是
  `{0,0}`/`0.0`；而且 `control-live-layers.md` 已把它们升成**活层** ⇒ 「有效风格」现在走
  `叶 → 势力默认 → 记录值` 的链，`Ship.doctrine/kiting` 只是**记录值**（出厂快照 + AI 流水）。
  设计图若还要带 `order`，必须说清它落在链的哪一层（spec §4.3，也是 Q1/Q2 的由来）。
* **造舰漏斗只有两条路，不是三条**：`spawn_ship`（`sim.rs`）仍是唯一漏斗，但 `SpawnVia`
  现在只剩 `Shipyard` / `Story`（`src/model/event.rs:94`）——第三条「反僵尸重建」随
  `step_resurgence` 在提交 `4283dc2` 删除。接入点仍然只有一个。

## 2. 设计图的形状（草案）

```rust
/// 一份设计图：舰级 + 选装 + （可选）面板修正。挂在建造区上，出厂时快照成舰。
pub struct Blueprint {
    pub name: String,                     // 唯一名（名字即 key）
    pub class: String,                    // 舰级（ShipSpec 的 key）
    pub components: Vec<ComponentId>,     // 选装表（替代 choose_loadout 的自动选装）
    pub doctrine: ShipDoctrine,           // 出厂风格（也可不在这里，交给控制层）
    pub kiting: f64,
}
```

两个"住哪"的方案（**✅ 已裁决：设计图库**）：

* **一坞一图**：`Building.design: Option<Blueprint>` —— 最贴「建造单位上的模板」这句原话，
  实现最小（不动阵营级数据面）。
* **设计图库**：势力级 `blueprints: BTreeMap<name, Control<Blueprint>>` +
  `Building{design: Option<BlueprintId>}` —— 能一图多坞、能统计「多少艘舰出自这张图」、
  也天然是 `eras-technology` 想要的「解锁式设计图」。
  *推荐后者*，理由：一坞一图在长局里会退化成「几十份重复的设计图」。

## 3. 语义决定（**✅ 已裁决**：以下四条按推荐全数通过）

即：设计图是**出厂快照**；设计图本身有**三态归属**（叶片）；`choose_loadout` **降级**成
`Auto` 设计图的默认生成器而不是被删；**refit 出本轮**（记进 `military-combat.md` 的剩余）。

* **§3.1 快照 vs 活层**：设计图**必须**是快照。
  理由：面板是舰的身份（`hull_max` 等由 `ship_panel(class, components)` 算出）。若设计图是
  活层，改一次蓝图会让**已有的舰**追溯变强/变弱——已经发布的轨迹（投影、战记、长局断言）
  会被追溯改写，那是本项目最忌讳的事。**推荐：快照**。
* **§3.2 设计图本身归谁**（三态）：`Auto` 时 AI 的舰队构成逻辑可以重估它（今天
  `buildings[].ship_type` 的命运，见 note §6 实测：r0 钉的巡洋舰到 r36 被改成战列舰）；
  `Player` 时系统不许动。**推荐：照 `control-live-layers.md` 的三态做叶片**。
* **§3.3 旧行为怎么衔接**：现在 `choose_loadout` 是"自动选装"。设计图落地后它应当降级成
  **默认设计图生成器**（`Auto` 的设计图 = 由它算出来），而不是被删除——基线的 8 个 AI 势力
  仍然需要它。
* **§3.4 再装配（refit）算不算本轮**：把老舰按新设计图改装（回本势力船坞、按差额付造价、
  可能掉完整度）。**推荐：不算本轮**，作为 `military-combat.md` 的剩余单独立项。

## 4. 实现草案（分步，等拍板后执行）

1. **数据面**：`Blueprint` 结构 + 势力级设计图表（或 `Building.design`）+ `SCHEMA_VERSION`
   升档 + `migrate()` 说明（新字段 `#[serde(default)]`，旧档缺字段 ⇒ 空表 = 继续走
   `choose_loadout` 的自动选装，行为不变）。
2. **造舰路径**：`spawn_ship` 的 `ShipSpawn` 加 `blueprint: Option<&Blueprint>`；船坞出厂路径
   （`sim.rs` 的造舰步进）按建造区的设计图传入；组件不再由 `choose_loadout` 现场决定，
   而是照设计图装配（`pay_components` 的成本逻辑不变）。
3. **控制面**：设计图作为**新叶片**（`blueprints` 补丁 + 读面），复用三态
   （`Control<Blueprint>`）+ 写值即接管 + `SkippedLeaf` 逐条丢弃报告。
4. **读面/投影**：`--control-schema` 自动跟随（`JsonSchema` 派生）；投影给 `ships` 懒表加
   `blueprint` 列（能统计"每张图造了多少艘"）。
5. **web**：建造区建筑编辑里选设计图（复用 `buildingEditor`）。
6. **测试**：出厂舰的面板/组件 == 设计图；改设计图**不**改已有舰；`Player` 的设计图不被
   AI 重估；长局守卫不崩。

## 5. 风险 / 开放问题

**实现规格已写好**：所有形状（数据结构 / config / 叶片 / 投影 / 迁移 / 测试 / 改动地图 / 验证命令）
在 [`ship-blueprint-spec.md`](ship-blueprint-spec.md)；**十条开放问题已逐条裁决**（同篇 §8.0）。
三条关键结论：

* **Q1 = (c)**：链插一层 `叶 → 图 → 舰队默认 → 势力 → 全局`，但**图的意图轴默认 `Inherit`**
  ——建图不等于表态；`Auto` 也算"有意见"，所以更要小心（加上 §3.2 那条：意图类叶片今天没有
  AI 写入者，"再加一个 Auto"必须先有执行者）。
* **Q2 = (b) 活层** + 读面加**出处列**（`order_source`）：一处改图、全级跟随，代价是必须能回答
  "这条意图是谁下的"。
* **Q3 = A**：`Building.ship_type` 仍是唯一真相，图的 `class` 必须相等。

下面是设计阶段就记下的风险（与 spec §9 互补，不重复）：

* **设计图爆炸**：AI 势力长期造舰会攒出几百份自动设计图 → 需要在 `Auto` 侧做**去重/复用**
  （例如按 (class, 选装签名) 归并），否则控制面读面会被淹没（对照 note §7 幽灵权重
  的教训：条目只会单调增长）。
* **平衡**：玩家拿到设计图编辑权 = 能造出「非法」组合吗？`ship_panel` 的槽位约束
  （`slots`）必须仍然生效；否则这是新的失衡入口（spec 的 Q4/Q7 也在这条线上）。
* **与 `eras-technology` 的解锁**：解锁式设计图需要一个「什么时代能造什么图」的门控表，
  那属于 `eras-technology.md` 的范畴，本 note 只把**容器**做好。

## 6. 实现记录（`feature/ship-blueprint`，`SCHEMA_VERSION` 9 → 10）

**一句话**：设计图 = 势力级库（`ControllableState.blueprints: BTreeMap<图名, Control<Blueprint>>`）
+ 建造区指针（`Building.blueprint`）+ 出厂快照（`Ship.components` / `Ship.blueprint`）。
十条裁决逐条落地的位置：

| 裁决 | 落地处 |
| --- | --- |
| Q1(c) 链插一层、**图的意图轴默认沉默** | `State::ship_control` / `State::ship_behavior`（`src/model/state.rs`）：图那一层**只在 `Blueprint.order.is_some()` 时参与**（建图 ≠ 表态），链 = `叶 → 图 → 舰队默认 → 势力 scope → 全局 scope` |
| Q2(b) 活层 + **出处列** | 舰上只记 `blueprint`（图名），取值时现查；投影 `ships.order_source` = `leaf` / `blueprint:<名>` / `fleet_default`（`State::ship_behavior_source`）。⚠ 它把「**叶不存在**」与「叶写着 `Inherit`」分开报：后者报 `leaf` |
| Q3 A `ship_type` 仍是唯一真相 | `apply_diff` 的**双向守卫**：改图或改区只要让二者不等就报 `blueprint_class_mismatch`；两处**一起写**（同一份 diff）则一次成功（`yard_ship_type_intent`） |
| Q4(b) 买不起不下水（**只对 `Player` 归属**） | `sim::blueprint_launch_blocked`（下水循环里按库存校验，买不起就 `break`、进度继续攒）＋ 可见标记：投影蓝图表 `launch_waiting` 列（由「回合末进度 ≥ `build_points` 却没下水」从状态推出来，**不落新状态**） |
| Q5 一张图一个 mode（不拆） | 图叶一个 `Control<Blueprint>`；`Auto` 只重估舰级/选装，**不**供 `order` 值 |
| Q6 图里不带面板修正 | `Blueprint` 只有 `class` / `components` / `order`；面板仍由 `config` + `ship_panel` 现算 |
| Q7 组件不许重复 | `apply_blueprint` 报 `duplicate_component`（另有 `no_such_component` / `too_many_components` / `no_such_class` / `missing_class`） |
| Q8 不预置标准图 | 世界生成零张图；`GameConfig.blueprints`（种子表）留着但**空**，且种子一律以 `Inherit` 写入 |
| Q9 `Ship.spawned_round` 同一次升档 | 状态字段 + 投影 `ships.spawned_round` 列（`null` = 旧档 ⇒ 未知）+ kit 的 `DEFAULT_REFRESH_RULE` 用它做「同分取最老的」 |
| Q10(a) 悬空指针停产 | `build_city` 收集建造区时跳过指针悬空的那些（不贡献产出速率 ⇒ 进度不涨）；`--apply` 报 `no_such_blueprint`；读面（`cities.buildings[].blueprint`）**原样输出**指针 |

其余三端（**写面 = 读面**）：
* **控制面**：`FactionControlView.blueprints`（读，含引擎算的 `ship_count`）/ `FactionControlPatch.blueprints`
  （写，`BlueprintPatch` 自带 `deny_unknown_fields` 白名单，收下只读的 `ship_count`）；
  `BuildingPatch.blueprint: Option<Option<String>>`（缺席 = 不动 / `null` = 拆指针 / 名字 = 指过去）；
  `BlueprintPatch.order` 同样是双 Option（缺席 = 不动 / `null` = **意图轴回到沉默**）；
  `blueprints` 在 `apply_diff` 里**先于** `buildings` 应用（同一份 diff 建图 + 挂指针要一次成功）；
  tagged 写法（`{"type":"dock","body":"地球"}`）在 `order` 上同样被 `normalize_behavior` 接受。
* **投影**：`ships` 加 5 列（`blueprint` / `blueprint_mode` / `order_blueprint_mode` / `order_source` /
  `spawned_round`）；`cities.buildings[]` 加 `blueprint`；新**派生表** `idx/blueprints.jsonl`
  （12 列，含 `effective_mode` / `ship_count` / `class_slots` / `component_cost` / `launch_waiting`）；
  `projection_schema()` 三处同步（ships 列 + 派生表条目 + `control` 表描述里写明「设计图**不在**本表」）。
* **kit**：`LEAF_KINDS["blueprints"]`、复合值分支（`_leaf_value`）、`set_blueprint` /
  `set_blueprint_and_retool` / `silence_blueprint_order` / `remove_blueprint` /
  `set_blueprint_pointer`、`buildings()` 加 `blueprint` 列、`roster()` 的 tie-break 换成
  `spawned_round`（缺列时跳过而不是抛错）；`planet_xq` 加三行的 `q.blueprints(round)`。
* **web**：建造区编辑器加「设计图」下拉（含「（无：自动选装）」= 拆指针、以及**悬空指针**的显式显示）。
* **迁移**：`v9 → v10` 只推版本号（四个新字段全部 `#[serde(default)]` ⇒ 空库 / 无指针 / 未知回合，
  每条建造路径都回到 `choose_loadout`）；`src/sim.rs` 的测试用**真的删掉那四个字段的 RON 文本**
  来证明这一点。

**验收数据**（2026-10，Git Bash + PowerShell，`--seed 7 --round 240 --digest 20`）：
`--digest` 的 12 行 JSON 与基线**逐字节相同**（sha256 `395E7D01…61DC8D`，改动前后同一个值；
改前/改后文件级的 sha256 只差一行编译器警告的行号——⚠ 第一次比对时把 stderr 也重定向进了文件，
别犯这个错）。「旧档 + 新二进制」`--start scratch/ckpt_v9.ron --round 228 --digest 20` 与
「旧档 + 旧二进制」同一 sha256（`3FF7B191…72A9FD`，11 行）。`cargo test --workspace` 全绿
（lib 139 + longhorizon 6 + projection_derived 4 + web 19 + web-main 2，ignored 11 条不变）。
kit demo「全部断言通过」，其中**同分取最老**由一对真实同分舰证明（`福煦`@r0 vs `北辰`@r4：
名字序会挑 `北辰`，编制表挑 `福煦`）。

**未做 / 偏离**（详见规格篇顶部状态行的同一张表）：refit（老舰套新图）、时代门控、
web 里**编辑/新建图**的面板（只做了建造区那一行的指针 + 悬空指针显示）、AI 主动建图
（`Auto` 图的执行者只有「重估已有图」这一半）、图形化的「买不起」提示（只有投影列）。
