# 测试墙钟：热点清单与待办（**§0.2/§0.3 已落地**，P1–P3 未做）

> 状态：P0（`[profile.test] opt-level = 2`）**已回退**——它当初的依据（长局住在 Rust 里）
> 随测试解耦消失了，而它自己变成了编译时间的大头；回退的实测与「其实 O1 才是甜点」见 §0.2。
> **2026-10 已把 §0.2 的甜点与 §0.3 的合并做掉**（用户裁决）：
> `[profile.test] opt-level = 1 + debug = 1`，`tests/` 四个探针合成一个 `probes`。
> **实测一轮门 23 s → 7–8 s**（真跑 ~10–12 s → **2.2 s**）。P1 / P2 / P3 **仍只是清单**。相关：[`test-tiers.md`](test-tiers.md)（分档本身已经把「内循环 4 s」做出来了——
> 这篇管的是**全档/长局本身跑多久**）、[`code-layout.md`](code-layout.md)（同一次重构里的文件拆分）、
> [`test-decoupled-suite.md`](test-decoupled-suite.md)（长局搬去 Python 的那一轮 = P0 依据消失的原因）。

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

### 0.2 回退（2026-10）：依据没了，而且它自己变成了大头

**依据消失**：长局/中档用例搬到 `play/tests/`（数据级断言，跑在 **release** 二进制 + 投影缓存上，
见 [`test-decoupled-suite.md`](test-decoupled-suite.md)）。Rust 侧只剩 **222 条快档单测**，
提档省下的「模拟机器码时间」从全档的 92% 掉到几秒。用户裁决：*「test 不要 opt」*。

**实测（同一台机器，`-p planet_x`，222 条）**——注意区分**冷/切档**与**增量**，这一步很关键：

| test 档 | 冷/切档首次 | 增量编译（改一个库文件） | 真跑 | **一轮门（增量）** |
| --- | --- | --- | --- | --- |
| `opt-level = 2`（原状） | **89.2 s** | 10.5 s | **5.3 s** | **15.8 s** |
| **`opt-level = 1` + `debug = 1`** | 69.9 s | **8.5 s** | **4.7 s** | **13.2 s** ← 甜点 |
| `opt-level = 0` + `debug = 1` | — | 7.6 s | ~16 s | ~23.6 s |
| **`opt-level = 0`（现在的选择）** | **28.9 s** | 12.1 s（另一次 9.5 s） | **16.5 s** | **~28.6 s** |

* ⚠ **更正一个我自己先写错的判断**：我曾说「Rust 门 62 s 里 58 s 是 test 档编译」——
  那个 58 s 是**冷建**（那次 target 里从没建过 test 档），**不是**改一个文件的增量。
  增量的编译只有 ~10 s。所以「编译是大头」这句只在**冷/切档/换 worktree** 时成立。
* ⇒ 于是 `opt-level = 0` 的账要算清：**冷建快 3×**（89 → 29 s），但**稳态每轮门慢 ~13 s**
  （真跑 5.3 → 16.5 s，编译几乎没变——因为那 10 s 里主要是**链接 6 个测试二进制**，
  与优化档位关系不大）。
* `opt-level = 1` 是两头都不亏的那个：真跑与 O2 相同（4.7 vs 5.3 s，222/222 绿），
  编译比 O2 还快一点 ⇒ **一轮门 13.2 s，比 O0 快一倍多**。
  **要不要改 O1 请用户裁决**（当前文件里是用户要的 O0）。

### 0.3 「每个测试一个二进制」的账（用户提的怀疑，量了：一半对）

拆开量（`opt-level = 0`，改一个库文件之后，每个目标单独加）：

| 目标 | 增量耗时 |
| --- | --- |
| lib + lib 测试二进制（`cargo test --no-run --lib`） | 4.5 s |
| 第一个集成测试二进制（`--test trade_probe`，含为集成目标重建 rlib） | 7.3 s |
| 之后每多一个集成测试二进制（`tech_probe` / `site_supply_probe` / `horizon_long`） | **1.0–1.3 s** |
| 空的 `horizon_mid`（没有用例，只剩一个编译单元 + 一次链接） | **0.4 s** |

⇒ `tests/` 下 5 个集成二进制合计 ~5–6 s，**约占每次增量编译的一半**（另一半是 lib 本身）。

### 0.3.1 落地（2026-10）：合并完成，与 §0.2 一起把一轮门从 23 s 压到 7–8 s

* **一个二进制**：`tests/probes/main.rs` + `{horizon_long,site_supply,tech,trade}.rs`
  （`git mv`，内容未动）；空的 `horizon_mid.rs` **删掉**（它想表达的「Rust 侧中档空了」
  搬进 `probes/main.rs` 的模块头 + `.config/nextest.toml` 的注释）。
* **跑法变了**（以前 `cargo test --test trade_probe -- --ignored --nocapture`）：
  `cargo nextest run -P full --run-ignored all`（全部 31 条）／
  `-E 'test(/^trade::/)'`（只跑一份）。⚠ 用例名是 `<模块>::<函数>`（`trade::probe_landless`），
  **不带** `probes` 前缀——那是**二进制 id**（`planet_x::probes`）。
* ⚠ `.config/nextest.toml` 里的 `binary(horizon_mid)` / `binary(horizon_long)` **必须删**：
  nextest 对「匹配不到任何二进制」的 `binary(...)` 是**报错**（不是警告）。档位一律按模块名认。
* 顺带查出一件事：**Rust 侧的 T2/T3 现在都是空的**——`probes/horizon_long.rs` 那 7 条全是
  `#[ignore]` 探针 ⇒ 默认档 / `-P mid` / `-P full` 选中的是**同一批 208 条**（实测三档都是 208）。
* 实测（同一协议：`touch src/sim/mod.rs` 后整跑 `-P full`）：

| | 一轮门 | 真跑 |
| --- | --- | --- |
| O0 + 5 个二进制（原来） | **23 s** | ~10–12 s |
| O1 + 一个 `probes`（现在） | **7–8 s** | **2.2 s** |

  一次性代价：切档要按新档重编一次（本机 ~81 s）。**行为中性**：release 档没动 ⇒
  digest 逐字不变（实测）。

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
