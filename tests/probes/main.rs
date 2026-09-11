//! **观测探针**（全部 `#[ignore]`：只打印、不断言）——**一个二进制**，不再一个文件一个。
//!
//! 为什么合成一个：`tests/` 下的每个文件都是**独立的测试二进制**，各自要静态链一遍整个
//! crate。实测（`test-wall-clock.md` §0.3）第一个二进制的增量是 7.3 s、之后每多一个
//! **1.0–1.3 s**，而这里 4 个探针文件里 **3 个 + 1 个空壳**⇒ 每轮门白付 ~4 s。
//! 探针是**真跑**的（要推进几百上千回合），合起来完全不吃亏；它们本来就只打印。
//!
//! 跑法（**变了**：以前是 `cargo test --test trade_probe -- --ignored --nocapture`）：
//!
//! ```text
//! cargo nextest run -P full --run-ignored all                              # 全部 31 条探针
//! cargo nextest run -P full --run-ignored all -E 'test(/^trade::/)'       # 只跑贸易那一份
//! cargo nextest run -P full --run-ignored all -E 'test(/probe_mond_control/)' --no-capture
//! PROBE_ROUNDS=800 PROBE_SEEDS=7,42 cargo nextest run -P full --run-ignored all -E 'test(/^tech::/)'
//! ```
//!
//! （`-E 'test(/probe/)'` 仍然能选中全部——每条探针的函数名都以 `probe_` 开头。）
//! ⚠ 用例名是 `<模块>::<函数>`（如 `trade::probe_landless`）——**不带** `probes` 前缀，
//! 那个前缀是**二进制 id**（`planet_x::probes`），选择器里要用 `binary(probes)` 才认。
//!
//! ## 档位：**Rust 侧的 T2/T3 现在是空的**
//!
//! `.config/nextest.toml` 把「高档用例」按**模块名**认（`horizon_mid` / `horizon_long`）。
//! 那两个名字还在，但里面**一条会跑的判据都没有**：
//!
//! * `horizon_mid`：**整个文件已删**（它唯一的用例早搬去 g1 的「同 seed 重跑逐字节一致」）。
//!   留在这里一句话是为了让「中档在 Rust 侧空了」这件事**看得见**，而不是让人以为是漏了一步。
//! * `horizon_long`：7 条**全是 `#[ignore]` 探针**（曾经是 T3 长局判据，已搬去 `g3_long.py`）。
//!
//! ⇒ 今天 `cargo nextest run`、`-P mid`、`-P full` **选中的是同一批 208 条**（都在 `planet_x`
//! lib 里，全是 T0/T1）。三档的区别只在你**带 `--run-ignored`** 跑探针时才显出来。

mod horizon_long;
mod site_supply;
mod tech;
mod trade;
