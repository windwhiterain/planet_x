# 算子回到实现库、按身份装载：一条缺陷、一条机制、一条判据

> 起因（用户口径）：**「优化 px_graph 的 pcg 系统：架构优雅合理最简、功能仍然强大可扩展、
> agent 写 graph scripts、agent 自定义泛型算子要方便易懂」**；其中被单独点名的性质是
> **「改算子实现不重编图程序」**（用户选了它，代价与机制见 §170）。
> § 号接 `docs/system/typed-scripts.md`（§161–§167 是「类型化算子契约」那一轮）。
> 这一篇是**决议 + 测量 + 已知代价**；代码在 worktree `.worktrees/pcg-op-dylib`（分支 `feature/pcg-op-dylib`）。

---

## §168 先修一条**真缺陷**：源码指纹根本没覆盖共享依赖

这一轮开头我按"改 `px_cook` 的源码应当换键"去验证，结果**键一位没动、全部命中**。
量下去，病根是两行：

```rust
let path = manifest.join("../px_cook");   // 带着 `..`
if path.starts_with(&manifest) { continue; }   // Path::starts_with 按**组件**比，不化简 `..`
```

`Path::new("/a/b/../c").starts_with("/a/b")` 是 **true** ⇒ 那句"自己的 `src` 已经收过"把
**每一个** path 依赖都判成"自己"跳过了。实测：`rerun-if-changed` 里只有本 crate 的 **12 个文件**，
`px_cook` / `px_graph_schema` / `px_field_schema` / `px_protocol` / `px_derive` 一个都没进。

⇒ 后果正是 §28.2 那条要防的：**改共享依赖不换键 = 陈旧命中**。改 `px_field_schema::payload`
的编解码、改契约的 `PayloadBundle`，算出来的产物会变，而键不变、CAS 以为命中。

修法（共享的指纹算法，今天住 `px_fingerprint` 这个 rlib）：

1. **路径词法规范化**（去掉 `.`/`..`）之后再比 —— 修完是 **35 个文件**（自己 12 + `px_cook` 2 +
   `px_derive` 1 + `px_field_schema` 5 + `px_graph_schema` 5 + `px_protocol` 10）。
2. 同时把"进哈希的名字"从**绝对路径**换成 `<crate 目录名>/<包内相对路径>`：
   绝对路径里带着 checkout 的位置，那会让"换一个目录签出"或"把仓库挪个地方"换掉所有键
   （§14.2：键是纯函数，不含路径）。这条是把缺陷修**对**的一半 —— 只修第 1 条会让键依赖盘上位置。

⚠ **代价必须说清**：修它的那一刻，全仓的键换了一次（指纹覆盖的源码集合变了）。
场景文档里嵌着成员键 ⇒ `art/anchor/hashes.txt` §三 那六格当场变红（实测 orbit-bare：
`1A0CAF9D78194406` vs 登记值 `2795F948E6987E11`）。六格的重登记连同 J1 的读数在 §171。

### §168.1 同一族的**第二个**洞：指纹只走**直接**依赖，漏了 proc-macro

修完 §168 之后我顺手量了一下"哪些共享件真的进了身份"，发现 `px_derive` 不在里面 —— 而它
生成的正是 `PxInputs::collect`（**进键**的那段）。病根：`path_dependencies()` 只收
`Cargo.toml` 里**直接**写的 path 依赖，而 `px_derive` 是 `px_field_op → px_field_schema → px_derive`
这条链上的**传递**依赖。

实测（改 `px_derive/src/lib.rs` 加一行注释 → 重建 → 跑 planet）：

| | 读数 |
|---|---|
| 修之前 | `px_field_op.dll` 的身份**一位不变**；`continents` 的键 `4ff22e96914a641a` **不变**，6/6 全命中 ⇒ **陈旧命中** |
| 修之后（改成传递闭包） | `continents` 的键 `0b979e1603521967` → `73d4cb9eb995`（**换了**）；撤回那一行 ⇒ 键**逐字节回到** `0b979e1603521967`，再跑 6/6 命中 |

⇒ 修法：`collect_crate()` 递归收可达的 path 依赖（`BTreeSet` 防环）。
⚠ 它也是**第二处换键**（`px_field_op` 的文件集 35 → 36）—— §三 那六格因此重登记了**第二遍**；
J1 六张**仍然逐字节相同**（复查时又抽查了 orbit-bare 与 orbit-rings，值不变）。
⇒ 两条一起说明一件事：**"键变了"未必是坏事，但每一次都得能说清是哪条覆盖面变了**。

---

## §169 三层各归其位（用户选的口径）

```text
px_protocol        线格式（Frame / Blob / ArtBundle）
     ↓
px_graph_schema    ★契约：Key / PayloadBundle / Grid / 算子身份（OpId）+ PxOp 契约 + **装载**（ops）
     ↓
px_*_schema        各领域的数据、参数、**以及算子声明**（ops.rs：px_op! 那几行）
     ↓
px_cook            图脚本唯一那扇门：cook + 图的生命周期 + 各域算子表（都是 re-export）
     ↓
px_graph           驱动（CAS / 参数 / 清单 / cameras / generate / shader）
     ↓
px_graphs          图脚本；实现库（px_*_op）**不在它的依赖里**

px_*_op            实现：crate-type = ["dylib"]，按身份被运行期装载
```

四条不变量（`px_graphs/tests/crate_graph.rs` 与 `px_graph/tests/source_hash.rs` 看着）：

1. 图程序**不 cargo 依赖**实现库 —— 这条就是"改一行实现不重编图程序"。
2. 实现库**不依赖** `px_graph` / `px_cook`（否则 dylib 里带一份驱动：一个进程两份驱动，§162）。
3. `px_graph` 不依赖任何算子（它只认 `Cache`）。
4. schema 层不依赖算子（数据层是算子与驱动共用的）。

一处**与用户原答不一致**、有实测依据的偏离：用户选的是"算子声明分到各 `px_*_schema`"，
在 M2（链接期导入）下这是**做不到**的 —— 链接期 extern 会污染无辜消费者（实测 LNK2019：
一个只依赖声明 rlib、从不链 dylib 的 bin 也链接失败）。M1 没有链接期符号，所以这个选择成立。

### §169.1 「怎么编自己的产物」那条契约：**与载荷类型住在一起**

把 `PxOp` 下沉到契约层时，`Build`（`detail` / `encode` / `decode` / 两个域标志）跟着
`PayloadBundle` 一起落到了 **`px_protocol`**，而不是 `px_graph_schema`。理由是**孤儿规则**
量出来的，不是审美：

* `VolumeData` / `MeshData` 定义在 `px_protocol::art`，`px_volume_schema` 只是 `pub use` 它；
* `impl Build for VolumeData` 想写在 schema 里 ⇒ trait（`px_graph_schema`）与类型（`px_protocol`）
  **都不是本地的** ⇒ `E0117`（实测：`px_volume_schema/src/payload.rs:33`）；
* 也不能把 impl 下移进 `px_graph_schema`：`px_graph_schema` 已经依赖 `px_protocol`，反过来成环，
  而且那会让契约层认识两个域的载荷形状。

⇒ 落成一条口径：**一个域的载荷编解码，与它的载荷类型住在一起**。
`Field` 住 `px_field_schema` ⇒ 那份 impl 在那边；`VolumeData`/`MeshData` 住 `px_protocol`
⇒ 这两份 impl 在 `px_protocol/src/payload.rs`。`PayloadBundle` 本来就是 CAS 里那份文件的形状
（线格式），住 `px_protocol` 反而更正 —— `px_graph_schema::payload::{Build, PayloadBundle}`
留成 re-export，别处的 import 一个没改。

图脚本一侧现在只有**一行**：

```rust
use px_cook::{Domain, GraphSpec, begin, cameras, cook, field, mesh, volume};
```

---

## §170 机制：M1（运行期装载），不是 M2（链接期导入）

两条路都**实测满足** R1（改实现 ⇒ 图 exe 字节不变、跑起来却是新实现）。探针读数
（`target/probe/`，一次性、已删）：

| # | 机制 | 量法 | 读数 |
|---|---|---|---|
| 1 | M2 | `-l dylib=probe_impl` | **LNK1181**：rustc 传 `probe_impl.lib`，而 MSVC 的导入库叫 `probe_impl.dll.lib` |
| 2 | M2 | 改成 `cargo:rustc-link-arg=<绝对路径>.dll.lib` | 链接、运行、测试全通；类型化 extern（`&Params` / `&[f32]` / `Result<i32,String>`）可用 |
| 3 | M2 | 改实现体 → `cargo build` | **0.44 s**，`probe_user.exe` **字节不变**，跑起来打印新值 ✓ |
| 4 | M2 | 加 `[build-dependencies] probe_impl` 当排序边 | **破坏 R1**：0.84 s，`probe_user.exe` **变了**（cargo 把 build 脚本的指纹传给了消费者） |
| 5 | M2 | 无辜消费者（依赖声明 rlib、不链 dylib） | **LNK2019**：`Fbm::render` 那个 CGU 被拉进来，未解析符号跟着来 |
| 6 | M1 | `libloading` + 一次 `transmute` | 同 #3 的形状；**没有** #1/#4/#5 那三条麻烦 |

⇒ 选 **M1**。它的全部代价收在三处，且都有出口：

* **一处 `unsafe` transmute**（全仓唯一）：`ops::body::<O>()` 把符号地址当成"签名由
  `Body<O>` 钉死的函数"。签名不是从字节里猜的 —— 它由 `O::Params/Inputs/Payload` 推，编译期检查。
* **DLL 搜索规则**：`PX_OP_DIR` > exe 同目录 > exe 的上一级（测试 exe 在 `deps/` 里）> 当前目录。
  找不到**当场拒**，报错里带该跑的命令。
* **装载是懒的**：第一次 `render` 才 `GetProcAddress`（一个算子一次）。

一条**契约握手**：实现库导出 `…__contract_hash`（它编的时候 `px_graph_schema` 是哪一份），
装载时与图程序手里那份比 —— 对不上就拒（改了契约而 DLL 没重编时，宁可拒也不要拿错布局去调）。
`…__source_hash` 则是**实现那一份**的指纹，它进键。

---

## §171 读数（改实现不重编图程序：一条条量出来的）

### §171.1 产物**逐字节**（M1 之后 vs 静态链时代的基准）

| 图 | 逐节点比对 | 说明 |
|---|---|---|
| planet | **6/6** | 键全换了（§168 + 实现搬家），产物字节一位不差 |
| desert | **8/8** | 同上 |
| clouds | **14/14** | 同上（含 26 MB 那一档体积） |

比对法：按**节点名**把新清单里的键映射到新 CAS 路径，与 `target/baseline/*.json`
（拆分前在主管 worktree 上取的 node→key）指向的基准产物逐文件 sha256 比。
**判的是字节不是键** —— 键本来就该变（它含实现身份）。这是"语义一位没动"的硬证据。

### §171.2 R1：改一行**实现** ⇒ 图程序 exe 字节不变

同一 worktree、`dev` profile、`cargo build --workspace --exclude px_render`：

| 量法 | 读数 |
|---|---|
| 空转（先把基线坐实） | **0.32 s**，七个图 exe 一个都没动 |
| 往 `px_field_op/src/ops/fbm.rs` 加一条注释 → `cargo build` | **1.09 s**；`px_field_op.dll` **变了** |
| 同一次改动下的七个图 exe（planet / desert / clouds / scene / shaders / passes / field_probe） | **一个都没变** ✓ |
| 只编实现库：再改一行 + `cargo build -p px_field_op` | **1.00 s** |
| 同一次改动下的 `cargo build -p px_graphs --bins` | 1.40 s，什么都不重编 |
| 跑一遍 `--bin planet` | 键换了（`continents` `8f2c6f198ab9` → `fdb742f220fa`）；读数一位不差：`780×520｜值域 0.1723..0.8130｜均值 0.5208` |

对照旧形状（§161 那一轮）：算子当**普通依赖**时改实现要重编重链图程序（直接依赖 dylib
实测 3.41 s 且 exe 被重链）。今天这条路是"实现变了、图 exe 一位没动"。

⚠ 量法上踩过一个坑（记下来免得下次白量）：**基线快照必须与对照用同一条命令取**。
第一次拿 `cargo run -p px_graphs --bin planet` 建好的 exe 当基线、再用
`cargo build --workspace` 对照 —— 特性合并不同（`--workspace` 会把 `px_render` 并进来），
exe 本来就会被重链，于是读数变成"图程序 exe 变了"的假警报。

### §171.3 J1：六张判据图与登记值**逐字节相同**

`scene <档>` + `px_render --offline --scene <产物> --width 960 --height 640`（Vulkan / RTX 3060）：

| 档 | 出图 sha256 前 16 | 登记值 | 字节数 |
|---|---|---|---|
| orbit-bare | `63184151909371A5` | 同 ✓ | 300012 |
| orbit-bare-nolight | `7BBB18CE3612D4F7` | 同 ✓ | 215193 |
| orbit-bare-shadow | `C03FFF3235264DD5` | 同 ✓ | 298289 |
| orbit-rings | `B5799E4F1649535C` | 同 ✓ | 508562 |
| orbit-soft | `FA20FAD37BC61EA2` | 同 ✓ | 358066 |
| orbit-proxy-fine-bound | `32872F80AC867BE3` | 同 ✓ | 405380 |

⇒ 键全换了、实现搬进了运行期装载的 dylib，而**画出来的像素一位没动**。
（`orbit-rings` 隔一轮再出一次图，仍是 `B5799E4F…`。）

### §171.4 测试与两道新门

`cargo test --workspace --exclude px_render` 全绿。新增：

* `px_graphs/tests/ops_load.rs` —— **真的去装载**每一个声明过的算子（十个）、读三个库的身份、
  验三个身份互不相同（顺带证明"实现那一半真的进了身份"）。这是"声明 ↔ 实现那条字符串线
  编译器看不住"的替代门：改名/改库名会在测试里当场红，而不是等到某次跑图。
* `px_graph/tests/source_hash.rs` —— 把 `px_graph_schema`（契约指纹，握手用）纳入
  "每个被指纹覆盖的 crate 都要有 build.rs" 那条门，并新增"每个实现库都要有 `px_impl_lib!()`"。
* `px_graphs/tests/local_op.rs` —— 图侧现写算子的真样本（见 §173）：泛型函数对两个闭包
  两个实例、现写算子能算/落盘/再命中、身份等于 `px_graphs::SOURCE_HASH`、两个现写算子键不同。
  ⚠ 它的断言**不许依赖缓存是空的**（CAS 是持久的，第二次跑必然命中）—— 第一版我写成
  `assert!(!band.hit)` 就踩了这个坑（单跑通过、全量跑失败）。

### §171.5 六格 anchor 重登记（`art/anchor/hashes.txt` §三）

**干净重编之后**的最终登记值（`cargo clean -p` 三个实现库 + 契约 + 门面 → 全编 → 先确认
"再跑一遍全命中"→ 才量的）：

* orbit-bare `5D120324D0FFDE42`（4123 B）、nolight `5ACEE28C67370B7B`（4136 B）、
  shadow `C17EAD3162D90066`（4138 B）、rings `49C8A4D887A9F17E`（4891 B）、
  soft `34F26DF603040D96`（5719 B）、proxy-fine-bound `6364535D62D448D3`（5749 B）。
* J1 抽查（同一状态）：orbit-bare `63184151909371A5` ✓、orbit-proxy-fine-bound `32872F80AC867BE3` ✓
  —— 与 §一 登记值逐字节相同。
* 更早三批（`2795F948…`、`2A7B42A7…`、`34B17990…`）**留在表里当历史**。⚠ 其中 `34B17990…`
  那一批是**过渡态**下量的（见下一条），不算数；留下它是为了记住这个坑。

⚠ **量这一格的方法坑（这一轮踩了两次，值得单列）**：§171.2 那种"改一行实现 → 量 → 撤回"的
探针，撤回若用**保留 mtime** 的复制（`Copy-Item` 一个 `.bak`），cargo 按 **mtime** 判"文件没变"
⇒ **不重编** ⇒ DLL 停在**探针身份**上，于是之后每一次读数（包括 anchor 的登记值）都在错的
身份上。对策：撤回后 `cargo clean -p <实现库>`（或在探针里改内容、用会刷新 mtime 的写法撤回），
然后**先确认"同一档再跑一遍全命中"**，再量 anchor。
⇒ 教训与 §171.2 那条同源：**先证明状态是静止的，再量**。

* ⚠ 本 worktree 是**新 checkout**，`art/shaders/blit.wgsl` 这类在本机是 CRLF（1581 B）
  而原工作副本盘上是 LF（1554 B）⇒ "同一个 recipe 的文档字节"在两个副本之间本就会不同。
  J1 不受影响（行尾约定不改像素），§三 的值属于**本副本**。

> ⚠ 就地更正（2026-09-20）：上面 §171.5 那六格是**那一轮**的值，**今天生效的不是它们** ——
> 是 `art/anchor/hashes.txt` §三 里 **`A55C3ED0391DA075` 那一批**（第四次重登记；根因见
> `docs/system/build-graph.md` §188）。§171.5 那几行**留着当历史值批次**（§六 那一格要拿它们讲事）。

---

## §172 已知代价与未做

> ⚠ 就地更正（2026-09-20）：本节写于 M1 那一轮。其中"生成式单态化实例"相关的两条**已经变了**：
> 当时记的是**图自建 `xxx_op` crate 并着用**（`16` §159 口径 5，且 `17` §166 有一版 `mono-gen` 生成器），
> 那条**原型期的路已删**；今天它以 **实例那一档**（图侧一张**数据收据** recipe + stage 1 生成类型，
> 见 `docs/system/codegen-types.md`；更早几轮是 `px_inst!` 宏，见 `docs/system/generic-instances.md` §174–§180）的形式**回来了**
> —— 而 `px_local_op!`（§173）仍是"泛型代码住在图程序里"那一档。本节其余各条
> （`cargo test -p px_graphs` 要先编实现库、跨 dylib ABI 的两道握手、实现库比源码旧只告警、
> 两处 `px_cook` 遗留字符串不许改、`Grid` 只有一个出口）**今天仍然成立**。

* **`cargo test -p px_graphs` 需要先编实现库**（`-p` 不会带上它们）：`cargo build` /
  `cargo test`（默认 members）会编；`tools/px.ps1` 跑图前也显式编了那三个包。
  缺库时的报错带命令，不静默。
* **跨 dylib 的 `extern "Rust"` ABI**：同一份 rustc + 同一份契约（握手只保证后者）。
  契约层源码一变，实现库就得重编 —— 这正是握手要挡住的那种情况。
* **实现库比源码旧**时只**告警**（不拒绝）：跑的是旧实现，键也跟着旧身份走，
  不会有陈旧命中；但"你以为在跑新的"这件事必须说出来。
* 六个 anchor 登记值因 §168 换了一次（新值在 §171.5，旧值留在表里当历史）。
* **两处 `px_cook` 时代的遗留字符串不许顺手改**（它们读起来像可改的，其实是**域分隔符**，
  改 = 全仓换键）：`keys.rs` 的 `b"px_cook/v1"`、`contract.rs` 的 `b"px_cook/interface/v1"`。
  两处都已就地写明。
* **画布（`Grid`）已经不存在了**（2026-09-27，用户裁定「不允许添加画布这个概念，一切皆参数」）：
  `Cache::grid()` 这个出口、`Build::RESOLUTION_IS_CANVAS`、`GraphSpec` 的尺寸/投影/相机三样
  全删 ⇒ 尺寸与投影是**参数**、相机是**场景脚本**的数据。见
  `docs/system/params.md`。
* `px_probe` / 探针那一侧没动（它们不碰算子库）。

---

## §173 图侧现写算子：`px_local_op!`（目标里那一句"agent 在图侧现写泛型算子"）

`px_op!` 与 `px_local_op!` 是同一张表的两个落点，**差别只在身份那一半**：

| | 实现住哪 | 身份（进键的那一半） | 什么时候重编 |
|---|---|---|---|
| `px_op!` | `px_*_op` 的 dylib | **那份库**的源码指纹（运行期从库里读） | 改实现 ⇒ 只重编库，图 exe 一位不动 |
| `px_local_op!` | **本图程序**（`impl PxOp` 就在图脚本里） | **本 crate** 的源码指纹（`env!("PX_SOURCE_HASH")`，由本 crate 的 `build.rs` 给） | 改图脚本 ⇒ 图程序重编 |

这就是"泛型算子"那一档的落点：泛型参数（一段现写的场函数、一个闭包）在**图侧**实例化，
单态化出来的代码编在图程序里 —— 实现库不参与、也不需要知道那个 `F` 是什么。

真样本在 `px_graphs/tests/local_op.rs`：一个图侧 `BandParams` + 一个泛型 `bake<F: Fn(..)->f32>`
+ 两个共用它的现写算子（`Band` / `Rings`）。它量三件事：现写的算子能算/能落盘/**再跑一次命中**、
身份等于 `px_graphs::SOURCE_HASH`、两个现写算子（id 不同）键不同。⚠ 它用自己的图名
（缓存落 `target/pcg/local-op/`），**不碰 `art/` 下任何既有图** ⇒ 加图侧算子动不到六份冻产物。

⚠ **粗是故意的**：图侧算子的身份是"整个图程序这一份源码"，所以改**任何**一个 bin
都会让所有图侧算子换键。要更细的粒度、要跨图复用 ⇒ 写成 `px_*_op` 里的正式算子。
这条口径也写在 `px_graphs/build.rs` 顶上。

