# build graph：用图组织**代码生成**，与数据图两阶段顺序执行

> 用户口径（2026-09-19）：**「添加一个 build graph 概念，里面用图执行组织 crate 生成，提供方便的
> 单态化 interface，和普通 graph 以两个 stage 顺序执行」**。§ 号接 `docs/system/generic-instances.md`（`px_inst!`）。

## §181 为什么是这个形状

`19` 把泛型实例做成了"声明 → `px_jit build` 生成 → 编译 → 内容寻址 → 运行期装载"。它还差三样：

1. **描述分两处**：图脚本里一行 `px_inst!`，而"要编哪些"靠 `insts.rs::catalogue()` 那张**手维护清单**
   （§179.5，只由一道"数宏调用"的门看着）；
2. **依赖是硬编码的**：`alg_roots: &[&str]` —— 参数想用别的 crate 时没有通道（`19` §179 之外的缺口）；
3. **两套 key / 两套 GC / 两个"缺了怎么办"的故事**：实例 key 与节点键是两套算法，`px_jit gc` 与
   数据 CAS 是两套口径，stage 1 靠显式批量预扫、stage 2 靠"缺就报错"。

把**代码生成本身**做成一张图，这三样一起消掉：节点 = 代码单元，**边 = "我编译时链了谁"**（于是
依赖不再是硬编码，而是图的边）、key 是同一套"内容寻址 + 构建指纹"、GC 是同一套"活键集可达"。

⚠ **一条硬边界（不因这个设计而消失）**：`Rust` 的**类型是编译期的**。build graph 组织的是
"**体**的生成"，它**造不出新的类型** —— stage 2 里那些类型化把手（`PxOp` impl、Params/Inputs/Payload）
必须在图程序编译期就存在。越过这条线就只剩 `dyn`/解释器/字符串分派（上一轮删掉的那版，
体积那一档实测 26 s，逐点解释 5–10× 是致命的）。

## §182 两个 stage

```text
px run <图>            ← 一条命令，两个 stage 顺序执行
├─ stage 1  build graph   ：节点=代码单元，边=依赖；命中就走，缺就编（**会跑 cargo/rustc**）
└─ stage 2  数据图        ：**只读装载**（今天的 `cook` 那条路，一位不改）
```

* 默认：`px run` 先跑 stage 1，**全部命中才**进 stage 2；有缺且没给 `--build`
  ⇒ 停下来报"该跑哪条命令"（保持"运行只读"这个今天用代价换来的默认值）。
  ⚠ 就地更正（2026-09-20）：当时另有一个 `PX_JIT=build` 环境变量兜底 —— **它已废弃、已从代码里删除**，
  今天是 `px run <图> --build`（或先 `px build`），见 §191.2。
* `px build` = 只跑 stage 1（`px build --gc [--deep] [--target]` 回收代码缓存）。⚠ 它**不吃图名**：
  stage 1 是 crate 的属性，不是哪张图的（`20` §191.2）。
  `tools/px.ps1` 的 task 与 driver **同名同义**：`-Task list|build|gc|run`（要图名时 `-Graph <图>`，
  要深层回收时 `-Deep`）。⚠ 参数名 2026-09-20 从 `-Target` 改成 **`-Task`** —— 原因是
  `--target` 会被 PowerShell 按前缀绑到脚本自己的 `-Target` 上、转发不进 driver。
* 两个 stage **共享同一份 key 纪律与 GC**：stage 1 的产物（实例库）在 stage 2 只按 key 取用。

## §183 build graph 的节点与边（定死的形状）

| 节点 | 是什么 | 它的 key（= 构建指纹） | 它的"算" |
|---|---|---|---|
| `Crate` | 盘上的一个 crate（alg crate、agent 的助手库、外部库） | 该 crate **可达全部源码名册**（传递闭包） | 无（它是输入；只算指纹） |
| `Toolchain` | 这一份构建的工具链 | `rustc -vV` + target + `RUSTFLAGS` + profile | 无 |
| `Contract` | 契约层（`px_graph_schema`） | `px_graph_schema::SOURCE_HASH` | 无 |
| `Inst` | 一个单态化：`(声明, 参数源文件, 体模板)` | `"px_inst/v1"` ‖ 上述三者的 key ‖ 声明指纹 ‖ **参数源字节** ‖ **体模板（去空白）** ‖ 接口形状 ‖ op id | **编一份 dylib** 到 `target/pcg/inst/<key>.dll` |

**边**：`Inst --→ Crate`（它编译时链的每一个 crate，**含通过 `deps` 显式加的**）、
`Inst --→ Toolchain`、`Inst --→ Contract`。
⇒ §181 的第 2 条自己就好了：**"参数要用别的 crate" = 加一条边**，
而它进不进 key 不再是"记得别忘了"（§177 那族洞的病根），而是**由边决定** ✓。

## §184 单态化 interface（图侧只写一处）

> ✅ **已落地（2026-09-20）——但形状与本节草图不同**：图侧**没有宏、也不手写类型**，
> 而是一张**数据收据**（`px_graphs/src/inst_recipe.rs` 的 `InstRecipe` / `INSTANCES`）+
> **stage 1 生成类型**（`px_graphs/build.rs` 写 `OUT_DIR/insts_gen.rs`，`insts.rs` `include!`）。
> `build()` 今天遍历 `recipe::INSTANCES` 调 `g.facts(…)`；"声明事实"由新 crate `px_decls` 给
> （它不进任何实现库名册 ⇒ 不换键）。设计全文与读数见 **`docs/system/codegen-types.md`**。
> ⚠ 下面这段草图里 `b.crate_()` / `b.inst::<B>()` / `px_inst!` 的写法是**当时的**设计，
> 今天对应的是 recipe 的 `roots` 那一栏与 `g.facts(…)`；"根由边决定"这条**判断仍然成立**。

```rust
// ⚠ 当时的草图（今天是 recipe + 生成物，见上面的 ✅ 与 `21`）
px_inst! { Band, "cloud.coarse/band", CloudCoarse, "art/inst/band.rs",
           |p, i, g, ARG| px_volume_alg::coarse_with(p, i.coverage.value(), ARG) }

pub fn build(b: &mut BuildGraph) {
    let alg = b.crate_("px_volume_alg");
    let noise = b.crate_("px_noise");            // ← 参数要用的别的 crate
    b.inst::<Band>(&[alg, noise]);               // ← 边：这个实例编译时链它们
}
```

⚠ 今天图侧的真实形状（照抄这个，不是上面那段）：

```rust
// px_graphs/src/inst_recipe.rs（数据）+ px_graphs/src/insts.rs（include! 生成物）
pub const INSTANCES: &[InstRecipe] = &[
    InstRecipe { op_id: "cloud.coarse/band", decl: "CloudCoarse", type_name: "Band",
                 roots: &["px_volume_alg"], source: "art/inst/band.rs", body: "…, ARG)" },
    /* … */
];
pub fn build(g: &mut px_cook::inst::BuildGraph) {
    for item in recipe::INSTANCES {
        let (interface, decl_hash) = generated::facts_of(item.type_name);
        g.facts(item.type_name, item.op_id, interface, decl_hash,
                item.roots, item.source, item.body);
    }
}
```

* **手维护清单消失**：`catalogue()` 没了 —— "要编哪些"就是 recipe 表里写了哪些条目
  （数据，和 stage 2 的图同性质）。`inst_gate` 今天守的是"**recipe 条数 == 图里节点数**"
  与"生成物与 recipe 一致"（`21` §读数 2）。
* 便利来自**类型**：`interface` / `decl_hash` 仍取自真类型，只是**问的那一侧**从"图侧宏"
  换成了 `px_decls` 声明表（build script 用它把事实写成生成物里的 const）。

## §185 与现有件的关系（不推倒重来）

| 今天 | 之后 |
|---|---|
| `inst::key(Inst{alg_roots,…})` | `inst::key(Inst{source_roots,…})` —— 根由**边**汇总（Q2 的正解） |
| `px_jit build`（扫 catalogue + 解析模板 + 生成 + 编 + 收） | stage 1 的 driver `px build`（**节点来自 `build()`**），其余步骤原样复用 |
| `px_jit gc` | stage 1 的 GC：活键集 = 本张 build graph 可达的 `Inst`（口径与数据 CAS 统一） |
| `insts.rs::catalogue()` + 计数门 | 删掉；改成两阶段一致门 |
| `tools/px.ps1 -Task inst` | 保留（= 只跑 stage 1）；新增 `-Task run` 两阶段 —— ⚠ 2026-09-20 更正：task **改名成 `list`/`build`/`gc`/`run`**、参数名 `-Target`→**`-Task`**（`inst` 作废，见 §191.2） |

## §186 判据与代价

* **R1 不破**：改 `art/inst/*.rs` 或 `deps` 里的 crate ⇒ stage 1 重编那一条实例，
  **七个图 exe 一个字节不变**（今天已实测：0.35 s、七份 exe 逐字节）✓
* 两阶段读数必须给：`px build` 热（依赖在缓存）**0.35–1 s**、冷（`--target` 清过）**14.3 s**；
  代码 CAS 与嵌套 target 的体积（实测 7.7 MB / 491 MB–1 GB）✓ 沿用 `px_jit gc` 的数字。
* ⚠ **不许**让 stage 2 默认触发编译：那会把"先证明状态是静止的，再量"这条纪律（`18` §171.5 用血换的）
  一并弄丢。`--build` 是显式请求。
* ⚠ stage 1 的节点里**只有** `Inst` 会写盘（代码 CAS）；`Crate`/`Toolchain`/`Contract` 是只读指纹节点。

## §188 B1 落地读数（本轮）

* `px_inst!` 的 alg 那一栏 → **根列表** `["px_volume_alg"]`（`INST_ROOTS`）；`catalogue()` 直接取
  **宏发出来的那份** ⇒ **根只有一处来源**（不再手抄，也不用加"两处一致"的检查）。
* **根进 key**（实测）：加第二个根 `px_verify` ⇒ key `07e3e815676d` → **`4ee60af3b45b`**；
  撤回 ⇒ 逐字节回到 `07e3e815676d` 且**命中**（CAS 按内容寻址，同一份产物）✓
* **根进生成物**（实测，双根时清空重建）：`[dependencies]` 四条都在 ——
  `px_graph_schema` / `px_volume_schema` / `px_volume_alg` / **`px_verify`**；`inst_gate` 3 passed ✓
* ⚠ 修这半**之前**的形状是"写着一半"：第二个根**进了 key 却没进 manifest** ⇒ 身份是对的、
  参数却 `use` 不到那个 crate（工具只写 `alg_roots.first()`）。
* ⚠ 又踩一次 **mtime 陷阱**：撤回用 `Copy-Item`（保留旧 mtime）⇒ cargo 判"没变"不重编
  ⇒ `list` 还报双根、manifest 也没重生成。对策：撤回后**刷 mtime**（或像本轮那样用
  `Set-Content` 重写）。这条 `18` §171.5 记过，我又犯了 —— 撤回一律"要么 `clean -p`、要么刷 mtime"。
* `§三` 漂移**根因已定**（不再悬着）：**共享指纹助手 `px_fingerprint` 被我改过**（补
  `rerun-if-env-changed`），而它是**三个实现库的 `[build-dependencies]` path 依赖** ⇒ 它的源码在
  它们的名册里 ⇒ **三个库的身份全换、所有节点键全换** ⇒ 场景文档换。
  实测通道：只给它追一行注释 ⇒ `field.fbm` 的节点键 `6ea07924bf66` → `fb3de5e8b797`、planet 6/6 重算；
  撤回 ⇒ 键回到 `fb3de5e8b797` 且**命中**。已按此**第四次重登记**（带根因）：
  `A55C3ED0391DA075` 那一批；J1 orbit-bare 一位没动、产物 **28/28** 逐字节 ✓

## §189 B2 第一片（本轮）：`BuildGraph` + `InstNode`，清单变成**派生**

* `px_cook::inst` 新增：
  * `pub trait InstNode { fn info() -> InstInfo; }` —— **类型化把手**：op id / 根 / 源文件 / 模板
    只在 `px_inst!` 那一行里写一次，别处**通过类型**取（⚠ 名字不叫 `Inst`：那个已经是 key 的输入结构）；
  * `pub struct BuildGraph` —— stage 1 的图（今天是个**记录器**：`inst::<T>()` 收节点，
    `nodes()` 就是"要编哪些"）。同 op id 声明两次**当场拒**。
* `px_inst!` 现在实现 `InstNode`（`info()` 走 `info_of::<Self>(…)`）。
* `px_graphs/src/insts.rs`：`pub fn build(&mut BuildGraph)` 是**声明处**；
  **`catalogue()` 变成 `build()` 的派生**（不再手写清单）。
* **实证（登记处唯一）**：往 `build()` 里加一行 `g.inst::<BandWide>();` + 一份宏声明 ⇒
  工具**立刻**看到 `实例 2 条`（第二条 key `49f9b87b62cc 缺`）；撤回 ⇒ `实例 1 条`、
  key 回到 `07e3e815676d`、全量 **72 个 target 全绿** ✓
* ⚠ 撤回又踩 mtime 陷阱（`Copy-Item` 保留旧 mtime）⇒ 本轮改成"重写文件 + 刷 mtime"，
  这条按 `18` §171.5 办事（撤回一律 `clean -p` 或刷 mtime）。
* 还没做的（§187 的 B2 后半 / B3）：把**执行**（按 key 命中/编译/收库）搬进 `BuildGraph`、
  `px build` 从 `build()` 取节点；`px run` 把两个 stage 串成一条命令。
  `inst_gate` 的计数门现在守的是"源码里每处宏调用都在 `build()` 里登记"（因为 `catalogue()`
  派生自它）—— 形状不变，含义更准。
  > ⚠ 就地更正（2026-09-20）：这一条**已经在 §191 做完了** —— 执行搬进了 `BuildGraph`、
  > `px run` 两阶段跑起来了；`catalogue()` 也在 §190 之后**删掉**了（现在只有 `build()` 一个来源）。

## §190 B2 后半（本轮）：工具**执行 build graph**，不再读清单

* `px_jit` 的 `list` / `build` / `gc` 三个子命令，节点来源从 `insts::catalogue()` 换成
  `nodes()`：**跑一遍 `px_graphs::insts::build()` 建出 `BuildGraph`，收它的节点**。
  ⇒ 从"读一张清单"变成"**执行图**"（`20` §182 的 stage 1 驱动）。
* 为什么工具必须这么取节点（而不是解析文本）：节点的 `interface` / `decl_hash` 要在**编译过类型**
  的那一侧才算得出（`19` §179.5）——工具遍历不了文本，只能问类型。
* 读数：`px_jit list` = `实例 1 条 / cloud.coarse/band … 07e3e815676d 有`；
  `build` = `已有 1、失败 0`；`gc` = `活 1、删 0、跳过 2`；`inst_gate` 3 passed；
  全量 **72 个 test target 全绿**、无 FAILED/error ✓
* `catalogue()` 现在只剩 `inst_gate` 在用（它是 `build()` 的派生 ⇒ 那道门守的仍是
  "源码里每处宏调用都在 `build()` 里登记"）。**下一步把它删掉**，让"取节点"只有一条路。
  > ✅ 2026-09-20：**已删**（`px_graphs/src/insts.rs` 里只有一行注释记着它）；
  > `inst_gate` 今天直接"跑 `build()` 建图取节点"（`20` §191，四道门）。

## §191 B2 后半 + B3 落地（两阶段跑起来了）

* **执行挂到了图上**：`px_cook::inst::BuildGraph` 从"记录器"升级为有 `missing()` / `compile_one()` /
  `compile_missing()`；事实来自**类型**（`InstNode`），模板**原文**来自**文本**（`inst_scan`）。
  `px_jit` 三个子命令退化成薄 driver（读数与行为不变：`list` 有、`build` 已有 1、`gc` 活 1）。
* **`px run <图> [--build] [-- 图参数…]`**：stage 1 只**计划**；全在 ⇒ 进 stage 2；有缺且没给 `--build`
  ⇒ **非零退出 + 打印确切命令**；`--build` ⇒ 先编再跑；stage 2 直接跑 `target/debug/<图>.exe`
  （**不嵌套 cargo**）。父代理独立复跑读数：
  ```
  (a) 删库后 px run planet        → [exit=1] 缺 1 条实例库（共 1 条）：07e3e815676d… + 两条命令
  (b) px run planet --build       → stage 1｜共 1 条：已编 1、已有 0、失败 0
                                    stage 2｜…\target\debug\planet.exe
                                    共 6 个节点：命中 6、重算 0        [exit=0]
  (c) 再跑一次（只读，不再编）      → stage 1｜命中 1、缺 0 → 共 6 个节点：命中 6、重算 0
  ```
* **R1（父代理独立复核）**：`cargo build` 两次坐实 → 快照七 exe → `art/inst/band.rs` 追一行注释 →
  `cargo build` ⇒ **七个 exe SHA256 逐一不变**、key `07e3e815676d` → `593e9777ccf3 缺`；
  撤回（整份 `Set-Content` 重写、刷 mtime）⇒ key **回到 `07e3e815676d 有`**、`band.rs` 逐字节相同。
* `px_jit gc --deep --target` 清嵌套共享中间物：**586.5 MB**（`target/pcg/inst` 只有 7.7 MB）。
* 测试：`inst_gate` 现在**四道门**（新增"图里登记的模板与源码里那一份相同"）；全量
  `cargo test --workspace --exclude px_render` **无 FAILED/error**。

### §191.1 三处设计裁决（父代理认可，附代价）

| 决定 | 理由 | 代价 |
|---|---|---|
| **`inst_scan` 搬进 `px_cook`**（⚠ 2026-09-20：`px_graphs::inst_scan` 那行 re-export **也删了**，只剩 `px_cook::inst_scan` 一处） | 执行住 `px_cook::BuildGraph`，而执行必须拿模板**原文**（`stringify!` 那份有损、编不了 Rust）⇒ 扫描器只能与执行同侧；它本就属于"图脚本"这个域，而 `px_cook` 是那扇门 | `px_cook` 收一个"图源目录"实参 —— 它**不认识具体图程序**（那是调用方给的） |
| **`px_cook` 新增 `build.rs`**（发 `PX_PROFILE`/`PX_TARGET`/`PX_RUSTFLAGS`/`PX_CARGO`/`PX_RUSTC_VERSION`） | `cargo:rustc-env` **不跨 crate 传播**（`px_fingerprint` 里记过）；`PX_CARGO` 编译期捕获才不怕 exe 被直接起 | 无（它**不进任何算子的名册** ⇒ 不换键；key 未变 + R1 实测） |
| **`tools/px.ps1 -Task run` 用 `-Graph <图>`** | 实测 PowerShell 会吃掉单独的 `-`、`--` 报歧义、`--%` 又让图名去撞 `-Level` | 只是包装层的手感；`px run` 本体仍是要的形状（⚠ `px_run` 这个 bin 已在 §191.2 并进 `px`；⚠ 参数名后来从 `-Target` 改成 `-Task`） |

另两条口径说明（非冲突）：`compile_missing()` = "第一条失败就回"，`px_jit build` = "剩下的也编完 +
报失败数"，两者调**同一个** `compile_one()`（没有第二条编译路）。

### §191.2 三个子命令并成一个 driver `px`（用户口径，2026-09-20）

> 用户口径：**「`px_jit` 与 `px_run` 并成一个 driver `px`」**。

```text
px list                                    计划（不编、不跑）
px build [--gc] [--deep] [--target]        stage 1：编缺的 / 回收代码缓存
px run <图> [--build] [-- 图参数…]          两阶段：计划 →（可选编）→ 跑 stage 2
```

* 理由：§191 里 `px_jit` 已经只剩"薄 driver"，`px_run` 也是"薄 driver"⇒ **两个入口、一份执行**
  没必要；用户只该记**三个词**（`list` / `build` / `run`），"编"与"跑"是子命令而不是两个 exe。
* ✅ 本节已落地：`px_jit.rs` / `px_run.rs` 两个 bin **已删**，只剩 `px_graphs/src/bin/px.rs`
  一个 driver（`list` 不认 `--gc`；`build --gc [--deep] [--target]` 里 `--deep`/`--target`
  必须与 `--gc` 同给）。`tools/px.ps1` 的 task 也跟着换成 `list` / `build` / `gc` / `run`。
  ⚠ 同一轮把脚本参数名从 `-Target` 改成 **`-Task`**：`--target` 会被 PowerShell 按前缀匹配
  绑到 `-Target` 上 ⇒ 转发不进 driver；改名之后才通。文档一律写 `-Task`。
* ⚠ 附带落地：`px_graphs::inst_scan` 那行 re-export **已删**，扫描器只有 `px_cook::inst_scan` 一处。
* ⚠ **文档一律按 `px list` / `px build [--gc --deep --target]` / `px run <图> [--build] [-- 图参数…]` 写**；
  不要再用 `px_jit` / `px_run` 的名字（它们已经不在仓里）。
* 判据不变（§186）：`--build` 仍是**显式请求**；`px run` 在有缺且无 `--build` 时**非零退出 + 打印命令**。
  ⚠ 曾经的 `PX_JIT=build` 环境变量兜底（`--build` 的旧别名）**已废弃、已从代码里删除** ——
  只留 `--build` 一条形状。这条是历史标记：谁见到旧命令里的 `PX_JIT=build`，当它是过期写法。

### §191.3 还没长的（下一轮）

§183 表里的 `Crate` / `Toolchain` / `Contract` 三类节点还没成为**图里的节点** —— 它们今天是 key 的
**隐式输入**（根列表 / 常量）。泛化它们的意义：一条实例改链的某个 crate 时，能只重编受影响的那些；
以及 `px run` 服务**多份**图程序 crate 时把多张 build graph 合起来判活（§190 记的那条口径）。

## §192 文档清理（本轮）与一处**故意不改**

* 文档代理清了 10 份文件（只碰 `*.md`）：`docs/…integration.md` §5.3 重写成真形状（`px_inst!` + `build()`
  两处声明、根列表、裸类型名、四道门、命令改 `px list|build|run` 与 `-Task`）；`19` 的 §175/§179.4/
  **§179.5 整节**加过期标记、§179.1 例子改成最终宏签名；`18` §172 加标记（被删的"生成式单态化"今天以
  `px_inst!` + build graph 回来）、§171.5 的六格标为历史并点名今天生效的 `A55C3ED0…` 批；
  `17`/`16`/`02` 加"本轮之后哪些已不在"的标记；`06`/`07`/`10`/`15` **各加一行** `-Task` 改名标记
  （那 31 处历史命令原文一字未改 —— 记录是记录），`docs/invariants.md` 三处就地改。
* 我自己补的两处：`art/inst/band.rs` 注释里的 `px_jit build` → `px build`（⚠ 这让**那条实例的 key 变了**，
  下次 `px build` 会重建它 —— 自愈 ✓）；`art/anchor/README.md` 的"重登记过三次" → **四次**。
* **故意不改（并说明理由）**：代码注释里还剩三处提到 `px_jit` 这个名字 ——
  `px_fingerprint/src/lib.rs`、`px_graph_schema/src/{contract.rs,ops.rs}`、`px_volume_alg/src/lib.rs`。
  它们**都在会被算进身份的 crate 里** ⇒ 改一个注释就要付**一次全仓换键**（所有节点键 + §三 重登记 + J1）。
  **为注释付一次换键是坏交易**，所以按"量过才改"的规矩留到**下一次这些 crate 真有功能改动时顺手改**。
  （`px_cook/src/inst_scan.rs` 那处是**刻意的历史注**（"从前 `px_jit`（那个 bin 今天叫 `px`）…"），
  它解释"为什么只有一份扫描口径"，**保留** ✓）

## §193 field 域也有真单态化实例了（本轮）：`field.remap/waves`

**A. `px_field_alg`（新 rlib，与 `px_volume_alg` 同构）**：`trait FieldFn { fn value(&self, upstream: f32, uv: [f32;2]) -> f32 }`
（与体积域同性质：纯函数、只吃 f32、实现者自己保证落 `[0,1]`）+ `Scale`（值域尺子，从 `px_field_op` 的 `Remap`
**逐字**搬出）+ **唯一那条循环** `map_grid`；两条入口走同一条：`remap_with(scale, params, upstream, grid, &F)`
（实例库入口）与 `remap_sampled(scale, input, grid)`（预置）。`px_field_op` 变薄壳。

**B. 声明**：`FieldRemap, "field.remap", "px_field_op", params::RemapParams, FieldRemapInput, Field`
（自己的 `FieldRemapInput`；`RemapParams { gain, bias, bands }` 走 `PxParams` ⇒ 全进键）。

**C. 例子（这是本轮的核心交付 —— 一个 feature 没例子等于没交付）**：`art/inst/waves.rs`（`Waves`：UV 径向条带 +
上游挪相位 + `gain`/`bias` 调对比，末了 clamp）+ `insts.rs` 两行 + **能跑的图** `px_graphs/src/bin/field_remap.rs`
（`field.fbm → Waves`，全默认参数）。`px run field_remap` 冷 = `共 2 个节点：命中 0、重算 2`（`bands` 216 ms，
`256×128｜值域 0.0000..1.0000｜均值 0.5603`）、热 = `命中 2、重算 0`。

**判据（父代理独立复核）**：`px list` **两条**实例（`cloud.coarse/band d1c8fd369338`、`field.remap/waves caa8318cda1b`）；
**R1**：只改 `art/inst/waves.rs` ⇒ **七个 exe SHA256 一位没变**、key `caa8318cda1b → d3d089ebeb87`、撤回 ⇒ 逐字节回原值；
**产物 28/28 逐字节**（planet 6/6、desert 8/8、clouds 14/14）；全量测试无 FAILED/error。

**⚠ 它自己抓到并删掉的一个隐蔽缺陷（值得记）**：`Scale::map` 第一版顺手加了 `.clamp(0,1)`，而搬出前那个循环
**没有**这次钳制 ⇒ 对 `out_min/out_max` 落在 `[0,1]` 之外的图会**静默换产物** ✗。已删（注释写清："算出来落在
`[0,1]`"是**图侧函数**自己的承诺，不是这把尺子兜的底），单测改成断言"照直映、不钳制"。
⇒ 教训：**"搬家"要把语义逐位搬，顺手加一个钳制就是静默换产物** —— 这类改动必须由"产物逐字节"的判据来抓。

**⚠ 两条新的量法坑（记进来）**：
1. 跑过 `cargo test --workspace` 之后**第一次** `cargo build` 会因**特性合并**重链七个 exe ⇒ 量 R1 必须在
   "上一次 cargo 命令是 build"的状态下量（先连跑到 `Compiling=0` 再快照）；
2. 撞到过一次 `拒绝访问 (os error 5)`（exe 被外部句柄占住）⇒ **那一趟 build 没写完**，cargo 下趟才补链
   ⇒ 探针读数会**假红**。先确认 `Compiling=0 且无拒绝访问`，两次 build 坐实，再动探针。

**§193.1 四处设计裁决（父代理认可）**

| 它报的点 | 裁决 |
|---|---|
| `remap_with` 多一栏显式 `scale`；两套 `Params` 不合并 | ✓ 认（预置/泛型必须走**同一条**循环；合并会改预置 `Remap` 的 TOML 形状 ⇒ 越界） |
| `uv` = **纹素中心**（`0.5/width … 1-0.5/width`） | ✓ 认（与 `Field::uv` 同口径，直接可喂 `sin`/距离） |
| `upstream` = 上游**过了共享尺子（钳+可选平滑）之后**的值 | ✓ 认（一份尺子在算子侧）；要"原值"是 `map_grid` 一行的事 |
| `tools/px.ps1` 的 `ValidateSet` 不枚举新图名 | ✓ **不加**（枚举图名就是一张手维护清单 —— 正是这轮在删的东西；`-Task run -Graph <图>` 已覆盖所有图） |
| `target/baseline/*.json` 里 clouds 那 14 个旧键产物已不在盘上 | ✓ 认它的做法：**改动前先留下 28 份快照**再比 ⇒ 28/28。⚠ 那三份 json 今天对 clouds 已**不可比**（记着） |

## §194 指南落地 + 一处**已知待收敛的垫片**（`Cell`）

* `docs/guides/writing-an-operator-library.md` 新增 **§5.4「场域的泛型实例 —— 与体积域逐处同构（能跑的最小例子）」**
  （§5.3 后，含图侧两处、要动的四处、冷热读数、`uv`/`upstream` 口径、预置与泛型的差别表）✓
  于是这个 feature **代码 + 例子 + 指南**齐了。
* ⚠ **文档代理报的一条已过期**：`art/anchor/README.md` 的"重登记过三次"与 `art/inst/band.rs:3` 的 `px_jit build`
  **父代理已经改掉了**（前者现在是"**五次**" —— field 那轮又重登记了一次；后者是 `px build`）。
* **`px_field_alg::remap_with` 今天写作 `F: Cell`，而 `FieldFn: Cell` 是 blanket impl**（`remap.rs:42–50`、`:160`）。
  这是为了"预置路径与泛型路径共用同一条 `map_grid`"临时加的垫片。**取名不好**（与 `std::cell::Cell` 撞概念），
  而且**它是多余的**：预置那一档本质就是"上游照抄"，用一个 `struct Passthrough; impl FieldFn for Passthrough
  { fn value(&self, upstream, _uv) -> f32 { upstream } }` 表达即可 ⇒ `Cell` 整个消失、只留 `FieldFn` 一个概念。
  **裁决：收敛方案记下，但不现在做。** 理由与 `px_jit` 注释那条**同一条规矩**：`px_field_alg` 在
  `px_field_op` 的名册里 ⇒ 改它一个字就换 `px_field_op` 的身份 ⇒ 全节点键换 + §三 再重登记一次 +
  J1 重跑。**为去掉一个内部垫片付一次换键是坏交易** ⇒ 等下一次 `px_field_alg`/`px_field_op` 真有功能改动时顺手做。
  （对外影响很小：`px_inst!` 的模板里只出现 `remap_with(...)` 的**调用**，泛型约束不露在图侧 ✓）

## §195 父代理独立复核（stage 1 生成的类型）+ 两个教训

**复核读数（我自己跑的，不是转述）**：
* 铁律 3：`px list` 两条 key **逐位** = `d1c8fd369338` / `caa8318cda1b` ✓
  （⚠ band 那条在这一刀之前是 `96d4feb75ec4`：删宏那一刀顺手改了 `art/inst/band.rs:1` 一句注释
  ⇒ **只有这一条实例重编**（`px build` 已编）、`waves` 全程未变、**anchor 未动** —— 因为 `art/inst/**`
  与 `px_cook/**` 都不在算身份的圈里）
* ⚠ **一类偶发红（不是缺陷）**：`inst_gate::a_real_instance_cooks_end_to_end` 曾有一趟报
  `CAS 里那份 <key> 解不开（IO 失败：failed to fill whole buffer）`，而那份 `.pxart` **存在且尺寸正确**
  （6591295 B）、下一趟同一个 key 就绿（`命中=true`）⇒ **Windows 文件系统瞬时读失败**。
  连续两遍全量复跑都不再出现。⇒ 遇到这类红**先复跑**，别去改代码。
* 铁律 4：`px_graphs/src` 里 `px_inst!` **零命中** ✓；生成物在
  `target/debug/build/px_graphs-*/out/insts_gen.rs`，里面有 `pub struct Band; / pub struct Waves;`、
  `const ID`/`const SYMBOL`、以及 `type Params = px_volume_schema::params::Params;`（路径由构建期
  `type_name::<T>()` 取得）⇒ **stage 2 用的类型确实是 stage 1 生成出来的** ✓
* 铁律 1：两次 build 坐实 → 给 `art/inst/waves.rs` 追一行注释 → `cargo build` ⇒
  **七个 anchor exe SHA256 一位没变**、**`insts_gen.rs` 字节不变且 mtime 不变**、**`Compiling` 一个都没有**
  ⇒ 这个形态比"图程序不重编"更强：**整趟构建什么都没重编** ✓（waves 的 key 变了 ⇒ 该实例库待重编，
  那是设计内的）
* 全量测试无 `FAILED|^error`；`px run field_remap` = `stage 1｜命中 2、缺 0` + `共 2 个节点：命中 2、重算 0`；
  `px run planet` = `共 6 个节点：命中 6、重算 0` ✓

**教训 1（我自己踩的）：untracked 文件的"撤回"必须按字节精确，并用 key 当判据。**
我给 `art/inst/waves.rs` 追探针用的 `Add-Content` 在末尾留下了一个 **CRLF**，而文件其余部分是 LF
⇒ 撤回后字节数 2336（原 2334）⇒ **key 变成 `31869dbf6811` 而不是 `caa8318cda1b`** ✗。该文件是
**untracked**（`git checkout` 救不了）。修法：**用 key 当 oracle 穷举尾部换行**（2333 ✗ / **2334 ✓**）。
⇒ 规矩：**改动 untracked 的实例源前，先记它的字节数**；撤回后必须 `px list` 看到 key 逐位回原值，
不能只看"我把文件重写回去了"。

**教训 2：`[System.IO.File]::*` 用的是进程 CWD，不是 PowerShell 的 `cd`。**
我第一次修的时候用 `ReadAllBytes('art/inst/waves.rs')` ⇒ 实际去读了 `C:\resource\planet_x\art\inst\waves.rs`
（会话工作目录）⇒ 全为空操作（好在没造成新破坏）。⇒ **`.NET` 静态方法一律传绝对路径**
（`(Resolve-Path $f).Path`），PowerShell cmdlet 才认 `cd`。

## §187 落地顺序

| 步 | 内容 | 判据 |
|---|---|---|
| B1 | `Inst.source_roots` 泛化 + `px_inst!` 加 `deps`（先把 §185 第一行做掉，它是边的底座） | 加 `deps` 的实例能编、key 随依赖源码变、R1 不破 |
| B2 | `BuildGraph`（节点/边/key/命中报告）+ `build()` 约定 + `px build` driver（复用 `px_jit` 的生成/编译/收集） | 一张 build graph 跑通；`--gc` 清非活；`catalogue` 删除、两阶段一致门替代计数门 |
| B3 | `px run`：两个 stage 顺序执行（stage 1 未全命中且无 `--build` ⇒ 停下报命令） | 两阶段一条命令跑完；stage 2 仍只读；`tools/px.ps1 -Task run`（⚠ 参数名今为 `-Task`，见 §191.2） |
| B4 | 文档：指南 §5.3 改写 + 本记录读数 | 你复核 |

⚠ 先冻源码再量 anchor（`18` §171.5）。⚠ `§三` 那格漂移仍未查清（`18`/`19` 的登记值现在是红的），
B1 动 `px_cook` 不会换 anchor，但顺手用那条二分命令把它定位掉。

> ⚠ 就地更正（2026-09-20）：上面那句"`§三` 那格漂移仍未查清"**只在本节写下的时刻成立** ——
> 它**已在 §188 查明并第四次重登记**：根因是共享指纹助手 `px_fingerprint` 被改过
> （它是三个实现库的 `[build-dependencies]` path 依赖 ⇒ 它的源码在它们的名册里 ⇒ 三个库身份全换）。
> 今天生效的是 `art/anchor/hashes.txt` §三 里 `A55C3ED0391DA075` 那一批（该文件已写对，未改动）。
> "先冻源码再量 anchor"那条**仍然生效**（它是纪律，不是这一格的读数）。
