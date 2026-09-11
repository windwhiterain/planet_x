# 测试与二进制解耦：数据级断言 + 轨迹复用（**Stage 1′ 已落地**：`play/tests/`）

> 状态：**已落地一部分**（分支 `feature/test-decoupled-suite`）。跑法：
> `uv run --project play/planet_xq python play/tests/run.py all`。
> 方向由用户提出（原话）：*「测试其实应该和游戏二进制解耦，直接测跑出来的数据，
> 这样不管多少测试只用编译一遍」*、*「也许可以是 python 测试」*、*「甚至轨迹可以复用」*，
> 落地形态由用户裁决为 ***「不一定测试框架，就弄几个 python 脚本分组跑就完了」***
> （⇒ 没有 pytest，三个 `g*.py` + 一个 `run.py`）。
> 相关：[`test-tiers.md`](test-tiers.md)（分档：判据是模拟回合数）、
> [`test-wall-clock.md`](test-wall-clock.md)（P0 的 `opt-level` 实测——本篇的档位讨论接着它）、
> [`step-intermediates.md`](step-intermediates.md)（B1–B5 做出来的 `--index` / `--derived` 读面
> **就是**「跑出来的数据」本体，这套方案是它的自然延伸）。
>
> **§4 的 Stage 1 写法（进程内 `OnceLock` 共享轨迹）是错的**，见 §10.1：`cargo nextest`
> **每个用例一个进程**，进程内缓存跨用例不共享 ⇒ 共享只能走**落盘**。落地时改成了
> 「投影落盘 + 摘要落盘」两级缓存（§10.2）。

# 0. 落地了什么（2026-10，`feature/test-decoupled-suite`）

| 东西 | 是什么 |
| --- | --- |
| `play/tests/_harness.py` | 脚手架：定位二进制、**按指纹缓存投影**（`target/test-fixtures/`）、跑世界、抽摘要、记断言 |
| `play/tests/g1_contract.py` | 快组（≤60 回合，**1.9 s**，31 条）：确定性、两个读面逐值一致（过程量 / 判定 / 贸易 / 输入面）、档起点保留过程量、无档自报重算、`--control` 不动点、中性值表没有死路径 |
| `play/tests/g2_mid.py` | 中组（400 回合，**0.9 s** 命中，12 条）：同回合复垦、选装、编年史（节拍/顺序/唯一/参与者）、战争最短回合 = 疤痕承诺 |
| `play/tests/g3_long.py` | 长组（1000 回合 × **7 seed**，**0.9 s** 命中，10 条）：读面没有非有限的数、不进吸收态、零活城复生、经济有界、建城必须有舰、合纵连横/制裁活着、霸权叙事自洽 |
| `play/tests/run.py` | 入口：`1`/`2`/`3`/`all`、`--bin debug|release|<路径>`、`--refresh`、`-j N`、`--list` |
| `play/planet_xq`（改） | `load(dir, only=…)` 只装需要的表（1000 回合的投影全装 12 s，只装 3 张 1.5 s）；**`precise_float=True`**（见 §10.3） |

**从 Rust 侧删掉的用例**（判据一条没丢，阈值一个没动，种子还放宽了）：

| Rust 侧（已删） | 现在住哪儿 |
| --- | --- |
| `tests/horizon_long.rs::no_nonfinite_over_long_run` | `g3_long.py`（前 4 条：有限性 / 吸收态 / 零城复生 / 防空转） |
| `tests/horizon_long.rs::world_value_is_bounded` | `g3_long.py`（世界总价值 < 2e6，`FactionRow.market_value` 求和） |
| `tests/horizon_long.rs::city_founding_requires_a_ship_there` | `g3_long.py`（事件层 join 舰表） |
| `tests/horizon_long.rs::coalition_mechanism_is_alive` | `g3_long.py`（联盟/制裁活着） |
| `tests/horizon_long.rs::test_power_statistic_matches_game_logic` | `g3_long.py`（霸权 = 占比最高者，且联盟/封锁自洽） |
| `tests/horizon_mid.rs::same_seed_reproduces_identically` | `g1_contract.py`（判据更强：整份投影每个文件的 sha256） |
| `src/tests/sim/horizon_mid.rs::a_city_razed_…_this_round` | `g2_mid.py`（同种子 `[1,7,42]` / 400 回合 / 拆平 ≥ 20） |
| `src/tests/sim/horizon_mid.rs::long_run_produces_customized_ships` | `g2_mid.py`（累计口径不变） |
| `tests/projection_derived.rs`（**6 条全搬**） | `g1_contract.py`：`derived_matches_…`（faction/city 过程量表 + control/scope）、`projecting_a_checkpoint_keeps_…flow`、`derived_without_checkpoint_says_…`、`decisions_table_matches_…`、`b3_tables_match_…`、`b5_input_face_matches_…` |
| `tests/control_read_face.rs::every_ship_gets_an_order_row_and_the_template_is_a_fixed_point` | `g1_contract.py::control_fixed_point`（`--control` → `--apply` 回传 → 逐字节相同） |
| `src/tests/sim/horizon_mid.rs::story_chronicle_grows_deterministically` | `g2_mid.py`（节拍清单**改从 `meta.json` 的 `story` 读**，不再写死「prologue 在 1 回合」） |
| `src/tests/sim/horizon_mid.rs::story_participants_are_concrete` | `g2_mid.py`（事件型节拍的参与者；RoundAt 那条改判「非空 + **跨种子逐字相同**」——`meta.json` 不发 beat 的静态 `participants`） |
| `src/tests/sim/horizon_mid.rs::war_scar_floor_…`（**真实长局那一半**） | `g2_mid.py`（`war_started`/`war_ended` 配对算时长 vs 配置算出的最短回合；400 回合 × 3 seed ⇒ 349 场战争，最短 9 = 承诺值）。**形状那一半**留在 `src/tests/sim/war_scar.rs`（要内部函数 + 手工世界；顺带从 `horizon_mid` 改名为 `war_scar` ⇒ 回快档：它不推进回合了） |
| `src/tests/projection/mod.rs::every_city_state_change_is_explained_by_an_event` | `g2_mid.py`（**城的完备性审计**：密集快照里每次归属/存亡变化都要有事件命名这座城）。样本 120 回合 × 1 seed → **400 回合 × 3 seed**，实测 **1933 次变化**全有解释（Rust 版下限只要 5） |
| `src/tests/projection/mod.rs::every_ship_state_change_is_explained_by_an_event` | `g2_mid.py`（舰的出现 = 造舰事件、消失 = 死因事件；实测 **337 出生 / 392 死亡**全有解释） |
| `src/tests/projection/mod.rs::no_city_changes_owner_twice_in_one_round` | `g2_mid.py`（实测 1848 次活城易主，0 次同回合翻转两遍） |
| `src/tests/projection/mod.rs::headline_names_every_participant` | `g2_mid.py`（实测 **32093 个实体**全部逐字出现在 `headline` 里） |
| `src/tests/projection/mod.rs::projection_is_deterministic` / `::event_milestones_is_deterministic` | `g1_contract.py`「同 seed 重跑逐字节一致」（比的是**整份投影每个文件**的 sha256 ⇒ 更强，两条并一条） |

**流程也改了**（用户裁决：*「python 测试的方式改为 build release 加 python 测试」*）：
`run.py` 现在**先按需 `cargo build --release`**（二进制比 `src`/`config` 旧或不存在时才编），
再跑各组 —— 改完 Rust 直接 `uv run --project play/planet_xq python play/tests/run.py all` 即可，
`--no-build` 可跳过。数据级一律走 release 二进制（长组的墙钟由机器码质量决定）。

**顺手丢掉的过时探针**（用户：*「一些过时的测试就丢掉」*）：`probe_world_health`、`probe_sanction`
（都被 `probe_multipolar` 这个升级版取代——同一批指标的更全口径，留两份同源仪器只会有一份开始说谎）、
`probe_zombies`（一次性调试器：打印第一次 ≥3 僵尸势力就 `return`，为当时那次「僵尸夺城—倒戈振荡」
调查写的；全球僵尸数已由 `probe_multipolar` 的 `zombies` 列覆盖）。理由写进了
`tests/horizon_long.rs` 的模块文档。

**留在 Rust 的**（`play/tests/README.md` 与各组 doc 里都写了理由）：纯函数/数学、**手工造世界的
合成场景**（`duel(…)`、`fresh_world(…)`、手工塞 `HistoryEntry` 的疤痕形状）、sink/中间量级契约、
错误路径（`Result`/`migrate`）、`#[ignore]` 探针、以及**类型层**守卫（读面每个叶子都声明了中性值
——那条要走 schemars 的类型 schema 遍历，Python 侧只做了**反向**的一半：`neutral.fields` 里的路径
都得活着）。

**还没搬的**：`tests/probes/` 下只剩探针（29 条 `#[ignore]`，全是「只打印不断言」：
`trade` 13 / `site_supply` 6 / `tech` 3 / `horizon_long` 7）。⚠ 2026-10 起这 4 个文件合成
**一个**二进制 `probes`（`tests/probes/main.rs`；每多一个二进制每轮门白付 1.0–1.3 s 链接），
空的 `horizon_mid.rs` 已删——它想表达的「Rust 侧中档空了」搬进了 `probes/main.rs` 的模块头。
`src/tests/**` 的 224 条是快档单测：纯函数、合成场景、内部契约、错误路径——按 §6 的两栏对账，
**它们没有「跑出来的数据」可测**，所以留在原处。再往后要搬的是**探针**（数据面完全够，而且
缓存之后比在 Rust 里跑快得多）：等哪天真要调平衡时按需搬。


# 1. 为什么：实测成本模型（这台机器，2025 一轮 B5 期间量）

| 环节 | 实测 | 说明 |
| --- | --- | --- |
| 改**一个**文件后，test 档（`opt-level = 2`）重建 | **46.7 s** | 1 个 lib 测试二进制 + 7 个集成测试二进制，**每个都要链一遍整个 crate** |
| 同一次改动的 **release** 档重建 | **105.4 s** | digest 检查若走 `cargo run --release`，每次改动都要重编一遍 |
| 全档 `-P full` **真跑**（热态） | **26.5 s**（b5 分支）/ **25.6 s**（main） | 与分支无关 ⇒ B5 一秒都没多加 |
| 快档（T0+T1，228 条）真跑 | **3.7 s**（O2）/ **19.1 s**（O0） | 见 §2 |
| 同一条「nextest + `cargo run --release`」热态再跑 | **63 s** | 27 → 63 的差**全是机器负载**（当时 msedge 1.5 万秒 CPU、NVIDIA Overlay、krita、Zed、Taskmgr 都在跑） |
| 改一个文件后 test 档（`opt-level = 0`）重建 | **7.6 s** | 内循环反而更快：**27 s vs 51 s 一次迭代** |
| 切档（改 `[profile.test]` 的 opt-level）一次性代价 | **51.6 s** | 档位一变，**依赖**也要按新档位重编一遍 |

**结论**：现在的时间几乎全花在**编译**上，而编译里最大的一块是「**同一份 crate 被链 8 遍**」以及
「改一行就把整份 crate 重编一遍（O2）」。

# 2. 「短档 debug / 长档优化」可行吗？（用户问过，已 A/B 实测）

临时把 `[profile.test]` 从 `opt-level = 2` 改成 `0`，同一个文件、同一台机器：

| 档 | 改一个文件后**编译** | 快档**跑** | 内循环一次合计 |
| --- | --- | --- | --- |
| `opt-level = 2`（现状） | 46.7 s | **3.7 s** | **51 s** |
| `opt-level = 0`（debug） | **7.6 s** | 19.1 s（5×） | **27 s** |

* **内循环：O0 赢**（27 s vs 51 s）——用户直觉对。
* 但**长档必须优化**：P0 笔记实测 debug 全档 ≈ 110 s、O2 全档 27 s（**4×**），而绝对分钟数全在长档。
* ⚠ **`[profile.test]` 的 opt-level 会作用到依赖上**（这是冷 worktree 要等 1–2 分钟的原因）。
* ⇒ 「短档 debug / 长档优化」= 两个 profile 名 + 两条命令（`cargo nextest run` 用 O0 的 `test`、
  `cargo nextest run -P full --cargo-profile test-opt` 用 O2）**能做**，代价是两套 artifact
  （每次切档重建依赖，实测 51.6 s）。
* **但 §4 的 Stage 2 之后这个问题会自动消失**：模拟跑在 **release 二进制**里，pytest 只是读 JSON
  ⇒ test 档可以放心用 O0（7.6 s 重编），长档的「跑得慢」也不再有影响。
  **所以不建议现在做 profile 拆分**——它在错误的一层解决问题。

# 3. 诊断：真正贵的是**重复**，不是「测试多」

1. **四条长局跑的是完全相同的一批世界**（`tests/horizon_long.rs`）：

   | 用例 | 种子 | 回合 |
   | --- | --- | --- |
   | `world_value_is_bounded` | `[1,2,3,4,5,7,11]` | 1000 |
   | `city_founding_requires_a_ship_there` | 同上 | 1000 |
   | `coalition_mechanism_is_alive` | 同上 | 1000 |
   | `test_power_statistic_matches_game_logic` | 同上 | 1000 |
   | （`no_nonfinite_over_long_run` 用 seed 42 / 1000 回合） | 42 | 1000 |

   ⇒ **同一批 2.8 万回合的模拟被重算了四遍**，四条用例只是从同一条轨迹上看不同的不变量。
   这正是用户说的「轨迹可以复用」。
2. **8 个测试二进制**（1 个 lib 测试 + 7 个 `tests/*.rs` 集成测试）**每个都静态链一遍整个 crate**。
3. **release 二进制的成本是线性的、可复用**：`--seed 42 --round 240 --digest 20` 实测 **4 s**
   ⇒ 1000 回合 ≈ 17 s/种子，7 个种子**并行** ≈ 20 s **一次**，之后所有断言都是表扫描。

# 4. 方案（三级，可分别落地、分别回退）

## Stage 1 · 共享轨迹（**只动测试内部**，收益立刻可量）

在现有测试框架里加一个**按 `(seed, 回合数)` 缓存的轨迹**（`OnceLock` / `Mutex` 里只跑一次），
把 §3.1 那四条（以及 mid 档里重叠的）改成**对同一份数据断言**。

* 预期：长档的模拟工作量 **÷4** ⇒ 全档 26.5 s → **~10 s**；不换框架、不碰依赖、不影响用例语义。
* 判据不变（还是断言那四条不变量），只是**世界的来源**从「每条自己跑」换成「共享一份」。
* 这一级**不需要**讨论 pytest/缓存失效——它是纯收益。

## Stage 2 · pytest + 投影缓存（用户提的形态）

* 二进制（release）**编一次**；
* **fixture 工厂**：按 `(二进制哈希, seed, 回合数, 分辨率)` 生成并缓存投影目录
  （`planet_x --seed S --round N --index <cache>/S-N/`，可 `--every` 降采样）；
* 所有「**跑世界 → 看数据**」的用例搬成 **pandas 断言**（`play/planet_xq` 已经是那套读面：
  `q.main()` / `q.derived('faction_process')` / `q.round_inputs()` / `q.salvos()` / `q.neutral()` …）；
* Rust 只留搬不走的（见 §6），并且**不进内循环**（按需跑）。

## Stage 3 · 档位（Stage 2 的副产品）

`[profile.test] opt-level = 0`（内循环重编 7.6 s）+ release 二进制编一次。
**模拟的机器码质量由二进制那一档决定**，测试进程快慢不再重要 ⇒ §2 的取舍自动化解。

# 5. 缓存键与失效（**唯一真危险，必须写死**）

* key = **二进制哈希**（或 mtime+size）+ seed + 回合数 + 分辨率 + 配置哈希（`config/game.ron`）。
* **代码一改，二进制哈希就变 ⇒ 缓存自动失效**——绝不手写「已知过期」的 golden 文件。
* 缓存目录建议放 `target/test-fixtures/`（`cargo clean` 会清掉，正是想要的行为）；
  `.gitignore` 里加一行即可，**不进版本库**。
* 若某天真要提交 golden（例如给 CI 省时间），必须**同时提交它的 digest**，并让守卫比对
  ——`--digest 20` 的 SHA-256 就是现成的机制（`.agents/notes.md` 的基线链）。

# 6. 什么必须留在 Rust（搬不走的）

| 类别 | 例子 | 为什么搬不走 |
| --- | --- | --- |
| 纯函数 / 数学 | `sigmoid`、`eligibility`、`review_chance`、`body_weight`、`mond_drift`、`draw_theme` 的加权 | 没有「跑出来的数据」可测，直接调最快 |
| **手工造世界的合成场景** | `src/tests/sim/shots.rs` 的 `duel(seed, …)`（造两条舰对轰）、`src/tests/autocontrol/*` 里造舰队 | 要绕开正常开局、直接摆出边界局面 |
| sink / 中间量级 | 直接读 `RoundSink`、`view_from_state` 与中性值表的对照 | 是**内部**契约，读面看不到 |
| 错误路径 | `Result` / `migrate` 的档位分支 | 需要构造非法输入 |
| 探针 | `#[ignore]` 的 `*_probe` / `diagnose_*` | 只打印不断言，跑在完整 crate 里 |
| 类型层守卫 | `every_read_face_field_declares_a_neutral`（schemars 双向集合相等） | ⚠ **这条其实可以搬**：`schema.json` 里既有字段集合也有 `neutral` 表 ⇒ Python 侧能做同一个集合相等 |

# 7. 验收判据（用数字，别用感觉）

| 指标 | 现在 | Stage 1 目标 | Stage 2/3 目标 |
| --- | --- | --- | --- |
| 全档 `-P full` 墙钟 | 26.5 s | ≤ 12 s | ≤ 8 s（数据已缓存时 ≤ 4 s） |
| 改一个文件的「内循环一次」（编+跑） | 51 s（O2）/ 27 s（O0） | 27 s | **≤ 12 s** |
| 用例条数 / 判据清单 | 239 条（含 34 skipped） | 不变 | **逐条对账「搬 / 留」两栏，一条不丢** |
| 防线 | digest 基线 + `neutral` 表 + schema | 不变 | 不变（搬家**不许**丢掉它们） |

# 8. 复核方式（量这些数用过的命令）

```bash
# 编译成本（改一个文件后重建）：touch src/lib.rs 然后
cargo nextest run --no-run              # test 档
cargo build --release                   # release 档
# 真跑成本
cargo nextest run -P full --no-fail-fast   # 合流门（26.5 s 热态）
cargo nextest run                          # 快档（3.7 s）
# 不要用 `cargo run --release` 做 digest 检查：直接用已编好的二进制（4 s）
./target/release/planet_x.exe --seed 42 --round 240 --digest 20
```

# 9. 待裁决（动手前先定）

1. **Runner**：~~pytest 还是自写脚本？~~ ⇒ **用户裁决：不用测试框架**，几个 Python 脚本
   分组跑（`g1/g2/g3` + `run.py`）。落地后看：够用——需要的东西只有「一份数据 + 多条断言 +
   非零退出码」，pytest 的 fixture/参数化在这里反而是多余的一层。
2. **谁能进 Stage 2**：~~先搬 horizon 一族还是按读面表逐张搬？~~ ⇒ **先搬 horizon 一族**
   （收益最大、最规整：全档 92% 的时间在它们身上，而且四条跑的是同一批世界）。按表逐张搬
   作为下一轮（§0 末列了还没搬的清单）。
3. **一条用例 = 一个种子，还是一条 fixture + 多条断言？** ⇒ **一份数据 + 多条断言**，
   但**失败信息里带 `(seed, 回合)`**（`Verdict.detail`），并且每条不变量**跨 seed 聚合**
   成一行——定位靠样例串，不靠用例名。
4. **Stage 1 要不要先做？** ⇒ 做了，但**形式必须改**（见 §10.1）：不是进程内共享，而是
   **落盘缓存**。

# 10. 落地时踩到/改掉的四件事（每条都改了一个原本会写错的决定）

## 10.1 `cargo nextest` **每个用例一个进程** ⇒ 进程内共享轨迹是不可能的

本方案 §4 的 Stage 1 写的是「`OnceLock` / `Mutex` 里只跑一次」。那是 `cargo test`（libtest
多线程同进程）的模型；本仓库用的是 **`cargo nextest`，它的设计就是 process-per-test**
（[官方文档：Why process-per-test?](https://nexte.st/docs/design/why-process-per-test/)）。
⇒ 同一个二进制里那四条长局用例活在**四个进程**里，进程内缓存一个都共享不到，Stage 1 按原样
写出来收益≈0。**共享只能跨进程**，所以落地成落盘两级缓存（§10.2）。

## 10.2 落地的缓存是两级：**投影**（真相）+ **摘要**（每回合一行的判据表）

* 一级：`planet_x --seed S --round N --index target/test-fixtures/<指纹>-sS-rN-eE/`。
  指纹 = 二进制 `mtime+size` + `config/game.ron` 的哈希 ⇒ **改代码/改配置自动失效**，
  没有手写 golden（方案 §5 的底线）。
* 二级：`g2`/`g3` 把投影压成**每回合一行的判据表**（活城/舰/总价值/有限性违规/事件/政治），
  也按投影缓存成 pickle。**摘要的名字里带「抽取逻辑的代码指纹」**
  （`_harness._code_stamp`：这个模块里除 `run()` 以外全部顶层函数的源码哈希）⇒
  **改断言（住在 `run()` 里）命中缓存，改抽取逻辑才重算**。这一条是刻意的：不然「加一条
  断言」又要重读 170 MB，而加断言正是这套东西存在的理由。
* 实测（1000 回合，7 seed）：投影 **13.1 s / 169 MB per seed**（第一次），抽取 **~2 s/seed**，
  之后**全部命中**——`g3` 整组 **13 s**（其中约 12 s 是读 7 份投影的摘要；断言本身 <1 s）。

## 10.3 pandas 的默认浮点解析是**不精确**的（1 ULP）

`pd.read_json` 默认走 `ujson` 的快速浮点路径，对需要 16 位有效数字的值会落到**相邻的 double**：
实测 `1.989510667009421` 被读成 `1.9895106670094211`，于是
「`--index` 的过程量表 vs `--derived` 的 `post`」这条逐值相等的守卫**假红**。
引擎那边为了这件事专门开了 serde_json 的 `float_roundtrip`（`Cargo.toml` 里有长注释），
Python 侧再用一个不精确的解析器 = 把那个坑从后门放回来。⇒ kit 的三处 `read_json`
一律加了 **`precise_float=True`**（代价是 1000 回合的主流 0.3 → 0.4 s）。

## 10.4 读面上的 `hegemon` **不是「最强的那个」**

按字面写「霸权 = 占比最高者」会**大面积假红**（实测 seed 1 有 1278 个回合）。真语义在
`sim/power.rs`：`hegemon` = `active_coalition_hegemon` = **占比达标的最强者**（`hegemon_power`）
**且**针对它的疏远成员 ≥ `min_members` —— 联盟还没成形时它就诚实地是 `null`（占比 0.32 > 阈值
0.30 却 `hegemon: null` 是**机制**）。同理联盟成员**可以与霸权开战**（那条判据只管成员**彼此**
不交战）。⇒ 数据级那条守卫写成：**游戏针对谁**（`hegemon`/`sanctioned`，那就是政治机制的输出）
必须**就是**读面统计说谁最强（按**占比**比、不按名字比——并列最强时 Rust 的 `max_by` 取最后一
个、Python 的 `max` 取第一个，按名字比会在并列上假红），联盟成员的资格（够数、疏远、彼此不交战）
在读面上也核。这比 Rust 版（比两个内部函数）**更贴目标**：跨的正是进程边界。

## 10.5 还没做 / 下一轮

* **`--index` 不加表过滤**（用户裁决 2026-10：*「不过滤了，省的后面新测试又要改」*）。
  背景：长组只要 main/events/ships/factions 四份，却要写 169 MB（`round_inputs` 25 MB +
  `control` 29 MB 全在里面），7 个种子 ≈ **1.2 GB** 缓存。加表过滤能把时间和磁盘砍到 ~1/3，
  但代价是「投影少了几张表」会变成一个新的坏档形态（`q.load()` 报错、每条新断言都要先问
  「这张表在这个投影里有吗」）——**省下的时间不值得让后面每个测试都多一层判断**。
* ⚠ **更正**：这条曾经写着「需要时用 `--every K` 降采样即可」——**错的**。实测
  `--seed 42 --round 1000 --index out --every 10` 与全量**一模一样**（169.3 MB / 8.7 s）：
  `--every` 只管 stdout 的轨迹快照（`coarse-trajectory-views.md`），**不动投影**。
  ⇒ 要真减投影：**少几个 seed / 少几回合**，或者给 `--index` 加表过滤（上面已裁决不做）。
  `_harness.py` 里那个 `every` 参数已按这条实测删掉（留着只会让人以为存在「分辨率」这一维）。
* **合流门已写进 [`AGENTS.md`](../../AGENTS.md) 的「验证」一节**（+ `notes.md` 的「验证手段」）：
  数据级 `python play/tests/run.py all` 与 `cargo nextest run -P full` **两条都要绿**。
* `g1` 的对账已经覆盖 `faction_process` / `city_process` / `decisions` / `market_trades` /
  `haul_steps` / `round_inputs` / `control` / `scope`（原 `projection_derived.rs` 六条全搬）；
  还留在 Rust 的是「读面每个叶子都声明了中性值」**反向那一半**（要走 schemars 类型 schema）。
* 未搬的只剩**探针**（`tests/` 下 29 条 `#[ignore]`，只打印不断言）与 `src/tests/**` 的
  224 条快档单测——按 §6 的两栏对账它们没有「跑出来的数据」可测。真要调平衡时，探针更适合
  搬到 Python（缓存之后比在 Rust 里跑快得多），但那要等下一个具体问题。

# 11. 现在的耗时结构（2026-10 合并后实测，同一台机器）

搬完之后，**大头从「真跑」变成了「编译 + 把数据吐出来」**——两者都与「改了多少代码」无关，
而与「要不要付一次冷启动」有关：

| 环节 | 实测 | 说明 |
| --- | --- | --- |
| `cargo nextest run -P full`（Rust 门） | 增量 **~13–16 s**（改一个库文件后：编译 ~10 s + 真跑 4–5 s）；**冷/切档首次 62–89 s** | ⚠ 我先前写的「62 s 里 58 s 是 test 档编译」是**冷建**，不是增量——更正与完整对照见 [`test-wall-clock.md`](test-wall-clock.md) §0.2 |
| `cargo build --release`（增量 / 冷） | 39.5 s / 1 m 48 s | 数据级长组的前置 |
| `cargo build`（debug 增量） | **3.3 s** | 内循环用 |
| 投影 1000 回合（`--index`，169 MB） | **8.5–9.4 s** | 纯模拟 5.9 s ⇒ **投影多花 ~+3.5 s 墙钟 / +1.2 s CPU** |
| 投影 400 回合（79 MB） | 5.5 s | ≈ 线性 |
| g3 冷（7 × 1000 回合 + 抽摘要） | 生成 ~60 s CPU（高负载那次 210 s）+ 抽取 ~15 s | `-j 7` 墙钟 ≈ 50 s（**CPU 争用**，不是磁盘：见 §11.1） |
| g3 命中缓存 | **~1 s** | 读 7 份摘要 pickle |
| g1 快组（release 冷 / 命中） | 3.7 s / 1.8 s | |
| 全组命中缓存 | **4–5 s** | 断言本身已经免费 |

**内循环该怎么走**（实测对比，改一个引擎文件之后）：

| 路线 | 编 + 跑 |
| --- | --- |
| release：`cargo build --release` + `run.py 1` | 39.5 + 3.7 ≈ **44 s** |
| **debug：`cargo build` + `run.py 1 --bin debug`** | 3.3 + 8.9 ≈ **12 s** |
| debug + `run.py all --bin debug`（合流门预演） | 3.3 + ~（g2/g3 在 debug 下 ≈4×）⇒ **仍该用 release** |

⇒ 结论：**内循环用 debug 二进制（快组），合流门用 release（长组的模拟机器码质量决定一切）**。
`test-tiers.md` / `test-wall-clock.md` 里那条「P0 把 test 档提到 O2」现在值得复评：快档真跑
只有 4.2 s，而它换来的是 58 s 的编译——把 `[profile.test] opt-level` 调回 0 也许净赚
（**未实测**，切档要重编依赖 ~50 s）。

## 11.1 「投影为什么贵」的更正：**是构造 JSON，不是写盘**

这条最早写成「投影多花的 2.9 s 全是写盘（I/O）」，**是错的**（用户一问就露了：*「你说 IO 是
问题，那为啥只保存最后的 state 速度没变化呢」*）。重量的口径：

| 模式（seed 42 / 1000 回合） | 墙钟 | CPU | 输出 |
| --- | --- | --- | --- |
| `--digest 1000`（纯模拟，1 行） | 5.9 s | 1.9 s | 0 MB |
| `--index`（投影） | **9.4 s** | 3.1 s | **169 MB** |
| `--save` + 全量 stdout | 6.1 s | 1.2 s | 40.3 MB |
| `--digest 1000 --save`（≈只留最终状态） | **5.7 s** | 1.8 s | **0.09 MB** |
| **裸磁盘基准**：169 MB 顺序写（1 个文件 / 19 个文件） | **0.1 s** | — | 169 MB |

* 这台盘（NVMe Micron 3400）**顺序写 169 MB 只要 0.1 s** ⇒ 磁盘带宽**不是**那 +3.5 s 原因；
  真正的代价是**把 JSON 建出来**（serde_json 序列化 + 逐表构造，CPU 侧那份 +1.2 s），
  外加写出这一堆小文件的开销。⇒ 结论对上一节那条**已裁决不做**的表过滤有影响：
  它省的是「少构造一部分 JSON」，**磁盘那一半本来就不存在**——所以那个决定更站得住。
* **「只存最终状态」现在没有专门的开关**：`--round N` 的合同就是**每回合吐一行**
  （不带 `--index` 时是一整份 agent 视图，1000 回合 ≈ 40 MB），`--save` 只是在收尾多写一个
  **0.09 MB** 的档 ⇒ 单独用 `--save` 看不到任何变化（实测 6.1 s vs 6.1 s）。
  **要最终状态就写 `--round N --digest N --save ckpt.json`**（`--digest` 的窗口 ≥ 总回合数
  ⇒ 只出 1 行摘要），实测 5.7 s / 0.09 MB，与纯模拟同价。
* 对数据级测试的意义：**169 MB 是买来的信息**（每条断言都要逐回合的数），不是浪费；
  要真减只能少 seed / 少回合。**不打算给 `--round` 加 `--quiet`**（用户 2026-10 同意）：
  它只省 ~3 s/1000 回合，而内循环真正贵的是编译。

# 12. 同回合相位错位：把样本放大才撞得到的三类（2026-10 发现，一类已修）

> 这是「搬去 Python + 放大样本」的第一个**非平迁**收获：Rust 版全是「跑一步、看开局那几座城」，
> 永远看不到这些；一放到 **7 seed × 1000 回合的每一行**，它们立刻冒出来（我第一版判据就被弄假红）。

**症状**：同一个回合里，**按城算的量**（`city_process`）与**按势力算的量**（`faction_process`）
在下列三种情况下**必然对不上**。根因同一个：一个回合内有多个步骤，**不同步骤看到的对象不同**，
而两张表都盖着「回合末」的戳。

| # | 情形 | 实测 | 为什么对不上 | 现状 |
| --- | --- | --- | --- | --- |
| 1 | **城本回合易主**（`city_defected` / `city_overrun`） | 2,009 行（7 seed） | `loyalty_target_*` 是**旧主**那时算的（距离按旧主首都、全国项按旧主），而 `faction_process.capital_loyalty_bonus` / `ideology_loyalty_penalty` 是**新主**的 ⇒ 分解式必然不符 | 未修（见 §12.1）；判据**排除**这类并要求「每一处排除都说得清」 |
| 2 | **势力本回合城集合变了** | 640 行 | 治理那一步跑在它这个城集合形成**之前** ⇒ 整行治理量是**缺省 0**（有活城却 0 行政开销）。真样本 seed 5 r212 俄罗斯：先被打光，又在**同一次治理步**里靠别家倒戈拿到 3 座城 | 同上 |
| 3 | **城本回合被复垦**（`colony_founded`） | 5 行 | `is_hub` 抄的是**产出那一步**的判定（旧主：旧主的首都就在那个天体上），城在本回合后半段被夷平+复垦后，行上的 `faction_id` 已是新主 ⇒ **同一势力读出两个 hub 天体** | **已修**（`sim/metrics.rs`：按写这一行时的主人重算 `is_hub`） |

## 12.1 修了哪一条、为什么不修另两条

**修 #3**：`is_hub` 的文档承诺就是「本城天体是不是**本势力**的首都」，而 `capital_body` 是权威值
⇒ 在 `observe()`（造 `CityRow` 的那一处）按**当前主人**重算，比抄产出那一步的旧值更贴承诺，
也不新增第二个位置存同一个数。**验证**：`--seed 42 --round 240 --digest 20` 的 sha256
**逐字节不变**（`BB2EEB2B…`）⇒ 只动读面、不动模拟；seed 5 r276 美国 的 hub 天体从
`{地球, 土星}` 变回 `{地球}`，整个种子 0 例外；g3 那条判据随即**去掉豁免名单**，改成严格形式
（16,387 个「回合·势力」无豁免地只有一个 hub 天体）。

**不修 #1 / #2**：它们不是「抄了旧值」，而是**行本身描述的是另一时刻的对象**——
`city_process` 的 `faction_id` 是回合末的主人，而那一行的忠诚分项/治理量是另一次计算留下的。
把它「修」成自洽只有两条路，都不划算：

* 在 `observe()` 里为新主人**重算**一遍忠诚/治理 ⇒ 等于把引擎的公式抄进读面（同一个数两个位置，
  正是这套投影一直在避免的）；
* 给行加**溯源列**（如 `process_faction_id` / `captured: bool`）⇒ 诚实，但要动 schema + 所有读的人。

⇒ 采用第三条路：**判据排除 + 单独要求每一处排除都能用 §12 解释**（否则排除就是藏违规的后门）。
这条「**排除即断言**」的写法应当成为本仓库写数据级不变量的默认姿势：允许排除，但排除本身也要有判据。

## 12.2 顺手踩到的 pandas 坑（会让「排除」与「未解释」同时为真）

左连接之后的标记列是 **object**（未匹配 = NaN）：`fillna(False)` 之后**仍是 object**，
`~excl` 会退化成整数的按位取反（`~True == -2`，**真值**）⇒ `viol & excl` 与 `viol & ~excl`
**同时为真**，于是「排除 2009 处」和「未解释 2009 处」一起打印（我就这么看了一次假红）。
正解：`j["flag"].notna()`（左连接里匹配上的就是非 NaN），或者 `.astype(bool)` 明确转成布尔列。
