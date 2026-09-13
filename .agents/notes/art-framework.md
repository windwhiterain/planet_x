# 美术框架：agent 截图友好 / 热重载 shader / 热重载程序化生成管线

> 状态 `[ ]` —— **只有调研与裁决点，一行实现都没写**。
> ⚠️ **2026-09-13 用户裁决：走 Rust 栈**，sim 与 renderer 之间用**协议 crate** 规定数据格式、
> 两边都只依赖它 ⇒ 见 **§10**（§2.2 TS 实测 / §2.3 QuickJS / §4.5 tier-1 Node 侧车**已被取代**，
> 三条仍然有效的结论已在 §10.4 列出）。
> 拟落地分支：`feature/art-gen` off **`feature/glsl-files`**（美术工作真正所在的分支；
> ⚠️ 当前 checkout 的 `v2` 与它**没有共同祖先**（orphan 血缘），合不过来）。
> ⚠️ **本文这一份是在 `v2` worktree 里写的**（该分支没有 `web/`、没有 `.agents/`），
> 落地时整份移到美术 worktree，别在 `v2` 上提交。
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
