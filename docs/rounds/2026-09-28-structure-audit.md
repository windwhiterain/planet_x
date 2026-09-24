# 2026-09-28 · 结构与文档整体重构（架构审计那一轮）

> 用户口径：「架构上和文档上整体 review，优化项目结构，规范文档布局，清理历史遗迹」。
> 后续裁定：worktree **全部清掉**、viewer-gui **合入**、市场血脉**收进 `game` 且降为非 default
> member**、`docs/` **立为唯一文档根并把笔记并入**、文件名**彻底去号**（接受键全换）、
> 「你负责总控，拆分小任务交子 agent」。
>
> 项目定位由用户重新钉了一句：**"通用 graph based PCG + render agentic 内容创作引擎"**。

## §1 审计量到的东西（都不是猜的）

| 发现 | 量到的数 |
|---|---|
| **两条血缘混在一个仓里** | 渲染 / PCG 栈（29 个 crate）是活的；市场 / 经济模拟（`src/` 9 模块 + 根包 `planet_x`）是死的 —— 根包**不在 workspace 成员里、零消费者**，而 `px_protocol` 快照测试自己的注释就写着世界视图「本来就没有生产者、也没有消费者」 |
| **`.worktrees/` 是真正的历史遗迹** | **18 个 worktree、51.3 GB**；14 个已 `prunable`（gitdir 指向不存在的位置），4 个已合进 v2（`elementwise-op` / `nurbs-op` / `nebula-sparse-skip` / `star-r3`），全部干净、无未提交改动 |
| **文档与真知识库错位** | `docs/` 只有 4 份，其中 3 份（147 KB）是死掉的市场文档；渲染栈的 44 份 / 11,586 行真知识库埋在 `.agents/notes/art/`，没有 README、没有索引 |
| **索引本身已过期** | `.agents/notes/art-framework.md` 开头写"现状只看 `art/10-handoff.md`"（真入口早就是 `00-current.md`），指向的 `11-graph.md` / `12-step0.md` 是 Bevy 时代的 |
| **`00-current.md` 的判据表已与实跑不符** | 表里写"两条实例"，实跑是 **7 条**；`field.remap/waves` 登记 `caa8318cda1b`，实跑 `eb126be744bf` |
| **一个名字两个所指的绕口令** | `Cargo.toml` 里整整八行注释在解释"`px_render` 今天指谁" —— 那是文档债渗进构建清单 |
| **⚠ 后端契约自相矛盾** | `px_gpu` / `px_probe` 写着裸 `wgpu = "29"`，而 `wgpu 29` 的**缺省特性**是 `std + parking_lot + dx12 + metal + gles + vulkan + wgsl + webgpu` ⇒ 把 §104 已经否决过的后端全拉回来；而 `px_gpu::backends()` 的**缺省是 DX12**，`px_render` 却编译期锁死 Vulkan ⇒ **烘图与出图跑在两个后端上**（实测 `px_probe --bin device` 改前报 `Dx12`、改后报 `Vulkan`） |

## §2 做了什么（三个提交）

**① `053e940` 市场血脉收进 `game`，根包 `planet_x` 退场**

`git mv src/**.rs -> game/src/`（历史保留）＋ `planet_x::` → `game::`；删三个死件
（`main.rs` 一个 `Hello, world!`、`state.rs` 一个零引用的空 struct、`config.rs` 零引用的两个 struct）；
根 `Cargo.toml` 改成**纯 workspace 根**。

⚠ 路上抓到一个**真实的语义回归**：删掉那个 edition 2024 的根包之后，虚拟 workspace
**不会**从成员 edition 推出解析器版本 ⇒ **静默退回 resolver v1**（cargo 当场 warning），
等于换掉特性合并与依赖解析的规则。补 `resolver = "3"`。

**② `5b2826f` 后端契约收敛到 Vulkan**

`px_gpu` / `px_probe` 的 `wgpu` 都改成 `default-features = false, features = ["vulkan","wgsl"]`
（与 `px_render` / `px_pass` 逐字同一档）；`px_gpu::backends()` 缺省从 DX12 改成 **VULKAN**，
`WGPU_BACKEND` 仍可覆盖（给"换一台机器、Vulkan 不在"留逃生门）。
顺手把 `clean.sh` 改名 `format.sh`（它只跑 `cargo fix` + `cargo fmt`，**不删任何东西**）。

**③ 文档重构（本提交）**

46 份笔记 `.agents/notes/art/*.md` → `docs/**/*.md`，按主题分 `system/` `render/` `art/`
`rounds/`（日期命名）`archive/` `guides/`；删掉 `.agents/` 与 `art-framework.md`（一份文档一个家）；
删掉三份死市场文档；新写 `docs/README.md`（索引 + 命名与引用约定）与 `docs/invariants.md`
（把 `art-framework.md` 里那些仍在生效的不变式收进来）。

**⚠ 去号的代价是量过的，不是估的**：37 份**参与源码身份指纹**的源文件里逐字写着旧路径
（共 67 处）⇒ 改它们就是改 `decl_hash` ⇒ **全仓节点键换一遍**。全仓（含文档）共 **339 处**引用，
本轮改掉 **325 处**。

## §3 判据

| 判据 | 读数 |
|---|---|
| `game` | **84 passed**（`cargo test -p game`） |
| `px_protocol` | 31 + 3 + 3 + 3 + 1 + 3 + 5 + 2 **全 ok** |
| `px_scene` / `px_graph` / `px_graphs` | 54 / 12+3+5 / 2+5+6+2+4+1+4+3 **全 ok** |
| `cargo build --workspace` | 通过（仅余既有 warning） |
| `px_probe --bin device` | 缺省后端 **Vulkan**，"全部通过" |
| 实例 key 变化 | `cloud.coarse/band` `d1c8fd369338` → `b59da0494ac6`；`field.remap/waves` `caa8318cda1b` → `eb126be744bf`（另 3 条 element 档同规格同键）；**7 条全部重编成功** |
| **产品零影响的对照实验** | `orbit-bare` 在**两个状态**（改动全 stash 掉的 `5b2826f` 原状 ／ 本轮改动生效）烘出**同一个内容键 `c223c17220ea`**、**同一个 `.pxart` 文件字节 `DC456C0E32D3A803`（3160 B）** ⇒ 改的确实只是注释文本，**内容没漂** |

## §4 顺带量到的两处陈旧（**按裁决只记录不改**）

1. **`art/anchor/hashes.txt` §三 那六格已确认陈旧。** 登记 `orbit-bare = 1E1C3A5AA2DBE56D`
   （4125 B），而**两个状态都给出 `DC456C0E32D3A803`（3160 B）**，少 965 B（≈23%）。
   行尾（几 B 到几十 B）与本轮改动（上面那张表）两个解释**都被排除** ⇒ 自 2026-09-20
   第七次重登记之后就走过的 `40`–`44` 那几轮里，至少有一轮改了场景文档的形状，而 §三
   从 2026-09-20 起就不再重登记。⇒ 已在 `hashes.txt` 里给那六行挂"已确认陈旧、不要当判据用"
   的标记，**不改登记值**（改成一个没人验证过的新数字比留一个明确标着陈旧的老数字更坏）。
   进 backlog **S9**（三条候选路，代价差一个数量级，属设计点）。
2. **`art/nebula/density_volume.toml` 还是旧字段 `res_ratio`**（`DensityParams` 只认 `res`）
   ⇒ `px run nebula` 烘不过。这是 `43-params-not-canvas`（现 `docs/system/params.md`）那次
   "一切皆参数"改名漏掉的一份 toml。进 backlog **S10**。

## §5 这一轮踩到的坑（留给下一次改注释的人）

- **文档路径写在参与指纹的源码里**，所以"整理文档"在这种仓库里**不是零成本动作** ——
  先数一遍影响面（本轮 37 份源文件 / 67 处），再决定改不改。
- **`art/shaders/*.wgsl` 也是产品输入**（字节进闭包指纹 ⇒ 进产物）。本轮脚本顺手动了
  `surface.wgsl` 里一处注释引用，**已撤回**；`art/anchor/README.md` 早就写过这条规矩。
- **批量改写必须保留原行尾**：脚本里对每个文件核 `CRLF` 计数前后一致（本轮 0 个意外改动）——
  换行会进源码指纹，"git 说我干净"与"字节没变"是两件事。
- **替换规则要按"长名优先 + 前缀归一"两步走**，否则 `docs/` 前缀已经存在时会写成
  `docs/docs/...`（本轮真踩到一次，7 处，已当场修掉）。
- **验证"没改行为"要用跨状态对照**：把改动 stash 掉、在两个状态各烘一次、比**内容键** ——
  比"我觉得只改了注释"硬得多，而且它顺手戳破了 §三 那格陈旧。
