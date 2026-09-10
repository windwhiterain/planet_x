//! **中档（T2，49–480 回合）**的端到端行为用例——**这个二进制现在是空的**。
//!
//! 它唯一的用例 `same_seed_reproduces_identically`（seed 42 / 200 回合，比 `render_state`
//! 的两次渲染）已搬到 Python 侧：`play/tests/g1_contract.py` 的「同 seed 重跑逐字节一致」
//! ——判据更强（比的是**整份投影的每个文件**的 sha256，不只是末回合的一份渲染），而且
//! **不重编**（见 `.agents/notes/test-decoupled-suite.md`）。
//!
//! ```text
//! uv run --project play/planet_xq python play/tests/run.py 1
//! ```
//!
//! ⚠ `.config/nextest.toml` 里 `binary(horizon_mid)` 那两条 filter 仍然留着：档位是**按
//! 模块名/二进制名**认的，留着不会选到不存在的用例，哪天这里又长出中档用例时也不用改配置。
//!
//! 文件本身留着（而不是删掉）是为了让「中档在 Rust 侧空了」这件事**看得见**，
//! 而不是让人以为是漏了一步。
