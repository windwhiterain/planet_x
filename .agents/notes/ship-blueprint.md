# 舰船设计图（非控制属性 = 建造单位上的模板）

> 状态 `[x]`（**已实现并提交**，`feature/ship-blueprint`：规格 §8.0 的十条逐条照办，
> `SCHEMA_VERSION` **9 → 10**，引擎 + `play/planet_x_ctl` + web 三端齐活，
> 同 seed `--digest` 与基线**逐字节相同**；实现记录 / 验收数据 / 未做项见本文 **§6**） ｜
> 索引：[notes.md](../notes.md) ｜ 关联：
> `ship-blueprint-spec.md`（**实现篇**：改动地图 / 叶片形状 / 测试计划）、
> `control-live-layers.md`（它的对偶：控制属性=活层）、`eras-technology.md`（「设计图分支」
> 剩余项就是它）、`military-combat.md`（换模块/再装配剩余项）、`agent-control-long-game.md`
> §6（结构性叶片的所有权不明）、`combat-behavior-doctrine.md`、`spawn_ship`（唯一的造舰漏斗）
>
> ## ⚠ 2026-10 修订：图带**倾向**，不带指令
>
> 用户裁决「蓝图不需要指定指令，可以指定 风格/角色」覆盖本文的 Q1/Q2：图的 `order` 换成
> **倾向三轴** `doctrine` / `kiting` / `role`（各自 `None` = 该轴沉默，逐轴独立），
> 舰队级 `default_ship_order` 同时删除。判据、实测与三端改动：
> [`blueprint-stance.md`](blueprint-stance.md)。下面保留原文以存裁决历史。

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

* **Q1 = (c)**（⚠ 2026-10 修订：**指令**链已无图层与舰队默认；这一条现在描述**倾向三轴**的链
  `叶 → 图 → 舰队默认 → 势力 → 记录值`）：链插一层，但**图上的每条轴默认沉默**
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

## 6. 实现记录（`feature/ship-blueprint`，实现提交 `fe534ff`，`SCHEMA_VERSION` 9 → 10）

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
web 里**编辑/新建图**的面板（只做了建造区那一行的指针 + 悬空指针显示；**→ 已由 §8 补上**）、AI 主动建图
（`Auto` 图的执行者只有「重估已有图」这一半）、图形化的「买不起」提示（只有投影列；**→ 已由 §8 补上**）。

web 里**编辑/新建图**的面板（只做了建造区那一行的指针 + 悬空指针显示）、图形化的「买不起」提示
（只有投影列）。

**后续补齐（`feature/ai-agency`，本轮）**：~~AI 主动建图（`Auto` 图的执行者只有「重估已有图」
这一半）~~ → `[x]` **已补**：`autocontrol::blueprints` 让 AI 自己**建图 / 重估选装 / 按
`(舰级, 选装签名)` 去重复用 / 回收没人指向的自建图**（图名 `自动{主题}·{舰级}` ⇒ 图库是
`O(主题数 × 舰级数)`，正是 §5「设计图爆炸」那条风险的答案），闸门照旧（玩家钉的图与它那个
建造区整个跳过、悬空指针不替玩家收拾）。同时把 `resolve_loadout` 的旧语义
「`Auto` 图的选装被静默忽略」改成 **图 = 出厂规格、`mode` = 谁可以改这张图**（否则 AI 建的图
影响不了任何一艘船）。实现记录 / 实测证据 / 平衡留白见
[`control-live-layers.md`](control-live-layers.md) §17。

## 7. 独立验收（**审查方**用另一套量具重跑，2026-10）

实现方自述只当线索：下面是**另一个进程**用自己的量具、不看实现代码、只对契约跑出来的结果
（两把量具都在 `scratch/`，gitignored）。

1. **总量具** `scratch/accept_blueprint.ps1`：
   * `cargo test --workspace` 全绿：lib **138 passed / 1 ignored**、longhorizon 6+10ignored、
     projection_derived 4、web **19 + 2**、doc-tests 0（共 9 份 `test result`）。
   * **行为中性**：`--seed 42 --round 240 --digest 20` 的 digest 行 sha256 = `293725C4…DBC4`
     —— 与合并前 main（v9）**逐字节相同**（取法：只取 `^\{` 行、`\n` 连接、UTF-8 无 BOM）。
   * **旧档零损失**：审查方自己那份 v9 档（`scratch/blueprint_fixture/ckpt_v9.ron`）在新二进制下
     跑 `--round 240 --digest 20`，与 **v9 二进制**逐字节相同（`38E08F94…`）。
   * **旧档读面 / 投影**：`--control` 只多出 `blueprints`（空库），其余**全部 9 个势力逐字段相同**；
     投影里 13 艘舰与 22 个建造区的 `blueprint` 全 `null`，`spawned_round` 全 `null`（= 未知）。
2. **契约探针** `scratch/accept_blueprint_contract.py`（A–E 五段**全部通过**）：
   * A 读面=写面：建图 → 读回**全量** `components` → **原样回传是不动点**；`remove` 删图也支持；
   * B 六个丢弃码一字不差地报出来（`no_such_component` / `duplicate_component` /
     `too_many_components` / `blueprint_class_mismatch` / `no_such_blueprint` / `not_a_shipyard`）；
   * C 链与出处四态齐全（`None` / `leaf` / `fleet_default` / `blueprint:守家`），且
     **图层压过舰队默认**由**一艘真下水的舰**（`玉衡`）验证：`order_source = blueprint:守家`、
     有效意图 = 图里的 `Dock 月球`；
   * D 投影与 `schema.json` 声明齐、`cities.buildings[].blueprint` 在、ships 五列齐
     （`spawned_round` 有真值）；
   * E `--control-schema` 有 `blueprints`、`--schema` 有 `blueprint`。
3. **kit 端到端**：`demo.py` **全部断言通过**（含 `[4b]`/`[4c]` 与「同分取最老」的真实同分对证明）。
4. **web 实机**（审查方自己点的一遍，3001；证据 `scratch/blueprint-web-verified.png`）：
   建图 → 建造区那一行显示 `设计图 [侦察护卫（护卫舰·Player）]` → **删图后**同一行变成
   `侦察护卫（库里没有这张图 ⇒ 本区停产）`（Q10(a) 在界面上可见）。
   ⚠ 第一次挂指针被**正确地拒了**：AI 的 `retool_shipyards` 早已把 `长三角#3` 从 `battleship`
   改成 `corvette`，而图还是按旧舰级建的 ⇒ `blueprint_class_mismatch`。这条顺带证明**守卫真的在工作**。

**审查方另外钉住的一条语义**（探针两个方向都测了）：指令轴上「叶写着 `Inherit` + 舰队默认是
**Player**」⇒ **舰队默认的值压过叶里的值**（出处诚实地报 `fleet_default`）；「叶存在就用叶里的值」
出现在**舰队默认不是玩家**时（出处报 `leaf`）。风格三轴与指令轴在这里**不一样**——
`order_source` 存在的意义就是把这件事说出来。

## 8. web：设计图库面板（`feature/web-blueprint-editor`）

**一句话**：建造区那一行的「设计图」下拉以前只能指向**库里已有的图**，而**库里永远只有零张**
（`feature/ship-blueprint` 只做了指针 + 悬空指针显示）。这一轮把**库**搬到界面上：
一个势力级的**设计图库**（新建 / 改 / 删 / 三态归属 / 派生列），并让 `launch_waiting`
（Q4(b) 的「买不起 ⇒ 不下水」）第一次**看得见**。

### 8.1 面板长什么样

* **位置**：左侧控制树里、势力下面**第一个**分类 `设计图库（N 张）`——排在「舰 / 预算 / 天体」
  之前。理由：建造区那一行的下拉只能指向库里已有的图，**先有库、才有指针**；空库也显示
  （否则建不出第一张图），并在下面写一句"库里还没有图：…"，把「图 → 指针」的动线说清楚。
* **一张图一行**，行摘要 = 名字 · 舰级 · 选装（组件的中文名，空装显示「空装（交给生成器）」）·
  意图（`不表态` 或行为摘要）· `N 艘（本图造过）`（引擎算的 `ship_count`）· `挂在 K 个建造区`
  · `+ ⚠ 买不起 ⇒ 未下水`（`launch_waiting` 为真时）。行头右侧是**归属三态**下拉（`modeToggleFor`，
  与其它叶同一套）、`恢复继承`、`删掉这张图`（`remove: true`，破坏性配色）。
* **行内编辑器**：舰级下拉（`config.ships`，显示 `label（key）`）、**组件多选**
  （`config.components` 全表复选框，勾选顺序 = 槽位顺序；到槽位上限就把没勾的禁用 ⇒
  `duplicate_component` / `too_many_components` 在界面上就发不出去）、**意图编辑器**
  （与「指令」编辑器同形：同一套类型 + 天体/城/舰下拉，只多一格 **「不表态（交给舰队默认）」**）。
* **「＋ 新建设计图」**：图名 + 舰级 + 归属 + 意图 + 组件 + `新建`。点「新建」只把它加进**编辑面**
  （一片**壳**），点「应用到服务器」才落地——于是「建图 + 挂指针」可以是同一份 diff（引擎按
  `blueprints` 先于 `buildings` 应用，`ship-blueprint-spec.md` §4.4 例 1）。
* **建造区那一行**（保留既有的指针下拉与悬空指针显示）新增三块**写在行上**的状态：
  悬空 ⇒ `库里没有这张图 ⇒ 本区停产`（**沿用既有措辞**，下拉 option 里那一条一字未改）、
  舰级对不上 ⇒ 把 `blueprint_class_mismatch` **预告出来**（列出是哪几个区、三条出路）、
  **`买不起 ⇒ 未下水（进度在攒）`**（琥珀色小牌 + tooltip 说明"进度不会丢"）。
  换图时这三块**就地重算**（下拉里的选择领先于读面，不能等下一次重画）。

### 8.2 发出的载荷（实机抓到的原文）

界面照旧**只回传差异**（`buildCommandDiff`），设计图只是多了一种叶：

```jsonc
// ① 新建一张图 + 同一份 diff 里把建造区指过去（真实抓包）
{"control":[{"faction_id":"中国",
  "blueprints":[{"name":"侦察护卫","class":"corvette","components":["kinetic","ion_drive"],
                 "order":null,"mode":"Player"}],
  "buildings":[{"city":"长三角","building":3,"blueprint":"侦察护卫"}]}]}

// ② 只改选装（其它字段一个都不发）
{"control":[{"faction_id":"中国","blueprints":[{"name":"侦察护卫","components":["kinetic"]}]}]}

// ③ 把意图还给"沉默"（与"删掉这张图"是两件事）
{"control":[{"faction_id":"中国","blueprints":[{"name":"侦察护卫","order":null}]}]}

// ④ 删掉整张图（挂它的建造区随后悬空 ⇒ 停产）
{"control":[{"faction_id":"中国","blueprints":[{"name":"侦察护卫","remove":true}]}]}

// ⑤ Auto 归属（选装留空 = 出厂时按库存现算）
{"control":[{"faction_id":"中国","blueprints":[{"name":"auto:护卫","class":"corvette",
              "components":[],"order":null,"mode":"Auto"}]}]}

// ⑥ 两处一起写（舰型 + 指针同一条 building 命令）——口径 A 的正解，界面也走得通
{"control":[{"faction_id":"中国","buildings":[{"city":"水星熔炉基地","building":21,
              "ship_type":"corvette","blueprint":"auto:护卫"}]}]}
```

### 8.3 引擎侧只加了两处**读面**数据（行为中性）

1. **`launch_waiting` 进控制读面**（`src/control.rs`）：`BlueprintEntry` 加一个
   `launch_waiting: bool`，由**投影用的同一个函数** `sim::blueprint_launch_waiting` 现算
   （不落状态）；`BlueprintPatch` 加一个只读的 `launch_waiting: Option<bool>`（收下不写，同
   `ship_count` 的做法），于是"读面即写面、模板原样回传"仍然合法。`control_view` /
   `control_surface` 因此多接一个 `&GameConfig`（只用于这条派生列，不参与任何取值决策）。
   **为什么不是前端重算**：那是第二把量具，会与投影列漂移；引擎算、前端只显示
   （`engine-data-plane.md` 的老规矩）。**开工前的实测**：`--control` 对**开局零张图**的世界
   **逐字节不变**（before/after 同一个 sha256 `AB14895A…5FE6`，21065 字节）。
2. **`POST /api/command` 把 `ApplyReport` 回给页面**（`web/src/lib.rs`）：新增
   `StateView.report: Option<ApplyReport>`（`skip_serializing_if`，所以 `/api/state` /
   `/api/advance` / `/api/new` 的响应形状**一字不改**），界面把它显示在「应用到服务器」旁边。
   **为什么必须回**：`--apply` 的语义是"只触碰 diff 里出现的叶片"⇒ 静默丢掉与成功落地在响应上
   完全一样；而设计图的四条守卫（`blueprint_class_mismatch` / `duplicate_component` /
   `too_many_components` / `no_such_component`）恰恰是**玩家最需要看到的一句话**。
   顺带把「应用到服务器」和回执一起做成 `position: sticky` 的底部条（**副作用**：那个"按钮滚出
   视口 / 被底部读面盖住"的坑不在了）。

### 8.4 实机（`scripts/web.ps1`，3001，pid 42020，构建 121s 内；本 worktree 的 exe）

`GET /api/ping` 先验明身份（pid / 端口 / 二进制路径 / 构建时长全对上），再把世界**重建为种子 42**
（`/api/new`，与 CLI 的世界同源），然后逐步点：

| # | 点的是什么 | 发出的载荷 | 落地结果 |
| --- | --- | --- | --- |
| 1 | 设计图库 → 名字/舰级(corvette)/组件 2 件/归属 玩家 → `新建` | （还没发；只是编辑面里的壳） | 库显示 `设计图库（1 张）`，行摘要有 `0 艘 … 还没挂到任何建造区` |
| 2 | 建造区 `长三角/3` 的「设计图」下拉 → 侦察护卫 → `应用` | 见 §8.2 ① | 回执 `✓ 全部落地：2 条`；读面 `blueprints[0] = {corvette, [kinetic,ion_drive], order:null, mode:Player, ship_count:0, launch_waiting:false}`，指针 = `侦察护卫` |
| 3 | 图行里取消一个组件 → `应用` | 见 §8.2 ②（**只带 `components`**） | 读面选装变 `["kinetic"]`；再 `应用` 一次 **发 `{"control":[]}`** ⇒ 幂等 |
| 4 | 推进 5 回合 | — | `ship_count: 2`（`北辰`@r3、`镇岳`@r5，两艘都带 `components:["kinetic"]`——**图真的决定出厂选装**） |
| 5 | 再把组件加回 2 件 → `应用` | 见 §8.2 ② | 图 = `[kinetic,ion_drive]`，而**已下水的两艘仍是 `["kinetic"]`** ⇒ 快照语义 |
| 6 | 推进 5 回合 | — | `launch_waiting: true`（进度 51 ≥ `build_points` 15，卡在 `ion_drive` 要的**氦-3**：中国库存 0）⇒ 建造区那一行出现 `买不起 ⇒ 未下水（进度在攒）` |
| 7 | 把图的舰级改成 `destroyer` → `应用` | `{"blueprints":[{"name":"侦察护卫","class":"destroyer"}]}` | **红框**：`blueprint_class_mismatch · 中国.blueprints[0].class` + 引擎原文（"长三角 / building=3 产的是 corvette…要么…要么…"）；状态行 `已应用 0 处，但有 1 处被引擎拒了`；状态**一个字节没动**（再读面仍是 corvette） |
| 8 | `删掉这张图` + `取消删除` 各点一次 → `应用` | 见 §8.2 ④ | 回执 `删掉了 1 片叶：中国.blueprints[0]`；建造区那一行变成 **`侦察护卫（库里没有这张图 ⇒ 本区停产）`**，并列出两条出路 |
| 9 | 推进 5 回合（悬空指针） | — | `长三角` 的 corvette 进度 **51.0 → 51.0（一动不动）** ⇒ Q10(a)「本区停产」在界面上看得到；12 艘已造好的舰不受影响 |
| 10 | 重新建一张**同名**图（意图 = 停泊轨道 → 水星）→ `应用` | `order:{"Dock":{"body":"水星"}}`，`mode:Player` | 指针**自动复活**（同名即修复），`ship_count` 立刻读到 2；推进 1 回合 ⇒ 2 艘下水、进度 51→30（本区恢复生产） |
| 11 | 意图改回 `不表态` → `应用` | 见 §8.2 ③ | 读面 `order: null`（"回到沉默"） |
| 12 | 建一张 `归属=自动` 的图 + 在同一份 diff 里把船坞舰型改成 corvette（见 §8.2 ⑥） | ⑥ | 落地干净；再推进 5 回合，`auto:护卫` 仍是 `Auto`、`components: []`、与船坞同级（`corvette`） |

**买不起那一格的数值证据**（第 6 步）：中国库存 `碳 1.68 / 铁 293 / 氦-3 0`，而 `kinetic` 要
`碳 2 + 铁 4`、`ion_drive` 要 `氦-3 1.5 + 铁 1` ⇒ 卡在**碳**上（不是"看起来卡住"）。
最后一轮的 `shot-09` 用了更硬的一组：`plasma`（氦-3 3 + 金 1.5）＋ `shield`（氦-3 1 + 铂 2），
中国这三样全是 0 上下，进度 45.7 ≥ 15 ⇒ 标记常亮。

### 8.5 两把量具（web 读面 vs CLI 读面）

* **同源**：界面重建世界用的就是**种子 42**（`/api/new`），与 CLI `--seed 42` 同一个世界。
* **同一份 diff**：把界面发的补丁（§8.2 ①）原样喂给 `planet_x --seed 42 --apply diff.json --control`。
* **逐势力比对**：`web /api/state` 的 `control` 段与 CLI `--control` 的 `control` 段，
  9 个势力**逐个字节相同**（`jq -S -c` 规范键序后比 sha256：中国 `67255B1F…`、俄罗斯 `864CC72C…`、
  无国界科学组织 `BB569700…`、星系矿业 `5ED44203…`、欧盟 `765D67F3…`、深空运输联盟 `FC76FC6A…`、
  美国 `F3A6CAEB…`、联合国 `8358667D…`、行星X崇拜教 `48F64A90…`），`scope` 段也相同。
  ⚠ 键序不同（CLI 走 `to_value` ⇒ 键有序；web 走 `Json<StateView>` ⇒ 结构体字段序），所以先规范再比。
* **另外 8 个势力逐字节没动**：上述 8 个哈希**与改动前的基线完全相同**（同一组哈希在
  "建图前后"两次比对里都成立）——即"造一张图只动造它的那个势力"。

### 8.6 回归闸门（本分支，2026-10）

* `cargo test --workspace` **全绿**：`planet_x` lib **138**（+1 ignored）、`longhorizon` **6**（+10 ignored）、
  `projection_derived` **4**、`trade_probe` 0（9 ignored）、**`planet_x_web` lib 21（+2 新）**、`planet_x_web` main 2。
* **行为中性**：`--seed 42 --round 240 --digest 20`（12 行 `^\{`、`\n` 连接、UTF-8 无 BOM）的 sha256 =
  `293725C43A0E26DC516977C04A5BD9C99977252B8C08D683EC2EA2749ADEDBC4`，与改动前**逐字相同**。
* `--control`（种子 42，开局零张图）**逐字节不变**（`AB14895A…5FE6`）——新派生列只在**真有图**时出现。
* 新增测试两条：读面带 `launch_waiting` 且"进度满 + 买不起 ⇒ true、补钱后下水 ⇒ false"、整面模板回传不炸写面；
  `POST /api/command` 的响应里有 `skipped[0].code == "blueprint_class_mismatch"`（且 `/api/state` 的响应**不带** `report`）。

### 8.7 截图（`scratch/`，gitignored；**每张都带左侧面板**）

| 文件 | 内容 |
| --- | --- |
| `shot-02-new-blueprint-in-edit-surface.png` | 库显示「1 张」、刚新建的图行 + 行内编辑器（组件 2/2 槽、多的被禁用） |
| `shot-03-library-after-apply.png` | 应用成功后：`0 艘…挂在 1 个建造区`、底部回执 `✓ 全部落地：2 条` |
| `shot-04-launch-waiting-yard-row.png` | 建造区那一行的 **`买不起 ⇒ 未下水（进度在攒）`** 琥珀牌 |
| `shot-05-guard-error-surface.png` | **红框回执**：`blueprint_class_mismatch` + 引擎原文 |
| `shot-06-dangling-pointer-stopped.png` | 删图后：`侦察护卫（库里没有这张图 ⇒ 本区停产）` + 行上两条出路 |
| `shot-07-waiting-hint-in-library.png` | 图行上的 `4 艘（本图造过）` + 编辑器里的"买不起"提示 |
| `shot-08-library-two-figures.png` | 一张 `玩家` + 一张 `自动` 的库，两种归属的措辞不同 |
| `shot-09-launch-waiting-final.png` | **最终代码**重跑一遍「买不起」：一行 `侦察护卫（Player）`→`买不起 ⇒ 未下水`，另一行 `auto:护卫（Auto）`（同一个面板里两种指针状态） |

### 8.8 顺带修掉的三件真事 / 记下的边角

1. **新建表单被重画清空**（实机撞到）：勾一个组件原来会 `renderTree()`，把**只活在 DOM 里**的图名
   悄悄清掉（"我填了名字，点两下组件，名字没了"）。改成 `componentPicker(leaf,{inPlace:true})`
   ——勾选只就地更新计数/禁用态/告警。
2. **模板常量被共享数组污染**（同上一次实机暴露）：模块级 `BP_TEMPLATE.components` 是**一只数组**，
   每个新建表单都往里 `push` ⇒ 上次勾的组件出现在下一张表单里；更糟的是**壳的原值**也指向它
   （`shellLeaf` 原来是浅拷贝），叶里一改原值跟着变 ⇒ 补丁"看起来没改过"而**发不出去**。
   现在：模板走工厂函数 `bpDraft(name)`（每次都新对象新数组）、`shellLeaf` 用 `structuredClone`。
3. **换指针后行上状态不刷新**：下拉的 `change` 原来只 `renderDiff()`（防重画把选择冲掉），于是
   刚选的图不会触发"舰级对不上/悬空"的告警。改成把这三块收进一个 `yardStatusBox`，换图时
   **就地整块替换**（选择按"下拉里现在选中的名字"算，而不是读面那个旧值）。
4. `+ 新建设计图` 的标题原来用 `.tnode-label`（`flex:1`），在 430px 的面板里被挤成**一列一个字** ⇒
   换成 `.bp-add-title{flex:0 0 100%}`。
5. **引擎回执的一点噪声**（既有行为，本轮没动）：只写值不写 `mode` 时 `write_mode_leaf` 无条件记
   `took_over`——**即使那片叶本来就是 `Player`**。界面照实显示，所以"改图选装"这一步会看到一句
   `隐含接管 1 片叶`。要静音得动引擎（写面回执），本轮不做。
6. **`launch_waiting` 的语义是"回合末进度仍然 ≥ `build_points`"**，不是"直接查库存"。所以
   **手工/旁路**造出"进度满"的时刻（例如刚把悬空指针接回来）它也会短暂为真，下一回合正常出厂即消失。
   界面按引擎的列显示（真值唯一），措辞用「买不起 ⇒ 未下水」是因为在正常推进下这两者等价。
7. **`Auto` 的措辞按代码校准**：`retool_shipyards` 对 `Auto` 图只写 **`class`**（选装保持空 = 出厂时
   `choose_loadout` 现算），所以界面上写的是"重估**舰级**"，不写"重算选装"（那会是空头承诺）。
   ⚠ **未实测**：本轮没有观察到 AI 真的改写一张 `Auto` 图（它改船坞 `ship_type` 是实测到了的——
   我这边 `水星熔炉基地/21` 在推进中被 AI 从 corvette 改成 cruiser，而这条正是"图的 `class` 必须
   跟着走"的那条路径）。

### 8.9 未做（明确留给下一轮）

* **改名**：图名是唯一 key，改名 = 删旧建新（界面不做"改名"按钮，直接新建 + 删旧的；`yardCountText`
  会把"哪些区会悬空"列出来）。
* **AI 主动建图**：`Auto` 图的执行者仍然只有"重估已有图"这一半，界面不承诺更多。
* **refit**：改图对**已下水的舰**无效（快照），界面上只写清楚，不提供"套用到老舰"。
* **组件的造价/槽位余量预览**：编辑器只显示 `n/slots` 与中文名，不显示单件造价（图里不带数字，
  `config` 是唯一真值源；要看造价仍走 `--meta` / 右侧状态面板的 `config` 根）。
* **`order` 的两轴叶（`{"temper":…}` 那种）**：意图只支持 `ShipBehavior` 的六种类型（与"指令"
  编辑器同形）；图里**没有**风格字段（Q6 的裁决：图里不带面板修正）。
