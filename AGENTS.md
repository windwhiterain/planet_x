# 太空沙盘游戏《行星X》

回合制太阳系沙盘：每回合 = 1 个月，全程数据驱动（`config/game.ron`）、确定性可复现。

- [设计 spec](.agents/spec.md)
- [笔记索引（活文档，记得回填）](.agents/notes.md) ← 每条主题的**具体描述**在 `.agents/notes/<主题>.md`
- **[agent 游玩手册](.agents/agent-play.md)** ← 想「玩」先读这个

- 不考虑向前兼容
- 随便杀游戏进程

## WebUI（`planet_x_web`）

- **只用 `scripts/web.ps1` 起**（git_bash 用 `scripts/web.sh`，同一个入口）。它在 `cargo`
  **之前**先杀掉本 worktree 里还在跑的旧实例：旧进程会锁住 `target\debug\planet_x_web.exe`，
  让重新编译换不掉顶层二进制（`failed to remove file … 拒绝访问`）——「我明明重编了」跑的还是
  老货。随后它把**自己的 pid 当租约**交给服务：脚本一没（Ctrl+C、关终端、后台 job 被收掉），
  服务立刻自退。**别用裸 `&` / `Start-Process` 脱离启动**：那样起出来的进程没人看护，
  会话一结束就是孤儿。
- `-Stop` 只停本 worktree 的实例；`-List` 看所有 worktree 的实例（pid / 启动时间 / 目录）。
- **自退规则（没有空闲计时器）**：启动它的进程退出 → 退；**最后一个页面关掉** → 退。
  `PLANET_X_WEB_CLOSE_EXIT=0` 关掉后者，`PLANET_X_WEB_CLOSE_GRACE_MS` 调刷新窗口（默认 500ms）。
- 端口仍**自动扫**（`3000` 被占就 `3001`…，多 worktree 同时跑互不干扰）；`GET /api/ping`
  给出 pid / 端口 / 二进制 + 构建时长——对不上就是连错了实例。
- 细节与坑见 [笔记：服务生命周期](.agents/notes/web-lifecycle.md)。

## 验证（合流门前都要过）

分两层，**判据住在哪一层由「能不能只看跑出来的数据」决定**（见
[笔记：测试与二进制解耦](.agents/notes/test-decoupled-suite.md)）：

```bash
# ① 数据级 —— **流程就是「build release + python 测试」**：run.py 会先按需
#    `cargo build --release`（二进制比 src/config 旧或不存在时），再跑各组。
uv run --project play/planet_xq python play/tests/run.py all      # 四组 76 条判据；缓存命中 ~4 s
uv run --project play/planet_xq python play/tests/run.py          # 只跑快组（1 + 4，内循环）
uv run --project play/planet_xq python play/tests/run.py --no-build  # 跳过前置编译
uv run --project play/planet_xq python play/tests/_g4_negative.py # 声明纪律自己的量具：注入 19 个错，全咬住

# ② Rust 侧（搬不走的那半：纯函数 / 合成场景 / 内部契约 / 错误路径 / 探针）
cargo nextest run -P full                    # 合流门
cargo nextest run -P full --run-ignored all  # 探针（只打印不断言）
```

- 数据级那套跑在**投影**上（`--index` 跑出来的数据）：改一个文件后**不用重编 8 个测试二进制**，
  世界按 `(二进制指纹, seed, 回合数)` 缓存在 `target/test-fixtures/`（代码一改自动失效）。
- **数据级一律走 release 二进制**（用户裁决：*「python 测试的方式改为 build release 加 python 测试」*）
  ——长组的墙钟由模拟的机器码质量决定（debug 下慢 ~4×）；`--bin debug` 只在只跑快组时可选。
- 现在的耗时结构（谁是大头）见 [笔记 §11](.agents/notes/test-decoupled-suite.md) 与
  [test-wall-clock §0.2](.agents/notes/test-wall-clock.md)：
  - **Rust 门**：增量（改一个库文件）**~13–16 s** = 编译 ~10 s + 真跑 4–5 s；
    **冷/切档首次 62–89 s**（那个大数只在换 worktree / 切档时出现，别拿它当稳态）。
  - **test 档不提 `opt-level`**（2026-10 起：长局搬走后提档的依据消失，编译成了大头）。
    ⚠ 实测 `opt-level = 1` + `debug = 1` 是甜点（真跑与 O2 相同、编译更省，一轮门 13.2 s
    vs O0 的 ~28.6 s）——**要不要切 O1 待用户裁决**。
  - **`tests/` 下 5 个集成二进制每次改库文件都要重编+链接，合计 ~5–6 s**（约占增量编译一半；
    空的 `horizon_mid.rs` 一个就 0.4 s）。合并成单个 `probes.rs` 能省 ~4–5 s，**待裁决**。
  - 投影每 1000 回合 169 MB 多花 ~+3.5 s 墙钟（**是构造 JSON，不是磁盘**：这台 NVMe 写
    169 MB 只要 0.1 s）。
- **只想要最终 state**（不要逐回合轨迹）：`--round N --quiet --save ckpt.ron`
  —— 1000 回合实测 5.2 s / **stdout 0 字节** / 档 0.09 MB（不加 `--quiet` 是 6.3 s / 40.3 MB；
  单独 `--save` 不会变快，因为 `--round` 的合同就是每回合吐一行）。
- **CLI 精简过一轮**（2026-10）：`--traj`/`--story`/`--notables`/`--milestones`/`--rounds`
  已删（信息全在 `--index` 投影里）；**裸调用 `planet_x` 打 help**，`--seed 42` 这种
  「有参数没动作」仍是机器可读的 `ERR_USAGE`。见 [笔记：CLI 读面](.agents/notes/cli-surface.md)。
- 加一条断言：写进 `play/tests/g*.py` 的 `run()` 里（判据写 `run()`、数据取自摘要 ⇒ 改断言
  不重读投影）；**每条守卫都要带防空转判据**（「这一局里真的发生过 X」）。
- 分档口径不变（[笔记：测试分档](.agents/notes/test-tiers.md)）：快组 ≈ T0/T1、中组 ≈ T2、长组 ≈ T3。

## 给 agent 的工作约定

- 永远用相对数值比例而非绝对数值
- 一切数值都用动态平衡/博弈来产生
- **默认用概率分布，而不是硬阈值/贪心**（用户裁决）。想一个机制时，先问一句：
  **「这件事用概率分布能不能表达？」** 能就优先用它。已落地的两个样板：
  - **导航（MOND）**：偏移是**伪随机范围**而不是写死的量 ⇒ 深处**没有进不去的目标**，
    只是要多试几个回合（`sim::mond_drift` + `nav_roll`）。
  - **集货派单**：每艘运输舰去哪个积压点，**概率 = 该处积压占比**（抽签），而不是
    「积压最大的先派」的贪心（`autocontrol::freight::route_for` + `sim::derived_roll`）。
  理由：
  1. **硬阈值会造出断崖**——跨过一条线就完全反过来。玩家看到的是「不合理」，不是「难」；
     概率分布给出的是**连续的难度**（越深越难，但永远有可能）。
  2. **贪心要维护一本全局账**（别人在干什么、船在路上、船被击沉、船改行……），而按概率
     抽签**一艘船掷一次骰子就走**：无中心、无顺序依赖、代码短得多。
  3. **期望上自动等于想要的比例**（按积压占比抽签 ⇒ 期望运力按积压成比例），
     不必额外写「怎么分配才公平」的那一层。
  4. **概率 ≠ 不可复现**：骰子走 `(势力, 舰名, 回合, 用途)` 派生的哈希
     （`sim::derived_roll`），**绝不消费主 `Prng` 流**——否则「多派一艘船」会改变整个世界
     后续的掷骰，同种子可复现就退化成「舰队数量一变后面全变」。
  例外：**本来就是物理/规则**的地方照旧用确定值（资源守恒、造价、航程、命中结算的几何）。
- **边实现、边想点子**：在做当前目标时，冒出的新机制/新平衡/新剧情只要值得做就**先尝试**；
  做成验证编译通过、长局 harness 不崩、确有增益的改动。
- **来不及实现的写进笔记**：任何想到但这一回合来不及做完/验证的点子，一律**新建
  [`.agents/notes/<主题>.md`](.agents/notes/)**（写得够细，让下一个 agent 能照做）**并在
  [`notes.md`](.agents/notes.md) 加一行索引**，别让它在对话里蒸发。已实现的勾成 `[x]` 并注明
  模块，避免重复劳动。
