# 动态 schema 与动态 render graph：调研

> 起因（2026-09-16，用户转述美术的要求）：**① shader / material 的 schema 要能动态改变（像 Unity Shader Graph 那样）；② render graph 也要能动态组装、重载。**
>
> 这一篇只干两件事：**把「今天已经动到哪儿」钉在证据上**，以及**把候选形态与代价摆出来**。
> 代码未动。§ 号接 `08-renderer.md`（§66 是同一件事的前一次调研：只到「参数块」，没到「render graph」）。

---

## §67 先钉事实：今天哪些已经是动态的

这一节全部来自源码，逐条给 `文件:行`。**先钉住这一半，是因为「要求动态」的直觉里有一多半是过期的**
—— §65（通用渲染）之后，渲染器侧对 schema 的约束已经只剩「绑定格集合」那一层。

### §67.1 已经是动态的（渲染器侧）

| 事实 | 证据 |
|---|---|
| 参数块的**大小与内容逐材质任意**：布局上 `min_binding_size: None`，每个材质各自一块 buffer | `px_render/src/material.rs:104-113`（注释原文「每个 shader 的结构体大小不同，而布局只有一份」）+ `:144-153` |
| 参数的**名字 → 字节偏移/类型**由那份 WGSL 反射出来（naga），Rust 侧没有第二张表 | `px_render/src/reflect.rs:294-426`；`write_value` 在 `:192-248` |
| 反射结果按 `(内容版本, 库指纹)` 缓存 —— 键 = 内容，同一版永远同一份契约 | `reflect.rs:456-475` |
| 换一份 WGSL = 换一条（套）管线：管线特化键是 `{shader 版本, 剔除档}` | `material.rs:34-38`、`:228-248` |
| 一份 WGSL 一个内容版本，最多同时养 4 版（LRU 淘汰，淘汰即释放资产 ⇒ Bevy 连带删管线） | `px_render/src/slots.rs:24`、`:131-161` |
| 装载时**不 reload**：新版本 = 新路径 = 新资产 ⇒ 旧版本的管线原封不动（A→B→A 是缓存命中） | `slots.rs:117-131` 的注释与实现 |
| 场景文档 v2 里已经**没有**「行星 / 云 / 大气」：物体 = 几何 + 材质 + 变换，材质 = shader 成员 + 按名字的参数 + 按绑定下标的贴图 + 三条渲染状态 | `px_protocol/src/scene.rs:307-324`、`:370-382` |
| 文档不认识的 kind 已经不存在了（`KINDS` / `assembler` / `PlanetSpec` 全删） | `08-renderer.md` §65 的搬走清单 |

**⚠ 一条过期笔记**：`09-instruments.md` §42.1 那张表写着「改 `CloudParams` 之类的 uniform 结构 ⇒ 必须重启」。
那是 §65 之前的形状（当时参数块是 Rust 的 `#[derive(ShaderType)]` 结构体）。**按代码读法**，今天改 WGSL 里的结构体
不需要重编 Rust：反射在装载时读那份文本（`reflect.rs:456-475`），绑定布局是固定超集且 `min_binding_size: None`
（`material.rs:104-113`），参数缓冲逐材质现建（`:144-153`）。
⚠ **但这是读代码得出的，不是实测**（本仓规矩：没跑过不算验过）⇒ 谁要动这一块，先补一次实测：
**改 WGSL 结构体 → 重烘 `shaders` + `scene` → 出图，全程不重编 exe**；再把 `09-instruments.md` §42.1 那张表改掉。

### §67.2 真正被钉死的（硬边界）

| 层 | 能不能动态 | 证据 |
|---|---|---|
| 绑定格**集合**（组号、格号、维度） | 不能：同一材质类型只有一份布局 | `bevy_render-0.19.1/src/render_resource/bind_group.rs:609`（`bind_group_layout_entries` 是**静态**方法） |
| 贴图**格数上限** | 不能：固定超集 —— 4 张（2D/2D/cube/cube，各带采样器） | `px_render/src/reflect.rs:22-30`、`material.rs:98-133` |
| 材质绑定组在**第几组** | 不能：Bevy 写死第 3 组 | `bevy_pbr-0.19.1/src/material.rs:466-486` |
| 参数块**字节上限** | 不能：`MAX_PARAMS_BYTES = 1024`（16 对齐） | `reflect.rs:31-34`、`:344-349` |
| 运行期问驱动「这程序有哪些 uniform」 | 没有这条路（WebGPU 明确放弃） | §66.1 已查实（[gpuweb#2470](https://github.com/gpuweb/gpuweb/issues/2470)） |
| 管线结构（有哪些 pass、谁读谁写） | **今天完全没有这条路**：内容只决定「画哪些物体」，pass 结构是 Bevy 的 `Node3d` | `px_render/src/main.rs:1729-1767`（相机 + `Camera3d`）、`:362-367`（GPU 时间戳只有 Bevy 那几条 pass） |
| 渲染目标格式 | 固定 `Rgba8UnormSrgb`（**LDR、无 HDR、`Msaa::Off`**） | `main.rs:1460-1464`、`:1487`、`:1734` |

### §67.3 挡住「美术自助」的其实在**烘图侧**，不在渲染器

把「加一个参数 / 加一种材质」今天要走的路摊开（每一步都要人）：

| 想做的事 | 今天必须改什么 | 要不要重编 Rust |
|---|---|---|
| 改一个已有参数的值 | `art/scene/<名>.toml` 的 `params` | 不要 |
| **加一个参数**（WGSL 结构体里加一格） | ① `art/shaders/*.wgsl` 加字段；② 想让它从配方里可写，还要改 `px_graphs/src/bin/scene.rs` 的 `PLANET_KEYS` / `CLOUDS_KEYS` / `ATMOSPHERE_KEYS` 白名单与映射函数；③ 重烘 `shaders` + `scene` | **要**（②） |
| **加一种材质 / 一种物体** | ① 写一份新 WGSL；② 把它加进 `px_graphs/src/bin/shaders.rs:8` 的 `SLOTS` 常量数组；③ 在 `scene.rs` 里给它写一份「配方词汇 → shader 词汇」的映射 | **要**（②③） |
| 换一种渲染结构（加 pass / 改 pass 顺序 / 后处理） | 没有这条路 | — |

证据：`px_graphs/src/bin/shaders.rs:8`（`const SLOTS: [&str; 3]`）、`px_graphs/src/bin/scene.rs:442-487`
（三张参数名白名单）、`:517-586`（`cloud_params`：把配方的 `extinction` 映射成 shader 的 `density` 之类）、
`:161-173`（`check_keys`：不认识的参数名当场报错 —— 这条本身是对的，代价是白名单必须手维护）。

**所以「schema 动态」的真实缺口是三条**：
1. **配方词汇与 shader 词汇之间那份手写的映射**（烘图侧 Rust）—— 加一个参数就要动它；
2. **schema 在烘图侧不可见** ⇒ 只能盲写名字，错了在装载时才报（§66.3 形态 1 的代价）；
3. **一份 shader 只能是一个槽里的一段手写 WGSL** ⇒ 没有「组合」这件事：想加一点效果就得进那份 1600 行的文件里改。

### §67.4 schema 的真相源今天有几份（这条决定「动态」的代价）

| 位置 | 它说了什么 |
|---|---|
| `px_render/src/reflect.rs:22-30` | 真源：组号、第 0 格 uniform、贴图只占 1/3/5/7 及各格维度 |
| `px_render/src/slots.rs:167-187` | 占位 WGSL 手抄了同一张表 |
| `px_render/src/shaders.rs:254` | 离线组装把 `#{MATERIAL_BIND_GROUP}` 替成**字面量** `"2"`（运行期 Bevy 给的是 3） |
| `px_protocol/src/scene.rs:262-284` | 半份：`TextureRef` 注释 + `check()` 只查「奇数格、≥1」 |
| `Value`（4 种写法）vs `ParamKind`（5 种类型） | 同一份类型词表的两半，对应关系只活在 `write_value` 的 match 里 |
| `px_graphs/src/bin/scene.rs:442-487` | **配方侧**的第二份词表（`PLANET_KEYS`/`CLOUDS_KEYS`/`ATMOSPHERE_KEYS`） |
| `px_probe/src/params.rs:19-45` | 探针侧的 `CloudParams` 镜像（判据的输入） |
| `px_render/src/reflect.rs:499-527` | 测试把**具体偏移**钉死（`tint@16`、`inner@32`、`params_bytes == 128`） |

⇒ 任何「让 schema 变成数据」的方案，都必须先回答：**这几份里哪些合成一份、哪些留**。
（§66.2 记的是五份；这一轮补上最后三行。）

---

## §68 一条必须先说清的对照：Unity 的 schema 是**编译期**的

这一节的结论直接改变「像 Shader Graph 那样」这句话的解释，所以放在候选形态前面。

- Shader Graph 的 Blackboard 属性在**编译期**烘进生成的 shader 与材质序列化数据里：**每个属性都占材质的体积，
  不管暴不暴露**；材质是 schema 的**快照**，改图之后已有材质**不会**自动同步（官方原文见 §71 证据表）。
- 运行时（player）**不能**改 schema：HDRP 文档明说换 surface option 只在编辑模式可行。
- 因此 Unity 那条路上的「动态」只到**值**这一层；**schema 本身是作者期的产物**。

⇒ **本仓今天已经比 Unity 更动态**：`slots.rs` 在运行期装一份新 WGSL、Bevy 在运行期编出新管线，
参数块大小逐材质任意（§67.1）。所以「要求 schema 能动态改变」如果指的是**运行期**，
那么**这一半已经做完了**；真正缺的是**作者期**的三件事：

1. **schema 在烘图侧不可见** ⇒ 只能盲写名字（§67.3）；
2. **配方词汇与 shader 词汇之间那份手写映射**（§67.3 第 1 条）；
3. **没有「组合」这一层**：一份材质 = 一份手写 WGSL，想叠一点效果就得进那份文件里改。

**这三件事与「运行期能不能改」无关**，它们是**工具层与数据层**的缺口。把这句话钉在这里，
是因为「像 Shader Graph」这个类比很容易把预算引到「运行期热插拔 shader」上去 —— 那条路已经通了。

---

## §69 候选形态（schema 轴）

按「代价 / 拿到什么」排。**S1 与 S2 是任何一条路的前置**，S3/S4 才是「图」。

### S1｜契约收口（§66 的 P1，不变语义）

把绑定契约表 + `ParamKind` + `Value↔kind` 合法映射收进**一份**（`px_protocol` 里放**类型**，
naga 反射放**叶子 crate**：`px_protocol` 的运行时依赖被门钉死只有 `serde` + `serde_json`，
见 `px_protocol/tests/crate_graph.rs:4`），§67.4 那八处手抄改成一处；schema descriptor 落进 shader 产物；
装载时按 `closure_check` 同款对账。

- **代价**：小（纯搬运 + 快照/键不动语义）。
- **收益**：后面每一条路都要用它；顺手灭掉 `#{MATERIAL_BIND_GROUP}` 的 2/3 那颗雷。
- **判据**：所有产物键**逐字节不变**（只搬代码不改语义）；现有 5 条离线门 + `orbit-*` 出图哈希不变。

### S2｜schema 由 shader 声明 ＋ 配方**透传**（0 编译加参数）

烘图侧也跑一次反射（`px_graphs` 引 naga —— 已在 lock 里），于是：

- 配方里的 `params` 不再过手写映射，**按名字原样透传**，由 descriptor 在**烘图时**校验缺参/多参/类型；
- `SLOTS` 常量数组（`px_graphs/src/bin/shaders.rs:8`）换成**扫描 `art/shaders/*.wgsl`**（与库加载同一套规则）；
- 「配方词汇 → shader 词汇」的映射（`extinction → density`）要么取消（配方直接用 shader 的名字），
  要么**显式写成数据**（配方里一张 `[params.map]` 表）。

- **代价**：中。改三处烘图代码 + 一个共享反射 crate。
- **收益**：**加一个参数 = 改 WGSL + 改配方，0 编译**；打错名字在**烘图时**就报（今天要等到装载）。
  这就是「美术自助」的**主要部分**。
- **判据**：新路径烘出来的文档与旧路径**逐字段相同**（场景键会变，因为配方词汇变了 ⇒ 要一条
  「文档等价」的断言，而不是「键相同」的断言）；一条专门的单测：故意写错参数名 ⇒ 烘图当场报错。

### S3｜schema 是一份文档，WGSL 由它生成（§66 形态 4）

`art/materials/<名>.toml` 声明属性（名 / 类型 / 缺省 / 范围 / UI 提示）+ 贴图槽 + body 的来源；
生成器输出：`struct Params { … }` + `@group(..) @binding(..)` 声明 + 入口骨架，body 仍是手写或库函数调用。

- **代价**：中。生成器 + 一致性校验（body 引用的名字由生成器校验）。
- **收益**：schema 变成**显式数据**（工具/UI 不必跑 naga 就能读）；新增属性 0 编译；
  参数块的字节布局由生成器唯一决定 ⇒ §67.4 里「偏移被测试钉死」那条耦合松掉。
- **代价的另一面**：**body 仍然是人写的** —— 生成器只管接口，不管算法。这不是「图」。
- **判据**：拿现有三份 shader 反推 schema，生成结果与手写文本**逐字节相同**（或逐字段相同 + 偏移相同）；
  同图两次生成必须逐字节相同（确定性 ⇒ 键才稳定）。

### S4｜节点图 → WGSL（Unity Shader Graph 那一档）

节点图文档（TOML/JSON）→ 类型推导 → 拓扑排序 → WGSL。

- **代价**：**大**，而且不在本仓现有任何一层里：需要节点注册表、类型系统与隐式转换、子图与去重、
  错误定位（节点 id → 行号）、确定性、节点版本迁移。**并且要一个编辑器**，否则手写节点 TOML
  并不会比手写 WGSL 容易 —— 收益全在编辑器那一侧。
- **⚠ 这是一次明确的路线反转**：§16.6 把「图不再是可读数据 / 美术自己拖节点不在路线图上」写成了**裁决**，
  代价原文是「这一层的天花板是『谁来写图』：现在是 agent 写在代码里，**将来要工具或人来写，
  才需要真正的图引擎**」（`01-decisions.md:93`、`02-pcg.md:10`）。
  ⇒ 这条要求**正好落在当年预判的那个触发点上**，但它要花的是「图引擎」那一档的钱。
- **现实的切法**：**别把 `clouds.wgsl` 重写成图**。那份文件 1600 行、约 40 个函数、
  带 7 档消融常量（`art/shaders/clouds.wgsl:39-59`）—— 它是重型算子的宿主，不是节点图能表达的规模。
  图的**第一批用户应当是「新材质 / 简单材质 / 组合既有库函数」**：节点库 = 现有 WGSL 库
  （`planet_x::common|noise|light`）的函数 + 少量内建节点（常量、算术、采样参数、贴图），
  而图编译出的入口可以用 `#import` 引库 —— 那样**库变了仍然由现有的闭包指纹兜住**（§52.3），
  键的机制一行都不用改。

#### S4 的工程代价清单（Unity / Unreal / MaterialX 的实践里挑出来的，**没有一条是可选优化**）

| 项 | 事实与出处 |
|---|---|
| **source map（错误定位）** | 编译错误的行号指向**生成代码**，不指向节点（实测形态：`Shader warning in 'Shader Graphs/…': implicit truncation of vector type at line 509`）。**必须自己实现「生成行号 → 节点 id」的映射**，否则这个工具在出第一个错误时就废了。⚠ 本仓有个白送的一半：生成的 WGSL 交给 **naga 自己 parse + validate，错误自带 span**，行号这一侧不用自己算 |
| **确定性（缓存命中的前提）** | Unity 的 shader 缓存命中判据就是「**identical source code**」；本仓是**键 = 内容**。⇒ 图遍历要按稳定 id 排序、临时变量按拓扑序编号、**不许依赖 HashMap 迭代顺序**、生成物里不许有路径/时间戳。**两条 A 级铁证**：① Unity 自己的 `ShaderGraphImporter` 在生成前对一个 `HashSet` 显式 `orderedSources.Sort()`；② Unreal 官方在 `bDeterministicShaderCodeOrder` 的文档里承认：关掉它时「the shader code will be stored in the library **essentially in a random order**」⇒ 上游工具自己都把哈希容器顺序当成不安全的 |
| **名字不能当 id** | Unity 真实历史：节点改名（`Normal Create` → `Normal From Texture`）、属性名与保留字冲突、HDRP 有一整张**禁用属性名表**。⇒ 节点/属性都要 **stable id + 版本 + 迁移函数表**，codegen 要有保留命名空间 |
| **类型推导与隐式转换要明文规定** | Unity：Vector 互相 promote/truncate、**truncate 直接砍通道、promote 补 `(0,0,0,1)`**、Dynamic Vector 按**最低维度**截断。⚠ Unreal 更严：*"Arithmetic between two inequivalent data types is invalid. For example, float2 + float3 is invalid and returns an error."* ⇒ **照 Unreal 做（不兼容就报错），别学 Unity 的静默截断** |
| **常量折叠 / 死分支剪枝是前提不是优化** | MaterialX 把它写进生成流程第 1 步，理由是「让 shader 小得多、省编译时间与内存、**代码更可读便于调试**」。⚠ Unreal 官方对「手写代码混进图」的代价说得更狠：*"**Using the custom node prevents constant folding** and may use significantly more instructions than an equivalent version done with built in nodes!"* |
| **变体爆炸** | Unity：`multi_compile` 编全部变体、`shader_feature` 只编用到的；keyword 上限**按版本读**（2020.2 手册写全局 256 / 每 shader local 64，**Unity 6.5 手册改成「超过 128 个就要小心」**）；生成器为**每个 keyword permutation** 走一遍全图（指数）；HDRP Lit 生成源码在 **4MB~16MB** 量级。HDRP 的选择是「**用 shader_feature 而不是 multi_compile**」。⚠ Unreal 对 Static Switch 的官方定性：*"applied at compile time, not at runtime… **effectively free at runtime**. On the other hand, **a new version of the Material must be compiled for every used combination of static parameters**… can lead to a **massive increase in shader permutations if abused**."* ⇒ **图里的「开关型节点」必须走「schema 变更 + 重编一次」，不许映射成运行期变体** |
| **子图是「函数」，不是文本内联**（更正） | Unity 的 Sub Graph 被 emit 成 **HLSL 函数 + Bindings 结构体**（`SG_<name>_<hash>(…)`），官方 Precision 文档也用函数类比它；MaterialX 把「公共子图合并」列为**可选**的图级优化。⇒ **去重是函数级的，不是调用点级的**：同一子图用 N 次 = N 次调用（CSE 要自己做，别指望下游驱动） |
| **生成代码与手写代码混用的顺序问题** | Unity 专门改过导入顺序（「graphs are imported before Models」）⇒ 混用要走「include + 约定签名 + 版本化路径」，依赖顺序要显式管。⚠ **本仓已有现成机制**：include 闭包指纹 + 装载时对账（§52.3）就是这一条的答案 |

### S5｜编辑器（两条独立的路）

- **Inspector（性价比最高）**：schema 今天在运行期就是已知的（`reflect::layout_of`），
  把它接成「自动生成的控制面板」是**纯增量**：窗口里那一套 `Instrument`（`main.rs:2134-2143`
  写死了 `ablate` + 三个键）就是这条路的雏形 —— 泛化它即可，不需要新的架构。
- **图编辑器**：最大的一件，且它**属于 S4**；S2/S3 不需要它也能把「自助」做到位。

---

## §70 候选形态（render graph 轴）

### §70.0 ⚠ 先纠正一个前提：Bevy 0.19 **已经没有节点式的 RenderGraph 了**

这一条推翻了很多现成资料的模型（包括本仓笔记里从没写过的部分），必须先钉住：

| 事实 | 证据 |
|---|---|
| `RenderGraph` 现在是一个 **`ScheduleLabel`**（单元结构体），不是图 | `bevy_render-0.19.1/src/renderer/mod.rs:32-35` |
| 它的 schedule 只有四段：`Begin → Render → Submit → Finish` | 同文件 `:38-51`（`base_schedule`）、`:54-64`（`RenderGraphSystems`）；`:69-76` `render_system` 里就是 `world.run_schedule(RenderGraph)` |
| 每台相机跑**自己那条** schedule（`CameraRenderGraph(InternedScheduleLabel)`） | `bevy_render-0.19.1/src/camera.rs:179`；`bevy_core_pipeline-0.19.1/src/schedule.rs:133-206` 的 `camera_driver` 逐个 `world.run_schedule(camera.schedule)` |
| 3D 那条叫 `Core3d`，四段是 `Prepass → MainPass → EarlyPostProcess → PostProcess`（chain） | `bevy_core_pipeline-0.19.1/src/schedule.rs:33-52`、`:54-70` |
| **pass 现在是系统**，排序靠 `.before()` / `.in_set()` | `bevy_post_process-0.19.1/src/bloom/mod.rs:78-79`：`bloom.before(tonemapping).in_set(Core3dSystems::PostProcess)` |
| 官方注释明说这是扩展点：「Additional systems can be added to these sets… 也可以相对这些核心集合新建集合」「一次性的 compute pass 可以排在 `camera_driver` 前后」 | `bevy_core_pipeline-0.19.1/src/schedule.rs:44-45`、`bevy_core_pipeline-0.19.1/src/schedule.rs:130-132` |

⇒ 「动态组装 render graph」在这套模型里 = **组装一条 schedule**。候选就两条，且都不必改 Bevy 的代码：

### R1｜**一条数据驱动的执行器系统**（推荐先做）

一个预先注册好的系统（挂在 `Core3dSystems::EarlyPostProcess`，或 `PostProcess` + `.before(tonemapping)`），
运行时读文档里的 pass 表，按拓扑序**逐个 `begin_tracked_render_pass`** 编进命令缓冲。

⚠ **插哪一档**：`EarlyPostProcess` 的官方语义就是「主 pass 之后的早期后处理」（`bevy_core_pipeline-0.19.1/src/schedule.rs:41`），
TAA / DLSS 就挂在这里（`bevy_anti_alias-0.19.1/src/taa/mod.rs:70-72`）；`PostProcess + .before(tonemapping)`
是 bloom / effect_stack / `FullscreenMaterial` 的默认（`bevy_post_process-0.19.1/src/bloom/mod.rs:77-80`、
`bevy_core_pipeline-0.19.1/src/fullscreen_material.rs:93-97`）。**用 `EarlyPostProcess` 更稳**
（不依赖 `tonemapping` 这个锚）。

**关键技术事实（都查实了，决定这一刀的可行性）**：

- 低层 pass 的写法在 0.19 里是 `RenderContext` + `ViewQuery`，**没有 `ViewNode`**：
  `bevy_render-0.19.1/src/renderer/render_context.rs:131-136`（`RenderContext`）、
  `:156-159` `command_encoder()`、`:162-175` `begin_tracked_render_pass()`、`:204-213` `CurrentView` + `ViewQuery`。
- **⚠ 一条推翻我先前假设的事实**：「绑定布局静态」这条**只对 `Material`/`MaterialPlugin` 成立**。
  自己写 pass 时布局是我们自己建的（`PipelineCache::get_bind_group_layout(descriptor)` 按 descriptor 去重），
  所以**一个 pass 能绑几张图、绑什么维度，可以逐 pass 由数据决定** ——
  这对 R1/R3 是决定性的好消息：通用执行器不必背「固定超集」那个包袱。
- 运行期建任意管线是可行的，而且**接收者是 `&self`**：`PipelineCache::queue_render_pipeline(&self, descriptor)`
  （`bevy_render-0.19.1/src/render_resource/pipeline_cache.rs:393-407`）；在 `Render`/`Prepare` 阶段入队**当帧可用**，
  在渲染进行中入队则下一帧可用（`:640-661` 的 `process_queue` 排在 `render_system` 之前，`bevy_render-0.19.1/src/lib.rs:430-432`）。
- ⚠ **`CachedRenderPipelineId` 是自增索引、不是内容键，官方明说不去重**
  （`pipeline_cache.rs:380-383`）⇒ **内容键要我们自己管**，落点是 `SpecializedRenderPipelines<S>`
  （`pipeline_specializer.rs:50-74`，内部 `HashMap<S::Key, CachedRenderPipelineId>`）或新的 `Variants<T, S>`
  （`specializer.rs:176-183`、`:269-307`）。这正好与本仓「键 = 内容」的习惯同构：键取
  `(shader 内容版本, 目标格式, blend/depth 状态, 输入格集合)`。
- ⚠ **失败是终态**：`ProcessShaderError` / `CreateShaderModule` 记一次错就 `return`、不再重试
  （`pipeline_cache.rs:685-708`）；`ShaderNotLoaded` / `ShaderImportNotYetAvailable` 才会回到 `Queued`。
  取不到管线时 `get_render_pipeline` 统一回 `None` ⇒ **pass 要自己判 `None`**（内建 pass 全是这么写的）。
  这与本仓 §62「坏管线当场拒、不静默出图」那条裁决**正面相关**：执行器必须把 `None` 变成拒绝，而不是跳过。
- 目标格式必须用 `view.target_format`（`bevy_render-0.19.1/src/view/mod.rs:370`、`:912-914`）——
  `RenderTarget::Image` 那条路的格式来自图片本身（`camera.rs:258-260`），HDR 打开时才被 `Rgba16Float` 覆盖
  （`camera.rs:614-620`）。ping-pong 有现成的 `ViewTarget::post_process_write()`
  （`view/mod.rs:715-720`、`:952-972`，文档警告「必须把 source 写进 destination，否则丢帧」）。
- **链尾不归我们管**：tonemapping 只在 `camera.hdr` 为真时跑（`tonemapping/node.rs:50-52`），
  输出到 out_texture 由 `upscaling` 独占（`upscaling/node.rs:89-101`）⇒ pass **不许碰 out_texture**。
- 可抄的官方样板：`examples/shader_advanced/custom_post_processing.rs`、
  `bevy_core_pipeline-0.19.1/src/fullscreen_material.rs:46-132`（`FullscreenMaterial` +
  `FullscreenMaterialPlugin<T>`：默认就插在 `PostProcess` 且 `before(tonemapping)`，带双纹理 ping-pong 绑定与
  `Specializer<RenderPipeline>` 特化）。⚠ 但 `FullscreenMaterial::fragment_shader()` 是**静态**的
  ⇒ 它适合「一个 Rust 类型一个 pass」，**不适合**「N 个数据声明的 pass」；要做数据驱动就得走上面那条自己建管线的路。

- 图是数据：顺序、依赖、读写目标全在文档里 ⇒ 「动态组装」= 换一份文档。
- 「重载」= 下一次请求（`main.rs:1627-1767` 每次请求都重建场景）⇒ **不需要**动 schedule，也没有
  「同一条内容键、两种图」的风险。
- 代价：执行器要自己做资源分配、管线创建/缓存、绑定、以及每个 pass 的 GPU 时间戳（`main.rs:362-367`
  现在只认 Bevy 那几条 pass 名）。

### R2｜**每个 pass 一个系统，按数据拼一条 schedule**（可行，但建议只用于结构性阶段）

源码级调研的结论比我预想的**宽松**（先把事实摆正）：

- **闭包就是合法 system**：`Func: Send + Sync + 'static` + `FnMut(系统参数…) -> Out`
  （`bevy_ecs-0.19.1/src/system/function_system.rs:886-889`），`IntoScheduleConfigs` 对函数与闭包一视同仁
  （`bevy_ecs-0.19.1/src/schedule/config.rs:561-568`），官方例子 `examples/ecs/system_closure.rs:13-23,32,38-45`
  ⇒ **捕获该 pass 的数据（`String` / `usize`）在运行期造一个 pass 系统，是可行的**。
- **`SystemSet` 也能运行期新建**：`Schedule::configure_sets` → `SystemSets::get_key_or_insert`
  （`bevy_ecs-0.19.1/src/schedule/node.rs:764-772`）。
- **加系统不需要 `&mut World`**：`ResMut<Schedules>` + `Schedule::add_systems` 就够
  （`bevy_ecs-0.19.1/src/schedule/schedule.rs:219-228`）；改完下次运行自动重建图（`:560-571`、`:601-635`）。
  可行的时机是**两帧之间**：`ExtractSchedule`（它在渲染世界内跑，`bevy_render-0.19.1/src/extract_plugin.rs:84-87`、`:120`）
  或 `Render` schedule 里排在 `render_system` 之前的系统。
- ⚠ **硬限制只有一条**：在**正在运行**的 schedule 内部改它自己 —— schedule 已被 `remove_temporarily` 摘走，
  写入落到一个新建的空 schedule 上，scope 结束时被覆盖，**只打一条 warn**
  （`bevy_ecs-0.19.1/src/world/mod.rs:3834-3846`；不变量原文见 `schedule.rs:44`；
  官方注释 `examples/showcase/stepping.rs:3-8`）。因为 `RenderGraph` 跑在 `render_system` 内、
  `Core3d` 又跑在 `camera_driver` 内 ⇒ **任何渲染 pass 系统都无法重组它所属的图**。
- ⚠ **第二个坑**：闭包是**匿名类型**，**不能**当 `.before()/.after()` 的锚
  ⇒ 运行期拼的链只能靠「可命名的 fn item + `SystemSet` + `.chain()`」来定位。

**那为什么仍然推荐 R1 做主干？** 三条理由，与可行性无关：

1. **pass 之间的数据依赖（谁读谁写）没法直接变成 system 之间的边** —— R2 里这张图的排序语义被抓在
   ECS 调度图里，而 R1 里它是**我们自己的数据**，能打出来、能校验、能拓扑排序。
2. **换文档就要改 schedule** ⇒ 「同一条内容键、两种图」这个风险是自己造出来的；R1 里图跟着文档走，不留残影。
3. R1 的失败模式是「管线没编好」，R2 的失败模式里多了一条「调度图被静默改写」——**静默**是这条路上最贵的东西。

**R2 的真正用武之地是「少数几条固定的结构性阶段」**：按文档给某台相机换一条 `CameraRenderGraph`
（主世界组件 `bevy_render-0.19.1/src/camera.rs:179`、`:188-192`，每帧 extract `:529`、`:630`，下一帧生效；
默认值由 `Core3dPlugin` 的 `register_required_components_with` 注入，`bevy_core_pipeline-0.19.1/src/core_3d/mod.rs:104-108`），
或运行期调整 `RenderScheduleOrder`（`bevy_render-0.19.1/src/lib.rs:266-286`、`:475-481`）。
这两条都是**现成的、粗粒度的**组装能力，可以作为「一条文档换个整体管线」的开关。

### R0｜文档 → Bevy 自带的后处理（几乎免费，应当先做）

`bevy_post_process` 已经在默认 feature 里（`bevy-0.19.1/Cargo.toml:2605-2633`：`3d` ⇒ `bevy_post_process`），
现成有 `Bloom`、`DepthOfField`、`MotionBlur`、`AutoExposure`，以及 `effect_stack`
（`ChromaticAberration` / `LensDistortion` / `Vignette`）。它们都是**相机上的组件**
（`Bloom` / `AutoExposure` 带 `#[require(Hdr)]`，见 `bevy_post_process-0.19.1/src/bloom/settings.rs:32-33`、
`.../auto_exposure/settings.rs:29-30`），节点插在 `tonemapping` 之前、`Core3dSystems::PostProcess` 内。

- 今天 px_render **一个都没用**，目标还是 8 位 sRGB（`main.rs:1461`、`:1487`）。
- 代价：文档加一节 + 装载时按文档挂组件；**外加把中间目标升到 HDR**（`Hdr` 组件在
  `bevy_camera-0.19.1/src/components.rs:89`），否则 bloom/自动曝光的 `#[require(Hdr)]` 无处落脚。
- 这是**性价比最高的一刀**：美术马上拿到一整套后处理，而且全程没有新框架。

### R3｜中间目标 + 依赖（mini FrameGraph）

在 R1 上加 `targets[]`（格式 / 尺寸规则 / usage）与 `passes[].reads/writes`，执行器做拓扑排序、
transient 分配/复用、barrier。**判据**：排序结果要打出来；同文档两次运行排序相同；
半分辨率云、调试视图（把中间目标直接出图）是这一档真正的收益。

### R4｜compute pass / storage

协议要再加一档固定超集（storage texture / buffer）。**边界在 §1 的裁决里**：
当年列的三条迁栈判据之一就是「要 compute 做交互级烘焙」；`px_probe` 已经证明这条路在本仓跑得通
（它自己建 bind group layout 与 compute pipeline：`px_probe/src/probe.rs:279-420`）。

### R5｜整个渲染器换成自研 graph

§1 已裁决不做（「自研 wgpu + naga 3–6 人月才到 parity」）。列在这里只为把边界画全。

### §70.1 「重载」在今天意味着什么（三个不同的东西，别混）

1. **换内容**（换一份文档 / 换一版 shader）：**已经通了**，而且是内容寻址的（§67.1）。
2. **换 pass 结构**：R0/R1 下 = 换文档，同上通；R2 下 = 改 schedule，是新东西。
3. **改** `.wgsl` **文件就生效**（开发期那 1 秒的近路）：今天只在「槽里的版本」这一层通了；
   而场景钉的是**产物键**，所以改文件之后要走「重烘 shaders → 重烘 scene」。
   §52.3 与 `10-handoff.md` P12 记的正是这条：引擎那半截已经在了，缺的是工具层的 watcher。

---

## §71 外部证据（每条标级别；A = 官方文档/源码/论文，B = 二手）

### §71.1 Unity：schema 是编译期产物（**A，我逐字核过原文**）

来源（官方 HDRP 文档，14.0.12）：
<https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@14.0/manual/Customizing-HDRP-materials-with-Shader-Graph.html>

- 「**Every Blackboard property contributes to the size of the Material on disk, whether you expose it or not.**」
  ⇒ 属性集合（schema）在编译期就进了材质的序列化数据，**不是运行期查表**。
- 「**Once you created the material, all it's properties are saved and never synchronized back with the Shader Graph**,
  even if you didn't changed anything on the material (mainly because there is no override system).」
  ⇒ 材质是 schema 的**快照**，不是视图。
- 「**Note that switching these surface options is only possible in edit mode, not in the player.** Thanks to this,
  it doesn't add any extra variants to the shader compilation process (we use shader features instead of multi compiles).」
  ⇒ **运行时（player）不能改 schema**，而且这是**有意**换来的（省变体）。
- 官方 Known issues 里还记着：改 Master Node 的属性，除非该材质正开在 Inspector 里，否则**不同步且渲染会坏**，
  只能靠 `HDEditorUtils.ResetMaterialKeywords(Material)` 手动修。

⇒ 结论见 §68：**「像 Shader Graph 那样动态改 schema」这个类比本身是错的**；Unity 的动态只到「值」，
schema 是作者期的。而本仓今天在**运行期**这一侧已经超过它（§67.1）。

### §71.2 工业界怎么做「动态组装」（**A**）

| 引擎/项目 | 图是数据还是代码 | 运行期能不能加 pass | 注入点形态 |
|---|---|---|---|
| **Unreal RDG** | 代码（immediate-mode），**每帧重建**，`FRDGBuilder` 单实例 | 不能加 pass；只能靠 `r.RDG.CullPasses` / `NeverCull` 等开关 | pass = lambda，读写/create 显式声明 |
| **Unity URP RenderGraph** | 代码，每帧 `BeginRecording → EndRecordingAndExecute` | 运行期只能 `SetActive` 开关 Feature | **`RenderPassEvent` 固定 18 槽** |
| **Godot 4** | 硬编码 C++，`RenderingServer` 不透明 | 不能改结构 | **`CompositorEffect` 固定 5 槽**，只能 compute |
| **Falcor（NVIDIA）** | **脚本/编辑器构图**，且**支持运行中实时改图**（图的 output 必须始终有效） | **能** | 任意连边（自带编译器） |
| **OGRE compositor** | **`.compositor` 脚本文件**（管线即数据） | 靠脚本重载 | 固定段落（texture / target pass / render_quad / compute） |

两条对本案最要紧的读数：

1. **主流引擎的插入点都是「有穷枚举」，不是任意连边**（Unity 18 槽、Godot 5 槽）。
   任意连边意味着自己实现 culling / aliasing / barrier / debug 视图（Granite 那份 ~3 ksloc），
   而 UE / Unity 都靠编译器兜底。⇒ 本项目要的是「**固定 stage 枚举 + 有穷 pass 种类**」，
   不是通用 FrameGraph。
2. **没有主流商业引擎把主场景 render graph 放进 JSON/TOML**（B 级：没找到反例）。
   最接近的两个「图即文件」样本是 Falcor（Python 脚本 + 编辑器）与 OGRE（`.compositor` 脚本）——
   也就是说：**本仓「文档声明 pass」这个方向有先例，但那是小众路线，工程量的账要自己认。**

其余可复用的通用模型（A）：pass 必须显式声明 Read/Write/Create；transient 可 alias、external 不可；
图的生命周期限定一帧；自动 barrier = 按首/末次使用构造 invalidate/flush 桶；
D3D12 Enhanced Barriers 把 Sync / Access / Layout 解耦（`ACCESS_NO_ACCESS` 做 alias 激活/去激活）。
**本仓的切法（R1，不碰 alias）正好绕开这一整块**。

### §71.3 现成工具链：能不能不自己写代码生成

| 方案 | 状态 | 对本案的意义 |
|---|---|---|
| **MaterialX**（ASF，USD/Hydra 那条线） | ⚠ **更正**：它的 `WgslShaderGenerator` **不生成真正的 WGSL**，只生成「便于转 WGSL 的 Vulkan GLSL」；官方 issue [#2751](https://github.com/AcademySoftwareFoundation/MaterialX/issues/2751)（2026-01 开、**至今 open**）原文：*"WgslShaderGenerator doesn't actually produce wgsl, but just an adapted variant of glsl that is suitable for conversion to wgsl."* 真 WGSL 后端只存在于**未合并**的 PR [#2996](https://github.com/AcademySoftwareFoundation/MaterialX/pull/2996)（2026-07 开，+10k 行，仍在 review；PR 正文自陈「the current glsl to wgsl pathway… is **not suitable for runtime systems**」）。它**真正有**的是 **Slang 后端**（默认**关闭**，`MATERIALX_BUILD_GEN_SLANG`）⇒ 现成路径是 **MaterialX → Slang → WGSL**（**A**） | **架构最值得抄**（见 §71.4 末尾），但**不要**把它当「节点图 → WGSL」的现成实现；⛔ Rust binding 不可用（`materialx`/`materialx-sys` 都是 2021 年的 v0.0.0 占位符） |
| **Rust 生态**（更正：**有**现成的「节点图 → WGSL」crate） | ⭐ `node_engine`（MIT/Apache-2.0，0.7.0）**直接产 WGSL 语法字符串**，自带 `FragmentOutputNode` / `UVNode` / `TextureNode` 等 shader 节点；`bevy_shader_graph` 0.4.0 是它的 Bevy 集成（含编辑器）；`reflow_shader`（~2.4k 行）也是「DAG → WGSL」；`bevy_hanabi::graph` 证明该模式在 Bevy 生产 VFX 里可行。⚠ 全都年轻/单人维护/文档薄（`node_engine` 文档 22%、下载量 7k）⇒ **可当原型与参考，不建议当依赖**。⛔ `egui_node_graph` **已被 yank**（0.1–0.4 全 yank），图形 UI 改用 `egui-snarl` | ⭐ **这一条把 S4 的代价估算改小了**：至少「DAG → WGSL 文本」这件事有可读的参考实现（一小时能读完 `reflow_shader`）。**但没有任何一个替你解决 source map / 确定性 / 迁移 / 变体**，那四条仍然是自己的活 |
| **naga 的反射** | ⚠ `naga::proc::TypeLayout` 是 **per-type** 的、**没有 per-member offset**；**成员偏移在 IR 里**：`StructMember { name, ty, binding, offset }`；`ModuleInfo` 里**没有** offsets。⭐ 最值钱的契约：*"If you do perform full validation and `Validator::validate` returns `Ok`, then **Naga promises that code generation will either succeed or return an error; it should never panic.**"*；`ParseError` 提供 `labels()` / `location(source)` / `emit_to_string(source)` ⇒ **自带 span** | **「名字 → 字节偏移」的正统实现**：偏移读 `StructMember::offset`，大小/stride 读 `Layouter`；生成物用 `parse_str` + `validate` 自检，**错误定位白送一半**（只剩「生成行号 → 节点 id」要自己做） |
| **Slang** | WGSL 后端是 **experimental**（README 脚注「WGSL support is still work-in-progress」）；**Apache-2.0 WITH LLVM Exception**，已转 Khronos 治理。⭐ 它的**反射最强**（`VariableLayoutReflection::getOffset/getBindingSpace`，WebGPU 下 binding space 就是 bind group；`TypeLayoutReflection::getSize/getStride`），⭐ 有**三套正交的运行期特化机制**（link-time specialization / generics+`specialize` / dynamic dispatch），⭐ 有 `[require(...)]` **能力系统**（能在 codegen **之前**拒绝一组能力不兼容的节点组合 —— 本调研里没有任何其它方案提供这种静态保证）。⚠ 反射**不能从 `slangc` 命令行拿**，必须走编译 API；⛔ Rust 侧**没有任何 Slang binding**，要自己写 FFI | ⭐ **一条绕开「WGSL 后端 experimental」的路**：**wgpu 直接吃 SPIR-V**，且官方原文 *"While WebGPU does not support any shading language other than WGSL, we will **automatically convert** your non-WGSL shaders if you're running on WebGPU."*（Bevy 侧 `Shader::from_spirv`）⇒ 真要上 Slang 就走 **Slang → SPIR-V → wgpu**，不赌它的 WGSL 文本后端 |
| **WESL**（Bevy 的方向） | `Shader::from_wesl` 已存在（`bevy_shader-0.19.1/src/shader.rs:152`，feature `shader_format_wesl`），`ShaderLoader` 扩展名表里已有 `"wesl"`（`:380-382`）；`wesl` crate 支持**运行期**编译（`Compiler::compile` → `syntax.to_string()` 得纯 WGSL）；Bevy Project Goal #23015 写着 WESL 要**取代**现在这套基于 naga_oil 的「Bevy 扩展 WGSL」。⚠⚠ **时序**：Bevy 协作者明确说 *"Bevy imports in wesl are **not supported until bevy 0.20** is shipped"* ⇒ **短期仍用 naga_oil 的 `#import`，别提前迁移** | ⚠ **对代码生成方案的影响**：生成物应当是**纯 WGSL**；`#import` 只许出现在**一个函数**里，将来换 WESL 是改一处 |
| **three.js TSL** | 节点图是**运行期的 JS 对象**，官方 wiki 明说 **同时编到 WGSL 与 GLSL**（`WGSLNodeBuilder` / `GLSLNodeBuilder`），节点可 `serialize()/deserialize()`；⚠ TSL compute 改图后设 `needsUpdate` **不会**重建节点（官方 issue #33061） | 主流方案里唯一「schema 住在运行期」的；但 §1 裁决 19 已定：整条 3D 走 web 才会用它 |
| **Babylon NodeMaterial** | ⭐ **最接近本仓想要的产品形态**：节点材质是**可序列化 JSON**（`ParseFromFileAsync`），且官方原文 *"The assigned engine parameter will **NOT** be saved in the node material's JSON file, **allowing you to still choose the correct version at runtime**."* ⇒ **同一份图数据，运行期决定目标语言与绑定** | 值得当作「图数据 = 资产、编译 = 运行期服务」的形态参考（与 MaterialX / MDL / OSL 同一族） |
| **Graphite**（Rust 节点图项目） | **build script** 把节点编成**一个 WGSL** 并以字符串 include（**构建期**，不是运行期）（**B**） | ⚠ 它只说明「连认真的 Rust 节点图项目也选了构建期」；**但 `node_engine` 证明运行期可行**（见上）⇒ 这条不再是「没有先例」的证据 |
| **明确不要碰** | ⛔ `shaderc`（无反射，而且它**把原有的反射删了**）；⛔ `SPIRV-Cross` **无 WGSL 输出**（只可当反射补强，Rust 侧用 `spirv-cross2`，别用停维的 `spirv_cross`）；⛔ `wgsl-analyzer` **不发布 crate**（只能当编辑器工具）；⛔ Blender 的 codegen 是 **GPL**；⛔ Unity ShaderGraph 是 **Unity Companion License（非开源，不可移植）**；⛔ `litsdf` / `zygote` 等**无 LICENSE** | 都是**法律或能力**上的硬阻塞，别浪费评估时间 |

### §71.4 同类系统的 schema 住在哪（横向对照，全部 A 级）

| 系统 | schema 住在哪 | 材质/实例存什么 | 运行期能不能改 schema |
|---|---|---|---|
| **Unity**（Shader Graph） | 图 → 生成 ShaderLab `Properties` 块（Blackboard 就是它的图形化编辑器）；`Shader` 有整套**运行期反射** API（`GetPropertyCount/Type/Flags/Attributes/RangeLimits/Default*`） | `.mat` 存**属性名字符串 → 值**（`Shader.PropertyToID` 跨运行/跨机器不稳定，**不能落盘**）；**每个材质复制一份全量槽位** | **不能**。改图 = 重新 codegen + 重新编译（秒级）；属性**改名后旧条目变孤儿、不报错不迁移**；改了 Master Node 而未开在 Inspector 里的材质**会失同步并渲染损坏**（官方 Known issue，只能 `HDEditorUtils.ResetMaterialKeywords` 修） |
| **Unreal** | 图里的 Parameter 节点 | Material Instance 只存**哪些参数被 override + 值**（⚠ 官方：未勾选字段的值在关窗时**丢失**） | **不能**。换父材质 ⇒ 新父材质没有的参数在实例上**消失**；Static Switch 改一次 = **一个新 permutation**；Epic 专门做了 Material Analyzer 做 reparent 来压 permutation 与存储 |
| **Godot 4** | shader **文本**（`uniform float x : hint_range(0,1)`），renderer 从文本解析 | `ShaderMaterial.set_shader_parameter(名字, 值)`；per-instance 走 `set_instance_shader_parameter` | 换 shader 可以；**加 uniform = 换文本** |
| **three.js** | 传统：JS 对象 + 字符串；**TSL：运行期的节点图对象** | uniforms 对象 | **TSL 是唯一「运行期可变」的主流方案**；传统路要 `needsUpdate` **且** program cache key 变了才会重编 |
| **Blender** | 节点树（渲染时才编译）；Script Node 写 OSL，自动 `.osl → .oso` | 节点值 | 无「运行期」概念（DCC） |
| **MaterialX / MDL / OSL** ★ | **schema 是资产**（XML / `.mdl` / `.osl`），**编译是运行期的服务** | 参数由 shader 声明 | **能** —— **这才是「运行期可改 schema」的真实工业先例**（但都不是游戏引擎；MDL JIT 到 PTX，OSL 用 LLVM JIT） |
| **本仓今天** | **WGSL 文本**（naga 反射出名字/类型/偏移） | 文档里 `params: 名字 → Value`（**没有冗余槽位**） | **能**（§67.1）—— 在「运行期」这一维上比 Unity/Unreal 都更靠前 |

**两条可以直接照抄的结论**：

1. **抄 Unity 的分层**（schema 在 shader / 值在材质 / Inspector = shader 的反射视图），
   **反着做 Unity 的「材质快照全量槽位」**：本仓今天已经是「只存稀疏的名字→值」，**别改回去** ——
   一旦把 schema 快照进材质，就必然重现「改图后材质静默失同步」那一类事故。
2. **要抄的对象是 MaterialX / MDL / OSL 这一类**（shader 是资产、编译是运行期服务），
   而不是 Unity / Unreal（schema 编译期冻结）。**MaterialX 的 ShaderGeneration 开发者指南可以直接当架构参考**：
   节点实现注册表（4 种实现方式 + `registerImplementation`）、常量折叠与死分支剪枝、拓扑排序 + 环检测、
   **把图的接口发布成 uniform 并能从生成结果读回**、scope 跟踪。


### §71.5 Rust / wgpu 生态：没有可依赖的 render graph

- `rend3`（1160★，**2024-07-08 后 archived**）、`kajiya`（5342★，**2025-07-07 后 archived**）—— 都不能当依赖（**A**）。
- `engawa` / `engawa-wgpu`：**纯数据 IR + 拓扑排序 + 环检测**，最贴近「图即数据」，但 v0.1.x、下载量三位数（**A**）⇒ 只能当参考。
- `Granite`（`render_graph.hpp/cpp`，作者自述 ~3 ksloc）、`skaarj1989/FrameGraph`：**值得当参考实现读**。
- **wgpu 自带的 `PipelineCache` 帮不上内容寻址**：只在 Vulkan 后端有实现，官方给的 key 只是
  `pipeline_cache_key(adapter_info)`（**适配器级，不含 shader/图内容**），而且没有缩容 API（**A**）
  ⇒ 「图变了但键没变」必须**我们自己堵** —— 与本仓「键 = 内容」那条不变式同一个位置。

### §71.6 Bevy 0.19 的其余相关事实（**A**，源码级，见 §70 各条行号）

- 0.19 **删掉了节点式 RenderGraph**（官方 0.19 发布说明：*"Bevy's RenderGraph architecture has been replaced with
  ECS schedules. Render passes are now regular systems…"*），`RenderGraph` 这个名字变成了 schedule label。
- 官方例子就是新形态：`.add_systems(Core3d, post_process_system.in_set(Core3dSystems::PostProcess))`
  （`examples/shader_advanced/custom_post_processing.rs:63-66`）、
  `.add_systems(RenderGraph, game_of_life.before(camera_driver))`。
- 0.19 新增 `RenderErrorPolicy`（`DeviceLost → Recover` / `OutOfMemory → StopRendering` /
  `Validation → Ignore` / `Internal → panic`）—— 「渲染坏了但别把 App 带走」的工业级做法，
  与本仓 §62「能证明坏了就当场拒」是同一条思路的两端，接 pass 执行器时要对齐。

---

## §72 推荐路线

**一句话**：两条线**分开走**；每条线的**第一刀都选「不新增架构」的那一刀**；
「节点图 + 编辑器」与「通用 FrameGraph」这两个最贵的形态**都被外部证据与源码级限制挤到最后**。

### §72.1 schema 线（形态名见 §69）

| 序 | 形态 | 拿到什么 | 代价 |
|---|---|---|---|
| **1** | **S5 的前半（Inspector）**：装载时把 `MaterialLayout`（名字/类型/偏移）打进日志或回给客户端；窗口那套 `Instrument`（现在写死 `ablate` + 三个键）泛化成「按 schema 自动生成控制项」 | 美术**当场**能看见并调每一个参数（含新加的），零架构 | 小（纯增量） |
| **2** | **S1 契约收口** | 后面所有形态的地基；灭掉 2/3 那颗雷 | 小 |
| **3** | **S2 schema 由 shader 声明 + 配方透传** | **加参数 = 改 WGSL + 改配方，0 编译**；错误从「装载时」提前到「烘图时」。这就是「自助」的主要部分 | 中 |
| **4** | **S3 schema 是文档、WGSL 由它生成** | schema 变成显式数据；布局由生成器唯一决定 | 中 |
| **5** | **S4 图 → WGSL**（真正的 Shader Graph 那一半） | 组合能力 | **大**（要节点注册表、类型系统、错误定位、确定性；**而且要先有编辑器才有收益**） |
| **6** | **S5 的后半（图编辑器）** | — | **最大**，且属于 S4 ⇒ 单独裁决 |

**为什么不建议一上来做 S4**：`clouds.wgsl` 是 1600 行、约 40 个函数、7 档消融常量的重型宿主
（`art/shaders/clouds.wgsl:39-59`），**它不是节点图能表达的规模**，也不该被重写。
S4 的第一批用户应当是**新材质 / 简单材质 / 组合既有库函数**；在那之前，第 1+3 步已经把
「美术自己加参数、自己看见、错了当场报」这条路走通 —— 而这正是「自助」的**大头**。

### §72.2 render graph 线（形态名见 §70）

| 序 | 形态 | 拿到什么 | 代价 |
|---|---|---|---|
| **1** | **R0 文档 → Bevy 自带后处理** + 中间目标升 HDR | 美术**立刻**拿到一整套后处理（今天一个都没接） | 小 |
| **2** | **R1 执行器系统 + 文档里的 pass 表**（固定 stage 枚举 + 有穷 pass 种类，先只做 fullscreen） | 「动态组装 / 重载」真的成立：换文档就是换图 | 中 |
| **3** | **R3 中间目标 + 尺寸规则 + 池化复用**（**明确不做 alias**） | 半分辨率云、A/B 复合、调试视图（中间目标直接出图） | 中 |
| **4** | **R4 compute pass + storage 绑定** | GPU 侧预计算 | 中（`px_probe` 已有先例：`px_probe/src/probe.rs:279-420`） |
| — | ~~R2 当作「pass 链」的主干~~ | — | **不做主干**（可行，但数据依赖没法变成调度边、且换文档要改 schedule ⇒ 自己造出「同键两种图」）；**结构性阶段用它**：按文档换 `CameraRenderGraph` 或调 `RenderScheduleOrder` |
| — | ~~通用 FrameGraph（任意连边）~~ | — | **不做**：culling / aliasing / barrier / debug 视图全要自己写（~3 ksloc 起）；主流引擎都收敛成固定注入点 |
| — | ~~R5 自研渲染器~~ | — | §1 已裁决（3–6 人月到 parity） |

### §72.3 两条线共用的四条不变式（**这是本方案能不能站住的关键**）

1. **键 = 内容，扩展到图与 pass**：图文档字节 + 生成器/节点库版本 + pass 定义 + shader 闭包 +
   绑定布局描述 + **目标格式 / samples / HDR** 全部进键。理由：wgpu 的管线缓存 key **只覆盖适配器**，
   不含内容（§71.4）；漏一个维度就是「同一个键、不同图」。
2. **装载时对账，对不上当场拒**：`scene::closure_check`（`scene.rs:63-93`）那一套原样复用；
   图产物也记自己的指纹与规模，装载时跟盘上现在的比。
3. **失败不许静默**：管线没就绪/编译失败 ⇒ **不出图**（§62 的裁决）。
   ⚠ 这条与 Bevy 内建 pass 的惯例**相反**（它们都是 `get_render_pipeline` 回 `None` 就跳过，
   见 `pipeline_cache.rs:336-344`）⇒ 执行器必须把 `None` 显式转成「拒绝」，否则就是
   §34 那条不变式禁止的「少了那个材质的成功图」。
4. **每个 pass 都要有仪器**：自己的 GPU 时间戳并进 `gpu_snapshot` 的求和
   （`main.rs:362-367` 现在只认 Bevy 那几条 pass 名）；**排序结果要打出来**（图怎么排的必须可见）；
   每种 pass 都要有消融档（开/关）当判据。

### §72.4 判据（每条都按本仓的规矩给可执行的判据）

| 步 | 判据 |
|---|---|
| A0 | 装载日志/回包里的 schema 与 `reflect::layout_of` 一致；参数面板改一个值 ⇒ 只变那个材质的那几个字节 |
| A1 | **所有产物键逐字节不变**（只搬代码）；现有离线门全绿；`orbit-*` 出图哈希不变 |
| A2 | 现有场景重烘后**文档逐字段相同**（键会变，配方词汇变了 ⇒ 判据是文档等价不是键相同）；故意写错参数名 ⇒ **烘图时**红 |
| A3 | 同图两次生成**逐字节相同**（确定性）；用现有 shader 反推 schema，生成结果与手写**逐字段 + 偏移相同** |
| B0 | pass 表为空且不挂新组件 ⇒ 与今天出图**逐字节相同**；打开 bloom ⇒ 差异可判、管线 0 失败 |
| B1 | pass 表为空 ⇒ 逐字节相同；开一条 pass ⇒ `pixdiff` 可判；每个 pass 有自己的 GPU 时间戳；坏 pass shader ⇒ **当场拒**（对齐 §62 的四条实测形态） |
| B2 | 同文档两次运行**排序相同**且排序被打出来；中间目标能直接出图（调试视图） |

### §72.5 代价里最该被记住的两条

1. **管线编译的墙**：服务冷启动已经是 15–25 s（预热那一段），而 pass / 材质变体是**乘**上去的。
   动态图会让变体数增长 ⇒ **pass 数要少、种类要有穷**，别把它做成「每个效果一条变体」。
   磁盘管线缓存那条捷径在本机**不可用**（§P6：DX12 上 no-op，要 fork `bevy_render`）。
2. **A3（图 + 编辑器）是本方案里唯一一个「新架构」**，也是唯一一次**明确的路线反转**
   （§16.6：「图不再是可读数据 / 美术自己拖节点不在路线图上」；`01-decisions.md:93` 原话是
   「将来要工具或人来写，才需要真正的图引擎」）⇒ **必须单独裁决，不要夹在别的批次里做**。

---

## §73 用户裁决（2026-09-16）

| 问题 | 裁决 | 含义 |
|---|---|---|
| schema 落点 | **「render 不需要重新编译」**（没有选 A/B/C） | 目标不是面板、也不是编辑器，而是**改 schema 不重编 Rust** |
| 先切哪条线 | **「不需要 Inspector，重点是改图改 schema 不重编译、不重启 render。可以接受切换/重构渲染器」** | ① **不需要**编辑器/面板；② 「改图」与「改 schema」都要**不重编、不重启**；③ **允许重构渲染器** —— 这条把「自建绘制路径」从禁区变成选项 |
| pass 范围 | **全屏 + compute pass** | pass 执行器第一版就要有 compute |
| 文档演进 | **允许「加法不升 `SCENE_SCHEMA`」，但未知字段不许静默忽略** | 加能力不再全链重烘 + 握手拒绝；同时必须堵住「旧渲染器读到新文档静默少画」 |

**这四条合起来是一句很清楚的话**：
> **把「改内容」的每一步都变成数据操作，一步都不许落回 Rust；并且这条性质要有可执行的判据。**

### §73.1 调研时提的四个问题与推荐档（留档）

1. **「像 Shader Graph」到底指哪一层？** (a) 自助改参数/贴图 + 自动面板；(b) 再加「组合」（图 → WGSL）；
   (c) 连拖节点的编辑器一起要。**推荐 (a)** —— 裁决选了「都不是，重点是**不重编**」，
   并把 (b) 收进了「改图不重编」这条要求里，(c) 明确不要。
2. **先切哪条线？** **推荐先做 R0**（文档 → Bevy 自带后处理，几乎免费）。裁决没有直接回答这一条，
   §74.3 按「一切服务于判据 P/R」重排为四步。
3. **pass 范围？** **推荐全屏起步、形状上给后面留位置**；裁决**直接要 compute** ⇒ 形状里必须有 compute。
4. **文档演进？** **推荐允许加法不升版本，但未知字段不许静默忽略** —— 与裁决一致。

---

## §74 按裁决修订后的路线

### §74.1 先定义判据（裁决要求的那条性质，必须先可判）

**判据 P（不重编）**：改 schema / 改 shader 图 / 改 pass 图之后，走完「重烘 → 请求 → 出图」，
**`px_render.exe` 的 sha256 不变，且没有发生任何 `cargo` 编译**。
两条都能一眼验：exe 哈希（或 mtime）在改动前后相同 + 流程里不出现 `cargo build`。

**判据 R（不重启）**：**同一个服务会话（pid 不变）**里连续请求两份「schema / pass 图不同」的文档，
两份都成功出图；服务日志里没有重启迹象。

**判据 D（确定性）**：同一份图/文档两次生成（或两次装载）**逐字节相同**；装载时对账不通过就当场拒。

> 这三条比任何设计文档都硬：**它们把「动态」从形容词变成可判定的东西**。后面每一步都要在报告里带这三条。

### §74.2 今天挡住判据 P/R 的东西（逐条点名）

| # | 挡路的东西 | 位置 | 谁要动 |
|---|---|---|---|
| 1 | 配方参数名的**手写白名单与映射** | `px_graphs/src/bin/scene.rs:442-487`、`:517-586` | 烘图侧（Rust） |
| 2 | shader 槽的**硬编码数组** | `px_graphs/src/bin/shaders.rs:8` | 烘图侧（Rust） |
| 3 | schema 在烘图侧**不可见**（只能盲写名字，错了装载时才报） | §66.3 形态 1 | 共享反射 crate |
| 4 | 材质绑定布局的**固定超集**（4 张贴图、参数块 1024 字节、组号写死） | `px_render/src/reflect.rs:22-34`、`material.rs:98-133` | 渲染器（**要么加宽，要么换路**，见 §74.4） |
| 5 | schema 变更**每次都要升 `SCENE_SCHEMA`** | `px_protocol/src/scene.rs:13`、`check()` `:530-536` | 协议（裁决已允许「加法不升」） |
| 6 | 未知字段被 `serde` **静默忽略**（旧渲染器读新文档 = 静默少画） | 全仓 `SceneSpec` 未用 `deny_unknown_fields` | 协议（裁决要求堵住） |
| 7 | 窗口那套仪器**写死 `ablate`** | `px_render/src/main.rs:2134-2143` | 渲染器（低优先级：裁决说不要 Inspector，但这条是「渲染器认识具体参数」的残留） |

### §74.3 四步（每步都带判据 P/R/D）

**第 1 步｜把 schema 变成数据（挡住 #1 #2 #3 #5 #6）**

- 契约收口（§66 的 P1 / §69 的 S1）：绑定表 + `ParamKind` + `Value↔kind` 一份；naga 反射挪进叶子 crate
  （`px_protocol` 的依赖被门钉死只有 serde，见 `px_protocol/tests/crate_graph.rs:4`）；
  schema descriptor 进 shader 产物，**装载时对账**（`scene::closure_check` 同款）。
- 配方 `params` **按名字透传**，由 descriptor 在**烘图时**校验；
  `SLOTS` 换扫描 `art/shaders/*.wgsl`；配方词汇与 shader 词汇**对齐或显式成数据**。
- 协议：`SCENE_SCHEMA` **加法不升**，但**显式拒未知字段**（`deny_unknown_fields` 或等价的能力位 + 报错）。
- **判据**：故意写错参数名 ⇒ **烘图时**红；改一个参数值走完全链 ⇒ **判据 P 成立**；
  现有场景重烘后**文档逐字段相同**（键会变，判据是文档等价）。
- **代价**：中。这是后面三步的地基，**先做**。

**第 2 步｜shader 图 → WGSL（裁决里的「改图」，且不要编辑器）**

- 先做 S3（schema 生成 `struct` + 入口骨架，body 手写或调库函数），再做 S4（节点图组合）。
- **没有编辑器**这一条把成本砍掉一半，但**剩下的四条一条都不能省**（§69 S4 的代价清单）：
  **source map**（生成行号 → 节点 id；naga 的 `ParseError` 自带 span，白送一半）、
  **确定性**（同图同字节；不许依赖哈希容器迭代顺序 —— Unity 自己的导入器都要先 `Sort()`）、
  **名字不是 id**（stable id + 版本 + 迁移表）、**类型规则明文**（照 Unreal：不兼容就报错，不静默截断）。
- 生成物是**纯 WGSL**；`#import` 只出现在一个函数里（WESL 是 Bevy 的方向：
  `Shader::from_wesl` 已存在 `bevy_shader-0.19.1/src/shader.rs:152`，但 ⚠ **Bevy 自己的 import 要等 0.20**
  —— 所以短期仍用 naga_oil 的 `#import`，别押注）。
- **判据**：同图两次生成逐字节相同（D）；用现有 shader 反推 schema，生成结果与手写**逐字段 + 偏移相同**；
  改图 ⇒ 重烘 ⇒ 出图，**判据 P 成立、R 成立**。
- **代价**：大（但没有编辑器，是「可做」的）。**两条降低不确定性的做法**：
  ① 先用 **`node_engine` / `bevy_shader_graph`**（真的产 WGSL、MIT/Apache-2.0）花一周打原型，
  把「DAG → WGSL → 出图」这条链走通，**但不要把产品押上去**（单人维护、文档 22%、Bevy 版本旧）；
  ② 生成物一律用 **naga `parse_str` + `Validator::validate` 自检**（naga 承诺「校验过就不会 panic」，
  且 `ParseError` 自带 span）—— 这等于把「生成器写错了」变成**构造期就能发现的错误**。

**第 3 步｜pass 图：全屏 + compute（裁决明确要 compute）**

- 文档加 `passes[]`（`kind = fullscreen | compute`）+ 预留 `targets[]`；执行器 = **一个**系统，
  按数据拓扑序逐个 pass 录制。
- **绑定布局逐 pass 由数据建**（这条只对 `Material` 不成立 ⇒ 我们要走的就是**自己建布局**那条路，
  见 §74.4）；管线键 = `(shader 内容版本, 目标格式, samples, HDR, 状态, 输入格集合)`；
  **wgpu 的管线缓存 key 不含内容** ⇒ 内容键必须自己管（`SpecializedRenderPipelines` / `Variants`）。
- **compute 的形状**：官方既有先例 —— `examples/shader/compute_shader_game_of_life.rs:110`
  把 compute 系统加在**根 `RenderGraph` schedule** 上（`.before(camera_driver)`），
  配 `pipeline_cache.queue_compute_pipeline(...)` + `get_bind_group_layout(...)` +
  `CachedPipelineState::Ok` 门控 + `Image::new_target_texture(..., RenderAssetUsages::RENDER_WORLD)`。
  ⇒ **全屏 pass 挂 `Core3d`（每相机），compute pass 挂根 `RenderGraph`（每帧）**，
  两类各一个执行器，共用同一份 pass 表与同一套内容键。
- **判据**：pass 表为空 ⇒ 与今天**逐字节相同**；开一条 pass ⇒ `pixdiff` 可判；
  每个 pass 有自己的 GPU 时间戳并进 `gpu_snapshot` 求和（`main.rs:362-367` 现在只认 Bevy 那几条）；
  坏 pass shader ⇒ **当场拒**（§62）；**判据 R**：同一会话换 pass 图，pid 不变、两份都出图。
- **代价**：中。**先做全屏，compute 紧接着同一份形状**（避免两次改协议）。

**第 4 步｜把「无顶」这件事做掉（§74.4 的那个选择）**

### §74.4 唯一还需要用户点头的一条：材质的硬顶怎么处理

裁决说「render 不需要重新编译」，而**只要 schema 还在固定超集里，就有顶**：
4 张贴图、参数块 1024 字节（`reflect.rs:25-32`）。超了就必须改 Rust ⇒ 撞判据 P。

| 选项 | 做法 | 代价 | 什么时候够用 |
|---|---|---|---|
| **(a) 加宽超集**（推荐先做） | 一次性把超集改成「参数块 + N 张 2D + M 张 cube」（例如 8 + 4）、参数上限提到 4 KB | 一次性 Rust 改动；每条管线多几个空格（§65 实测「多绑 6 个空格无代价」） | **绝大多数 schema 变更都在顶以下** ⇒ 判据 P 成立。代价最低 |
| **(b) 自建材质绘制路径**（无顶） | 不用 `MaterialPlugin`：自己建布局、自己建管线、自己走一个 draw 阶段（Bevy 的 `FullscreenMaterial` / `bevy_anti_alias` 是现成模板） | **大**：prepass / 阴影 / 变体要把 Bevy 那条路接管过来 | 只有真要做到「任意张数的贴图」时才需要。**裁决允许重构渲染器，但不建议为此先付这个钱** |

**建议**：先 (a)，把 (b) 记成明确的后备 —— 判据 P 是「在顶以下」的性质，
而 (a) 让顶高到实际用不到；只有当美术真的撞到顶，才动 (b)。

### §74.5 明确不做（裁决后仍然不做）

- **运行期改 Bevy 的 schedule**：改正在运行的 schedule 会被**静默覆盖**（§70 R2）。
- **通用 FrameGraph（任意连边）**：culling / aliasing / barrier / debug 视图全要自己写；
  Unity 用 18 个固定注入槽、Godot 用 5 个 ⇒ 我们也要**固定 stage 枚举 + 有穷 pass 种类**。
- **自研渲染器**（§1 裁决：3–6 人月到 parity）。裁决说的「可以接受切换/重构渲染器」是
  **在 Bevy 之内换路**（§74.4 (b)），不是换掉 Bevy。
- **Inspector / 图编辑器**：裁决明确不要。

### §74.6 下一步

**代码未动**。按 §74.3，第 1 步（schema 数据化：契约收口 + 配方透传 + 协议加法不升版本）是唯一的前置，
而 §74.4 那条选择（加宽固定超集 (a) vs 自建绘制路径 (b)）是路线分叉点。
另有一处待实测（本仓规矩：没跑过不算验过）：**改 WGSL 结构体 ⇒ 重烘 ⇒ 出图，全程不重编 exe**
（§67.1 那条过期笔记的更正）。这一条实测通过，判据 P 的第 1 步就算立住了。
