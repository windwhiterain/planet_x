# WebUI 生命周期：孤儿进程 / exe 被锁（启动者租约 + 关页即退）

> 状态 `[x]` ｜ 索引：[notes.md](../notes.md) ｜ 前身：`ideas.md` §25

- **动机（真事故）**：某个 session 在后台 job 里起了 `planet_x_web` 就再没管；job / daemon 没了之后
  exe 成了孤儿继续监听（**Windows 不会替你收孙进程**）。它顺带把
  `target\debug\planet_x_web.exe` **锁住**：后来那次重建只更新了 `deps\` 里的副本，顶层 exe 换不掉
  ——`cargo build` 报 `failed to remove file … 拒绝访问。 (os error 5)`，「我明明重新编译了」
  跑的还是老二进制。`web-auto-port.md` 的自动选端口又让这件事**不可见**：`3000` 被旧实例占着，新的悄悄跑 `3001`。
- `[x]` **启动者租约**（`web/src/owner.rs` + `web/src/main.rs`）：启动器把自己的 pid 通过
  `PLANET_X_WEB_OWNER_PID` 交给服务；Windows 用 `OpenProcess(SYNCHRONIZE)` +
  `WaitForSingleObject` 等它结束（unix 退到轮询 `kill(pid, 0)`，200ms），**一结束就退**。
  查不到/打不开一律当「已结束」——确认不了主人还活着就不该继续保活。
  实测：`job_kill` 掉启动脚本后 **0.15s** 内 exe 自退、端口释放。
- `[x]` **最后一个页面关掉即退**（`WebCtx` + `POST /api/tab` / `POST /api/bye`；前端在
  `pagehide` 用 `sendBeacon` 注销，`pageshow(persisted)` 时重新报到）：**没有空闲计时器**——
  没人操作不会退，只有「最后一个看它的页面走了」才退。陌生 tab id 的注销**什么都不动**
  （一个迟到的、来自上一个服务的注销不许带走当前服务）。`PLANET_X_WEB_CLOSE_EXIT=0` 关掉这条。
  实测：关掉最后一个页面 0.61s 自退。
- `[x]` **刷新窗口** `PLANET_X_WEB_CLOSE_GRACE_MS`（默认 500ms）：刷新 = 旧页面注销先到 +
  新页面登记后到；没有这 500ms，**按一次 F5 就会把服务带走**。实测：`bye` 之后 200ms 内
  新页面报到 → 服务活着。它不是空闲超时，只是吸收一个交错。
- `[x]` **身份可辨**（把 `web-auto-port.md` 的遗留做掉）：`GET /api/ping` 给 pid / 端口 / 二进制路径 +
  **构建了多久**（`exe_age_secs`——「页面里跑的是不是刚编的那个」的关键线索）/ 看护 pid / 页面数；
  启动横幅也印 pid + 二进制 + 构建时长（`human_age`）。
- `[x]` **`scripts/web.ps1`（唯一入口；git_bash 走 `scripts/web.sh` 薄壳）**：在 `cargo` **之前**
  先杀掉 `target` 落在本 worktree 的旧实例——这一步只能在 build 前（锁发生在 cargo 里，
  服务端代码那时还没跑，救不了）；然后 `cargo run` 并把脚本自己的 pid 当租约传进去。
  `-List` 列出**所有** worktree 的实例（pid / 启动时间 / 本目录还是别目录），`-Stop` 只停本
  worktree 的。**端口仍自动扫**（多 worktree 并存互不干扰），这里只清本目录。
- 验证：`cargo test -p planet_x_web` **15 passed**（新增：租约 pid 解析、死 pid 立刻返回、
  tab 登记 / 陌生 id 不动、最后一个页面才退、`close-exit` 可关、退出闸门只认第一次、
  `/api/ping` 身份、生命周期路由挂上、`human_age`、env 解析）；实机：**复现锁**
  （起旧实例 → `cargo build` → `os error 5` + 顶层 exe 不更新）→ `scripts/web.ps1` 先清后建
  成功（顶层 exe mtime 变新）→ `job_kill` 启动脚本 0.15s 内 exe 自退 → 关最后一个页面 0.61s
  自退 → 伪造的「别目录」实例被认成「别的目录」且 `-Stop` 不碰它。
- 坑（写给下一个改这个脚本的人）：PowerShell 5.1 里**只有一个元素**的函数返回会被拆成
  `PSCustomObject`，而它**没有 `.Count`**（求值成 `$null`）→ `-not $x.Count` 恒为真。脚本里必须
  `@(Get-WebInstance)` 包一层，否则它会「一边说没有实例、一边把真实例喂进 kill 分支」，而且
  `-List` 什么也不显示。（本轮真踩了。）
- `[ ]` **（留给以后，真正的兜底在 harness 侧）** 后台 job 跑进 Windows Job Object
  （`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`），或在 owner dispose 时 `taskkill /T` 整棵树——
  这样**所有**长驻子进程都不会跨 session 活下来（不只是 `planet_x_web`）。DSH 里已有
  `packages/subprocess/subprocess-local/src/windows-inspector.ts::taskkillTree` 可复用；
  动的是 harness 本仓，风险另算。
- `[ ]` **（留给以后，且是「不要超时」的已知代价）** 浏览器**崩溃**时没机会发 `pagehide`，
  服务不会自退；那时只能靠租约（启动者没了就退）或 `scripts/web.ps1 -Stop`。
- `[ ]` **（留给以后）** WebUI 的内存世界**没有存档**：关掉最后一个页面 = 世界没了。若将来
  做存档/续玩，得想清「关页面自退」与「保存对局」的关系（现在两者都由人脑记住）。
