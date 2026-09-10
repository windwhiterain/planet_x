# 重构脚本（本次「大文件拆小 + 测档」用过的，当模板照改）

这些脚本**不是通用工具**，是 `feature/refactor-modules` 那次重构实际跑过的版本——留着
是因为「拆下一个大文件」还会再做（候选见 [`../../.agents/notes/code-layout.md`](../../.agents/notes/code-layout.md) §5），
照抄改表比重新发明安全。每个都带**硬自校验**，改错了不会写坏文件。

## 规矩（照抄这三条，别手工剪）

1. **搬运必须自校验**：拆出来的块按原顺序拼回去，必须与原文**逐字节相同**，不成立就
   `return 1` 且**不写任何文件**。`split_sim.py` / `split_control.py` 的 `main()` 里就是
   这段——第一次跑它挡下了两处「少拼一层 `\n`」的边界错位。
2. **块 = 从文档注释/属性行到函数体收尾的列 0 `}`**（`find_block`）。别用「行号区间」，
   行号一改就错位。
3. **改完立刻验行为没变**：`cargo run --bin planet_x -- --seed 42 --round 240 --digest 20`
   的 SHA-256 必须与拆前逐字节相同（基线见 `code-layout.md` §3）。

## 各脚本干了什么

| 脚本 | 干什么 | 再用时改哪里 |
| --- | --- | --- |
| `split_sim.py` | `src/sim.rs` → `src/sim/`（17 个子模块 + `mod.rs`），带自校验 | 顶部 `ASSIGNMENT`（条目→模块，**顺序必须与文件里的出现顺序一致**）与 `DOC` |
| `split_control.py` | 同上，`src/control.rs` → `src/control/`（8 个子模块） | 同上 |
| `widen.py` | 把子模块里的裸 `fn/struct/enum`（含 `impl` 内的方法）放宽成 `pub` | ⚠ 当年第一版写成 `m.end(1) or 0`，**无可见性前缀时 `m.end(1)` 返回 `-1`**（`-1` 是真值！）⇒ `fn foo() {` 被改成 `pub {`。已修成 `m.end(1) if m.group(1) else 0`，别改回去 |
| `move_tests.py` | 内联 `#[cfg(test)] mod tests {...}` → `src/tests/…`，源码里留 `#[path]` 引入 | `TABLE`（源文件、模块目录、目标文件、文档） |
| `tier_split.py` | 按模拟时间把高档用例挪进 `horizon_mid` 子模块 | `move(...)` 的用例名单 |
| `theme_split.py` | 两个测试大队按主题拆成子模块 | `SIM` / `CTRL` 两个字典 |
| `horizon_scan.py` / `horizon_scan2.py` | 量每个用例**推进多少回合**（决定进哪一档）；`-2` 会解循环上界的局部绑定并打印证据行 | 直接跑，参数是文件列表 |
| `rounds_scan.py` | 挖「跑多少回合」的证据（`write_index(120)` / `--round 60` / `const ROUNDS`） | 直接跑 |

## 为什么 `widen.py` 要放宽到 `pub`

因为 `pub use m::*;` 会**静默过滤掉 `pub(crate)` 项**：调用点报的是
`E0425 cannot find function`（**不是**可见性错误，很容易误判成名字拼错）。本仓不对外提供
库，所以内部项一律 `pub` 最省事，也免了以后在文件间挪函数时修可见性链。详见
`code-layout.md` §3。

## 扫描脚本的口径（别被它骗）

`horizon_scan*.py` 用正则猜「推进多少回合」：**循环上界是纯计算循环时会把月份算多**
（例如集货抽签的 `for _ in 0..4000` 其实一次都不推进）。所以档位是拿它 + `cargo +nightly
test -- -Z unstable-options --report-time` 的逐条实测时间**对出来的**：时间用来发现谁
可疑，回合数用来定档。加新用例时按这个顺序核一遍。
