# 测试墙钟：热点清单与待办（P0 已落地，P1–P3 未做）

> 状态：**P0 已落地并实测**（本分支 `feature/test-perf`）；P1 / P2 / P3 **只是清单，一行代码
> 都没动**。相关：[`test-tiers.md`](test-tiers.md)（分档本身已经把「内循环 4 s」做出来了——
> 这篇管的是**全档/长局本身跑多久**）、[`code-layout.md`](code-layout.md)（同一次重构里的文件拆分）。

## 0. 这一轮做了什么（P0）

`Cargo.toml` 增加 `[profile.test] opt-level = 2`。理由：`cargo test` / `cargo nextest run`
用的是 **test 档，默认继承 `dev` 的 `opt-level = 0`**，而引擎是「字符串键 BTreeMap + 大量
小函数」的形态——这种代码未优化时慢一个数量级。只动 `opt-level`，`debug-assertions` /
`overflow-checks` 照旧开着。

### 0.1 实测（同一台机器，A/B 只差这一行）

| 探针 | dev 档（`opt-level = 0`） | O2 档（本分支） | 倍数 |
| --- | --- | --- | --- |
| 快档 `cargo nextest run -p planet_x`（178 条） | **4.395 s** | **2.049 s** | 2.1× |
| 中档 `-P mid`（184 条，含 200 回合复现用例） | 29.7 s（笔记旧值） | **8.015 s** | ~3.7× |
| 单条最重的长局 `coalition_mechanism_is_alive`（1000 回合 × 2 种子） | **77.240 s** | **18.074 s** | **4.27×** |

* 两条 A/B 都跑在**同一份代码**上（只差 `Cargo.toml` 那一行）：基线在主 worktree 的热 target 上跑，
  处理组在 `planet_x-perf` 的 target 上跑。
* 全部用例都**绿**（178/178、184/184；`-P full` 未跑，见 §4）。
* **一次性代价**：档位变了，依赖要按新档位重编一次——本机实测 **1 m 07 s**（之后每轮都省）。
* **行为中性**的两条依据：① `src/` 里**没有任何** `debug_assertions` / `cfg!` / `target_feature`
  分支（grep 过），所以两档之间只有机器码质量不同；② Rust 无 fast-math，f64 仍是严格 IEEE。
  中档里的 `same_seed_reproduces_identically`（200 回合同种子复现）在 O2 下照样过。

## 1. 诊断方法（可复现）

**没有跑任何测试**，是**读代码**找出来的：判据是「这个量是不是每回合被重复算了多次」，
以及「内层循环里有没有 O(实体) 的现算」。两条都指向同一类错误：**纯函数被反复调用**。

## 2. 热点（按推断的收益排序）

> ⚠ 这一节是**当时读代码**留下的诊断快照：`文件:行号` 那批数字此后动过两次（`sim` 大拆分见
> [`code-layout.md`](code-layout.md)；派生读面换代把 `round_metrics` 改名 `observe`，见
> [`pre-post-unify.md`](pre-post-unify.md)）。**行号已删掉**（照不准了，也不编新的）——
> 要动手就按函数名 grep 定位。

| # | 热点 | 证据 | 每回合重复次数（推断） |
| --- | --- | --- | --- |
| 1 | `sanctioned_hegemon` → `dominant_hegemon` → `faction_power_share`：每次都会给**每艘舰**现算 `ship_panel` | `src/sim/power.rs`（`ship_panel` 在势力×舰的双层循环里）；调用点：`src/sim/market.rs` 的**逐挂单**过滤（`trade_blocked` → `trade_block_cause`）、`src/sim/metrics.rs` 的逐势力×逐势力计数 | 数百次 |
| 2 | `coalition_war_focus` 逐势力各算一次「谁是霸权」 | `src/sim/military.rs` → `src/sim/power.rs`（内部又 `faction_power_share`） | 势力数次 |
| 3 | `observe`（当时的 `round_metrics`）内部 `faction_power_share` 被算 **3 遍** | `src/sim/metrics.rs` 的 `balance_picture` + `active_coalition_hegemon` + `sanctioned_hegemon`（各自再走 `dominant_hegemon`） | 3 次 |
| 4 | `threat_motive` 逐势力各算一次全量实力占比 | `src/autocontrol/shipbuilding.rs`（AI 每回合建图/改装都调它） | 势力数次 |
| 5 | 索敌内层循环：每发武器扫**全舰队**，且先查 `hostile`（势力线性扫 + BTreeMap）再算距离 | `src/autocontrol/tactics.rs` | 舰 × 武器发数 |
| 6 | 每个候选目标都重算 `ship_weapons()` + `attack_hist.clone()`（堆分配） | `src/autocontrol/tactics.rs`（在候选循环**内**） | 候选数 |
| 7 | `doctrine_weight` 每个候选都 `ship_doctrine(String 克隆)` + 两次 `deterrence()`（后者自己 O(舰队) × `ship_power`） | `src/autocontrol/tactics.rs` → `src/sim/ships.rs` | 候选数 |
| 8 | `State::ship/city/faction` 全是**名字线性扫**，在循环里被反复调用 | `src/model/state.rs` | 到处都是 |
| 9 | 长局用例每回合**再调一次** `observe`（当时的 `round_metrics`，把最贵那块翻倍） | `tests/horizon_long.rs`（`top_power`） | 2 次/回合 |

## 3. 待办（P1 → P3，都还没做）

**P1 = 纯去重（值相同 ⇒ digest 逐字不变，属「纯搬运」级改动）**：

1. `observe`（当时的 `round_metrics`）：`faction_power_share` / `sanctioned_hegemon` / `war_pairs` 各只算一次，
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
`test_power_statistic_matches_game_logic` 改读 `advance` 返回的 `RoundView`（不再经
`derived.metrics`），省掉第二次 `observe`。断言不变——它们读的 `hegemon` / `coalition_members` /
`sanctioned` 与**过程量**无关（过程量只影响产出/维护/治理那些字段）。

## 4. 还欠的验证

1. **全档 `-P full` 未跑**（本轮只做有界验证：快档 + 中档 + 一条最重的长局）。按 4.27× 推算
   约 25–30 s——**这是推算，不是实测**，谁跑了谁把秒数填进
   [`test-tiers.md`](test-tiers.md) §2 的表。
2. **CLI digest 对账**：`cargo run --release -- --seed 42 --round 240 --digest 20`，取 `^{` 行、
   `\n` 连接、UTF-8 无 BOM，SHA-256 应 = `657F2DC97901BD612E6F784B97FA10A73EC677C7C4AEBD4B1F17179723576665`
   （`main` = `98c4b70` 的基线，见 [`notes.md`](../notes.md) 末节）。P0 只碰**测试档**，CLI 走 dev/
   release 档 ⇒ 按构造不受影响，所以这条不是 P0 的门，而是**P1 的门**（P1 改的是引擎代码本身）。
3. **P1 每一条做完都要过 §4.2 的 digest 对账**（P1 声明是「纯去重」，digest 变了就是判断错了）。
