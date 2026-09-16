# pass 表：一条数据驱动的 pass 执行器（D2 实验）

> 起因：`11-graph.md` §73 的四条裁决（不重编 / 不重启 / 不要编辑器 / 要 compute）与 §74 的四步路线。
> D1（契约收口 + 无顶 + 生成器透传）由另一条 session 走；**这一篇只记 D2**：文档里的 pass 表 + 一个执行器。
> 用户在本轮追加了两条口径，**它们改变了形状**：
> ① **不要 stage**，数组顺序就是执行顺序；
> ② **`px_pass` 不依赖 bevy**（可以依赖轻量的 wgpu 层库）；
> ③ `reads` / `writes` **不是「图」** —— 那是汇编代码的操作数，**所有同步信息由这份数据结构自己给出**。

---

## §85 三层，各自不认识什么

| 层 | 是什么 | 依赖 | **不认识**什么 |
|---|---|---|---|
| **文档** `px_protocol::scene` | `[[resources]]`（名 / 格式 / 尺寸规则 / usage）+ `[[passes]]`（kind / shader / entry / reads / writes） | serde | 没有 stage；`view` 只是文档与宿主之间的一个**约定名**（`VIEW_BUILTIN`） |
| **执行器** `px_pass`（新 crate） | `Plan` + `Executor::execute(&Device, &mut CommandEncoder, &Plan, &Frame) -> Result<String, String>` | **只有 `wgpu`**（`grep -i bevy` 在 `px_pass/` 下 **0 命中**） | Bevy、资产系统、调度、"相机"、**任何内建名字**（连 `view` 都不认识） |
| **宿主** `px_render::passes` | 文档 → `Plan`（按内容键取 CAS 成员 + naga 校验）、抽进渲染世界、建 ping-pong、**一处调用点** | Bevy | — |

**名字怎么解析**（执行器侧唯一的规则，逐条都会报错，不静默）：

1. 名字是文档声明的 `resources` ⇒ 执行器**自己建**（按 `(名, 格式, 尺寸)` 池化）并用它；
2. 否则看宿主这一帧给的外部目标（`External { name, role, view, format }`，`role` = 读 / 写）；
3. **两者同名 ⇒ 拒**（"说不清该用哪一个"）；两者都没有 ⇒ 拒，并把"文档声明了的"与"宿主给了的"都列出来。

⇒ 由此得到用户要的那条性质：**执行器里没有一个内建名字**，`view` 之所以是 `view`，是宿主给的名字。

---

## §86 实测（`.worktrees/pass-table`，2026-09-16）

命令链（`cargo run -p px_graphs --bin planet` → `shaders` → `scene orbit-bare` → `passes <场景> <配方>`）
与仪器都在 `target/passdoc/`（`run.ps1` / `pixdiff.py`，不入 git）。

### §86.1 判据四条

| 判据 | 做法 | 读数 |
|---|---|---|
| **A｜空表＝今天** | 空 pass 表的文档 vs 基准场景 | `63184151909371A5` == `63184151909371A5`；**像素差 0 / 614400** |
| **P｜不重编** | 烘图 + 6 次请求全程不跑 `cargo build`，比 exe sha256 | `2C8858917C63B704…` 在**烘图前 / 烘完后 / 收工**三次**相同** |
| **R｜不重启** | 一个服务会话（pid 24176）连吃 **4 份不同 pass 图**的文档 + 2 次坏文档 | pid 全程不变；4 张图全出，2 次当场拒 |
| **D｜确定性** | 同一份文档再请求一次 | `733408200119C203` == `733408200119C203`；**像素差 0** |

### §86.2 四份文档各自出了什么（"顺序即执行顺序"的证据）

| 文档 | pass 表 | PNG | 与基准比 |
|---|---|---|---|
| `orbit-bare`（基准） | 无 | 300012 B | — |
| `none` | 空 | 300012 B `63184151909371A5` | **0 / 614400** |
| `invert` | 1 条：`view → view` | 176865 B | **100.00%**（最大通道差 255，均值 32.84 → 249.51） |
| `invert_vignette` | 2 条：`view → view` ×2 | 390223 B | 与 `invert` 差 **99.63%**（第二条真的按顺序跑了） |
| `scratch` | 1 条写中间目标 `scratch`（rgba16float / **半分辨率**），1 条读回写 `view` | 349759 B | 与 `invert` 差 **99.71%**（均值 249.51 → 204.61） |

服务端日志里**执行器自己打的审计**（证明它录了哪几条、写到了哪种格式）：

```
pass 表：1 条（数组顺序就是执行顺序）
pass 0 'invert' 读 [view] 写 'view'（Rgba8UnormSrgb）
pass 0 'invert-to-scratch' 读 [view] 写 'scratch'（Rgba16Float）
pass 1 'vignette-back-to-view' 读 [scratch] 写 'view'（Rgba8UnormSrgb）
```

### §86.3 坏 pass 当场拒（不出图，且不把服务钉死）

| 坏法 | 拒词 | 出图 |
|---|---|---|
| 入口点写错 | `pass 'wrong-entry' 的 shader 里没有 @fragment 入口 'fs_wrong'；它有的片段入口：fs_main` | **没出** |
| `kind = "compute"`（这一版不兑现） | `pass 'reduce' 的 kind 是 compute：这一版执行器只有 fullscreen。声明了执行器不兑现的东西就当场拒 —— 静默跳过正是要避免的那种故障` | **没出** |
| 拒完再请求 `invert` | 正常出图 | ✓（明细不是锁，对齐 §62） |

### §86.4 合并之后：pass shader 统一到材质契约（§85 的「C 案」，2026-09-16）

**撞到了什么**：v2 的「槽表扫目录」（`08-renderer.md` §81.2）把 `art/shaders/*.wgsl` 里**每一份**入口
都当材质烘，而 `px_ops::write_shader` 现在**无条件**要求第 0 格是 `var<uniform> params: <struct>`
并反射出 schema —— 两份 pass shader 没有参数块（第 0 格是 `var px_source: texture_2d<f32>`），
烘图当场红：`px_grade 没有声明参数块`。

**用户裁决**：不走「分开目录」也不走「pass shader 不进 CAS」，走 **C —— 统一契约**：
pass shader 也声明参数块，`px_pass` 改用材质布局，参数一次做到位（配方能按名字给数）。

**落点**（一个数都不许抄）：

| 在哪 | 变成什么 |
|---|---|
| `px_pass::Layout` | **宿主给的** `{group, params_binding, params_align, slots}` —— 执行器仍然一个内建名字都不认识 |
| `px_render::passes::executor_layout()` | 从 `px_protocol::material` 读那张表填进去（抄一份常量进 `px_pass` 就是第二个会漂开的真相，§66.1 那颗 2/3 的雷同一族） |
| `PassPlan` | 多两格：`params`（按这份 shader 自己声明的结构体打好）与 `slots`（哪条 read 落哪一格） |
| 格位 | 由**反射出来的契约**决定：声明了几个 2D 格就必须有几个 reads，对不上当场拒 |
| 兜底贴图 | 用**同一个编码器**清成白色（执行器拿不到 `Queue`）；"没给这一格 ⇒ 采到纯白"与材质一侧同语义 |
| `PassSpec.params` | 新增；与材质走同一条 `MaterialLayout::pack`，三档在**烘图时**红 |
| `px_graphs::params` | `schema_of` / `coerce_value` 从 `bin/scene.rs` 搬来共用（pass 那条烘图路要的是同一件事） |

**判据（RTX 3060 Laptop / Vulkan / 960×640 / `orbit-bare`）**：

- **六张图与合并前的基线逐字节相同**（`target/passdoc` → `target/passaccept`）：

| 图 | 旧（合并前） | 新（合并 + C 案之后） |
|---|---|---|
| `a-base` / `a-none`（空表） | `63184151909371A5` | `63184151909371A5` ✓ |
| `r-invert` / `r-invert-again` | `9E733B8A17E3F8A2` | `733408200119C203` ✓ |
| `r-two`（invert + vignette） | `745BE24FE1467192` | `745BE24FE1467192` ✓ |
| `r-scratch`（半分辨率中间目标） | `2D61519C6544E3C1` | `2D61519C6544E3C1` ✓ |

> ⚠ **就地更正（`15-render-wgpu.md` §146.5 实测，2026-09-16）**：上表"旧"那一栏的
> `r-invert` 原来写的是 **`9E733B8A17E3F8A2`**，那是**笔误** —— 与"新"栏的
> `733408200119C203` 本应是同一个数（这一列说的是"绑定组换了、贴图格挪了、多了一个参数块，
> 而一个像素都没动"）。两处磁盘产物（`target/passdoc/r-invert.png`、
> `target/passaccept/r-invert.png`）与那一轮重出的图**三份哈希全是 `733408200119C203`**、
> 176865 字节、`--diff` 差异 0 / 614400。同一次核查把另外三格也对了（`745BE24FE1467192`
> / `2D61519C6544E3C1` / `63184151909371A5`），都对。**记这一笔是因为接的人会拿这张表当权威。**

  ⇒ 绑定组从第 0 组挪到**第 3 组**、贴图从第 0 格挪到**第 1 格**、**多了一个参数块**，
  而**一个像素都没动**。§86.1 那四条判据（A/P/R/D）也全过（A、D 的读数与上表一致）。
- **新能力有判据**：`art/passes/grade_half.toml` 的 `strength = 0.5` 在纯反相上是**数学上的平场**
  （`mix(x, 1−x, 0.5) ≡ 0.5`）⇒ 实测 `min = max = 188 = sRGB(0.5)`，
  证明 uniform 里到的就是精确的 0.5（不是 0、不是 1、不是垃圾）。
- **坏参数当场拒（烘图时）**：`pass 'bad' 不认识参数 'strengh'：这份 shader 声明的参数：strength: f32`。
- `cargo test` 全绿（`px_render` 26 单测 + 集成 11，含新增 2 条「格位/参数块当场拒」）。

**一条被这次实测改掉的旧话**：`art-framework.md` 原来写着「pass 的绑定布局不像材质那样固定」——
现在**不成立了**，两套合并成一套。旧文档里那句已就地更正。

---

## §87 ⚠ 过程中被门抓到的两个**真 bug**（这一节比上面的读数值钱）

### §87.1 "同一条 pass 不许同时读写同一个目标" 把 ping-pong 的正典也拒了

- **症状**：`invert` 的配方在烘图时就被自己的 `check()` 拒：`第 0 条 pass 'invert' 同时读和写同一个目标`。
- **根因**：我把**名字**当成了**纹理**。`reads = ["view"], writes = ["view"]` 在"读的是 A、写的是 B"时完全合法 —— 那正是 ping-pong。
- **修法**（两处，位置不同）：
  - 文档声明的 `resources`：仍是静态禁止（执行器只给它**一张**纹理）；
  - 宿主给的外部目标：允许，改成**执行期按视图身份判**（wgpu 的 `TextureView` 有按句柄的 `Eq`/`Hash`，`impl_eq_ord_hash_proxy`）—— 读到的视图与写目标**是同一个对象**才拒。
- **被谁抓到**：配方生成器里那一句 `spec.check()`（烘图时红），不是出图之后。

### §87.2 读、写两格同名 ⇒ 宿主集合里 `find(name)` 取到了同一项

- **症状**：第一版跑通后，`invert` 在执行期被拒：`读目标与写目标是同一个视图`。
- **根因**：宿主为一条 pass 准备了一个名字集合，读那一格是 `pair.source`、写那一格是 `pair.destination` —— **两个条目同名**，而执行器按名字 `find` ⇒ 两处都拿到第一个（source）。
- **修法**：外部目标加 **`role`（读 / 写）**，解析按 `(name, role)`。
- **教训**：这正是"**所有同步信息由数据结构给出**"缺的那一格 —— 我漏了"同一个名字在不同角色上是不同的东西"。

---

## §88 没做 / 已知缺口（都是明账）

- **compute 只有"声明了 ⇒ 当场拒"**，没有实现（§73 裁决要它，是下一步的第一件）。
- **pass 没有自己的 GPU 时间戳**：Bevy 那几条 pass 的读数不受影响，但新 pass 目前不进 `gpu_snapshot` 的求和。
- **宿主只提供一个外部名字 `view`**：环境 cube、深度、上一帧、storage 都还没有路。
- **一条 pass 只画一个颜色附件**（MRT 没有）。
- **pass shader 有 `#import` 一律拒**（不跑 naga_oil）：要么离线组装成自足的 WGSL，要么以后接组装。
- **没有逐帧账**：审计只在 plan 变化时打一次；没有"这一帧录了哪几条"的逐帧记录。
- ⚠ **没验过的**：多相机（`--sheet` 12 格）下每条 pass 每格都会跑一次 —— 语义上每台相机有自己的 ViewTarget，但**没实测**。

---

## §89 文件清单

| 文件 | 性质 |
|---|---|
| `px_pass/Cargo.toml`、`px_pass/src/lib.rs` | **新 crate**：唯一依赖 `wgpu`（`features = ["wgsl"]`）；全屏 pass 执行器 + 资源池 + 管线/绑定布局缓存。**布局是宿主给的**（`Layout { group, params_binding, params_align, slots }`），执行器里没有一张写死的绑定表（§86.4） |
| `px_protocol/src/scene.rs` | `PassResource` / `PassSpec` / `VIEW_BUILTIN` + `check()` + `audit()`；`resources`/`passes`/`passes[].params` 用 `skip_serializing_if` ⇒ **空表与无参数不落盘**，旧文档逐字节不变 |
| `px_protocol/src/material.rs` | 材质契约（表 + 类型 + `pack`）—— pass 与材质**共用这一份**（§86.4 的 C 案） |
| `px_render/src/passes.rs` | **新**：`executor_layout()`（契约 → 执行器布局）、`resolve()`（文档→Plan，含 naga 校验、契约反射、参数打包与格位分配、compute 的能力对账）、`extract_passes`、`run_passes`（宿主侧 ping-pong）、`PassPlugin` |
| `px_render/src/main.rs` | `Built`/`DocumentScene` 带 `passes`；`SceneTools` 多两格；`pipeline_gate` 多一道 pass 失败；`drive` 多一个 `Res`；两处 `.add_plugins(passes::PassPlugin)` |
| `px_graphs/src/params.rs` | **新**：`schema_of` + `coerce_value` + `merge_named` —— `scene` 与 `passes` 两条烘图路共用（§86.4） |
| `px_graphs/src/bin/passes.rs` | **新**：`art/passes/<名>.toml` 配方 → 带 pass 表的场景产物（**图的作者是生成器**，不是渲染器）；参数按契约透传，三档烘图时就红 |
| `art/passes/{none,invert,invert_vignette,scratch,bad_entry,compute,grade_half}.toml` | 7 份配方（`grade_half` 是参数那条路的判据：`strength = 0.5` ⇒ 平场 sRGB(0.5)） |
| `art/shaders/px_grade.wgsl`、`px_vignette.wgsl` | 两份 pass shader：**与材质同一条绑定契约**（参数块在第 0 格、源纹理第 1 格、采样器第 2 格、`#{MATERIAL_BIND_GROUP}` 占位符），自足 WGSL 无 `#import` |
| `px_graphs/src/bin/shaders.rs` | **不用改了**：槽表从写死的数组改成扫 `art/shaders/*.wgsl`（v2，`08-renderer.md` §81.2）⇒ 两份 pass shader 自动进表 |

**执行器与宿主之间唯一的接口**（这就是将来搬走时要换的那一样）：

```rust
pub struct External<'a> { pub name: &'a str, pub role: Role, pub view: &'a TextureView, pub format: TextureFormat }
pub struct Frame<'a>    { pub width: u32, pub height: u32, pub sets: &'a [Vec<External<'a>>] }
```

---

## §90 与 §74 / §75 的关系

- §74.3 的**第 3 步（pass 图：全屏 + compute）**：全屏这一半**做完了**，compute 只有能力对账。
- §75.3 那条"把执行器写成与宿主无关的一层"：现在有了**编译期保证**（`px_pass` 依赖里没有 bevy），
  剩下的宿主耦合只有一处调用点与一个 `ViewTarget`（§85 表里第三行）。
- §74.5 "不做通用 FrameGraph"：仍然不做 —— 没有拓扑排序、没有 alias、没有 barrier 推导；
  `reads` / `writes` 只当**操作数**用（解析 + 绑定 + 立刻能报错的检查）。
