# 美术框架：agent 截图友好 / 热重载 shader / 热重载程序化生成管线

> **状态：已落地并在用** ✓。当前分支 `feature/art-stack`，worktree `.worktrees/art-stack`。
> **新 session 请先读 §38「交接」** ✓ —— 那里是现状、已验证的工作流、以及下一步（体积云）。
> 早期章节（§0–§10 的调研与裁决）保留为历史 ✓，其中 §2.2 / §2.3 / §4.5 的结论已被 §10 取代 ✗，
> 不要当现状读。§34 有一条**已被推翻**的结论 ✗，就地留了更正。
> 依据：2026-09-13 四路并行调研（DAG 层 / 几何与烘焙 / 渲染栈 / agent 视觉回路）
> ＋ 通读 `feature/glsl-files` 的 `web/static/**`、`scripts/shots/**`、`.agents/notes/**`。

---

## 0. 结论（先看这三条）

1. **DAG 层：写一个「图即 TS 模块 + 内容哈希记忆化求值器」**，求值器跑在**长驻 Node 进程**里，
   浏览器只收数据。不要 DSL、不要 Houdini/Blender/Substance、不要通用编排器。
   方向已由用户定：**要类型检查**（`ask_user_question` 2026-09-13 原文：
   *「为啥不用 ts，有类型检查对 agent 不是更友好吗？」*）。
2. **几何/烘焙：SDF 统一形态 + 只引两个库**（`manifold-3d` WASM 做鲁棒布尔、`meshoptimizer` 做 LOD）。
   烘焙的真实收益是**把噪声从 shader 搬走**——本仓自己已经把账记在
   [`shader-compile-stall.md`](shader-compile-stall.md) §4（冷编译 6–14 s，首选解法就是烘贴图）。
3. **渲染框架：没定，而且判据变了。** 用户 2026-09-13 裁决：
   *「不一定看 web 的，只要是面向未来的渲染框架，截图方便，重载方便，agent 用起来方便最好」*
   ⇒ 浏览器不再是硬约束 ⇒ **「交付形态」成了头号待裁决**，它决定一切（§4）。

---

## 1. 现状盘点：有什么、缺什么

### 1.1 已有的（都挺硬，别推翻）

| 件 | 在哪 | 状态 |
|---|---|---|
| GLSL 分文件 + three `#include` 图 | `web/static/shaders/px/**`（38 个 chunk / 24 个入口 stage） | **已经是一张 DAG**（材质层），带 manifest 门 + glslang 离线编译门 |
| 截图/判据 harness | `scripts/shots/`（scenes / baselines / metrics / pixdiff / field / refstat / cdp） | 场景 = 固定 URL + **解析相机** + 冻结时间；自己的 headless Edge（CDP 9333 真 GPU） |
| 参考图标定 | `refs/` + `refstat.mjs` / `refcloud.mjs` | 已明确「只标定、不进构建」 |
| **程序化烘焙先例** | `web/static/map3d/sunfield.js` | 流场 → 512×320 RGBA32F 切片图集 + 顶点手写三线性；烘 0.2 s，运行时**成本≈0** |
| 参数管线 | `model::SurfaceParams` → `config/game.ron` → `kinds.js` → `planet.js` | 手工四段同步 |

### 1.2 缺的（本条目的需求书）

1. **热重载：完全没有**（全仓 grep 不到任何 hot reload / watch）。迭代 = 改文件 → 重载整页 →
   冷编译 6–14 s → 截图。
2. **同一逻辑两份实现 → 必然漂移**（本仓两次前科）：
   `scripts/shots/cloudstat.mjs` 把 `cloud.frag::cover()` **逐行搬进 JS** 统计球面覆盖率；
   `sun-prominence.md` §633 原话：这条路的后果是 *「同一个场有两份实现（JS 烘带级 + GLSL 算顶点级），
   **必然漂移**」*。
3. **四处手工同步、错了不报错**：`SurfaceParams` 字段 / `game.ron` / `kinds.js` 的
   `SURFACE_DEFAULTS` + `VARIANT_CLASS` / `planet.js` 的 uniform 读取。
   Rust `class_index` 与 JS `VARIANT_CLASS` 不一致时，症状只是「某类行星悄悄换了公式」。
4. **参数填了不生效**（`crater_density` 事故）：`param → uniform → 着色器真的读它` 这条链没有结构保证。
5. **「画面空白」分不清是哪一种失败**：GPU 选错 / context 创建失败 / shader 编译失败 / 逻辑没生效
   —— 四种症状都是黑屏（详见 §5）。

> **所以框架的第一职责不是「做个节点编辑器」，而是把「参数唯一真相」和「生成逻辑只有一份」
> 这两件事结构性钉死。** DAG 只是手段。

---

## 2. Q1 —— 程序化生成 DAG：脚本？DSL？

### 2.1 四条路线（逐条排除）

| 路线 | 可 diff | 零构建 | 浏览器热求值 + Node 权威烘同构 | 值级增量缓存 | 判定 |
|---|---|---|---|---|---|
| Houdini（SOP/PDG/VEX，hython headless） | ✗ `.hip` 二进制（可 Save as text，但几何仍二进制） | ✗ 商业许可，Indie 格式与商业 license 不通 | ✗ | ✓✓ 最强（`pdg.cacheMode`） | **不采用**，只借纪律 |
| Blender Geometry Nodes + bpy | ~（图只能靠 bpy dump） | ✗ | ✗ | ✗（depsgraph 脏标记，非内容哈希） | 不采用（`-b` 下 `bpy.app.timers` 不触发） |
| Substance `.sbs`(XML) / Material Maker `.ptex` | ✓ | ✗ 要它们的运行时 | ✗ | ✗ | 借格式，不采用 |
| 通用编排器（Dagster/Prefect/Snakemake/Ninja/Bazel） | ✓ | 多数要 server+DB+daemon | ✗ 图在 Python、产出是文件 | ✓✓（Bazel/Ninja/Nextflow） | **只借失效语义** |
| 自研「图即代码 + 内容哈希记忆化求值器」 | ✓✓ | ✓✓ | ✓✓ | ✓ | ✅ **推荐** |

关键事实（已核实）：
- **没有任何现成框架同时满足**「纯文本图 + headless CLI + 强制确定性 + 亚秒热重载 + 无 bundler/无 node_modules」。
  每一个都要写同样多的胶水，还要背它的额外重量。
- 借鉴对象（不是采用对象）：**Ninja 的 `restat`**（= early cutoff 的最小实现）、**Bazel action cache**、
  **Salsa 的 red-green 增量**（Rust，`ra_ap_salsa`）、**Houdini PDG 的 expand/cook 两阶段 + work-item 文件缓存**、
  **fogleman/sdf 的 `~/.sdf` 内容寻址缓存**、**Minecraft density function**（真实工业先例：地形就是一棵
  **纯 JSON 的密度函数 DAG**，可 diff、可版本控制、由数据包分发）。
- SDF/CSG 小 DSL 的共同选择是 **「宿主语言 + 少量语法糖 + 一个 CLI」**，**不是发明一门新语言**
  （libfive=Scheme、fogleman/sdf=Python、OpenSCAD=自有文本语言但**无增量缓存所以慢**）。

### 2.2 为什么是 **TS** 而不是 JS —— 本机实测（这是本节最重要的一段）

```
$ node --version
v24.14.1
$ node probe.ts          # 含 type / interface / as const，不带任何 flag
field/fbm:3.2:abc:1      # ✅ 直接跑通，零构建、零转译步骤
$ node probe_enum.ts     # 同一个文件里加了 enum E { A = 1 }
SyntaxError [ERR_UNSUPPORTED_TYPESCRIPT_SYNTAX]:
  TypeScript enum is not supported in strip-only mode
```

⇒ **Node 24 默认开启 type stripping**：`type` / `interface` / 注解 / 泛型 / `as` / `satisfies` /
`as const` 全部**直接跑**，**不需要 tsc、不需要构建步骤**。只有 `enum` / `namespace` /
构造函数参数属性 / `import =` / 装饰器被拒（要它们得 `--experimental-transform-types`，不建议）。
**`enum` 本来就不该用**（`as const` + `keyof typeof` 是更好的写法）—— 这条限制与好风格一致。

于是「类型检查对 agent 更友好」这条诉求可以**不付任何构建代价**地拿到，且有三种强度可用：

| 强度 | 做法 | 代价 |
|---|---|---|
| 运行时（免费） | Node 直接跑 `.ts`；类型写错但**语法错**会立刻炸 | 0 |
| **类型门（推荐）** | `tsc --noEmit` 当合流门的一道（与 `check-glsl-manifest` / `check-shaders` 并列） | 需要 dev-only 的 tsc —— 放 `scratch/ts-tools/`，**与 glslang 的先例一致**（外部工具不进版本库、缺了就跳过） |
| 生成 Rust | 从图/节点的类型定义生成 Rust `SurfaceParams` + `class_index` | 一次性写几十行生成器 |

**第三行才是真正的收益**：§1.2 的第 3、4 条（四处手工同步、错了不报错）**结构性消灭**。
这正是本仓一贯的做法——「结构上消灭，而不是继续靠一道门看住」（`glsl-files.md` 原话）。

**浏览器怎么办？** 浏览器不能跑 `.ts`。解法是**求值器根本不住在浏览器里**：
图求值跑在长驻 Node 进程，浏览器只通过 WebSocket 收**数据 blob**（ArrayBuffer / ImageBitmap）。
这顺带回避了另一条坑：**浏览器里 `await import(url + '?t=' + Date.now())` 绕 ESM 缓存会泄漏模块**
（原生 ESM 没有 `import.meta.hot`，垃圾回收失效的 ESM 模块至今无解）。
⇒ **热重载的是「数据」，不是「代码模块」。**

### 2.3 为什么不引 QuickJS（用户提问，已实测）

用户 2026-09-13 提问：*「此外 node 不是启动很缓慢吗，用 quickJS 之类的轻量运行库如何？」*

实测本机：

| 量 | 实测 |
|---|---|
| `node -e "0"` 冷启动 | **43.3 / 44.2 / 44.4 / 48.0 / 56.4 ms** |
| `node file.js` 冷启动 | 45.5 / 52.4 / 74.5 ms |
| 常驻进程内 1e6 次 `Math.sin` + `fround` | **17 ms** |

**Node 不但不慢，而且冷启动在本场景根本不进热路径**：热路径是「文件变更 → 重算脏子图 → 推 blob」，
全部发生在**已经在跑的进程内**，不重启进程 ⇒ 冷启动只付一次（~45 ms），
QuickJS 省下的那 40 毫秒买不到任何东西。

而 QuickJS 的代价是**致命的**：

1. **标准 QuickJS / quickjs-ng 没有 WASM** ⇒ `manifold-3d`、`three-mesh-bvh`、`meshoptimizer`
   这些我们打算用的几何库**在里面跑不了**（§3 的整个几何方案建立在 WASM 上）。
2. 没有 npm 生态、没有 `fs.watch`、没有 `WebSocket`、没有 `node:crypto` —— 都要自己补。
3. QuickJS 真正值钱的场景**现在都不成立**：① 沙箱化执行**不可信**的 agent 代码（我们写的是自己仓库里的图）；
   ② 把 JS 引擎**嵌进 Rust 进程**（`rquickjs`）省掉一个 sidecar —— 但那会丢掉 WASM 库。

**若哪天真要「Rust 进程内求值」，正确顺序是 `wasmtime` 跑 WASM 版求值器**（wasmtime 能跑 WASM 库，
QuickJS 不能），这条路自洽得多。**结论：用 Node 做长驻 watch 进程，不引 QuickJS**；
把「引擎可嵌入」留给未来当可选优化，先不付它的代价。

### 2.4 形状（拟）

```
tools/pxgen/                     ← 一个长驻 Node 进程 + 一个 CLI（不进 web/static）
  nodes/                         ← 一节点一文件（纪律同 shaders/px/noise/*.glsl）
    field/{value3,fbm,ridged,curl,warp,mask,erode}.ts
    geom/{sphere,lathe,extrude,sdfUnion,sdfSmooth,isosurface}.ts
    assembly/{scatter,ring,kit}.ts
  graphs/{planet,ship,station,cloud}.ts    ← 纯数据：节点声明 + 参数（唯一真相）
  eval.ts                        ← 拓扑排序 + 内容哈希 + 记忆化 + early cutoff
  cas.ts                         ← 内容寻址存储（大值落盘）+ GC
  watch.ts                       ← fs.watch → 脏子图 → WS 推 blob
  gen-rust.ts                    ← 从类型生成 Rust SurfaceParams / class_index
```

节点契约（**默认值住在节点里**，与 `SURFACE_DEFAULTS` 的教训一致）：

```ts
export const type = 'field/fbm';
export const params = { freq: 1.0, oct: 5, lac: 2.02, gain: 0.5 } as const;
export const backend = 'cpu' as const;              // 'cpu' 权威 | 'gpu' 缓存
export type P = typeof params;
export function evalNode(ctx: Ctx, p: P, ins: Ins): Out { /* 纯函数 */ }
```

图 = 声明，不是节点（一个节点文件被多张图复用）：

```ts
export default {
  seed: 42,
  nodes: [
    { id: 'h', type: 'field/fbm',  params: { freq: 3.2, oct: 5 } },
    { id: 'w', type: 'field/warp', in: { src: 'h' }, params: { amt: 0.6 } },
  ],
  out: { height: 'w' },
} satisfies Graph;
```

**节点 id 用确定性可读名（`noise_continent/main`），不用随机 uuid** —— 可读 id 直接决定 diff 质量
（反面教材：Babylon NodeMaterial 的 JSON 里 id 是生成的，diff 全是噪声）。

### 2.5 三条必须抄的语义（写代码时直接用）

1. **early cutoff**：节点重算后如果**输出内容哈希没变**，就**不再往下游传播**。
   没有这一条，改一个叶参数会重算整条链，亚秒预算立刻爆。（抄 Ninja `restat` / Salsa red-green）
2. **分层 CAS**：小值（标量、小数组）内联进结果；大值（heightfield、纹理、mesh）写
   `blobs/<hash>` 只留引用。**这同时解决「diff 友好」与「大二进制」的矛盾**：
   图文件永远小、永远可 diff；大资产按内容寻址、可 GC。
3. **expand 与 cook 分离**（抄 Houdini PDG）：先展开（分配 seed / 算实例数 / 不计算），再 cook。
   于是 agent **毫秒级**就能看到「我这改动会生成多少实例、seed 怎么分布」，不必等烘焙。

**缓存键**必须包含影响正确性的全部因素：
`hash(opVersion, params, inputHashes, rngId, precisionMode, platformTag)`。
**改算法必须 bump `opVersion`** —— 否则「升级了 libm 之后旧缓存被静默复用、产出悄悄变了」这种 bug 迟早发生。

### 2.6 一条纪律：**禁止双实现**

本仓已经为「同一逻辑两处实现」烧过两次钱（§1.2 第 2 条）。规则的三种合法形态：

- **(a) 只在 GPU 上实现**，CPU 侧要数据就 **readback**（本仓已有「headless Edge + 真 GPU」的现成路子）。
  ⇒ `cloudstat.mjs` 那种「把 `cover()` 抄进 JS」从此退休。
- **(b) 只写一份 CPU 实现**，要上 GPU 就烘成纹理（§3.2）。
- **(c) 表达式子集同时编译到 JS 与 GLSL/WGSL** —— 成本最高，**先不做**。

### 2.7 确定性铁律（跨机器）

- **禁 `fract(sin(x)*43758.5453)` 这类 hash**：`sin` 跨 libm 不同 ⇒ 不可复现。用整数 hash / 位运算。
- Rust 侧默认 RNG **不可复现**：`StdRng` / `SmallRng` 被官方标为 **non-portable**（任何 release 都可能改值）
  ⇒ 要可复现就用 **`rand_chacha`**；`rand_distr` 的正态分布需 `libm` 特性。
- GPU 浮点跨厂商**不保证 bit-identical** ⇒ **GPU 只能做「运行时缓存」，绝不能当基线来源**；
  权威烘走 CPU。这一条必须写死，否则本仓辛苦建的像素判据体系会被「基线漂移」毁掉。

---

## 3. Q2 —— 几何库 & 贴图烘焙系统

### 3.1 几何：SDF 是统一语言，**只引两个库**

| 需求 | 选型 | 理由（已核实） |
|---|---|---|
| 布尔 / 硬表面 kitbash | **`manifold-3d`（WASM）** | v3.5.3 / 2026-09-07 / **Apache-2.0**；ESM，`Module({ wasmUrl })` **绕开 bundler**（对「无构建」是决定性的）；**v3.5.0 起跨平台浮点确定性**（CI 与浏览器产出一致）；比 CGAL 系快 1–3 个数量级；**Blender 4.5 已内置它当 boolean 求解器**；`levelSet(sdf,…)` 用 marching tetrahedra 从 SDF 出网格（**只要求 SDF 符号正确**，不要求是距离函数）；`extrude/revolve/slice/hull/decompose/simplify/calculateCurvature` 覆盖面板、旋转体、切片 |
| 有机形态 / 小行星掏洞 | 同上 `levelSet` | 不引第二个依赖 |
| 简化 / LOD / 量化 | **`meshoptimizer@1.2.0`（npm, MIT）** | `simplifyWithAttributes` 保 UV/法线；**`simplifyPrune` 专治 isosurface 碎片**；`compactMesh` / `encodeFilterOct`；纯 Node 可跑；⚠️ `clusterlod.h` **只有 C++ 侧**（npm 不导出），但本仓（数千小物体）用实例化 + 多级 index buffer 就够，**不需要 Nanite 式 CLOD** |
| 运行时轻量布尔 | `three-bvh-csg` + `three-mesh-bvh`（MIT） | 纯 ESM、相对路径 + `three` 裸说明符 ⇒ 现有 importmap 直接可用；只在「必须时」用 |
| 参数化图元 / 面板 | three 内建 `Shape`/`ExtrudeGeometry`/`LatheGeometry`/`TubeGeometry` | 零依赖；舰体是「数据决定形状」，已在本仓 `models.js` 验证 |

**不要碰**：CGAL（GPL/商业双授权 + 慢）、OpenVDB/nanoVDB（GPU 侧只有 CUDA/OptiX/OpenCL/OpenGL/DirectX，
**没有 WebGPU/WebGL**）、OCCT/opencascade.js（wasm 数十 MB）、**xatlas-web（2024-07-22 已归档）**、
Instant Meshes / QuadriFlow（产出非确定性、需人工清理）、OpenSubdiv（无 JS 绑定）、
**three 的 `SimplifyModifier`**（社区实测高密度网格上明显差于 `MeshoptSimplifier`）。
**也别做**：把行星/卫星「统一成 mesh」——「解析球 + 位移」是性能与画质双赢，网格化是倒退。

**UV：优先不做。** 行星 UV 是解析的（经纬 / cube-sphere）；硬表面走 triplanar / box projection。
xatlas 只在「硬表面 UV 真的痛」时离线引入。

**架构建议**：**不要把生成的 mesh 当资产传**。传 **seed + recipe**，在消费端用同一套确定性代码重新生成。
体积≈0，且天然满足「严格程序化」约束。

### 3.2 烘焙：三层分清，收益最大的那层**还没做**

「烘焙」在本项目里必须先拆开，否则选型必错：

| 层 | 本仓现状 | 建议 |
|---|---|---|
| shader 内解析计算 | 主战场（`planet.frag` 里 17 处 `fbm` + 4 处 `warp` + `ridged`） | **只留高频细节** |
| **页内 GPU 烘 → 缓存** | 只有 `sunfield.js` 一处（流场图集） | **推进**：低频部分（大陆/带纹/云底）烘成场纹理；缓存 key = 图 hash，落 IndexedDB/CacheStorage |
| Node/CPU 权威烘 | 无 | 用于基线 / 判据 / CI（**逐位可复现**） |

- **烘焙的真实收益本仓已记账**：`shader-compile-stall.md` §4 —— 冷编译 ultra 档 **6–14 s**，
  残留原因是「固定的内联规模」（每 program ~17 份 `vnoise` × 24 program），
  而笔记里列的**首选解法就是「把噪声烘成贴图——shader 退化成纹理采样，编译量与运行成本一起塌」**。
  ⇒ 这不是新想法，是**本仓自己写下但没做的那一条**。
- **一条必须写死的纪律**：GPU 烘跨驱动**不保证逐位一致** ⇒ GPU 烘只能当**运行时缓存**，
  **绝不能当基线来源**（与 §2.7 同一条）。
- **噪声 LUT 是性价比最高的一招**（不改架构）：加载时生成一张 **64³/128³ RGBA8 3D 噪声纹理**
  （1–8 MB，全材质共享），shader 里把多 octave FBM 换成「3D 纹理采样 + 手动叠 octave」。
  依据：Khronos 论坛的噪声查表实践 + Unity 移动端「贴图永远比 procedural 便宜」的结论。
  （显存算术：1024³ 3D 纹理 = 1 GB 不可行；**64³ = 1 MB、128³ = 8 MB 完全可行**。）
- **派生贴图**：法线**不烘**（本仓已在算解析法线）；AO / curvature 是**唯一真正值得离线烘**的一类，
  但只在「几件 hero 资产」上做（舰/站），数千实例靠图集 + 每实例 seed 去重复。
- **容器**：暂**不引 KTX2/Basis**（要带 transcoder + loader，收益不抵现状）；自研 PNG（已有）够用。
  Float 场沿用本仓已验证的 **RGBA32F + NearestFilter + 手写三线性**（float 纹理线性过滤要
  `OES_texture_float_linear`，这正是当初选 Nearest 的原因）。
- **AI 贴图（SD/ControlNet/ArmorLab/Meshy）排除**：非确定性（无法 byte-identical 复现）、
  许可条款有门槛、云端方案违反「无下载资产」。可作为美术离线探索，**不进管线**。

### 3.3 逐资产决策表（照抄即可）

| 资产 | 策略 |
|---|---|
| 行星表面 / 卫星 | **shader 实时** + 3D 噪声 LUT（LOD/位移每帧变，烘了会锁死细节层级） |
| 小行星 / 小行星带 | **加载时 GPU 烘 8–16 变体图集** + 每实例 `vec4(u0,v0,us,vs)` 选片 + seed 扰动（数千实例共享材质 = 图集方案的完美场景） |
| 行星环 / 尾焰 | shader 实时（极便宜 / 必须动态） |
| 地表城市 | 加载时烘（屋顶/窗户图集）+ shader 细节（重复度最高，收益最大） |
| 舰船船体 / 空间站 | 几何靠 Manifold，**离线只烘 AO/curvature** 且只烘旗舰级 hero 资产 |
| 星空 / 银河背景 | **CubeCamera 烘 cubemap**（已做）+ 值得补 `PMREMGenerator` 预过滤当全场景唯一 IBL |

---

## 4. Q3 —— 渲染框架（本节因用户裁决而**重开**）

用户 2026-09-13 裁决：*「不一定看 web 的，只要是面向未来的渲染框架，截图方便，重载方便，
agent 用起来方便最好」* ⇒ **浏览器不再是硬约束**。

### 4.1 头号待裁决：**画在哪儿 = 交付形态**

这不是技术偏好问题，是**架构分叉**，因为本仓最贵的一类 bug 就是「两处实现漂移」（§1.2）：

- **若玩家侧的 3D 视图必须是浏览器** ⇒ 美术层也只能是 web ⇒ 「面向未来」在 web 侧**只有一条路**：
  **TSL / WebGPU**（见 §4.3）。继续在 WebGL2 + 手写 GLSL 上加码 = 往一条不再演进的分支投资。
- **若交付形态可以是原生**（桌面 app；浏览器可以是可选目标） ⇒ **Bevy + WGSL** 是
  「面向未来 + 截图方便 + 重载方便 + agent 方便」四项全胜的答案（见 §4.2），
  而且**模拟核本来就是 Rust**，渲染层可以直接链接 sim crate，
  **把 JSON/HTTP 边界整个消灭** —— 连本仓烧时间最多的那类坑
  （「我改的代码到底生效没有 / 我连的是哪个实例」，见 `glsl-files.md` 结尾）一起消灭。

> ⚠️ **反面必须说清**：**两个渲染器 = 又一次双实现漂移**。
> 本仓已经有过「JS 烘场 vs GLSL 算场必然漂移」「cloudstat.mjs 抄 cover()」两次前科。
> 所以要么整条 3D 视图都走原生，要么都走 web，**不能让「agent 看到的」和「玩家看到的」是两个渲染器**。
> 若两边都要：材质/生成逻辑必须**单一源**（Slang 离线编译到 WGSL + GLSL 是唯一自洽的现成方案），
> 这引入新工具链，成本要单独评估。

### 4.2 原生路线：Bevy + WGSL（若交付形态允许）

| 判据 | Bevy 的答案 |
|---|---|
| **重载方便** | **WGSL 资产热重载内建**（`AssetPlugin` 文件监听 → `ShaderCache` 重处理 → pipeline 重新特化，约 100 ms–1 s）；naga 诊断带**源码 span**，`naga_oil` 组合后仍能映射回原文件行号 ⇒ 错误 UX 优于 three 的 GLSL info log |
| **截图方便** | 无窗口 `WindowPlugin { primary_window: None }` + `headless_renderer` 模式（render-to-`Image` → CPU readback）；**`TimeUpdateStrategy::ManualDuration` 精确渲染固定 dt 的 N 帧** ⇒ 帧精确，**不依赖浏览器 / compositor / rAF / CDP**，也没有双 GPU 抽签、浏览器缓存、「连错实例」这一整类坑 |
| **agent 方便** | 一条命令 `cargo run -- --scene sun-limb --t 12 --out shot.png --stats json`；RenderDoc 直接可用；compute 是一等公民（程序化生成 / GPU 剔除 / indirect draw） |
| **代价** | 24 个 GLSL stage → WGSL 重写 + 后处理用 render graph 自建（Bevy 内置 bloom/tonemap/MSAA/AgX，**god rays / flare / grade 要自写**）；Bevy 渲染 API 版本间 churn 大、高层实例化封装不如 three 顺手 |

### 4.3 浏览器路线：three.js + 增量 TSL（若交付形态必须是 web）

**先更正本仓一条已过期的结论**：`web-vfx-pipeline.md` §6 写的是
「`WebGPURenderer` 不支持裸 GLSL，且 `EffectComposer`/`UnrealBloomPass` 在 WebGPU 下不存在
⇒ 后处理要整条重写」。**前半句仍然成立，后半句已经过期**：

- **仍成立（已核实）**：`WebGPURenderer` 不支持裸 GLSL `ShaderMaterial`（GLSL 被忽略，
  症状正是「物体不见了」）；raw WGSL 也不能直接用，必须走 TSL。
  `WebGLRenderer` 从 **r164** 起彻底移除 NodeMaterial 支持。
- **已过期（已核实）**：**three r183 引入 `RenderPipeline`** —— 节点式后处理，
  target `WebGPURenderer` **并带 WebGL2 fallback**，effect 就是 TSL 节点函数。
  且官方文档索引里 **`GLSLNodeBuilder` 与 `WGSLNodeBuilder` 并存** ⇒
  **TSL 能编译到 GLSL、可以跑在 WebGL2 上**，迁移因此是**增量的、可回退的**，不是 flag day。
- **减压阀**：TSL 有 `glsl()` / `wgsl()` 内联函数 + `GLSLNodeFunction` 节点 ⇒
  现有 24 个 stage **不必逐行翻译**，可以整段内联。
- **官方已内置我们手写的东西**：TSL 显示函数里有 **`godrays()`**、**`lensflare()`**、`dof`、`ssao`、
  `ssr`、`fxaa`、`traa`、`lut3D`、`vignette`、`saturation`、`toneMapping`…（`bloom` 未确认，索引抓取被截断）。
- **顺带一个战略级发现**：**TSL 图就是 JS 代码** ⇒ 它天然是「可热重载、可 diff、agent 可编辑的 DAG」，
  **与 §2 的 `pxgen` 是同一族东西，将来两层可以合并**（`NodeMaterialLoader`/`NodeLoader` 存在，
  序列化保真度未验证）。
- **迁与不迁的判据（写死成三条，能量出来）**：① 需要 **compute** 做交互级烘焙 / GPU 剔除；
  ② draw call 破 2–3k；③ 需要 storage texture / bindless。**为「fill rate」迁 WebGPU 是错的**
  —— 同一块硅，fragment 成本不变；naive 重写甚至可能更慢。
- **WebGPU 的真正收益（不是帧率）**：compute 驱动程序化生成 + 原子 compaction + indirect draw
  （消掉每帧 CPU→GPU 同步点）、**`ReadbackBuffer`**（agent 数值回读从编码 hack 变成一等 API）、
  **`TimestampQueryPool`**（可靠 GPU 帧时间）。

**明确不采用**：Babylon（换栈 = 24 stage + 后处理全部重写；它唯一的两张牌是
**NodeMaterial 的 JSON 序列化**（`serialize()`/`Parse()`，NME 直接产出该 JSON，且 `CustomBlock`
可内嵌现有 GLSL）与**可能存在的运行时 GLSL→WGSL 转译**——⚠️ 后者未核实，
**若成立会改变整个成本评估，值得单独做一小时 PoC**）；
Godot（编辑器热重载最好、Movie Maker 帧精确，但 `--headless` 是 dummy rasterizer **拿不到像素**、
web 导出最差）；自研 wgpu+naga（**3–6 人月**才到 parity，且没有成熟的浏览器回退；
它的唯一工程优势是 **naga 可脱离 device 独立校验 ⇒ 天然保留 last-good pipeline**）。

### 4.4 三个判据的横向对照（用户给的判据）

| | three.js WebGL2（现状） | three.js + TSL（WebGPU，WebGL2 fallback） | **Bevy + WGSL** | Godot 4 | wgpu + naga |
|---|---|---|---|---|---|
| **重载方便** | 需自建（SSE + 影子编译，~200 行） | 自建；TSL 是 JS ⇒ 重建节点图（资源泄漏语义未验证） | ✅ **内建资产热重载** | ✅ 仅编辑器内 | 自建；naga 独立校验最优雅 |
| **截图方便** | CDP + compositor + 自建确定性契约 | 同左 + WebGPU headless 旗标（未验证） | ✅ **无窗口离屏 + 帧精确 step** | ⚠️ `--headless` 无像素，要 Movie Maker | ✅ 离屏 readback（但全自建） |
| **agent 方便** | 中（多实例/缓存/GPU 抽签都是坑） | 中高（节点级错误 + 节点堆栈） | ✅ **一条命令 + RenderDoc + 直连 sim** | 中 | 低（什么都要自己造） |
| **迁移成本** | 0 | 中（post 链必须重写） | 高（WGSL 重写 + 后处理自建） | 极高 | 3–6 人月 |

### 4.5 编译经济学（用户 2026-09-13 追问：Bevy 编译速度？能否让渲染器脱离 PCG 的 crate 依赖？）

用户原话：*「我不在乎上不上浏览器，关键是 bevy 的编译速度支持 agent 快速迭代吗？或者说如果选择
rust 栈，能让渲染器和我们的 PCG 美术框架脱离 crate 依赖关系吗，避免美术修改导致渲染器/ecs
系统重新编译」*。

**本机实测**（rustc 1.97.1 / `x86_64-pc-windows-msvc` / LLVM 22.1.6）：

| 操作 | 时间 |
|---|---|
| `cargo build` 无改动（cargo 自身开销下界） | **0.07 s** |
| 改一行 `src/lib.rs`（根 crate，`game` 依赖它）→ 重编 + 链接 | **0.47 / 0.48 / 0.58 s** |
| `rust-lld.exe` | **已经躺在工具链里**：`<sysroot>/lib/rustlib/x86_64-pc-windows-msvc/bin/rust-lld.exe`（只是不在 PATH）⇒ 切 lld 只需 `.cargo/config.toml` 一行，**不用装东西** |

**Bevy 侧本机实测**（2026-09-13；真 app：`App::new().add_plugins(DefaultPlugins)` + `Camera3d`；
552 个 crate，冷编 193.8 s，产物 167.6 MB）—— ⚠️ **官方那条「`dynamic_linking` 影响最大」
在本机是反的**：

| 配置 | 改一行 `main.rs` → 重编 + 链接 |
|---|---|
| 默认（`link.exe` + 完整 debug info） | **22.6 / 19.5 / 20.1 s** |
| **lld**（`rust-lld.exe` 本来就躺在工具链里）+ 完整 debug info | 21.0 / 25.0 / 20.7 s —— **没有帮助** |
| **lld + `[profile.dev] debug = "line-tables-only"`** | **6.9 / 6.7 / 6.7 s** ✅（产物 167.6 → 150.7 MB） |
| `dynamic_linking`（官方口径「影响最大」） | **40.9 / 41.1 / 40.3 s** —— **反而翻倍** |

⇒ 三条结论：

1. **真正的成本是调试信息，不是链接器**。`debug = "line-tables-only"` 一项就买到 3×，
   而且保留 file:line ⇒ panic 回溯仍然可读（对 agent 很重要）。
2. **`dynamic_linking` 在 Windows/MSVC 上是负收益**，别照抄官方那页。
3. 剩下那 ~6.7 s 是「任何 Rust 改动」的地板；**美术迭代不该踩到它**（§10.4：图是数据 ⇒ 0 编译）。

（方法论：第一轮测量是**无效的**——`cargo init` 生成的 `main.rs` 没用到 Bevy，
量到的 0.58 s 只是「链一个空 main」。教训：**测编译速度必须让被测代码真的用到那个依赖**。）

**但真正要紧的是这一句：美术迭代的热路径根本不该经过 rustc。**

| 美术改动 | 是否触发 Rust 重编 |
|---|---|
| 改 shader 内容 | **0**（Bevy 的 WGSL 资产热重载 / three 的 SSE 通道，都只重建管线） |
| 改参数 / 调数值 | **0**（uniform 更新） |
| 改生成图（PCG 逻辑） | **0**（图是数据、求值器常驻 —— §10.4 已裁决为 Rust 数据驱动） |
| 加一个新 pass / 改渲染管线结构 / 改 ECS 系统 | **要重编**（Bevy）；而 three 里这是改 JS ⇒ **0** |

⇒ **Rust 栈的编译税只落在最后一类改动上**。从本仓 history 看（大气壳重做、云层重做、
日珥改顶点驱动、噪声编译爆炸），**绝大多数是 shader 内部改动**，少数是管线结构改动
⇒ 比例偏低但不为零。这一条是 Bevy 与 three 之间**唯一的结构性迭代速度差异**。

**能不能让渲染器脱离 PCG 的 crate 依赖？—— 能，三档，由强到弱：**

1. **进程隔离（推荐；且与渲染栈无关）**：PCG = Node 长驻进程（TS），产出 CAS blob / 共享内存，
   渲染器**完全不依赖 PCG crate**。改美术 = **0 行 Rust 重编**。
   这条同时兑现「TS 类型检查」与「零 Rust 编译」两个诉求，**并把「浏览器 vs 原生」从中
   美术迭代速度里解耦出去** —— 渲染栈于是可以纯按「截图方便 / 重载方便 / 与 sim 的关系」来选。
2. **数据契约 crate 平行分层**：`px_contract`（只含类型 + serde，冻结）← `px_render` 与 `px_pcg`
   **都依赖它、彼此不依赖**。⚠️ **依赖方向是硬的：渲染器绝不能依赖 PCG 实现**，否则每次美术改动
   都会重编 ECS / 渲染管线。节点注册走**数据驱动的注册表**，不是编译期依赖。
3. **dylib 热插拔**（`libloading` / `abi_stable` / `bevy_dynamic_plugin`）：能，但 Rust 没有稳定 ABI，
   跨边界只能传 `repr(C)` 或序列化，且社区明确劝退热重载场景 ⇒ **不作为主机制**。

**若最终选 Bevy，快编译清单（缺一不可）**：`dynamic_linking` + lld（本机已具备）+ 
`[profile.dev] opt-level=1`（自己的 crate）/ 依赖 `opt-level=3` + **Defender 排除 target 目录** +
独立 target dir + `cargo nextest`（本仓已在用）。

---

## 5. Agent 截图友好：还缺 6 件事（现有 harness 已很硬）

1. **热重载**（最大杠杆）：迭代延迟从「冷编译 6–14 s + 重载整页」降到**一次 rAF / 一次 re-eval**。
2. **「画面空白」必须能分辨是哪一种失败**（现在四种症状都是黑屏）：
   - **GPU 身份断言**：每次捕获都查 `UNMASKED_RENDERER_WEBGL` 是否匹配白名单（本机两块 GPU，
     浏览器默认可能落在 Intel Iris Xe）；
   - **context 断言**：`canvas.getContext('webgl2') !== null` 且 `!gl.isContextLost()`
     —— ⚠️ **Edge 144 起 SwiftShader 被弃用，WebGL context 创建会直接失败**（不是静默降级），
     这会伪装成 shader bug；
   - **三通道 shader 错误采集**：① 逐 program `LINK_STATUS` + `getProgramInfoLog`（唯一能指出「哪个物体」）
     ② CDP `Runtime.consoleAPICalled` + `Log.entryAdded` 关键词 ③ 不变量 `(triangles===0) !== (image_blank)`。
     **错误通道优先级最高**：着色器编译失败时**不要把图像指标交给 agent**，否则它会去调视觉参数而不是修语法。
3. **通用「物体消失 / 纯白」判据**：现在靠每个场景手写 `limits`；应做成默认量具
   （非背景像素占比、clipped white/black 比例、tile 网格 Δ、edge density、熵、RAPS 斜率）。
4. **资产指纹写进 shot JSON**：`graphHash / nodeHashes / shaderHash / tier / camera / GPU / DPR / toneMapping`
   ⇒ 基线漂移能立刻归因到**哪一层**变了，而不是「图变了」。任何一项变 ⇒ **基线自动作废**。
5. **`run.mjs --watch`**：SSE 驱动，改文件即重拍并覆盖图 ⇒ agent 侧就是一条 `read_image`。
6. **确定性地基**（已有部分，补两条）：
   - 捕获帧**与 rAF 解耦**：页面暴露 `__step(n)` / `__renderOnce()`；自检 = 「同 seed 同时刻连拍两次
     sha256 相同」，不成立就**拒绝写基线**（这是所有其他 gate 的前置条件）。
   - **readback 优先级**：应用内 `gl.readPixels` → base64 **>** `Page.captureScreenshot` **>** `toDataURL`
     （前者绕开 compositor/rAF，是最确定的一条）。

参考图仍按既有裁决用：**只抽无量纲统计量（分位数比例、chroma/hue 分布、RAPS 斜率）当判据，
不比像素、不进构建**。

---

## 6. 门禁设计（沿用现成文化）

| 门 | 抓什么 | 与现成门的关系 |
|---|---|---|
| `node --check` / **`tsc --noEmit`** | ES module / TS 语法与类型 | 扩 `check-js.sh` 第 ③ 道 |
| `check-glsl-manifest` / `check-shaders` | GLSL 清单 + glslang 编译 | **不动**（现有 3 道保留） |
| `check-gen-manifest` | 一个节点一个文件 + 登记齐全 | 抄 `check-glsl-manifest` 的形状 |
| `check-gen-hash` | 同参数两次求值 hash 必须相同 | **确定性门**（新） |
| `check-gen-dry` | 图能拓扑排序、无环、无未注册节点、无全局 RNG、无 `sin`-hash | 新 |
| `shots --strict` | 像素判据 + 基线 | 现成，补 §5 的 3、4、6 |

---

## 7. 待裁决（用户 2026-09-13 已答的标 ✅）

1. ✅ **DAG 形态**：要**类型检查** ⇒ **TS**（实测 Node 24 零构建直接跑）。
   细则待定：求值器放 **Node 长驻**（推荐：浏览器只收数据，且天然避开「浏览器内 dynamic import
   绕缓存会泄漏 ESM」那条坑）还是浏览器内？
2. ✅ **图的作用域**：**兼管 sim 参数** —— 从图的类型生成 Rust `SurfaceParams` / `class_index`
   ⇒ 真正消灭 §1.2 那类「四处手工同步、错了不报错」。
3. ⏳ **头号：渲染栈**。用户 2026-09-13 原话：*「我不在乎上不上浏览器，关键是 bevy 的编译速度
   支持 agent 快速迭代吗？或者说如果选择 rust 栈，能让渲染器和我们的 PCG 美术框架脱离 crate
   依赖关系吗，避免美术修改导致渲染器 / ecs 系统重新编译」*。两问都已在 §4.5 回答：
   - **编译税只落在「加新 pass / 改管线结构 / 改 ECS 系统」这一类改动上**；shader / 参数 / 图的改动 **0 编译**；
   - **能脱离**：① 进程隔离（PCG 走 Node 侧车，渲染器不依赖 PCG crate）② 数据契约 crate 平行分层。
   ⇒ 选择于是退化成二选一：**Bevy（内建热重载 / 帧精确离屏截图 / 直连 sim / compute）
   vs three（引擎侧改动零编译 / 已有 24 stage + 后处理 / 玩家侧 UI 本来就是 web）**。
   本机 Bevy 实测（冷编 + 改一行重编 + `dynamic_linking` 前后）后台跑着，数字回填。
4. ⏳ **是否立刻做 §4.5 那套架构**（Node PCG 侧车 + 契约 crate）—— 它**与渲染栈无关**，
   是先做先收益的一步，做完之后渲染栈可以慢慢选。
5. ⏳ **`px_contract` 的边界**：哪些类型进契约（= 冻结、很少变），哪些留在 PCG 内部。
6. ⏳ **是否把噪声烘成贴图**（`shader-compile-stall.md` §4 的三选项：砍 fbm 调用点 / 烘贴图 / 定 oct 上限）。
7. ⏳ **几何是否引 `manifold-3d`**（WASM +~1 MB，换来鲁棒布尔 + 跨平台确定性）。
8. ⏳ **是否开一个 spike**（一小时量级）：验 `WebGPURenderer` 吃不吃裸 GLSL、`NodeMaterialLoader`
   往返保真、`compute()` 在 WebGL 后端的行为；顺带验 Babylon 的运行时 GLSL→WGSL 是否还成立。
9. ⏳ **`refs/` 是否加一条 deny 路由**（现在参考图与服务静态根的关系未核）。
10. ✅ **落地方式**：先只写笔记，不动 worktree（本文即该动作的产物）。

---

## 8. 证据分级

**A 级（实测 / 官方来源，可直接引用）**
- 本机实测：Node **v24.14.1** 直接跑 `.ts`（strip-only；`enum` 报 `ERR_UNSUPPORTED_TYPESCRIPT_SYNTAX`）；
  冷启动 **43–56 ms**；常驻进程 1e6 次浮点 17 ms。
- three.js：`WebGPURenderer` 不支持裸 GLSL（[forum 84845](https://discourse.threejs.org/t/having-a-hard-time-figuring-out-tsl-for-webgpu-support-dqs-implementation/84845)、
  [forum 87974](https://discourse.threejs.org/t/tsl-and-webgl2-vs-webgpu/87974)）；r164 起 `WebGLRenderer` 移除 NodeMaterial
  （[forum 64909](https://discourse.threejs.org/t/r164-nodes-no-longer-working-with-webgl-webgl2/64909)）；
  **r183 `RenderPipeline`**（[2026 后处理指南](https://threejsroadmap.com/blog/the-complete-guide-to-threejs-post-processing-in-2026)）；
  TSL 可在 WebGL 与 WebGPU 两用（[Maxime Heckel: Field Guide to TSL and WebGPU](https://blog.maximeheckel.com/posts/field-guide-to-tsl-and-webgpu)）；
  `compileAsync`/`KHR_parallel_shader_compile`（[forum 56572](https://discourse.threejs.org/t/reducing-shader-compile-time-on-scene-initialization/56572)）、
  `onShaderError`（[forum 40770](https://discourse.threejs.org/t/displaying-shader-error-in-console/40770)）。
- Manifold：v3.5.3 / Apache-2.0 / WASM 可指定 `wasmUrl`（[repo](https://github.com/elalish/manifold)、
  [npm](https://registry.npmjs.org/manifold-3d/latest)、[discussions/372](https://github.com/elalish/manifold/discussions/372)、
  [v3.5.0 release](https://github.com/elalish/manifold/releases/tag/v3.5.0)）；Rust 绑定 `manifold-csg`（[lib.rs](https://lib.rs/crates/manifold-csg)）。
- meshoptimizer 1.2.0 / MIT / JS API（[js README](https://github.com/zeux/meshoptimizer/blob/master/js/README.md)、
  [v1 说明](https://meshoptimizer.org/v1.html)）；`SimplifyModifier` 劣于它（[forum 63002](https://discourse.threejs.org/t/mesh-simplification-using-meshoptimizer/63002)）。
- xatlas（MIT，[repo](https://github.com/jpcy/xatlas)）；**xatlas-web 已归档**（[repo](https://github.com/MozillaReality/xatlas-web)）。
- **Edge 144+ SwiftShader 弃用 ⇒ WebGL context 创建直接失败**（[Microsoft Learn](https://learn.microsoft.com/en-us/deployedge/microsoft-edge-policies/enableunsafeswiftshader)）。
- headless 时间线：`--headless=old` 已不存在（M132，[Chromium headless README](https://chromium.googlesource.com/chromium/src/+/lkgr/headless/README.md)）。
- 噪声查表实践（[Khronos 论坛](https://community.khronos.org/t/perlin-noise-in-a-fragment-shader/46986)）；
  「贴图永远比 procedural 便宜」（[Unity 讨论](https://discussions.unity.com/t/comparing-performance-of-textures-vs-procedural-shaders-on-mobile-gpu/662156)）。
- Rust `rand` 可复现性政策（[Rand Book](https://rust-random.github.io/book/crate-reprod.html)）；
  浮点跨平台不确定（[Gaffer On Games](https://gafferongames.com/post/floating_point_determinism)）。
- Salsa（[repo](https://github.com/salsa-rs/salsa)）；LiteGraph.js（客户端+服务端都能跑的图引擎，可作**可选**可视化编辑器层）。
- Babylon `serialize()`/`Parse()` JSON（[serializationTools.ts](https://github.com/BabylonJS/Babylon.js/blob/master/packages/tools/nodeEditor/src/serializationTools.ts)）。
- 原生 ESM 绕缓存会泄漏（[ar.al](https://ar.al/2021/02/22/cache-busting-in-node.js-dynamic-esm-imports)）。

**B 级（⚠️ 未核实，用前必须复核）**
- Babylon 的**运行时 GLSL→WGSL 转译**是否仍有效（若成立会显著改变 §4.3 的成本评估）。
- Bevy / Godot / wgpu-naga / Slang 的**具体版本号与 API 名称**（Bevy 坏 WGSL 是否保留旧 pipeline、
  Godot Movie Maker 旗标、naga GLSL 前端对 `#version 300 es` 的支持矩阵）。
- three 的 `NodeMaterialLoader` 往返保真度、TSL 节点图重建的 `dispose`/泄漏语义、
  `compute()` 在 WebGL 后端的确切行为、`bloom()` 是否为 TSL 显示函数。
- Houdini 定价与授权边界（Indie 格式与商业 license 不互通）。
- WebGPU 浏览器覆盖矩阵与 headless 旗标组合。

---

## 9. 复现命令（本轮实测用的）

```bash
node --version                                  # v24.14.1
printf 'type P = {a:number};\nconst x: P = {a: 1};\nconsole.log(x.a);\n' > /tmp/p.ts && node /tmp/p.ts
# ↑ 直接跑通 ⇒ 零构建 TS；把 enum 加进去 ⇒ ERR_UNSUPPORTED_TYPESCRIPT_SYNTAX
for i in 1 2 3 4 5; do /usr/bin/time -f '%e s' node -e '0'; done   # 冷启动 ~45 ms
```

---

## 10. 已裁决（2026-09-13）：走 Rust 栈 + 协议 crate

用户原话：*「那我们就走 rust 栈吧，sim 和 renderer 之间用一个协议 crate 规定数据格式，
两边都只依赖这个协议 crate」*

### 10.0 第二轮裁决（2026-09-13）与它带来的四个新结论

| 裁决 | 取值 |
|---|---|
| 血缘 / 落地位置 | **`v2`（当前 checkout）** |
| sim ↔ renderer | **跨进程** —— 协议 crate **同时是 wire 格式** |
| 模块划分 | **三个模块 `sim` / `pcg` / `render`，全部 Rust**，通过协议 crate **序列化数据**连接 |
| PCG 求值器 | **Rust 数据驱动求值器**（§10.4；Node 侧车方案撤回） |

**结论 1 —— wire 格式要分两档，别用 bincode 当长期格式。**
控制面/状态（小、要能被人和 Python kit 读、要能进 `--digest`）走 **serde_json**（本仓既有事实）；
大数组（mesh / heightfield / 场纹理）走 **header JSON + 原生 little-endian payload**
（`dtype`/`shape`/`stride` 写在 header 里）。bincode / postcard 快但没有 schema 演进能力，
不适合当**跨版本**的长期 wire 格式。

**结论 2 —— 跨进程的代价必须用握手兜住。** 本仓为「连错实例」白烧过七八轮
（`glsl-files.md` 结尾）。握手报文必须带：`SCHEMA_VERSION` + `protocol_hash` + `pid` + `exe` 路径
+ `git rev`，**对不上就拒绝连接**——这是现有 `/api/ping`（只对 exe 路径）的直接延伸。

**结论 3 —— 但跨进程还白送一个巨大的收益：渲染器可以消费「录制的流」而不是活的 sim。**
`pxsim --record world.pxstream --round 240` 跑一次，之后美术迭代**根本不需要 sim 在跑**，
而且渲染器的输入变成一个**文件**（可 hash、可 diff、可当基线、可进 CI）。
⇒ 在美术迭代这条路径上，「连错了实例」这一类问题**直接清零**。

**结论 4 —— 数据必须单向流：`sim → pcg → render`，pcg 的产出绝不回流 sim。**
否则美术改动会污染 sim 的确定性，同 seed `--digest` 基线（本仓的核心验收手段）立刻失效。

拓扑：render 同时需要 **sim 状态**（位置/选择/城市）与 **pcg 产出**（资产），
所以协议 crate 内含两个族：`sim::*` 与 `art::*`。

**⚠️ v2 血缘的现实（重要）**：`v2` 的 `src/state.rs` 目前是 **`pub struct State {}`（1 行）**，
整个血缘约 4k 行，**没有 `web/`、没有 `.agents/`、没有 `scripts/shots/`**；
而**全部美术工作**（24 个 GLSL stage、shots 判据体系、基线、笔记）在 `feature/glsl-files`
那条**孤儿血缘**上。⇒ 两条推论：
① 协议 crate 在 v2 上是**建骨架**，不是「抽已有投影面」（我上一轮那句基于另一条血缘，**作废**）；
② 「搬视觉语言」实际只能是**按文件拷贝**（`git checkout feature/glsl-files -- <path>`
不受血缘限制），shader / 工具 / 笔记能搬，但要连带它们的依赖一起搬（场景表、metrics、tuning）。

### 10.1 crate 图（依赖方向是硬约束）

```
                    px_protocol        ← 只有类型 + serde + SCHEMA_VERSION
                    ↑     ↑     ↑         无逻辑、无内部依赖、几乎不变
                    │     │     │
              px_sim │  px_render │  px_web
              (权威) │   (Bevy)   │  (现有 axum JSON API + 现有 web UI)
```

- `px_render → px_protocol ← px_sim`：**渲染器不依赖 sim，sim 不依赖渲染器**。
- `px_web → px_protocol ← px_sim`：现有 JSON API **从协议类型序列化** ⇒ web UI 与原生渲染器
  **消费同一份类型**，两边不会各自演化出第二套形状。
- 于是：改 sim 内部 / 改渲染器内部**互不重编**；改协议**两边都重编**
  —— 这正是协议该有的性质，所以它必须**冻结、必须小**。

### 10.2 语言管不住的那一个漏洞 ⇒ 用门看住（必须写进理由）

**Rust 不禁止传递依赖**：`px_render` 完全可以 `use px_sim::...` 而**不报任何错**，
于是「两边都只依赖协议 crate」在语言层面**根本没有强制力**，它会静静退化。

⇒ 新门 **`scripts/check-crate-graph.mjs`**：读 `cargo metadata --format-version 1`，断言
① `px_protocol` 的依赖集合 ⊆ 白名单（`serde`，也许 `glam`）；
② **不存在** `px_render → px_sim` / `px_web → px_sim` 的**任何**依赖路径（不是直连，是可达性）。
本仓一贯的做法是「结构上消灭，而不是继续靠一道门看住」——**这里是结构上做不到，
所以只能靠门看住**，理由要写在门的注释里（否则下一个 agent 会以为它是冗余的）。

### 10.3 协议版本与快照（沿用本仓已有的两个惯例）

- 协议 crate 拥有 `SCHEMA_VERSION`（本仓已有 9→10→13 的先例，只是那个住在 sim 里）。
- 新快照测试：把 `WorldView` 序列化成**稳定 JSON**，与 `px_protocol/tests/protocol.snapshot.json`
  **逐字**比对 ⇒ 改协议**必须显式更新快照**（与 `--digest` 基线同一个思路）。
- 精度纪律：**sim 权威 f64，渲染消费 f32**；换算只允许出现在协议 crate 里的**一处**（写明在哪）。

### 10.4 顺带把编译速度问题彻底解决：**图是数据，不是 Rust 代码**

这是本轮最重要的修正。上一轮我为了拿「美术改动 0 编译」建议了 **Node 侧车**；
**同一个收益在纯 Rust 里也能拿到，只要图是数据**：

| 层 | 形态 | 改它的代价 |
|---|---|---|
| 图的**结构 + 参数 + seed** | `.pxg.ron` / `.pxg.json` **数据**文件 | **0 编译**（当资产热重载，或自己的 watch） |
| shader | `.wgsl` **资产** | **0 编译**（Bevy 内建资产热重载） |
| 节点的**实现**（新算子类型） | Rust 代码 | **1–3 s**（`dynamic_linking` + lld） |
| 渲染管线结构 / 新 pass / ECS 系统 | Rust 代码 | **1–3 s** |

⇒ **日常美术迭代（改图、调参、改 shader）= 0 编译**；只有「加新算子 / 加新 pass」才付编译。
于是 §4.5 的 tier-1（Node 侧车）**不再必要**：Rust 常驻求值器 + 数据图 = 同等收益，
且不引第二语言、不引进程边界；几何库直接用 **Rust 绑定**（`manifold-csg`）而不是 WASM。

⚠️ 因此 **§2.2（TS 实测）与 §2.3（QuickJS）在「走 Rust 栈」下大部分作废**，
但两条结论仍然有效、并被本方案直接继承：
① **热重载的是数据，不是代码模块**（§2.2 末）；② **禁止双实现**（§2.6）。
另外 §2.7 的确定性铁律原样适用（整数 hash、`rand_chacha`、GPU 不当基线来源）。

### 10.5 分阶段（**在 v2 上**）

- **P0 建协议骨架**：`px_protocol`（类型 + serde + 两档 wire 编解码 + 快照门 + crate 图门）。
  在 v2 上它承载的是现有 `State {}` 的投影面（很小）⇒ P0 的重点是**把管线立起来**，不是搬运。
  **验收（自证，不需要审美裁决）= 一份 `--record` 出来的 `.pxstream` 能被重放成同一个 `WorldView`，
  且两次序列化逐字节相同。**
- **P1 起 `px_pcg`**：数据图（`.pxg.ron`）+ Rust 常驻求值器 + CAS；先不接渲染，
  只出 `art::*` 数据，用 CLI + 快照验收。
- **P2 起 `px_render`（Bevy）**：只读协议（+ 录制流）；先渲染最简场景；
  headless 截图命令 + 判据按 `scripts/shots` 的思路重建（v2 上没有那套，要拷或重写）。
- **P3 搬/重写视觉语言**：24 个 GLSL stage → WGSL（原文件可从 `feature/glsl-files` 拷贝参考），
  后处理按 §4.3 的顺序迁；每步像素对照 + 帧时间对比。

### 10.6 唯一还没解决的结构问题：web UI 与原生渲染器的**重叠**

> ⚠️ **在 `v2` 血缘上这一节暂时不成立**：v2 **没有 `web/`**（它在 `feature/glsl-files` 那条孤儿血缘上），
> 所以「两套渲染器并存」在 v2 上根本不会发生。将来若要把 web UI 拿回来当第二消费者，
> 再回来看这一节（协议 crate 会让它便宜很多）。

现有 web UI（状态面板 / 控制行 / 设计图库 / `map3d`）是一大笔投入。走原生之后只有三种收场：

1. **web UI 保留但去掉 3D**（原生 app 负责 3D，web 继续管面板）；
2. **整体转原生**（`bevy_egui` 之类把 UI 也重做）；
3. 一段时间内**两套 3D 并存** —— ⚠️ 违反 §4.4 的反面纪律（两个渲染器 = 双实现漂移），
   只能是**有期限的过渡**，且必须写清截止条件。

**待用户裁决。**

### 10.7 待裁决（本节更新）

1. ⏳ **`.pxstream` 录什么**：sim 的**完整状态**，还是**render 需要的那部分投影**？
   （决定文件大小，也决定「美术能脱离 sim 迭代到什么程度」）
2. ⏳ **Bevy vs wgpu**：Bevy 白拿内建热重载 + 帧精确离屏 + ECS/render graph，
   代价是版本 churn 与编译体量；wgpu 自己拿管线、无 churn，但整套 VFX 要自建。
3. ⏳ **web UI 的去留**：v2 上**没有 `web/`**（它在另一条血缘）⇒ 这个问题在 v2 上暂时不存在；
   将来要么不做 web，要么从 `feature/glsl-files` 拷 `web/` 过来当**第二个消费者**（协议 crate 让这变得便宜）。
4. ⏳ 其余见 §7 的 4–9 项（是否烘噪声 / 是否引 manifold / spike / refs deny 路由）。

---

## 11. P0 完成记录（2026-09-13，分支 `feature/art-stack`）

worktree：`.worktrees/art-stack`（off `v2` @ `3dfd8b2`）。`cargo test --workspace` **全绿（125 个）**。

| 件 | 内容 |
|---|---|
| `px_protocol` crate | `sim::*`（`WorldView`）/ `art::*`（`ArtBundle`、`AssetManifest`）/ `wire::*`（**两档**：JSON 控制面 + `header JSON + LE payload` 二进制块）/ `stream::*`（`.pxstream` 录制与重放）/ `ProtocolId` + `Handshake` / `SCHEMA_VERSION` |
| **门 1** `tests/crate_graph.rs` | ① `px_protocol` 运行时依赖集合**必须恰好 = `{serde, serde_json}`**；② **不存在** `px_render`／`px_web` → `px_sim` 的依赖（直接解析各 `Cargo.toml`，而不是在测试里调 `cargo metadata`——那会和 cargo 自己的锁死锁） |
| **门 2** `tests/snapshot.rs` | 协议形状快照逐字比对；只有 `PX_UPDATE_SNAPSHOT=1` 才允许更新 |
| 协议指纹 | `protocol_hash()` = 快照字节的 FNV-1a 64 ⇒ **形状一变指纹就变，跨进程握手自动拒绝旧对端**（门与运行时是连着的） |
| 录制入口 | `cargo run -p game -- -n 240 -s 11 -f 0.4 --record target/x.pxstream` |
| 自证 | 录制 → 重放 → 再录制**逐字节相同**；帧里 `round` 稠密有序；内容**确实随轮次变化**（防「录了一坨常数」） |

### 11.1 P0 抓到的两个真问题

**① 会话身份混进了内容**（我的设计错）。第一版 `Handshake` 带 `pid`/`exe` 且被录进流里
⇒ **同 seed 两次录制字节不同**（实测 `0ED5…` vs `7EA2…`）。修法：拆成
**`ProtocolId { schema_version, protocol_hash, git_rev }`（稳定 ⇒ 进流）**
与 **`Handshake { id, pid, exe }`（易变 ⇒ 只用于连接握手）**。
一句话判据：**一个字段该不该进内容，看它跨进程/跨次运行是否稳定。**

**② 这个 sim 在默认参数下与 seed 无关**。`DomesticEconomy::new` 里 `with_fluctuation(0.0)`
⇒ RNG 没被消费到影响聚合值的地方 ⇒ `-s 11` 与 `-s 12` 录出的文件**逐字节相同**（实测）。
不是 bug，是既有性质，已固化成测试 `zero_fluctuation_makes_the_seed_irrelevant`
（**它一旦失败，说明 sim 开始在 f=0 时消费 RNG，基线口径要重定**——这条测试是留给未来的警报）。
为了让录制能造出真正随机的世界，CLI 新增 `--fluctuation/-f`：实测 `-f 0.4` 下
**同 seed 可复现、换 seed 不同**（两种性质都有测试守）。

⚠️ 踩过的坑：`game` 头部会打印 `种子 {seed}`，所以**「换 seed 输出不同」不能靠字符串比较判定**
——第一版验收就是这么误判的，改成直接比录制文件的 SHA256 才看见真相。

### 11.2 尚未做（P0 之外）

- `.pxstream` 目前只真正用了 `Protocol` / `World` 两类帧；**`Art` / `Blob` 两条通路有类型、
  有往返测试，但没有生产者**（P1 `px_pcg` 的任务）。
- **直播通道没做**（socket / 握手校验 / 背压）——按 §10.0 结论 3，美术迭代走**文件流**即可；
  直播通道等 `px_render` 真起来再说。
- 这个血缘上**没有 `.agents/notes.md` 索引文件**，所以本笔记暂时没有索引行。
- 主 worktree 下的 `target/bevy-probe/` 是编译探针（在 gitignore 的 `target/` 里），用完应删。
  ⚠️ 教训：**`cargo init` 会自动把新包登记进根 manifest 的 `members`**（本轮污染过一次，已恢复）。

---

## 12. P1 完成记录：px_render 骨架（提交 `25ddd47`）

**做了什么**：`px_render`（Bevy 0.19.1）读 `.pxstream` → **离屏渲染** → PNG。

```
cargo run -p game     -- -n 240 -s 11 -f 0.4 --record target/world.pxstream
cargo run -p px_render -- --stream target/world.pxstream --round 200 --out target/shot.png
```

- 渲染内容：后排三根**部门库存柱**（按商品配色堆叠）、前排三根**价格柱**；
  高度按**本流自身的峰值归一化**（相对比例，不是绝对量——沿用本仓「相对比例优于硬阈值」的约定）。
- 协议指纹不匹配的流会被**拒绝**（`px_render` 显式比对 `ProtocolId.protocol_hash`）。
- 实测：同一轮跑两次**逐字节相同**（960×640；第 60 / 200 / 240 轮都验过，且与固定帧数预热版本
  的 sha 完全一致——因为场景是静态的，这也顺带证明了改造前后画面等价）。

### 12.1 踩到的四个坑（都记下来，别再踩）

**① 窗口路径在这台机器上画不出东西。** 最初开窗渲染，画面**只有清屏色**；把清屏色改成洋红后
确认「什么都没画」。**把 Bevy 官方 `screenshot` 例子原样复刻后同样空白** ⇒ 不是我们的场景问题，
是窗口/交换链这条路径在这台机器上不行（DX12 与 Vulkan 都试了；Vulkan 还刷一堆
`vkAcquireNextImageKHR` semaphore 校验错误）。
⇒ **改用离屏 render-to-image**（`Image::new_target_texture` + `RenderTarget::Image` +
`Screenshot::image`），配合 `WindowPlugin { primary_window: None, .. }` +
`disable::<WinitPlugin>()` + `ScheduleRunnerPlugin`。
**这条路本来就是美术迭代该走的**：无窗口、无 DPI 缩放、`--width/--height` 就是像素数。

**② 离屏也会拍到空白画面——Bevy 的管线是异步编译的。** 20 帧、120 帧都是空白，**400 帧才有内容**。
根因：`CachedPipelineState::Creating(Task<…>)`。编译完成前画面里没有几何，**但清屏色正常**，
所以看起来像「场景写错了」。⇒ **别用固定帧数预热**，正解是读渲染世界的 `PipelineCache`：

```rust
app.get_sub_app_mut(RenderApp).unwrap()
   .insert_resource(RenderReady(flag.clone()))   // Arc<AtomicBool>，两个世界各插一份
   .add_systems(Render, watch_pipelines);
```

**③ 就绪门写成「所有管线都是 Ok」会立刻开**（我的第一版 bug）：队列为空时 `pending == 0`
天然成立，第 3 帧就"就绪"，照样拍到空白。**必须加「至少有一条管线」**：

```rust
if total > 0 && pending == 0 { ready.store(true) }
```

修正后：41 条管线 / 首帧 20 条待编译 → **第 167 帧就绪** → 从启动到出图 **7.5 s**
（固定 600 帧是 14.7 s，而且随时可能拍到空白）。

**④ DPI 会偷偷改尺寸。** 开窗时 `WindowResolution::new(960, 640)` 出来的截图是 **1680×1120**
（这台机器 175% 缩放）⇒ 要 `with_scale_factor_override(1.0)`。离屏路径没有这个问题。

### 12.2 「画面空白」的四种成因与定性手段（可直接抄进未来的门）

四种失败症状都是「只有背景色」：① GPU/后端不对 ② 管线还没编译完 ③ 几何被剔除 ④ 真的没画。
本轮**四种都遇到过**，定性手段分别是：

| 手段 | 一眼看出什么 |
|---|---|
| **把清屏色改成洋红** | 区分「什么都没画」与「画了但很暗」——一秒钟的事，先做这个 |
| **打印 `ViewVisibility`** | 剔除还是没画（本轮 3 根柱子堆到 12 米高跑出画面被正确剔除，一度让我误判「全被剔除了」） |
| **打印管线队列**（总数 + 待编译数） | 「还没编译完」还是「编译失败」 |
| **官方最小例子复刻** | 分清「我的场景错」与「环境/后端错」——本轮靠这条才没有继续在场景里瞎找 |

⇒ 前三条应该成为 `px_render` 的常驻诊断（现在 `diagnose` 打印可见性，管线状态在就绪时打印一次）。

### 12.3 还没做

- **没有判据门**：现在只出 PNG，没有基线、没有 `stats`、没有阈值 —— 出图要人（或 agent）看。
- **一次出图 7.5 s**，大头是管线编译。Bevy 能否借 wgpu 的磁盘管线缓存把第二次降到 <1 s，未查。
- **`Art` / `Blob` 帧还没有消费者**：`px_render` 只读 `World` 帧。
- 场景是「能看出状态」的最小可视化，**离真正的美术层（行星 / 星空 / 光照）还很远**。
- 编译经济性已落地：根 `Cargo.toml` 有 `[profile.dev] debug = "line-tables-only"`，
  实测改一行 `px_render` 重编 ≈ **6.7 s**（完整 debug info 是 ~20 s）。

---

## 13. P1.5：常驻渲染服务（用户要求：「截图是持久进程，模块随时要图」）

用户原话：*「让 渲染器，截图 是持久进程，美术/sim 模块可以随时发起渲染请求然后获得一张图供
agent 查看」*。

```
px_render --serve [--port N] [--width W] [--height H]   # 常驻服务，启动时预热管线
px_render --round 200 --out shot.png                    # 客户端：连上服务要一张图
```

- **wire = `px_protocol` 的 Frame 流**（新增 `Request` / `Response` / `Refused` 三种帧），走 TCP 127.0.0.1。
- **握手**：`ProtocolId`（schema 版本 + 协议指纹 + git rev）双向校验，不匹配直接 `Refused` 并报出双方指纹。
- **租约文件** `target/render-server.json`（pid / 端口 / 指纹 / rev / exe）；
  **删掉租约即停服务**（服务每 120 帧自查，实测 4 秒内自行退出）。
- 任意 Rust 模块可以直接调 `px_protocol::client::request(Request { .. })` 要图，不必经过 CLI。

**实测（同一台机器）**：

| 路径 | 出图耗时 |
|---|---|
| 一次性进程（§12） | **7.5 s** |
| 常驻服务第一次请求（含服务预热） | 2.9 s |
| **常驻服务后续请求** | **0.33 s**（服务端 267–280 ms） |

⇒ **约 24×**。而且服务端出的图与一次性路径**逐字节一致**（60 / 200 / 240 三轮都比过）。

### 13.1 这一段抓到的三个 bug（都属于「看起来对、其实错」）

**① 空白图：把相机和世界一起 despawn 了。** 第一版每来一个请求就 despawn 所有 `ScenePart`
再重建 —— **相机也在里面**，而相机只在尺寸变化时才重建 ⇒ 之后没有任何相机在渲染，
四张图全是 10757 字节的背景色。修法：分成 `StageCamera` / `Stage`（灯）/ `WorldPart` 三类，
重建世界时只动 `WorldPart`。

**② 换尺寸会弄丢灯。** resize 分支里我 despawn 了全部 `Stage` 却只重建相机 ⇒ 环境光与方向光消失，
画面随之变化。**是靠比 SHA256 发现的**：960×640 → 480×320 → 960×640 得到 325212 字节，
而基准是 326452。修好后往返逐字节回到基准。
⇒ 教训：**「改尺寸再改回来应该回到原样」是一条免费的强断言**，它替我抓到了这个 bug。

**③ 自动拉起会把调用方挂死。** 客户端发现没有服务时顺手 spawn 一个 ⇒ **服务成了客户端的子进程**，
于是任何等整棵进程树结束的调用方（PowerShell 作业、`cargo test`、DSH 本身）都会一直等下去。
⚠️ 这个坑特别阴：**服务端日志显示它一切正常、图也出了**，只有调用方挂着。
⇒ 默认改成「连不上就快速失败并打印怎么起服务」（实测 2.37 s 失败并给提示），
自动拉起降级为显式 `--autostart`。

### 13.2 使用须知（会咬人的两条）

- ⚠️ **服务运行时 `target\debug\px_render.exe` 被占用 ⇒ `cargo build` 报「拒绝访问」。**
  改代码前先停服务：**删掉 `target/render-server.json`**（4 秒内自查退出）或 `Stop-Process`。
  （我第一次就踩了，还以为编译坏了。）
- 请求里的 `stream` 是**相对路径，由服务端 cwd 解析**；客户端与服务端 cwd 不同时要用绝对路径。

### 13.3 还没做

- **仍然没有判据门**（§12.3 那条还在）：出图要人 / agent 看，没有 stats、基线、阈值。
- 服务是**单请求串行**的（一次一个 job）；并发请求会排队。
- `Art` / `Blob` 帧仍然没有消费者。
- 场景仍是 3×3 经济体的最小可视化。

---

## 14. PCG 层：类 Houdini 的缓存 DAG 该怎么落（设计，尚未实现）

用户原话：*「思考一下 PCG 层就应该怎么实现类 houdini 的缓存 DAG。用脚本不用重新编译，
但需要连接 rust 端的算子，某些脚本语言类型检查弱小；直接 rust 写编译时间长」*。

### 14.0 四个约束里，有一个根本不是 Rust 的问题

(A) 脚本不用重编 (B) 要能连 Rust 算子 (C) 脚本类型检查弱 (D) 直接 Rust 写编译慢。

**关键观察：(A)(C)(D) 只有在「脚本必须是另一门语言」这个前提上才互相冲突。** 而 (D) 这个数实测下来
不属于 Rust，属于 **Bevy 的链接**：

| 改一行后重编 | 时间 |
|---|---|
| Bevy app（`px_render`，链 552 个 crate） | **6.7 s** |
| 纯 Rust 库（`px_protocol`，不链 Bevy） | **0.52 / 0.53 s**（首次 0.88） |
| 纯 Rust 库 + 测试目标 | **1.02 s** |
| **cdylib 算子插件**（serde + serde_json，产出 106 KB DLL） | 冷编 7.5 s；**增量 0.25 / 0.25 / 0.23 s** |

⇒ **7 s 是 Bevy 的链接税，不是 Rust 的税。** 一个「算子 crate」改一行是 **0.25–0.5 s**，
和任何脚本语言的热重载同量级，而且**类型检查是满的**。

再加上 §10.0 已决定 **sim / pcg / render 跨进程、协议 crate 当 wire**：PCG 进程可以随时重启，
**不需要进程内热重载**。「脚本」存在的最后一个理由（免重编）也就弱了。

### 14.1 脚本还有位置吗？——有，但只在很窄的两处

1. **参数表达式**（Houdini 里九成的"脚本"其实是 `ch("../ridge/scale")*2`）：需要一个
   **极小的带类型 AST 求值器**（约 200 行），**不是一门语言**。
2. **逐元素片段**（VEX 那种 per-point 代码）：先用算子组合覆盖（map / remap / blur / scatter / warp），
   **先不做**。

⇒ 结论：**不引入通用脚本语言**。图是数据、算子是 Rust、表达式是小 AST。
将来真要引语言，也要先做到「从算子注册表生成带类型的绑定」，而不是反过来。

### 14.2 缓存 DAG 的骨架（真正的难点，与语言无关）

**① 币种就是协议里的 `ArtBundle`。** 节点输出 = 一个 `ArtBundle`（`AssetKind` + params + `Blob`）；
缓存里存的东西**就是渲染器要吃的东西**，中间不需要转换层 —— 这是 §10.0「协议 crate 当 wire」的直接红利。

**② 缓存键是纯函数，不含任何易变身份**：

```
node_key = H( op_id ‖ op_version ‖ 求值后的参数 ‖ [各输入的 key] )
```

- **不含**路径、时间、pid、主机名（P0 的教训：易变身份混进内容就毁掉可复现性）。
- 文件**只贡献内容哈希、不贡献路径** ⇒ 挪文件不会让缓存失效。
- **「脏」不是一个状态，而是「key 不在 CAS 里」** —— 没有要维护的脏标记、不怕重启、缓存可共享。
  这比脏标记法强，因为**没有状态可以写坏**。

**③ 两遍走：先 key、后 cook。** 算 key **不需要求值**，只需要参数 + 上游 key（记忆化）⇒
改一个叶子参数 = O(深度) 次哈希，而不是全图重算；**上游命中是自动的**。

**④ 磁盘 CAS**：`target/pcg/<key 前两位>/<key>.pxart`（git 对象那样的分片）。
重启后仍然命中 ⇒ 第二次、第三次迭代几乎免费。

**⑤ 类型检查放在图上，不放在脚本里**（这是对约束 C 的回答）：

```rust
NodeSpec {
    id: "field.noise",
    inputs:  [("mask", Field2D)],
    outputs: [("field", Field2D)],
    params:  schema,      // 名字、类型、默认值、范围
    version: u32,         // 一改就换 key ⇒ 旧缓存自动失效
}
```

cook 之前做**图的类型检查**：边类型不匹配 / 缺参数 / 未知算子 / 有环 ⇒ 精确报错（带节点 id）。
**这套检查是完整且零成本的**，不需要脚本语言自带类型系统。

**⑥ 确定性**：单线程 cook 起步；将来并行必须保证「并行不改变结果」（纯算子 + 确定性归约）。
时间在 §10.3 已冻结，别引时间戳。

### 14.3 加一个新算子的三条路

| 路径 | 代价 | 类型检查 | 何时用 |
|---|---|---|---|
| 写进 `px_pcg`（普通 Rust） | **0.5 s** 重编 + 重跑 CLI | 满 | **默认** |
| cdylib 插件 + `libloading` | **0.25 s** 重编，不重启进程 | 满 | 渲染器开着的同时改算子 |
| WASM 沙箱节点 | 编 wasm 约 1–3 s + 一个运行时依赖 | 满 | 要开放仓库外第三方算子 |

⇒ **先做第一条**；第二条**留接口不做实现**；第三条不做。

### 14.4 分阶段

- **P2a**：算子注册表 + 图（`.pxg` 文本）+ 图类型检查 + 磁盘 CAS + cook + `px_pcg stats`
  （**纯 Rust、无脚本**；这一步就能把整套缓存语义验完）
- **P2b**：参数表达式（小 AST）
- **P2c**：把节点产物直接交给常驻渲染服务出预览图（§13 的服务就是为这个准备的）
- **P2d**：按需再谈 snippet / dylib / wasm

### 14.5 待裁决

1. **哈希用什么**：`px_protocol` 被依赖白名单门锁死（只能 serde / serde_json），但 `px_pcg` 不受限。
   候选：复用 FNV-1a 64（零依赖、但碰撞概率不适合当 CAS 键）vs `blake3`（快、无碰撞之忧、多一个依赖）。
2. **CAS 放哪**：`target/`（gitignored，合「可再生派生物不入库」）还是工程目录（可共享、可入库）？
3. **需不需要「仓库外的算子」**：需要 ⇒ wasm 那条要认真设计；不需要 ⇒ 前两条足够。
4. **CPU 场与 GPU 着色器的一致性**：另一条血缘已有 `fbm / perlin / ridged / warp / vnoise` 的 GLSL
   （§1）。CPU 烘出来的场要和它们对得上 ⇒ **要么 CPU 侧照抄同一套噪声定义，要么把 CPU 当权威、
   GPU 只做预览**（§3 的结论是「CPU bake 权威、GPU bake 只做缓存」）。这条会直接影响 `field.noise`
   的参数设计。

---

## 15. PCG 层（修订）：图就是一支依赖算子表的 Rust 程序（用户 2026-09-13 裁决）

用户原话：*「算子和图为啥不这样处理：每个算子在算子表 crate 里注册类型和 cdylib 位置，
然后一张图就是一个依赖算子表 crate 的 rust 程序？」*

**采纳。** 这个方案**比我 §14 那套（图=数据 + 解释器）更好**，理由不是口味而是"删代码"：

| §14 我提议的 | §15 用户提议的 |
|---|---|
| 自己写算子注册表（名字/类型/参数 schema） | **Rust 函数签名本身就是注册表** |
| 自己写图的类型检查器 | **rustc 就是类型检查器**（错边=编译错误） |
| 自己写表达式求值器 | **Rust 表达式就是表达式** |
| 解释器 + 调度器 | **生成的 `main` 就是调度器** |
| 约束 (C) 靠自建类型系统解决 | 约束 (C) **消失**（类型检查是白拿的） |

代价只有一个：**改图要过一次 cargo**。实测（§14.0）：生成程序只改自己 ≈ **0.25–0.5 s**，
首次冷编要带上依赖 ≈ 5–10 s。这个代价买下"类型检查免费 + 少写三个子系统"，值。

### 15.1 具体形状

```
px_ops/                     算子表 crate（普通 Rust 依赖，静态链）
  noise.rs  field.rs  mesh.rs  ...
  每个算子 = 一个有类型的纯函数 + const VERSION: u32
  cook(key, || op(params, inputs))  ← 唯一和缓存打交道的地方

px_graphs/                  workspace 成员，**唯一**一个成员，里面全是生成出来的 bin
  src/bin/<图名>.rs         由 .pxg 生成（派生物，gitignored）

art/<图名>.pxg              作者手写的数据：节点、边、参数、可选的 Rust 片段
```

- **加一张图 = 多一个文件，不动 `Cargo.toml`**（`src/bin/*.rs` 是 cargo 自动发现的）
  ⇒ 不会重演上次「`cargo init` 污染根 manifest」那种事，也不需要每张图一个 workspace 成员。
- 所有生成的 bin **共享 workspace 的 target 目录**（cargo 的增量缓存全部复用）。
- 生成文件在 `src/bin/` 下 ⇒ 编译产物、断言、`cargo run -p px_graphs --bin <名>` 全都是现成的。

### 15.2 cdylib 那半截：先不做，但要留对位置

用户方案里"注册 cdylib 位置"这一半，**在当前约束下是多余的**：既然图程序是 Rust 程序，
算子直接**静态依赖** `px_ops` 就拿到了完整类型检查；换成 cdylib 反而把类型检查换成 ABI 检查、
还多出加载期失败模式。而"改算子不想重编图程序"这个动机，被 0.25 s 的 cdylib 增量编译实测
和 0.5 s 的普通重编一起消解了。

⇒ **裁决：算子默认静态链进 `px_ops`。** cdylib 只在出现下面任一需求时才启用：
① 要在**不重启**图程序的前提下换算子实现（热插拔）；② 算子来自**仓库外**（第三方）。
两条都不是现在的需求。真启用时，`px_ops` 的**函数签名不变**，只是实现从静态改为按路径加载
—— 这也是为什么现在就要把算子写成**独立、无状态、只依赖参数与输入**的纯函数。

⚠️ **2026-09-13 用户追问：「算子不是 cdylib 吗，怎么成了 exe 了」** ⇒ 说明上面这条裁决
（是我提的、用户当时没明确表态）没有传达到位，**待用户确认**。两条路各自的后果：

| | 静态（上表左） | cdylib（上表右） |
|---|---|---|
| 算子在哪 | 编进**图程序的 exe** | 独立的 `.dll`，运行时按路径加载 |
| `engine_hash` 要覆盖 | **exe 一个文件** | **exe + 所有被加载的 dll**（§17.1 的盲区） |
| 类型检查 | rustc 全程 | 跨 dll 边界退化成 **ABI 检查**（错误推迟到运行期） |
| 改一个算子 | 重编图程序 ~0.5 s | 只重编 dll ~0.25 s，**图程序可不重启** |
| 谁来调用算子 | 只有图程序 | 只有图程序（跨进程流动的是产物，不是算子） |

**关键事实：算子只在一个进程内被调用，跨进程流动的是产物（`ArtBundle`）。**
所以"共享算子"这个需求不存在，cdylib 唯一买到的是"不重启图程序"，
而图程序本来就是一次 CLI 运行（~0.5 s）—— 这也是我建议砍掉它的全部理由。

### 15.3 缓存怎么活下去（§14.2 全部保留，只有一处要改）

`node_key = blake3( op_id ‖ op_version ‖ 求值后的参数 ‖ [各输入的 key] )`（用户已裁决用 blake3）

- **key 必须由 `.pxg` 的内容算出来，不能由生成出来的 Rust 源码算**。否则：生成器一升级、
  路径一变，整个缓存全废。**`.pxg` 是权威输入，生成代码是派生物** —— 这条是整个设计的承重墙。
- 「脏」= key 不在 CAS 里；两遍走（先 key 后 cook）；CAS 分片存（`target/pcg/ab/<key>.pxart`）。
- 生成代码里 `cook` 的调用点就是缓存边界 ⇒ **改下游节点不会重算上游的贵烘焙**。

### 15.4 白拿的那个好处：类型化的"脚本"

图程序既然是 Rust 程序，**"VEX 式逐元素代码"这个问题就消失了** —— 直接在 `.pxg` 里放一个
`snippet` 节点，body 就是 Rust 代码（仍然存在 `.pxg` 文本里 ⇒ 仍然参与哈希、仍然可 diff、
仍然是数据）。约束 (C)「脚本类型检查弱」至此彻底不存在：**片段是被 rustc 检查的 Rust**。

⚠️ 但设一条红线：**片段必须是纯函数**（不碰 IO、不碰时间、不碰全局），否则缓存与确定性会崩。
这条要靠 codegen 只允许闭包体、以及算子 API 不暴露副作用来强制。

### 15.5 编译错误要能指回节点

生成代码里变量名用节点 id（`let n_ridge_7 = ...`），rustc 的报错就能直接读出是哪个节点错了。
这是"图=程序"方案唯一的体验短板（错误信息变成 rustc 的话），变量命名是便宜的补偿。

### 15.6 CPU / GPU（用户裁决：按需）

*「需要实时变化噪声参数的 GPU，涉及复杂计算的烘焙」* ⇒ **不搞全局统一，按算子声明后端**：

- 一张图里**同一个概念可以有两个实现**（`field.noise.cpu` / `field.noise.gpu`）；
- **只有两边都被用到时才需要一致性检查**；否则明确标注哪边权威；
- 烘焙路径（贵、可缓存）与实时路径（参数要能拖）分开，正好对上 §3「CPU bake 权威、GPU bake 只做缓存」。

### 15.7 修订后的分阶段

- **P2a**：`px_ops`（3–5 个真算子：常数场、噪声场、混合、重映射）+ `.pxg` 解析 + codegen +
  `px_graphs` + `cook`/CAS（blake3）+ 每节点 hit/miss/耗时打印
- **P2b**：`snippet` 节点（类型化片段）
- **P2c**：节点产物 → 常驻渲染服务出预览图（§13）
- **P2d**：算子后端选择（CPU/GPU）与一致性门

### 15.8 待裁决（更新）

1. ~~哈希~~ ⇒ **blake3**（已裁决）。
2. ~~算子来源~~ ⇒ **默认仓库内静态链**，cdylib 留接口不做（§15.2）。
3. ~~CPU/GPU~~ ⇒ **按算子声明后端**（§15.6）。
4. ⏳ **CAS 放哪**：`target/pcg/`（gitignored，合「可再生派生物不入库」）还是工程目录（可共享）？
5. ⏳ **codegen 由谁触发**：`px_pcg build <图>` 一条命令（内部 gen + cargo）还是一个 `scripts/pxg.ps1`
   门脚本（合本仓 `check-*` 脚本惯例）？我倾向前者做成库、后者做成入口。

---

## 16. PCG 层（再修订）：节点与边在 Rust 里，`.pxp` 只给参数（用户 2026-09-13 再次裁决）

用户原话：*「节点和边在 rust graph 里就已经确定了，.pxg 只能提供 graph 的参数」*。

**采纳。这一刀把 §15 里的 codegen 也删掉了**，设计又少一个子系统：

| §15 的形状 | §16 的形状 |
|---|---|
| `.pxg` 描述拓扑 → codegen 出 Rust → 编译 → 跑 | **拓扑就是手写 Rust**，没有生成代码 |
| 「key 必须从 `.pxg` 算、不能从生成代码算」是承重墙 | **没有生成代码 ⇒ 这条约束自动消失** |
| codegen 要保证纯函数、要能指回节点 id | **节点名就是作者自己的变量名**，rustc 报错天然可读 |
| 参数校验要自己写（schema） | **`serde` 就是校验器**（`deny_unknown_fields` 连拼写错误都能抓） |

### 16.1 为什么"参数走数据、拓扑走代码"是对的

**因为美术迭代的热路径是参数，不是拓扑。** 一张图的结构一旦成形，值会被改上千次，而结构改动很少。
把热路径放进数据文件、冷路径放进代码，正好各得其所：

| 改动 | 代价 |
|---|---|
| 改参数（`.pxp`） | **0**（不重编，只重跑 cook） |
| 改拓扑 / 加节点（Rust） | ~0.5 s 重编 |
| 加算子（`px_ops`） | ~0.5 s 重编 |

### 16.2 形状

```
px_ops/                   算子库（稳定）：有类型的纯函数 + const VERSION + cook/CAS
px_graphs/                workspace 成员：src/bin/<图名>.rs（**手写、入库**，不是生成的）
art/<图名>.pxp            只有参数（数据、入库、改它不重编）
```

- 加一张图 = **多一个文件，不动 `Cargo.toml`**（`src/bin/*.rs` cargo 自动发现）。
- `px_ops` 与 `px_graphs` 分开：算子稳定、图多变，改图不重编算子库。
- 命名建议：`.pxg` → **`.pxp`**（parameters），因为图已经不在里面了，叫 graph 会误导。

### 16.3 缓存键：结构变化自动传播，唯一的新风险是 `op_version`

```
node_key = blake3( op_id ‖ op_version ‖ 参数值 ‖ [各输入的 key] )
```

**拓扑不用显式版本号**：改一条边 ⇒ 下游节点的**输入 key 变了** ⇒ 它的 key 变了 ⇒ 自动重算；
换个算子 ⇒ `op_id` 变了。**结构变化沿着 key 链自己传播**，这正是内容寻址的免费红利。

⚠️ **唯一会咬人的地方**：你改了**算子的代码**却忘了 bump `VERSION` ⇒ 旧缓存被错误命中。
三种应对，按推荐度：

1. **`op_version` 常量 + 一条门脚本**（`check-pcg-versions`）：把每个算子文件的哈希记在一个入库的小
   JSON 里，脚本比对"文件变了但版本没变"就报错。**合本仓"门是一等公民"的惯例**。
2. `build.rs` 把 `px_ops` 全部源码哈希进二进制 ⇒ **任何算子改动都让整个缓存失效**：安全但粗，
   写算子时会反复全量重烘（不过写算子本来就少，且参数迭代不受影响）。
3. 加一个 `PX_PCG_FRESH=1` 逃生门（无论如何都该有）。

### 16.4 结构灵活性从哪来：**参数驱动的循环**

这是"拓扑写死"最容易引起的担心（"那我想改层数怎么办"），答案是：
**让结构变化成为参数的函数，而不是新的拓扑。**

```rust
for layer in 0..params.layers {          // layers 是 .pxp 里的一个整数
    let ridged = field::noise(&current, params.layers_params(layer));
    current = field::mix(&current, &ridged, params.layer_weight(layer));
}
```

⇒ `.pxp` 里改一个整数就能改"拓扑"，**零重编**。真正需要改 Rust 的是**出现了代码里没有的新算子组合**，
那是结构设计，本来就该付一次 0.5 s。

### 16.5 免费拿到的东西：子图去重

key 是内容寻址的 ⇒ 两个节点只要 `op_id + 参数 + 输入` 相同就**共享同一份产物**，
公共子表达式自动只算一次。不需要 CSE 优化器，是这套键设计的副产品。

### 16.6 放弃了什么（要认）

- **图不再是可读数据**：没有图编辑器、没有"列出所有节点"、没有 DAG 可视化 ——
  工具只能 grep Rust。对 agent-first 的工作流可以接受（agent 本来就读代码），
  但**"美术自己拖节点"这件事从此不在路线图上**。
- 结构的复现性 = 二进制 + 参数 + 输入，所以产物里要记 **git rev + 协议指纹**（复用 P0 的 `ProtocolId`）。

### 16.7 修订后的分阶段

- **P2a**：`px_ops`（常数场 / 噪声场 / 混合 / 重映射 4 个真算子 + `cook`/blake3 CAS）
  + `px_graphs` 里 **一张手写的图** + `.pxp` 参数文件 + 每节点 hit/miss/耗时打印
- **P2b**：参数驱动的循环（§16.4）与 `op_version` 门（§16.3 的第 1 条）
- **P2c**：节点产物 → 常驻渲染服务出预览图（§13）
- **P2d**：算子后端声明（CPU/GPU，§15.6）

### 16.8 待裁决（再更新）

1. ~~哈希~~ ⇒ **blake3**（已裁决）。
2. ~~算子来源~~ ⇒ **仓库内静态链**，cdylib 留接口（§15.2）。
3. ~~CPU/GPU~~ ⇒ **按算子声明后端**（§15.6）。
4. ~~codegen 谁触发~~ ⇒ **没有 codegen 了**（§16）。
5. ⏳ **CAS 放哪**：`target/pcg/`（gitignored）还是工程目录（可共享）？
6. ⏳ **`.pxp` 的格式与粒度**：一个图一个文件（RON / TOML）还是"一个节点一个文件"（合 §1 的
   "one node/file granularity"惯例）？格式我倾向 **RON**（与 Rust 类型一一对应、注释友好）。
7. ⏳ **`op_version` 纪律怎么强制**：门脚本（推荐）/ build.rs 全量哈希 / 只留逃生门。
8. ⏳ **别名**：`.pxg` 是否改名为 `.pxp`（图已不在文件里）。

---

## 17. PCG 层（定稿）：缓存键含引擎指纹，参数一节点一文件

用户裁决：*「CAS 放 `target/pcg/`」* / *「一节点一文件」* /
*「缓存的哈希包含二进制的信息，二进制变了缓存自然失效」*。

**三条都采纳。第三条把 `op_version` 纪律和门脚本一起删掉了** —— 能用机制保证的，就不靠自觉。

### 17.1 键 = 引擎指纹 + 算子 + 参数 + 输入键

```
node_key = blake3( engine_hash ‖ op_id ‖ 规范化后的参数值 ‖ [各输入的 key] )
```

**`engine_hash` = 正在跑的那支二进制**（用户原话：「缓存的哈希包含二进制的信息」）。
这句的准确含义要写清楚，因为它决定实现方式：**哈希的对象是整支二进制，不是算子库**。
于是图程序里写死的常量（`field::mix(&a, &b, 0.35)` 里的 `0.35`）**本来就是键的一部分** ——
它确实是一种输入，只不过是**从代码进来的输入**，不是从数据进来的。
（我一度按"只哈希 `px_ops`"理解，那才会漏掉它；按二进制口径没有这个洞。）

**怎么算这个哈希 —— 两选一：**

| 做法 | 覆盖面 | 稳定性 |
|---|---|---|
| A. `build.rs` 编译期哈希**源码** | 得自己枚举全（源码 + `Cargo.lock` + rustc 版本 + 构建参数），**漏一项就漏一类输入** | 无事重编不失效 |
| B. **运行时哈希正在跑的 exe 文件**（推荐） | **凡是影响执行的都覆盖**，不需要枚举、不需要纪律 | 无事重编时 cargo 根本不重新链接 ⇒ exe 字节不变 ⇒ 也不失效 |

选 B 的理由是**失败模式**：B 最坏是"多失效一次"（可预期，只是慢一点），
A 最坏是"漏掉某类输入"（不可预期，画面悄悄不对）。与本仓"用可预期换不可预期"的口径一致。
代价：启动读一次 exe（150 MB 量级约 0.1–0.3 s），可按 (路径, 大小, mtime) 记忆化。

⚠️ B 的已知盲区：**若将来启用 cdylib（§15.2），dll 不在 exe 里**，那时要把 dll 文件一并纳入哈希。

- **不需要 `op_version`**，**不需要门脚本**：没有需要人维护的东西。
- 参数迭代不动二进制 ⇒ `engine_hash` 不变 ⇒ **缓存照常命中**。
  ⇒ 恰好是我们要的语义：**改参数不失效、改代码才失效。**

**代价（要认）**：任何一次会改变二进制的改动都让整个缓存失效，写 `px_ops` 时会全量重烘。
但改代码是你主动做的、你知道自己在改代码；"旧缓存被错误命中 ⇒ 画面悄悄不对"是你不知道的。
（若将来重烘真的疼，细化路径是每算子 `include_str!(file!())` 自哈希——精确到文件、仍然自动；
弱点是抓不到公共模块与常量的改动，所以**现在不做**。）

### 17.2 参数：一节点一文件

```
art/<图名>/<节点名>.toml        ← 这就是节点 id 的来源（文件名词干）
```

- 节点 id = **文件名词干** ⇒ 与 §1 那条「one node/file granularity」的仓库惯例天然一致。
- 图程序里按键取参数：`let p = px_ops::params::<MixParams>("mix")?;`（文件缺失 ⇒ 用默认值，DX 好）。
- **每个节点的参数独立成文件** ⇒ agent 改一个节点的参数就只动一个小文件，diff 干净、可并行改。
- **参数哈希用的是"规范化后的值"，不是文件原文** ⇒ 改注释、调格式**不会**让缓存失效。
  做法与 §P0 的快照一致（serde → 键序固定的 JSON → 哈希），仓库里已经有这个模式。
- **节点名不进 key**（只进清单）⇒ 两个节点 `op_id + 参数 + 输入` 相同就共享产物（§16.5 的免费 CSE）。

### 17.3 清单（给 agent 看的）

每次 cook 写 `target/pcg/manifest.json`：节点名 → key、命中/未命中、耗时、产物大小。
**这是"图上发生了什么"的唯一可读出口**（图不再是数据，§16.6），也是 agent 判断
"我这次改动到底重算了哪几个节点"的依据。

### 17.4 分阶段（定稿）

- **P2a**：`px_ops`（4 个真算子 + blake3 CAS）+ `build.rs` 算 `engine_hash` +
  `px_graphs/src/bin/<图>.rs`（手写）+ `art/<图>/<节点>.toml` + manifest 打印
- **P2b**：参数驱动的循环（§16.4）
- **P2c**：节点产物 → 常驻渲染服务出预览图（§13）
- **P2d**：算子后端声明（CPU / GPU，§15.6）

### 17.5 待裁决

1. ⏳ **`.pxp` 格式**：一节点一文件的情况下我倾向 **TOML**（扁平键值最自然、注释友好、
   `toml` 这个依赖在仓库里已经出现过），**除非**参数里有枚举/变体（那 RON 更顺）。
   这条取决于第一批真算子的参数长什么样 —— 可以放到 P2a 里用真参数再定。
2. ⏳ **命名**：`.pxg` 不再承载图，是否改名（`.pxp` / 直接 `.toml`）。

---

## 18. 「静态链接那这不就是一个普通的 Rust 项目吗」

用户 2026-09-13 追问。**是 —— 而且这是设计目标，不是副作用**（§15 那张"删掉三个子系统"的表就是这件事）。
但"普通 Rust 项目"之外还剩三样东西，它们才是这一层真正的产出：

| 剩下什么 | 为什么不能省 |
|---|---|
| **`cook` 边界 + 内容寻址 CAS** | 程序每次从头跑到尾，**没有缓存就是每次全量重烘**。参数迭代（改一个数、看一眼）全靠它把"重算"压成"命中" |
| **`.toml` 参数文件** | 这是"改参数不重编"的唯一实现方式 |
| **产物格式 = `px_protocol::ArtBundle`** | 这是与渲染器之间唯一的接口 |

**代价（要认）**：因为图不是数据，下面这些能力**没有了** ——
运行期查看/编辑 DAG、任意节点的 bypass / lock、美术自己搭网络、把图交给不能编译 Rust 的人。
⇒ **这一层的天花板是「谁来写图」**：现在是 agent 写在代码里；将来要工具或人来写，才需要真正的图引擎。

**折中（推荐）：拓扑隐式、节点边界显式。** 每个算子调用都过一层

```rust
let ridge = px_ops::node!("ridge", field::fbm, &[&base]);
```

宏负责：① 按节点名读 `art/<图>/ridge.toml` ② 算 key ③ 命中就取、未命中就 cook ④ 登记进 manifest。
**框架语法就这一个宏，其余全是普通 Rust。** 它顺带给了**运行期可见的节点清单**，
所以将来真要接图引擎，算子不用重写 —— 它们已经是"有 id、有类型化参数、无状态的纯函数"。

**一句诚实的补充**：P2a 的玩具图只有几个噪声算子，**此刻缓存买不到速度**（全量重算也就几毫秒）。
现在建它，是因为**边界**（`cook` + 参数文件 + 产物格式）是架构；
等要烘的东西真变贵（高分辨率场、网格、侵蚀模拟）时，它是唯一能救迭代速度的东西。

---

## 19. 缓存失效：手动版本号（用户 2026-09-13 裁决，§17.1 的二进制哈希方案作废）

用户原话：*「那还是规定每个算子改动必须手动在源码里升版本吧」*。

```
node_key = blake3( op_id ‖ op_version ‖ 规范化参数值 ‖ [各输入的 key] ‖ graph_version )
```

**为什么这条比"哈希二进制"好**（选它的正当理由，记下来免得将来有人又想改回去）：

- **失效范围更细**：改算子 A 不该让用算子 B 的节点重烘；二进制哈希是"一动全废"。
- **经得起重构**：改注释、换文件名、纯重构不影响语义 ⇒ 本来就不该失效。
- **语义由人声明**：版本号说的是"这个算子的**输出语义**变了"，不是"这个文件被碰过"。

**代价（必须配兜底，否则等于把静默错误请回来）**：忘了升版本 ⇒ 旧缓存被命中 ⇒
**画面悄悄不对，而且没有任何报错**。这正是本仓被咬过多次的那类失败。

### 19.1 兜底：源码变而版本没变，就在命中时喊出来

每个算子声明三样：

```rust
pub const ID: &str = "field.fbm";
pub const VERSION: u32 = 3;
pub const SOURCE_HASH: u64 = fnv1a(include_str!("fbm.rs"));   // 编译期算好，自动跟着文件走
```

⚠️ **`SOURCE_HASH` 不进键**（进键就等于自动失效，手动版本号就没意义了）——
它只进**产物元数据**，用于命中时比对。命中时若版本一致而 `SOURCE_HASH` 不一致：

```
⚠ field.fbm 的源码变了但 VERSION 仍是 3；若输出语义变了请升版本并 PX_PCG_FRESH=1 重烘
```

⇒ **不需要 CI、不需要锁文件、不需要提交时的纪律**；告警恰好出现在"即将发生陈旧命中"的那一刻。
确认是纯注释改动就忽略；`PX_PCG_FRESH=1` 永远保留作逃生门。
（`fnv1a` 写成 `const fn` 即可在编译期算；用 FNV 而不是 blake3 是因为它只做变更检测、不做安全。）

### 19.2 图程序自己也要版本号

不再哈希二进制 ⇒ 图程序里写死的常量（`field::mix(&a, &b, 0.35)` 的 `0.35`）
**重新变成缓存盲区**。所以：**`const GRAPH_VERSION: u32 = 7;` 写在图程序源码里，进键，同样受告警保护。**

⇒ 规则收敛成一句：**每个算子一个版本号、每张图一个版本号；源码变了而版本没变，会被喊出来。**

### 19.3 待确认

- ⏳ **告警要不要升级成门**：把"源码变了但版本没升"做成 `check-pcg-versions` 失败？
  我倾向**先只告警** —— 门在纯重构时会误报，而误报的门最后会被人绕过去。

---

## 20. P2a 完成记录：PCG 层落地

**做了什么**（`cargo test --workspace` 133 个全绿）：

| 件 | 内容 |
|---|---|
| `px_ops` | `FieldOp` trait / `Field` / 值噪声 + fbm + ridged / `node::<Op>(名字, 输入)` / blake3 CAS / `index.json` + `manifest.json` / 版本不匹配告警 |
| 算子 5 个（**一算子一文件**，各带自己的 ID / VERSION / SOURCE_HASH） | `field.constant`、`field.fbm`、`field.ridged`、`field.mix`（3 输入 a/b/mask）、`field.remap` |
| `px_graphs` | `src/bin/planet.rs`：手写的一张图（continents / mountains / weight → terrain → height），带 `GRAPH_VERSION` |
| 参数 | `art/planet/<节点>.toml`，**一节点一文件**；文件缺失即用默认值 |
| 缓存 | `target/pcg/ab/<前两位>/<blake3>.pxart`（**内容就是 `px_protocol` 的流格式**）+ `index.json` + `manifest.json` |

用法：`cargo run -p px_graphs --bin planet`

### 20.1 验收结果（六条全过）

| 验收 | 结果 |
|---|---|
| 改一个参数（mountains.frequency） | **命中 continents / weight，重算 mountains → terrain → height**（2 命中 3 重算） |
| 只改注释 | **全部命中，键不变** |
| `PX_PCG_FRESH=1` | 全部重算，且**重算出的值域与缓存里的完全一致** |
| 改算子源码但没升 VERSION | **⚠ 精确点名 continents，缓存照常命中** |
| 升 VERSION | 只有该算子与其下游重算（continents + terrain + height） |
| 参数改回去后再全量重算 | **11 个产物逐字节不变** |

最后一条是关键：**键是纯函数**。把参数改回去，键就回到原值、缓存自动命中 ——
**不需要任何"撤销/失效"逻辑**，这是内容寻址白送的。

### 20.2 实测数字

| | 时间 |
|---|---|
| 5 个节点全量 cook（384×192，6 阶 fbm + 5 阶 ridged） | **52 ms** |
| 5 个节点全命中 | **10 ms** |
| 改一行 `px_ops` 重编（不链 Bevy） | **0.5 s 量级** |

⚠️ 如实记一笔：**这个规模下缓存买不到速度**（全量也才 52 ms）。它现在买的是**边界** ——
`cook` 的接口、参数文件的约定、产物格式；等要烘的东西变贵（高分辨率场、网格、侵蚀）时才回本。

### 20.3 一个设计判断被实测支持

`node::<Op>(名字, 输入)` 是**泛型函数、不是宏**：算子把
`ID / VERSION / SOURCE_HASH / INPUTS / eval` 放在一个 `impl FieldOp for X` 里，
所以"算子表"就是 trait 实现，编译期解析。**框架语法为零** —— 图程序读起来就是普通 Rust，
这验证了 §18 的判断。

### 20.4 还没做

- **产物还没有消费者**：`px_render` 只读 `World` 帧，还不会读 `.pxart`（P2c）。
- **只有 `Field` 一种产物类型**（Mesh / Instances 只在协议里有类型声明）。
- 图程序与 sim 还没接（拿 `WorldView` 当输入是 P2c 的事）。
- 输入个数是常量（`INPUTS` 是 `&'static [&'static str]`）；可变输入要等真有需求。
- 噪声是**自己写的值噪声**，与另一条血缘的 GLSL（perlin / fbm / ridged / warp）**还没对过** ——
  §14.5 第 4 条那个 CPU/GPU 一致性问题，现在正式变成一个待办。

---

## 21. P2c 完成记录：程序化星球（PCG → 渲染器这条通路通了）

**通路**：

```
art/planet/<节点>.toml          改这里（数据，不重编）
   ↓  cargo run -p px_graphs --bin planet          命中时 10 ms、全算 52 ms
target/pcg/ab/<xx>/<blake3>.pxart                 高度场（px_protocol 流格式）
   ↓  px_render --planet <该文件> --palette rocky --out shot.png     0.34 s
PNG（960×640，星空背景 + 位移球体 + 明暗界线）
```

**关键点：渲染器不依赖 `px_ops`。** 它用 `px_protocol` 自己解 `.pxart` ——
这是 §14.2「缓存里存的就是渲染器要吃的东西」的兑现，两个 crate 之间零转换层。

**用法**：

```bash
cargo run -p px_graphs --bin planet          # 烘高度场
target/debug/px_render --serve               # 常驻服务（一次预热，之后每张图 ~0.34 s）
target/debug/px_render --planet target/pcg/ab/xx/yy.pxart --palette rocky --out target/planet.png
```

四个色板：`rocky`（海洋 + 陆地渐变 + 沙滩 + 雪线）/ `gas`（条带）/ `ice`（冰海 + 裂纹）/
`lava`（暗岩 + 自发光裂缝）。`--displace / --sea / --radius / --spin` 可覆盖各色板的默认值。

**全链路（改一个参数 → 烘图 → 出图）< 1 s。**

### 21.1 这一段踩的三个坑（都属于「看着像那么回事、其实是错的」）

**① Bevy 的 UV 球极轴在 ±Z，不在 ±Y。**
`Sphere::mesh().uv()` 里 `z = radius * sin(stack_angle)`，v=0 就是 +Z 极点。相机在 +Z 上
⇒ **正对北极**：一整片极地冰盖 + 放射状条纹汇聚在正中 + 中心一个黑点（极点奇异点）。
我第一反应是"贴图错了"，看图才认出那是极点。修法：`Quat::from_rotation_x(-FRAC_PI_2)` 立起极轴。

**② `emissive_texture: None` + 非零 `emissive` = 整个物体均匀发白光（最阴的一个）。**
我给所有色板都设了 `emissive: rgb(3,3,3)`，只有熔岩配了自发光贴图。Bevy 把**缺失的自发光贴图
当白色**，于是三个星球变成"均匀自发光"：**与光照完全无关**，既没有明暗界线、调光照也没反应
—— 我把 illuminance 从 14000 降到 3800，画面**纹丝不动**，这就是证据。修法：无贴图时 `emissive` 必须为 0。

**③ 我自己写的旋钮没接上。** `--sea` 只被**位移**用了，着色里写死了陆地色带的阈值
⇒ 改海平面不影响陆海比例。**命名与实际不符**是最难发现的一类 bug，因为它"看起来在工作"。
修法：着色按海平面分成水体/陆地两段，并让海面变平（`height < sea` 时取 `sea`），海岸线才干净。

### 21.2 诊断方法论（延续 §12.2）

这一轮靠的是**把中间产物直接量出来**，不是盯着图猜：

- 白球卡了三轮后，我在贴图生成处加了一行统计（平均 RGB / 近白像素占比），
  一眼得到"贴图不是白的（平均 (124,153,159)、近白 9.3%）" ⇒ **排除贴图**；
- 对照：经济世界场景在 9000 lux 下颜色正常 ⇒ **排除曝光**；
- 两条合起来只剩自发光。

⇒ **在图上猜三次，不如打一行统计。** 这条应该成为美术通路的常规做法：
每个会"看起来不对"的中间产物，都该有一行可打印的统计。

---

## 22. P2c 续：把「各种星球」做宽

| 新增 | 内容 |
|---|---|
| **`field.warp`（新算子）** | 域扭曲：用第二个场偏移采样坐标（双线性 + 经度环绕），把笔直的 ridged 脊线变成**蛇曲峡谷** |
| **第二个图** `px_graphs/src/bin/desert.rs` | 台地 `fbm` + 峡谷 `ridged` → `warp` 扭曲 → `mix` 切入 → `remap`；参数在 `art/desert/*.toml` |
| **环形系统** | 程序化环带（径向条带 + 卡西尼缝 + 透明度），与星球一起倾斜 19°；深度测试正确（环在星球前挡住、在后被遮） |
| **`desert` 色板** | 沙丘 / 盆地两段 + 层理条纹 |
| **色板移出协议** | `Scene::Planet.palette` 由枚举改成**字符串** —— 色板是美术内容、不是 wire schema，**以后加色板不再动协议**（这条省下的是每次加色板都要升 SCHEMA + 重录流） |

**加一张图仍然不用改 `Cargo.toml`**（`src/bin/*.rs` 自动发现）—— §16.2 那条设计第二次兑现。

**五种可区分的星球**：`rocky`（海洋 / 陆地 / 雪线）、`gas`（条带 + 风暴斑 + 环）、
`ice`（冰海 + 裂纹）、`lava`（暗岩 + 自发光裂缝）、`desert`（峡谷 + 沙丘）。

### 22.1 热路径实测

| | 时间 |
|---|---|
| 直接跑烘图二进制、全部命中 | **0.06 s** |
| 出图（服务已热） | **0.34 s** |
| **整轮：改一个参数 → 拿到一张图** | **0.49 s** |
| 同样两步但走 `cargo run` | 1.67 s（其中 **1.2 s 是 cargo 自己的启动开销**） |

⇒ goal 那条「参数迭代走缓存热路径、单张图 < 1 s」**达标**。
⚠️ 但注意：**走 `cargo run` 会白花 1.2 s**，迭代时应直接跑 `target/debug/planet.exe`。

### 22.2 已知瑕疵（没修，记下来免得当成已完成）

- **UV 球的极点收缩**：倾斜之后能看到北极，那里纹理向一点汇聚出放射状条纹。
  治本是换 icosphere + 三平面映射，或者干脆不让极点入镜。
- 环**没有投在星球上的阴影**，也**没有大气边缘光** —— 加上会更像照片。
- 沙漠色板偏灰，沙与峡谷的对比还不够"沙漠"。
- 噪声仍是自写的值噪声，与另一条血缘的 GLSL 没对过（§14.5 第 4 条）。

---

## 23. 调研：球的 UV / 极点问题该怎么处理

### 23.1 先把现象拆成两半

极点上那团"放射状条纹"是**两个不同的问题叠在一起**：

| | 是什么 | 换网格能不能治 |
|---|---|---|
| **① 几何退化** | UV 球的极点是一圈退化三角形（每列两个），法线与 UV 都塌到一点 | **能**。icosphere / quad sphere 没有极点顶点 |
| **② 域不匹配** | equirect 贴图把**一整行纹素压到一个点**上：极区方位角转一点点，采样的 x 就跑过整行 ⇒ 极点周围是一圈"不同高度/颜色的楔形"，有位移就变成**放射状山脊** | **不能**。只要域还是 equirect，换什么网格都还在 |

### 23.2 实测对照（同一份高度场、同一台相机）

| 网格 | 气态（平滑无位移） | 岩石（有位移） |
|---|---|---|
| UV 球 `uv(224,112)`（原本） | 明显的三角扇汇聚 + 中心黑点 | 放射状条纹 |
| **icosphere `ico(49)`** | **扇没了，条带平滑地翻过极点** | **仍有放射状山脊**（几何扇消失） |

⇒ **结论：换网格只解决了一半。** 气态行星受益明显（它没有位移，退化三角形是主要噪声源）；
岩石行星的放射状山脊是 ② 造成的，换网格治不了。对照图：`target/ico-gas.png`、`target/ico-rocky.png`。

**顺带两个坑**：
- Bevy 的 `SphereKind::Ico` 极轴在 **±Y**（`inclination = acos(point.y)`），而 UV 球在 **±Z**。
  换网格时必须把 `Quat::from_rotation_x(-FRAC_PI_2)` 一起删掉，否则极点会转到侧面去。
- Bevy 的 icosphere **是带 UV 的**，但用的是同一套 equirect 公式
  （`[0.5 - atan2(z,x)/TAU, acos(y)/PI]`）⇒ **它不解决 ②**，只解决 ①。

### 23.3 外部做法（调研结果）

| 做法 | 出处 | 对我们的意义 |
|---|---|---|
| **Quad sphere / cube sphere**：立方体六面细分后投影到球，**每面各自一套方正 UV** | [Catlike Coding: Cube Sphere](https://catlikecoding.com/unity/tutorials/procedural-meshes/cube-sphere)、[UE5 程序化星球：cube-face 参数化](https://www.youtube.com/watch?v=JjRsfRSbr5w&vl=en) | 极点、接缝、密度不均**一次全解**；游戏工业的标准答案 |
| **Icosphere + 自己算 UV** | [Bevy issue #4987](https://github.com/bevyengine/bevy/issues/4987) —— "There is no trivial way to uv map an icosphere" | 只解决几何退化，不解决域 |
| **Triplanar / UV-free texturing**：按世界坐标做三向投影、按法线混合 | [Ben Golus: Normal Mapping for a Triplanar Shader](https://bgolus.medium.com/normal-mapping-for-a-triplanar-shader-10bf39dca05a) | 适合**细节层**，不适合"一整张大陆高度图"；代价是三份采样 |
| **Cube map / 环境贴图**：按方向采样，不经网格 UV | [Blender 论坛：spherical texture mapping](https://blenderartists.org/t/how-to-achieve-this-spherical-texture-mapping/693359) | "不以网格 UV 为依据就没有极点问题" —— 与 ② 的诊断一致 |
| **球面直接用 3D 噪声**：在球面点上求值，不烘 2D 图 | [libnoise: Creating spherical planetary terrain](https://libnoise.sourceforge.net/tutorials/tutorial8.html) | **最彻底**：无极点、无接缝、密度天然均匀；这才是"程序化星球"该有的形态 |
| equirect 在极区有大量数据冗余 | [PanoTools: Equirectangular Projection](https://hugin.sourceforge.io/docs/manual/Equirectangular_Projection.html) | 解释了 ② 的本质 |

### 23.4 五条可选路线（按代价排序）

| | 做法 | 代价 | 治什么 |
|---|---|---|---|
| **A** | **只构图**：不让极点入镜（相机 / 自转固定） | 0 | 眼不见为净 |
| **B** | **极点缓释**：烘色带时把顶/底若干行朝该行均值混合；位移在极区衰减 | ~30 行，全在渲染器 | ② 的可见部分（**但等于丢掉 PCG 在极区的内容，是撒谎**） |
| **C** | **三平面细节层**：基底仍 equirect，叠一层按位置采样的高频细节 | ~40 行 | 用细节盖住极点糊掉的地方 |
| **D** | **立方体域**：PCG 直接烘 6 个面（或一张 atlas），渲染器用 quad sphere 按面采样 | ~250 行，**要动 PCG 接口** | ①② **全治**：无极点、无接缝、密度均匀 |
| **E** | **球面 3D 噪声**：算子按方向直接求 3D 噪声，根本不烘 2D 域 | 更大，等于改 PCG 产物形态 | 最彻底，真·程序化星球的做法 |

**D 需要动 PCG 接口**（`Field` 要带"域"的概念、算子要能问"这个像素对应哪个方向"），
所以这是一个**架构决定**，不该我替用户定。

### 23.5 建议

1. **立刻**：保留 icosphere（白拿的一半）+ 走 A（构图），把 D 排进路线图。
2. 若近期就要"俯视极点"的镜头，再加 B 当权宜（并明确标注它丢掉极区内容）。
3. D 一旦做，顺手给噪声算子加 `space = "3d"`，就往 E 靠 —— 那时"星球"才真正是**球面定义**的，
   而不是"把一张图贴到球上"。

**本轮已落地**：网格从 `uv(224,112)` 换成 `ico(49)`（顶点数相当，约 2.5 万），
并相应去掉 `Rx(-90°)`（极轴变了）。

### 23.6 用户追问「有没有球面坐标 / 极坐标噪声」——有，但要分清两种

**伪的那种**：把极坐标 `(θ, φ)` 喂给 2D 噪声。这只是 equirect 域换了个参数名 ——
极点奇异点原封不动（`φ` 在极点没有定义，极区一整行纹素仍被压到一点）。**我们之前就是这个。**

**真的那种**：**在球面上求值的 3D 噪声** —— `noise3(x, y, z)`，其中 `(x, y, z)` 是**方向向量**。
定义域是 R³，球面只是其中一张曲面 ⇒ **不存在坐标奇异点**：极点连续、`φ=180°` 处无缝、
特征尺寸处处相同（各向同性）。这也是 [libnoise 教程 8](https://libnoise.sourceforge.net/tutorials/tutorial8.html)
与几乎所有程序化星球的做法。

**其他真正的球面构造**（备选，本轮未采用）：

- **球谐（SH）**：带宽受限、天然无缝，擅长低频（大气、重力场、行星尺度趋势）；代价 O(波段²)。
- **球面 Voronoi**：Fibonacci 球面均布种子 + 大圆距离 ⇒ 等尺寸胞元，做**板块构造**最合适。
- **HEALPix / 测地网格**：等面积**域**（不是噪声函数），天文学界的标准采样网格。

### 23.7 实验：给噪声算子加 `spherical`（默认开）

给 `field.fbm` / `field.ridged` 各加一个 `spherical: bool`（默认 `true`），内部把 `(u,v)` 映成方向再求
**3D 值噪声**。**产物格式、网格、渲染器一行都没改**：

| | 极点 | 特征尺寸 | 烘图耗时 |
|---|---|---|---|
| 2D equirect 噪声 | 放射状山脊 + 三角扇 | 极区被拉伸 | 18–22 ms |
| **球面 3D 噪声** | **干净**（地形平滑翻过极点，只剩冰盖） | **处处一致** | 29 ms（约 1.4×） |

对照图：`target/sphere3d-rocky.png`、`target/sphere3d-desert.png`。

⇒ **结论：这个问题指向的正确答案就是「按方向求值的 3D 噪声」，而且它不需要改域格式。**
equirect 图只是变成"极区冗余但数值正确"（正是 §23.3 里 PanoTools 那条讲的冗余）。

**代价与注意**：

- 3D 值噪声要取 8 个晶格点（2D 是 4 个）⇒ 约 **1.4× 耗时**，可以接受。
- ⚠️ **频率语义变了**：单位球上绕赤道一圈是 2π，所以 `frequency = f` ⇒ 赤道上约 `2πf` 个特征。
  旧参数（3.2 / 11.0）按 3D 语义相当于"赤道上 20 个格子"，**必须重调** ——
  现在大陆 `0.55`、山脉 `1.9`、沙漠台地 `0.45`、峡谷 `3.2`。这是本轮唯一"必须重做"的东西，
  而重做一次只要 0.5 s。
- **`field.warp` 仍然是图空间扭曲**：它在像素坐标里偏移采样，极点附近并不"球面正确"。
  真正的球面域扭曲应该扭曲**方向向量**再归一化。⇒ 列为待办。

### 23.8 路线重排（极点动机消失后）

D（立方体域）当初的两个动机是**极点**和**接缝** —— 球面 3D 噪声把两个都解决了，
而且是在**不动产物格式**的前提下。所以 D 的优先级从"该做"降到"可选"，
剩下的动机只有"极区纹素冗余"（浪费算力和显存，不影响画面）。
**建议：D 先不做**，把力气花在别处（大气、环阴影、更多星球配方）；等真的遇到
"极区分辨率不够"或"要按面做 LOD"时再回头做。

### 23.9 纠正：「极区分辨率不够」是我说错了，实际是反过来

用户追问：*「为啥三维噪声在两极分辨率不够？球面的方向向量在 R3 上不是均匀的吗」*

**用户是对的。** 方向向量在球面上是均匀的、3D 噪声的晶格在 R³ 里也是均匀的
⇒ **噪声本身在极点没有任何分辨率问题** —— 这正是 §23.7 那张表里"特征尺寸处处一致"的含义。
我上一轮口头说的"极区分辨率不够"是错的；§23.8 里写的"纹素**冗余**"才是对的。特此更正。

**不均匀的不是噪声、也不是方向，而是"烘焙网格"**：equirect 图把 v 均匀分行、u 均匀分列，
于是每个纹素覆盖的球面弧长随纬度变化：

| 纬度 | 方位向弧长 / 纹素 | 相对赤道 |
|---|---|---|
| 0°（赤道） | `2π/W` | 1× |
| 60° | `2π·0.500/W` | 2× 密 |
| 80° | `2π·0.174/W` | **5.8× 密** |
| 89° | `2π·0.017/W` | **57× 密** |

（W=384、H=192 时：赤道上方位向 `2π/384 = 0.01636`、经向 `π/192 = 0.01636`
—— **赤道上正好是方纹素**，这正是 2:1 贴图配 360°×180° 的意义。）

⇒ **equirect 的稀缺方向是"赤道上的方位向"；极点是把纹素浪费掉了。**
要抱怨分辨率，也只能抱怨赤道。极点的问题是**冗余**（算力/显存），不是不足。

**equirect + 3D 噪声 剩下真正会咬人的两点**：

1. **放大时极点的各向异性**：极点附近一个屏幕像素的纹理足迹在纬度方向极窄、方位方向极宽
   ⇒ mip 选择要么糊要么闪。要放大到极点才看得见（我们现在的图里看不出来）。
2. **±180° 接缝的采样方式**：`ImageAddressMode::default()` 就是 **`ClampToEdge`**（已核对源码），
   而我们的 u 是绕球一圈 ⇒ 列 `W-1` 与列 `0` 之间是"夹住"而不是"接上"，一个纹素宽。
   现在看不见，但值得修：给这张纹理的 u 轴设 `Repeat`（约 3 行）。

**因此立方体域（D）的好处也要重新表述**：它不是"修正错误"，而是"**提高纹理预算效率**"——
密度几乎均匀（同一面内最差/最好只差约 1.7×），同样纹素数下最差处比 equirect 略好，
外加没有接缝、没有极点各向异性。**结论不变：不急。**

---

## 24. 用户提案：3D 场应该是 PCG 的一等公民，球面格式只是"导出"

用户原话：*「把三维纹理直接丢给渲染器其实浪费了我们的 PCG 框架，我们完全可以在 pcg 框架里生成
高精度的中间三维场，然后再采样到普通的 cube map 上或者任何球面纹理格式」*

**同意这个判断**，它指出了现在设计的真实缺陷：

- 3D 只是**求值时的瞬态** —— 算子在内部算 `fbm_3(direction(u, v))`，算完就拍平成 equirect 图；
- 因此**投影知识写在算子里**（`direction` 调用在算子内部）⇒ 换一种球面格式就得改算子；
- 因此**真正需要 3D 的操作写不出来** —— 球面正确的 warp（§23.7 那个待办）就是这个病根；
- 同一份场想再导出一份 cube map，只能重跑整张图。

### 24.1 先量价格：烘一个体 vs 烘一张球面图（实测）

`px_graphs/src/bin/volume_probe.rs`（**探针，不是图**），六阶 fbm 3D、单线程：

| | 样本数 | 耗时 | f32 体积 |
|---|---|---|---|
| 64³ | 262k | 92 ms | 1.0 MB |
| 96³ | 884k | 321 ms | 3.5 MB |
| **128³** | 2.1M | **754 ms** | **8.4 MB** |
| 160³ | 4.1M | 1498 ms | 16.4 MB |
| 对照：现在这条 384×192 **球面**烘焙 | 74k | **27 ms** | 0.29 MB |

⇒ 128³ 的体是**同一张球面图的 28 倍代价**（体素数之比正好也是 28×；每样本耗时两者都是 ~0.36 µs）。
比预想便宜（我以为要 1–2 s），但也不小。

### 24.2 关键推论：**"能导出多种格式"并不足以证明该烘体**

因为**只采样球壳的话，按方向直接求值的代价和现在完全一样**（每个输出纹素一次 3D 求值）。
烘体的真正理由是另外三条：

1. **要采内部**：等值面提取（悬崖 / 洞穴 / 真实地形）、体积云与大气、需要内部信息的侵蚀模拟；
2. **要摊薄**：一份体导出很多份高分辨率输出（4K equirect + 6×2K cube + 各级 mip）时才回本；
3. **要共享 / LOD**：体天然可以带 mip 金字塔做 LOD，也可以被别的图直接消费。

所以该拆成**两件事**，而不是一件。

### 24.3 第一步（便宜、现在就能做）：把"投影"从算子里拿出来

- **`Field` 带一个 `Grid` 描述**：`{ projection: Equirect | CubeAtlas, width, height }`；
- 算子**不再自己算 `direction`**，而是向 Grid 问"这个像素对应哪个方向"；
- ⇒ **图声明自己的输出投影，算子完全投影无关**：想导 cube map 就换 Grid，**一个算子都不用改**
  （这正是用户要的"采样到普通 cube map 或任何球面纹理格式"）；
- 顺带**球面正确的 warp 可以写了**：`warp3(p) = fbm3(p + strength · vecfield3(p))` ——
  逐输出纹素求值，**代价和现在一样**。

### 24.4 第二步（等真需要内部采样时再做）：`AssetKind::Volume3D`

- 协议加一种资产类型（`BlobHeader.shape` 本来就是 `Vec<u32>`，支持任意维度 ⇒ 改动很小）；
- `VolumeOp` trait（或把 `FieldOp` 泛化成 `Grid<T>`）；
- 新增 `volume.project` 节点：把体按 Grid 采样成球面图，可做超采样 / 面积平均
  ⇒ **顺手把极点各向异性也解决**（有内部就能做正确的滤波）。
- **触发条件写清楚**：要悬崖 / 洞穴 / 体积云，或要按面做 LOD，或一份体要喂多份导出。

### 24.5 待裁决

1. **Grid 的归属**：每个节点自带（源算子建立、后续继承）还是**整张图一个**（放 `GraphSpec`）？
   我倾向**图一个** —— 节点级 Grid 现在没有需求，而且会让缓存键更复杂。
2. **cube atlas 布局**：3×2 六面，还是 4×3（留 padding 防 mip 渗色）？
   我倾向 **3×2 + 采样时按面夹紧**，先简单。
3. **第二步要不要现在就把 `Volume3D` 写进协议**（哪怕先不实现）？

---

## 25. 把球面 UV 这条路做正确（用户裁决：不烘体）

用户裁决：*「那还是先不做体烘焙，体烘焙丢掉解析以后继续处理精度会掉。把正确的球面 UV 贴图效果做好」*，
Grid 那套抽象也不做。

**这个理由比"28× 代价"更本质，记下来**：烘体 = 把解析形式离散化，
此后每一次下游处理都在离散数据上插值采样，**误差会累积**；
而保持解析（按需在方向上求值）则每一步都是精确的。§24 的 §24.3/§24.4 两步方案就此作废，
**只保留"把球面映射本身做对"这条线。**

### 25.1 这一轮修掉的四件事

**① UV 约定统一（`v = y/(h-1)`，第 0 行 = 北极）。**
之前 `px_ops::Field::uv` 用 `y/height`、`px_render::Field::sample` 用 `(1-v)*(h-1)`
—— 两边差一行、还差一个南北翻转。现在三处一致：`Field::uv`、`noise::direction`（v=0 ⇒ θ=0 ⇒ 北极）、
网格 UV（Bevy icosphere 的 v=0 就是 +Y）。

**② ±180° 接缝**：给表面纹理的 u 轴设 `Repeat`（v 轴 `ClampToEdge`），环纹理反过来。
⚠️ **踩到的坑**：一旦给 `Image` 显式设 `ImageSampler::Descriptor`，
过滤模式就**不再继承 `ImagePlugin` 的 `default_linear()`**，而是取
`ImageSamplerDescriptor::default()` —— 而 `ImageFilterMode::default()` 是 **`Nearest`**。
画面立刻变成方块。必须显式写 `mag/min/mipmap_filter: Linear`。

**③ 值噪声的轴对齐晶格（方块感的真凶）。**
换了分辨率、修了过滤，方块还在。算一下尺寸就清楚了：`frequency 0.55 × 2⁵ = 17.6`，
晶格间距 `1/17.6` 单位 ≈ 球面 3.2° ≈ 屏幕上 **9 px** —— 与看到的方块大小吻合。
**值噪声的值长在轴对齐的立方晶格上，高频八度会把方格暴露出来。**
⇒ 换成 **Perlin 梯度噪声**（12 个立方体边梯度、8 角点、smoothstep 权重）。
海岸线立刻变成有机曲线 + 峡湾。`field.fbm` / `field.ridged` 因此升到 **v3**。

**④ 球面正确的 `field.warp`**：位移单位从"画布宽度比例"改成"**弧长（占半径比例）**"，
经度方向按 `1/ring` 换算（`ring = sqrt(sin²θ + 0.16)`，**平滑下限**）。
⚠️ 第一版用 `sin.max(0.25)`（硬下限）：极区经度位移被放大到近 100°，画面被抹平，
而且硬下限造出一条**锯齿状的边界**。改成平滑下限后，峡谷网络均匀地绕过整个球。
`field.warp` 升到 **v2**，沙漠 `strength = 0.40`。

### 25.2 缓存机制立功：画布尺寸没进键

把画布从 384×192 改成 768×384 后，**所有节点照样命中**（产物还是 295111 字节的旧尺寸），
是 §19.1 那条 ⚠ 告警把它喊出来的（"图 planet 的源码变了但 GRAPH_VERSION 仍是 1"）。

⇒ **真 bug**：`GraphSpec.width/height` 影响每个节点的输出，却没进缓存键。
已修：`node_key` 增加 `canvas: (u32, u32)`，并补了测试
`the_canvas_size_is_part_of_the_key`。

**这条值得单独记**：告警机制第一次真正拦下一个静默错误，而且拦下的不是"忘升版本"，
而是**"有一个参数根本没进键"** —— 设计告警时没想到的用法。

### 25.3 代价更新（诚实记账）

| | 旧（384×192，值噪声） | 新（768×384，Perlin） |
|---|---|---|
| 单个噪声节点烘焙 | 18–29 ms | **287–297 ms** |
| planet 图全量 | 67 ms | **619 ms** |
| desert 图全量 | 96 ms | **779 ms** |
| 出图 | 0.34 s | 0.37 s |
| **整轮热路径** | 0.49 s | **≈ 1.0 s** |

⇒ 质量提升的代价是**热路径刚好顶到 1 s**。若嫌慢，可把画布降到 512×256（约省 55%）。

### 25.4 还没做

- mipmap：运行时生成的贴图**没有 mip**（Bevy 只给压缩格式生成）⇒ 各向异性过滤现在是空转，
  缩小时会闪。要做就得在 CPU 侧生成 mip 金字塔。
- 球面正确的 warp 用的是"两处采样当两个方向位移"，不是真正的 3D 矢量场扭曲（后者要 §24 的 3D 语义）。
- 沙漠色板在 Perlin 分布下偏灰，配方还要再调（属于美术活，不是通路活）。

---

## 26. mipmap：让各向异性过滤不再空转

**问题**：运行时生成的贴图**没有 mip**（Bevy 只给压缩格式 / DDS / KTX2 生成），
所以 `anisotropy_clamp` 一直是空转，而且一旦缩小就闪。

**做法**：CPU 侧建整条 mip 链，2×2 盒式滤波，两处按球面语义处理 ——
**经度方向环绕平均**（列 `W-1` 与列 `0` 其实是邻居）、**纬度方向夹紧**。
一次覆盖四张贴图：星球表面色带 / 熔岩自发光 / 环带 / 星空。
`anisotropy_clamp` 提到 **8**（有了 mip 才有效）。

数据排布：整条链按级顺序拼进 `Image::data`，并把
`texture_descriptor.mip_level_count` 设为级数 —— Bevy 的上传走
`RenderDevice::create_texture_with_data`，**会按级数自己切**（已核对 wgpu 侧实现）。

**验证**：
- 诊断行报 **`mip 10 级（约 1.6 MB）`**（768×384 ⇒ `floor(log2 768) + 1 = 10` ✓）；
- 管线 41/41 就绪、无校验错误（`anisotropy_clamp: 8` 被接受）；
- 480×320 的缩小渲染干净、星点均匀 —— 这才是 mip 真正起作用的情形。

**诚实记账**：
- 主视角下星球是**放大**采样，所以 mip 目前主要帮到**星空**、更小的星球构图，
  以及将来自转动画时的闪烁；不要以为它让主图变好看了。
- 代价：数据 **+33%**（768×384 全链 1.6 MB）；表面贴图每个请求重建一次链（几 ms，
  实测渲染耗时仍是 ~330 ms，没变）。
- 星空那张是一次性建好的（启动时），环带与表面贴图每请求建。

---

## 27. 持久预览窗口（用户要求）+ 一条经线的接缝 + 极轴转错

用户要求：*「让 render 可以用命令开一个交互式的 viewer 窗口，这样你做完一个效果可以调出这个窗口让我 review」*，
随后补充：*「这个应当是一个持久化的窗口」*。

### 27.1 形状：常驻窗口 + 请求文件（不是每次弹新窗）

- `px_render --view` 开一个**常驻**窗口（轨道相机、自转、热键）；
- `px_render --show --planet <文件> --palette <色板> [--shot]` **推场景**进这个窗口
  （写 `target/viewer-scene.json`，含纳秒时间戳去重；窗口每帧轮询该文件）；
- 窗口每次重建场景时**同时盯着场文件的 mtime** ⇒ 我重新烘图，窗口自动更新；
- 心跳写 `target/viewer.json`，`--show` 读它报"窗口在线/没在跑"。
  这套"薄客户端 + 请求文件 + 心跳"与渲染服务的租约是同一个思路，但不占第二个端口、不需要回复。
- 热键：`1-5` 换色板（连带该色板的默认位移/海平面/环）、`[ ]` 海平面、`- =` 位移、`r` 环系、`空格` 自转、`s` 存图、`q` 退出。

### 27.2 三个坑

**① 窗口截图要等管线就绪。** `--shot` 一开始定在 12 帧后截图 ⇒ **全黑**（就是 §12 那条异步管线编译：
当年离屏 20/120 帧也是黑的）。截图前必须检查管线。
⚠️ 而且 `PipelineCache` 是**渲染世界**的资源：把它写进主世界的 `Update` 系统会直接
`Parameter failed validation: Resource` **panic**。正确做法是照 `serve()` 的样，
在 `Render` 调度里跑就绪检查、通过 `RenderReady(Arc<AtomicBool>)` 把标志传回主世界。

**② 窗口会因为 resize 直接退出。** 日志：`wgpu_hal::dx12: ResizeBuffers failed` →
`surface configuration failed: window is in use` → `Quitting the application due to Validation RenderError`。
Bevy 默认**任何渲染错误都退出**；预览窗口显然不该这样。
⇒ 装一个自定义 `RenderErrorHandler`，策略返回 `Ignore`（记日志、继续渲染）。
（另外：不要用 `Start-Process -WindowStyle Hidden` 起它——之后 `SetForegroundWindow`
会让窗口从隐藏变可见，正好触发上面那次重配。）

**③ 父实体缺 `Visibility`。** 我把星球挂到一个只带 `Transform` 的倾斜父实体下 ⇒ Bevy 报
`B0004`（父实体缺子实体需要的组件）。补 `Visibility::default()`。

### 27.3 那条"赤道"接缝，其实是经线 —— 而我看错了两次

用户先报"赤道上有接缝"，随后自己纠正：*「不是赤道，是某一条经线，而你把两极平放了让我误以为是赤道」*。
**用户是对的，而且这个纠正直接指向真凶**：`icosphere` 的 UV 在 ±180° 经线上不连续
（正是 §23.3 引用的 [bevy#4987](https://github.com/bevyengine/bevy/issues/4987)），
跨接缝的三角形的 UV 在贴图空间里横跨整张图 ⇒ 一条锯齿带。
**`AddressMode::Repeat` 治不了它**，因为问题出在**顶点插值**，不在采样器。

⇒ 自己生成球面网格：**接缝处复制一列顶点**（u=0 与 u=1 各一份）、
**极点每个扇区一个顶点**（UV 取该扇区中点，否则极冠三角形又在整张贴图上横扫）、
并且按位置**焊接法线**（`compute_smooth_normals` 是按顶点索引累加的，接缝两列会各算各的）。

### 27.4 极点是"预期的效果吗" —— 不是，而且原因是我的旋转写错了

用户问：*「这个极点是预期的效果吗？」*（一张对着极点拍的、满是白色星芒的特写）

**不是预期的。** 但根因不是极点滤波，而是：自建球面的极轴是 **±Y**，
而子实体上还留着 `Quat::from_rotation_x(-FRAC_PI_2)` ——
那是当年给 **Bevy UV 球（极轴 ±Z）** 用的。于是**南极几乎正对相机**：
你看到的"星芒"就是**正对极轴俯视时的极地冰盖 + 极点三角扇**，
并且**环系也落在了错误的平面上**（不再是赤道面）。

⇒ **换图元时必须重新推导约定**，这是同一个坑的第三次（§21.1 的 ±Z、§23.2 的 ico ±Y、这里的遗留 `-90°`）。
判据：气态色板的**条带方向**立刻告诉你极轴在哪 —— 这是最快的极轴检查法。

### 27.5 极点滤波（仍然成立了，只是当时被 27.4 掩盖）

极点那一行纹素**覆盖的是整整一圈**（整圈经度塌到一个点）⇒ 放大极点时看到的是
**一整行纹素的径向拉伸**。**正确的值就是那一圈的平均** —— 这不是遮丑，是退化映射的正确滤波。
所以：贴图顶部/底部各 `height/32` 行朝该行均值混合（权重 1→0），位移也同样处理
（`Field::sample_capped`）。修完之后极点是一块干净的白盖。

---

## 28. 换成八面体域（用户裁决）—— 极点退化是滤波治不了的

用户先问：*「球是不是还在使用经纬度 UV？切换成 cubemap 之类的方案 uv 均匀性更好吧」*，随后补充
*「或则八面体 UV」*，并纠正了我的理解：

> *「cubemap 不是 3d texture，cubemap 是天空盒子常用的正方形表面，八面体是八个象限，
> 每个象限一个等边三角形拼成的盒子」*

**采纳**。而且这两个说法指向同一个东西的两种讲法：**把八面体摊开就是一个正方形** ——
正方形正中的菱形是 +Z 的四个象限，四个角是 −Z 的四个象限。这就是文献里的 octahedral map。
它比 cubemap 少一整套"六个面 + 面边界 + gutter + 逐面采样"的管路。

**为什么必须换**：经纬度图的极点纹素**覆盖整整一圈**（§27.5），是**退化映射**，
任何滤波都只能"平均掩盖"；八面体图**每个纹素对应一块确定的立体角**，
texel 密度变化 ≤ 2×（cubemap 单面约 1.7×，同一量级）。

**这一版的形状**：
- `px_protocol::art` 提供 `octahedral_direction` / `octahedral_direction_y_up`
  （**双方共用**，因为它是产物格式的一部分）；
- `GraphSpec` 增加 `projection`，它**和画布尺寸一样进缓存键**；
- 星球图改为 `512×512` + `Projection::Octahedral`；**沙漠图仍是 equirect**
  （它的 `field.warp` 是图空间扭曲，跨折叠面会错，等 3D 矢量扭曲做完再迁）。

### 28.1 迁移途中三个 bug（每个都是量出来的，不是猜出来的）

**① 烘焙和网格用了不同的约定。** `px_ops` 用的是"极轴 +Z"的映射，网格用的是 `_y_up` 变体
⇒ **贴图相对几何转了 90°**。

**② 纬度被按整行平均。** 对 equirect 一行就是一条纬线（没错），
但**八面体图里一行会跨过折叠面**，纬度处处不同 ⇒ 改成逐纹素算。

**③ 最大的一个：`compute_smooth_normals` 在角点附近给出了零法线。**
四个角是 −Z 极点，那里纹素高度重合、三角形退化成薄片，面法线平均后**长度为 0**，
`try_normalize().unwrap_or(ZERO)` 于是得到**零向量** ⇒ 着色全黑。
**排查链**（每步都排除了一个可能）：
- 三角形绕序审计：`203522 个三角形，0 个面法线朝内` ⇒ 不是绕序；
- 位移半径审计：`最远顶点 1.0360`（恰好是 1 + 位移上限）⇒ 不是坏顶点；
- 关阴影：图像**字节完全没变** ⇒ 不是自阴影；
- **`unlit` 对照：贴图完全正确**（海洋、大陆、冰盖、无接缝）⇒ **锁定法线**；
- 法线审计：`0 个与半径反向，最小点积 0.000` ⇒ 有一批零法线 ✓ 抓到。

修法：**不再依赖面法线平均，改用网格邻接的有限差分算解析法线**
（`grid_normals`：`∂r/∂u × ∂r/∂v`，再按半径方向定号）。审计变成 `最小点积 0.745` ✓，
光照恢复正确（太阳从左上打、有明暗界线）。

**方法论**：`unlit` 对照是"贴图 vs 光照"最快的分界实验，值得记成常规手段。

### 28.2 顺带发现：源码哈希有个洞

算子只哈希**自己那个文件**（`include_str!("fbm.rs")`），所以改**共享代码**
（`field.rs` 里的方向约定）时，§19.1 那条"源码变了"警告**不会响**。
本轮手动把 `field.fbm` / `field.ridged` 升到 **v4**。
⇒ 待办：`SOURCE_HASH` 应该把依赖（`field.rs` / `noise.rs`）也哈希进去。

### 28.3 残留

南极（八面体图的四个角）还有**一条细黑影**。它**不是几何**（半径审计干净、关星空后仍在），
是角点退化薄片上的法线残影。下一刀：把角点附近的法线做一次局部平滑，或者让网格避开角点的奇异采样。

---

## 29. 几何改由 PCG 生成（用户裁决）—— 四条黑缝的真正修法

用户看到四条从一点放射出去的黑线后纠正：*「wait，这个网格应该从 pcg 生成传给 render」*。
**采纳，而且这正是黑缝的正确修法** —— 缝合问题出在"用什么拓扑构造球面"，那属于几何生成，
本来就该在 PCG 里。

### 29.1 形状

- **线格式不动**：`Blob { dtype, shape }` 已支持 `U32` ⇒ 一个 Mesh 产物 = **4 个数据块**
  （positions / normals / uvs / indices），顺序与 `px_protocol::art::MESH_*` 常量一致，
  `MeshData::{blobs, from_blobs}` 由**协议自己**读写（和八面体映射一样，格式属于协议）。
- `px_ops` 的产物从"一个场"变成 `Payload::{Field, Mesh}`，缓存/索引/清单对两者通用；
  新增 `MeshOp` trait 与 `mesh_node::<Op>()`（`MeshOp::eval` 收 `&[&Field]`，所以网格算子能吃场）。
- 新算子 **`mesh.octasphere`**：把**八面体的八个面各自**细分成三角网格。
  相邻面共享边的顶点**位置与 UV 完全一致**（UV 由 `octahedral_uv_y_up(方向)` 给出，
  是方向的函数 ⇒ 两侧必然相同）⇒ **天然闭合，四条缝消失**。
  极点（世界 ±Y）成为四个面共享的顶点，周围是正常的三角伞。
- 渲染器不再造几何：`--mesh <产物>` 直接加载；`Scene::Planet` 增加 `mesh` 字段（SCHEMA 5）。
  没给 `--mesh` 时仍走自带球面（沙漠图等还用得上）。
- `Field::sample_direction(方向)`（投影无关：八面体走 `octahedral_uv_y_up`，经纬度走 acos/atan2）
  ⇒ 网格算子与投影解耦，沙漠图迁过来时可以直接复用。

### 29.2 途中三个 bug，最后一个最值得记

**① 一半的 patch 绕序是反的。** 我按 `(corners[a], corners[b], corners[c])` 固定顺序生成，
但 `(−X,+Y,+Z)` 这类组合的手性与 `(+X,+Y,+Z)` 相反 ⇒ **半个球被背面剔除**。
unlit 截图里"左上整块是黑的 + 月牙缺口"就是这个。
修法：每个面用**面心方向**做一次定向检查，反了就交换两个角。

**② 法线扇形在边界上取错了邻居。** `(i+1).min(n-j)` 在最后一行会把邻居**替换成第一排的远处顶点**，
差分横跨整个面 ⇒ 法线全废（审计：`最小点积 0.000`），球又变黑。
修法：六邻域**环形**累加（`cross(p_k-here, p_{k+1}-here)` 求和），越界就 `continue` 跳过。

**③ 真正让球变黑的是"提前 return 跳过了场景末尾的太阳和环境光"。**
PCG 网格分支为了图省事提前 `return Ok(...)`，而 `DirectionalLight` / `AmbientLight` 是在函数**末尾**
才 spawn 的 ⇒ 只剩环境光。
**最强的线索是：改了法线之后图像字节完全不变**（说明法线根本没进画面），而 unlit 一切正常。
修法：抽 `spawn_lights()`，两条路都调用。

> **教训**：场景构造函数的"提前 return"会静默跳过末尾的灯光/相机，类型系统一点忙都帮不上。
> 判据就是那条"字节完全相同"。

三个"最小点积"审计（PCG 算子内、渲染器加载后）都保留下来了 —— 本轮它们抓到了两个 bug。

---

## 30. 探针框架（用户要求）+ 为什么最终要回到 cubemap

用户两次指出接缝仍在，并要求：*「你的 probe 框架应当把环境光拉高一点然后一次性多个角度拍几张，两极一定是重点」*。

### 30.1 `tools/probe.ps1`

一条命令出 **10 个角度**（赤道四向 / ±45° / 南北极各两向）的对照图，环境光默认 **260**
（暗面也照清楚，接缝无处可藏），自动拼成一张 4 列网格图并在每格左上角打角度标签。
渲染器侧新增两个**渲染器选项**（不属于场景内容，所以放进 `Request.view`）：
`--ambient F` 与 `--cam yaw,pitch,dist`。

**这个工具立刻改变了诊断效率**：接缝的位置（哪条棱、哪个极）、形态（黑线/白线/大块错位）一眼可见。

### 30.2 它查出来的事实

**① 网格拓扑上根本没缝上。** 探针之后的边审计：
`309120 条边，3840 条只属于一个三角形` —— **3840 = 8 面 × 3 边 × 160 段**，
即八个 patch 各自独立，只是顶点在空间里碰巧重合。
⇒ 改成**按方向量化去重、全局共用一套顶点索引**后：`102402 顶点，开口边 0` ✓。

**② 仿射 UV 是死路（被单元测试当场抓住）。** 我一度按"每个面在展开图里是仿射三角形"来插值 UV，
测试 `the_affine_table_agrees_with_the_decoding` 报 **最大误差 1.91**：
**八面体映射在面内不是仿射的**（是 L1 归一化的投影）⇒ 重心插值必然错位，
表现就是整块面被镜像采样（`probe-rocky2` 里的大块硬边暗区）。
⇒ UV 必须回到**按方向精确编码**（`octahedral_uv_y_up(方向)`），并配一条不变量测试
（`every_vertex_uv_points_back_at_its_direction` ✓）。

**③ 剩下那几条细线，是折叠破坏了纹理过滤 —— 这是八面体图的固有性质。**
焊接闭合 + 精确 UV + 累加法线都做完之后，细线仍在，且**固定在八面体的棱上**：
折叠线两侧的相邻纹素在球面上是**镜像的不相邻方向**，双线性/各向异性过滤跨过折叠线时会把无关纹素混进来。
mip 链同理（我现在的 mip 是 x 环绕、y 夹取，对八面体布局本来就是错的）。
octahedral map 是**为法线编码设计的**，那种用途不看过滤质量；
**要贴图的球面应该用 cubemap**：六个正方形面彼此独立、没有折叠，跨面边缘两侧的纹素对应几乎相同的方向，
过滤几乎正确，再加一圈邻面 gutter 就完全正确 —— 这也正是天空盒用 cubemap 的原因，
也就是用户最初建议的方案。

**下一步**：把 `Projection` 从 `Octahedral` 换成 `Cube`（3×2 图集 + gutter，或六层），
烘焙路径、网格 UV、mip 规则一起换，然后用 `tools/probe.ps1` 复验。

---

## 31. 换到 cubemap（用户裁决：3×2 图集 + gutter）—— 接缝清零

### 31.1 布局

一张图集，**3 列 × 2 行**，每格 `cell = face + 2×gutter`（本轮 face 256、gutter 2 ⇒ cell 260、
图集 **780×520**）。面序 `+X, −X, +Y, −Y, +Z, −Z`。
`px_protocol::art` 提供 `cube_direction(face, s, t)` / `cube_face_of(方向)` / `cube_atlas_uv(...)`，
**双方共用**（协议拥有格式），并有一条测试钉住"方向↔面内坐标"往返与"gutter 落在邻面贴边处"。

### 31.2 gutter 为什么天然无缝

面的参数化 `direction = f(face, s, t)` 允许 **s、t 超出 [0,1]** —— 外推出去的方向**正好就是邻面的方向**。
所以 gutter 里不需要"复制邻面边缘"这一步特殊处理，照着同一个映射烘就行 ✓。
（`cube_face_of` 的测试证实：越过 +X 面 s<0 的半步，落点确实在 +Z 面的贴边处。）

### 31.3 网格与结果

`mesh.cubesphere`：六个面各 n×n 四边形、顶点**按量化方向焊接**。
焊接容差从 `2^20` 放到 `2^18` 是必须的 —— 2^20 时立方体的棱上还有 **48 条开口边**（两个面算出的同一方向
在浮点末位不一致），放到 2^18 后：

```
153602 顶点 / 307200 三角形，面 256²、格子 260、开口边 0，法线朝内 0、零长 0、最小点积 0.769
```

对照八面体那一版（`开口边 0`、最小点积 **0.579**）：cube 版的法线更平滑，
且 **10 角度对照图里接缝肉眼不可见**（`target/probe-cube2.png`），两极干净。
行/列突变扫描的最大值现在落在**轮廓线与明暗界线**上，而不是横贯球面的直线。

### 31.4 为什么 cube 赢

八面体图的折叠会让**折叠线两侧的相邻纹素对应镜像的不相邻方向**，
双线性/各向异性/mip 过滤跨过它就混入无关纹素 ⇒ 棱上必然有线，这是布局的固有性质 ✗。
cube 的六个面彼此独立、**没有折叠**，跨面边缘两侧的纹素对应几乎相同的方向，
过滤几乎正确，加上 gutter 就完全正确 ✓ —— 这正是天空盒用 cubemap 的道理。

（`Projection::Octahedral` 与它的映射测试都留着，作为这条推理的证据链；
星球图现在走 `Cube`，沙漠图仍走 `Equirect`，等 3D 矢量扭曲做完再迁。）

---

## 32. 立方体棱上那条细线 + 环境光其实一直是死的

用户放大截图：*「截图了，有细线接缝」*，并提出探针要*「从两极移一个到角点」*。

### 32.1 细线的真因：棱上顶点被两个面共用，却只有一个 UV

cube 域里每个面在图集里有自己的位置，**棱上的顶点需要两个不同的 UV**（各面一份）。
我上一版按"方向"焊接 ⇒ 棱上顶点被共用 ⇒ 其中一个面的三角形被拉到另一面的图集区域 ⇒ 沿线细带。

⇒ 焊接键改成 **(面, 方向)**：棱上复制顶点、各带本面 UV，**位置仍然严格重合**（方向相同 ⇒ 位移相同）
⇒ 没有几何缝；法线再在算子内**按位置焊接**一次（`法线焊接 1916 组`）⇒ 跨面平滑。
结果：`155526 顶点 / 307200 三角形、开口边 3840`（cubemap 的标准状态：棱上顶点成对、位置重合）、
`最小点积 0.769`；特写扫描里最强整行只有 2.6%、整列 6.7% 跳变（真直缝应是几十个百分点）。

### 32.2 探针按用户要求改：角点取代冗余极视角，且要算上倾斜

`SYSTEM_TILT` 会让"按 yaw/pitch 直瞄"打偏，所以脚本里加了 `AimLocal`：
把局部方向 `(1,1,1)` / `(1,-1,1)` / `(1,1,0)` / `(0,0,1)` 过一遍 `Rx(tilt)` 再换算成 yaw/pitch。
现在 12 个视角 = 赤道四向 + ±45° + 南北极 + **立方体角点×2 / 棱中点 / 面心**（后四个是 1.40 倍距离的特写）。
**cube 域里奇异点从两极搬到了立方体的十二棱与八角**，所以探针重心也跟着搬。

### 32.3 顺手挖出一个一直存在的 bug：`AmbientLight` 在 0.19 是**相机组件**

我此前把 `AmbientLight { brightness }` 当**场景实体** spawn（`commands.spawn((ScenePart, AmbientLight {...}))`）——
在 Bevy 0.15 之后它只是"可以挂在相机上覆盖 `GlobalAmbientLight` 的组件"，**散在场景里完全无效** ✗。
所以：`--ambient` 从头到尾没用，用户要的"拉高环境光"**一直没生效**（探针其实一直在默认亮度下拍）。
判据很干脆：`--ambient 4` 与 `--ambient 400` 出图**字节完全相同**。
修法：挂到相机实体上（`accept_jobs` 用请求里的值、预览窗口用默认值），复核：两者出图不同 ✓。

### 32.4 一个"看起来该修、其实不用修"的地方

gutter 只有 2 纹素，理论上 mip 2 之后就只剩 0.5 纹素 ⇒ 整图 mip 链会**跨面混合**。
我实现了**逐面 mip**（每层把六个面各自滤波后重新拼进同一层），结果**图像字节不变** ——
因为 gutter 里装的本来就是**邻面的方向**，跨不跨面混出来的值差异小于 1/255。
⇒ 这一环天然正确，逐面 mip 留着（语义更清楚、未来换更大 gutter 也不会退化），但不是接缝的成因。
之前放大看到的那条"柔和带"其实是**北极冰盖的纬度边界**，不是接缝。

---

## 33. `field.warp` 三维化，沙漠图迁 cube

### 33.1 扭曲改成方向空间的矢量位移（v3）

旧版在**图空间**做偏移：经纬度下有 `1/sinθ` 的极区换算，八面体/cube 下会跨折叠、跨面 ✗。
新版只做三件事，全部投影无关：

1. `方向 = grid.direction(x, y)`；
2. 在该方向建**切框架** `(east, north)`（`tangent_frame`：极点附近换个 up 轴，避免退化）；
3. 两个扭曲分量来自**同一个扭曲场**在两个方向上的采样（原方向、以及沿 east 探出 `probe` 的方向 ——
   这样两个分量去相关，且不需要第二个输入）；位移后归一化，再用 `input.sample_direction(新方向)` 重采样。

参数只剩 `strength`（弧长，弧度）/ `lateral` / `probe`，`spherical` 开关删掉（不再需要）。
**测试**：`a_constant_warp_field_leaves_the_input_alone_in_every_projection` ——
常量扭曲场下，三种投影（经纬度/八面体/cube）都必须"不动"，最大偏差 < 0.02 ✓。
这条测试同时覆盖了 `direction_at` 的三个分支。

顺带把方向与切框架抽成 `px_ops::field::{direction_at, tangent_frame, normalize, cross}` 与 `Grid::direction`，
`Field::direction` 改为调用共用实现（此前渲染器里还有一份自己的 `Projection::direction` ✗，将来也应收敛）。

### 33.2 沙漠图迁 cube

`desert` 图：画布 780×520、投影 `Cube`、末尾加 `mesh.cubesphere` 节点（`art/desert/surface.toml`），
`GRAPH_VERSION` 升到 2。烘出来 8 个节点 2.0 s（网格 789 ms、155526 顶点 / 307200 三角形）。
12 视角对照图（`target/probe-desert.png`）**无接缝**，含棱/角特写 ✓。

### 33.3 一个待办：两张图共用一个 manifest

`planet` 与 `desert` 都写 `target/pcg/manifest.json`，取产物只能"取最后一个同名节点" ✗ 有点脆。
应改成**按图分文件**（`target/pcg/<图名>/manifest.json`），顺带让渲染命令能按图名找产物。

---

## 34. 第一个自写 shader：大气边缘光

### 34.1 结构

- `px_render/src/atmosphere.rs`：`AtmosphereMaterial`（`AsBindGroup` + 两个 uniform：参数、色调），
  `impl Material` 只给 `fragment_shader()` 与 `alpha_mode() = Add`；`AtmospherePlugin` 注册 `MaterialPlugin`。
- `px_render/assets/shaders/atmosphere.wgsl`：菲涅尔轮廓 ——
  **把法线用 `view.view_from_world` 转到视图空间**，轮廓就是 `normal_view.z → 0`，
  再按"朝太阳程度"调制，加法混合。
  ⚠️ **这一段描述的是第一版，现已迭代三代** ✗：菲涅尔 → **弦长积分** → **弦长终点由深度决定** ✓。
  现状、以及为什么改，见 **§38.4**。菲涅尔写法已删除 ✗。
- 几何：比星球大 3.5% 的平滑球（`Sphere::new(r*1.035).mesh().ico(48)`），挂在同一个 system 实体下。
  **不需要正面剔除**：正对相机的部分 `dot` 自然接近 1 ⇒ 菲涅尔为 0，只在轮廓发亮。
- 色调/强度/幂次**按调色板给**（`Palette::atmosphere()`）；`--atmo` 是渲染器侧倍率（0 关掉），
  和 `--ambient`/`--cam` 一样放在 `Request.view` 里（属于渲染器，不属于场景）。

### 34.2 三个坑

**① 只在预览窗口的 App 注册了插件。** 服务端是**另一个 App**，`Assets<AtmosphereMaterial>` 不存在
⇒ `accept_jobs` 的系统参数校验失败、整个服务 panic。两个 App 都要注册。

**② `AssetPlugin.file_path` 是相对可执行文件解析的**，不是相对工作目录：
我写 `"px_render/assets"` 结果去找 `target/debug/px_render/assets/...` ✗。
改成 `asset_root()`：先试工作目录、再沿 exe 往上找，找到含 `shaders/atmosphere.wgsl` 的那个。

**③ `DEFAULT_AMBIENT` 我拍了个 16，其实 Bevy 的全局默认是 80。**
把环境光从"无效实体"改成"挂相机"之后，等于把场景环境光从 80 悄悄压到 16 ⇒
整颗星球的观感都变了（发暗发蓝），我一度以为是 mip 链坏了、还做了 A/B ✗。
**换掉一个隐式默认值之前，先量出它到底是多少。**

**④ ⚠️ 本条结论已被推翻 ✗（2026-09-13 晚）。** 当时判"`world_position` 语义不可信"✗，
现在 shader 正大光明地用 `view.world_position` ✓，且**实测与传 uniform 版逐值相同** ✓（§38.4）。
真正的原因是**几何**：壳只比行星大 3.5% ⇒ 可见壳面处处贴近自身轮廓 ⇒ 菲涅尔处处 ≈1 ⇒
整片糊在球面上 ✗。而且这个误判是在**管线还在输预热竞态**的期间做出的 ✗。
**教训：测量结论必须绑定测量时的系统状态 ✓** —— 系统半坏时测出的"引擎不可信"极易误导 ✓。

---

## 35. 用标准 cubemap 统一天空（用户裁决 A 档）+ 统一到行星的代价表

用户问：*「bevy 的 cubemap 能统一我们的星球和天空盒子吗？用 bevy 的标准 cubemap，我们的 pcg 管线需要引入的代价是什么」*。
先把两条事实查实：

- **`Skybox` 就是标准 cubemap，且按方向采样**：`skybox.wgsl` 里 `var skybox: texture_cube<f32>;`
  `textureSample(skybox, skybox_sampler, ray_direction * vec3(1.0, 1.0, -1.0))` ✓
  （那个 `vec3(1,1,-1)` 是它的面朝向约定）。`Skybox { image, brightness, rotation }` 定义在 `bevy_light`。
- **`StandardMaterial` 收不了 cubemap**：`pbr_bindings.wgsl` 是 `var base_color_texture: texture_2d<f32>;` ✗
  ⇒ 想让**行星**也走标准 cubemap，必须**自写 surface material**（大气那个 shader 是第一步）。

**A 档（本轮做的）：只统一天空。**
`star_cube(face)` 生成**真正的 cube `Image`**（6 层 + `TextureViewDimension::Cube` ✓），
星点按**面内纹素**哈希 ✓；相机挂 `Skybox { image, brightness: 900 }` ✓；
**星空球实体与它的 UV 记账全部删掉** ✓✓。12 角度对照图（`target/probe-skycube.png`）星点是干净细点 ✓。

**代价（PCG 侧）：0。** 天空从来不是 PCG 产物 ✓ —— 这正好回答了用户的问题：
*cubemap 统一的代价全在渲染器要不要自己拥有 surface shader，不在 PCG* ✓。

**B 档（全面统一到行星）的代价表**（待做时按此执行）：

| 项 | 代价 |
|---|---|
| 布局 | 小：6 张等大正方形、**无 gutter**；线格式加层轴（`BlobHeader.shape` 本是 `Vec<u32>`，写 `[face, face, 6]` 即可） |
| 缓存 | 一次性全失效（键含画布尺寸）≈ 每图 1–2 秒 |
| 算子规则 | **中，唯一真规则**：需要邻居的算子必须走 `sample_direction`（模糊/腐蚀/距离场/流向），不能假定整图是平面；逐纹素算子不受影响（fbm/ridged/remap/mix ✓，warp 已合规 ✓） |
| 面朝向 | 小但真实：面序与**面内朝向**要对齐 wgpu 约定（我的面序已一致，面内朝向可用 `Skybox` 对拍一张验证） |
| 渲染器 | **大**：自写 surface material（cube 按方向采样 albedo/roughness/emissive + 光照），等价于把 feature 栈提前做掉 |

**教训（花钱买的）**：同一件事，**手工做**（图集 + per-face UV 记账）我翻车两次 ✗✗，
**让平台做**（标准 cubemap + 按方向采样）一次通过 ✓✓ ——
平台已经为某件事定义了标准格式时，不要手搓一个等价物。

---

## 36. 试 Bevy 内置大气：挂起，以及已经测出来的边界

按用户要求接了 Bevy 的散射大气（`Atmosphere` + `ScatteringMedium` + `AtmosphereSettings`），
代码留在 `--scatter earth` 后面（默认关 ✓）。**结论：挂起，壳式后端继续用。**

### 36.1 接口（已查实）

- `Atmosphere` 是 `bevy_light::Atmosphere`（**世界坐标球**，实体的 `GlobalTransform` 即球心；
  文档明说**用缩放在世界空间重定尺度**，`inner_radius`/`outer_radius` 单位是米）。
  它在 `on_add` 时若 `GlobalTransform` 仍是默认值，会把球心挪到原点下方 `inner_radius` 处
  （"站在行星上"的默认姿态 ✗ —— 轨道视角必须自己给变换）。
- `bevy_light::atmosphere::ScatteringMedium`（`earth(256,256)` / `mars(...)` / `from_curve(...)`；
  `Default` = `earth(256,256)`）。
- `bevy_pbr::AtmosphereSettings` **挂在相机上**（LUT 尺寸与采样数），"最近的大气"参与渲染。
- `AtmospherePlugin` **不在** `PbrPlugin` 里（显式添加会 panic："plugin was already added" ✗ ——
  实际上 `DefaultPlugins` 已经带了它 ✓，加两次直接炸）。

### 36.2 测出来的边界（四个数据点，都是哈希/截图）

| 配置 | 结果 |
|---|---|
| 默认（无 `AtmosphereSettings`，壳式大气） | **行星正常** ✓（与已知好图逐字节相同） |
| 只挂 `AtmosphereSettings`（没有 `Atmosphere` 实体） | **行星消失** ✗ 只剩星空 |
| `AtmosphereSettings` + `Atmosphere`（真实米制半径 × 缩放 1/6.36e6） | **整帧全黑** ✗ |
| 同上但半径改成场景单位（1.0 / 1.0125，缩放 1） | 与上一行**同一 hash** ✗ ⇒ **不是尺度问题** ✗ |

也就是说：**只要让大气真正参与渲染，我们的离屏场景就被清空** ✗，且与半径标定无关 ✓。

### 36.3 下次的便宜实验（按顺序）

1. **在窗口相机上试**（预览窗口是真窗口 ✓，`--serve` 走的是离屏 `RenderTarget::Image` ✗）——
   这一步能把"离屏路径的问题"与"功能本身的问题"分开 ✓，是最便宜的一刀。
2. 检查与 `disable::<WinitPlugin>()` + `ScheduleRunnerPlugin` 驱动方式的关系
   （大气插件往我们没跑的调度里加系统的可能性 ✗）。
3. 若都不行 ⇒ 走我们自己可控的那条：**自写 surface material 采样 `aerial_view_lut`**
   （`render_sky.wgsl` 已经证明这条路成立 ✓，它用深度纹理做 in-scattering + transmittance 合成 ✓）。

**当前状态**：壳式大气是可用后端 ✓；散射后端挂起 ✓；默认路径经复核对已知好图**无回归** ✓。

---

## 37. Shader 热重载：一直是没打开，不是不能用

用户问"不是说 shader 能热重载吗，为什么你老是关掉重开 viewer" ✓ —— 答案是我自己的工具链漏了：

- Bevy 的 `file_watcher` 是 **opt-in 特性** ✗，而 `Cargo.toml` 里只写了 `bevy = "0.19"`（默认特性不含它 ✗）
  ⇒ **热重载从来没启用过** ✗，我却把"改 shader 要重启"当成了常态 ✗（而且从没验证过 ✗）。
- 修法两步：`bevy = { version = "0.19", features = ["file_watcher"] }` ✓ +
  `AssetPlugin { watch_for_changes_override: Some(true), .. }` ✓。

**验证方式（不靠假设）** ✓：起一次 `--serve` ✓，拍一张记下盘面 RGB ✓，
**只改 `.wgsl`**（不重建 ✗ 不重启 ✗）✓，再拍同一处 ✓：

| | 盘面 RGB |
|---|---|
| 改前 | (59, 80, 111) |
| shader 里加 ×3 后（同进程） | (71, 91, 122) |
| 改回后（同进程） | (59, 80, 111) ✓ |

**从此的迭代规矩** ✓：改 shader ⇒ **不重建、不重启** ✓，直接再拍/看窗口 ✓。
只有改 **Rust** 时才需要重建 ✓（而重建前必须先结束 `px_render`，否则 exe 被占用 ✗）。


---

## 38. 交接（新 session 从这里开始）

### 38.1 现在能跑什么

```
cargo run -p px_graphs --bin planet        # 烘星球图（打印产物路径 + 可直接粘贴的渲染命令）
cargo run -p px_graphs --bin desert        # 沙漠图
px_render --serve                          # 常驻渲染服务（离屏出图，改 shader 不用重启它）
px_render --view                           # 持久预览窗口（用户 review 用；hotkey 见 §27）
px_render --planet <FIELD> --mesh <MESH> --palette rocky --out x.png
cargo test --workspace                     # 含离线 WGSL 校验
.\tools\probe.ps1 -Field <F> -Mesh <M> -Palette rocky -Out target\probe.png   # 12 角度对照图
```

### 38.2 已验证的工作流（不要再凭感觉 ✗）

- **改 shader ⇒ 不重建、不重启** ✓：`file_watcher` 已打开 ✓，同一进程内改 `.wgsl` 立即生效 ✓
  （实测：盘面 59,80,111 → 71,91,122 → 改回 59,80,111 ✓）。窗口和 `--serve` 都是这样 ✓。
- **改 Rust ⇒ 必须先结束 `px_render`** ✓，否则 `cargo build` 报 exe 被占用 ✗。
- **改完 shader 跑 `cargo test`** ✓：`px_render/tests/shaders.rs` 用 naga 离线校验所有 `.wgsl` ✓，
  两个测试保护校验器自身（喂类型错误 / 未知 import 必须被抓 ✓）。新增 Bevy import 时要给它加桩 ✓。
- **看细节，不看总览** ✓（用户明确要求）：开发阶段用**单行剖面 / 局部放大图**做判据 ✓，
  总览对照图只在收尾时看 ✓。`tools/probe.ps1` 是收尾用的 ✓。
- **`--serve` 的租约会让服务自杀** ✗：`watch_lease` 发现租约易主就 `AppExit::Success`（退出码 0 ✗）。
  **不要一边起服务一边删 `target/render-server.json`** ✓，否则你会以为是代码崩了 ✗。
- 服务刚起来时要**等预热完**（日志出现「渲染管线全部就绪 … 失败 0 条」）✓ 再出图 ✓。

### 38.3 血泪不变式（每条都付过代价）

1. **`AlphaMode::Add` 忽略片元 alpha** ✓ ⇒ shader 里必须**预乘**（`color * alpha` ✓）。
   不预乘 = 处处满强度加色 = 平直饱和色块 + 轮廓处**硬边** ✗ + 密度旋钮**完全无效** ✗✓。
2. **不要用引擎提供的逐片元"位置"** ✗：本 shader 现在只需要 `world_normal`（取掠射角 ✓）
   + `@builtin(position)` 的屏幕坐标（取深度 ✓）✓ —— 位置一概自己从相机与法线推 ✓。
3. **调试读数要按 sRGB 解码** ✗：目标缓冲是 sRGB ✓，我按线性读导致"三个本来正确的 uniform 看起来全错" ✗。
4. **换掉一个隐式默认值前先量出它到底是多少** ✓（Bevy 全局环境光是 80，我一度拍成 16 ✗）。
5. **子系统半坏时不要下"引擎不可信"的结论** ✗（§34 ④ 的更正 ✓）。

### 38.4 大气（当前实现）

`px_render/assets/shaders/atmosphere.wgsl`，加法混合的一个球壳，**不需要任何逐帧注入** ✓：

- 掠射角余弦 `cosine = |dot(N, V)|` ✓（N 是壳面法线 ✓，V 是"指向相机"✓）；
- 入射距离 `entry = distance(camera, surface)` ✓，`surface = N * outer` ✓；
- 终点 = `min(entry + 2*outer*cosine, 场景深度)` ✓ —— 深度来自 **`depth_prepass_texture`**
  （`DepthPrepass` 已挂在四台相机上 ✓）；深度为 0（天空 ✓）就用整条弦 ✓；
- ⇒ **`inner` 半径 shader 已完全不读** ✓ ⇒ 球体假设与半径比都**删掉了** ✓，地形山脊会被正确遮挡 ✓；
- 沿弦 5 个采样点，逐点判"该处空气是否被太阳照到" ✓ 并累积 ⇒ 晨昏线是**渐变** ✓，夜侧停在底光 ✓；
- 绕外缘一圈实测：`28 30 54 92 116 128 130 121 102 69 34 28` ✓（亚太阳侧最亮 ✓ 两侧连续衰减 ✓）。

### 38.5 已知未解 / 挂起

- 把相机字段与 `sync_cameras` **从 Rust 里删掉** ✗ 会让 **`--serve` 在预热途中退出** ✗
  （可复现 ✓、退出码 0 ✓、无 panic ✓；回退即恢复 ✓）。现在那些字段是"写了没人读"的状态 ✓。
  **这是一个独立 bug，值得查** ✓。
- **Bevy 内置散射大气：挂起** ✗ —— 一试就让我们的离屏场景整帧清空 ✓，且与半径标定无关 ✓（§36）。
  它的默认姿态是"站在行星上" ✓，与轨道视角不合 ✓。它读场景深度做**空气透视** ✓ —— 那条能力
  我们已用自己的方式补上了 ✓（38.4）。
- **B 档（行星也走标准 cubemap）**：需要自写 surface material ✗，代价全在渲染器不在 PCG ✓（§35）。

### 38.6 下一步：体积云（方案已定，未开工）

- **位置**：球壳 1.01–1.06 ✓（地面最高点 1.036 ✓、大气顶 1.14 ✓）—— 正好"在大气与地面之间" ✓。
- **密度来源（用户裁决）**：**PCG 烘基础形状 + shader 补细节** ✓ ——
  64³ 基础体积受图控制 ✓ 可缓存 ✓ 可截图评审 ✓；细节在 shader 里加 ✓。
  行星地表之后也照此办理 ✓ ⇒ **这部分 shader 要写成通用的** ✓（用户原话：做好 shader 的 project management ✓）。
- **渲染**：主 raymarch 48–64 步 ✓ + 每步向太阳 6 步求自阴影 ✓（立体感全靠它 ✓）+ Beer–Lambert ✓。
- **协议代价很小** ✓：`BlobHeader.shape` 本来就是 `Vec<u32>` ✓ ⇒ 加一个 3D 形状的 kind 即可 ✓。
- **前置已就位** ✓：`DepthPrepass` 已在跑 ✓（云要知道地面多远 ✓）、`common.wgsl` 共享库已在 ✓、
  热重载工作流已通 ✓。
- **风格**：先做**地球型积云** ✓（覆盖度掩码 + 高度剖面）。
- 顺带一条遗留 ✓：`px_graphs/src/bin/volume_probe.rs` 是当初"烘体"路线的探针 ✓，
  用户已裁决**不烘体** ✗（§25）⇒ 它现在是**死代码** ✗，做云的时候顺手处理 ✓。

---

## 39. 阶段 1–2 完成记录：云覆盖度图 + 体积云（2026-09-13）

用户裁决把 §38.6 的方案改了，而且改得更好：

> *「云直接用一张 cubemap，高度上的分布和细节用 shader 提供」*

**这一句解掉了 §38.6 里没被发现的坑** ✓：64³ 体积铺满 `[-1.1,1.1]³` 时体素边长 0.0344，
而云带只有 0.05 厚 ⇒ **竖着只有 1.45 个体素** ✗，PCG 表达不了云顶/云底。
高度交给 shader 之后，**烘出来那张图不需要垂直分辨率** ✓ ⇒ 既不用 `Volume3D`、
也不用 `texture_cube_array`，连 §24.4 预判的"把算子泛化到 3D"都免了 ✓。

第二句裁决：地面云影走**自写 surface material**（"自写，反正也要换 cubemap"）✓ —— 落在阶段 3/4。

### 39.1 现在能跑什么

```
cargo run -p px_graphs --bin clouds            # 烘云覆盖度图（256²×6，冷 ~1.5 s，缓存 ~0.3 s）
px_render --serve                              # 常驻渲染服务
px_render --planet <H.pxart> --mesh <M.pxart> --palette rocky --clouds <C.pxart> --out x.png
px_render ... --cloud <K>                      # 消光倍率，默认 1（= 900）
px_render ... --cam yaw,pitch,dist             # 局部放大判据（开发期主力，§38.2 那条"看细节"）
cargo test --workspace                         # 全绿（含 §38 交接时那条红的快照测试）
```

### 39.2 `Domain::CubeMap`：六面沿行堆叠

- **布局**：`width = 面边长`、`height = 6 × 面边长`，行主序 ⇒ **与 wgpu 的 cube 层布局逐字节一致** ✓，
  上传时不用重排 ✓（`clouds::coverage_image` 只做 f32 → f16）。
- **面朝向：代价 0** ✓。仓里原有的 `cube_direction` / `cube_face_of` **本来就已经是**
  wgpu 的约定（面序 +X −X +Y −Y +Z −Z，面内朝向也一致）✓ —— §35 代价表里"面朝向：小但真实"
  那一格实际没花钱 ✓。
- **验证方式（值得照抄）**：把覆盖度**直接当颜色输出**（`return vec4(vec3(mask), 1.0);`），
  热重载看一眼：整球图案连续、六面缝合处无痕 ⇒ 约定端到端正确 ✓。
  总览图看不出这个，**这个判据是唯一能证明它的** ✓。
- `px_ops` 侧 `sample_direction` 走**跨面双线性** ✓（面的四条边是别人的纹素，
  通用 uv 路径会 clamp）。两条连续性测试让一张光滑解析场跨 +X/+Z 与 +X/+Y 棱走一遍：
  正确实现最大跳变 < 0.02，clamp 实现是 **0.43** ✗。

### 39.3 血泪四条（新的）

1. **`#define_import_path` 的模块必须被"加载"** ✗✓。`noise.wgsl` 声明了 import path
   但**从没被当成资产加载过** ⇒ 每个 import 它的管线都报
   `Shader import not yet available` ⇒ **Bevy 会一直重试**（`pipeline_cache.rs:688` 把它重新入队）
   ⇒ 永久失败且**不报错到日志** ✗。`atmosphere.rs` 里那个"写了没人读"的 `ShaderLibrary`
   其实就是在干这件事 ✓ —— 只服务 `common.wgsl` 一个库。
   现在 `ShaderLibraryPlugin` 扫 asset 目录、**凡声明 import path 的一律加载** ✓，这类 bug 不会再犯。
2. **就绪判据不能把"失败"当"完成"** ✗✓。旧 `watch_pipelines` 里 `Err` 既不算 pending 也不算失败
   ⇒ 管线编译失败时照样宣布"全部就绪" ⇒ 出一张**少了那个材质的"成功"图** ✗。
   **症状特征**：一整轮参数扫描的 PNG **字节数完全相同** ✗（我第一次就撞上了，
   还把它误读成"云太薄"）。现在失败即不就绪，并把**管线名 + 错误原文**打出来 ✓ ——
   一行就定位了上面那条 ✓。
3. **`--serve` 原本没有"空闲"这个状态** ✗ —— 不管有没有人点图，它都让离屏相机**全速一直画** ✗。
   场景便宜时这不花什么代价，谁也没注意到；换成体积云之后 GPU 被持续占住，
   **空闲的服务器在 20–120 s 内必然掉设备** ✗（DX12 `DXGI_ERROR_DEVICE_REMOVED` = `0x887A0005`，
   日志里先看到的是下游症状：`Command allocator creation failed`、
   `clustering metadata staging buffer is invalid`）。
   A/B 实测把这条钉死：同一台服务、同一颗行星、**只去掉云**，空闲 120 s **零掉设备** ✓；
   带云的每次必掉 ✗。
   修法：`idle_between_jobs` 没活时把相机 `is_active` 关掉 ✓，有活了再打开、照样留 6 帧预热 ✓。
   修完实测：**空闲 240 s ✓ ＋ 连打 30 张图全成 ✓，零掉设备** ✓，且出图**逐字节不变** ✓。
   ⚠ **更正（同一天，我自己先记错了）**：我一度把"连续热重载之后每张图涨到 5–7 s"记成热重载的锅 ✗ ——
   那是**同一个劣化过程的前兆**（先变慢、后掉设备），热重载只是恰好在那几次里同时发生 ✗。
   §38.2 那句"改 shader ⇒ 不重建不重启"本身没错 ✓，错的是我把别的东西记到了它头上 ✓。
4. **怀疑"shader 被内联成一坨巨大字节码"⇒ 能量的就别猜** ✓。用户提的这条，值得单独记：
   `tests/shaders.rs` 现在把每个入口 shader 过一遍 **naga 的 HLSL 后端并报体量** ✓。
   实测 `clouds.wgsl` → **503 行 HLSL**（对照 `atmosphere.wgsl` 182 行）✓；
   naga 把 `gradient_noise_3` 输出成**真函数 + 真循环** ✓，两个 march 循环的边界是 uniform
   ⇒ **驱动也展不开** ✓。**这条怀疑不成立** ✓，但守卫留在测试里（涨一个数量级就失败）✓。
   ⇒ **记法**：WGSL 的内联确实是按调用点复制的 ✓，所以"循环体里塞几个噪声函数"这件事
   值得量 ✗ —— 只是这次量出来没那么大 ✓。

### 39.4 价格（都在 RTX 3060 Laptop / DX12 上实测）

| | 每张图 |
|---|---|
| 无云 | **375 ms** |
| 云（56 主步 + 6 太阳步，3 阶 fbm） | **396 ms** |
| 云（塔状噪声，5 阶 fbm） | ~600 ms |
| §25.3 记的旧热路径 | ~1.0 s（含烘图） |

⇒ 云**没有**打破"每次 ~0.3 s"这条工作流 ✓。烘云图冷 1.5 s、命中缓存 0.3 s ✓。
⚠ 上面这些数**只能在"服务刚起来、且没在掉设备的路上"时测** ✗ —— 劣化之后同一个场景会变成 5–7 s ✗（见 §39.3 第 3 条）。

### 39.5 已测出的边界（用户已知情并选择保留）

- **云带 1.01–1.06 与地形打架** ✓（§38.6 原样保留，用户裁决）：
  地形最高点 **1.036** ⇒ 陆地高于 1.01 的地方，云带**下半截被地表截掉**
  （`scene_distance` 把射线切在地形上 ✓，行为正确，但云底埋在山里）。
  画面上表现为"云在山尖上被削平" ✓，海洋/低地上没有这个问题 ✓。要改只需动 `CLOUD_BASE`。
- **相位与多散射**：单次散射 + 太阳在相机背后时云是灰的 ✗（背散射几何）。
  现在用**三阶透射率近似**（`exp(-τ)`、`exp(-τ/2)`、`exp(-τ/4)` 加权）✓
  并把 HG 相位归一化到前向 = 1、与各向同性按 0.4/0.6 混合 ✓ ⇒ 云终于是白的 ✓。
- **云只在边缘浓**：这是**薄壳的正确光学** ✓（天底路径 ~0.05，掠射路径 ~0.64，差 13 倍），
  不是 bug。消光取 900 让天底也能压到不透 ✓。

### 39.6 下一步（阶段 3/4）

- **阶段 3**：行星地表迁标准 cubemap + 自写 surface material（§35 的 B 档）。
  ⚠ 判据要换：哈希**必然**变，"对已知好图无回归"这条不再适用，得改成并排 + 差异带。
- **阶段 4**：云影落进自写材质（只作用直接光）；把壳内介质 raymarch 提炼成通用库
  （`clouds.wgsl` 里的 `cloud_density` 是唯一一份密度实现，云影复用它 ✓）。

---

## 40. 用户 review 回路：做完一个功能**必须**把窗口调出来

用户在 §27 就要求过：*「让 render 可以用命令开一个交互式的 viewer 窗口，
这样你做完一个效果可以调出这个窗口让我 review」* ✓。
**这是一条工作流义务，不是可选项** ✓。我在做完体积云之后漏了它，用户的原话是：

> *「文档里没写完成一个功能之后起 viewer/update viewer 让我 review 吗」*

漏的原因很具体，值得记：我读 §38 交接时只看见 `px_render --view` 这一行命令 ✓，
**没读 §27** ✗ —— 而 §27 才是那条命令的用法与坑之所在 ✗。
⇒ **交接文档的"能跑什么"清单不等于用法说明** ✓；清单里出现的每条命令，
都要回到它自己的那一节去读 ✓。

### 40.1 两步；第二步可以反复做

```
px_render --view --planet <H.pxart> --mesh <M.pxart> [--clouds <C.pxart>] --palette rocky
px_render --show --planet <…> [--clouds <…>] --palette rocky [--shot]      # 推新场景
```

- 窗口是**常驻**的 ✓：只开一次，之后每次都只用 `--show` **推** ✓，不要每次重开（§27.1）。
- `--show` 写 `target/viewer-scene.json`，窗口每帧轮询 ✓；
  窗口还会盯**场文件的 mtime** ✓ ⇒ **重烘之后不用推，窗口自己就更新了** ✓。
- `--show` 会报"窗口在线 / 没在跑" ✓（读 `target/viewer.json` 心跳）。
- **`--shot` 让窗口自己存一张 `target/viewer-shot.png`** ✓ ——
  这是**我自己验收的判据** ✓：不用真的看屏幕，就知道窗口里画的是什么 ✓。
  窗口截图要等管线就绪（§27.2 ①）。

### 40.2 起窗口必须"脱离"，否则 agent 会被它卡死

用户原话：*「你为啥不把它作为 hidden？不然 viewer 跑起来了你不就停止工作了吗」* ✓ —— 对。
用一个**永不退出**的 GUI 进程当后台作业去等 = 把自己挂住 ✗。
（我这一轮就是这么挂的：`run_in_background` 起窗口、再去 `wait`，直接被 abort ✗。）

正确起法（PowerShell，**不要**加 `-WindowStyle Hidden`，理由见 §27.2 ②）：

```powershell
$p = Start-Process -FilePath target\debug\px_render.exe `
     -ArgumentList @("--view","--planet",$P,"--mesh",$M,"--clouds",$C,"--palette","rocky") `
     -RedirectStandardOutput target/viewer.log -RedirectStandardError target/viewer.err -PassThru
```

`Start-Process` **不带 `-Wait`** 立刻返回 ✓，窗口留在用户桌面上，agent 继续干活 ✓。
日志重定向到 `target/viewer.log` ✓ —— 判据（`云层：…`、`渲染管线全部就绪…失败 0 条`）都在里面。

### 40.3 一个静默陷阱（已修）

`view()` 原本**优先读 `target/viewer-scene.json`**，于是命令行给的场景被**静默忽略** ✗。
我传了 `--clouds` 却看到"没看到云"，查日志才发现窗口用的是**上一个 session 留下的请求文件**
—— 那份 JSON 里没有 `clouds` 字段 ⇒ `None` ✗。
**这与 §1.2 第 4 条"参数填了不生效"是同一个病** ✓：填了、不报错、不生效 ✗。

现在：**命令行给了 `--planet` 就以命令行为准** ✓，没给才回退到请求文件 ✓
（保留"重开窗口续上上次场景"这个原意 ✓）。

⚠ 顺带一条还没解释的观察：窗口里的**整体曝光比 `--serve` 出图暗**（实测均色
`{41,47,53}` → `{32,33,35}`，蓝通道掉得最多）✗，两者 `AmbientLight` 都是 `DEFAULT_AMBIENT` ✓、
大气壳日志也一致 ✓。**没查出来，先记着** ✗。

---

## 41. 怎么量单帧时间（`--fps`），以及一个会骗人的测量

用户问：*「你能测出单帧时间吗」* ✓。能，但要先去掉两道帧率上限，否则量到的是上限不是成本 ✗：
`--serve` 的 `run_loop(1/60)` 会**睡觉**凑 60 Hz ✗，窗口默认 **vsync** ✗。

```
px_render --serve --fps --width W --height H      # 服务端不再限速
px_render --view  --fps  --planet … --clouds …    # 窗口 AutoNoVsync
```

`--fps` 每 **120 帧**打一行平均帧时间 ✓（`FRAME_PROBE_WINDOW`）。
⚠ `--fps` 下 `idle_between_jobs` **不关相机** ✓ —— 不然后台渲染只持续 6 帧，根本凑不满 120 帧 ✗。

### 41.1 会骗人的那一半：低分辨率量到的是"地板"，不是 shader

**实测（RTX 3060 Laptop / DX12，行星 + 大气 + 云）：**

| 分辨率 | 像素 | 无云 | 有云 | 云的成本 |
|---|---|---|---|---|
| 320×200 | 6.4 万 | 13.87 ms | 13.65 ms | **量不到** |
| 960×640 | 61 万 | 13.5 ms | 13.6 ms | **量不到** |
| 2240×1400 | 314 万 | 15.29 ms | 24.34 ms | **9.05 ms** |

⇒ 这里有一条**与分辨率无关的 ~13.5 ms 地板** ✓（307k 三角形网格 + 两个壳 + 预通道的
顶点/绘制成本，换分辨率不变）。**只要 shader 的成本低于地板，帧时间就一动不动** ✗ ——
"帧时间没变"**不等于**"shader 免费" ✗。我在 960×640 上量到 13.5 → 13.6 ms，
一度把这个读成"云是免费的" ✗，**那是错的** ✓：真正卡的是**窗口**（314 万像素），
那里云要 9 ms，整帧 24 ms ✓。

**记法**：量 shader 一定要在**GPU 真的成为瓶颈**的分辨率上量 ✓，
或者扫一遍分辨率找到地板在哪里 ✓。

### 41.2 优化前后（同一个仪器，2240×1400）

| | 整帧 | 云单独 |
|---|---|---|
| 无云 | 15.29 ms（65 fps） | — |
| 优化前 shader | **35.17 ms（28 fps）** | **19.9 ms** |
| 优化后 shader | **24.34 ms（41 fps）** | **9.05 ms** |

真实窗口（2240×1400 物理像素）：无云 16.8–17.1 ms，有云 31–37 ms ✓。

⚠ **更正我自己**：perf 那一条提交里我按**算子计数**说"干了 7 倍的活" ✓，
但**实测只快 2.2 倍**（19.9 → 9.05 ms）✗。
原因大概是：算子砍掉之后 shader 从**吞吐受限**变成**延迟受限**（依赖式纹理采样、
分支发散），而且提前退出/自适应步数本来就让不少像素提前走了 ✓。
⇒ **算子计数只能用来找嫌疑人，不能当成绩** ✓ —— 成绩要用 §41.1 的仪器量 ✓。

---

## 42. shader 热重载到底是好的 —— 我白重启了十几次

用户问：*「现在究竟能不能 shader hot reload? 为啥你有重启服务？」* ✓。
**能** ✓。实测（把 `clouds.wgsl` 故意写坏一行、**不碰任何 Rust、不重编**）：

```
INFO  bevy_asset::server:                Reloaded shaders\clouds.wgsl
ERROR bevy_render::...::pipeline_cache:  failed to process shader error: shaders/clouds.wgsl:332:25
```

**1 秒内重载并重编** ✓；文件恢复后自己又好 ✓。serve 和 viewer 两个进程都验过 ✓。

### 42.1 什么时候**必须**重启

| 改了什么 | 要不要重启 |
|---|---|
| **只有 `.wgsl`** | **不要** ✗ 存盘即可，约 1 秒生效 |
| `CloudParams` 之类的 **uniform 结构** | **要** ✓ 布局变了 ⇒ 必须 `cargo build` ⇒ exe 被占用 ⇒ 得先停进程 |
| CLI / 系统 / 插件 | **要** ✓ 同上 |

我这一串重启里，**改 uniform 的那几次是必须的**（`slope_scale`、`taper`、`coverage_gain`、`ablate` 都是加字段），
但**纯 WGSL 的那几次（比如法线翻符号）完全没必要** ✗ ——
代价是每次多 15-25 秒预热 ✓，而且**每次都把用户的窗口杀掉** ✗，
所以用户连着问了好几次"你倒是起 viewer 啊" ✓。**这是我的流程错，不是工具不行。**

### 42.2 两条要一起记的教训

**① 证据要看全，不能只看尾部。** ✗
我第一次测热重载时用了 `Select-Object -Last 12`，而窗口的 resize 报错刚好排在新增日志的后面，
把 shader 报错挤出了视野 ⇒ 我据此说了句"**没被拾取**" ✗ —— **完全说反了** ✓。
和 §41.1 那次"在错的分辨率上取证"是同一类错：**取样方式决定了你会得出什么结论。**

**② §38.2 那条"反复热重载同一个材质 shader 可疑"应当作废** ✓。
那个怀疑来自 DeviceLost 调查，而 DeviceLost 后来定到了**空闲还在满速渲染**上（§39.4），
跟热重载没关系 ✗。**当时"保险起见重启"的做法，被我继承成了习惯，白付了代价。**

---

## 43. 云密度场的**正确**梯度组装（调研结论，务必照抄）

子 agent 调研 + **实测**得出 ✓。我原先写下的形式：

```
∇ρ = (∂ρ/∂r)·r̂ + (1/r)·∇_d ρ          ✗ 只有在"∂ρ/∂r 是固定 d 的真偏导、
                                           且 ∇_d ρ 已经是切向"时才对
```

**完整且经验证的形式**（用 shader 里的名字，`along = across·span()`，所以 `along/span = across`）：

```
P_t = I − d⊗dᵀ                                    // 切向投影算子

∇_p ρ = (ρ_a / span) · d                                              // ① 高度
      + ρ_n · [ (s/r)·P_t·g_q  +  (along/span)·(d·g_q)·d ]            // ② 噪声
      + (ρ_cover / r) · P_t · g_cover                                 // ③ 覆盖度贴图
```

⚠ **②中方括号里第二项是最容易漏的，而且漏了是灾难性的** —— 因为 `q` 通过 `a` 依赖 `r`，
**噪声有一个"超出 ∂ρ/∂a"的径向依赖**。实测（20000 个壳内随机点，对比三维中心差分）：

| 组装方式 | 中位数 | 最大 |
|---|---|---|
| **正确** | 1.3e-07 | 2.0e-05 ✓ |
| 漏掉 `P_t` 投影 | 3.0e-01 | 6.2 ✗ 百分比级错 |
| **漏掉 `(along/span)(d·g_q)d`** | **7.0e+00** | **9.7e+01 ✗✗ 灾难** |
| 往 `g_cover` 里加 3.7·d 的径向垃圾 | 与正确**逐位相同** ✓ | ← `P_t` 把它吃掉了 |

⇒ **③ 只需要 `g_cover` 的切向部分** ✓ —— 所以**那张烘焙图正好就是答案**：
一次 `textureSampleLevel` 同时拿到 mask 和梯度，再 `P_t` 投影 ✓✓。
用户那句"梯度不是烘焙了吗，只需要一次采样"**完全成立** ✓。

⚠ 但必须确认 `gba` 是**喂给 `coverage_of` 的那个标量**的梯度（`.r` 的 mask）。
现在 R=`mixed`、GBA=`∇mixed` ✓ 一致；但 `coverage_of` 之上还套了
`smoothstep(0,0.45, clamp(...))`，所以 `∂cover/∂mask = 6t(1-t)/0.45 · 1/max(1-coverage,1e-4) · [0<t<1]` ✓。

### 43.1 分段点的处理约定（不是障碍，是约定）

`smoothstep` 的导数在两端**自己归零**（实测 `s'(0)=0`、`s'(1)=0`），
所以 CAS 输出里那堆 Piecewise/Heaviside **会塌缩成一串 `select(...)`** ✓ —— 12 行就能转写完：

```wgsl
let t1 = clamp(a / base, 0.0, 1.0);
let ds1 = select(0.0, 6.0 * t1 * (1.0 - t1) / base, t1 > 0.0 && t1 < 1.0);
let dc_dn = select(0.0, top * detail_strength, m > base + 0.02);
```

⚠ 需要显式决定的：**解析梯度要不要在"值被强制为 0"的地方也返回 0** ✓。
现在的 FD 版本在**壳边界会返回巨大的差商** ✗，而 `ABLATE_NORMALS` **正是靠这个尖峰找表面的** ✗ ——
换解析版会改变那里的行为，要有意识地选，而不是让它悄悄变。

### 43.2 工具选型（已调研，不要再重新查）

| 工具 | 结论 |
|---|---|
| **`num-dual`**（MIT OR Apache，活跃） | ✓ **用它**。做 dev-dependency：把 `shape_of` + 噪声泛型化到 `D: DualNum<f64>` 上，得到 **f64 精度的 ∇ρ 参考** —— **这才是能长期复用的产物** ✓ |
| `symbolica` | ✗ **不要**。源码可见但**非开源**：受雇使用需付费 **EUR 3000/年/开发机**，CI 单独报价 |
| `egg` | ✗ 它是 e-graph 重写库，**你不写规则它就不会求导** —— 等于手推 |
| `rust-gpu` / `cubecl` | ✗ 前者只出 SPIR-V；后者是 compute DSL，等于把场**写第三遍** |
| `naga-rust` | 只能当"无 GPU 时执行"的备选，作者自己说"expect compilation failures" |

⚠ **`num-dual` 的分段语义是它的卖点**：其源码注释明确
*"Comparisons are only made based on the real part. This allows the code to follow
the same execution path as real-valued code would."* ⇒ 分支与真值代码一致 ✓。
但它**没有 `floor`/`min`/`max`/`clamp`/`abs`**（`Dual` 甚至不实现 `num_traits::Float`），
要自己写 ~6 个泛型小工具 ✓ —— **这是好事**：分支语义变成显式可审的。

