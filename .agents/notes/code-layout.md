# 代码布局：大文件拆小 + 单测搬出源码

> 状态：`feature/refactor-modules` 已落地（纯搬运，行为零变化，digest 逐字节不变）。
> 相关：[`test-tiers.md`](test-tiers.md)（测试按模拟时间分档，与本篇同一次重构）。

## 1. 为什么

`src/sim.rs` 6341 行、`src/control.rs` 3863 行，两个文件占全仓 Rust 的 33%。它们
**同时**装着生产代码和几万字的单元测试：

| 文件 | 拆前 | 其中测试 | 拆后（生产代码） |
| --- | --- | --- | --- |
| `src/sim.rs` → `src/sim/` | 6341 | 2590（53 个用例） | `mod.rs` **170 行** + 17 个子模块 |
| `src/control.rs` → `src/control/` | 3863 | 1444（33 个用例） | `mod.rs` **85 行** + 8 个子模块 |
| `src/projection.rs` | 1859 | 837 | 1063 |
| `src/autocontrol/freight.rs` | 1497 | 862 | 669 |
| `web/src/lib.rs` | 1284 | 839 | 563 |

拆完的直接收益：**改一处战斗逻辑不用滚过 2500 行测试**；两个人同时改生产代码和测试
不再必然撞同一个文件；`sim/mod.rs` 里那条回合流水线（`advance`）第一次变得可读——
它旁边不再堆着 6000 行。

## 2. `src/sim/` 地图（一个子模块 = 一个主题）

| 子模块 | 装什么 | 行数 |
| --- | --- | --- |
| `mod.rs` | `advance`（**回合编排**：生产→承包→维护→市场→建造→军事→治理→迁都→外交→均势→事件跃迁→剧情→思潮→历史裁剪）+ `derived_from_state` + 子模块声明与 `pub use` 兜路径 | 170 |
| `production.rs` | 矿产出、货栈、劳动比、投资/建造权重、维护费、建造预算扣减 | 275 |
| `market.rs` | 价格、封锁、挂单/成交、雇佣运力市场、供需撮合 | 439 |
| `construction.rs` | 开工结算、造舰、设计图下水门控 | 449 |
| `military.rs` | 舰队行为与移动、战斗结算（命中/护盾/点防/本土防御）、轰炸、殖民 | 525 |
| `haul.rs` | 集货：配额 → 抽签派单 → 装卸分录（货权守恒） | 280 |
| `governance.rs` | 治理成本、人均面积、忠诚度、思潮扣分、城市叛变 | 315 |
| `ideology.rs` | 思潮相似度/距离、驱动量（战争得失/MOND/经济）、每回合演化 | 145 |
| `capital.rs` | 首都：亡城强迁 + 周期性 AI 评估 + 迁都代价 | 140 |
| `relations.rs` | 关系与外交：宣战/停战阈、好感调整、战痕地板、`step_diplomacy` | 211 |
| `power.rs` | 权力/霸权/联盟/制裁与均势外交 | 252 |
| `metrics.rs` | `balance_picture` / `round_metrics`（单一权威观测） | 128 |
| `story.rs` | 剧情编年史：叙事弧、触发条件、参与者 | 164 |
| `cities.rs` | 城市生命周期：立城/复垦/夷平/易主接线 + 殖民地建筑种子 | 240 |
| `ships.rs` | 下水与蓝图装载、行为合法性、行为目的地、单舰战力与威慑 | 170 |
| `events.rs` | `ev` / `kill_ship` / 清尸 + 由事件历史推导的军事信号 | 130 |
| `geometry.rs` | 位置/距离/趋近等纯几何 | 73 |
| `mond.rs` | MOND 异常导航与**派生掷骰**（`(势力,舰名,回合,用途)` 哈希，不消费主 `Prng`） | 136 |

`src/control/` 同理：`wire`(673，线上类型) `view`(242，读面) `apply`(314，写面入口+叶子原语)
`ship`(467) `budget`(209) `building`(392) `blueprint`(211) `normalize`(111)。

## 3. 拆分手法（含一次实测到的坑）

* **脚本搬运 + 逐字节自校验**：`scratch/split_sim.py` 那套做法——把「块拼接回去 == 原文
  代码区」当作硬断言，不成立就**不写任何文件**。第一次跑就被它挡下两处边界错位（少拼一层
  `\n`）。**要再拆别的文件就照抄这个套路**，别手工剪。
* **子模块一律 `use super::*;`，`mod.rs` 里 `pub use <子模块>::*;` 兜住原有路径**
  ⇒ `crate::sim::advance` 这些对外路径一字未变，web / projection / agent / 集成测试全不用改。
* ⚠ **`pub use m::*` 会静默过滤掉 `pub(crate)` 项**（实测：调用点报的是
  `E0425 cannot find function`，而**不是**可见性错误——很容易误判成"名字拼错"）。
  所以子模块里的内部项统一放宽到 `pub`（本仓不对外提供库，不存在 API 兼容包袱），
  顺带免掉「以后在文件之间挪函数还要修可见性链」的摩擦。
* `impl` 块里的方法**也要**一起放宽：块内私有方法跨模块后是 `E0624 method is private`。

**验收**：`cargo run --bin planet_x -- --seed 42 --round 240 --digest 20`（取 `^{` 行、
`\n` 连接、UTF-8 无 BOM、SHA-256）= `657F2DC97901BD612E6F784B97FA10A73EC677C7C4AEBD4B1F17179723576665`
（12 行），与拆分**前**逐字节相同。任何一次搬运都该这么验——纯搬运就该零行为差异。

## 4. 单测为什么在 `src/tests/` 而不是 `tests/`

`tests/` 是**集成测试**：只能摸公开 API。这批单测要调 `mond_drift` / `hit_factor` /
`military_deltas` / `war_scar_floor` 这类内部函数，搬过去就得把它们**公开成永久 API**，
还要给每个测试二进制各链一次 crate。

所以用了这个组合：

```rust
// src/sim/mod.rs 里只剩这一句
#[cfg(test)]
#[path = "../tests/sim/mod.rs"]
mod tests;
```

测试模块在**模块树里仍然是 `sim` 的子模块**（只是物理文件搬到 `src/tests/`）⇒
`use super::*;`、私有夹具、私有函数访问**原样可用，零可见性放宽**；`src/tests/` 又
真的做到「一眼看过去全是测试」。

布局（**测试文件本身也按主题拆开**，最大的现在是 829 行，没有 2000 行以上的文件）：

```
src/tests/
  sim/mod.rs            夹具（fresh_world / attach_blueprint / spawn_at / stock）+ 零散用例
  sim/{fleet,combat,haul,mond,ideology,capital,story,blueprints}.rs   按主题
  sim/horizon_mid.rs    中档（T2）用例                    ← 见 test-tiers.md
  control/mod.rs        夹具（some_building / pin_blueprint）
  control/{apply,normalize,view,ship,blueprint}.rs
  projection/mod.rs
  autocontrol/{freight,contract,shipbuilding,tactics,blueprints,style,economy}.rs
  model/{state,contract,market}.rs
  config.rs agent.rs json.rs prng.rs
web/src/tests.rs        （web 是另一个 crate，同理 #[path] 引入）
```

规矩：**主题文件放短档用例，档位文件（`horizon_mid`/`horizon_long`）放高档用例**——
一个用例只属于一个文件，两者不混。子模块写 `use super::*;` 就能拿到 `mod.rs` 里的夹具
（父模块的私有项对子模块可见），所以夹具只写一份。

## 5. 还没拆的（下一轮候选）

按行数：`src/model/event.rs` 1141、`src/model/game_config.rs` 992、`src/world.rs` 865、
`src/main.rs` 711、`src/autocontrol/shipbuilding.rs` 746、`src/projection.rs` 1063、
`src/agent.rs` 375。它们**已经没有内联测试**这个包袱，拆起来是纯体力活；本轮没做是为了
把这次改动的 diff 收在「两个大山 + 测试搬家 + 分档」上，好验证。动手时照 §3 的脚本套路。
