# 代码生成出来的类型：stage 2 用的类型 = stage 1 生成出来的（图侧零宏）

> 用户口径（2026-09-20）：**「让 stage 2 用上 stage 1 代码生成出来的类型」**，并且**图侧零宏**。
> 既定设计 = **方案 D(ii)**；§ 号自定（本篇），接 `19-generic-inst.md`（`px_inst!`）与
> `20-build-graph.md`（build graph、两阶段 `px list|build|gc|run`）。

## 目标

```text
cargo build（图 crate = px_graphs）
 └── build.rs ＝ stage 1 的「**计划**」段（**绝不调 cargo、绝不编译任何东西**）
      · 读 recipe（数据表）→ 校验 → 用「声明表」拿类型级事实 → 算 key
      · 写 OUT_DIR/insts_gen.rs：每条实例一个**生成出来的类型**（PxOp / InstNode impl，事实为 const）
      · **内容与盘上完全相同就不重写那个文件**（保 mtime）★硬要求
      · 图侧 src/insts.rs `include!` 它 ⇒ stage 2 用的类型 = stage 1 生成出来的 ✓
px build                     ← 仍然**显式**：编缺的实例 dylib（体住 art/inst/*.rs，只被实例 crate include!）
px run <图> [--build]         ← 计划 →（可选编）→ 跑 target/debug/<图>.exe（按 key 装载）
```

**一句话的形状变化**：从前"图侧写一行 `px_inst!` 宏 + 工具按文本读它的模板"；
今天"图侧写一张**数据表**（`inst_recipe.rs`）+ build.rs 按它**生成类型**"。图侧**一个宏调用都没有**了。

## 管线（三处，各自的职责不许越界）

```text
px_graphs/src/inst_recipe.rs        数据表：op_id / decl / type_name / roots / source / body
        │                           ⚠ 只有类型定义 + 字面量（build.rs 会直接读它）
        ↓  px_graphs/build.rs（cargo build 期；= stage 1 的「计划」段）
        │   ① px_fingerprint 算本 crate 指纹（PX_SOURCE_HASH，从前就有）
        │   ② 读 recipe → 校验 → 问 px_decls（声明表）要类型级事实 → 算 key
        │   ③ 写 OUT_DIR/insts_gen.rs（＋旁挂件 insts_gen_catalogue.rs）
        ↓  include!（px_graphs/src/insts.rs）
px_graphs/src/insts.rs              stage 2 用的**类型**（Band / Waves）就是生成物那一份
        ↓  px build / px run --build（**唯一的**编译入口：stage 1 的「执行」段）
target/jit/<key>/ → target/pcg/inst/<key>.dll    按 key 装载（口径一位未改）
```

**为什么生成器必须住 build.rs**：`interface()` / `decl_hash()` / 三个关联类型只有**编译过类型**
的那一侧算得出（`19` §179.5）。`px_graphs/build.rs` 编译不了算子类型（它只看得见
`[build-dependencies]`）⇒ 需要一张**编译过它们**的表来问（见下"声明表"）。
**为什么生成物进得了图程序**：`src/insts.rs` `include!` `OUT_DIR/insts_gen.rs` ——
于是 stage 2 手里是**类型**（`PxOp` impl 齐全），不是字符串/dyn。

## 四件新东西

### ① recipe：一张数据表（图侧零宏）

`px_graphs/src/inst_recipe.rs` —— 只有类型定义 + 字面量（`build.rs` 用 `#[path]` 当模块读它）：

```rust
pub struct InstRecipe {
    pub op_id: &'static str,          // "field.remap/waves"
    pub decl: &'static str,           // "FieldRemap"（声明名 —— 由声明表解析成类型）
    pub type_name: &'static str,      // "Waves"（图侧类型名；生成器要写出来）
    pub roots: &'static [&'static str],   // ["px_field_alg"]
    pub source: &'static str,         // "art/inst/waves.rs"
    pub body: &'static str,           // 体表达式原文
}
pub const INSTANCES: &[InstRecipe] = &[ /* band 与 waves 两条 */ ];
```

⚠ **`body` 那一栏里的泛型参数那一位写占位符 `ARG`**（`19` §179.1 当年那个口径）——
原因只有一个、但它是硬的：**`ARG` 那一份是 key 的一轴**。实测把它换成真名
（`&Band`）⇒ key `96d4feb75ec4` → `8fcd505c8000` ✗。key 的算法一个字不许改
（铁律 3）⇒ 表里写 `…, ARG`，而**抄进生成物**的那一份是把 `ARG` 整词换成 `&<type_name>`
（`InstRecipe::generated_body()`，替换规则与 `px_cook::inst_scan` 当年那一份**逐字相同**）。

### ② 声明表：`px_decls`（新 rlib，只被 `px_graphs/build.rs` 用）

```rust
pub struct DeclFacts {
    pub interface: u64, pub decl_hash: &'static str,
    pub params: &'static str, pub inputs: &'static str, pub payload: &'static str,
    pub schema: &'static str, pub module: &'static str,
}
pub fn decl(name: &str) -> Option<DeclFacts>;   // "CloudCoarse" → 事实
pub fn names() / entries() / SCHEMAS / workspace_root();
```

* **事实全部取自真类型**：`<O as PxOp>::interface()` / `::decl_hash()` /
  `type_name::<O::Params>()` / `type_name::<O::Inputs>()` / `type_name::<O::Payload>()`；
  `schema` / `module` 由 `<O>` 的**全路径**（`px_volume_schema::ops::CloudCoarse`）切出来
  —— 表里**没有一处人写的路径**。
* ⚠ 它不是 `const fn`：`any::type_name` 与 `PxOp::interface()` 今天都不是 const
  （实测 `is not yet stable as a const fn` / "const traits are not yet supported"）
  ⇒ 事实在**运行期**算，但调用它的是 build script 与测试（一次编译各一遍），开销无关紧要。
* **门**（`px_decls/tests/inst_gate.rs`，3 条）：三个 schema 的 `src/**` 里 `px_op!` 的处数
  == 表里条数（11 == 11）；表里每个名字都 `decl()` 查得到、且都指回自己的 schema / `ops` 模块；
  每条事实都活着（`interface != 0`、`decl_hash` 非空、三条路径都是**干净的类型路径**）。
  ⚠ 处数走 `px_cook::inst_scan::count_named(dir, "px_op!")` —— **同一份**扫描口径
  （"两份解析器数出不同的数"正是从前那道门失效的原因，`19` §180）。

**⚠ 为什么不是"往 `px_*_schema` 里加一个 `decl()`"（那是第一版设计，被否）**：
`decl_hash()` 取的是**声明所在 crate 的全量源码指纹**（`px_fingerprint`，刻意粗粒度，`19` §177）
⇒ 往 schema 里加**任何字节**（包括一行注释）就换掉它的 `SOURCE_HASH` ⇒ 换掉全部实例 key 与节点键
+ `art/anchor` 要重登记。实测（只给 `px_volume_schema/src/ops.rs` 追一行 `// PROBE`）：
`decl a62f63a3a8b6 → cd09d1f43e9b`、`band key 96d4feb75ec4 → f1464c38ea80 缺`。
而 `px_decls` **不进任何实现库的名册**（那三个 `[dependencies]` 由 `px_cook::inst` 按边
一条一条写出来，里面没有它）⇒ **三个 schema 一个字节不用动 ⇒ 两条 key 逐位不变**。

**怎么保证事实里的路径与生成物对得上**：生成物把那三条路径原样写进
`type Params = px_volume_schema::params::Params;` ⇒ **图程序编译**就是校验（写错/类型删了就编不过）；
门那一侧再查形状（无空格、无泛型参数）。

### ③ 生成器（`px_graphs/build.rs`）

输入：recipe + 声明表 + workspace 根。**校验**（任何一条不过就 `panic!` 并点名是第几条 recipe）：

| # | 校验 | 不过时的形状 |
|---|---|---|
| ① | `op_id` 唯一 | `recipe 第 N 条（…）：op id 重复` |
| ② | `decl` 在声明表里 | 见"错误映射"实测 |
| ③ | `roots` 里每个都是 workspace 成员目录 | 点名那个根 |
| ④ | `source` 文件存在、且里面**真的定义了** `type_name`（文本检查） | 点名文件与类型 |

**算 key**：`px_cook::inst::key_of_facts(…)` —— 它在 `px_cook` 里**转手就构造 `Inst` 走
`key()`**，所以生成器与图程序拿到的 key 出自**同一个函数**，不可能是两份算法。
输入与今天完全一致：`"px_inst/v1"` ‖ toolchain ‖ 契约 ‖ `decl_hash` ‖ 各根名册 ‖ `interface`
‖ `op_id` ‖ 源文件字节 ‖ **归一化后的体**（`normalize_template`）。

**写 `OUT_DIR/insts_gen.rs`**（每条实例一段，形状照 `px_inst!` 从前展开出来的那一份）：

```rust
pub struct Waves;
impl Waves { INST_ID / INST_DECL / INST_SOURCE / INST_ROOTS / INST_BODY / INST_TEMPLATE }
impl ::px_cook::inst::InstNode for Waves { fn info() -> InstInfo { info_of_facts(…) } }
impl ::px_graph_schema::PxOp for Waves {
    const ID / LIB = "" / SYMBOL = "px_inst__FieldRemap";
    type Params = …; type Inputs = …; type Payload = …;
    fn decl_hash() / fn source_hash()   // ← OnceLock 缓存 + key_of_facts
    fn render(…)  // ← 按 key 取库、load_at::<Self>、调体
}
pub fn facts_of(type_name: &str) -> (u64, &'static str)   // 末尾那张小表
```

* `source_hash()` **只能运行期算**（要读 `art/inst/*.rs` 的字节）—— 生成物里**不含**任何源文件字节
  ⇒ 改 `art/inst/*.rs` 生成物不变（R1 的全部机关）。
* **内容不变就不写文件**（先读旧文件比较）：`write_if_changed`。
* 生成物里的注释一律 `//`（不是 `//!`）：它被并进一个**模块体**，内层文档注释在那儿报
  `expected outer doc comment`（实测）。

### ④ 错误映射

`px build` / `px run --build` 失败时，除 cargo/rustc 原文外还打：

```text
✗ 这个实例编不过：op id field.remap/waves
  · 体（body）来自 px_graphs/src/inst_recipe.rs:101（recipe 里那一条的 `body` 一栏）
  · 参数文件 art/inst/waves.rs（生成物里是 include! 进去的 ⇒ 报告里的 target/jit/<key>/src/lib.rs:<行> 对应它）
  · 生成物：target/jit/<key>/（留着，不删）
```

* 体住哪：生成器算好行号写进**旁挂件** `OUT_DIR/insts_gen_catalogue.rs`
  （`InstCodegen { op_id, key, schema, module, decl, source, body, recipe_line, recipe, generated }`），
  `px build` / `px run --build` 读它。**旁挂件不参与任何 key、也不进图程序**。
* 生成物**留着不删**（`px build --gc --deep` 才会回收非活的那些）。

## 铁律（一条不许退步）——逐条读数

| # | 铁律 | 读数 |
|---|---|---|
| 1 | 改 `art/inst/*.rs` ⇒ 生成物不变 ⇒ 图程序不重编、七个 exe 一位不变 | ✓ 见「读数 R1」 |
| 2 | `cargo build` 绝不调 cargo / 编实例 | ✓ build.rs 只读盘 + 写 `OUT_DIR`；实测探针那趟 `Compiling=0` |
| 3 | 两条现役实例的 key 一位不变（`96d4feb75ec4` / `caa8318cda1b`） | ✓ 见「读数 2」 |
| 4 | `px list` / `build` / `gc` / `run` 行为与读数口径不变；`tools/px.ps1 -Task` 不变 | ✓ 见「读数 3」 |
| 5 | 别碰 `px_fingerprint/**` / `px_graph_schema/**` / `px_volume_alg/**` / `px_field_alg/**` / `px_*_op/**` / `art/anchor/**` / 任何 `*.md` | ✓ 本轮只动 `px_cook/**`、`px_graphs/**`、新 crate `px_decls/**`、根 `Cargo.toml`／`Cargo.lock`；`art/inst/waves.rs` 只做过"追一行注释再撤回"（逐字节回原） |
| 6 | 不提交、不动 `main`；撤回刷 mtime；量 R1 前先跑到 `Compiling=0` 且无"拒绝访问 (os error 5)" | ✓ 两天都按这条办的（见「量法」） |

**方案 D(ii) 能同时满足 ②③ 的关键**：**声明事实住一个不进任何名册的 crate**
（`px_decls`），于是"每加一个 `px_op!` 就要在表里加一行"这条要求**不碰到 schema 的指纹**。

## 读数（真读数，原样）

### 1. `cargo build --workspace --exclude px_render` 尾行

```
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 9.55s
```

### 2. `cargo test --workspace --exclude px_render`

`FAILED|^error` **无输出**；`test result:` 行 **78** 条、`passed` 合计 **286**、`failed` **0**。
新增那道门（声明表）与生成物那道门：

```
$ cargo test -p px_decls --test inst_gate
test every_row_resolves_by_name ... ok
test the_facts_are_alive ... ok
test the_table_has_one_row_per_px_op_declaration ... ok
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo test -p px_graphs --test inst_gate
test the_symbol_name_is_the_same_string_on_both_sides ... ok
test the_generated_body_is_the_substituted_recipe_body ... ok
test every_recipe_row_is_in_the_graph ... ok
实例算子：65×65×1647750｜命中=true → true｜key e7aded3f4563
test a_real_instance_cooks_end_to_end ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

### 3. `px list` → `px build` → `px list`（两条 key 与冻结值逐位相同）

```
$ px list
实例 2 条：
  cloud.coarse/band    a62f63a3a8b6 px_volume_alg    art/inst/band.rs   96d4feb75ec4 有
  field.remap/waves    943655dc0a27 px_field_alg     art/inst/waves.rs  caa8318cda1b 有

$ px build
已有 cloud.coarse/band（96d4feb75ec4）
已有 field.remap/waves（caa8318cda1b）
共 2 条：已编 0、已有 2、失败 0

$ px list
（同上，逐位相同）
```

`px build --gc`：`活 2 条：96d4feb75…, caa8318cda1b…` / `共 2 条：活 2、删 0（释放 0.0 MB）、跳过 4`
—— 与 `20` §191 的口径一致。

### 4. 运行读数

```
$ px run field_remap                （冷：PX_PCG_FRESH=1）
stage 1｜实例 2 条：命中 2、缺 0
stage 2｜…\target\debug\field_remap.exe
图 field_remap｜画布 256×128｜…｜PX_PCG_FRESH=1：本次全部重算
重算 source       field.fbm        @04a71661  40838b712b57     62 ms  131314 B  256×128｜值域 0.0998..0.9024｜均值 0.4903
重算 bands        field.remap/waves @73abaf19  e862da5c2ffe     31 ms  131313 B  256×128｜值域 0.0000..1.0000｜均值 0.5603
输出 bands（field.remap/waves）：256×128｜值域 0.0000..1.0000｜均值 0.5603
共 2 个节点：命中 0、重算 2，合计 93 ms

$ px run field_remap                （热）
命中 source       field.fbm        @04a71661  40838b712b57      0 ms  131314 B
命中 bands        field.remap/waves @73abaf19  e862da5c2ffe      0 ms  131313 B
共 2 个节点：命中 2、重算 0，合计 0 ms

$ px run planet
stage 1｜实例 2 条：命中 2、缺 0
共 6 个节点：命中 6、重算 0，合计 0 ms       ⇒ 全命中 ✓
```

（`field.remap/waves` 的节点键 `e862da5c2ffe`／`@73abaf19` 与 `20` §193 那一轮完全一致。）

### 5. R1 实测

```text
两次 build 坐实：build1 Compiling px_graphs→Finished；build2 Finished（Compiling=0）、无"拒绝访问"
快照九个 exe（SHA256）＋ insts_gen.rs（SHA256 + mtime）
只给 art/inst/waves.rs 追一行注释（Add-Content + 刷 mtime）
cargo build  ⇒  Finished（**Compiling 一个都没有**）
  七个图 exe（planet/desert/clouds/scene/shaders/passes/field_probe）+ field_remap + px：
      ALL EXES UNCHANGED = True      （九个全"不变 ✓"）
  insts_gen.rs：SHA256 7740DD0CCFABBB0B00DCB3F55715025901FEDB38C3CE7A72E343D5A4E1D8FE2F
                前后**同一个值**、mtime **同一个时刻**（2026-09-20T12:08:44.1313484+08:00）⇒ 字节不变
  px list：field.remap/waves  caa8318cda1b **有** → 43b8ad5845d2 **缺**   （key 变了 ✓）
撤回（整份重写 + 刷 mtime）⇒ cargo build（Compiling=0）⇒ px list：
  cloud.coarse/band 96d4feb75ec4 有 ／ field.remap/waves caa8318cda1b 有   （逐位回原值 ✓）
```

### 6. 生成物证据

`OUT_DIR/insts_gen.rs` 全文（`SHA256 = 7740DD0C…8FE2F`，见本文件末「附录 A」）。
**`Waves` 那一段**（逐字）：

```rust
/// 实例 `field.remap/waves`（声明 `FieldRemap`）—— 体来自 `px_graphs/src/inst_recipe.rs`。
pub struct Waves;

impl Waves {
    pub const INST_ID: &'static str = "field.remap/waves";
    pub const INST_DECL: &'static str = "FieldRemap";
    pub const INST_SOURCE: &'static str = "art/inst/waves.rs";
    pub const INST_ROOTS: &'static [&'static str] = &["px_field_alg"];
    pub const INST_BODY: &'static str = "px_field_alg::remap_with(&px_field_alg::identity(), p, i.input.value(), g, &Waves)";
    pub const INST_TEMPLATE: &'static str = "px_field_alg::remap_with(&px_field_alg::identity(), p, i.input.value(), g, ARG)";
}

impl ::px_cook::inst::InstNode for Waves {
    fn info() -> ::px_cook::inst::InstInfo {
        ::px_cook::inst::info_of_facts(
            Self::INST_ID, 8334948060302358246u64,
            "943655dc0a27a55a40dd95e2135654659909a16c4ae50c2ae2aae90ccd0c97b2",
            Self::INST_ROOTS, Self::INST_SOURCE, Self::INST_TEMPLATE,
        )
    }
}

impl ::px_graph_schema::PxOp for Waves {
    const ID: &'static str = "field.remap/waves";
    const LIB: &'static str = "";
    const SYMBOL: &'static str = "px_inst__FieldRemap";
    type Params = px_field_schema::params::RemapParams;
    type Inputs = px_field_schema::ops::FieldRemapInput;
    type Payload = px_field_schema::field::Field;
    fn new() -> Self { Waves }
    fn decl_hash() -> &'static str { "943655dc0a27a55a40dd95e2135654659909a16c4ae50c2ae2aae90ccd0c97b2" }
    fn source_hash() -> ::core::result::Result<&'static str, ::std::string::String> { /* OnceLock + key_of_facts */ }
    fn render(…) -> … { /* key → 库 → load_at::<Self> → body(p, i, g) */ }
}
```

**图侧不再有任何 `px_inst!` 宏调用**：`px_graphs/src` 里 grep **零命中** ✓
（剩四处提到那个词的全在**注释**里：`build.rs` 的文档、`inst_recipe.rs` 的口径说明、
`px_graphs/tests/inst_gate.rs` 的历史注、`px_graphs/tests/crate_graph.rs` 的文档 —— 都是历史注）。

### 7. 错误映射实测

**(a) `decl` 写错**（`"CloudCoarse"` → `"NoSuchDecl"`）⇒ `cargo build`：

```
error: failed to run custom build command for `px_graphs v0.1.0 (…\px_graphs)`
  thread 'main' (34556) panicked at px_graphs\build.rs:35:59:
  recipe 第 1 条（cloud.coarse/band）：声明 `NoSuchDecl` 不在声明表里（`px_decls`）
  ⇒ 要么名字写错了，要么那是一个新的 `px_op!` —— 后者要先在 `px_decls/src/lib.rs` 的 `decl()` 里加一臂
```

**(b) `body` 写错**（`remap_with` → `remap_with_nonexistent`）⇒ `cargo build` **过**（生成器不
类型检查体，那一步只能由实例库编译做）⇒ `px build`：

```
生成 …\target/jit\26ba9c0eccb1…（体来自 px_graphs/src/inst_recipe.rs:101）
error[E0425]: cannot find function `remap_with_nonexistent` in crate `px_field_alg`
  --> src\lib.rs:11:29
error: could not compile `px_inst` (lib) due to 1 previous error
失败 field.remap/waves（key 26ba9c0eccb1…）：
✗ 这个实例编不过：op id field.remap/waves
  · 体（body）来自 px_graphs/src/inst_recipe.rs:101（recipe 里那一条的 `body` 一栏）
  · 参数文件 art/inst/waves.rs（生成物里是 include! 进去的 ⇒ 报告里的 target/jit/26ba9c0eccb1…/src/lib.rs:<行> 对应它）
  · 生成物：…/target/jit/26ba9c0eccb1…（留着，不删）
共 2 条：已编 0、已有 1、失败 1        [exit=1]
```

（行号 `101` = 表里那条 `body` 一栏所在行，实测对得上；那次刻意失败的 `target/jit/<key>/`
已由 `px build --gc --deep` 回收。）

### 8. `px_decls` 与 schema crate 的 diff

* **`px_decls`**：新 crate（untracked）—— `Cargo.toml` / `build.rs` / `src/lib.rs` /
  `tests/inst_gate.rs`。
* **`px_field_schema` / `px_volume_schema` / `px_mesh_schema`**：本轮**没有改它们**。
  ⚠ `git diff --stat` 对这三个 crate **不是空的** —— 那 12 个文件是本分支**开工前**就带着的
  diff（分支未提交的旧改动），与这一轮无关。**"没换键"的书面证据**是这两个：
  1. 会话开头那次探针："给 `px_volume_schema/src/ops.rs` 追一行注释 ⇒ key 变"
     ⇒ 这三个 crate 的源码**确实在 key 里**；
  2. 探针撤回后两条 key **逐位回到** `96d4feb75ec4` / `caa8318cda1b` ⇒ 它们今天与开工时**逐字节相同**。
  3. 内容哈希（`SHA256` 前 16 位）留档：
     `px_field_schema/src/ops.rs 70B594BE39FD4606`、`px_volume_schema/src/ops.rs 92DF23F462572112`、
     `px_mesh_schema/src/ops.rs AFB31CBF67822AE9`、`px_volume_schema/src/params.rs 831FD35809D21186`、
     `px_field_schema/src/params.rs 3AFC675D8A20E658`。

### 9. `git status --short`（我动过的文件）

```text
 M Cargo.toml                  ← members/default-members 加 px_decls
 M Cargo.lock                  ← 新 crate 的 path 依赖
 M px_cook/src/lib.rs          ← 删 inst_scan 的说明改成"工具用"，其余不动
 M px_cook/src/inst.rs         ← key_of_facts / info_of_facts / InstCodegen / InstCatalogue / compile_one 新签名
 M px_cook/src/inst_scan.rs    ← count_named()；删 Invocation::body()/replace_word()
 M px_graphs/Cargo.toml        ← [build-dependencies] + px_decls + px_cook
 M px_graphs/build.rs          ← 生成器（四件新东西 ③）
 M px_graphs/src/insts.rs      ← include! 生成物 + build() 走 facts()
 M px_graphs/src/bin/px.rs     ← 去掉文本扫描，读生成物的旁挂件
 M px_graphs/tests/inst_gate.rs← 门改成"recipe 条数 == 图里节点数 / 生成物与 recipe 一致"
 M px_graphs/tests/crate_graph.rs ← 一句过期文档（`px_inst!` → 今天那张表）
?? px_decls/                    ← 新 crate（四件新东西 ②）
?? px_graphs/src/inst_recipe.rs ← 新数据表（四件新东西 ①）
```

（其余 `M`/`??` 全是本分支**开工前**就有的未提交改动，与本轮无关。**没提交任何东西**、
**没动 `main`**。）

## 落地时的四个坑（都实测踩过，记下来）

1. **key 的一轴是"占位符那一份"**：recipe 直接写 `&Band` ⇒ key 从 `96d4feb75ec4` 变成
   `8fcd505c8000`。修法：表里写 `ARG`，生成器做**整词替换**（key 用原文、生成物用替换后的）。
2. **`//!` 与 `#[path]`/`inc` 的次序**：`inst_recipe.rs` 被两个地方读（`insts.rs` 的模块、
   `build.rs` 的模块）。它若以 `//!` 开头，在 `insts.rs` 那一侧（它排在 `use` 之后）报
   `expected outer doc comment`；而 `#[path] mod` 在 `//!` 存在时的报错是**误导性的**
   `file not found for module recipe`。⇒ 那个文件一律用 `//`。
3. **`#[path]` 优于把文件正文抄进来**：`build.rs` 里出现那个"抄一份源码"的写法会被
   `px_graph/tests/source_hash.rs` 正当地拒（共享助手是 crate，不许有副本）⇒ 用 `#[path]`。
4. **生成物里不许 `//!`**：它被并进一个模块体 ⇒ 只能 `//`。

## 遗留

* **`inst_recipe.rs` 里那两条 `decl` 与 `type_name` 仍是"名字"，不是类型**：`build.rs` 读它们时
  只在**生成物编译**时才知道对不对（`decl` 由声明表当场校验 ✓；`type_name` 由"源文件里有没有
  这个类型"的文本检查 + 生成物编译两处看着 ✓）。要更硬就是让 recipe 直接持类型（那需要图侧
  依赖 alg/声明 crate，会破 R1 的边界）—— **不做**。
* **`px_cook` 仍是 `px_graphs` 的常规依赖**（图程序运行期要用 `PxOp` 的那些关联类型/装载）；
  `[build-dependencies]` 里也加了它（生成器要用 `key_of_facts` / `inst::symbol`）。
  它与 `px_decls` 都不在任何实现库的名册里 —— 实测两条 key 逐位不变就是这一条的证据。
* **`facts_of` 那张小表**是"recipe 的 `type_name` → 两个 const"的落点：加一条实例要在
  recipe 表里加一条（生成物会自动多一段 + 多一臂）—— 图侧仍然零宏、`insts::build` 不用改。
* `px_*_op` / `art/anchor` / `px_fingerprint` / `px_graph_schema` 本轮**一个字节没动**
  ⇒ **不需要重登记**。

### 遗留一：**参与算身份的 crate 里那 21 处旧机制名 —— 现在不改**（2026-09-20 补）

**规则**：**参与算身份的 crate 只应在产品语义变化时改**；注释里的旧机制名（今天说的是
"`px_inst!` 生成的 / 展开出来的"，而那个宏已经删了）等这些 crate **下次真有功能改动时顺手改**
—— **为注释付一次全仓换键是坏交易**（`20` §192/§194 记过两次同一条：`px_fingerprint` 追一行注释
⇒ 三个 schema + 四个库身份全换；`px_field_alg` 去掉一个内部垫片 ⇒ 全节点键换 + §三 重登记 + J1 重跑）。

**判据（为什么它贵）**：这些 crate 的 `src/**` + `build.rs` 在**算子身份**的传递闭包里
（`px_fingerprint::roster`，`19` §177）⇒ 改一行注释 = 换 `SOURCE_HASH` = 换**全仓节点键**
+ `art/anchor/hashes.txt` §三 要**重登记** + J1 六张要重跑。而 `px_cook/**` 与 `art/inst/*.rs`
**不在任何算子的名册里** ⇒ 改它们的注释只换**那一条实例**的 key（一条实例重编、秒级），
anchor 一动不动 —— 所以同一类清理要**按成本分两堆**。

**不动的清单（21 处，8 个 crate；全部只是注释/文档字符串，没有一处是调用）**：

| crate | 处数 | file:line |
|---|---|---|
| `px_graph_schema` | 2 | `src/contract.rs:97`、`src/ops.rs:37` |
| `px_field_schema` | 6 | `src/ops.rs:7`、`:41`、`:85`、`:90`、`src/params.rs:15`、`build.rs:3` |
| `px_volume_schema` | 1 | `build.rs:3` |
| `px_mesh_schema` | 1 | `build.rs:3` |
| `px_field_alg` | 5 | `src/lib.rs:4`、`src/remap.rs:5`、`:17`、`:143`、`Cargo.toml:8` |
| `px_volume_alg` | 3 | `src/lib.rs:11`、`src/field_fn.rs:17`、`Cargo.toml:9` |
| `px_field_op` | 2 | `src/lib.rs:8`、`Cargo.toml:17` |
| `px_volume_op` | 1 | `src/lib.rs:9` |

⚠ **`px_field_op` / `px_volume_op` / `px_mesh_op` 的 `Cargo.toml` 也算在内**：`roster` 收的是
`src/**` + `build.rs`，**不收 `Cargo.toml`** —— 但 `Cargo.toml` 是"边"的声明，改它要重跑 build script、
且容易顺手带出别的改动 ⇒ 与注释同一批处理，**不单独为注释去动**（这条只是纪律，不是机制）。

⚠ 同一类事**已经按这条规矩改完的那几处**（因为改它们**免费**）：
`px_cook/src/inst.rs`、`px_cook/src/inst_scan.rs`（4 处）、`art/inst/band.rs:1`。
⚠ **保留的历史注**：`px_graphs/build.rs:279`（"照当年宏展开的形状"）—— 那是**本轮的说明**，
它讲的是"生成物为什么长这样"，不是过期引用。
⚠ `px_graphs/**` 也不在算子名册里（它是图程序，`20` §191.1）⇒ 它上面的同类清理同样免费。

### 遗留二：`px_inst!` 宏本体已删 + 图侧那一批旧机制名已改（2026-09-20 第二刀）

* **宏本体删了**：`px_cook/src/lib.rs` 里 `#[macro_export] macro_rules! px_inst {…}` 连它上方那段
  文档注释一起删掉，原地留一条墓碑注（说明"图侧零宏了、体模板的口径活在 recipe 的 `ARG` 里、
  下面 `px_local_op!` 是另一条路照旧"）。**删前逐处核过**：全仓（含隐藏目录）只有
  **宏定义本体**一处真调用，其余全是注释 / 字符串字面量 / 错误消息文案。
* 留下的东西**一个没动**：`px_local_op!`、`inst_scan`（`count_named` 那道门与生成器在用）、
  `px_graph_schema` 的 `px_body!` / `px_impl_lib!`（生成的实例 crate 在用）。
* **免费那一堆（不在任何算子名册里）已按实义改写**：`px_cook/src/inst.rs`（3 处）、
  `px_cook/src/inst_scan.rs`（4 处 + 文件头）、`art/inst/band.rs:1`。改的是"主语"——
  仍然成立的事实（key 由这一层算、模板是一轴、`ARG` 口径）照旧写着，只把"那个宏展开出来的"
  换成"生成器生成的 `impl`（recipe + `px_decls`）"。
* ⚠ **代价**：`art/inst/band.rs` 的内容是 **`cloud.coarse/band` 那条实例 key 的一轴** ⇒
  改那一句注释**换了那条实例的 key**（预期内、且只此一条）：
  **`96d4feb75ec4` → `d1c8fd369338`**（`px build` 已把新库编出来：`target/pcg/inst/d1c8fd369338*.dll`，
  8054784 B）。`field.remap/waves` **一位没变**（`caa8318cda1b`）—— 它链的是 `px_field_alg`，
  与 `px_cook` / `art/inst/band.rs` 都无关。
  ⚠ 于是 **§「读数 3」里那个 `96d4feb75ec4` 是这一刀之前的值**；今天生效的冻结值是
  **`d1c8fd369338`（band）/ `caa8318cda1b`（waves）**。anchor 不受影响（改的都不在名册里）。
* 读数：`cargo build` 尾行 `Finished`；`cargo test --workspace --exclude px_render` 的
  `FAILED|^error` **无输出**、`test result:` 78 行、**passed 286 / failed 0**（连跑两遍一致）；
  `px build` = `共 2 条：已编 1、已有 1、失败 0`；改宏前后**重新生成的那份 `insts_gen.rs`
  字节不变**（SHA256 前后同值 —— "宏与生成物无关"的书面证据）。

## 附录 A：`OUT_DIR/insts_gen.rs` 全文（逐字）

```rust
// **生成物**（`px_graphs/build.rs` 写的）—— 别手改。
//
// 每条实例一个「生成出来的类型」：它复用某个**声明**的接口，体来自 recipe，
// 身份（`source_hash`）= 实例 key。stage 2 用的类型就是这里这一份
// （`.agents/notes/art/21-codegen-types.md`）。
//
// ⚠ key 与源文件字节有关 ⇒ 它只能在**运行期**算；生成物里只有不随源码字节变的
// 那几样是 const（op id / 声明指纹 / 三个类型 / 根 / 源路径 / 体）。
//
// ⚠ 这里是 `//` 而不是 `//!`：这个文件是被并进 `px_graphs/src/insts.rs` 的一个**模块体**，
//   内层文档注释在那儿会报 `expected outer doc comment`（实测）。

/// 实例 `cloud.coarse/band`（声明 `CloudCoarse`）—— 体来自 `px_graphs/src/inst_recipe.rs`。
pub struct Band;

impl Band {
    pub const INST_ID: &'static str = "cloud.coarse/band";
    pub const INST_DECL: &'static str = "CloudCoarse";
    pub const INST_SOURCE: &'static str = "art/inst/band.rs";
    pub const INST_ROOTS: &'static [&'static str] = &["px_volume_alg"];
    /// ⚠ `ARG` 已换成 `&Band`（那才是能编的体）；key 那一轴用的是
    ///   recipe 里那一份原文 —— 见 `INST_TEMPLATE`。
    pub const INST_BODY: &'static str = "px_volume_alg::coarse_with(p, i.coverage.value(), &Band)";
    /// recipe 里那一栏**原文**（`…, ARG`）：它是 **key 的一轴**（`InstInfo.template`）。
    pub const INST_TEMPLATE: &'static str = "px_volume_alg::coarse_with(p, i.coverage.value(), ARG)";
}

impl ::px_cook::inst::InstNode for Band {
    fn info() -> ::px_cook::inst::InstInfo {
        ::px_cook::inst::info_of_facts(
            Self::INST_ID,
            917032432185140074u64,
            "a62f63a3a8b6ed5271d98fc8dd38e9c505be1143b1b2a370df0a569200e3ed59",
            Self::INST_ROOTS,
            Self::INST_SOURCE,
            Self::INST_TEMPLATE,
        )
    }
}

impl ::px_graph_schema::PxOp for Band {
    const ID: &'static str = "cloud.coarse/band";
    /// 空串：实现不在任何**预置**实现库里 —— 它是运行期按 key 装载的实例库。
    const LIB: &'static str = "";
    const SYMBOL: &'static str = "px_inst__CloudCoarse";

    type Params = px_volume_schema::params::Params;
    type Inputs = px_volume_schema::ops::CloudCoarseInput;
    type Payload = px_protocol::art::VolumeData;

    fn new() -> Self {
        Band
    }

    /// 接口形状从**复用的那三个关联类型**推，声明指纹取被复用声明那一份。
    fn decl_hash() -> &'static str {
        "a62f63a3a8b6ed5271d98fc8dd38e9c505be1143b1b2a370df0a569200e3ed59"
    }

    /// 身份 = **实例 key**（图程序不重编也能看出泛型参数/alg/契约/工具链变了）。
    /// ⚠ 算一次就记住（`OnceLock`）：key 要读盘数名册，别每次调都重算
    ///   —— 这正是从前那份宏展开出来的形状。
    fn source_hash() -> ::core::result::Result<&'static str, ::std::string::String> {
        static KEY: ::std::sync::OnceLock<
            ::core::result::Result<::std::string::String, ::std::string::String>,
        > = ::std::sync::OnceLock::new();
        match KEY.get_or_init(|| {
            ::px_cook::inst::key_of_facts(
                Self::INST_ID,
                917032432185140074u64,
                "a62f63a3a8b6ed5271d98fc8dd38e9c505be1143b1b2a370df0a569200e3ed59",
                Self::INST_ROOTS,
                Self::INST_SOURCE,
                Self::INST_TEMPLATE,
            )
        }) {
            ::core::result::Result::Ok(text) => ::core::result::Result::Ok(text.as_str()),
            ::core::result::Result::Err(err) => ::core::result::Result::Err(err.clone()),
        }
    }

    fn render(
        &self,
        p: &Self::Params,
        i: &Self::Inputs,
        g: ::px_graph_schema::Grid,
    ) -> ::core::result::Result<Self::Payload, ::std::string::String> {
        let key = Self::source_hash()?;
        let path = ::px_cook::inst::library_path(key);
        if !path.is_file() {
            return ::core::result::Result::Err(format!(
                "{}{}",
                path.display(),
                ::px_cook::inst::missing_hint(key)
            ));
        }
        let body = ::px_graph_schema::ops::load_at::<Self>(
            path.to_string_lossy().as_ref(),
            Self::SYMBOL,
        )?;
        body(p, i, g)
    }
}

/// 实例 `field.remap/waves`（声明 `FieldRemap`）—— 体来自 `px_graphs/src/inst_recipe.rs`。
pub struct Waves;

impl Waves {
    pub const INST_ID: &'static str = "field.remap/waves";
    pub const INST_DECL: &'static str = "FieldRemap";
    pub const INST_SOURCE: &'static str = "art/inst/waves.rs";
    pub const INST_ROOTS: &'static [&'static str] = &["px_field_alg"];
    /// ⚠ `ARG` 已换成 `&Waves`（那才是能编的体）；key 那一轴用的是
    ///   recipe 里那一份原文 —— 见 `INST_TEMPLATE`。
    pub const INST_BODY: &'static str = "px_field_alg::remap_with(&px_field_alg::identity(), p, i.input.value(), g, &Waves)";
    /// recipe 里那一栏**原文**（`…, ARG`）：它是 **key 的一轴**（`InstInfo.template`）。
    pub const INST_TEMPLATE: &'static str = "px_field_alg::remap_with(&px_field_alg::identity(), p, i.input.value(), g, ARG)";
}

impl ::px_cook::inst::InstNode for Waves {
    fn info() -> ::px_cook::inst::InstInfo {
        ::px_cook::inst::info_of_facts(
            Self::INST_ID,
            8334948060302358246u64,
            "943655dc0a27a55a40dd95e2135654659909a16c4ae50c2ae2aae90ccd0c97b2",
            Self::INST_ROOTS,
            Self::INST_SOURCE,
            Self::INST_TEMPLATE,
        )
    }
}

impl ::px_graph_schema::PxOp for Waves {
    const ID: &'static str = "field.remap/waves";
    /// 空串：实现不在任何**预置**实现库里 —— 它是运行期按 key 装载的实例库。
    const LIB: &'static str = "";
    const SYMBOL: &'static str = "px_inst__FieldRemap";

    type Params = px_field_schema::params::RemapParams;
    type Inputs = px_field_schema::ops::FieldRemapInput;
    type Payload = px_field_schema::field::Field;

    fn new() -> Self {
        Waves
    }

    /// 接口形状从**复用的那三个关联类型**推，声明指纹取被复用声明那一份。
    fn decl_hash() -> &'static str {
        "943655dc0a27a55a40dd95e2135654659909a16c4ae50c2ae2aae90ccd0c97b2"
    }

    /// 身份 = **实例 key**（图程序不重编也能看出泛型参数/alg/契约/工具链变了）。
    /// ⚠ 算一次就记住（`OnceLock`）：key 要读盘数名册，别每次调都重算
    ///   —— 这正是从前那份宏展开出来的形状。
    fn source_hash() -> ::core::result::Result<&'static str, ::std::string::String> {
        static KEY: ::std::sync::OnceLock<
            ::core::result::Result<::std::string::String, ::std::string::String>,
        > = ::std::sync::OnceLock::new();
        match KEY.get_or_init(|| {
            ::px_cook::inst::key_of_facts(
                Self::INST_ID,
                8334948060302358246u64,
                "943655dc0a27a55a40dd95e2135654659909a16c4ae50c2ae2aae90ccd0c97b2",
                Self::INST_ROOTS,
                Self::INST_SOURCE,
                Self::INST_TEMPLATE,
            )
        }) {
            ::core::result::Result::Ok(text) => ::core::result::Result::Ok(text.as_str()),
            ::core::result::Result::Err(err) => ::core::result::Result::Err(err.clone()),
        }
    }

    fn render(
        &self,
        p: &Self::Params,
        i: &Self::Inputs,
        g: ::px_graph_schema::Grid,
    ) -> ::core::result::Result<Self::Payload, ::std::string::String> {
        let key = Self::source_hash()?;
        let path = ::px_cook::inst::library_path(key);
        if !path.is_file() {
            return ::core::result::Result::Err(format!(
                "{}{}",
                path.display(),
                ::px_cook::inst::missing_hint(key)
            ));
        }
        let body = ::px_graph_schema::ops::load_at::<Self>(
            path.to_string_lossy().as_ref(),
            Self::SYMBOL,
        )?;
        body(p, i, g)
    }
}

/// 图侧类型名 → `(interface, decl_hash)` —— recipe 与生成物之间那一条的落点。
///
/// ⚠ 名字不认识就是 recipe 与生成物**不同步**（改了表没重新生成？）⇒ 当场说清楚。
pub fn facts_of(type_name: &str) -> (u64, &'static str) {
    match type_name {
        "Band" => (917032432185140074u64, "a62f63a3a8b6ed5271d98fc8dd38e9c505be1143b1b2a370df0a569200e3ed59"),
        "Waves" => (8334948060302358246u64, "943655dc0a27a55a40dd95e2135654659909a16c4ae50c2ae2aae90ccd0c97b2"),
        other => panic!(
            "recipe 里那条实例的 `type_name` 是 `{other}`，而生成物里没有这个类型\n  ⇒ `inst_recipe.rs` 与 `insts_gen.rs` 不同步（跑 `cargo build` 重新生成？）"
        ),
    }
}
```
