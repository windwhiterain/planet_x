# 写一个泛型算子库，再接进 graph scripts

> **操作清单**：从一个空目录到图脚本跑出产物，一共要写哪些文件、敲哪些命令、每处约束是为什么。
> 所有片段与仓里代码**逐字对齐**（`px_field_op` / `px_volume_op` / `px_mesh_op` 是已落地的实例）。
>
> 例子里那个 `mesh.iso`（吃场函数吐网格）是**文档例子，没落地** —— 它比现有算子更"泛型"，
> 正好把三条边界都走一遍。`px_volume_op::CloudCoarse` 是它的已落地对应物。

---

## 0. 图脚本里只看到这些（先把结论摆出来）

```rust
use px_cook::{cook, Unary1 as One, Unary2 as Two, Unary3 as Three};

let clusters = cook::<field::Fbm>(&cache, "clusters", (), canvas)?;
let carved   = cook::<field::Warp>(&cache, "carved", Two { a: billows, b: flow }, canvas)?;
let mixed    = cook::<field::Mix>(&cache, "mixed", Three { a: clusters, b: carved, c: weight }, canvas)?;
let coarse   = cook::<volume::CloudCoarse>(&cache, "coarse", One { a: mixed.clone() }, canvas)?;
let proxy    = cook::<mesh::Proxy>(&cache, "proxy", mesh::VolumeInput { a: coarse }, canvas)?;
```

**就这些**。没有 `OpKind`、没有 `encode`/`decode`、没有 `cook_field`/`cook_volume`/`cook_mesh`、
没有 `&[&a, &b]`、没有描述符、没有注册表。域、相机口径、编解码、身份全从算子类型推。

三件事因此是**编译期**的：

| | 靠什么 | 报错长什么样 |
|---|---|---|
| 输入个数/形状 | `O::Inputs`（关联类型钉死） | `expected Unary3<Cooked<Field>>, found Unary2<Cooked<Field>>` |
| 输出域 | `O::Payload` | 把体积喂给声明 `In<Field>` 的结构就编不过 |
| 参数类型 | `O::Params` | 参数文件字段写错当场报（`deny_unknown_fields`） |

---

## 1. 三层分工

```text
px_graph_schema::op      ① 契约：OpTable / OpDescriptor / OpCall / ParamsCanonical
        ↑
px_<域>_schema           ② 数据：超参数 struct（serde）+ 载荷类型 + 序列化
        ↑
px_<域>_op               ③ 实现（dylib）+ **声明**（rlib：typed.rs 里那几行宏）
        ↑
px_graphs                ④ 图脚本：普通 Rust 调用链
```

**一句话判据**：算子的**实现**住在 dylib ⇒ 改实现不重编图程序；
图脚本**静态链**算子的 rlib ⇒ 参数/输入/输出都在编译期判。
两条同时成立的关键是：**rlib 那一半只有声明，没有实现**。

---

## 2. 全部要写的文件

### 2.1 算子库作者

| # | 文件 | 写什么 |
|---|---|---|
| 1 | `px_<域>_schema/src/params.rs` | 超参数 struct：`#[derive(PxParams, Serialize, Deserialize, Default)]` |
| 2 | `px_<域>_schema/src/payload.rs` | 载荷 ↔ 字节（跨 dylib 边界只走这里） |
| 3 | `px_<域>_op/Cargo.toml` | `crate-type = ["dylib", "rlib"]` + `px_cook` / `px_derive` |
| 4 | `px_<域>_op/src/<算子>.rs` | **算法体**：普通 Rust 函数 |
| 5 | `px_<域>_op/src/typed.rs` | **声明**：`px_op!` 一行 / 算子 + 输入别名 |
| 6 | `px_<域>_op/src/lib.rs` | **三行宏**（见 §2.3） |

### 2.2 图作者

| # | 文件 | 写什么 |
|---|---|---|
| 7 | `px_graphs/Cargo.toml` | `px_<域>_op`（`[dependencies]` **和** `[dev-dependencies]` 各一条） |
| 8 | `px_graphs/src/bin/<图>.rs` | `cook::<算子>(...)` 调用链 |
| 9 | `art/<图>/<节点名>.toml` | 这个节点的**超参数** |

### 2.3 一个算子库的 `lib.rs` 全文

```rust
pub mod typed;
pub mod ops;          // 算法体

px_cook::px_canonical_params!(typed::Fbm, typed::Mix);   // 超参数规范化
px_cook::px_dylib_call!(typed::Fbm, typed::Mix);         // 字节 → 字节
px_cook::px_op_table!("px_field_op", typed::Fbm, typed::Mix);  // 入口符号
```

⚠ `px_op_table!` 的第一个参数必须等于 **crate 名**（dll 名）：驱动按**文件名词干**算入口符号
（`px_field_op.dll` → `px_field_op_table`）。写错是**编译错**（`const` 断言），不是运行期惊喜。

---

## 3. 逐件写清

### 3.1 超参数：`px_<域>_schema/src/params.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default, px_derive::PxParams)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    pub frequency: f32,
    pub octaves: u32,
}
```

- `PxParams` **必须**：它生成 `PxKeyed`（每个字段按自己的类型写进键）。
  手写的话，"加了字段却忘了进 `key`"是个**静默** bug —— 改了参数却命中旧产物。
- `deny_unknown_fields` **要开**：参数文件写错字段名时，这是唯一在"读参数"那刻就报的机制。
- 纯局部开关（不进键的那种）加 `#[nohash]`。

新增字段类型时要在 `px_graph_schema::HashField` 上加一条实现（闭集，编译器会当场报）。

### 3.2 算法体：`px_<域>_op/src/ops/fbm.rs`

普通 Rust 函数，签名怎么写都行 —— 它只被 `typed.rs` 里那一声 `render` 调用。

```rust
pub fn eval(params: &params::fbm::Params, inputs: &[&Field], grid: Grid) -> Field { … }
```

### 3.3 声明：`px_<域>_op/src/typed.rs`

```rust
use px_cook::{px_op, Cooked, Unary1, Unary2, Unary3};
use px_graph_schema::OpKind;

pub struct Fbm;
pub struct Mix;

px_op! { Fbm = params::FBM, 4, params::fbm::Params, (), Field,
         OpKind::Field, &[],
         [include_str!("fbm.rs"),
          include_str!("../noise.rs"),
          include_str!("../../px_field_schema/src/field.rs"),
          include_str!("../../px_field_schema/src/params.rs")],
         |p, _i, g| crate::ops::fbm::eval(p, &[], g) }

px_op! { Mix = params::MIX, 1, params::mix::Params, Unary3<Cooked<Field>>, Field,
         OpKind::Field, &["a", "b", "mask"],
         [include_str!("mix.rs"),
          include_str!("../../px_field_schema/src/field.rs"),
          include_str!("../../px_field_schema/src/params.rs")],
         |p, i, g| crate::ops::mix::eval(p, &[i.a.sample(), i.b.sample(), i.c.sample()], g) }
```

参数顺序：`名字 = id, 版本, 超参数类型, 输入形状, 输出域, OpKind, 输入名, 源码清单, |超参数, 输入, 画布| 怎么算`。

| 字段 | 是什么 |
|---|---|
| `id` / `版本` | 键认得出的那一半。⚠ **版本只在接口变了时才升**；改实现不用动（源码哈希自己会变） |
| `超参数类型` | 从 `art/<图>/<节点>.toml` 解出来的那个 struct |
| `输入形状` | `()` / `Unary1<Cooked<T>>` / `Unary2<…>` / `Unary3<…>` —— **个数写进类型里** |
| `输出域` | `Field` / `VolumeData` / `MeshData` —— 相机口径与编解码都从它推 |
| `OpKind` | 描述符里的档；**必须与输出域一致**（`Field`↔`Field`、`VolumeData`↔`Volume`…）。⚠ 不一致是**编译错**（`px_op!` 里的 `const` 断言） |
| `输入名` | 老路径描述符里的输入名（`["coverage"]` 那种）。图侧真正的检查在 `输入形状` 上 |
| `源码清单` | **它自己的实现文件 + 它依赖的共享件**。漏一份 ⇒ 改了它却命中旧产物 |

### 3.4 输出域与相机

`OpKind` 决定键里掺不掺评审相机：`Field`/`Mesh` 掺、`Volume` **不掺**（相机是"怎么看"）。
这一条由 `cook` 统一处理，算子不用管 —— 但它必须与 `Payload` 对得上，否则会出现
"同一个键、不同内容"。

### 3.5 图脚本

```rust
use px_cook::{Cooked, Unary1 as One, Unary2 as Two, Unary3 as Three, cook};

// 无上游
let clusters = cook::<field::Fbm>(&cache, "clusters", (), canvas)?;
// 两个上游：具名字段
let carved = cook::<field::Warp>(&cache, "carved", Two { a: billows, b: flow }, canvas)?;
// 三个
let mixed = cook::<field::Mix>(&cache, "mixed", Three { a: clusters, b: carved, c: weight }, canvas)?;
// 体积与网格
let coarse = cook::<volume::CloudCoarse>(&cache, "coarse", One { a: mixed.clone() }, canvas)?;
let proxy = cook::<mesh::Proxy>(&cache, "proxy", mesh::VolumeInput { a: coarse }, canvas)?;
```

- **上游是值，不是引用**：`Cooked<T>` 里是值 ⇒ 同一份被多处用就 `.clone()`。
  深拷一次比借用的 `'a` 链条干净（实测：引用版会把整个调用链拖进生命周期标注）。
- `cook` 的第二个类型参数**不用写** —— `inputs` 收的是 `O::Inputs`，关联类型会反推。
- 参数文件按**节点名**取（`art/<图>/mixed.toml`）。同一个算子在不同节点下可以有不同参数。

### 3.6 参数文件 `art/<图>/mixed.toml`

```toml
frequency = 1.7
octaves = 6
```

超参数**留在这里**（不在 Rust 里）：改调参只需要重跑图，不重编。

---

## 4. 命令

```bash
cargo build -p px_graphs --bin <图>     # 类型化那一路
cargo build -p px_<域>_op              # 把 dylib 编出来（老路径/测试用）
cargo run   -q -p px_graphs --bin <图>  # 跑图
cargo test  -p px_graphs                # 两条门（见 §5）
```

---

## 5. 泛型算子额外的一步：让它有"实例落点"

上面 1–9 已经给了编译期检查。**但 `render` 的实例还没落点** ——
若要让图侧**现写**一个场函数（不先栅格化成一张场），实例必须住在**图侧自建的 dylib** 里：

```text
px_graphs/src/bin/<图>/mono/
  fields.rs            ⭐ stage 1：图自己的场函数（唯一编辑面）
  fields.mono          ⭐ 声明：lib / id / version / ingredient
  template.Cargo.toml / template.lib.rs / template.identity.rs   stage 2 模板
```

```bash
cargo run -q -p px_graphs --bin mono-gen -- px_graphs/src/bin/<图>/mono/fields.rs
```

生成器只吃 **stage 1 的路径**，其余全从它推出来（stage 1 可以在任何位置）：

| 从哪来 | 是什么 |
|---|---|
| 命令行 | stage 1 的 `.rs` |
| 同目录、同主名的 `<主名>.mono` | 声明（`lib` 默认 `px_mono_<主名>`） |
| 同目录的 `template.*` | stage 2 的三份模板 |
| 计算的 | `target/mono/<库名>/{crate,build}` → `target/debug/<库名>_op.dll` |

图脚本用 `node(<id>, <节点名>, &[&上游])` 接上（老路径，因为这一份实例导出的是描述符表）。

**读数**（已实测）：改一行场函数 → 生成 + 编 **1.7 s**，同期图程序 exe 的 sha256 **不变**，键必变。

**为什么必须这个形状**：泛型的实例化要求「`render<F>` 的定义」与「类型参数 `F`」在**同一个
编译单元**里；`F` 是图侧的东西、图程序是 `bin` ⇒ 实例只能落在图侧自建的 dylib 里。

---

## 6. 会咬人的地方（都踩过）

1. **入口符号由库名派生**（`<文件名词干>_table`）。`px_op_table!` 现在有 `const` 断言看着它 ⇒
   写错是编译错。**但改了 crate 名之后别忘了同步那个字面量。**

2. **`typed.rs` 里不许放实现**（放了就失去"改算法不重编图程序"）；
   **dylib 那一半不许依赖 `px_graph`**（否则同一进程两份驱动，比"重编"严重得多）。
   两道门看着：`px_graphs/tests/crate_graph.rs`。

3. **参数写错字段名**：靠 `deny_unknown_fields` 当场报；**缺文件是静默用默认值**
   —— 设计师看不到自己少写了什么。**这一处还扎人**。

4. **身份清单漏一份** ⇒ 改了共享依赖而身份没变 ⇒ 缓存静默给旧产物。
   `px_graph/tests/source_hash.rs` 那道门扫每个算子那段清单（至少两段 + 点到共享依赖）。

5. **`OpKind` 必须与 `Payload` 一致**（相机掺不掺、解码走哪条都从它推）——
   `px_op!` 里那条 `const` 断言会在算子库编译时炸，不是运行期的怪事。

6. **载荷里别放节点名**：节点名与相机是**驱动**写盘时补的（`Driver::store` 的 `bundling`）。
   算子只回 `bundle.placeholder()`。

7. **生成物与主 workspace 不能同时污染同一个 `target/`**：混过之后算子 DLL 会变成半成品，
   而 `cargo build` **判它 fresh 不重编** ⇒ 莫名 `LoadLibraryExW failed`；只能
   `cargo clean -p <那几个算子 crate>`。**未根治**（笔记 §166.5）。

8. **`dylib` 这个 crate-type 的 ABI 是递归的**：生成的实例运行时还要它自己那一套上游 DLL，
   所以"拷到别处单独跑"不成。**未根治**（同 §166.5）。
