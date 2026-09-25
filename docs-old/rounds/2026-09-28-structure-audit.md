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

## §4 顺带量到的陈旧件（**后来都当场收口了**）

1. **`art/anchor/hashes.txt` §三 那六格已确认陈旧。** 登记 `orbit-bare = 1E1C3A5AA2DBE56D`
   （4125 B），而**两个状态都给出 `DC456C0E32D3A803`（3160 B）**，少 965 B（≈23%）。
   行尾（几 B 到几十 B）与本轮改动（上面那张表）两个解释**都被排除** ⇒ 自 2026-09-20
   第七次重登记之后就走过的 `40`–`44` 那几轮里，至少有一轮改了场景文档的形状，而 §三
   从 2026-09-20 起就不再重登记。
   ⇒ 当时按"只记录不改"处理；**随后用户裁决整个锚退休**（"我不需要和 bevy 对比，
   这个东西还有别的用途吗？没有就彻底删掉"）⇒ 见 §7。
2. **`art/nebula/density_volume.toml` 还是旧字段 `res_ratio`**（`DensityParams` 只认 `res`）
   ⇒ `px run nebula` 烘不过。这是 `43-params-not-canvas`（现 `docs/system/params.md`）那次
   "一切皆参数"改名漏掉的一份 toml。进 backlog **S10**。

## §5 ⚠ 提交之后才现形的两件事（同一个根因：默认成员变了 ⇒ 第一次真的跑到）

把 `px_volume_alg` 与 `px_nurbs_gpu_op` 补进 `default-members` 之后，默认 `cargo test`
**第一次**真的跑了这两个包里那些判据 —— 于是冒出两处**与重构无关、但一直被捂着**的红：

**① `default-features = false` 之后必须点名 `std`。**

`px_nurbs_gpu_op` 的 6 条判据在多线程下随机红：
`Mismatched pop_error_scope call: error scopes must be popped in reverse order`，
而且**同一份二进制每次失败的条数都不一样**（1/3/4/5 条都出现过）—— 看着像竞态。
二分实测（每个变异跑 3–5 次）：

| `px_gpu` 的 wgpu 特性 | 缺省后端 | 结果 |
|---|---|---|
| 裸 `wgpu = "29"`（完整缺省特性） | DX12 | 6/6 绿 ×3 |
| 完整缺省特性 | **Vulkan** | 6/6 绿 ×3 |
| `false` + `vulkan` + `wgsl` | Vulkan | **红 3–5 条** |
| `false` + `vulkan` + `wgsl` + `parking_lot` | Vulkan | 仍然红 |
| `false` + `vulkan` + `dx12` + `wgsl` + `parking_lot` | Vulkan | 仍然红 |
| `false` + **`std`** + `vulkan` + `wgsl` | Vulkan | **6/6 绿 ×5** |

⇒ 根因是**缺 `std`**：wgpu-core 走了 no-std 那一套同步实现。**不是竞态、不是 Vulkan 的锅**。
⚠ 而 `px_render` / `px_pass` **一直有同样的漏洞** —— 它们的 `std` 是靠 `egui-wgpu`
顺带合并进来的（偶然，不是声明）。三处一并补上，并进 `docs/invariants.md`。

**② `px_volume_alg` 的 5 条判据是"一切皆参数"改名时漏改的。**

`DensityParams.res` 从"**占画布宽度的比例**"改成"**绝对数**"（`43-params-not-canvas`，
现 `docs/system/params.md`），判据里的字面量没跟着改：`res: 32` 当"8 的 0.5 倍"、
`res: 64` 当"取满"，而默认值正是 `64` ⇒ 五条一起红（`left: 32 / right: 4`、
`left: (64, 8) / right: (8, 8)`）。

修法是**逐条判据自己把 `res` 写明白**（同一份参数在不同判据里有不同意图）：
`params_for` 那个助手回到只填公共栏，纯搬运那一档显式 `res = 上游.res`（走不插值那条路）、
重采样那一档显式给比上游粗的值。⚠ 路上我先把助手改成 `res = shape.res`，结果
`every_voxel_lands_at_its_own_coordinate` 报"得 501、期望 502" —— 因为那一改把 4 条判据
从"重采样"推到了"纯搬运"，而它们**本来就该在重采样那一侧**。

⇒ 这两件都记成一条纪律：**"某个 crate 不在 default-members 里"等于它的判据长期没被跑过；
把它补进去的那一刻要准备好接红**。

## §6 这一轮踩到的坑（留给下一次改注释的人）

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

## §7 锚（`art/anchor/`）整体退休

用户裁决：**"我不需要和 bevy 对比，这个东西还有别的用途吗？没有就彻底删掉。"**

查"还有别的用途吗"的过程**翻出两件事**（细节见 `docs/anchors.md`）：

1. **六张判据图与登记表确实没有别的用途** —— 全仓引用都是注释里的**指向**，
   没有任何程序、测试、构建步骤读它们 ⇒ 删。
2. **但六份 `frozen/*.pxart` 有用途，而且那条用途已经死了。**
   它支撑 `px_protocol` 的 `the_frozen_originals_round_trip_byte_for_byte`
   （"给 `PassSpec` 加字段是纯加法"的**唯一**证据）。两件事一起塌：
   - 那条判据**一直在静默跳过** —— 它读的是 `target/oracle/pxart-frozen`，而那个目录
     在 2026-09-18 那场清理里就没了；六份的永久家早已按锚 README 自己的规矩搬进
     `art/anchor/frozen/`，**但没人改测试里的路径**；
   - 把路径改对之后它**当场红**：六份全是 `"schema": 2`，今天读写 v3 ⇒ `scene_bytes` 直接拒。
     **永久不可满足**，不是能修的 bug。

**做了什么**：删六张 PNG（1.98 MB）、删 `hashes.txt`（28 KB）与 `README.md`（12 KB）、
删六份 v2 冻产物（28 KB）；把两条**仍然生效**的禁令（`§155.4` 那两处 `px_render::reflect`
不许改、`§157` 的名字分义）与行尾约定那一格搬进 [`docs/anchors.md`](../anchors.md)；
12 处悬空指针改指新家（`docs/archive/` 里的历史记录按原样保留 —— 那里写得对）。

**那条判据换成**：`what_the_writer_produces_reads_back_and_writes_again_byte_for_byte`
—— 拿今天这份代码自己写出来的字节当靶子（复用本模块已有的最小夹具 `doc()`，
**几毫秒、不需要 GPU 或实例库**），判"读→写幂等"。

⚠ **说清丢掉了什么**：它证的是**幂等**，不是**与某个历史字节序列相同**。
原来那六份冻产物能证的"跨版本加字段没动老字节"**没有了** ⇒ 今天"加字段是纯加法"
只在**加字段时的人眼**那一层拦。这是明知代价的选择。

⚠ **留给下一次的教训**（已写进 `docs/invariants.md`）：**搬靶子的时候要连读它的那行路径一起改，
并实跑一次确认它没有在静默跳过。** 一份打着一行 `⚠ 跳过…不是通过，是没测` 就返回的判据，
会淹没在一串 `ok` 里瞎掉一整轮。

