这篇讲什么：这个美术栈为什么长这样、哪些路线被否掉了、否掉的理由是什么。

## §2 PCG DAG 的形态：脚本 / DSL / TS / QuickJS 全否
- **没有任何现成框架同时满足**「纯文本图 + headless CLI + 强制确定性 + 亚秒热重载 + 无 bundler / 无 node_modules」，每一个都要写同样多的胶水还要背它的额外重量。**只借失效语义，不采用**：Ninja `restat`（early cutoff 的最小实现）、Bazel action cache、Salsa red-green（Rust / `ra_ap_salsa`）、Houdini PDG 的 expand/cook 两阶段 + work-item 文件缓存、fogleman/sdf 的 `~/.sdf` 内容寻址缓存、Minecraft density function（真实工业先例：地形就是一棵纯 JSON 的密度函数 DAG）。
- **Houdini**（`.hip` 二进制不可 diff、商业许可与 Indie 格式不通）、**Blender Geometry Nodes + bpy**（`-b` 下 `bpy.app.timers` 不触发）、**Substance `.sbs` / Material Maker `.ptex`**（要它们的运行时）、**通用编排器**（Dagster / Prefect / Snakemake / Ninja / Bazel，多数要 server + DB + daemon，图在 Python、产出是文件）—— 全部不采用。SDF/CSG 小 DSL 的共同选择是「宿主语言 + 少量语法糖 + 一个 CLI」，**不发明新语言**。
- **TS 路线**（Node 24 默认 type stripping ⇒ `.ts` 零构建直接跑）随 §10 走 Rust 栈而作废；它留下的两条结论仍生效：**热重载的是数据、不是代码模块**（浏览器里 `await import(url + '?t=' + Date.now())` 绕 ESM 缓存会泄漏模块）；**禁止双实现**（合法形态只有：只在 GPU 上实现、CPU 侧要数据就 readback / 只写一份 CPU 实现、上 GPU 就烘成纹理；表达式子集同时编到 JS 与 GLSL/WGSL 成本最高，不做）。
- **不引 QuickJS**：标准 QuickJS / quickjs-ng **没有 WASM** ⇒ `manifold-3d`、`three-mesh-bvh`、`meshoptimizer` 在里面跑不了；没有 npm 生态、`fs.watch`、`WebSocket`、`node:crypto`。冷启动也省不到东西：`node -e "0"` **43.3 / 44.2 / 44.4 / 48.0 / 56.4 ms**，`node file.js` 45.5 / 52.4 / 74.5 ms，常驻进程内 1e6 次 `Math.sin` + `fround` **17 ms** —— 热路径是「文件变更 → 重算脏子图 → 推 blob」，全发生在已经在跑的进程内。⚠️ 将来真要「Rust 进程内求值」，正确顺序是 `wasmtime` 跑 WASM 版求值器（wasmtime 能跑 WASM 库，QuickJS 不能）。
- **§2.7 确定性铁律（跨机器，必须遵守）**：禁 `fract(sin(x)*43758.5453)` 这类 hash（`sin` 跨 libm 不同），用整数 hash / 位运算；Rust 侧要可复现就用 **`rand_chacha`**（`StdRng` / `SmallRng` 被官方标 non-portable，任何 release 都可能改值），`rand_distr` 正态要 `libm` 特性；⚠️ **GPU 浮点跨厂商不保证 bit-identical ⇒ GPU 只能做运行时缓存，绝不能当基线来源**，权威烘走 CPU。

## §3 几何库与贴图烘焙
- **几何只引两个库**：`manifold-3d`（v3.5.3 / 2026-09-07 / Apache-2.0 / WASM，`levelSet(sdf,…)` 用 marching tetrahedra 出网格、**只要求 SDF 符号正确**，`extrude/revolve/slice/hull/decompose/simplify/calculateCurvature` 覆盖面板与旋转体；**v3.5.0 起跨平台浮点确定性**；Blender 4.5 已内置它当 boolean 求解器）+ `meshoptimizer@1.2.0`（npm / MIT，`simplifyWithAttributes` 保 UV/法线、`simplifyPrune` 治 isosurface 碎片、`compactMesh` / `encodeFilterOct`；⚠️ `clusterlod.h` 只有 C++ 侧，本仓用实例化 + 多级 index buffer 就够，不需要 CLOD）。走 Rust 栈后几何直接用 **Rust 绑定 `manifold-csg`**，不是 WASM（§10.4）。
- **不要碰**：CGAL（GPL / 商业双授权 + 慢）、OpenVDB / nanoVDB（GPU 侧只有 CUDA / OptiX / OpenCL / OpenGL / DirectX，**没有 WebGPU / WebGL**）、OCCT / opencascade.js（wasm 数十 MB）、**xatlas-web（2024-07-22 已归档）**、Instant Meshes / QuadriFlow（产出非确定性、需人工清理）、OpenSubdiv（无 JS 绑定）、three 的 `SimplifyModifier`（高密度网格上明显差于 `MeshoptSimplifier`）。**也别做**：把行星 / 卫星「统一成 mesh」——「解析球 + 位移」是性能与画质双赢，网格化是倒退。
- **UV：优先不做**（行星 UV 是解析的经纬 / cube-sphere；硬表面走 triplanar / box projection；xatlas 只在「硬表面 UV 真的痛」时离线引入）。**不要把生成的 mesh 当资产传**，传 **seed + recipe**，在消费端用同一套确定性代码重新生成（体积≈0，天然满足严格程序化）。
- **烘焙分三层**：shader 内解析计算（只留高频细节）/ 页内 GPU 烘 → 缓存（cache key = 图 hash，落 IndexedDB / CacheStorage）/ Node-CPU 权威烘（基线、判据、CI，逐位可复现）。⚠️ **GPU 烘跨驱动不保证逐位一致 ⇒ 只能当运行时缓存，绝不能当基线来源**（与 §2.7 同一条）。收益已记账：`shader-compile-stall.md` §4 —— 冷编译 ultra 档 **6–14 s**，残留原因是固定的内联规模（每 program ~17 份 `vnoise` × 24 program）。
- **噪声 LUT 是性价比最高的一招**（不改架构）：加载时生成 **64³ / 128³ RGBA8 3D 噪声纹理**（**64³ = 1 MB、128³ = 8 MB**，全材质共享；1024³ = 1 GB 不可行），shader 里把多 octave FBM 换成「3D 纹理采样 + 手动叠 octave」。**派生贴图**：法线**不烘**（已在算解析法线）；AO / curvature 是唯一真正值得离线烘的一类，且只烘舰 / 站这类 hero 资产。**容器**：暂不引 KTX2 / Basis；float 场沿用 **RGBA32F + NearestFilter + 手写三线性**（float 纹理线性过滤要 `OES_texture_float_linear`，这正是当初选 Nearest 的原因）。**AI 贴图（SD / ControlNet / ArmorLab / Meshy）排除**：非确定性、许可有门槛、云端方案违反「无下载资产」，可作美术离线探索，**不进管线**。
- **逐资产落点**：行星表面 / 卫星 = shader 实时 + 3D 噪声 LUT（烘了会锁死 LOD 细节层级）；小行星带 = 加载时 GPU 烘 8–16 变体图集 + 每实例 `vec4(u0,v0,us,vs)` 选片 + seed 扰动；行星环 / 尾焰 = shader 实时；地表城市 = 加载时烘屋顶 / 窗户图集 + shader 细节；舰船 / 空间站 = 几何靠 Manifold、离线只烘 AO / curvature 且只烘 hero 资产；星空 / 银河 = `CubeCamera` 烘 cubemap（已做）+ 补 `PMREMGenerator` 预过滤当全场景唯一 IBL。

## §4 渲染框架：three.js vs Bevy（已裁决）
- 用户裁决：**浏览器不再是硬约束**（「只要是面向未来的渲染框架，截图方便，重载方便，agent 用起来方便最好」）。落点是 **Bevy + WGSL**（内建 WGSL 资产热重载、无窗口离屏 + `TimeUpdateStrategy::ManualDuration` 帧精确 step、`cargo run -- --scene sun-limb --t 12 --out shot.png --stats json` 一条命令、RenderDoc 可直接用、compute 是一等公民）。若交付形态必须是 web，则 **three.js + 增量 TSL**（r183 `RenderPipeline` 是节点式后处理且带 WebGL2 fallback；`GLSLNodeBuilder` 与 `WGSLNodeBuilder` 并存 ⇒ **TSL 能编到 GLSL**，迁移是增量可回退的；`glsl()` / `wgsl()` 内联可整段搬现有 24 stage）。⚠️ **两个渲染器 = 又一次双实现漂移**（前科：「JS 烘场 vs GLSL 算场必然漂移」「`cloudstat.mjs` 抄 `cover()`」）⇒ 要么整条 3D 都走原生、要么都走 web，两边都要则材质 / 生成逻辑必须单一源（Slang 离线编译到 WGSL + GLSL）。
- **不采用**：Babylon（换栈 = 24 stage + 后处理全部重写；两张牌是 NodeMaterial 的 JSON `serialize()`/`Parse()` 与 ⚠️ 未核实的运行时 GLSL→WGSL 转译）；Godot（编辑器热重载最好、Movie Maker 帧精确，但 `--headless` 是 dummy rasterizer **拿不到像素**）；自研 wgpu + naga（**3–6 人月**才到 parity，唯一工程优势是 naga 可脱离 device 独立校验 ⇒ 天然保留 last-good pipeline）。为 fill rate 迁 WebGPU 是错的（同一块硅，fragment 成本不变）；迁的判据只有三条：要 compute 做交互级烘焙 / GPU 剔除、draw call 破 2–3k、要 storage texture / bindless。
- **§4.5 编译经济学实测**（rustc 1.97.1 / `x86_64-pc-windows-msvc` / LLVM 22.1.6）：`cargo build` 无改动 **0.07 s**；改一行根 crate（`game` 依赖它）**0.47 / 0.48 / 0.58 s**；`rust-lld.exe` **已经躺在工具链里**：`<sysroot>/lib/rustlib/x86_64-pc-windows-msvc/bin/rust-lld.exe`（只是不在 PATH，切 lld 只需 `.cargo/config.toml` 一行）。Bevy 真 app（`DefaultPlugins` + `Camera3d`，**552 个 crate**，冷编 **193.8 s**，产物 **167.6 MB**）改一行 `main.rs`：默认（`link.exe` + 完整 debug info）**22.6 / 19.5 / 20.1 s**；lld + 完整 debug info **21.0 / 25.0 / 20.7 s**（没有帮助）；**lld + `[profile.dev] debug = "line-tables-only"`** **6.9 / 6.7 / 6.7 s** ✅（产物 167.6 → **150.7 MB**）；**`dynamic_linking` 40.9 / 41.1 / 40.3 s —— 反而翻倍**。
- ⇒ 三条：① **真正的成本是调试信息，不是链接器**，`debug = "line-tables-only"` 一项买到 3×，且保留 file:line（panic 回溯仍可读）；② **`dynamic_linking` 在 Windows/MSVC 上是负收益**，别照抄官方那页；③ 剩下那 ~6.7 s 是「任何 Rust 改动」的地板，**美术迭代不该踩到它**。快编译清单：lld + `debug = "line-tables-only"` + 自己的 crate `opt-level=1` / 依赖 `opt-level=3` + **Defender 排除 target 目录** + 独立 target dir + `cargo nextest`（根 `Cargo.toml` 的 `[profile.dev]` 现在就写着 `debug = "line-tables-only"`）。
- **美术热路径与 rustc 的关系**：改 shader / 改参数 / 改生成图 = **0 编译**；只有「加新 pass / 改渲染管线结构 / 改 ECS 系统」要重编 —— 这是 Bevy 与 three 之间唯一的结构性迭代速度差异。让渲染器脱离 PCG crate 的三档：① **进程隔离（推荐，与渲染栈无关）** ② **数据契约 crate 平行分层**（⚠️ 依赖方向是硬的：渲染器绝不能依赖 PCG 实现，节点注册走数据驱动的注册表）③ dylib 热插拔（Rust 没有稳定 ABI，跨边界只能传 `repr(C)` 或序列化，**不作为主机制**）。

## §5 Agent 截图友好：还缺 6 件事（现有 harness 已很硬）
热重载（最大杠杆）；「画面空白」必须能分辨是哪一种失败（GPU 身份断言 `UNMASKED_RENDERER_WEBGL` 白名单 / context 断言 `gl.isContextLost()` / 三通道 shader 错误采集，⚠️ **Edge 144 起 SwiftShader 被弃用，WebGL context 创建会直接失败**，不是静默降级；⚠️ **错误通道优先级最高**，着色器编译失败时不要把图像指标交给 agent）；通用「物体消失 / 纯白」判据（非背景像素占比、clipped white/black、tile 网格 Δ、edge density、熵、RAPS 斜率）；**资产指纹写进 shot JSON**（`graphHash / nodeHashes / shaderHash / tier / camera / GPU / DPR / toneMapping`，任何一项变 ⇒ 基线自动作废）；`run.mjs --watch`（SSE 驱动，agent 侧就是一条 `read_image`）；确定性地基（捕获帧与 rAF 解耦，同 seed 同时刻连拍两次 sha256 相同才允许写基线；readback 优先级 **应用内 `gl.readPixels` → base64 > `Page.captureScreenshot` > `toDataURL`**）。参考图只抽无量纲统计量当判据，不比像素、不进构建。

## §7 当时待裁决的项，最终落在哪
1. **DAG 要类型检查** ⇒ TS 随 Rust 栈作废，最终是 **Rust 数据驱动求值器**（§10.0），拓扑就是 Rust 代码（§16）。
2. **图兼管 sim 参数**（从图的类型生成 Rust `SurfaceParams` / `class_index`）⇒ §16 把 codegen 一并删掉之后，**不存在这条生成通路**；节点参数只有 `art/<图名>/<节点名>.toml`。
3. **头号：渲染栈** ⇒ **Bevy**（`px_render` 是 Bevy app，协议只读；编译税只落在「加新 pass / 改管线结构 / 改 ECS 系统」）。
4. **是否立刻做 Node PCG 侧车 + 契约 crate** ⇒ Node 侧车**撤回**（§10.0）；契约 crate 就是 **`px_protocol`**（P0 已建）。
5. **`px_contract` 的边界** ⇒ `px_protocol` = **只有类型 + serde + `SCHEMA_VERSION`**，无逻辑、无内部依赖、几乎不变；内含 `sim::*` 与 `art::*` 两个族。
6. **是否引 `manifold-3d`** ⇒ 走 **Rust 绑定 `manifold-csg`**（§10.4）。
7. **是否把噪声烘成贴图** ⇒ 方向定了（推进页内 GPU 烘 + 3D 噪声 LUT，§3.2），本文档范围内没有落地记录。
8. **spike（`WebGPURenderer` 吃不吃裸 GLSL 等）、`refs/` deny 路由、落地方式（先只写笔记）** ⇒ 前两项没有后续记录；第三项已执行，这份笔记即产物。

## §9 复现命令（Node 24 type stripping 与冷启动那批实测用的）
`node --version` ⇒ **v24.14.1**；`printf 'type P = {a:number};\nconst x: P = {a: 1};\nconsole.log(x.a);\n' > /tmp/p.ts && node /tmp/p.ts` ⇒ 直接跑通（零构建 TS），把 `enum` 加进去 ⇒ `ERR_UNSUPPORTED_TYPESCRIPT_SYNTAX`；`for i in 1 2 3 4 5; do /usr/bin/time -f '%e s' node -e '0'; done` ⇒ 冷启动 ~45 ms。

## §10 已裁决：走 Rust 栈 + 协议 crate
- 裁决：血缘 **`v2`（当前 checkout）**；`sim` ↔ `renderer` **跨进程**，协议 crate **同时是 wire 格式**；模块划分为 `sim` / `pcg` / `render` **全部 Rust**，通过协议 crate 序列化数据连接；PCG 求值器 = **Rust 数据驱动求值器**（Node 侧车方案撤回）。
- **wire 格式分两档**：控制面 / 状态（小、要能被人和 Python kit 读、要能进 `--digest`）走 **serde_json**；大数组（mesh / heightfield / 场纹理）走 **header JSON + 原生 little-endian payload**（`dtype` / `shape` / `stride` 写在 header 里）。⚠️ bincode / postcard 快但没有 schema 演进能力，**不能当跨版本的长期 wire 格式**。
- **跨进程的代价用握手兜住**：握手报文必须带 `SCHEMA_VERSION` + `protocol_hash` + `pid` + `exe` 路径 + `git rev`，**对不上就拒绝连接**（现有 `/api/ping` 只对 exe 路径的直接延伸）。
- **跨进程白送的收益**：渲染器可以消费「录制的流」`pxsim --record world.pxstream --round 240` —— 美术迭代不需要 sim 在跑，渲染器的输入变成一个**文件**（可 hash、可 diff、可当基线、可进 CI）。
- **数据必须单向流：`sim → pcg → render`，pcg 的产出绝不回流 sim**，否则美术改动污染 sim 确定性，同 seed `--digest` 基线立刻失效。拓扑上 render 同时需要 **sim 状态**（位置 / 选择 / 城市）与 **pcg 产出**（资产）⇒ 协议 crate 内含两个族：`sim::*` 与 `art::*`。
- ⚠️ **v2 血缘的现实**：`v2` 的 `src/state.rs` 目前是 **`pub struct State {}`（1 行）**，整条血缘约 4k 行，**没有 `web/`、没有 `.agents/`、没有 `scripts/shots/`**；全部美术工作（24 个 GLSL stage、shots 判据体系、基线、笔记）在 `feature/glsl-files` 孤儿血缘上 ⇒ ① 协议 crate 在 v2 上是**建骨架**，不是「抽已有投影面」；② 搬视觉语言只能是**按文件拷贝**（`git checkout feature/glsl-files -- <path>`，shader / 工具 / 笔记能搬，但要连带依赖：场景表、metrics、tuning 一起搬）。

### §10.1 crate 图（依赖方向是硬约束）
```
        px_protocol     ← 只有类型 + serde + SCHEMA_VERSION（无逻辑、无内部依赖、几乎不变）
        ↑    ↑    ↑
   px_sim  px_render  px_web
   (权威)   (Bevy)    (axum JSON API + 现有 web UI)
```
- `px_render → px_protocol ← px_sim`：渲染器不依赖 sim，sim 不依赖渲染器。`px_web → px_protocol ← px_sim`：现有 JSON API 从协议类型序列化 ⇒ web UI 与原生渲染器消费同一份类型。
- 改 sim 内部 / 改渲染器内部**互不重编**；改协议**两边都重编** ⇒ 协议必须**冻结、必须小**。

### §10.2 语言管不住的那一个漏洞 ⇒ 用门看住
- ⚠️ **Rust 不禁止传递依赖**：`px_render` 完全可以 `use px_sim::...` 而不报任何错 ⇒「两边都只依赖协议 crate」在语言层面**没有强制力**，会静静退化。这里是结构上做不到，所以只能靠门看住。
- 门落在 **`px_protocol/tests/crate_graph.rs`**（原计划 `scripts/check-crate-graph.mjs`，读 `cargo metadata --format-version 1`），断言 ① `px_protocol` 的运行时依赖 **= 白名单 `serde` + `serde_json`**（原计划里“也许 `glam`”没有发生）② `px_render` / `px_web` 在 dependencies / dev-dependencies / build-dependencies 里都不依赖 `px_sim` ③ `px_protocol` 必须是 workspace 成员（否则不被任何门覆盖）。⚠️ 实现只查 manifest 直连，**计划里的“可达性”断言没落地**。理由要写在门的注释里，否则下一个 agent 会以为它是冗余的。

### §10.3 协议版本与快照
- 协议 crate 拥有 `SCHEMA_VERSION`（本仓已有 9→10→13 的先例，只是那个住在 sim 里）。快照测试把 `WorldView` 等协议形状序列化成**稳定 JSON**，与 **`px_protocol/snapshots/protocol.snapshot.json`**（原计划放 `px_protocol/tests/`）**逐字**比对 ⇒ 改协议**必须显式更新快照**：`PX_UPDATE_SNAPSHOT=1 cargo test -p px_protocol`；快照一变 `protocol_hash` 就变，跨进程握手会因此拒绝旧对端。
- 精度纪律：**sim 权威 f64，渲染消费 f32**；换算只允许出现在协议 crate 里的**一处**。

### §10.4 图是数据，不是 Rust 代码（编译速度问题的解法）
| 层 | 形态 | 改它的代价 |
|---|---|---|
| 图的结构 + 参数 + seed | `.pxg.ron` / `.pxg.json` **数据** | **0 编译**（资产热重载或自己的 watch） |
| shader | `.wgsl` **资产** | **0 编译**（Bevy 内建资产热重载） |
| 节点的实现（新算子类型） | Rust 代码 | **1–3 s**（`dynamic_linking` + lld） |
| 渲染管线结构 / 新 pass / ECS 系统 | Rust 代码 | **1–3 s** |
- ⇒ **日常美术迭代（改图、调参、改 shader）= 0 编译**，只有「加新算子 / 加新 pass」才付编译。（表里的 `.pxg.ron` / `.pxg.json` 形态后被 §16 / §17 取代：拓扑是 Rust、参数是 TOML，**代价矩阵不变**。）
- ⚠️ §2.2（TS 实测）与 §2.3（QuickJS）在「走 Rust 栈」下大部分作废，但两条被继承：**热重载的是数据、不是代码模块**；**禁止双实现**。§2.7 的确定性铁律原样适用。

### §10.5 分阶段（在 v2 上）
- **P0 建协议骨架**：`px_protocol`（类型 + serde + 两档 wire 编解码 + 快照门 + crate 图门）。验收（自证，不需要审美裁决）= 一份 `--record` 出来的 `.pxstream` 能被重放成同一个 `WorldView`，且两次序列化逐字节相同。
- **P1 起 `px_pcg`**：数据图 + Rust 常驻求值器 + CAS；先不接渲染，只出 `art::*` 数据，用 CLI + 快照验收。
- **P2 起 `px_render`（Bevy）**：只读协议（+ 录制流）；先渲染最简场景；headless 截图命令 + 判据按 `scripts/shots` 的思路重建（v2 上没有那套，要拷或重写）。
- **P3 搬 / 重写视觉语言**：24 个 GLSL stage → WGSL（原文件可从 `feature/glsl-files` 拷贝参考），后处理按 §4.3 的顺序迁；每步像素对照 + 帧时间对比。

### §10.6 唯一还没解决的结构问题：web UI 与原生渲染器的重叠
- 现有 web UI（状态面板 / 控制行 / 设计图库 / `map3d`）是一大笔投入，走原生后只有三种收场：① web UI 保留但去掉 3D ② 整体转原生（`bevy_egui` 之类）③ 一段时间内两套 3D 并存 —— ⚠️ 违反「两个渲染器 = 双实现漂移」，只能是**有期限的过渡**且必须写清截止条件。**待用户裁决。**
- ⚠️ 在 v2 上这一节暂时不成立：v2 **没有 `web/`**（它在 `feature/glsl-files` 那条孤儿血缘上），「两套渲染器并存」根本不会发生；将来要把 web UI 拿回来当第二消费者，再回来看这一节。

### §10.7 待裁决
1. ⏳ **`.pxstream` 录什么**：sim 的**完整状态**，还是**render 需要的那部分投影**（决定文件大小，也决定「美术能脱离 sim 迭代到什么程度」）。
2. ⏳ **web UI 的去留**：v2 上没有 `web/` ⇒ 要么不做 web，要么从 `feature/glsl-files` 拷 `web/` 过来当**第二个消费者**（协议 crate 让这变得便宜）。Bevy vs wgpu 已定：`px_render` 就是 **Bevy**；其余见 §7 的第 4–9 项。

## §18 「静态链接那这不就是一个普通的 Rust 项目吗」
- 是 —— 而且这是设计目标，不是副作用。但「普通 Rust 项目」之外还剩三样东西，它们才是这一层真正的产出：**`cook` 边界 + 内容寻址 CAS**（程序每次从头跑到尾，没有缓存就是每次全量重烘；参数迭代全靠它把「重算」压成「命中」）、**`.toml` 参数文件**（「改参数不重编」的唯一实现方式）、**产物格式 = `px_protocol::ArtBundle`**（与渲染器之间唯一的接口）。
- **代价（要认）**：因为图不是数据，运行期查看 / 编辑 DAG、任意节点的 bypass / lock、美术自己搭网络、把图交给不能编译 Rust 的人 —— 这些都没有了 ⇒ 这一层的天花板是「谁来写图」：现在是 agent 写在代码里，将来要工具或人来写，才需要真正的图引擎。
- **折中：拓扑隐式、节点边界显式。** 每个算子调用过一层，它负责按节点名读 `art/<图>/ridge.toml`、算 key、命中就取 / 未命中就 cook、登记进 manifest，框架语法就只有这一个入口（原提议的宏 `px_ops::node!("ridge", field::fbm, &[&base])` 最终落成泛型函数 `node::<Op>(名字, 输入)`，见 §20.3）。**一句诚实的补充**：P2a 的玩具图只有几个噪声算子，此刻缓存买不到速度（全量重算也就几毫秒）；现在建它，是因为**边界**（`cook` + 参数文件 + 产物格式）是架构。

## §24 3D 场应是 PCG 的一等公民、球面格式只是导出
真实缺陷：3D 只是**求值时的瞬态** —— 算子在内部算 `fbm_3(direction(u, v))`，算完就拍平成 equirect 图 ⇒ **投影知识写在算子里**（换一种球面格式就得改算子）、**真正需要 3D 的操作写不出来**（球面正确的 warp 就是这个病根，§23.7 那个待办）、同一份场想再导一份 cube map 只能重跑整张图。

### §24.1 先量价格：烘一个体 vs 烘一张球面图
`px_graphs/src/bin/volume_probe.rs`（探针，不是图），六阶 fbm 3D、单线程；列依次为样本数 / 耗时 / f32 体积：

| | 样本数 | 耗时 | f32 体积 |
|---|---|---|---|
| 64³ | 262k | 92 ms | 1.0 MB |
| 96³ | 884k | 321 ms | 3.5 MB |
| **128³** | 2.1M | **754 ms** | **8.4 MB** |
| 160³ | 4.1M | 1498 ms | 16.4 MB |
| 对照：384×192 **球面**烘焙 | 74k | **27 ms** | 0.29 MB |

⇒ 128³ 的体是同一张球面图的 **28 倍代价**（体素数之比正好也是 28×；每样本耗时两者都是 ~0.36 µs）。

### §24.2 「能导出多种格式」并不足以证明该烘体
只采样球壳的话，按方向直接求值的代价和现在完全一样（每个输出纹素一次 3D 求值）。烘体的真正理由是另外三条：① **要采内部**（等值面提取 → 悬崖 / 洞穴 / 真实地形、体积云与大气、需要内部信息的侵蚀）；② **要摊薄**（一份体导出很多份高分辨率输出：4K equirect + 6×2K cube + 各级 mip）；③ **要共享 / LOD**（体天然可以带 mip 金字塔）。

### §24.3 两步走 + §24.4 / §24.5 的落点
- 第一步（便宜、现在就能做）：`Field` 带一个 `Grid` 描述 `{ projection: Equirect | CubeAtlas, width, height }`，算子**不再自己算 `direction`**、向 Grid 问「这个像素对应哪个方向」⇒ 图声明自己的输出投影、算子完全投影无关，换 cube map 一个算子都不用改；顺带球面正确的 warp 可以写了：`warp3(p) = fbm3(p + strength · vecfield3(p))`，逐输出纹素求值，代价和现在一样。
- 第二步（等真需要内部采样）：协议加 `AssetKind::Volume3D`（`BlobHeader.shape` 本来就是 `Vec<u32>`，支持任意维度 ⇒ 改动很小）+ `VolumeOp` trait（或把 `FieldOp` 泛化成 `Grid<T>`）+ 新增 `volume.project` 节点（把体按 Grid 采样成球面图，可超采样 / 面积平均，顺手解决极点各向异性）。触发条件：要悬崖 / 洞穴 / 体积云，或要按面做 LOD，或一份体要喂多份导出。
- ⚠️ **§25 用户裁决推翻这条路线：不烘体**（体烘焙丢掉解析以后继续处理精度会掉），**Grid 那套抽象也不做** ⇒ §24.3 / §24.4 / §24.5（Grid 归属、cube atlas 布局、是否先把 `Volume3D` 写进协议）均未落地。
