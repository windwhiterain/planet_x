# 09 交接：现在在哪儿，下一步做什么

> 这篇是**唯一的现状出口**。别处只写「是什么 / 为什么」，只有这里写「做到哪了、还差什么」。
> 每次开工前先读这一篇，收工前改这一篇。

---

## 9.1 现在在哪儿

### 9.1.0 本轮（2026-09-16，`.worktrees/graph-research`）：动态 schema 调研 ＋ **第 0 步实测**（**代码未动**）

**在哪条线上**：`.worktrees/graph-research`（分支 `feature/graph-research`，从 `v2` 的 `b2273e8` 拉），
**还没并入 `v2`**。到这一轮为止**只有笔记**：`11-graph.md` §67–§78（动态 schema 与动态 render graph 的调研、
四条用户裁决、修订后的路线与开工顺序、裸 wgpu 与 Bevy 图能有多动态的追问），本轮的实测**单独一篇**：
**`art/12-step0.md` §79**。
**代码一行没改**：`px_render.exe` 的 sha256 全程不变（`d1e39651066b0ed8…`），
本轮改过的 `art/shaders/clouds.wgsl` 与 `art/scene/orbit.toml` **全部撤回**。

**用户本轮重申的口径**：**「对于美术设计师来说 material schema 应当表现为动态的」**
⇒ 目标**不是**面板 / 编辑器（§73 已裁决不要），而是**改 WGSL + 改配方 ⇒ 重烘 ⇒ 出图：0 编译、0 重启**（判据 P/R）。

**做了什么**（读数与四道墙在 `art/12-step0.md` **§79**）：借 `.worktrees/generic-render/target` 当构建缓存
（复用 Bevy 依赖，本地 crate 39.8 s），**一个 `--serve` 会话（pid 18172）跑完八步**，
中间只改 `art/shaders/clouds.wgsl`（结构体）与一次 `art/scene/orbit.toml`。
仪器落在 `target/step0/`：`serve.ps1` / `req.ps1`（每次请求打 pid、退出码、png 哈希、exe 哈希）/
`forge-param.py`（改产物里的参数表）/ 服务日志 / 六张 png / 一份手工造的产物。

**已验证**：

- **判据 P/R 在渲染器这一层成立**：换结构体布局（E2）与「新字段 + 新值」（E5）都在同一 pid 里出图，
  exe sha256 不变、流程里没有一次 `cargo`；日志 `渲染管线全部就绪：共 42 → 46 → 47 → 48 → 49 → 50 条，失败 0 条`
  ⇒ **新版本当帧现编管线**。
- **拒是当场拒、服务不死**：E3（产物没给 `witness`）/ E6（参数块 1088 > 1024）/ E7（贴图声明在第 9 格）
  三条都是退出码 1 + 点名拒词，之后同一会话接着出图（E8 与基线**逐字节相同**）。
- **键是内容的纯函数**：撤回之后**键与图都**逐字节回到基线（`clouds=5a88f3986ab8`、场景 `18b091fc2654`）。
- **两个硬顶的实测读数**：参数块 1024 字节、贴图只有 4 格（1/3/5/7）—— 第 1 步要加宽的正是这两个。
- **离线门**：结构体**换布局**会让 `px_render/src/reflect.rs:499-527` 红（`wind_skin` 128 ≠ 120）；
  **尾部加一格不会**（128 → 128）⇒ 那道门对「加参数」是瞎的。

**没做 / 待办**：

1. **第 1 步已开工并提交**（同一天的续，见下）⇒ 只剩第 2 步（配方透传 / `SLOTS` 扫描 / 协议加法不升版本）
   与第 3 步（pass 图 —— 另一个 worktree `.worktrees/pass-table` 在做）。
2. `art/shaders/*.wgsl` 与配方**本轮改的都已撤回**；`target/step0/` 那批仪器**不在 git 里**（`/target` 被忽略）。
   ⚠ 收工时 `art/shaders/clouds.wgsl` 上**还挂着一处不是本轮实验留下的**未提交改动
   （`@align(16) density`——写它的那个会话已经搬到 `.worktrees/pass-table`）⇒ 按用户 2026-09-16 的裁决**留着不动**。
3. 这一轮**没跑** `--view` / `--sheet`（只走 `--serve` 出图）；`orbit-bare` 只当对照。
4. `clouds.exe` 这次是**全量重算 41.9 s**（这个 worktree 的 CAS 是新建的）。

#### 续（同一天）：第 1 步「契约收口 + 加宽超集」已提交 **`c44e136`**

**判据**：产物键逐字节不变｜出图哈希四张全同 `2b1a76f4…`（= 第 0 步基线）｜配对 gpu p50
a4 **4.633** / b12 **4.537 ms**（−2.1%，噪声内）｜84 个用例通过。细节在 **`art/08-renderer.md` §80**。

**做的事**：唯一一份表 `px_protocol::material`（组号 / 格号 / 维度 / 上限 / `ParamKind` / `pack`）；
naga 反射与组装搬进叶子 crate `px_shader`；**灭掉 2/3 那颗雷**（一律替成 3 = Bevy 的
`MATERIAL_BIND_GROUP_INDEX`，探针材质组 2→3、job 3→4）；占位 WGSL 由表生成；
schema descriptor 进产物（第二个 U8 blob，不参与键）+ 装载时 `schema_check` 对账；
**加宽**：贴图 4 → 12 格（8×2D + 4×cube）、参数块 1024 → 4096 字节（老四格一个没动）。

**这一步之后还没做的**：① 第 2 步（配方透传）—— 在那之前「加一个参数」仍要改 Rust（§79 的 W1）；
② 探针三个 bin 没跑（只改了绑定组号）；③ `--view` / `--sheet` 没跑；
④ ⚠ `tests/cloud_field.rs` 那条门红着 —— 是 `clouds.wgsl` 上那处**外来改动**把它从 128 撑到 144，
不是这一步引入的；⑤ 构建已从借用的 `generic-render/target` **改回本 worktree 自己的 `target/`**
（两个会话共用那个目录时轮流覆盖 exe，已经因此白烧过一次烘图）。

**⚠ 并发会话的两个后果，后来人要知道**：① 另一个会话在 `.worktrees/pass-table` 里做 pass 表，
也在编辑 `.agents/notes/art/11-graph.md` —— 本轮的实测因此**另立** `art/12-step0.md`（用户裁决）；
`11-graph.md` 第 734 行那份副本**已删**（2026-09-16，原地留一行指路）。
② 起 `--serve` 前 harness 会拦（单例闸门）：对方在跑时**不要杀**，等对方自己收；两个并列的渲染循环
会让双方的性能数据都作废（§80.3 记了那次被污染的读数）。

#### 续（同一天）：第 2 步「schema 变成数据」已提交 **`34c9b3f`**

**这一步买的是「加一个参数 = 改 WGSL + 改配方，0 编译、0 重启」**（第 0 步 §79 的 W1 那道墙拆了）。
细节在 **`art/08-renderer.md` §81**。

**做的事**：烘图侧判据从**写死的白名单**换成**这份 shader 自己的契约**（产物里的 descriptor）——
结构键编译器消化、契约里的名字按类型透传、两边都不是就报错（`merge_params`）；
槽表从 `const SLOTS: [&str; 3]` 换成**扫 `art/shaders/*.wgsl`**（今天 4 份，`ring` 也进清单了）；
协议给文档结构加 `deny_unknown_fields`（**加法不升 `SCENE_SCHEMA`，但未知字段不许静默忽略**；
自由的名字表——材质参数、图元参数——保持自由）。

**判据**：20 / 20 个场景与旧判据**键逐字节相同**（A/B：stash 掉改动用旧 exe 再烘一遍）｜116 个用例通过｜
人工路走通：改 WGSL + 改配方 ⇒ 键换、**26 个参数**、像素变 **23.80%**，而 **exe sha256 不变、服务同 pid**；
回退后逐字节回到基线｜三种写错（名字打错 / 少给参数 / 值形状不对）都在**烘图时**红（101）｜
descriptor 闸门端到端：拿第 0 步那份没有 descriptor 的产物请求 ⇒ 当场拒 + 重烘配方。

**下一步的位置**：`art/11-graph.md` §76 开工序的第 3 步（**pass 图**：文档里的 pass 表 + 一个执行器系统）
—— 那条线在 `.worktrees/pass-table` 里由另一个会话并行做，动手前先跟那边对齐。

#### 再续（同一天）：尾巴三件已提交 **`7f8e144`**（另加探针修复 **`9c0add2`**）

细节在 **`art/08-renderer.md` §82**：

1. **探针镜像**：`px_probe/src/params.rs` 不再手抄字节布局 —— 结构体只管**值**，
   **名字 ↔ 字节**来自 `clouds.wgsl` 的契约。判据：`field_dual` 三组读数**一字不差**
   （⇒ 契约打包与旧 `encase` 布局逐字节等价）；`encase` / `glam` 两个依赖删掉。
2. **窗口模式复验**：`--sheet`（出图 4.17 MB）、`--view`（46 条管线 0 失败）、`--show`（推送生效、同键不重建）✓。
   ⚠ 记下一条**窗口模式下的 Vulkan 校验错**（`vkAcquireNextImageKHR` semaphore，10 s 6 条）：
   呈现路径、**不给场景也复现** ⇒ 不是这一轮引入；离线服务无 swapchain 所以干净；**没做二分**。
3. **笔记清理**：`11-graph.md` 里那份 §79 副本已删（原地留 5 行指路），三处引用同步。

⚠ **还欠着的**：`gradient` 的第 8 条（归因 check 的 2.259e-1）用户裁决**记账、不追**（§81.6 末）；
探针的 `field_dual` / `gradient` 是手动跑的，**没进任何自动门**（本仓的规矩：探针不进 `cargo test`，
退出码才是判据）—— 也就是说：**谁会想到去跑它，谁才会发现它红了**，这一轮就是这么发现的。

#### 已 `--no-ff` 并入 `v2`：合并提交 **`df82596`**（17 个提交，`dbd227c` §79 → `6efb264` 尾巴）

**并入后的冒烟（主工作区 / `v2`，2026-09-16）**：

- `cargo test`（默认 members）退出码 0；`cargo test -p px_render` **27 个用例全过**；
  `cargo build -p px_render -p px_graphs` 通过。
- 主工作区重烘：`shaders` **4/4**（`clouds=5a88f3986ab8` / `atmosphere=d4501946bb0c` /
  `ring=fcf9f5f88282` / `surface=f679cdf81015`）与 `planet` **6/6** 的键与 worktree **逐字节相同**；
  把 `clouds` 图与 20 个场景（`orbit` / `orbit-soft*` / `orbit-rings` / `soft-*` …）逐个重烘之后，
  场景键也与 worktree **逐个相同**（`orbit=18b091fc2654`、`orbit-soft=4b115d94428c`、`orbit-bare=28a9b516c132` …）
  ⇒ **两个 checkout 各算一遍，键一字不差**（键是内容的纯函数，这条同时钉住「并入没有改语义」）。
- 出图（Vulkan / RTX 3060 / 960×640 / `--cam 0,5,3.2`）：`target/postmerge-orbit-bare.png` =
  **301107 字节**，sha256 `74d197431f…` —— 与 worktree 上的基线**逐字节相同**。
- ⚠ 清理记录：主工作区当时留着一份**陈旧租约**（`target/render-server.json`，pid 28528，
  exe 指向 `.worktrees/pass-table`）—— 进程早没了但文件没清，harness 的单例闸门会把它读成假故障。
  确认进程不在之后删掉，冒烟才跑起来。**用 `Start-RenderServer` 起的服务一定要走 `Stop-RenderServer`。**
- ⚠ `nebula-*` 三个场景**没重烘**（不属于这条线，仍钉着更早的产物；要出图得先重烘它们各自的图）。

**这一支带来的「下次要注意」**：`SCENE_SCHEMA` 还是 2、协议形状没变，但**shader 产物多了 schema descriptor**
⇒ 并入之后主工作区里任何**契约收口之前烘的**产物都会被装载闸门当场拒（提示重烘配方）。
主工作区已经按上面那条重烘齐了。

### 9.1.1 本轮（2026-09-16，`.worktrees/shader-include`）：shader 缓存对 include 敏感 ＋ §28.2 收尾

**在哪条线上**：`.worktrees/shader-include`（分支 `fix/shader-include-aware-key`，从 `v2` 的 `479cef0` 拉）。
**已 `--no-ff` 并入 `v2`**：合并提交 **`b0737c3`**（三个提交 `d799488` §28.2 / `35a1938` include-aware / `1dc172b` 笔记）。

**做了什么**（起因：用户指出「PCG cache 机制并没有 shader include aware」）：

- **新叶子 crate `px_shader`**：模块发现（两个根：`px_render/assets/shaders` 与 `art/shaders`）、`#import`
  解析（最长模块名前缀，与 naga_oil 同口径）、**可达闭包**与它的指纹、指纹进出清单参数（`closure_hi/lo`）。
  一份实现，烘图侧 / `px_render::shaders` / `reflect` / 门共用（原先 `reflect.rs` 自己搓过一份 FNV，已收敛）。
- **`px_ops::shader_key` 升到 `px_shader/v2`**：键 = `SHADER_VERSION ‖ 闭包指纹 ‖ WGSL 字节`；`write_shader`
  把闭包指纹与规模写进清单参数。两个烘图点接上：`px_graphs --bin shaders`（三个槽）与 `scene.rs::ring_shader`。
- **装载时闸门**：`px_render::scene::closure_check` —— 产物记的闭包 ≠ 盘上现在的闭包（或老产物没记过）
  ⇒ 当场拒 + 重烘配方（用户选的档：拒绝，不警告后继续）。
- **§28.2**：算子 `SOURCE_HASH` 改为 `fnv1a_sources(&[…])`，覆盖共享依赖（`field.rs` / `noise.rs`；
  `px_mc` 加 `volume.rs`；`cloud_proxy` 再加 `px_verify/{cloud_field,noise,dual}.rs`）。新门
  `px_ops/tests/source_hash.rs` 钉住形状（退回单文件版就红）。

**已验证**：

- `cargo test`（默认 members）全绿；`cargo test -p px_render` 全绿（含离线 shader 门 —— 我改了它脚下的
  `shaders.rs`：`shader_files` / `module_sources` 现在走 `px_shader`，返回 `BTreeMap`）。
- 烘图实测（本 worktree 的 CAS）：`planet` → `shaders` → `scene orbit-bare`；日志打出每个槽的
  `include 闭包 …｜可达模块 …｜外部符号 …`。
- 端到端闸门实测（Vulkan / RTX 3060 / 960×640 / 一个服务会话）：基线出图 300012 字节 → 给
  `light.wgsl` 加一行注释（不重烘）⇒ **退出码 1、不出图**，拒词点名 `shaders/surface` 与两个指纹；
  重烘 `shaders`＋`scene` ⇒ 三个 shader 键与场景键全换、出图恢复且**与基线逐字节相同**；撤回那行
  再重烘 ⇒ 键**逐字节回到基线**（键是纯函数）。细节在 `08-renderer.md` §52.3。
- 旧键对照：主 checkout 的 `clouds` 是 `b52f7a0d…`，新键 `5a88f398…`（命名空间与闭包都进了键）。

**没验证 / 没做**（如实记）：

1. **没跑 `orbit`（带云）那条全链**：这一轮的 CAS 里只有 `planet` / `shaders` / `generated` /
   `scene orbit-bare` —— 云图（`clouds` / `cloud_proxy`）没重烘、带云场景没出图。
2. **没跑 `px_probe` 的三个 bin**（`field_dual` / `gradient` / `device`）：`assemble()` 的调用签名没变，
   但它们读的是盘上的 shader，值得单独跑一次确认。
3. **`--view` / `--show` 那条路没跑**；`tools/probe.ps1` / `frame-probe.ps1` 也没跑（只用了 harness 的
   `Start-RenderServer` / `Invoke-Client`）。
4. **口径边界**：`bevy_pbr::*` 这类外部符号只按**名字**进指纹 —— Bevy / naga_oil 换版本时键不会动，
   要靠手动 `SHADER_VERSION`（§19.1 那一档）。这条是有意留的，不是漏的。
5. **热重载那条残留**（`08-renderer.md` §52.3 写着）：服务在跑时改库文件，watcher 会把新模块组装进去、
   槽版本没变 ⇒ 下一次装载场景之前那一小段画的是"新库 + 旧键"。下一次请求会被闸门拒，窗口本身没兜住；
   要彻底得把库也搬进 CAS（用户裁决过的 C 档，本轮没做）。
6. `ring` 那条（`scene.rs::ring_shader`）走同一个 helper，但本轮**没烘带环的场景**（`orbit-rings`）⇒
   没有实测图。
7. `generic-render` worktree 的 `target/` 被借来当构建缓存（复用 Bevy 依赖）；那里会多出这个分支的
   `px_render.exe` —— 与主 checkout 的 `target/` 无关。
8. **材质参数 schema 的调研（§66）**：只落笔记，**代码未动**；候选批次 P1 / P2 / P3 见 `08-renderer.md` §66.5。
   一并裁决待做的一条：**`Value::Text` 从 wire 的 `Value` 里删掉**（它今天在参数块里永远非法）。
9. ⚠ **并入 `v2` 之后主 checkout 的 CAS 是"部分待重烘"状态**：老 shader 产物没有闭包指纹（`px_shader/v1` 时代），
   新的装载闸门会**当场拒**任何还钉着它们的场景。并入时已经在主工作区重烘了 `shaders` / `planet` /
   `scene orbit-bare`（见下面那条并入记录），**其余场景（`orbit` / `orbit-soft*` / `orbit-rings` / `nebula-*`）
   仍然钉着老产物** ⇒ 要出图得先 `cargo run -p px_graphs --bin shaders`，再逐个 `--bin scene <名>`
   （带云的场景还要先把 `clouds` 图重烘）。

**并入后的冒烟（主工作区 / `v2`，2026-09-16）**：

- `cargo test`（默认 members）全绿；`cargo test -p px_render` 全绿（lib 16 含三条 `closure_check` 单测、
  `tests/shaders.rs` 5、`cloud_field` 2、`spike_reflect` 1）；`cargo build -p px_render` 通过
  （只剩 `main.rs` 那条 `unused_mut` 老警告，不是本轮的）。
- 重烘 `shaders` / `planet` / `scene orbit-bare`：**键与 worktree 上逐字节相同**
  （`clouds=5a88f3986ab8` / `atmosphere=d4501946bb0c` / `surface=f679cdf81015` / 场景 `28a9b516c132`）
  —— 键是纯函数，两个 checkout 各算一遍也该一样，这条同时钉住了「并入没有改语义」。
- 出图（Vulkan / RTX 3060 / 960×640）：`target/postmerge-orbit-bare.png` = **300012 字节**，
  sha256 `6318415190…` —— 与 worktree 上的基线**逐字节相同**；服务端日志打出每个 shader 成员的
  `include 闭包 …｜可达模块 …｜外部符号 …`。

### 9.1.2 本轮（2026-09-15，`feature/cloud-surface-perf` worktree）：软档收影 ＋ 地表云影

**在哪条线上**：`.worktrees/cloud-surface-perf`（分支 `feature/cloud-surface-perf`）。软档与
场景产物那条路（§52）只活在这个 worktree 里，**v2 上没有** ⇒ 这一轮的所有改动都在这里。

**这一轮做了什么**（用户的两条要求，口径与实测全在 `06-clouds.md` §59）：

- **云收"别人"的影**：软分支每步采样 Bevy 的 directional shadow map（山尖 / 环挡住的光），
  云自己的自阴影**仍旧**是每步法线的 N·L。`DirectionalLight.shadow_maps_enabled` 由 planet part
  的 `shadows` 参数说了算（缺省 0 = 老行为）；级联按这颗行星定（2 级、0.1–2.5–6.0）。
- **地表云影**：`surface` 槽不再是"走 Bevy 内建材质"的槽 ⇒ 新增 `art/shaders/surface.wgsl`
  ＋ `px_render/src/surface.rs` 的 `SurfaceMaterial`；云影 = 按**指定高度** `shadow_height`
  查云覆盖度立方图的解析近似（切向偏置 ＋ 三方向半影），**只压直接光**。
- **顺手修的仪器**（§59.5）：`frame-probe.ps1` 把"取产物路径"排在重烘之前 ⇒ 新场景报"清单里
  没有这个节点"、改过内容的场景**安静地量上一份产物**。已改成一律先重烘再取。

**已验证**：

- `cargo check -p px_render --all-targets` ✅；`cargo test -p px_render` 全绿（含 shader 门：
  三个 shader 解析＋校验＋体量，`clouds.wgsl` 1378 行 HLSL / `surface.wgsl` 552 行，上限 4000）。
- 出图（Vulkan / 2240×1400 / 5 档一批）：`orbit-soft`、`orbit-soft-plain`、`orbit-soft-noshadow`、
  `orbit-soft-nocloudshadow`、`orbit-bare`；**管线 0 失败**（44 → 51 条）。
- 差异带（`target/pixdiff.ps1`，逐像素最大通道差）：只换材质 max 47、云影 max 84（13.4% 像素）、
  shadow map max 229 但只有 0.68% 像素 —— 数字与图都在 `06-clouds.md` §59.3。
- 回归：`orbit` / `orbit-surface` / `orbit-proxy` / `orbit-bare` 重烘重出，都正常（无洋红、无丢云）。
- 代价（Vulkan / GPU p50 / 780×520 / 配对）：软档全开 − 全关 = **+0.31 ms**（轮间抖动同量级，
  只能说"小于 1 ms 量级"）。

**没验证 / 没做**（详见 §59.6）：

1. `--sheet`（12 视角对照图）**没出**；`soft-e300/e6000/e24000` 只重烘了场景、没重出图。
2. `--view` / `--show` 那条路这一轮**没跑**（只用了 `--serve` 出图与 `-Phase stable` 收帧）。
3. 云自己的**曝光**没动：本仓两个自写材质（云、大气）都不乘 `view.exposure`，所以本来就偏亮
   （云的"均匀白"有一部分来自这里）。这次只给新的 surface 材质乘了曝光。
4. 环影（`rings > 0`）只有代码路径，**没有实测图**：这批场景 `rings = 0.0`。
5. 三个探针 bin（`field_dual` / `gradient` / `device`）这一轮仍没跑。

**review 用的图**（都在 `target/`，2240×1400）：`shot4-shot-r1-orbit-soft.png`（全开）、
`shot4-shot-r1-orbit-soft-nocloudshadow.png`（关云影）、`shot4-shot-r1-orbit-soft-noshadow.png`
（关 shadow map）、`shot4-shot-r1-orbit-soft-plain.png`（两个都关）、`shot4-shot-r1-orbit-bare.png`
（无云），以及三张差异图 `pixdiff-material/cloudshadow/shadowmap.png`、晨昏线并排 `crop-limb.png`。

- **这条线已经合进 `v2`**：`a666c13 Merge branch 'wip/field-dual-arbiter' into v2`（117 个文件，
  +22892/−14，零冲突）。`wip/field-dual-arbiter` 已完全被 v2 包含 ⇒ **后续开工在 v2 上**，
  或从 v2 拉新分支；原 worktree `.worktrees/field-dual` 只是个旧址。
- 合并后 CPU 链在 v2 上复验过：`cargo check -p px_protocol -p px_ops -p px_graphs -p px_verify
  --all-targets` ✅ 4.34 s；同四个 crate 的 `cargo test` 全绿。
- ⚠️ **`game` 编译不过 —— 已知，且用户裁决「不管 game」**（2026-09-14）：
  `game/src/project.rs:17` 读 `snapshot.executions`，而 `game` 自己的 `Snapshot` 已把这个读数
  换成 `intake`（`game/src/lib.rs:47` 的注释：「原来这里是 `execution`（执行率），该读数已随
  `distribution` 归一化一起删除；物理活跃度由 `intake` 承担」）。默认 members **含 `game`**
  ⇒ `cargo test` 是红的。要绿用：
  `cargo test -p px_protocol -p px_ops -p px_graphs -p px_verify`。

### 9.1.3 本轮续（同一天，接着 9.1.2 的两条之后）：光源去常量（点光源）＋ 细节风

**用户的两条**：①"不要硬编码 `SUN_DIRECTION`，用通用的光源来处理"（追问定为**先支持点光源、
太阳换点光源**，影一起接，光源做成场景参数）；②"让细节随着 noise 场时间变化而变化，
用 shader 内时间，不要逻辑帧注入"（追问定为**分两层、不同风速**，缺省关）。
口径、实测与踩到的坑全在 `06-clouds.md` **§60** 与 **§61**。

- **光源**：删 `SUN_DIRECTION`；新增 shader 库 `px_render/assets/shaders/light.wgsl`
  （`sun_light()`：先点光源（走 `bevy_pbr::clustered_forward` 的三跳去查 `clustered_lights`），
  没有才退回第 0 盏平行光）；`spawn_lights` 换成 `PointLight`，位置/色/强度进 planet part
  （`light_position` / `light_color` / `light_intensity`，缺省 = 旧平行光的坐标与照度）；
  影子跟着灯的种类走（cube / 级联）；**地表云影那条解析解一个字没改**。
- **细节风**：`billows` 两层各加一份随 `globals.time` 的**有界正弦**偏置（时间尺度写在 WGSL 里、
  两层不同），场景参数 `wind` / `wind_skin`（幅度，缺省 0 = 不动）。偏置存在 `var<private>` 里，
  片段入点开头 `arm_wind()` 设一次 —— 探针的 compute 入点不碰它，所以空 bind group 0/1 照样跑。

**已验证**：

- `cargo test -p px_render` 全绿（含 shader 门：三个 shader，`clouds.wgsl` 1606 行 HLSL /
  `surface.wgsl` 672 行，上限 4000）；`cargo check -p px_render --all-targets` ✅。
- 出图（Vulkan / 2240×1400）：`orbit-soft` / `-wind` / `-nocloudshadow` / `-noshadow` 两轮一批，
  管线 0 失败、没有"等管线超时"。
- 判据（`uv run target/pixdiff.py`，numpy，0.2~0.4 s 一对）：
  - **换光源** 39.08% 像素、p99 45、max 227；**亮度标定**：近天底比值 0.97~0.99（远处 0.70~0.85 = 1/d² 与受光帽收缩）；
  - **云影**（点光源时代）10.89%、p99 35、max 79；**cube 阴影** 1.71%、p99 5、max 231；
  - **细节风**：`orbit-soft-wind` 两轮 13.27%、p99 85、max 222；**对照 `orbit-soft` 两轮 0 像素**。
- `tools/px.ps1 -Target field_dual` **全绿**（新 import 没把探针的空 bind group 0/1 打挂）。

**没验证 / 待办**：

1. `tools/px.ps1 -Target gradient` **有 1 个 check 红着**（`the_residual_is_attributed_to_one_channel`，
   `[简化]` 夹具里解析路径 vs 值路径的噪声值差 2.26e-1，判据要求 < 1e-6）。推理上**不是这次引入的**
   （那条夹具直接调两个这次没改的噪声函数），但没有 stash 基线实测 ⇒ 谁再动噪声库之前先把这条查清（§61.4）。
2. 点光源的**多灯累加 / spot / rect / 灯间遮挡**都没做（现在取"cluster 里最近的一盏"）。
3. `--sheet` 12 视角、`soft-e*` 三个不透明度档、环影（`rings > 0`）这一轮仍没出图。
4. 光源换了 ⇒ `orbit*` 的旧基线像素全作废（要重出再对账）；`soft-e*` 之间互比仍有效。
5. 云与大气**仍不乘 `view.exposure`**（§59.2 末尾那条），这次没动。

**review 用的图**（`target/`，2240×1400）：`wind3-shot-r1-orbit-soft.png`（全开）、
`-nocloudshadow.png`、`-noshadow.png`、`wind3-shot-r1/r2-orbit-soft-wind.png`（两轮 ⇒ 细节在动）、
`light1-shot-r1-orbit-bare.png`（点光源下的裸行星），差异图
`pixdiff-light.png` / `pixdiff-cloudshadow-point.png` / `pixdiff-shadowmap-point.png` / `pixdiff-wind.png`。

### 9.1.4 本轮（2026-09-15，`fix/pipeline-fail-fast` worktree）：坏管线当场拒，不许一直 pending

**用户的两条**：①把 `feature/cloud-surface-perf` 合进 v2；②"server 请求遇到坏管线要提前退出
而不是一直 pending"。追问定下的口径：**失败当场拒绝，不能靠超时**；超预算时**只让这一步请求
失败退出、不终止管线**（再请求一次可以拿到）。

- **① 已合**：那条线的 tip `ffcef4b` 本来就是 v2 的祖先，真正没合的是 worktree 里
  **63 项 / +12581−1356 未提交改动**（全 git 只此一份）⇒ 先落成一个提交 `52298d9`，再
  `--no-ff` 合进 v2 = `f7da895`（零冲突；v2 那处编不过的 `ready.get()` 残留按用户裁决丢弃）。
  合并结果复验：`cargo check -p px_render --all-targets` ✅、六个 CPU crate `--all-targets` ✅、
  `cargo test -p px_render` 与五个 CPU crate 全绿。
- **② 已做**：口径、落点表与实测全在 `08-renderer.md` **§62**。要点：`pipeline_gate` 成了出图前
  唯一的闸（能证明坏就当场拒；只是没编完就有界地等，超预算**不出图**）；`accept_jobs` 在搭场景
  **之前**就查失败明细与 shader 库装载态；等待预算从"全局 `Ticks`"改成"这一步的 `rebuilt_instant`"。

**已验证**（Vulkan / RTX 3060 Laptop / 场景 `orbit-soft`）：坏 shader 库 ⇒ 请求 **1~2 s** 被拒
（点名管线 + naga 原文、不出图）；修好后**同一个服务**再请求成功（357843 字节 /
`de36e672a30b502f…`，没重启）；把预算临时改成 300 ms ⇒ 第一次请求被拒、同服务第二次成功
（管线没被终止）；`cargo test -p px_render` 全绿。

**没验证**：Bevy 那条"无限重试"支路本机造不出来（两种造法都被 naga_oil 放过）⇒ "等超预算"
只有人为把预算改成 300 ms 那一次实测。`drive_stable` 的 `Assets` 相位、viewer（`--view` /
`--show`）没接这道闸。

### 9.1.5 本轮（2026-09-15，`feature/soft-cloud-perf` worktree）：代理 + 软

**在哪条线上**：`.worktrees/soft-cloud-perf`（分支 `feature/soft-cloud-perf`，从 v2 的 `7a300e6` 拉）。
口径与实测全在 `06-clouds.md` **§63**。

**用户的三条**（2026-09-15）：①**"需要：代理 + 软"** —— 粗代理几何接到软云档
（`gradient = 2`）上；②追问后定 **代理要进 `orbit-soft` 本体**（成为软档默认）；
③接着做**软档循环内的上界早退**。窗口 review 排在最后再起。

**改了什么**

- `art/scene/orbit-soft.toml`（本体）：clouds part 加 `proxy = "clouds::proxy"` 成员
  ＋ `bound = 1`。**渲染侧一行没改**；`clouds`/`planet` 两张图这次重烘**全部命中**
  （15/15，键没变）⇒ 代理的形状不用重烘（`art/clouds/coarse.toml` 的 τ/inner/outer/形状参数
  与这一档本来就逐项相同）。
- `art/shaders/clouds.wgsl`：软分支的步进循环里加**五行**（算完 `cover` 之后、`billows` 之前）——
  `if soft && params.bound != 0u && shape_of(cover, medium.altitude, 1.0) <= params.surface_level { continue; }`。
  上界够不着等值面 ⇒ 那一步的 `smoothstep` 恰为 0 ⇒ `visible = 0`、透射率不变 ⇒ 跳过与算出来
  **逐位相同**，省掉 `billows` ＋ 那一步的法线 ＋ **一次 shadow map 采样**。
  `params.bound` 缺省 0、且只挂在 `soft` 上 ⇒ 体积那条老分支与所有老场景一个像素都不动。
- 消融档：`orbit-soft-shell`（球壳、无 `bound` = §63 之前的软档内容）、
  `orbit-soft-proxy`（只有代理）。量完那个临时档 `orbit-soft-proxy-bound` 已删（折进本体了）。

**已验证**：

- **逐字节（早退）**：`orbit-soft-proxy` ↔ `orbit-soft`（只差 `bound`）
  **2240×1400 与 480×300 两个分辨率哈希全同**；且折进来之后 `orbit-soft` = `04fe9dac8042a721…`
  （= 量的时候那档的哈希）、`orbit-soft-shell` = `11b0a659bb71ad47…`（= §63 之前的 `orbit-soft`）✓。
  改 shader 前后重出球壳档也逐字节相同 ⇒ 对没写 `bound` 的场景零影响。
- **像素（代理）**：有差 227,733/3,136,000（7.26%），但 **90.5% 恰好只差 ±1**、|Δ|≤2 占 99.53%、
  >20 只有 195 px。云掩码（Δ vs `orbit-bare` > 3）：**859,745 → 859,741**，丢 40 / 多 36，
  Δ>8 的最大连通块 **15 px**（没有 ≥100 px 的块）⇒ **不是 §51.14 那种"轮廓丢一圈"**。
- **帧时间**（Vulkan / GPU p50 / 协议 v11 / 四臂同批 / 两轮 / 逐轮换序）：
  `bare ~0.67｜orbit-soft-shell 14.46/14.53｜orbit-soft-proxy 12.60/12.68｜**orbit-soft 7.53/7.55**`
  ⇒ **云净成本 13.83 → 6.87 ms（−50.3%）**：代理 −1.85、早退 −5.10，两条近似相加。
  三次会话跨批复现 ≤0.15 ms。`orbit-soft` 的 7.5 ms 是目前量到**最省的云路**
  （§51.20 表里最省的 `orbit-proxy` 是 +10.04）。
- 门：`cargo test -p px_render` 全绿（含三个 shader 的解析/校验/体量门）。

**为什么代理只值 13.5%**（§63.4，这条定死了"下一刀切哪里"）：软档的 `chord`/`steps`/起始点
**全由解析壳定**，代理只能剔 fragment；而被剔掉的那些 fragment 在软档里每步只做 `coverage_of`
就 `continue`（`billows`/法线/shadow 采样都没发）⇒ 同样的剔除，硬表面值 7 ms、软档只值 1.85 ms。
**真杠杆在循环里** —— 也就是上面那五行。

**没做 / 待办**：

1. **这条线还没合进 v2**（全在 `feature/soft-cloud-perf`）。
2. ⚠ **老的软档消融族口径变了**：`orbit-soft-plain` / `-noshadow` / `-nocloudshadow` / `-wind`、
   `soft-e*` 文件没动 ⇒ 它们现在指的是**折叠前**的软档 = `orbit-soft-shell`（再叠各自那个开关）。
   谁要用它们做配对，要么按 `orbit-soft-shell` 读，要么把它们重新折到新本体上。
3. `--sheet` 12 视角、`--view` 窗口、`soft-e*` / `orbit-soft-wind` 都没跟着重出（改动没碰到它们）。
4. 只测了 `review` 相机的第 1 个视口（`--perf` 不出 sheet），相机相关性没扫。
5. 早退只测了 2240×1400 / 480×300 两个分辨率与 `orbit-soft` 这一档；`shadow = 0`、`steps` 变小
   这些档没扫（上界那条论证与 `shadow` 无关，但没实测）。

### 9.1.6 同一天接着的一条**用户报的缺陷**：窗口里的「正中心接缝」——**已定位、已修、已并入 v2**

**用户口径**：**"背面光照会出现跳变"**（2026-09-15，配窗口截图）；随后追加 **"不需要兜底，宇宙里没有平行光"**
与 **"确保平行光被删除了，从渲染器里"**。全过程与证据链在 `06-clouds.md` **§64 / §64.9**。

**根因（探针实测）**：`px_render/assets/shaders/light.wgsl::sun_light` 取灯走的是 **Bevy 聚类网格**
（`view_fragment_cluster_index` 定格子 + `view_z_to_z_slice` 定 z 切片）。相机拉到 `distance = 14` 时，
大气沿 chord 的 5 个采样点落在与近处完全不同的切片里，**有一半查不到灯** ⇒ 退回兜底（方向 (0,0,1)、
颜色 0）⇒ 云/大气的受光沿网格边界**硬跳一档**。实测左右两半"5 个采样点全拿到灯"的比例是
**57% 对 4.4%**，而"该格有没有灯"两边都是 55% ⇒ 不是"没有灯"，是**部分采样深度查不到**。

**修法**：`sun_light` **不问网格**，直接取场景那盏灯（`clustered_lights.data[0]`），**并且没有兜底**
（没有点光源就是没有光：颜色 0、`point = 0` ⇒ 全黑）。
⚠ **不能拿 `position_radius.w` 当"有没有灯"的阈值**：那格不是 range（实测对这盏灯读到 0）。

**平行光从渲染器里删干净（§64.9）**：出图路那盏遗留的 `DirectionalLight { 9000 lx }`（`main.rs` 两处）、
`sun_light` 的方向光兜底、`FAR_LIGHT`、`view_z_of`/`is_orthographic`、`clouds/surface` 里
`fetch_directional_shadow` 的 **else 死支**、离线门桩里的方向光结构 —— 全删；`AmbientLight` 留着（不是平行光）。
**验证**：出图路四张判据图**删前删后逐字节相同** ⇒ 删掉的全是死代码。

**验证（单变量）**：① 复现视角那条台阶 **+1.863 → −0.014 / −0.304**；② 出图路判据图**逐字节不变**；
③ 窗口**初始轨道**修前修后**除 OSD 外 0 像素**变；④ `cargo test -p px_render` 全绿。
⇒ **§59/§60/§63 的图与判据全部保持有效**。
⚠ `clouds` / `surface` 的**内容键变了**（`cc4111d3d0f4 → 36def398451e`、`d59c0d736f3d → 9453bf54f629`）
⇒ 产物键跟着变（`orbit-soft`：`d1ca4eb89781 → d3bcdfcc11b9`）；旧笔记钉的**产物键**按新键读，**图的哈希没变**。

**新加的窗口相机 API**：`px_render --where` 读回当前方位（并打印可直接粘回命令行的 `--place y,p,d`），
`px_render --place y,p,d` 把窗口摆过去（不换场景、不重烘）。

⚠ **方法论（这一轮最贵的教训）**：窗口的轨道相机**是用户拖的**，且这个缺陷**依赖相机** ——
我早期三次"换了缝就没了"的读数都是**没做自检**、在漂移后的相机上量的，**全部作废**。
以后量窗口固定三拍：**连拍 A、B、A**，第 1 与第 3 张**除左上 OSD 外逐像素相同**才认。

**没做的**：多光源仍不支持（取第 0 盏；§64.9.3 记了以后怎么改）。分支已 `--no-ff` 并入 `v2`。

### 9.1.7 本轮（2026-09-15，`.worktrees/generic-render` 分支 `feature/generic-render`）：**通用渲染**

用户原话：「目前 pcg->render 构架仍然不是通用渲染，`.pxart` 改成通用渲染」。口径与全部判据在
`08-renderer.md` **§65**（这一轮新增），这里只写"做到哪了"。

**形状**：`.pxart` 从"参数 + part 名"变成**渲染文档**（`SCENE_SCHEMA = 2`）：物体（几何 + 材质 +
世界系变换）+ 灯表 + 环境（环境光 / 天空盒）+ 相机表 + 期望标签。渲染器里**再没有**
planet / clouds / atmosphere 这些词：`KINDS` / `assembler` / `SceneBuild` / `PlanetSpec` /
`spawn_planet` 与 `planet.rs`、`clouds.rs`、`surface.rs`、`atmosphere.rs` 全删，代之以
`reflect.rs`（按产物那份 WGSL 反射参数块）+ `material.rs`（一种通用材质，固定超集绑定布局）
+ `scene.rs`（通用装配）+ `mesh.rs`。

**搬走**：色板贴图 / 覆盖度立方图 / 星空 / 环（含 mip 链、极点滤波）→ `px_ops::generate`
（**8/8 逐字节相同**，判据见 §65）；行星/云/大气的语义 → `px_graphs --bin scene`（配方文件形状
**没变**）。倾斜、太阳射程系数、天空盒亮度、云影 gain 也都搬到烘图侧。

**已验证**（同一 worktree、同一工具链，只差代码；`--cam 0,5,3.2`、960×640）：

- `cargo test -p px_protocol -p px_ops -p px_graphs -p px_verify -p px_render` 全绿；
  `cargo check -p px_probe --bins` 过。
- 出图：`orbit-bare`（无云）与 `orbit-allmiss`（云全 discard）**逐字节同像素**；
  `orbit-soft` 22/614400、`orbit-soft-nocloudshadow` 33/614400 个像素差，**最大通道差 2**；
  12 视角对照图 944/2352000（0.04%）、最大 17。
- 出图路、管线门、`--sheet`、报告（`has_cloud` 判据靠文档里的 `expects = ["clouds"]`）都在跑。

**没验证 / 没做**：

1. **带云那两档的 20~30 个像素没归因**（所有输入与数学都逐项验过相同；唯一剩下的差别是绑定
   布局）。§65 末尾记了钉死它需要的那个专门实验。
2. `--view` / `--show` 预览窗口这一轮**没跑**（收尾时给用户拉起来看过一次，见下）。
3. 三个探针 bin：`device` / `dual_noise` 跑过（✓，见下面的补验块）；`field_dual` / `gradient`
   这两个重的**没跑**；`px_probe` 整包只保证编译过。
4. 环（`rings > 0`）**有实测图了**（本轮补验，见下），但**没有逐像素基线**：迁移前那条路
   （Bevy 内建 `StandardMaterial{unlit}`）没出过图，两条路的曝光处理也不同。
5. `--stream` 那条经济世界的老路（`build_world_scene`）**一个字没动**，还在。
6. `.agents/notes` 里其它篇（02/03/06/07/09）仍按旧形状描述 KINDS/槽/`spawn_planet` ——
   这一轮**只**新增 §65，没有逐篇回改。

**本轮补验（同一轮内追加）**：

- **环**（唯一一条从没跑过的路径）：新增配方 `art/scene/orbit-rings.toml` 并出图，
  `placeholder_px = 0`、管线 0 失败、环面与行星同倾斜、近侧压住行星远侧被挡 —— 图
  `target/rings-shot.png`（1200×800）。细节见 `08-renderer.md` §65.1。
- **harness 的一致性闸门**：`Get-SceneShaderMembers` 原来按 v1 的 `parts[]` 读 ⇒ v2 下
  返回空表、闸门**静默失效**。已改成读 `objects[].material.shader`，且读到 0 条就抛错；
  拿一份 v1 旧产物当反例验过（§65.2）。
- 探针：`cargo run -p px_probe --bin device`（✓ 全部通过）与 `--bin dual_noise`（✓ 2/2）
  都跑过；`field_dual` / `gradient` 这两个重的**没跑**。

**已并入 `v2`**：`e600ae9`（`--no-ff`，无冲突；用户要求"merge to v2"）。

**并入后的冒烟**（在**主工作区**的 v2 上，不是在我的 worktree 上）：

- `cargo test -p px_protocol -p px_ops -p px_graphs -p px_verify -p px_render` 全绿。
- `cargo run -p px_graphs --bin shaders` → `clouds=b52f7a0d391b / atmosphere=381a0bd89054 /
  surface=9453bf54f629`；`--bin scene orbit-soft` → 产物键 `13566f8817bd…`（与我 worktree 里
  那把**同一个键**：键 = 内容，说明合并搬过来的就是同一份东西）。
- 出图：`target/smoke-v2.png`（960×640、`placeholder_px = 0`），sha256
  `eeda0f66be6967b5b2c8d212d6f3d205324b41f46008b852cdaec9e1589e600b` —— 与合并前分支上那张
  **逐字节相同**（同产物 + 同代码 ⇒ 同图）。


## 9.2 已经能跑什么

> ⚠ **本节的命令行是旧接口（原文保留）**：内容旗标已在 §52 / P10 删除，内容只走 `--scene`；用法见 `09-instruments.md` §40 与 `08-renderer.md` §52。下面的 `--planet / --mesh / --clouds / …` 只能当历史形状看。

```
cargo run -p px_graphs --bin planet     # 烘星球（height + surface mesh）
cargo run -p px_graphs --bin clouds     # 烘云（mixed + coverage + 三个 slope）
target\debug\px_render.exe --serve --width 512 --height 352
target\debug\px_render.exe --planet <H.pxart> --mesh <M.pxart> --palette rocky `
   --clouds <C.pxart> --cloud-slope <SX,SY,SZ> --width 512 --height 352 --sheet target/sheet.png
```

`--sheet` 用产物自带的相机表（`.pxart` 的 `AssetManifest.cameras`）出**一张多视角对照图**，
一次请求一个场景 N 个视口。给了 `--sheet` 就不要再给 `--cam`。

## 9.3 已验证 / 未验证（**这条最要紧**）

**已验证**：

- 编译门：`cargo check -p px_protocol -p px_ops -p px_graphs -p px_verify -p px_render -p px_probe --all-targets` 全过。
- 测试（CPU，秒级）：`cargo test -p px_protocol` 28 个、`cargo test -p px_render` 11 个
  （含 6 条 `art_cache` 单测）、`cargo test -p px_protocol -p px_ops -p px_graphs -p px_verify` 全绿。
- GPU 冒烟（DX12 / RTX 3060 Laptop / 44 条管线 0 失败）：
  - `--sheet` 12 格都画了、构图正确（第 3 行是棱/角/面心特写）、无 shader 报错；
  - 同一场景连发 4 次：**冷 1867 ms → 热 1525 ms**，热/热抖动 ±6 ms，
    **四张图 SHA256 全同**（`42ff067e…`）；
  - 程序化球面那条路（不带 `--mesh`）：`sphere`/`texture` 冷→热全命中；换 `--palette ice`
    时**场仍命中、球面与贴图重造**；冷落淘汰真的触发。

**未验证**：

1. **三个探针 bin 从没跑过**（`field_dual` / `gradient` / `device`）。它们守着梯度对错的**唯一**判据，
   而这条判据自 §46 起就没再被执行过。
2. **viewer 那条路**（`--view` / `--show`）本轮没跑：`poll_field` 的「mtime 动过但指纹没变 ⇒ 不重建」
   分支只有 `cargo check`。
3. `--sheet` 的对照图**不可与旧的 `target/probe-*.png` 逐像素比**：相机语义从「世界 yaw/pitch + 倾斜」
   换成「局部方向」，整体转了 19.5°（`SYSTEM_TILT`）——这更正确，但旧图作废。
4. §40.3 那条**窗口比 `--serve` 暗**仍未解释。

## 9.4 第一次冒烟的顺序

```powershell
# 0) 最便宜的 GPU 门（约 10 秒）：探针能不能起、后端是不是 DX12
cargo run -p px_probe --bin device

# 1) 重烘（相机表进了缓存键 ⇒ 会得到新的 CAS 路径）
cargo run -p px_graphs --bin planet     # 记下打印的 <HEIGHT.pxart> 与 <MESH.pxart>
cargo run -p px_graphs --bin clouds     # 记下 <mixed.pxart> 与三个 <slope.pxart>

# 2) 常驻服务；等日志出现「渲染管线全部就绪：共 … 条，失败 0 条」
target\debug\px_render.exe --serve

# 3) 一张对照图
target\debug\px_render.exe --planet <HEIGHT> --mesh <MESH> --palette rocky --sheet target\sheet.png
```

⚠️ 改代码前先停服务（`target\debug\px_render.exe` 被占用会让 `cargo build` 报「拒绝访问」）。
删 `target/render-server.json` 即可（4 秒内自查退出）。

## 9.5 下一步（按价值排）

**P4｜`tools/frame-probe.ps1` / `probe-clouds.ps1` 的 fail-fast** —— ✅ 已做

实证：`target/fp-*.log` 四个文件 mtime 精确相隔 ~60 s ⇒ 那一次 `NoiseSample` 编译失败
让每个 case 白等满 60 s，而真错误被 `continue` 吞了。现在两个脚本都 dot-source
`tools/harness.ps1`：按**图名 + 节点名**从 `target/pcg/<图>/manifest.json` 解析产物
（缺图/缺节点/产物不在 ⇒ 抛错并列出可选项）、客户端退出码非 0 与 `Refused` 一律硬失败、
按日志的「渲染管线全部就绪」等就绪、按租约 pid 停服务。顺手发现并记进 §51.1 的更深一层：
**六个默认路径全是内容哈希，早就死了**，而脚本吞退出码 ⇒ §46.1 那张表是空转测出来的。

**P5｜`--cloud-ablate` 进 per-request `View`** —— ✅ 协议侧已加（`View.ablate`），
渲染器侧仍读服务级的 `ServerAblate`，等 P10 一起收尾。

> **P5 状态**：✅ 已随 P10 收尾（`ServerAblate` 全删，消融改成 clouds part 的 `ablate`）；原文保留。

**P6｜预热与管线**

只 warm 真正会用到的管线（现在 44 条里混着 `StandardMaterial` / `Skybox` 变体）；
评估 wgpu 的**磁盘管线缓存** —— 它是把「每次重启服务 15–25 s」降到 <1 s 的唯一现成手段。

> **P6 状态**：⚠ §51.19/§51.20 已查实结论 —— **本机不值得做**（DX12 上 no-op；要用只能 fork `bevy_render`），等上游 bevy#19809；原文保留。

**P7｜删死代码**：`atmosphere.rs::sync_cameras` + `AtmosphereParams.camera_x/y/z`
（shader 早就改读 `view.world_position` 了，它却把最后一台相机的世界位置写进**全局** material uniform）。
注意 uniform 布局要和 WGSL 同步改。

**P8｜`tools/probe.ps1` 退休** ⇒ 直接 `px_render --sheet`。删之前先跑一次 9.4 的第 3 步、和旧图并排比一眼。

**P9｜缓存只覆盖了资源的一半**：两个 `ico(64)` 壳（云、大气）、环、`ring_image(1024,4)`
仍是每请求现造 —— 相对那几百万 texel 是零头，但不是零。

**P10｜删掉内容旗标（§52）** —— ✅ 已做（含相机表与批量）

`render::Scene::Planet` 与那串内容旗标（以及 `Options::planet_spec()`、`ServerAblate`）全删；
`--scene` 可给多次，每步的 `--out/--cam` 配在它前面那个 `--scene` 上；
`Scene::Sequence { shots: Vec<Shot> }`（`Shot { scene, out, cam? }`）批量出图，一步一行「出图：」；
`--sheet` 变裸开关、用 `.pxart` 里那 12 台评审相机；消融改成 **clouds part 的参数**
（`ablate = "surface"`），`params.steps` 与新增的 `params.surface_level` 现在真被 WGSL 硬表面路径读
（之前场景里写的 `steps` 对硬表面路是个谎）。`SCHEMA_VERSION 8 → 9`。

⚠ **这一段最值钱的发现**：每请求 `install + reload` 会把管线打回重编，而出图只等 6 帧 ⇒
那一张图上云直接消失，**且退出码 / 颜色 / 图片大小 / 编译 / 单测全绿**。判别只能靠哈希
（三张图逐字节相同）。修法：只在槽内容真变了时才装 + `drive` 等管线入队（有界 40 帧）。详见 §52.3。

**P11｜viewer 走场景** —— ⚠ 只到编译级

`ViewRequest` 已换成「场景产物路径 + 内容键」，窗口只在键变了才重建，`--show` 只推场景路径；
但**没有开窗口实跑**。

> **P11 状态**：viewer 的契约与坑已由 `09-instruments.md` §56 / §56.1 补齐（含"起窗口必须脱离"、判档日志、修复没进 exe 的两个坑）⇒ 本节状态以 §56 为准；原文保留。

**P14｜装过 shader 之后要等 READY**：现在只等"管线入队被看见"（有界 40 帧，超时带警告照常出图）。
如果一份 WGSL 真换了内容，那一张仍可能赌输。要彻底就得等 READY，但要处理"管线永远编不出来"时别把任务挂住。

> **P14 状态**：⚠ v11 起主路径改成"等条件"（管线全部就绪 + 8 渲染帧，见 `09-instruments.md` §57/§58）⇒ 本节"只等入队（有界 40 帧）"是旧形状；原文保留。

**P15｜删死码**：`--scatter` 删掉后 `planet::spawn_scattering`（Bevy `AtmosphereSettings` 那条）与
`planet::check_scene` 没人调了。

**P12｜把改 shader 的近路做回工具层**：引擎那半截（`slots://` 稳定槽 + `reload`）已经在了
（§52.3 查实它就是热重载本体）。缺的是「监视 `art/shaders/*.wgsl` → 重烘 `shaders` +
`scene` → 用新场景路径再请求」这一步；做完就把「改一个字等 1 秒」还回来，而且比原来更硬
（场景键钉住当时用的是哪版 WGSL）。

**P13｜把 §51.4 那三条量完** —— ✅ 已做完（§51.7 + §51.9）

上界早退（`bound = 1`）逐字节不变、中位 30.00 → 18.56 ms（−38%）；
四档归因（§51.9）：**梯度只值 1–2 ms，大头是全 miss 的步进（7.8–13.7 ms）**
⇒ 优化该往"少走步"做，不该往"优化梯度"做。
剩：分辨率扫描（要避开 2240×1400 的驱动天花板）。

> **P13 状态**：⚠ 它引用的 §51.7/§51.9 **已被 §51.12 作废**（泄漏期数）；修后重测见 §51.12/§51.13，且**步数曲线与分辨率扫描仍未重做**；原文保留。

**P16｜`bound` 的读法要严**：现在用 `optional_number(...)? as u32`，`bound = 0.5` 会被截断成 0
而不报错。缺省 0 是对的（老场景没有这个参数），但写了非整数应当报错。

**P18｜云的代理几何接通了（渲染侧）** —— ⚠ 判据差一点 + 撞到一个既有显存泄漏

`art/scene/orbit-proxy.toml`（与 `orbit-surface` 只差 clouds part 多一个 `proxy` 成员）+
装配器按成员取代理 mesh + shader 硬表面分支改成"壳入射点为绝对栅格、代理落点只定起始下标、
可双向"。详见 §51.11。三件事要接着做：

1. **代理 mesh 的缠绕朝里**（有向体积 −0.4709）。现在渲染侧读的时候按有向体积自动翻面
   （`planet.rs::outward_winding`，星球那张 +4.25 不动）。**要么**就这么留着（以后谁修了烘焙侧
   也不会双重翻面），**要么**在 `px_mc` 里把缠绕翻正 —— 后者换内容键，且渲染侧那段自省逻辑
   仍然安全。
2. **逐字节判据没达到**：差 1924 px @480×300 / 12901 px @2240×1400，其中 98% 只差 1–7 个
   通道值。控制实验（球壳只换镶嵌 `ico(64)→ico(60)`）差 1917 px、直方图相当 ⇒ 残差来自
   fragment 落点变了 ⇒ `ray` 末位变 ⇒ 量化翻一位，**不是**搜索逻辑。真要逐字节得把光线改成
   从像素反推（会动到 orbit-surface 的像素）。
3. **2240×1400 长跑必炸：显存线性泄漏 ~150 MiB/s（云档），无云档是平的、480×300 也是平的**
   ⇒ 与像素数成正比、只跟云有关；`orbit`（体积，本段没碰）漏得一样多 ⇒ 既有缺陷，不是代理带来的。
   修掉它之前 `frame-probe` 在 2240×1400 上跑不满 n=8（n=4 可以）。

> **P18 状态**：第 3 条（显存泄漏）**已闭环** —— 修法与验证在 §51.12，根因与上游 issue 文字在 `09-instruments.md` §55；第 1 条（缠绕）仍按渲染侧自动翻面处理，第 2 条（逐字节判据）仍未达到（§51.11）；原文保留。

**P17｜仪器要先热身一张再取判据**：冷启动后第一个请求仍可能丢云壳（装 shader 打回重编的窗口，
§52.3 的同一个坑）。已有一条实测留证（`p13-surface.png` = 无云那张）。
⚠ **补充（本段实测）**：这个窗口比想象的长 —— 装完 shader 之后**同一个批量请求里背靠背的每一步**
都还在窗口里（`p480-orbit-proxy.png` 三张全无云）。**要按"热身一张 + 停顿几秒 + 再出判据图"
做**，不是往同一批里多塞几步。

> **P17 状态**：⚠ v11 起判据侧由 `has_cloud` 兜（`09-instruments.md` §57/§58.1）、"热身 4 s"已退休（§51.18）；本节的手工步骤属旧协议/回退路（`--windows`/`--drop`）的形状；原文保留。

### 一条改变排序的实测

资源缓存（§50）省下的是 **1867 → 1525 ms ≈ 340 ms（18%）**，剩下 82% 是**渲染本身**
（12 个视口 × 体积云 56 步）。早先记下的「探针巨慢的真正大头 = 服务端每请求全量重建场景」
**只对了 18%** —— 真正的大头在 shader 里，该用 §41 那台仪器去量。
所以 P4（白等 60 s）仍然值钱，但「服务端重建场景」不该再排第一。

## 9.6 常用命令

```powershell
.\tools\px.ps1 -Target test                    # 快速测试链（默认 members，不碰 bevy）
.\tools\px.ps1 -Target test-all                # 全量（含 px_render / px_probe，慢）
.\tools\px.ps1 -Target check                   # 只 check 不产 GPU 的那三个 crate
.\tools\px.ps1 -Target device                  # 最便宜的 GPU 门（约 10 秒）
.\tools\px.ps1 -Target field_dual              # §46.3 的 arbiter（唯一梯度判据）
.\tools\px.ps1 -Target gradient                # 探针冒烟 + 逐通道归因
.\tools\px.ps1 -Target planet   -Level opt     # 烘星球图；-Level opt 只提升本地 crate，不重编 bevy
```

## 9.7 文件地图

| 想知道什么 | 看哪个文件 |
|---|---|
| 路线裁决与「为什么不那样做」 | `art/01-decisions.md` |
| PCG 图程序怎么写、缓存键与版本号 | `art/02-pcg.md` |
| 产物里有什么（域 / 指纹 / 相机表 / diff） | `art/03-assets.md` |
| 球面网格、位移、法线、mip、接缝与极点 | `art/04-geometry.md` |
| 天空与大气（含 Bevy 散射大气的挂起状态） | `art/05-sky.md` |
| 体积云与覆盖度 | `art/06-clouds.md` |
| 云密度场的梯度：组装形式与判据 | `art/07-gradient.md` |
| 渲染器怎么跑（离屏 / 服务 / viewer）＋ 资源缓存 | `art/08-renderer.md` |
| 改完怎么验（热重载 / review 回路 / 单帧时间 / 探针） | `art/09-instruments.md` |
| 不变式（每条都付过代价） | `art-framework.md` 顶部 |

代码入口：`px_protocol/src/art.rs`（产物与协议）、`px_ops/src/lib.rs`（算子与缓存键）、
`px_render/src/{main.rs,planet.rs,art_cache.rs,clouds.rs,atmosphere.rs,surface.rs,slots.rs}`、
shader 库 `px_render/assets/shaders/{common,noise,light}.wgsl`（**库住这里**，
入口 shader 住在 `art/shaders/`）、
`px_probe/src/{common.rs,probe.rs,field_dual.rs}`。

**判据图怎么比**：`uv run target/pixdiff.py -A <png> -B <png> -Bands 4,8,20 -Out <diff.png>`
（numpy，313 万像素一对 0.2~0.4 s）。
⚠ 别再用 `target/pixdiff.ps1` 那版 PowerShell 逐像素 —— 同一件事要几分钟，
而"仪器慢"会直接变成"少测几档"（§59.3 注）。

**P19｜用修好的仪器重测 2240×1400（§51.12）**：显存泄漏修掉之后 n=8 才第一次真正可跑，而**泄漏期间量的**那些数（§51.3 步数曲线、§51.7 的 −38%、§51.9 的四档归因）**全部作废** —— 那时打出来的"帧时间"是 CPU 提交速率（~30 fps 的假象），GPU 真实只有 ~16 fps。修后的真实中位：`bare 14.76｜orbit 52.15｜orbit-surface 71.63｜orbit-proxy 49.43｜orbit-bound 44.09 ms`。**优先重测这几条**：
1. 代理 vs 球壳（`orbit-proxy` vs `orbit-surface`）到底省多少；
2. 上界早退（`orbit-bound` vs `orbit-surface`）的收益与逐字节判据；
3. 步数曲线（现在步数是场景参数，好做）；
4. 四档归因（梯度 vs 全 miss 步进）。
⚠ ~~还有一条"没闭环"：为什么偏偏**有云**才把 wgpu 的间接绘制校验抬到约 2 笔/帧~~ —— **已闭环（§55）**：
无云档**也**产生间接绘制（3 笔/帧 vs 有云 4 笔/帧），云不是原因；有云只是**让 GPU 成为瓶颈**，
CPU 于是无限跑在前面，每个在飞提交钉住 wgpu 那对 1 MiB 校验缓冲。上游 issue 文字（含最小复现、
分配点行号、三选修法实测）在 `09-instruments.md` §55.4，**没有真去提交**。
⚠ 顺带更正：§51.12 原来那次"修前后逐字节相同"用的是**两张无云图**（P17 的重编窗口），
已按 P17 重做并留证（480×300 `orbit-surface` = `DF6D5BC1…` / 124433 字节，修前/修后构建相同）。

> **P19 状态**：**1/2/4 已做**（修后重测 = §51.13，另见 §51.12）；**3（步数曲线）未见修后重扫**。⚠ 且 §51.20 判定产品口径是 Vulkan、并注明"DX12 时代按 GPU 毫秒算的百分比改进在 Vulkan 下余量小得多 ⇒ 优化判断需重做" ⇒ 本节的"重测"在 Vulkan 口径下**部分仍待做**；原文保留。

**P20｜清掉「细代理」这条弯路的残留**：`px_mc/src/lib.rs` 里为"几何外扩消轮廓丢片"加的 `pub offset`（默认 `0.0`，老路径逐位不变）+ 310–354 行的外扩块，是给**已排除的 `field=final` 路径**打的补丁 ⇒ 若确定不再回头，删掉它（与 P15 死码一起清）。`art/clouds/proxy_fine.toml` 里的 `offset` 已退回（不再设值）。`art/scene/orbit-proxy-fine*.toml` 两份**留着**当已排除选项的可复现记录（§51.14.1），别当正式档。

> **P20 状态**：§51.14.1 已把该路排除（"不再继续投入"）；本节仍是"若确定不再回头"的条件句，`orbit-proxy-fine*.toml` 按原文留档，`px_mc` 的 `offset` 待清理；原文保留。

**P21｜硬表面云剩下的两笔真实开销**（都不是 mesh 的事，见 §51.14）：
1. **22% 的 fragment 擦面而过** ⇒ 沿线 32 个采样全 ≤ τ，走到栅格末端才放弃（~7.6 ms）。**要先诊断**这 22% 是"代理覆盖了但云确实不在"（正常剔除损耗）还是"云在那儿但点采样漏了峰"（真问题，与 §51.10.3 的 `reach=1` 同族）—— 两者修法完全不同。
2. **命中像素的解析梯度 + 着色 + 透明混合 ~10 ms**（该档 27%，与几何无关）。梯度本身只值 **2.08 ms**（`surface−nograd`）。
