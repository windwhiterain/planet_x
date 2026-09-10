# 舰船设计图（非控制属性 = 建造单位上的模板）

> 状态 `[~]`（设计已定 + 实现规格已就绪；**规格 §8 的十条已裁决**，见
> [`ship-blueprint-spec.md`](ship-blueprint-spec.md) §8.0。实现排在 **web 三条 → 两轴叶**之后，
> 并与 `spawned_round` 同一次升 `SCHEMA_VERSION`） ｜ 索引：[notes.md](../notes.md) ｜ 关联：
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
