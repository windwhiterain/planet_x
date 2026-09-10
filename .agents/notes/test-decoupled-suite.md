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
| `play/tests/g1_contract.py` | 快组（≤48 回合，**1.8 s**，5 条）：同 seed 逐字节可复现、`--derived` 的 `post` ≡ `--index` 的 `view`、过程量表与视图同源、中性值表没有死路径 |
| `play/tests/g2_mid.py` | 中组（400 回合，**7.6 s** 冷 / 缓存后 ~1 s，4 条）：拆平的城不被旧主同回合复垦（含「拆平数 ≥ 20」防空转）、整局里出现过装组件的活舰 |
| `play/tests/g3_long.py` | 长组（1000 回合 × **7 seed**，**13 s**，10 条）：读面没有非有限的数、不进吸收态、零活城复生、经济有界、建城必须有舰、合纵连横/制裁活着、霸权叙事自洽 |
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

**留在 Rust 的**（`play/tests/README.md` 与各组 doc 里都写了理由）：纯函数/数学、**手工造世界的
合成场景**（`duel(…)`、`fresh_world(…)` 夹具）、sink/中间量级契约、错误路径（`Result`/`migrate`）、
`#[ignore]` 探针、以及**类型层**守卫（读面每个叶子都声明了中性值——那条要走 schemars 的
类型 schema 遍历，Python 侧只做了**反向**的一半：`neutral.fields` 里的路径都得活着）。

**还没搬的**（下一轮照「读面表」逐张来）：`src/tests/sim/horizon_mid.rs` 剩下的三条
（战痕地板 / 编年史 / 编年史参与者）、`tests/control_read_face.rs`、`tests/projection_derived.rs`
里 `decisions`/`market_trades`/`haul_steps`/`round_inputs` 那几张表的逐列对账（g1 现在只对了
`faction_process` 的 9 列）。


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

* **`--index` 没有表过滤**：长组只要 main/events/ships/factions 四份，却要写 169 MB
  （`round_inputs` 25 MB + `control` 29 MB 全在里面），7 个种子 ≈ **1.2 GB** 缓存。
  加一个 `--index-tables main,events,ships,factions` 能把时间和磁盘都砍到 ~1/3——但它是
  **读面**的改动（缺表的投影会让 `q.load()` 报错），**要不要做请用户裁决**。
* `g1` 的对账现在只覆盖 `faction_process` 的 9 列；`decisions` / `market_trades` /
  `haul_steps` / `round_inputs` 的逐列对账仍在 `tests/projection_derived.rs`（下一轮按表搬）。
* 「读面每个叶子都声明了中性值」**反向那一半**仍在 Rust（要走 schemars 类型 schema）。
* 要不要把 `python play/tests/run.py all` 写进合流门（`AGENTS.md` 的验证约定）——现在是
  各组自己绿，**没人替它把关**。这一条要用户点头才改 `AGENTS.md`。
