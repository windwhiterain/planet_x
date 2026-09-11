# 测试按「模拟时间」分档（cargo-nextest 分组）

> 状态：`feature/refactor-modules` 已落地。相关：[`code-layout.md`](code-layout.md)（同一次
> 重构里的文件拆分与单测搬家）。
>
> ⚠ **墙钟已换代（`feature/test-perf` 实测）**：以前测试跑在 dev 档（`opt-level = 0`）；该分支
> 加了 `[profile.test] opt-level = 2`（见 [`test-wall-clock.md`](test-wall-clock.md)）。
> **分档判据（模拟回合数）完全不变，只有墙钟变了**：单条最重的长局 **77.2 → 18.1 s（4.27×）**、
> 快档 4.4 → 2.0 s、中档 29.7 → 8.0 s。**全档没重测**（当时只做有界验证），推算约 25–30 s
> ——谁跑了谁把实测填进 §2 的表，别让推算冒充实测。

## 1. 问题：时间全花在 7 条用例上

全档 `cargo test --workspace` 实测 **~110 s**，但分布极端（nightly `-Z unstable-options
--report-time` 逐条量的 CPU-秒，214 个用例共 415 s）：

| 档 | 用例数 | CPU 时间 | 例子 |
| --- | --- | --- | --- |
| ≥10 s | **7** | **380 s（92%）** | `coalition_mechanism_is_alive` 79.5s、`world_value_is_bounded` 71.1s、`a_city_razed_…` 27.9s、`long_run_produces_customized_ships` 18.4s（各 400–3000 回合） |
| 1–10 s | 11 | 28 s | 400 回合的复垦卫兵、120 回合的投影守卫 |
| 0.05–1 s | 24 | 5.5 s | 编年史/战痕（60 回合） |
| <0.05 s | **196** | **1.9 s** | 纯函数与建模用例 |

也就是说：**分档的收益几乎全在把长局挪出默认档**，代价只是「想跑长局时要显式说一声」。

## 2. 档位（判据 = 该用例真正推进的回合数）

| 档 | 模拟时长 | 谁来跑 | 实测 |
| --- | --- | --- | --- |
| **T0** | 不推进回合（纯函数/决策/建模） | 默认 | — |
| **T1** | ≤ 48 回合（4 年内） | `cargo nextest run` | **149 条 / 2.2 s**（另 31 条 `#[ignore]` 探针） |
| **T2** | 49–480 回合（4–40 年） | `cargo nextest run -P mid` | **同一批 180 条**（Rust 侧 T2 已空） |
| **T3** | > 480 回合（40 年+） | `cargo nextest run -P full`（合流门） | **同一批 180 条 / 2.2 s**（Rust 侧 T3 已空） |

判据是**模拟时间**而不是墙上时间：它稳定、可复现、写在用例里看得见，不会因为换了台机器
就换了档。墙钟只用来**发现**谁该进高档（上表就是用它筛出来的）。

## 3. 机制：档位 = 模块名

高档用例必须住在名叫 `horizon_mid` / `horizon_long` 的模块里：

* 单元测试：`src/tests/<区>/horizon_mid.rs` → 用例名带 `sim::tests::horizon_mid::…`
* 集成测试：**模块**就叫这个名字 → `tests/probes/horizon_long.rs`（用例名 `horizon_long::…`）
  ⚠ 2026-10 起 `tests/` 下只有一个二进制 `probes`（4 个探针文件合一）⇒ filterset 里
  **不要再写 `binary(horizon_mid)` / `binary(horizon_long)`**（nextest 对匹配不到任何二进制的
  `binary(...)` 是**报错**，不是警告）。档位一律按模块名认。
  ⚠ **Rust 侧的 T2/T3 现在是空的**：中档/长局判据全搬去了 `play/tests/`，`probes/horizon_long.rs`
  里那 7 条**全是 `#[ignore]` 探针** ⇒ 默认档 / `-P mid` / `-P full` 选中的是**同一批 186 条**
  （149 条断言 + 31 条 `#[ignore]` 探针；2026-10 第 7 批收口时的数）。

`.config/nextest.toml` 里三个 profile 用 filterset 选档：

```toml
[profile.default]
default-filter = 'not (test(/horizon_mid/) | test(/horizon_long/))'
[profile.mid]
default-filter = 'not test(/horizon_long/)'
[profile.full]
default-filter = 'all()'
```

外加两个 **test-group**（`t2-mid` 6 线程 / `t3-long` 5 线程）：长局每条只吃 1 核，限到 5
是为了 `-P full` 时给其余 178 条留核（20 核机器上实测全档 95.8 s，与分档前 `cargo test`
的 110 s 基本持平，但**快档快了 27 倍**）。

**为什么不用 cargo features 或 `PLANET_X_*` 环境变量**：features 每切一次档要重编
（30 s+，内循环反而更慢），环境变量则只能**跳过**用例——而跳过的用例会以 `passed`
出现在摘要里（Rust 稳上没有 "skipped" 这个测试状态），**谎报是比慢更贵的问题**。
nextest 的 filterset 不重编、且没被选中的会老实写成 `N skipped`。

## 4. 命令

```text
cargo nextest run                        # 快档：内循环（T0+T1，~4 s）
cargo nextest run -P mid                 # 中档：+T2（4–40 年）
cargo nextest run -P full                # 全档：合流门前跑这条（= 全部非 ignore 用例）
cargo nextest run -P full --run-ignored all                      # 加上探针/诊断
cargo nextest run -P full --run-ignored all -E 'test(/probe/)'   # 只跑探针
cargo nextest list -E 'binary(horizon_long)'                     # 看某档选中了谁
cargo test --workspace                   # 没装 nextest 时的退路：**仍然是全档**（慢）
```

装 nextest：`cargo install cargo-nextest --locked`，或从 <https://get.nexte.st> 下预编译包
（放到 `~/.cargo/bin`）。

⚠ **老笔记里的 `cargo test --test longhorizon` 已作废**：文件改名为 `tests/horizon_long.rs`，
且「长局 6 passed/10 ignored」那些数字是旧口径。现在等价命令是 `-P mid`（含 200 回合的
`same_seed_reproduces_identically`）或 `-P full`。

## 5. 加用例 / 加档的规矩

1. **新用例先问它推进多少回合**：≤48 写进所属区的普通测试文件；49–480 放该区的
   `horizon_mid.rs`；>480 放 `horizon_long`（单元或集成二进制）。
2. **新档不用改 `.config/nextest.toml`**——放对文件就自动被选中，因为档位在名字里。
   （真要加档才动配置，别忘了 `code-layout.md` §3 那种「命名即约定」的一致性。）
3. **探针/诊断**（只打印不断言）继续用 `#[ignore]`：它们不是回归门，是给人看的观测面。

## 6. 本轮的取舍（留着备用）

* **读面契约用例仍在快档**：`src/tests/projection/horizon_mid.rs` 那类 80–120 回合的
  投影/控制面守卫**没有**算进 T2。理由：它们测的是**读面契约**（投影 = state 的函数、
  每舰一行是不动点），模拟只是取样手段，不是被测对象；而它们合计只值 ~5 s。
  如果哪天快档还想更快，把这 4 条（`every_city_state_change_…` /
  `every_ship_state_change_…` / `no_city_changes_owner_twice_in_one_round` /
  `headline_names_every_participant`）挪进同目录的 `horizon_mid.rs` 即可，配置一行都不用动。
* **`cargo test` 与 `cargo nextest run` 默认档不同**：前者跑全档（110 s），后者跑快档
  （4 s）。这是 profile 筛选的固有差别，别拿 `cargo test` 当内循环。
