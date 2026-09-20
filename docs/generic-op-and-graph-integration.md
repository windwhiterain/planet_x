# 写一个泛型算子库，再接进 graph scripts

> **操作清单**：从一个空目录到图脚本跑出产物，一共要写哪些文件、敲哪些命令、每处约束是为什么。
> 所有片段与仓里代码**逐字对齐** —— 范本是**场域**（`px_field_schema` / `px_field_op`，七个算子已落地）。
> 机制与读数记在 `.agents/notes/art/18-operator-libraries.md`（为什么运行期装载、握手、
> 身份两半、搜索路径）；这一篇只讲**怎么写**。

---

## 0. 图脚本里只看到这些（先把结论摆出来）

```rust
use px_cook::{Domain, GraphSpec, begin, cameras, cook, field, mesh, volume};
```

`begin(GraphSpec { … })` 交回一个 `Graph` 句柄；`cook::<算子>(&graph, "节点名", 输入)` 走一趟
「算键 → 查 → 命中就解载荷；不命中就调实现、编码、落盘」；最后 `graph.finish()` 写参数索引与清单。
`cook` 的真实签名是
`cook<O: PxOp>(cache: &dyn Cache, node: &str, inputs: O::Inputs) -> Result<Cooked<O::Payload>, String>`
（`Graph` 就是那个 `Cache`）：

```rust
use px_cook::cook;

// 不吃上游：输入形状是 `()`（无上游那一档住在契约里）
let clusters  = cook::<field::Fbm>(&graph, "clusters", ())?;
let mountains = cook::<field::Ridged>(&graph, "mountains", ())?;
let weight    = cook::<field::Constant>(&graph, "weight", ())?;

// 两个上游、三个上游：输入形状由算子自己定义（具名字段）
let carved = cook::<field::Warp>(&graph, "carved",
                 field::FieldPairInput { field: clusters, offset: mountains })?;
let mixed  = cook::<field::Mix>(&graph, "mixed",
                 field::MixInput { a: clusters, b: carved, mask: weight })?;
let height = cook::<field::Remap>(&graph, "height",
                 field::FieldInput { field: mixed })?;

// 体积与网格
let coarse = cook::<volume::CloudCoarse>(&graph, "coarse",
                 volume::CloudCoarseInput { coverage: height.clone() })?;
let proxy  = cook::<mesh::Proxy>(&graph, "proxy",
                 mesh::ProxyInput { volume: coarse.clone() })?;
```

**就这些**。没有 `encode`/`decode`、没有 `cook_field`/`cook_volume`/`cook_mesh`、
没有 `&[&a, &b]`、没有 `OpLibrary`/`OpTable`、没有描述符表、没有运行期按字符串 id 分派。
域、相机口径、编解码、身份全从算子类型推。

**字符串 id 还是有一个**（`px_op!` 那一行的 `"field.fbm"`），但它只做两件事：进键、给人读的读数。
它**不**用来找函数 —— 「去哪个库、取哪个符号」是编译期常量
（`PxOp::LIB` / `PxOp::SYMBOL = concat!(库名, "__", 类型名)`）。

三件事因此是**编译期**的：

| | 靠什么 | 报错长什么样 |
|---|---|---|
| 输入个数/形状 | `O::Inputs`（关联类型钉死） | 少给一个上游、给错域 ⇒ `expected MixInput, found …` —— 见 §3.3b |
| 输出域 | `O::Payload` | 把体积喂给要 `Cooked<Field>` 的字段就编不过 |
| 参数类型 | `O::Params` | 参数文件字段写错当场报（`deny_unknown_fields`） |

---

## 1. 四层各归其位

```text
px_protocol        线格式（Frame / Blob / ArtBundle）
     ↓
px_graph_schema    ★契约：Key / PayloadBundle / Grid / 算子身份（OpId）+ PxOp 契约 + **装载**（ops.rs）
     ↓
px_*_schema        各域的数据、参数、**以及算子声明**（src/ops.rs 里那几行 px_op!）
     ↓
px_cook            图脚本唯一那扇门：cook + 图的生命周期 + 各域算子表（全是 re-export）
     ↓
px_graph           驱动（CAS / 参数 / 清单 / cameras / generate / shader）
     ↓
px_graphs          图脚本；实现库（px_*_op）**不在它的依赖里**

px_*_op            实现：crate-type = ["dylib"]，运行期按身份装载
```

**一句话判据**：**声明**住 schema（`px_*_schema/src/ops.rs`），**实现**住实现库（`px_*_op`，
`crate-type = ["dylib"]`），图程序**不 cargo 依赖**实现库 —— 它按身份在运行期装载。
参数类型、输入个数、输出域仍然全在**编译期**判（这是声明那半带来的，与实现住哪无关）。

四条不变量（`px_graphs/tests/crate_graph.rs` 与 `px_graph/tests/source_hash.rs` 看着）：

1. 图程序**不 cargo 依赖**实现库 —— 这条就是「改一行实现不重编图程序」。
2. 实现库**不依赖** `px_graph` / `px_cook`（否则 dylib 里带一份驱动：一个进程两份驱动）。
3. `px_graph` 不依赖任何算子（它只认 `Cache`）。
4. schema 层不依赖算子（数据层是算子与驱动共用的）。

⚠ **为什么会咬人**：这四条写在 `Cargo.toml` 里就生效，**任何一层都不会报错** —— 所以用门看住。

---

## 2. 全部要写的文件

### 2.1 算子库作者（以「场域加一个 `field.scale`」为例，六件）

| # | 文件 | 写什么 |
|---|---|---|
| 1 | `px_field_schema/src/params.rs` | 超参数 struct：`#[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]` + `#[serde(default, deny_unknown_fields)]` |
| 2 | `px_field_schema/src/ops.rs` | **声明**：`px_op! { … }` 一行；输入 struct（`#[derive(px_derive::PxInputs)]`）也住这儿 |
| 3 | `px_field_schema/src/lib.rs` | 已经有一行 `pub mod ops;` —— **新算子不用动它**（忘了这一行是「声明不见了」的头号原因） |
| 4 | `px_field_op/src/ops/<算子>.rs` | **实现那一行** `px_body!` + 算法体（普通 Rust 函数） |
| 5 | `px_field_op/src/ops/mod.rs` | 加一行 `pub mod <算子>;` |
| 6 | `px_field_op/Cargo.toml` / `build.rs` / `src/lib.rs` | **只在开新库时要**；往已有库加算子**一个字都不用动**（`px_impl_lib!()` 一个库写一次） |

⚠ 第 3 条值得单说：`px_field_schema/src/lib.rs` 里那行 `pub mod ops;` 是**一次性的**，
但它是**新域**最容易漏的一行 —— 漏了以后 `field::Scale` 根本不存在，而报错只会在图脚本上说
「找不到 `field::Scale`」，不指向真正的原因。

⚠ 第 6 条里已经存在的库**一个字都不用动**（`px_impl_lib!()` 一个库写一次，
`Cargo.toml` 的 `crate-type = ["dylib"]` 也不动）—— 只有**开新库**（新域）时才有那三件。

范本是**逐字**的（七个场算子就是这么写的）：

* 声明 → `px_field_schema/src/ops.rs:37-70`
* 实现那一行 → `px_field_op/src/ops/fbm.rs:9`（`Fbm`）、`mix.rs:6-9`（`Mix`）、`warp.rs:6-9`（`Warp`）
* 算法体 → 同一个文件里紧接着的那个 `pub fn eval(...)`

### 2.2 图作者

| # | 文件 | 写什么 |
|---|---|---|
| 7 | `px_graphs/Cargo.toml` | **不用动**：图程序依赖的是 `px_cook`（唯一那扇门）。⚠ 这里**不许**出现任何 `px_*_op` |
| 8 | `px_graphs/src/bin/<图>.rs` | `cook::<算子>(…)` 调用链 |
| 9 | `art/<图>/<节点名>.toml` | 这个节点的**超参数** |

### 2.3 一个算子库的 `src/lib.rs` 全文（`px_field_op/src/lib.rs`）

```rust
//! **场域**的算子**实现**：七个算子，编成一份 dylib。

pub mod noise;
pub mod ops;

/// 这个库的身份：`…__source_hash`（进键的"实现是哪一份"）与 `…__contract_hash`
/// （装载时与图程序对账"我们是不是同一份契约编出来的"）。
px_graph_schema::px_impl_lib!();
```

**就这些。** ⚠ 从前这里还有 `pub mod typed;` + `pub use typed::{…}`（声明搬去 schema 了，
`typed.rs` 已删）；三行 dylib 管道（`px_canonical_params!` / `px_dylib_call!` / `px_op_table!`）
也随描述符表那一层一起删了。

---

## 3. 逐件写清

### 3.1 超参数：`px_<域>_schema/src/params.rs`

住 schema 这一侧，因为**两处都要它**：算子声明（`ops.rs`）与判据仪器 —— 谁也不许自己再抄一份字段。

```rust
pub mod fbm {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
    #[serde(default, deny_unknown_fields)]
    pub struct Params {
        pub frequency: f32,
        pub octaves: u32,
        pub lacunarity: f32,
        pub gain: f32,
        pub seed: u32,
        pub aspect: f32,
        pub spherical: bool,
    }

    impl Default for Params {
        fn default() -> Self {
            Self { frequency: 4.0, octaves: 6, lacunarity: 2.0, gain: 0.5, seed: 7, aspect: 2.0, spherical: true }
        }
    }
}
```

- `PxParams` **必须**：它生成 `PxKeyed`（每个字段名 + 字段值按自己的类型写进键）。
  手写的话，「加了字段却忘了进 `key`」是个**静默** bug —— 改了参数却命中旧产物。
- `deny_unknown_fields` **要开**：参数文件写错字段名时，这是唯一在「读参数」那刻就报的机制。
- 纯局部开关（不进键的那种）加 `#[nohash]`。
- 新增字段类型时要在 `px_graph_schema::HashField` 上加一条实现（闭集，编译器会当场报）。

### 3.2 算法体：`px_<域>_op/src/ops/<算子>.rs`

普通 Rust 函数，**签名怎么写都行** —— 它只被本文件顶上那一声 `px_body!` 调用。
范本（`px_field_op/src/ops/fbm.rs` 全文）：

```rust
use px_field_schema::field::{Field, GridField};
use px_field_schema::noise::FbmSettings;
use px_field_schema::ops::Fbm;
use px_field_schema::params;
use px_graph_schema::Grid;

use crate::noise;

px_graph_schema::px_body! { Fbm, |p, _i, g| crate::ops::fbm::eval(p, &[], g) }

pub fn eval(params: &params::fbm::Params, _inputs: &[&Field], grid: Grid) -> Field {
    // …逐纹素算…
}
```

⚠ 算法体**不认识**驱动、不认识 CAS、不认识 `art/`：进来的是**类型化的值**（参数 + 上游），
出去的是同一个形状 —— 因为这一份与图侧编译的是同一份契约，装载时握过手（§3.3c）。

### 3.3 声明：`px_<域>_schema/src/ops.rs`

```rust
use px_graph_schema::{Cooked, px_op};

use crate::field::Field;
use crate::params;

// 不吃上游：输入那一栏是 `()`（它没有名字问题，impl 住在契约里）。
px_op! {
    /// 分形布朗噪声。
    Fbm, "field.fbm", "px_field_op", params::fbm::Params, (), Field
}

// 吃三张场：形状是下面那个 `MixInput`（§3.3b）。
px_op! {
    /// 按权重混两张场。
    Mix, "field.mix", "px_field_op", params::mix::Params, MixInput, Field
}
```

参数顺序：`类型名, "op.id", "px_库名", 超参数类型, 输入类型, 输出类型`。
宏展开出的就是：`pub struct 类型名;` + `impl PxOp for 类型名`（`ID` / `LIB` /
`SYMBOL = concat!(LIB, "__", 类型名)` / 三个关联类型 / `new()`）。**声明里一行实现都没有。**

| 那一栏 | 是什么 |
|---|---|
| `"op.id"` | 人读的那一半（`field.fbm`）。**它也进键** —— 但**不**用来找函数 |
| `"px_库名"` | 实现住在哪个库（`px_field_op`）。装载、报错、陈旧检查都用它；符号名由它 + 类型名拼出来 |
| `超参数类型` | 从 `art/<图>/<节点名>.toml` 解出来的那个 struct |
| `输入类型` | 这个算子**自己定义**的输入 struct（`()` = 不吃上游）—— 见 §3.3b |
| `输出类型` | `Field` / `VolumeData` / `MeshData`。⚠ **域就是这个类型** —— 相机掺不掺、画布算不算分辨率都由 `Payload` 自己声明（`WITH_CAMERAS` / `RESOLUTION_IS_CANVAS`），没有能对不上的第二处 |

⚠ **没有「版本」这一栏，也没有「源码清单」那一栏** —— 身份两半都是自动的，见 §3.3c。

### 3.3b 图参数的形状：**算子自己定义**，`PxInputs` 生成

形状是这个算子**接口的一部分**，所以住 schema 的 `ops.rs`（声明旁边），字段名就是它吃的东西的名字：

```rust
/// 三张场（两张待混 + 一张权重）。**字段名有语义。**
#[derive(px_derive::PxInputs)]
pub struct MixInput {
    pub a: Cooked<Field>,
    pub b: Cooked<Field>,
    pub mask: Cooked<Field>,
}
```

`#[derive(PxInputs)]` 只生成**一条** `collect`：把字段名与每个 `Cooked::key` 折进键。
与超参数那边「键用宏、解 TOML 用 serde」是同一套口径：

| | 超参数（`#[derive(PxParams)]`） | 图参数（`#[derive(PxInputs)]`） |
|---|---|---|
| 键 | 宏：字段名 + `HashField`（按类型写字节） | 宏：字段名 + `Cooked::key`（上游的键） |
| 值 | serde：`toml` → 结构 → 规范 JSON（进键） | **普通 Rust 值**：图脚本把 `Cooked<T>` 直接交给 `cook` |

**为什么图参数没有编解码**：图脚本是把**值**（`Cooked<T>` 里那个内存中的值）交过来的，
从来没有「按位置解上游字节」那条路 ⇒ 字段**顺序**不是接口的一部分，**字段名**才是。
（从前那条 `Cooked::from_bytes` 是把上游字节按字段顺序解回来的，已经随「载荷当字节边界」删掉；
今天跨算子边界走的仍然是 `PayloadBundle` —— 它是**存进 CAS 的那一份**，不是调用约定。）

⚠ **接错就是编译错**：`O::Inputs` 是具体类型，少一个字段、给错域都编不过。

**为什么不是 `px_cook` 给一组通用的 `Unary1/2/3`**（那一版我做过，删了）：字段名会变成
`a`/`b`/`c` —— **没有语义**，而且形状就「谁都不属于」了。

### 3.3c 身份：两半都自动（你什么都不用记）

```text
接口形状哈希 = interface_hash([type_name::<Params>(), type_name::<Inputs>(), type_name::<Payload>()])
源码指纹     = build.rs 遍历「这个实现库编译进去的全部源码」：
               自己 src/ 的 .rs（递归）+ 自己的 build.rs + Cargo.toml 里**所有 path 依赖**的 src/
```

| 你改了什么 | 源码指纹 | 接口哈希 | 结果 |
|---|---|---|---|
| 算法体一行 | 变 | 不变 | 重算（键变） |
| 参数 struct 加/删/改字段 | 变 | **变** | 重算 |
| 输入 struct 改 | 变 | **变** | 重算 |
| 输出域改 | 变 | **变** | 重算 |
| 只改了图脚本 | 不变 | 不变 | 全命中 |

- **接口哈希**管「形状变了」（取代手写的 `version`）：从三个类型名推，`cook` 里算一次，
  同时喂给键、读数、清单三处 —— 进键的是完整 64 位，清单里那一格 `ManifestEntry::op_version`
  只是它的低 32 位（给人对账用）。
  ⚠ **不缓存**：泛型函数里的 `static` 不按单态化分开（实测过），缓存反而制造 bug。
- **源码指纹**管「形状没变而算法变了」：⚠ 它是**运行期**从实现库的身份符号
  （`…__source_hash`）读出来的 —— 图程序不重编也能看见它换了，所以「改了实现却命中旧产物」
  不可能发生。`px_graph_schema::ops::source_hash(lib)` 就干这件事。
- 两半都由共享的指纹算法与 `PxOp::interface()` 自动给，**不用列清单、不用升版本**。
  那个算法住 `px_fingerprint`（**rlib**）：三个实现库 + 契约层的 `build.rs` 与**运行期**的
  `px_cook::inst` 共用同一份（`build.rs` 走 `[build-dependencies]`，运行期走 `[dependencies]`）。

⚠ 指纹覆盖的是**这个实现库**编译进去的全部源码（含 path 依赖）：改 `px_verify`、改场域的
`field.rs` 都会让场库的键变。⚠ 反过来，**图脚本与别的实现库的源码不在里面**。

⚠ 接口哈希参的是 `core::any::type_name`，**不保证跨编译器稳定**。我们的键只要求
「同一台机器上前后一致」，所以够用。

### 3.3d 装载：一次 `GetProcAddress`，加一道契约握手

实现库里那一行（每个算法文件顶上）：

```rust
px_graph_schema::px_body! {
    Mix,
    |p, i, g| crate::ops::mix::eval(p, &[i.a.value(), i.b.value(), i.mask.value()], g)
}
```

`px_body!` 用 `export_name = concat!(env!("CARGO_PKG_NAME"), "__", stringify!(名字))` 导出一个符号，
签名由 `PxOp::Body<O>` 钉死：

```text
fn(&O::Params, &O::Inputs, Grid) -> Result<O::Payload, String>
```

⚠ `$body` 外面套了一层 `Ok(…)`：它是一个**值**。内部函数回 `Result` 时（等值面那条路），
就在块里用 `?` —— 那个 `?` 从 `px_body!` 生成的这个函数往外传：

```rust
px_graph_schema::px_body! {
    Proxy,
    |p, i, _g| {
        let grid = px_volume_schema::VolumeGrid::new(i.volume.value());
        crate::proxy::surface(p, &grid)?
    }
}
```

装载是怎么发生的（`px_graph_schema/src/ops.rs`）：

1. **懒**：第一次 `render` 才去取函数指针（一个算子一次，进程内一份）。
2. **搜索路径**（缺了**当场拒**，报错里带该跑的命令）：
   `PX_OP_DIR` > exe 同目录 > exe 的上一级（测试 exe 在 `deps/` 里）> 当前目录。
3. **契约握手**：DLL 导出 `…__contract_hash`（它编的时候 `px_graph_schema` 是哪一份），
   与图程序手里的 `px_graph_schema::SOURCE_HASH` 比；对不上就拒 ——
   「改了契约而 DLL 没重编」时，宁可拒也不要拿错的类型布局去调。
4. **陈旧只告警**：库文件比它的源码旧 ⇒ 打一句「这一趟跑的是**旧实现**」（要看见的那行是
   `⚠ <库名> 的实现比库新`）。
   这**不会**让产物认错（跑的是哪一份实现，源码指纹就带哪一份进键），但「你以为在跑新的」必须说出来。

⚠ **进程内不重载**：`library()` 把已装载的库漏成 `'static` 存进一张表，一个进程只装一次。
所以「重编了实现库、同一个跑着的进程接着跑」还是旧实现 —— 重跑那个 exe 才会换。

⚠ 全仓**唯一**一处 `unsafe transmute` 就在那里：把符号地址当成「签名由算子钉死的函数」。
它不是类型擦除 —— `Body<O>` 的签名是编译期写死的，运行期只解析「这个地址在不在」。

### 3.4 输出域与相机

域（`Payload` 类型）自己声明两件事（`impl px_graph_schema::Build for …`）：

| | `WITH_CAMERAS` | `RESOLUTION_IS_CANVAS` |
|---|---|---|
| 场（`Field`） | `true` | `true`（场的分辨率**就是**画布） |
| 网格（`MeshData`） | `true` | `false`（尺寸由参数给） |
| 体积（`VolumeData`） | `false` | `false`（相机是「怎么看」，体积没人直接看） |

**域决定键里掺不掺评审相机**：`cook` 读 `<O::Payload as Build>::WITH_CAMERAS` 决定要不要
`key_with_cameras`；画布同理走 `RESOLUTION_IS_CANVAS`。算子不用管 —— 而且**没有第二个地方要写它**：
域就是 `Payload` 类型。

### 3.5 图脚本：`px_graphs/src/bin/<图>.rs`

```rust
use px_cook::{Domain, GraphSpec, artifact_path_of, begin, cameras, cook, field, mesh};

let graph = begin(GraphSpec {
    name: "planet".to_string(),
    width: 780,
    height: 520,
    projection: Domain::Cube,
    cameras: cameras::review(),
});

let continents = cook::<field::Fbm>(&graph, "continents", ())?;
let mountains  = cook::<field::Ridged>(&graph, "mountains", ())?;
let weight     = cook::<field::Constant>(&graph, "weight", ())?;
let terrain    = cook::<field::Mix>(&graph, "terrain",
                     field::MixInput { a: continents, b: mountains, mask: weight })?;
let height     = cook::<field::Remap>(&graph, "height",
                     field::FieldInput { field: terrain.clone() })?;
let surface    = cook::<mesh::CubeSphere>(&graph, "surface",
                     mesh::CubeSphereInput { height: height.clone() })?;

let stats = height.value().stats();
println!(
    "输出 height：{}×{}，值域 {:.4}..{:.4}，均值 {:.4}",
    height.value().width, height.value().height, stats.min, stats.max, stats.mean,
);
println!("输出 surface：{} 顶点 / {} 三角形", surface.value().vertices(), surface.value().triangles());

graph.finish();
```

- **上游是值，不是引用**：`Cooked<T>` 里是值 ⇒ 同一份被多处用就 `.clone()`。
- `cook` 的第三个参数收的是 `O::Inputs`，**类型参数不用写全** —— 关联类型会反推。
- 读结果走 `.value()`（`Cooked::value()`）；键与读数在 `.key` / `.hit` / `.millis` / `.bytes`。
- 参数文件按**节点名**取（`art/<图>/terrain.toml`）。同一个算子在别的图里叫别的名字 ——
  算子不该知道节点名。
- `begin` 交回**句柄**（不是一个全局单例）：同一个进程里可以同时跑两张图，彼此不串。

### 3.6 参数文件 `art/<图>/<节点名>.toml`

> 跑完看一眼 `target/pcg/<图>/params.json`：它是**每个节点实际生效的参数值 + 字段名**。
> `graph.finish()` 还会打一行
> `参数索引：N 个节点（M 个走默认值：<名字 / 名字>）；字段名与生效值见 <params.json>（⚠ 缺参数文件的都在默认值上）`
> —— 照着那份 JSON 补文件即可。

```toml
frequency = 1.7
octaves = 6
```

超参数**留在这里**（不在 Rust 里）：改调参只需要重跑图，不重编。

---

## 3.7 键的语义：**一个节点 = 产出它的那些东西**

```text
节点键 = op_id ‖ 接口形状哈希 ‖ 实现的源码指纹 ‖ 规范参数 ‖ 上游的键
                 [+ 画布（那一档才掺）] [+ 相机（那一档才掺）]
```

**不在键里的**：

| 不在 | 为什么它不该在 |
|---|---|
| `graph_version` | 它是**图的属性**。改图脚本里别处一行代码，不该让这个节点的产物作废 |
| 画布尺寸（体积/网格） | 分辨率由**参数**给，与画布无关。一刀切会让「改画布」连带重烘它们 |
| 投影 | 它只影响编解码的字节布局，那件事该由 `Payload` 的 `encode`/`decode` 承担 |

⇒ 同样的算子、同样的参数、同样的上游（+ 该域的画布）⇒ **同一个产物**，不管图脚本长什么样。

⚠ **节点名不进键**：键由算子身份 + 参数 + 上游算，节点名只在清单与产物里当名字。
两个节点名不同、其余全同 ⇒ 同一个键（CAS 里就是同一份字节）。

---

## 4. 命令

```bash
cargo build -p px_<域>_op                  # 编实现库（dylib）
cargo run   -q -p px_graphs --bin <图>      # 跑图（实现库得先在盘上，否则当场拒）
cargo test  -p px_graphs                   # 两道门（见 §6 的 1、2）
cargo run   -q -p px_graphs --bin scene    # 出场景文档（消费上面几张图的清单）
```

⚠ `cargo test -p px_graphs` **需要先编实现库**（`-p` 不会带上它们）：
`cargo build` / `cargo test`（默认 members）会编；`tools/px.ps1 -Task planet` 跑图前也显式编了那三个包。
缺库时的报错带命令，不静默。

---

## 5. 泛型算子的实例落在哪

**结论**：泛型参数在**图脚本侧**被实例化成一个**已经采样好的场**（`px_field_schema::field::Field`），
交给实现库那个算子。泛型的单态化发生在**实现库内部**，图侧看不见。

范本是体积域（`px_volume_op`）：`bake<F: FieldFn>` 是泛型方法，唯一那个实例
`SampleField`（把一张采样好的场当函数用）就编在**这份 dylib 里**；图脚本交给算子的是一个 `&Field`：

```rust
// px_volume_op/src/lib.rs —— 那一行 px_body! 就是这条线
px_graph_schema::px_body! {
    CloudCoarse,
    |p, i, _g| crate::eval_sampled(p, i.coverage.value())
}

/// `eval_sampled` 就是 `bake<SampleField>`：上游那张**采样好的场**当覆盖度函数。
pub fn eval_sampled(params: &params::Params, coverage: &Field) -> VolumeData {
    bake(params, &SampleField { field: coverage })
}
```

图脚本那一侧只有：

```rust
let coarse = cook::<volume::CloudCoarse>(&graph, "coarse",
                 volume::CloudCoarseInput { coverage: mixed.clone() })?;
```

⇒ **实现库不参与图侧的单态化，图侧也不需要链接它。** 跨 dylib 边界流动的只有
「参数 + 上游载荷（`PayloadBundle`）+ 画布」这三样（见 §1 的第 2 条不变量）。

### 5.1 哪些东西变了要重编、哪些不要

| 你改了什么 | 实现库（`px_*_op`） | 图程序（`px_graphs`） | 产物 |
|---|---|---|---|
| 算法体一行（`ops/<算子>.rs`） | **重编那一份库** | **不重编**（exe 字节不变） | 重算（源码指纹变了 ⇒ 键变） |
| 参数 struct / 输入 struct / 输出域 | **重编** | **重编**（声明住 schema，图程序链的正是它） | 重算 |
| 给某个算子换一个实现库（改 `px_op!` 的库名） | 两份都编 | **重编** | 重算（符号名与身份都变了） |
| 图脚本一行（拓扑、节点名） | 不重编 | 重编 | 只影响这张图 |
| `art/<图>/<节点>.toml` 一个值 | 不重编 | 不重编 | 只有该节点与它的下游重算 |

⚠ 这张表里最值钱的一行是**第一行**：改实现不重编图程序
（实测：静态链 1.91 s、直接依赖 dylib 3.41 s 且 exe 被重链，运行期装载 **0.44 s 且图 exe 字节不变**）。

⚠ 代价也在这张表里：**改声明就得重编图程序**（参数类型/输入形状/输出域是编译期的事，
那是「接错就编不过」换来的）。

### 5.2 另一半：泛型的**代码**也住在图侧（`px_local_op!`）

上面那一档是"泛型参数已经落成一张场、交给实现库"。如果泛型参数是**一段现写的代码**
（一个闭包、一个场函数），那就不是"交给实现库"，而是**整个算子都住在图程序里**：

```rust
// px_graphs/tests/local_op.rs（真样本，可直接抄）
fn bake<F: Fn([f32; 3]) -> f32>(grid: Grid, sample: F) -> Field { … }   // 泛型：单态化落在图侧

struct Band;
px_local_op! { Band, "local.band", BandParams, (), Field,
    |p, _i, g| bake(g, |d| ((d[2] * p.frequency).sin() * 0.5 + 0.5) * p.gain) }
```

| | 实现住哪 | 身份（进键的那一半） | 改一行谁重编 |
|---|---|---|---|
| `px_op!` | `px_*_op` 的 dylib | **那份库**的源码指纹（运行期从库里读） | 只重编库，图 exe 字节不变 |
| `px_local_op!` | **本图程序** | **本 crate** 的源码指纹（`env!("PX_SOURCE_HASH")`，本 crate 的 `build.rs` 给） | 图程序重编，实现库一位不动 |

* 超参数照样是图侧自己定义的 struct（`#[derive(px_derive::PxParams)]` ⇒ 加字段自动进键）；
* `cook` 对它一视同仁：算键 → 查 → 命中就解载荷、不命中就调 `render`；
* ⚠ 身份是"**整个图程序这一份源码**"（粗是**故意的**）：改任何一个 bin 都会让所有图侧算子换键。
  要跨图复用、要更细的粒度 ⇒ 写成 `px_*_op` 里的正式算子（那时连图程序都不用重编）。
* ⚠ 唯一的前提：这个 crate 要有 `build.rs`（一行 `px_fingerprint::cargo_fingerprint_for_crate(&[])`，
  共享的指纹算法住 `px_fingerprint` 那个 rlib），否则 `env!("PX_SOURCE_HASH")` 会在编译期报
  "变量没定义" —— 这是**故意的**（宁可编不过，也不要一个来路不明的身份）。

### 5.3 第三种：复用一个**声明**，泛型参数单独编成**实例库**（数据收据 + 代码生成）

`px_local_op!` 的代价写在它自己那一行里：**泛型参数跟着图程序重编**。如果那段泛型参数
要**跨图复用**、而且希望「改它不必重编图程序」，就用**实例**那一档：一个实例 =
「复用一个算子的**声明**（`Params` / `Inputs` / `Payload` 全从它取）+ 一段现写的**泛型参数**」，
编译成一份**内容寻址**的 DLL，运行期按 key 装载。

⚠ **图侧今天一个宏都没有**：从前那一行 `px_inst!` 已经删了（见本节的过期标记）。
现在图侧是一张**数据收据**（`recipe`），**类型由 stage 1 生成**出来：

```rust
// px_graphs/src/inst_recipe.rs —— 只有**类型定义 + 字面量**（build.rs 会直接读它）
pub struct InstRecipe {
    pub op_id: &'static str,              // "cloud.coarse/band"
    pub decl: &'static str,               // "CloudCoarse"（声明名，由声明表解析成类型）
    pub type_name: &'static str,          // "Band"（图侧类型名；生成器照它写 `pub struct Band;`）
    pub roots: &'static [&'static str],   // ["px_volume_alg"] —— 编译时链的 crate（build graph 的**边**）
    pub source: &'static str,             // "art/inst/band.rs"（泛型参数住哪）
    pub body: &'static str,               // 体表达式**原文**（泛型参数那一位写占位符 `ARG`）
}
pub const INSTANCES: &[InstRecipe] = &[ /* cloud.coarse/band + field.remap/waves */ ];
```

```rust
// px_graphs/src/insts.rs —— 图侧把 build script 生成出来的**类型** include! 进来
#[path = "inst_recipe.rs"]
pub mod recipe;

pub mod generated { include!(concat!(env!("OUT_DIR"), "/insts_gen.rs")); }   // ← stage 1 的产物
pub use generated::{Band, Waves};

pub fn build(g: &mut px_cook::inst::BuildGraph) {   // stage 1 的"要编哪些"
    for item in recipe::INSTANCES {
        let (interface, decl_hash) = generated::facts_of(item.type_name);
        g.facts(item.type_name, item.op_id, interface, decl_hash,
                item.roots, item.source, item.body);
    }
}
```

⇒ **加一条实例 = 在 `inst_recipe.rs` 那张表里加一栏**（`build()` 不改、宏不存在），
而"**stage 2 用的类型**"（`px_graphs::insts::Band`）就是 build.rs 写出来的那份生成物。

> ⚠ 已废弃（2026-09-20）：**`px_inst!` 那个宏已删除**（图侧早就零宏；宏本体也作为死代码清掉了
> —— `px_cook/src/lib.rs` 里只剩一行"已删除"的注记），"图侧写一行宏 + `build()` 里一行
> `g.inst::<T>()`"的写法**不要再照抄**。今天见 `.agents/notes/art/21-codegen-types.md`
> （recipe + 声明表 + 生成器），以及 `19-generic-inst.md` 顶部那条过期标记。
> 下面这段旧示例只作历史：
>
> ```rust
> // ⚠ 旧写法（已废弃）—— 图侧一行宏 + 一行登记
> px_cook::px_inst! {
>     Band, "cloud.coarse/band", volume::CloudCoarse, ["px_volume_alg"], "art/inst/band.rs",
>     |p, i, g, ARG| px_volume_alg::coarse_with(p, i.coverage.value(), ARG)
> }
> pub fn build(g: &mut px_cook::inst::BuildGraph) { g.inst::<Band>(); }
> ```

* ⚠ **`roots` 是"根列表"，不是单个 crate**：`["px_volume_alg"]` = **这个实例编译时链的
  crate**（目录名）。列进来的每一个都会 ① 被写进生成物的 `[dependencies]`、② 它的源码名册**进 key**。
  想再用一个 crate ⇒ 往列表里加一个（`["px_volume_alg", "px_noise"]`），**不是**改什么宏的形状。
  这就是 build graph 里那条**边**（`20` §183）；⚠ 少列一个 = 改它**不换 key**（§177 那族洞）。
* ⚠ **声明那一栏写"声明名"、`type_name` 写"图侧类型名"，两处都不许带路径**：符号名由
  `px_cook::inst::symbol(decl) = "px_inst__<声明名>"` 拼出来（装载时拿它去库的导出符号里找），
  而 `type_name` 要能被 `include!` 进来的源码解到 —— 写多段路径两处都会坏（当年那一版
  `stringify!` 还会拼出带空格的符号名，实测踩过的真缺陷）。
* ⚠ **`body` 那一栏里的泛型参数那一位仍然写占位符 `ARG`**：它是 **key 的一轴**
  （实测把它换成真名 `&Band` ⇒ key `96d4feb75ec4` → `8fcd505c8000`，铁律 3 不许改）。
  抄进实例库的那一份由 `InstRecipe::generated_body()` 做**整词替换**（`ARG` → `&<type_name>`），
  与 `px_cook::inst_scan` 当年那一份逐字相同。

```rust
// art/inst/band.rs —— agent 写的**只有泛型参数**（会被原样 include! 进实例库）
pub struct Band;
impl px_volume_alg::field_fn::FieldFn for Band {
    fn cover(&self, _cloud: &px_volume_alg::field_fn::CoverCloud, direction: [f32; 3]) -> f32 { … }
}
```

```bash
# ⚠ 只有一个 driver `px`（`px_jit` / `px_run` 本轮并成它，见 `20-build-graph.md` §191）。
# 两个 stage **顺序执行**的那一条命令（`20` §182）：
px list                       # 只计划：列出每条实例的 key 与「有|缺」（不编、不跑）
px build                      # stage 1：编缺的那些（**不吃图名**；`px build --gc [--deep] [--target]` 回收代码缓存）
px run planet                 # 两阶段：stage 1 计划全命中才进 stage 2（**只读**）
px run planet --build         # stage 1 有缺就编那几条，再跑 stage 2
# 或者（包装层，task 与 driver **同名同义**）：
#   .\tools\px.ps1 -Task list
#   .\tools\px.ps1 -Task build              # 加 -Deep 连深层回收一起
#   .\tools\px.ps1 -Task gc                 # = px build --gc
#   .\tools\px.ps1 -Task run -Graph planet [--build] [-- 图自己的参数…]

# `px` 是**薄 driver**：执行住在 `px_cook::inst::BuildGraph`
# （`missing()` / `compile_one()` / `compile_missing()`，`20` §191）。
```

⚠ **`px run` 不嵌套 cargo**：stage 2 直接跑 `target/<profile>/<图>.exe`。
⚠ 有缺且**没给** `--build` ⇒ **非零退出 + 打印该跑的确切命令**（"运行只读"是默认值：
`20` §186 不许 stage 2 偷偷触发编译）。⚠ 从前那个 `PX_JIT=build` 环境变量兜底**已废弃、已从代码里删除**
（今天就是 `px run <图> --build`，或先 `px build`）。
⚠ 读 key 直接跑 `target/debug/px.exe`，**别经过 `cargo run`** —— 特性合并不同会让它无谓重链（`19` §180 的量法坑）。

* 产物收在 `target/pcg/inst/<key>.dll`（+ `<key>.json` 那份 sidecar）：**机器本地、可删、
  不进 git** —— 与数据 CAS 同性质；`cargo build` 的中间物在 `target/jit/`。
  `px build --gc [--deep] [--target]` 按"build graph 可达的活键集"清非活的（口径与数据 CAS 统一，`20` §185）。
* key 里有什么：工具链指纹 + 契约源码指纹 + **声明所在 crate** 的指纹 + **根列表里每个 crate**
  的源码名册 + 接口形状 + op id + 泛型参数源文件的**内容** + **体模板（去空白）**。少一样就是陈旧复用（§177）。
  ⚠ 体模板也进 key（`normalize_template`）：模板改一个字就该换键，而"图里登记的那份 == 源码里那份"
  由 `inst_gate` 的一道门逐字符看着（`20` §191）。
* ⚠ 图程序**不 cargo 依赖根列表里那些 crate**：它们的源码名册是**从盘上**数的（`px_fingerprint`），
  于是"改 alg 一行 ⇒ 键变"而图 exe 一位不动。
* ⚠ **泛型参数改了要重跑 stage 1**（`px run <图> --build`，或先 `px build`）：键变了、
  旧库只是**孤儿**（不是"命中错的"）。缺件且只读运行时的报错带那条命令，不静默。
* ⚠ 三档怎么选：泛型参数是**一个已经采样好的值** ⇒ 用 `px_op!`（§5）；是**一段只在这张图里
  用的代码** ⇒ 用 `px_local_op!`（§5.2）；是**一段要跨图复用、且不想跟着图程序重编的代码**
  ⇒ 用**实例那一档**（图侧一条 recipe，见本节 + 下面 §5.4 那条**能跑的最小例子**）。

### 5.4 场域的泛型实例 —— 与体积域逐处同构（**能跑的最小例子**）

体积域（`cloud.coarse/band` 那条 `Band`）与场域（`field.remap/waves` 的 `Waves`、
`field.remap/latbands` 的 `LatBands`）是同一套机制的几次落地，它们在 `px_graphs/src/inst_recipe.rs`
那张表里并排住着 —— 要抄就照这几条抄（2026-09-20 之前只有两条；`latbands` 是「同一张声明、
另一种场函数」那一步，它顺手把 `band` 的**球面**档验了一遍）。
图侧如今**一行数据**（不是宏、也不是手写类型）：

```rust
// px_graphs/src/inst_recipe.rs —— 场域那一条就是这样一个 InstRecipe 字面量
InstRecipe {
    op_id: "field.remap/waves",
    decl: "FieldRemap",                                  // ← 由 px_decls 声明表解析成类型级事实
    type_name: "Waves",                                  // ← 生成器照它写 `pub struct Waves;`
    roots: &["px_field_alg"],
    source: "art/inst/waves.rs",
    body: "px_field_alg::remap_with(&px_field_alg::identity(), p, i.input.value(), g, ARG)",
}
```

`px_graphs/build.rs`（stage 1 的**计划**段）读它、生成 `OUT_DIR/insts_gen.rs`，`insts.rs`
`include!` 进来 ⇒ `px_graphs::insts::Waves` 这个类型（连同 `PxOp` / `InstNode` impl）是**生成出来的**。
详见 §5.5 与 `.agents/notes/art/21-codegen-types.md`。

**要动的四处**（一处声明、一处参数、一个图侧源文件、一份算法 rlib；"怎么声明"那一处现在就是上面那张表的一栏）：

| # | 落点 | 写什么 |
|---|---|---|
| 1 | **声明** `px_field_schema::ops` | `px_op! { FieldRemap, "field.remap", "px_field_op", params::RemapParams, FieldRemapInput, Field }` —— ⚠ 它**自己的 `Inputs`**（`FieldRemapInput`），而且**没有预置实现**（`px_field_op` 里没有它的 `px_body!`）：它存在的意义就是给实例当声明。⚠ 加了新声明还要在 `px_decls::TABLE` 里加**一行**（那道门数 `px_op!` 处数与表里条数；2026-09-20 之前这里是两份会漂开的清单，见 `px_decls/src/lib.rs` 的注释） |
| 2 | **参数** `px_field_schema::params::RemapParams` | `#[derive(…, px_derive::PxParams)]` ⇒ 每栏进键（`gain` / `bias` / `bands`） |
| 3 | **图侧函数** `art/inst/waves.rs` | `impl px_field_alg::field_fn::FieldFn for Waves`：纯函数、只认识 `px_field_alg`、自己保证落在 `[0,1]`。⚠ 它拿得到**节点参数**与**球面方向**（见下） |
| 4 | **算法** `px_field_alg`（rlib） | `remap_with` 与预置 `remap_sampled` 走**同一条** `map_grid`（只有那一份循环） |

三条口径（照体积域抄）：① 声明与 `type_name` 都**不带路径**（符号名按声明名拼、类型名要在
`include!` 的作用域里解得到）；② 实例源文件里 `use` **写全路径**、不放 `#[cfg(test)]`
（它会被原样 `include!`）；③ 改 `art/inst/*.rs` ⇒ **生成物一个字节不变** ⇒ 只有该实例库重编，
**七个图 exe 一个字节不动**（R1）。

```bash
px list                       # 应当看到三条实例：cloud.coarse/band、field.remap/waves、field.remap/latbands
px build                      # 缺哪条编哪条
px run field_remap            # 图侧自己的那张图（新图用 -Task run -Graph field_remap 走包装层）
px run field_remap --build    # 缺实例库时先把 stage 1 跑了再跑图
```

实测读数（本副本，`field_remap` 图）：

```text
冷：共 2 个节点：命中 0、重算 2
    输出 bands（field.remap/waves）：256×128｜值域 0.0000..1.0000｜均值 0.5603
热：命中 2、重算 0
（2026-09-20 复量：均值仍是 0.5603 —— `FieldFn` 多了两栏之后**算式一个字没动**，
  这正是「接口变了、内容不许变」那条判据的读数）
```

图侧函数收到的四栏（2026-09-20 起；`px_field_alg::field_fn::FieldFn` 的文档是权威）：

| 栏 | 是什么 | ⚠ 为什么是它 |
|---|---|---|
| `params: &RemapParams` | **这个节点在 `art/<图>/<节点名>.toml` 里给的那一份** | 在那之前 `remap_with` 把这栏丢掉了（`let _ = params`），图侧函数只能读 `RemapParams::default()` ⇒ **参数进键、不进计算**：改 TOML 会换节点键、会重算、写出来的产物**逐字节相同**（实测 `bands = 3 / gain = 0` 与不写文件内容都是 `bc8ab272f232`）。体积域从第一天起就把形状参数递给场函数（`CoverCloud` 由算子建）—— 场域这一档现在同一条规矩 |
| `upstream: f32` | 上游值**过了共享尺子（钳到 `[0,1]` + 可选平滑）之后**的值 | 归一化与钳制在算子侧（一份），「这一格的值怎么算」在图侧函数里（可换） |
| `uv: [f32; 2]` | 这一格的**纹素中心**坐标（`[0,1]²`，与 `Field::uv` 同口径） | 平面图案用它（径向波纹就是 `uv`） |
| `direction: [f32; 3]` | 这一格的**球面单位方向**（与 `Field::direction` 同口径） | ⚠ **球面函数只能用它**：`uv` 是图像坐标，`CubeMap` 投影下 `v` 跨的是「六张面叠起来的那一条」（`height = 6 × face`）而不是纬度 —— 拿 `uv[1]` 当纬度会在面与面之间跳变。`latbands` 那条实例就靠它取纬度 |

⚠ **两条预置/泛型的差别，别记混**（同一个 op id `field.remap`，两个不同的 op）：

| | 预置 `field.remap`（`Remap`，dylib） | 泛型 `field.remap/waves`（实例库） |
|---|---|---|
| 参数 | `params::remap::Params` | `RemapParams`（`gain` / `bias` / `bands`，**给图侧函数读**） |
| 尺子 | `in` / `out` / `smooth`（由本节点的 TOML 给） | `identity()`（固定 `[0,1] → [0,1]`） |
| 这一格的值 | 照抄上游 | 由图侧函数算 |

两者共享的只有 `map_grid` 那一份循环（`px_field_alg/src/remap.rs`）—— 这是"同一条路径"
那条纪律的落点，也是"改 TOML 里的对比度不会顺手改掉值域口径"的原因。

⚠ 走包装层时**图名不进 `ValidateSet`**（枚举图名就是又一份手维护清单）：`-Graph` 是自由参数，
`-Task run -Graph field_remap` 即可。新图的产物与参数索引落在 `target/pcg/field_remap/`，
`art/field_remap/` 今天不存在 ⇒ 参数全走 `Default`。

### 5.5 为什么类型要由 stage 1 生成、为什么 `cargo build` 里不编实例

设计全文与全部读数在 `.agents/notes/art/21-codegen-types.md`；这一节只讲**为什么是这个形状**。

**① 为什么"类型"必须由 stage 1 给出来。**
`PxOp` 的 `interface()`、`decl_hash()`、三个关联类型（`Params` / `Inputs` / `Payload`）
只有**编译过类型**的那一侧算得出（`19` §179.5）。而图侧今天不许写宏、也不该手写类型
（手写就会与 recipe 两处不一致）⇒ 只能**在编译图程序的时候**把类型造出来：

```text
cargo build（px_graphs）
 └─ build.rs ＝ stage 1 的「计划」段（**绝不调 cargo、绝不编译任何东西**）
      · 读 recipe（数据表）→ 校验 → 用「声明表」`px_decls` 取类型级事实 → 算 key
      · 写 OUT_DIR/insts_gen.rs（每条实例一个**生成出来的类型**：PxOp / InstNode impl，事实为 const）
      · **内容与盘上相同就不写那个文件**（保 mtime）★硬要求
      └─ src/insts.rs `include!` 它 ⇒ stage 2 手里的类型 = 生成出来的那一份
```

⚠ 生成器**必须**住 build.rs：它编译不了算子类型（build script 只看得见 `[build-dependencies]`）
⇒ 需要一张**编译过它们**的表来问 —— 那就是新 crate **`px_decls`**（一行一个声明、引用真类型，
事实全部取自真类型：`interface()` / `decl_hash()` / `type_name::<O::Params>()` …，
表里**没有一处人写的路径**）。它配一道门（`px_op!` 处数 11 == 表里 11 条）。
⚠ 它**不进任何实现库的源码名册** ⇒ 三个 schema 一个字节不用动、两条 key 逐位不变。
（第一版设计曾想"往 `px_*_schema` 里加 `decl()`"—— **被否**：往 schema 加任何字节，
包括一行注释，都会换掉它的 `SOURCE_HASH`，于是全部实例 key 与节点键一起换、`art/anchor` 要重登记。）

**② 为什么 `cargo build` 里绝不编实例。**
那是 `20` §186 那条纪律（`18` §171.5 用血换的）在**新形状下**的延续：
`cargo build` 只应该"**做计划**"，编译实例库**只由显式命令触发**。

* `px build` / `px run <图> --build` 是**唯一**的编译入口（stage 1 的「执行」段）；
* 默认的 `px run <图>` 仍**只读**：有缺就非零退出 + 打印该跑的命令；
* 实测那条铁律：build.rs 那一趟 `Compiling=0`（探针），而 `px build` 才真的起 cargo；
* ⚠ 于是"改一行泛型参数"的最坏代价仍然是**只重编那一条实例库** —— 七个图 exe 一位不动（R1）。

**③ "计划 / 执行"这条缝在代码里的落点**（读代码时按这三个名字找）：

| 段 | 住在哪 | 干什么 |
|---|---|---|
| **计划** | `px_graphs/build.rs` | 读 recipe + 问 `px_decls` → 生成 `OUT_DIR/insts_gen.rs`（+ 旁挂件 `insts_gen_catalogue.rs`） |
| **执行** | `px_cook::inst::BuildGraph`（`missing()` / `compile_one()` / `compile_missing()`） | 按 key 查盘、编缺的、收库 |
| **入口** | `px list` / `px build` / `px run <图>` | 只计划 / 计划+编 / 计划+（可选编）+跑 stage 2 |

**④ 出错时你看到什么（两条错，分开的）** —— 这一档最容易"猜错在哪一步"：

* **recipe 写错**（声明名不在 `px_decls` 表里 / 根不是 workspace 成员 / 源文件不在盘上 /
  源文件里没有那个 `type_name`）⇒ **`cargo build` 就 `panic!`**，并**点名是第几条 recipe**：

  ```text
  error: failed to run custom build command for `px_graphs …`
    recipe 第 1 条（cloud.coarse/band）：声明 `NoSuchDecl` 不在声明表里（`px_decls`）
    ⇒ 要么名字写错了，要么那是一个新的 `px_op!` —— 后者要先在 `px_decls/src/lib.rs` 的 `decl()` 里加一臂
  ```

* **体（`body`）编不过**（生成器不类型检查体 —— 那只能由实例库编译做）⇒ `cargo build` **过**，
  失败在 `px build` / `px run --build`，并打出**四行映射**：

  ```text
  ✗ 这个实例编不过：op id field.remap/waves
    · 体（body）来自 px_graphs/src/inst_recipe.rs:101（recipe 里那一条的 `body` 一栏）
    · 参数文件 art/inst/waves.rs（生成物里是 include! 进去的 ⇒ 报告里的
      target/jit/<key>/src/lib.rs:<行> 对应它）
    · 生成物：target/jit/<key>/（留着，不删）
  ```

  ⚠ 生成物**故意留着**：`target/jit/<key>/src/lib.rs` 就是那条实例库的全文（`include!` 了
  `art/inst/<名>.rs`），上面那两处行号对着它读最省事；要回收是**显式**的
  `px build --gc --deep`。⚠ 前两行里的行号来自那份**旁挂件** `OUT_DIR/insts_gen_catalogue.rs`
  —— 它**不参与任何 key、也不进图程序**（只在工具与测试里被 `include!`）。

---

## 6. 会咬人的地方（都踩过）

1. **声明与实现是两根字符串拼出来的**（`concat!(库名, "__", 类型名)`），编译器看不住。
   `px_graphs/tests/ops_load.rs` 有一道门**真的去装载**每一个声明过的算子 ——
   改了名字没人发现的那个，正是它要抓的。⚠ 跑这道门之前实现库得先在盘上。

2. **图程序不许 cargo 依赖实现库**；**实现库不许依赖 `px_graph` / `px_cook`**；
   `px_graph` 不许依赖任何算子；schema 层同理。`px_graphs/tests/crate_graph.rs` 用四条门看着。
   ⚠ 这几条语言一条都管不住（写进 `Cargo.toml` 就生效、不报错）。

3. **不许再手列源码清单**：源码指纹由 `build.rs` 遍历源码树算。
   `px_graph/tests/source_hash.rs` 那道门守的就是「**没有人再把 `include_str![…]` 那张表加回来**」
   （那正是「忘了补一份 ⇒ 陈旧命中」这条病的载体）。

4. **身份两半都是自动的**，所以「漏一份清单」「忘了升版本」这两类错**没有载体**了。
   ⚠ 反过来：**改了契约（`px_graph_schema`）而没重编实现库 ⇒ 装载时被握手拒**（带命令的报错）。
   跨 dylib 的 `extern "Rust"` ABI 只保证「同一份 rustc + 同一份契约」。

5. **实现库比源码旧时只告警**：跑的是旧实现，键也跟着旧身份走 —— 不会有陈旧命中，
   但读数上那行 `⚠ … 的实现比库新` 不能当噪音看。

6. **参数写错字段名**：靠 `deny_unknown_fields` 当场报。**缺文件仍是静默用默认值**，
   但 `finish()` 会打一行「`参数索引：N 个节点（M 个走默认值：<名字>）；字段名与生效值见 …`」，
   并写一份 `<图>/params.json`：每个节点实际生效的参数值 + 字段名。
   想补参数文件，照着那份 JSON 抄字段名即可。

7. **别给「域」再加一个并行声明**。`OpKind` 就是这样一个东西，已经删了 ——
   域就是 `Payload` 类型。**下次想加「域标签」时，回来读这一条。**

8. **载荷里别放节点名**：节点名与相机是**驱动**写盘时补的（`Graph` 的 `store` 里
   `PayloadBundle::to_bytes(node, cameras)`）。算子只回一个**无名、无相机**的 `PayloadBundle`。

9. **清单里「命中」也要有条目**。`store` 一度只在重算时记 —— 于是一趟**全命中**的运行
   写出**空清单**，而下游（`scene`）是**按节点名查清单拿键**的，当场断在
   「图 'planet' 的清单是空的」。清单是「这一趟图的成员与它们的键」，与命中与否无关。

10. **实例那张数据收据（recipe）有几条硬口径**（`21-codegen-types.md`，都是实测踩出来的）：
   * `body` 那一栏的泛型参数那一位**必须写占位符 `ARG`** —— 它是 **key 的一轴**；
     直接写真类型名（`&Band`）⇒ key 换（`96d4feb75ec4` → `8fcd505c8000`），旧实例库全成孤儿。
   * `inst_recipe.rs` **只许有类型定义 + 字面量**（`build.rs` 会读同一份、在 build script 里执行它），
     而且**注释只能用 `//`**（`//!` 在"被 `include!` 进模块体"的那一侧报
     `expected outer doc comment`；`#[path] mod` 遇 `//!` 时的报错还会误导成"文件找不到"）。
   * **加了新声明**（新 `px_op!`）⇒ 还要在 `px_decls` 的声明表里加一臂，否则 `cargo build` 就红
     （那道门数 `px_op!` 处数与表里条数）；**加一条实例** ⇒ 在 `INSTANCES` 里加一条（`build()` 不改）。
   * ⚠ 声明事实**不许**搬进 `px_*_schema`：那会给 schema 加字节 ⇒ 换 `SOURCE_HASH` ⇒
     全部实例 key 与节点键一起换、`art/anchor` 重登记。`px_decls` 不进任何实现库名册 ⇒ 不换键。
