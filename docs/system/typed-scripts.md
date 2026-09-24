# 图脚本走「类型化算子契约」：op_id/字节边界换成普通 Rust 调用链

> 起因（2026-09-19，用户口径）：**「不显式建图，只是普通 Rust 逻辑加缓存辅助函数，类型检查最大化」**。
> § 号接 `docs/system/graphs.md`（§159/§160 是拆分那一轮）。
> 这一篇是**决议 + 事实 + 读数 + 未做**，代码已落（提交 `7feaecf`，分支 `feature/typed-graph`）。
>
> ⚠ 本篇是**那一轮**的记录。之后两轮的落地形状以 `docs/system/operators.md`（算子实现回 dylib、
> 运行期按身份装载）与 `docs/system/generic-instances.md` + `docs/system/build-graph.md`（`px_inst!` / build graph）为准。
> 本篇里这些具体东西**已经不在了**：§166 那套「每图自建 `mono-gen` + `mono.rs` 配料清单」的
> 生成器（连同 `px_graphs/src/mono.rs` 的 `INGREDIENTS`）、描述符表（`OpLibrary`/`OpTable`）、
> 老 `node()` 路径、以及 `px_cook/src/field_fn.rs` 那个位置（`FieldFn` 搬去 `px_volume_alg::field_fn`）。
> §166/§167 各自另有就地过期标记。

---

## §161 先钉三条**测量**出来的事实（它们改了设计）

这一轮开头我按「静态链算子 + 类型化调用」画了一版设计，**量了三组读数之后推翻了它的一半**。
读数是这一轮的起点，所以放在最前面（同一 worktree、`dev` profile、Windows/MSVC、rustc 1.97.1）：

| # | 量法 | 读数 | 它推翻了什么 |
|---|---|---|---|
| 1 | 冷编全套（三个算子库 + 图程序） | **19.0 s** | 「编译时间」这条线的收益上限**很低** —— 为它做复杂机制不划算 |
| 2 | `cargo build -p px_graphs` 空转 | **0.16 s** | 基线 |
| 3 | 改 `px_field_op` 的算子体（新增 `pub fn`、真改 MIR）→ `cargo build -p px_graphs` | **0.16 s，什么都没重编** | ⚠ **「改算子不重编图程序」原本就是真的** —— 因为 `px_field_op` 只住在 `px_graphs` 的 `[dev-dependencies]` |

⚠ 第 3 条最要紧：我原计划把算子做成图脚本的**普通依赖**，那会**把这个性质弄丢**。
所以设计改成：**算子体留 dylib；图脚本静态链接的只是"契约"那一层（没有实现）**。

### §161.1 一条方法上的坑（别浪费时间去撞）

**rustc 的指纹是按内容（MIR）的，不是按文件字节**：
- 加一个没人调用的 `pub fn` ⇒ 什么都没重编（0.15 s）；
- `0.0 * 1.0` 被常量折叠 ⇒ 也没重编；
- 只有改了**会留进机器码**的东西才触发。

⇒ 量增量编译时，"追加个注释/死函数"这种量法是**空转**（[同一现象在社区也被独立发现过](https://trigodil.github.io/blog/rust-compiler-build-times.html)：
作者一开始用"追加注释"量，报出的 15% 全是噪声）。

---

## §162 决议：契约在 `px_cook`，实现在 `px_*_op`，两者分家

```text
px_protocol          线格式
     ↓
px_graph_schema      契约层（Key / PayloadBundle / 描述符 / OpLibrary）
     ↓
px_*_schema          各领域的数据与参数
     ↓
px_cook        ★新   类型化契约：Op trait（Params / Inputs / Payload）+ 缓存辅助函数
     ↓                ⚠ 一个算子实现都不依赖 —— 它只认识 px_graph 的 Cache 接口
px_*_op              各领域算子：**实现**（dylib 那一半）+ **typed 接线**（rlib 那一半）
     ↓
px_graph             图库本体：新增 `Cache` trait + `Driver` 实现（查/写 CAS、清单、读数）
     ↓
px_graphs            图脚本：普通 Rust —— `cook_field::<field::Fbm>(&cache, "clusters", (), grid)?`
```

**两条不许破的线**（`px_graphs/tests/crate_graph.rs` 看住）：

1. **动态装载的那一半（`crate-type` 含 dylib/cdylib）不许依赖 `px_graph`** ——
   否则运行时装载进图程序会让**同一个进程里出现两份驱动**（CAS / 清单 / 索引各一份）。
   ⚠ 这是替代老门的那一条：老门禁的是"图脚本静态依赖算子"，而图脚本现在**必须**静态依赖算子
   （那是拿回类型检查的手段）。
2. **`px_graph` 仍然不许静态依赖任何算子**：它只认 `Cache` 那几个方法。

### §162.1 图脚本长什么样（老 vs 新）

```rust
// 老（§159 的形状）：字符串 id + 字节边界，参数类型/输入个数/输出域都是运行期的事
let clusters = node(field_params::FBM, "clusters", &[]);
let mixed    = node(field_params::MIX, "mixed", &[&clusters, &carved, &weight]);

// 新：普通 Rust —— 类型检查拿回来了，缓存藏在辅助函数里
let clusters = cook_field::<field::Fbm>(&cache, "clusters", (), canvas)?;
let mixed    = cook_field::<field::Mix>(&cache, "mixed", &[&clusters, &carved, &weight], canvas)?;
let coarse   = cook_volume::<volume::CloudCoarse>(&cache, "coarse", &mixed, canvas)?;
let proxy    = cook_mesh::<mesh::Proxy>(&cache, "proxy", &coarse, canvas)?;
```

拿回来的三样：**参数类型**（`Params` 是具体类型，写错字段编译不过）、
**输入个数**（`Inputs<'a>` 里写死，接错编不过）、**输出域**（`Payload = Field`，
把它喂给吃体积的算子编不过）。

### §162.2 键多了一维：**源码哈希进键**

`px_cook::cook_key` 与 `px_graph_schema::node_key` 的差别只有一项：
`Identity.source_hash`（编译期常量，`include_str!` 的 FNV）**也进键**。

理由：类型化这条路里，算子语义可能随源码变而 `VERSION` 没升 ⇒ 那样缓存会**静默给旧产物**。
进了键就是"必然重算"，而 `VERSION` 仍旧只用于打告警。
⚠ 老路径（`node_key`）**一位未动** ⇒ 老图的老键全部照旧命中。
⚠ 相机那一档照旧：产物里带着相机表 ⇒ 相机进键（`key_with_cameras`），体积不进。

---

## §163 判据（这一轮的硬读数）

### §163.1 产物**逐字节相同**：14/14 ✓（决定性的一条）

做法：同一 worktree，先跑新路径存清单，再 `git stash` 回老路径跑一遍，逐节点对比产物**去掉清单帧里节点名之后**的字节。

```text
  billows       1574081 B  sha 628affb25d = 628affb25d   [逐字节相同]
  carved        1574080 B  sha 48fb6aa1ec = 48fb6aa1ec   [逐字节相同]
  clusters      1574082 B  sha 9247318ba3 = 9247318ba3   [逐字节相同]
  coarse        6591297 B  sha b65e7f630f = b65e7f630f   [逐字节相同]
  coarse_fine  25960267 B  sha 8588bde6a9 = 8588bde6a9   [逐字节相同]
  coverage      1574081 B  sha c387437b33 = c387437b33   [逐字节相同]
  flow          1574078 B  sha 3b00266d12 = 3b00266d12   [逐字节相同]
  mixed         1574077 B  sha b3b4a7cc92 = b3b4a7cc92   [逐字节相同]
  proxy         1639947 B  sha 1594399211 = 1594399211   [逐字节相同]
  proxy_fine   14774239 B  sha f7f6b04f07 = f7f6b04f07   [逐字节相同]
  slope_x       1574080 B  sha 931fde9a1e = 931fde9a1e   [逐字节相同]
  slope_y       1574079 B  sha c8c047422e = c8c047422e   [逐字节相同]
  slope_z       1574081 B  sha 53320a83cf = 53320a83cf   [逐字节相同]
  weight        1574079 B  sha 3fe565a10e = 3fe565a10e   [逐字节相同]

⇒ 14/14 逐字节相同
```

⚠ 这条**抓到过一个真缺陷**，记在这里（它是这一轮最值钱的一处发现）：

> 算子回的是**无名、无相机**的占位载荷 —— `bundle.placeholder()`。
> 「节点名 + 相机表」是**驱动**在写盘时补的（老路径那句 `bundle.to_bytes(name, &cameras)`）。
> 新路径第一版直接把算子回的字节写盘了 ⇒ 产物里 `id` 空、相机空。
> 逐字节对账当场抓到（差异正好是**节点名长度 + 相机表大小**）。
> 修在 `Driver::store` 里：先 `bundling(...)` 补成产物，再写、再记清单。

**这一条说明"逐字节对账"值得当规矩**：它抓到的不是浮点差异，是**协议语义漏了一步**。

### §163.2 判据（判据本身）照旧跑通

`check("coarse", ...)` 与 `check("coarse_fine", ...)` 两条：
包住判据（212 / 127 条有交点的方向，漏 0 条）、梯度上界（140.671 ≤ scale 240、162.550 ≤ 1024）、
场网格规模（6 面 × 65² × 65 层、6 面 × 129² × 65 层）—— 与老路径**同一组数**。

### §163.3 测试

`cargo test`（默认 members）：**51 个测试目标、180 个用例通过、0 失败**。
其中 `px_graphs/tests/crate_graph.rs` 改成 4 条（新口径，见 §162）。

### §163.4 增量编译**代价**（如实记，这条是退步）

| 改动 | 老路径 | 新路径 |
|---|---|---|
| 空转 | 0.16 s | 0.29 s |
| **改算子体**（`px_field_op/src/ops/fbm.rs`） | **0.16 s（什么都不重编）** | **3.5 s（重编 `px_field_op` + `px_graphs` + 重链）** |

⇒ **这一刀把「改算子不重编图程序」那条性质换成了类型检查**。冷编 19 s、改算子 3.5 s，
都在可接受范围；但要把这条性质**也**拿回来，唯一的路是**把单态化实例放进独立的 dylib**
（§164 的第二条），那需要生成器。⚠ 按 §161 的读数，这件事的收益上限不高，**别先做**。

---

## §164 泛型方法：`bake<F: FieldFn>`（这一轮的原始目标）

**要的是泛型方法，不是 `dyn`。** 落成的形状：

```rust
// px_volume_alg/src/field_fn.rs —— 算子唯一需要的那个抽象
// （⚠ 2026-09-20 更正：它今天住 `px_volume_alg::field_fn`，不是 `px_cook/src/field_fn.rs`；
//   那个路径已随"泛型算法搬进 alg rlib"一并删掉 —— 见 `19` §178 的 S1）
pub type CoverCloud = px_verify::cloud_field::CloudFieldParams;
pub trait FieldFn {
    fn cover(&self, cloud: &CoverCloud, direction: [f32; 3]) -> f32;
}
pub struct SampleField<'a> { pub field: &'a Field }     // 上游采样场当函数（老路径）
// ⚠ `ClosedForm<F>`（图脚本现写的闭式场）已删：今天那一档走 `px_inst!`（§174–§180）。
// px_volume_alg/src/lib.rs —— 泛型方法
pub fn bake<F: FieldFn>(params: &params::Params, cover: &F) -> VolumeData
pub fn eval_sampled(params, &Field) -> VolumeData        // = bake::<SampleField>
pub fn coarse_with<F: FieldFn>(params, &Field, &F) -> VolumeData  // = 泛型实例那一档的入口
```

⚠ **一处接口设计上的坑**（记下来，别重犯）：第一版把云参数放进 `SampleField` 的字段里，
结果是「`bake` 自己建云、而场函数也要云」——**同一个东西建两遍/借两次**。
正确的分法是：**算子建上下文、当参数交给场函数**（`cover(&cloud, direction)`）。
这样「云参数怎么算」只有一处（`proxy::from_volume`），闭式场不必自己再推一遍
`to_local` / `cover_from_mask` 的口径。

### §164.1 判据

* **默认路径逐字节不变：14/14 ✓**（`PX_PCG_FRESH=1` 强制重算后对账，同 §163.1 的方法）。
  泛型化**没有**动语义 —— `SampleField::cover` 就是转发到 `proxy::cover_at`。
* **闭式路径能跑**（`cargo run -p px_graphs --bin clouds -- --closed-cover`）：

```text
覆盖度对账（2000 条方向）：老路（回采混合场）0.0000..0.8479｜闭式路（图上现算）0.0000..1.0000
  两路最大差 +1.0000 —— 两条路问的是**不同的场函数**，这个差就是「换了一个覆盖度」的代价，不是误差
  ⇒ 同一个算子、同一份 `bake<F>` 体；闭式路省掉了整张覆盖度场（393216 个采样）
```

⚠ 第一次量的时候我比的是**烘出来的体积**，两条路的值域**完全相同**（都 `-0.0008..0.0033`）——
因为 `shape` 在覆盖度低于阈值时两边都归零 ⇒ **那个比较没有信息量**。
改成在**同一批方向**上并排量覆盖度本身之后才有读数。这条值得记住：
**判据要选在信号还没被下游压平的那一层。**

### §164.2 泛型方法的代价（必须说清）

`bake<F>` 的实例在**图脚本那一侧**生成 ⇒ **`bake` 的体被静态链进图程序**。
后果：改 `bake` 的体要重编图脚本（这是 §161 第 3 条那条性质在**类型化这一支**上必然的代价）。
老路径（dylib 里那份 `eval_sampled`）仍在，所以 dylib 那一支一位没动。

---

## §166 生成单态化实例 + 动态链接（这一级终于做了）

> ⚠ 已过期（2026-09-20）：本节描述的**具体机制整套已删** —— `px_graphs/src/bin/mono-gen.rs`、
> `px_graphs/src/bin/<图>/mono.rs` + `mono/` 模板、`px_mono_<图>_op` 这些**今天都不存在**；
> `px_graphs/src/mono.rs` 的 `INGREDIENTS` 配料清单（§167 第 3 条）也没了。
> ⚠ 但本节要的**性质**（改一行泛型参数 ⇒ 图 exe 字节不变、键必变、动态装载）**今天成立了**，
> 只是换了形状：**`px_inst!` + build graph**（`docs/system/generic-instances.md` §174–§180、
> `docs/system/build-graph.md` §183/§184），另有图侧现写的 `px_local_op!`（`18` §173）；
> 产物落在 `target/pcg/inst/<key>.dll`。本节留下的是"这一级存在的理由"与那几条构建卫生的教训
> （§166.3/§166.4/§166.5 的"`px_` 导入为零、别往 `target/debug/` 拷兄弟 DLL、两份构建图各有各的
> `target/`"——这些**仍然有效**）。

**要的是「自动生成单态化 → 编成 dylib → 动态装载」，不是把实例静态烘进图程序。**
⚠ 而且**两个 stage 都必须住在 graph scripts 底下**（用户裁定：不是在 `tools/` 那种通用位置）。

### §166.1 布局：**每个图的 mono 跟它自己的图脚本同一个目录**

⚠ 这一条是用户裁定的：单态化是**这张图自己的事**，不是全图共用的一个位置
（放在共用位置时，第二张图会撞上第一张：同名库互相覆盖、同一个 id 互相抢、
一份 `INGREDIENTS` 绑住所有图）。

```text
px_graphs/src/bin/
  clouds.rs                    图脚本本体（stage 2 的调用者）
  clouds/
    mono.rs                    ⭐ 这一张图的**单态化声明**（`key = value` 文本，不是 Rust 模块）
    mono/
      fields.rs                ⭐ stage 1：要单态化的那一半（**编辑面**）
      Cargo.toml / lib.rs / identity.rs   stage 2 的三份模板
  mono-gen.rs                  生成器（吃图名：`-- clouds`）
```

用法：`cargo run -q -p px_graphs --bin mono-gen -- clouds`。

**为什么声明是 `key = value` 文本而不是 Rust 模块**：它要读的东西只有三样（库名 / id / 配料清单），
而被三个不同的宿主牵动（`clouds.rs` 要 id、生成器要全部、`fields.rs` 谁都不要）。
做成 Rust 模块就得为三个宿主凑出都能编译的形状（`//!` 与 `///` 在 `include!`/`#[path]`
的位置会报错 —— 实测 `E0753` / `unused_doc_comments`）。文本声明把这些纠缠一次去掉：
**id 只有一处**，图脚本用 `include_str!("clouds/mono.rs")` + `declared("id")` 读它。

**每张图三样都不同**：库名（`px_mono_clouds`）、算子 id（`clouds.volume.cloud.coarse.closed`）、
stage 1 的场函数。生成物落在 `target/debug/px_mono_clouds_op.dll`
（入口名由**库名**派生：`#[export_name = concat!("<库名>", "_op_table")]`）。

**`bake<ClosedForm<…>>` 的实例是在那个 dylib 内部单态化出来的** —— 这是这一级存在的全部理由
（泛型的实例化要求"定义"与"类型参数"在同一个编译单元里，而图程序是 `bin`）。
图脚本只写 `node(declared("id"), "coarse_closed", &[&source])` ——
**`clouds.exe` 里没有那个算子的任何代码**。

### §166.2 读数（改一行场函数）

| | 读数 |
|---|---|
| 改一行 `clouds/mono/fields.rs` → 生成 + 编 dylib | **1.7 s 级** |
| 同期 `clouds.exe` 的 sha256 | **不变**（图程序一个字节都没重编） |
| 下一次跑（`--closed-cover`） | 键必变 ⇒ **必然重算**，不可能陈旧命中 |
| 生成物大小 | **2.89 MB**（对比 `px_volume_op.dll` 的 13.8 MB） |
| 全套测试 | **180 通过 / 0 失败** |

### §166.2 ⚠ 身份与陈旧命中：**算子的源码哈希必须进键**

做这一级时暴露了一个**真问题**，值得单独记：

> mono 实例的身份住在 `SOURCE_HASH` 里，而**键不含 source_hash** ⇒ 改了场函数之后
> **命中旧产物**，只有一行 stderr 告警（`⚠ 源码变了但 VERSION 仍是 1`）。
> 这正是 §159.4 那条对账机制在守的那件事，而它**只告警、不拦**。

修法（`px_graph/src/driver.rs`）：在相机之后**把 `descriptor.source_hash` 折进键**
（新函数 `key_with_source_hash`，做法与 `key_with_cameras` 同一个位置、同一种混法）。

⚠ **代价要说清：老产物的键会因此全部失效（第一次重烘一遍）** ——
这是故意的：换掉的正是"源码变了而版本没升"那一档的陈旧命中。之后老键稳定。
**这一条是改口径，需要用户确认**（类型化那一支 `px_cook::cook_key` 从一开始就是这么做的）。

### §166.3 ⚠ 构建卫生：这一级最难的地方（三条都踩过）

| 做法 | 结果 |
|---|---|
| 生成的 crate **共用主 workspace 的 `target/`** | ❌ **回归**：它的依赖图不同（自己一份 `Cargo.lock`、自己的特征统一）⇒ cargo 把 `px_volume_op.dll` 等**再编一份**进同一个 `target/debug/deps/` 覆盖主 workspace 那份 ⇒ `LoadLibraryExW failed`（实测复现） |
| **target 也按 mono_key 分** | ❌ 每次改一行场函数都在**新目录**里从零编依赖（**20.8 s/次**），比不带这一级还差 |
| **构建目录固定 + 源码目录固定** | ✅ **1.56 s**（依赖编一次、之后复用）。代价：`target/mono/crate` 这个位置**不代表身份**，身份由内容（`SOURCE_HASH`）带 —— 想同时要"内容寻址的源码路径"与"复用的依赖"，得自己管依赖产物的存放（**未做**） |

### §166.4 ⚠ 一个会咬人的操作约束（实测两次）

**生成器与主 workspace 不能同时污染同一个 `target/`**：混过一次之后，
`target/debug/` 里的算子 DLL 会被另一个构建图的产物覆盖（或留下半成品），
表现是 `LoadLibraryExW failed`，而 `cargo build` **判它 fresh、不会重编** ——
所以看上去"什么都没改却坏了"。恢复办法是 `cargo clean -p <那三个算子 crate>` 重编。

⚠ 更阴的一点：**装载器的诊断探针（单独一个 crate）能把 4 个库全装好，而图程序却失败** ——
两个进程的模块搜索路径不同（探针从自己的构建目录起、图程序从 `target/debug` 起），
于是同一个半成品在两边表现不同。所以"探针说没事"**不能**当"图程序没事"的证据。

⚠⚠ 而这条错**最可能的来源**是"把另一个构建图的同名 DLL 拷进 `target/debug/`" ——
我自己在早期版本的生成器里干过这件事（见 §166.5 的更正）。教训：
**从 `target/mono/` 里往 `target/debug/` 拷任何 `px_*.dll` 都是错的，一份都不行。**

### §166.5 ⚠ 更正：生成的 dylib **是自足的**（我先前那条结论是错的）

先前这里写着"`dylib` 这个 crate-type 的 ABI 是递归的 ⇒ 生成物要一整套上游 DLL"。
**那个结论错了**，测出来的事实是：

```text
px_mono_clouds_op.dll：9 个导入，全是系统库（kernel32 / ntdll / …）
                       px_ 前缀的导入：**0 个**
```

cargo 对 **path 依赖**选的是 `rlib` ⇒ 上游被**静态链**进这一份 cdylib。
上游 crate 虽然写了 `crate-type = ["dylib", "rlib"]`，那只是"**被直接依赖时**也可能要 dylib"，
而生成物这边拿到的是 rlib。于是这一份实例**自足、不需要任何兄弟 DLL**。

**那 `LoadLibraryExW failed` 是怎么来的**：是我早期版本的生成器**把 mono 构建目录里的
同名 DLL 拷进 `target/debug/`**（`px_field_op.dll` 等）—— 那份属于**另一个构建图**
（`target/mono/<库名>/build/`，自己一份 `Cargo.lock`、自己的特征统一），
覆盖掉主 workspace 的正确产物 ⇒ 后者再也装不上。**是我自己造的，不是 ABI 约束。**
（mono 构建目录里确实躺着一份 `px_volume_op.dll` —— 那是 cargo 为那个 package 的
`dylib` crate-type 顺手产的，**没人链接它**。）

⇒ 现在的规矩（**已落到生成器里**）：只deposit**那一份 cdylib**，一个兄弟都不拷；
并且每次生成都**读一遍 PE 导入表**，任何 `px_` 导入就当场炸
（`mono-gen.rs` 的 `assert_self_contained`）——
因为这一条是**脆的**（哪天有上游改成纯 `dylib` crate-type 就变），而它坏了只在运行期冒出来。

⚠ 剩下那条**仍然成立**的约束：`mono-gen` 与主 workspace 的构建必须**各用各的 `target/`**。
两份构建图各有各的依赖解析，产物**不能互换**；谁把它们写进同一个目录，谁就制造
"`cargo build` 判它 fresh、不重编"的半成品。混过之后要
`cargo clean -p <那几个算子 crate>` 才恢复。

---

## §167 未做 / 下一步（按价值排）

> ⚠ 已过期（2026-09-20）：这是**那一轮**的待办清单。逐条今天的状态：
> ① "另外三张图没迁" —— 老 `node()` 路径**整体已删**，不存在这条迁移了；
> ② "生成的实例只接进 `--closed-cover` 演示分支" —— 那套生成器已删，今天实例那条路是
> `px_inst!`（`19`/`20`），并且有 `inst_gate` 端到端在调；
> ③ "生成器的配料清单仍是手维护的（`px_graphs/src/mono.rs` 的 `INGREDIENTS` 15 条）" ——
> **形状整个换了**：`catalogue()` 和 `INGREDIENTS` 都不在了，节点由 `insts::build()` 这一处派生；
> ④ "§166.4 那条 dylib 依赖（上游 DLL 递归）没解决" —— 那条结论本身已在 §166.5 **更正**（cdylib 自足）；
> ⑤ "`source_hash.rs` 那道门没覆盖 `typed.rs`" —— `typed.rs` 已删；
> ⑥ "`node()` 与 `cook_*` 的键坐标系不同" —— `node()` 已删，这条随之消失。
> 保留本节是为了记住"那一轮还欠什么"。

1. **另外三张图没迁**（`planet` / `desert` / `scene` 仍是老 `node()` 路径）。纯机械活。
2. **生成的实例只接进 `--closed-cover` 这个演示分支**，没进任何一张图的正路。
3. **生成器的配料清单仍是手维护的**（`px_graphs/src/mono.rs` 的 `INGREDIENTS` 那 15 条，
   写成定长数组就是为了多一条编译期就报）。这一条比"手写脚本"好，但还没到
   「由 `fields.rs` 的声明自动推出来」。
4. **§166.4 那条 dylib 依赖**（上游 DLL 递归）没解决。
5. **`source_hash.rs` 那道门没覆盖新的 `typed.rs`** 与模板里的 `SOURCE_HASH`。
6. **`node()` 与 `cook_*` 的键坐标系不同**（前者现已含 source_hash，后者含 source_hash 但
   混法不同）⇒ 两条路的同名节点键不同。已知、未堵。

---

## §168 与 `docs/system/graphs.md` 的关系

| §159 的口径 | 这一轮之后 |
|---|---|
| 图脚本依赖 `xxx_schema` | **仍成立**，但它现在**也**依赖 `px_cook`（类型化契约） |
| 图脚本不许静态依赖任何 `*_op` | **改口径**：允许（那是类型检查的手段）；替代门见 §162 |
| `xxx_op` 是动态链接 | **仍成立**，而且**生成的实例也走这条路**（§166） |
| 算子之间只走 schema 的序列化数据 | **仍成立**：跨边界仍是 `PayloadBundle`；**载荷仍逐字节相同**（§163.1 / §164.1） |
| 需要单态化的算子由图自建的 `xxx_op` 并着用 | ✅ **样本有了**；但**形状换过**：§166 那版生成器已删，今天是 `px_local_op!`（`18` §173）与 `px_inst!`（`19`/`20`） |
| 键 = 内容 | **加强**：算子的源码哈希现在也进键（§166.2，⚠ 老键全失效一次） |

⇒ 一句话：**这一级把「生成的单态化实例」变成了一个可装载的 dylib ——
改一行场函数 1.56 s、图程序 sha 不变、键必变。最难的地方不是单态化，是构建卫生（§166.3）。**
