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

## 给 agent 的工作约定

- **边实现、边想点子**：在做当前目标时，冒出的新机制/新平衡/新剧情只要值得做就**先尝试**；
  做成验证编译通过、长局 harness 不崩、确有增益的改动。
- **来不及实现的写进笔记**：任何想到但这一回合来不及做完/验证的点子，一律**新建
  [`.agents/notes/<主题>.md`](.agents/notes/)**（写得够细，让下一个 agent 能照做）**并在
  [`notes.md`](.agents/notes.md) 加一行索引**，别让它在对话里蒸发。已实现的勾成 `[x]` 并注明
  模块，避免重复劳动。
