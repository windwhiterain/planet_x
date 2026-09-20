# 泛型算子的自动单态化：内容寻址的**代码**缓存 + 运行期装载（`px_inst!`）

> ⚠ **图侧的 `px_inst!` 宏已经没有了（2026-09-20）**：图侧今天是一张**数据收据**
> （`px_graphs/src/inst_recipe.rs`）+ **stage 1 生成类型**（`px_graphs/build.rs` 写
> `OUT_DIR/insts_gen.rs`，`insts.rs` `include!` 它）—— 设计全文与读数见
> **`21-codegen-types.md`**；宏本体也已作为死代码删除（`px_cook/src/lib.rs` 只剩一行注记）。
> 本篇**唯一没变**的是 `px_cook` 里那套机制：key 的算法与各轴、
> 内容寻址的实例库、两道握手、按 key 运行期装载、`px build` / `px run <图> --build` 是唯一编译入口。
> 读本篇时：凡是"图侧写一行宏 / 工具扫文本找宏调用 / 替换 `$arg`"的地方**都已过期**
> （下面 §175/§179.1/§179.4/§179.5 各自另有过期标记），而"key 里必须有源码名册""两道握手"
> "缺库报错带命令"这些**仍然生效**。

> 用户口径（2026-09-19）：**「泛型自动单态化缓存再动态链接」**，并选定三件事：
> ① 装载入口放**契约层**（接受一次换键）；② 泛型参数的代码住**具名源文件**；
> ③ 实例构建由**显式命令**触发（默认跑图 = 只读装载）；④ **v1 合并 v2**：agent 只写泛型参数，
> 算法复用（所以体积域的泛型体要搬进 alg rlib）。
> § 号接 `18-operator-libraries.md`（M1：声明住 schema、实现住 dylib、按身份装载）。

## §174 它买到什么、不买到什么

| | 今天 | 加 `px_inst!` 之后 |
|---|---|---|
| 改**实现体**（`px_*_op/src/ops/x.rs`） | 只重编那份库，图 exe 字节不变 ✓ | 同 |
| 改**泛型参数**（agent 写的那个函数） | 只能写进图脚本 ⇒ **重编图程序** | 写 `art/inst/<名>.rs` ⇒ 重编**实例库**，图 exe 字节不变 ✓★ |
| 加一个新泛型算子 | 改图脚本（类型 + 体） | 图里一行声明 + 一个源文件 |

**不买**：跨机器共享代码缓存（只在 `target/pcg/inst/`，机器本地，与键同性质）；`dyn` 泛型参数；
运行期默认调 cargo（默认缺实例就报错带命令，**缺件要用 `--build` 显式请求**
—— 见 `20-build-graph.md` §182/§186）。⚠ 当时另有一个 `PX_JIT=build` 环境变量兜底，**后来废弃并已从代码里删除**。

## §175 形状（图侧只写一行）

> ⚠ 标题里的"只写一行"**已过期**（2026-09-20）：图侧现在是**一行数据**（`inst_recipe.rs` 里的一条
> `InstRecipe`），类型由 stage 1 生成 —— **既没有宏、也不手写类型**（`21-codegen-types.md`）。
> 下面这一行是**这一轮的形状**——第 4 段当时是**单个** alg crate 字面量。
> 它今天成了**根列表** `["px_volume_alg"]`（可多个，= 这个实例编译时链了谁，`20-build-graph.md` §183，
> 今天住在 recipe 的 `roots` 那一栏）。以 `19` §179.1 与 `21-codegen-types.md` 为准。

```rust
px_inst! { Mine, "cloud.coarse/mine", volume::CloudCoarse, ["px_volume_alg"], "art/inst/mine.rs" }
// 用起来与寻常算子一样：
let coarse = cook::<Mine>(&graph, "coarse", volume::CloudCoarseInput { coverage })?;
```

```text
art/inst/mine.rs（agent 写：只写泛型参数，比如 impl FieldFn for Mine）──┐
契约指纹 + 工具链指纹 + 声明指纹 + 源码名册（根列表里每个 crate）+ 接口形状 + id ─┤ px_cook::inst::key
                                                                      ↓
px build: key → target/jit/<key>/{Cargo.toml,src/lib.rs} → cargo build（独立 target-dir）
                                                                      ↓
target/pcg/inst/<key>.dll + <key>.json（sidecar：key / toolchain / 符号名 / 源名册）
                                                                      ↓
运行期：ops::load_at(path, "px_inst__CloudCoarse") → 契约握手 + **工具链握手** → 调体
```

⚠ 就地更正（2026-09-20）：上面 `px build` 那一行当时写的是 `px_jit build`；今天对外的命令是
`px list` / `px build` / `px run <图> [--build]`（`px_jit` / `px_run` 本轮并成一个 driver `px`，
见 `20-build-graph.md` §191.2）。key 那一行里的 "alg 源码名册" 今天是**根列表里每个 crate** 的名册。

## §176 三条硬守卫

1. **key 由装载器自己的代码算** ⇒ 无法陈旧：算 key 的代码一变，全部 key 跟着变，旧实例库只是孤儿。
   没有清单、没有索引表（`18` 那两条洞的教训：靠人维护的清单必然漏）。
2. **两道握手**：契约（`__contract_hash`）+ **工具链**（`__toolchain_hash` = `rustc -vV` + target
   + `RUSTFLAGS` + profile 的 blake3）。跨 dylib 的 `extern "Rust"` ABI 从此是**验过的**，
   不是"侥幸同一份 rustc"。⚠ `DEBUG`/`opt-level` **不进**这个哈希：它们不动布局
   （而 DLL 的文件字节从不进任何键），进了只会制造"两个 workspace 必须逐字对齐"的无谓摩擦。
3. **缺实例/编译失败**：报错带该跑的那条命令（今天是 `px run <图> --build` 或先 `px build`）
   + 缺哪个 key；rustc stderr 原样透出；失败**不写缓存**。

## §177 为什么 key 里必须有 **alg 源码名册**（这条最容易漏）

`O::interface()` 只哈希三个**类型名**（`px_volume_schema::ops::CloudCoarseInput` 这种字符串），
**不含字段布局**。若 `CloudCoarseInput` 加一个字段：图程序会重编（它链 schema），而实例库
的 key 不变 ⇒ 复用一份按**旧布局**编出来的 DLL ⇒ 越界读写。所以 key 必须覆盖
**声明所在 crate + alg crate 的源码指纹**。图程序不能 cargo 依赖 alg crate（那会弄丢 R1）
⇒ 只能**从盘上算**其 `src/` 名册 ⇒ 指纹算法必须抽成一个 **rlib**（`px_fingerprint`），
被 build.rs（`[build-dependencies]`）与 `px_cook::inst`（`[dependencies]`）**共用同一份**。

> ⚠ 就地更正（2026-09-20）：这一节今天泛化成了**根列表**（`INST_ROOTS`，可多个 crate）——
> 上面说的 "alg crate" 就是列表里的第一个元素；键覆盖的是**根列表里每个 crate** 的名册
> （`20-build-graph.md` §183/§188）。理由与本节一模一样，只是从"一个"变成"一组"。

## §179 接口定稿（实现按这一节写，别再改形状）

### §179.1 `px_inst!`：四个名字 + 一段**体模板**

> ⚠ **已过期（2026-09-20）：宏在本篇之后的两轮里已经被删了（连本体一起）。** 今天图侧写的是
> `px_graphs/src/inst_recipe.rs` 那张**数据收据**（`op_id` / `decl` / `type_name` / `roots` /
> `source` / `body`），类型由 `px_graphs/build.rs`（stage 1 的**计划**段）生成到
> `OUT_DIR/insts_gen.rs`、`insts.rs` `include!` 进来 —— 见 **`21-codegen-types.md`**。
> ⚠ 本节的以下口径**仍然生效**、只是换了个落点：根列表（= recipe 的 `roots`）、
> 体模板里的占位符（recipe 的 `body`，**仍是 `ARG`**，仍是 key 的一轴）、
> "体逐字抄进生成物、只换那一位"（今天 `InstRecipe::generated_body()` 整词替换）、
> 生成物里包名固定 `px_inst` ⇒ 符号名 `px_inst__<声明名>`、声明那一栏不许带路径。

```rust
// ⚠ 已废弃的旧写法（今天图侧零宏 —— 别照抄）
px_inst! {
    Mine,                                   // 图侧算子类型名（宏声明 `struct Mine;`）
    "cloud.coarse/mine",                    // 它的 op id（进键、进读数）
    volume::CloudCoarse,                    // 复用哪个**声明**（Params/Inputs/Payload 全从它取）
                                            // ⚠ 这一栏写**裸类型名**（同文件 `use`）—— `stringify!` 拼 `SYMBOL`
    ["px_volume_alg"],                      // ⭐ 第 4 栏 = **根列表**：编译时链的 crate（目录名），可多个
    "art/inst/mine.rs",                     // 泛型参数住哪（agent 写的那个文件，git 里）
    |p, i, g, $arg| px_volume_alg::coarse_with(p, i.coverage.value(), $arg)   // 体模板
}

// 同一个文件里还要写 stage 1 的**图**：上面那行只声明，这一行才点名"要编它"
pub fn build(g: &mut px_cook::inst::BuildGraph) { g.inst::<Mine>(); }
```

> ⚠ 就地更正（2026-09-20，`20-build-graph.md` §183/§189/§190）：第 4 栏是**根列表**
> （`INST_ROOTS`，`["px_volume_alg"]`，可多个）—— 它今天住在 **recipe 的 `roots` 那一栏**，
> 而 `build()` 改成遍历 `recipe::INSTANCES` 调 `g.facts(…)`（`21-codegen-types.md`）。
> 根只有一处来源，**没有** `catalogue()` 那份手抄清单了。

* **体模板里的占位符指代泛型参数**（一个类型名）。工具把模板**逐字**抄进生成物、只把那一位
  换成真类型名 ⇒ 不需要跨 crate 的宏管线、生成器也不必知道任何算子的输入怎么接。
  ⚠ 2026-09-20：占位符今天**仍叫 `ARG`**，住在 recipe 的 `body` 那一栏，替换由
  `InstRecipe::generated_body()` 做**整词替换**（`ARG` → `&<type_name>`）；`$arg` 那个写法随宏一起没了。
* 生成物 `target/jit/<key>/src/lib.rs`（⚠ `px_body!` 收的是**裸标识符**，不接路径 ⇒ 先把声明的
  真路径 `pub use` 进来，再写裸名 —— 实测 `px_body! { volume::CloudCoarse, … }` 会 `no rules expected ::`）：
  ```rust
  //! 生成物（`px build` 写的）。别手改 —— 改 art/inst/mine.rs 里的源文件，然后重跑。
  #[allow(unused_imports)]
  pub use px_volume_schema::ops::CloudCoarse;      // 引进来当裸名 `CloudCoarse`
  include!("<绝对路径>/art/inst/mine.rs");          // 泛型参数那一份
  px_graph_schema::px_body! {
      CloudCoarse,
      |p, i, g| px_volume_alg::coarse_with(p, i.coverage.value(), &Mine)   // ← 真类型名，不是占位符
  }
  px_graph_schema::px_impl_lib!();
  ```
  ⚠ **今天 `px_body!` 只收两段**（声明 + `|p, i, g|` 体）：体里直接写**真类型名**（`&Mine`），
  因为 `include!` 之后那个类型就在作用域里 —— "四位参数 + 替换 `$arg`"是旧形状（`21` 把
  占位符留给 key 那一轴，不再进生成物）。
  包名固定 `px_inst` ⇒ 符号名是**编译期字面量** `px_inst__CloudCoarse`（两边都写得出）。
  ⚠ 生成物里的路径一律用**正斜杠**：Windows 反斜杠在 TOML 字符串与 Rust 字面量里都是转义。
* 图侧那个类型**不再由宏展开**，而是 stage 1 生成出来的一段：`PxOp` impl 里
  `LIB = ""`、`SYMBOL = "px_inst__<声明名>"`、三个关联类型 = `<声明 as PxOp>` 的、
  覆盖 `source_hash()`（返回实例 key）与 `render()`（`ops::load_at::<Self>(path, SYMBOL)` 再调）
  —— 形状与当年宏展开的**逐字相同**（`21` §四件新东西 ③）。

### §179.2 key（`px_cook::inst::key`）—— 一个轴都不许漏

```
blake3("px_inst/v1" ‖ toolchain ‖ px_graph_schema::SOURCE_HASH ‖ 声明crate::SOURCE_HASH
       ‖ alg 名册哈希(px_fingerprint) ‖ O::interface() ‖ op_id ‖ 泛型参数源文件字节)
```
* `toolchain` = `$RUSTC -vV` + `TARGET` + `RUSTFLAGS` + `PROFILE`（**不含** `DEBUG`/`opt-level`，见 §176）。
  ⚠ **运行期不许算它**（子代理实测）：`TARGET`/`PROFILE`/`RUSTFLAGS` 是 cargo **只给 build script**
  设的环境变量 ⇒ 同一个函数在 build.rs 与运行期会算出两个值（`b5b4454c…` vs `08aac4ea…`），
  握手会把每一个库都拒掉。所以这一轴在两处都取**编译期嵌进各自构件的常量**：图程序侧
  `px_graph_schema::TOOLCHAIN_HASH`，库侧它自己 build.rs 发的 `__toolchain_hash` ——
  两个常量比，运行期一个字都不用算。`px_fingerprint::toolchain_hash()` **只给 build.rs 用**。
  另发三个原料（`PX_TARGET`/`PX_PROFILE`/`PX_RUSTFLAGS`）给**工具**转给嵌套 cargo，
  保证生成的实例库与图程序算出同一个值（今天 RUSTFLAGS 恰好空，一有人带就会看出来）。
  ⚠ 2026-09-20 补：工具链那几样（`PX_PROFILE`/`PX_TARGET`/`PX_RUSTFLAGS`/`PX_CARGO`/`PX_RUSTC_VERSION`）
  今天由 **`px_cook/build.rs`** 发（`cargo:rustc-env` 不跨 crate 传播 ⇒ 靠它自己的 build.rs 捕获）；
  它**不进任何算子的名册** ⇒ 不换键（`20` §191.1）。
* 声明/alg 的源码名册**从盘上算**（不能 cargo 依赖 alg crate，否则丢 R1）⇒ `px_fingerprint` rlib。
* 工具与图程序**调同一个函数**算 key ⇒ 不可能对不上（对不上就是工具库过期，握手会拒）。

### §179.3 契约层新增（S2，也是这一次换键的唯一原因）

* `ops::load_at<O: PxOp>(library: &str, symbol: &str) -> Result<Body<O>, String>`（复用查表/握手/告警）。
* `pub fn toolchain_hash() -> &'static str`；装载时与库的 `__toolchain_hash` 比，不符**当场拒**。
* `px_impl_lib!` 多导一个 `__toolchain_hash`；`px_op!` 里嵌 `const DECL_HASH = env!("PX_SOURCE_HASH")`
  （自动 = 声明所在 crate 的指纹，给 §179.2 用；三个 schema 因此各要有 `build.rs`）。
* `library()` 的缓存键从 `&'static str` 改成 `String`（实例库的名字是运行期算出来的路径）。

### §179.4 `px_jit`（S4）：只有两个子命令

> ⚠ 已过期（2026-09-20，`20-build-graph.md` §185/§190/§191）：`px_jit` 已退化成**薄 driver**，
> **执行**住在 `px_cook::inst::BuildGraph`（`missing()` / `compile_one()` / `compile_missing()`）；
> 节点来源不再"扫文本得清单"，而是跑 `px_graphs::insts::build()` **执行 build graph** 取节点
> （`interface` / `decl_hash` 只有编译过类型的那一侧算得出）。今天对外的命令是
> `px list` / `px build` / `px run <图> [--build]`（`px_jit` / `px_run` 本轮并成一个 driver `px`）。
> ⚠ **2026-09-20 再补一刀**：连"扫 `px_graphs/src/**/*.rs` 找 `px_inst!` 取模板原文"这一条
> **也没有了** —— 模板原文今天住在 recipe 的 `body` 那一栏（`21-codegen-types.md`），
> 工具读的是 build script 写的**旁挂件** `OUT_DIR/insts_gen_catalogue.rs`。

`px_jit list`：扫 `px_graphs/src/**/*.rs` 找 `px_inst!`，打印 `(id, 声明, alg, 源, key, 有/缺)`。
`px_jit build`：对每个缺的 key → 生成 → `cargo build --manifest-path target/jit/<key>/Cargo.toml
--target-dir target/jit/target` → 把 DLL 收到 `target/pcg/inst/<key>.dll` + 写 sidecar `<key>.json`。
缺实例时运行期的报错必须带这条命令。

## §179.5 实例目录：为什么留**一行**登记（以及它凭什么不会烂）

> ⚠ **整节已过期（2026-09-20）**：`catalogue()` 那份手维护清单**已删除**，"数宏调用 vs 清单条数"
> 那道计数门也**不再存在**。今天节点只有一个来源：跑 `px_graphs::insts::build(&mut BuildGraph)`
> —— 见 **`20-build-graph.md` §184**（单态化 interface：声明处只有 `build()`）、
> **§189**（`BuildGraph` + `InstNode`，清单变成**派生**）、**§190**（工具**执行图**取节点，
> `catalogue()` 删掉）。
> 本节末尾 `19` §180 那两条"本轮没做完"（计数门在改、J1 只抽查 orbit-bare）**后来都做完了**：
> 计数门进了 `inst_gate`（今天四道门，`20` §191），J1 六张全跑（`art/anchor/hashes.txt` §一）。
> ⚠ 本节**唯一仍然成立**的机制性理由：`interface()` 与 `decl_hash()` 只有**编译过类型**的那一侧
> 算得出 ⇒ 工具必须以**图程序**的身份算 key ⇒ 它只能"问类型"（跑 `build()`），遍历不了文本。
> 保留本节是为了记住"手维护清单"这条形状为什么被咬过两次。

key 里必须有 `O::interface()`（三个类型名）与 `O::DECL_HASH`（声明所在 crate 的指纹）——
这两样只有**编译过类型**的那一侧算得出来；而工具只解析文本 → 工具拿不到 ⇒ 工具必须以
**图程序**的身份算 key。于是实例声明住 `px_graphs/src/insts.rs`（lib 模块，所有 bin 都看得到），
每个实例除 `px_inst!` 之外再登记**一行**（`catalogue()` 里的 `InstInfo`）—— 工具遍历它。

⚠ 这是"手维护清单"，正是 `18` 里被咬过两次的形状，所以配一道门：
`px_graphs/tests/inst_gate.rs` 数 `src/**` 里 `px_inst!` 的出现次数与 `catalogue().len()` 比，
不等就红。清单**紧挨着**声明（同一个文件、同一行的下一行），且漏了会当场红 ⇒ 可以接受。
（要彻底免清单就得引入 `inventory`/`linkme` 这类"链接期注册"或运行期注册表 —— 本仓已明确
不要那种机制。）

## §180 读数（v1+v2 落地当轮）

* **实例链路**：`px_jit list`（`cloud.coarse/band`，缺）→ `px_jit build` → `target/pcg/inst/<key>.dll`
  （8.0 MB）+ `<key>.json`（360 B，`"symbols":"px_inst__CloudCoarse"`，toolchain 与
  `px_graph_schema::TOOLCHAIN_HASH` 一致）。生成物导出四个符号
  （`px_inst__CloudCoarse` / `__source_hash` / `__contract_hash` / `__toolchain_hash`，子代理用
  Win32 `LoadLibraryW`+`GetProcAddress` 直接验过）。
  ⚠ **"装载成功"这一条本轮拿到了**（修完两条路径口径之后，`inst_gate` 端到端第一次真调体）：
  ```
  重算 band  cloud.coarse/band @0cb9f423  2610dd56d3f1  1442 ms  6591295 B  6 面 × 65² × 65 层
             ｜值域 -0.0008..0.0033｜均值 0.0014
  命中 band  cloud.coarse/band @0cb9f423  2610dd56d3f1      0 ms  6591295 B  （同上）
  实例算子：65×65×1647750｜命中=false → true｜key 2610dd56d3f1
  test result: ok. 3 passed; 0 failed
  ```
  ⇒ 独立 workspace 编出来的实例库，被运行期装载、过两道握手、调进 agent 写的泛型参数、
  烘出 6×65²×65 的体积（6.6 MB），**第二次命中**。泛型 → 单态化 → 缓存 → 动态链接，闭环。
* **本轮抓到的两条真缺陷（都是"写着像能用、其实必然失败"）**：
  1. `ops::open` 只按**包名**找库（`format!("{DLL_PREFIX}{name}{DLL_SUFFIX}")` 去几个目录里搜）
     ⇒ 实例库传进来的是**绝对路径**，被再拼一次 `lib….dll`，永远找不到。修法：路径档原样用，
     包名档走老搜索；两档在**同一道握手**后面（陈旧告警只对包名档做 —— 实例库没有"同名目录"可查）。
  2. `SYMBOL = concat!("px_inst__", stringify!($decl))`：`stringify!` 对**多段路径**带空格
     （`"volume :: CloudCoarse"`）⇒ 符号名拼错。修法：声明那一栏写**裸类型名**（同文件 `use`）。
* **R1 变体（这一轮最值钱的那条）**：只改 `art/inst/band.rs` ⇒ `cargo build` **0.35 s**、
  **七个图 exe（planet/desert/clouds/scene/shaders/passes/field_probe）一个字节都没变**；
  实例 key `8e0b765e8a1c…` → `badcca4e1bc9…`（`px_jit list` 从"有"变"缺"）→ `px_jit build` 编 1 条；
  撤回后**逐字节回到** `8e0b765e8a1c…` 且"有"（旧 DLL 还在 store 里 ⇒ 零重编）。
  ⚠ 量法坑又踩了一次：`cargo run -p px_graphs --bin px_jit` 与 `cargo build --workspace` 的
  特性合并不同，会让 `px_jit.exe` 无谓重链 —— **读 key 直接跑 `target/debug/px_jit.exe`**，
  别经过 `cargo run`。
* **产物逐字节 28/28**（planet 6/6、desert 8/8、clouds 14/14）—— 换键之后仍然。
  `art/anchor/hashes.txt` §三 六格随之第三次重登记（`B49C426247AF996A` 那一批）。
* **工具链握手现场验过**：薄壳重编后 `clouds` 跑到新 DLL，`__contract_hash` 与 `__toolchain_hash`
  两道都过（子代理读数）；十四节点先全重算、再全命中。
* **本轮抓到的真缺陷**：`SYMBOL` 用 `concat!("px_inst__", stringify!($decl))`，而 `stringify!`
  对**多段路径**带空格（`"volume :: CloudCoarse"`）⇒ 装载按那个名字找不到符号。
  修法：声明那一栏写**裸类型名**（同文件 `use`），宏文档写死这条。
* **文本扫描的坑**：注释/文档里出现那个宏名带感叹号的写法，会让"按文本找调用"的扫描器
  **从注释里开始配平括号**，报"字符串字面量没有收尾"（实测）。⇒ 注释里不写它。
  ⚠ 工具与门**共用同一个扫描器**，否则计数门会越写注释越红。**2026-09-20 更正它住哪**：
  今天只有一个位置 **`px_cook::inst_scan`**（执行住 `px_cook::inst::BuildGraph`，而执行必须拿模板
  原文 ⇒ 扫描器与执行同侧，`20` §191.1）；`px_graphs` 那行 `pub use px_cook::inst_scan;` 的
  re-export **也一并删掉**（搬家搬干净，图侧要扫就写 `px_cook::inst_scan`）。
* ⚠ **`px_fingerprint` 自己也在名册里**（它是各 crate 的 `[build-dependencies]` path 依赖）⇒
  动它一个字符，三个 schema、四个库、`px_volume_alg`、`px_graphs` 的身份**全换**。
  这一轮本来就要全换，下一轮谁去改它的注释，就得再付一次。
* **本轮没做完**：`inst_gate` 的计数门（在改）；J1 只抽查了 orbit-bare（产物逐字节相同 ⇒
  像素必然相同，但"六张全跑"这条证据本轮没重跑）。

## §178 文件清单与顺序（一次换键：所有结构改动做完再编）

> ⚠ 已过期（2026-09-20）：这是**那一轮的施工单**。其中 S4 的 `px_jit` 与
> `tools/px.ps1 -Target inst` 都不在了（两个 driver 并成一个 `px`；脚本 task 换成
> `list`/`build`/`gc`/`run`、参数名 `-Target`→`-Task`；见 `20-build-graph.md` §191.2）；`inst_scan` 也只剩
> `px_cook::inst_scan` 一处。留在这里是为了看清"一次换键"的顺序纪律（最下面那条仍然生效）。

| 步 | 谁 | 内容 |
|---|---|---|
| S0 | 本记录 | 这份方案 |
| S1 | 子代理 A | `px_fingerprint`（rlib：纯哈希）+ 各 `build.rs` 改用 `[build-dependencies]`；`px_volume_alg`（rlib）：`bake<F: FieldFn>`/`SampleField`/helpers 从 `px_volume_op` 搬出，`px_volume_op` 变薄壳（预置实例）+ 导出 `SOURCE_HASH` |
| S2 | 我 | 契约：`ops::load_at(library, symbol)`、`toolchain_hash()`、工具链握手、`px_op!` 里嵌 `DECL_HASH = env!("PX_SOURCE_HASH")`；各 schema 加 `build.rs` |
| S3 | 我 | `px_cook::inst`（`key` / `library_path` / `symbol` / 缓存）+ `px_inst!` 宏 |
| S4 | 子代理 B | `px_jit` 工具（扫 `px_inst!` → 生成 → 构建 → 收集 + sidecar；`list` / `build`）+ `tools/px.ps1 -Target inst` |
| S5 | 我 | 真样本 `art/inst/<名>.rs` + `px_graphs/tests/inst_op.rs`（自己的图名，不碰 `art/` 既有图）+ 逐轴门 + **R1 变体**（改 inst 源 ⇒ 图 exe 字节不变、键变、只有该节点重算） |
| S6 | 我 | **一次换键**：全编 → 烘三图 → 28/28 → 干净重编后重登记 `art/anchor/hashes.txt` §三 → J1 六张逐字节 |
| S7 | 我 | 指南加 §5.3（两种泛型算子怎么选：`px_local_op!` vs `px_inst!`）+ 本记录补读数 |

⚠ **量 anchor 的协议**（`18` §171.5 用血换的）：撤回用 `cargo clean -p`，先确认"再跑一遍全命中"，
再量。
