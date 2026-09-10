# 测试与二进制解耦：共享轨迹 + 数据级断言（**方案，一行代码没动**）

> 状态：**方案**。方向由用户提出（原话）：*「测试其实应该和游戏二进制解耦，直接测跑出来的数据，
> 这样不管多少测试只用编译一遍」*、*「也许可以是 python 测试」*、*「甚至轨迹可以复用」*。
> 相关：[`test-tiers.md`](test-tiers.md)（分档：判据是模拟回合数）、
> [`test-wall-clock.md`](test-wall-clock.md)（P0 的 `opt-level` 实测——本篇的档位讨论接着它）、
> [`step-intermediates.md`](step-intermediates.md)（B1–B5 做出来的 `--index` / `--derived` 读面
> **就是**「跑出来的数据」本体，这套方案是它的自然延伸）。

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

1. **Runner**：pytest（有 session 级 fixture / `tmp_path` / 参数化生态）还是自写脚本？kit 的
   `pyproject.toml` 已经在，pytest 只差一个依赖。
2. **谁能进 Stage 2**：先把 horizon 那一族搬过去（收益最大、最规整），还是按「读面表」逐张搬
   （`faction_process` / `city_process` / `market_trades` / `decisions` / `round_inputs` / `salvos`）？
3. **一条用例 = 一个种子，还是一条 fixture + 多条断言**？后者是这套方案的核心，但会让
   「哪条断言失败」的定位变差 ⇒ 建议断言名里带 `(seed, round)`。
4. **Stage 1 要不要先做**：它不动框架就有 ~2.5× 的全档收益（§3.1 的重复），风险接近零。
