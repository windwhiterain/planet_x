# Agent 讲故事模式：轨迹生成器 + 外部 jq（已落地）

> 状态 `[x]` ｜ 索引：[notes.md](../notes.md) ｜ 前身：`ideas.md` §13

> 形态转变：agent 从「玩家（每回合 advance→读→apply diff）」变成「**导演/说书人**」——
> 跑一段轨迹、**任意时间轴查询**、讲给用户看。已决定并实现：**外部 jq 为必选依赖；
> 删掉自研 jq 引擎与查询 REPL；agent 仍可用 `--apply` diff 影响故事走向；Web
> （`planet_x_web`）是玩家界面，完整保留。** 落地设计见
> `schema-query-architecture.md` §10。

- `[x]` **删掉自研 jq 引擎**（`src/query.rs` 整文件 + `lib.rs` 的 `pub mod query` + 其全部
  12 个单测）。raw 任意查询全权交给外部 jq。
- `[x]` **删掉查询 REPL 与 `--query`/`--strict`/`--script`**；`main.rs` 重写为**批次轨迹
  生成器**：
  - `--round N`：输出 JSON Lines（回合 0 先，之后每回合一行 agent 视图，含 `.events`）——
    `| jq` 任意时间轴查询（`jq -s` 可整段收集）。
  - `--traj N`：输出一个自包含 `{schema_version, meta, story, trajectory:[...]}`（story pack）。
  - `--meta`/`--schema`/`--story`/`--control`：各自输出一个 JSON 值（规则字典 / agent 视图
    schema / 编年史 / 可编辑控制面模板）。
  - `--apply <diff>`：叠加控制 diff 定向故事；`--start <ckpt>`+`--save <ckpt>` 分段续玩
    （每段纯函数，seed/checkpoint 保证可复现）。
- `[x]` **验证**：`cargo build` 全目标成功；`cargo test --lib` 37 passed（原 49 = 删掉
  query 模块 12 个单测）；`cargo test --test longhorizon` 5 passed / 4 ignored。smoke：
  `--round 30 | jq` 时间序列、`--traj` pack（schema_version=1、trajectory_len=13、
  story_len=7）、`--schema`（title=AgentState）、`--control`（control+scope）、`--apply`
  定向（faction0 iron budget=999 mode=Player 确实落入控制面）、分段
  `--start ckpt +--round 6` 与直跑 `--round 12` 的末行**逐字节一致**（确定性满足）。
- `[ ]`（事实，非待办）**Web 玩家界面未动**：`planet_x_web` 保留有状态 advance/apply_patch
  交互；`web::control_surface`/`apply_patch` 与 CLI `--apply` 共享同一控制/diff 契约。
- `[x]` **`jq` 成为宿主运行时依赖**（本次已落地，`tools/jq.exe`）：主机未预装 jq 时，
  `Invoke-WebRequest https://github.com/jqlang/jq/releases/download/jq-1.7.1/jq-windows-amd64.exe`
  下载独立二进制到 `tools/jq.exe`，`Get-Command jq` 前先 `& tools\jq.exe`（或加目录到 PATH）。
  PowerShell 直传 jq 程序的**引号会被吞**（`"1"` 变 `1`、`"\(..)"` 变 `..`），**可靠做法是把
  jq 过滤写成 `.jq` 文件再 `jq -s -f file.jq`**——已用此范式完成 `--round`/`--traj`/`--digest`
  全量查询验证（脚本见 `play/jq/*.jq`）。
- `[~]` `semantic-view-api.md` 的「命名语义工具」仍可作为 Layer 2 方向，但有了外部 jq 的全量语法后，「裸 jq
  探 JSON」不再棘手；语义工具聚焦「稳定契约 + 参数校验 + 由模拟生成数据」，而非替代 jq。
