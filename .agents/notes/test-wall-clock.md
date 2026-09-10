# 测试墙钟：热点清单与待办（P0 已落地，P1–P3 未做）

> 状态：**P0 已在本分支（`feature/test-perf`）落地**；P1 / P2 / P3 **只是清单，一行代码都没动**。
> 相关：[`test-tiers.md`](test-tiers.md)（分档本身已经把「内循环 4 s」做出来了——这篇管的是
> **全档/长局本身跑多久**）、[`code-layout.md`](code-layout.md)（同一次重构里的文件拆分）。

## 0. 这一轮做了什么（P0）

`Cargo.toml` 增加 `[profile.test] opt-level = 2`。理由：`cargo test` / `cargo nextest run`
用的是 **test 档，默认继承 `dev` 的 `opt-level = 0`**，而引擎是「字符串键 BTreeMap + 大量
小函数」的形态——这种代码未优化时慢一个数量级。只动 `opt-level`，`debug-assertions` /
`overflow-checks` 照旧开着。

**行为中性**：Rust 无 fast-math，f64 仍是严格 IEEE ⇒ 同 seed 的 `--digest` 必须逐字不变。
**未验证**（用户本轮禁止跑测试）：本分支的实测墙钟与 digest 对账都还欠着——见 §4。

## 1. 诊断方法（可复现）

**没有跑任何测试**，是**读代码**找出来的：判据是「这个量是不是每回合被重复算了多次」，
以及「内层循环里有没有 O(实体) 的现算」。两条都指向同一类错误：**纯函数被反复调用**。

## 2. 热点（带 `文件:行号`，按推断的收益排序）

| # | 热点 | 证据 | 每回合重复次数（推断） |
| --- | --- | --- | --- |
| 1 | `sanctioned_hegemon` → `dominant_hegemon` → `faction_power_share`：每次都会给**每艘舰**现算 `ship_panel` | `src/sim/power.rs:31,42`（`ship_panel` 在势力×舰的双层循环里）；调用点：`src/sim/market.rs:214` 的**逐挂单**过滤（`trade_blocked` → `trade_block_cause` 的 `market.rs:71`）、`src/sim/metrics.rs:63-67` 的逐势力×逐势力计数 | 数百次 |
| 2 | `coalition_war_focus` 逐势力各算一次「谁是霸权」 | `src/sim/military.rs:28-32` → `src/sim/power.rs:146`（内部又 `faction_power_share`） | 势力数次 |
| 3 | `round_metrics` 内部 `faction_power_share` 被算 **3 遍** | `src/sim/metrics.rs:11`（`balance_picture`）+ `:12`（`active_coalition_hegemon`）+ `:38`（`sanctioned_hegemon`，各自再走 `dominant_hegemon`） | 3 次 |
| 4 | `threat_motive` 逐势力各算一次全量实力占比 | `src/autocontrol/shipbuilding.rs:115`（AI 每回合建图/改装都调它） | 势力数次 |
| 5 | 索敌内层循环：每发武器扫**全舰队**，且先查 `hostile`（势力线性扫 + BTreeMap）再算距离 | `src/autocontrol/tactics.rs:112-128`、`:145-165`、`:282-301` | 舰 × 武器发数 |
| 6 | 每个候选目标都重算 `ship_weapons()` + `attack_hist.clone()`（堆分配） | `src/autocontrol/tactics.rs:133-134`（在候选循环**内**） | 候选数 |
| 7 | `doctrine_weight` 每个候选都 `ship_doctrine(String 克隆)` + 两次 `deterrence()`（后者自己 O(舰队) × `ship_power`） | `src/autocontrol/tactics.rs:95-99` → `src/sim/ships.rs:100-113` | 候选数 |
| 8 | `State::ship/city/faction` 全是**名字线性扫**，在循环里被反复调用 | `src/model/state.rs:117,127,137,146` | 到处都是 |
| 9 | 长局用例每回合**再调一次** `round_metrics`（把最贵那块翻倍） | `tests/horizon_long.rs:406`、`:465`（`top_power`） | 2 次/回合 |

## 3. 待办（P1 → P3，都还没做）

**P1 = 纯去重（值相同 ⇒ digest 逐字不变，属「纯搬运」级改动）**：

1. `round_metrics`：`faction_power_share` / `sanctioned_hegemon` / `war_pairs` 各只算一次，
   下传给内部使用者（`balance_picture`、`dominant_hegemon` 需要「收一份份额进来」的变体）。
2. `step_market`：把 `sanctioned_hegemon` 提到买家循环**之前**——该步进期间 state 不变，
   值恒等，所以安全。
3. `step_military`：把 `active_coalition_hegemon` 提到 `focus_of` 之前，逐势力只查表。
4. `threat_motive`：份额由调用方传入（调用方本来就有一份）。
5. 索敌：把 `ship_weapons` / `hist` / `deterrence` / `ship_doctrine` 提到候选循环**外**；
   把**廉价的 `dist` 先判**、再查 `hostile`（两个判据都是纯函数 ⇒ 结果不变）。
6. `State` 加「名字 → 下标」索引（增删实体时维护），或把 `&Ship` 直接传进内层。

**P2 = 需要先量再动**（收益不确定，风险更高）：

* 整轮级缓存（要 state 版本号/脏标记，否则会在「同回合中途 state 变了」处出错）；
* 索敌的空间索引（按天体/距离桶预筛）；
* `deterrence` 的邻域缓存。

**P3 = 测试侧**：`coalition_mechanism_is_alive` / `world_is_multipolar` /
`test_power_statistic_matches_game_logic` 改读 `advance` 返回的 `derived.metrics`，省掉第二次
`round_metrics`。断言不变——它们读的 `hegemon` / `coalition_members` / `sanctioned` 与 `flow`
无关（`flow` 只影响产出/维护/治理那些字段）。

## 4. 欠的验证（谁接手都先做这个）

1. **digest 对账**：`cargo run --release -- --seed 42 --round 240 --digest 20`，取 `^{` 行、
   `\n` 连接、UTF-8 无 BOM，SHA-256 必须 = `657F2DC97901BD612E6F784B97FA10A73EC677C7C4AEBD4B1F17179723576665`
   （`main` = `98c4b70` 的基线，见 [`notes.md`](../notes.md) 末节）。
2. **墙钟重测**：`cargo nextest run`（快档）/ `-P mid` / `-P full` 三档的实际秒数，
   回填 [`test-tiers.md`](test-tiers.md) §2 的表——**那张表现在是 P0 之前的口径**。
3. **P1 每一条做完都要过上面两条**（P1 声明是「纯去重」，digest 变了就是判断错了）。
