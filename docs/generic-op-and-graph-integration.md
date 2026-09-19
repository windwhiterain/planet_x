# 写一个泛型算子库，再接进 graph scripts

> **操作清单**：从一个空目录到图脚本跑出产物，一共要写哪些文件、敲哪些命令、每处约束是为什么。
> 所有片段与仓里代码**逐字对齐**（`px_field_op` / `px_volume_op` / `px_mesh_op` 是已落地的实例）。
>
> 例子里那个 `mesh.iso`（吃场函数吐网格）是**文档例子，没落地** —— 它比现有算子更"泛型"，
> 正好把三条边界都走一遍。`px_volume_op::CloudCoarse` 是它的已落地对应物。

---

## 0. 图脚本里只看到这些（先把结论摆出来）

```rust
use px_cook::cook;

let clusters = cook::<field::Fbm>(&cache, "clusters", ())?;
let carved   = cook::<field::Warp>(&cache, "carved",
                   field::FieldPairInput { field: billows, offset: flow })?;
let mixed    = cook::<field::Mix>(&cache, "mixed",
                   field::MixInput { a: clusters, b: carved, mask: weight })?;
let coarse   = cook::<volume::CloudCoarse>(&cache, "coarse",
                   volume::CloudCoarseInput { coverage: mixed.clone() })?;
let proxy    = cook::<mesh::Proxy>(&cache, "proxy",
                   mesh::ProxyInput { volume: coarse })?;
```

**就这些**。没有 `encode`/`decode`、没有 `cook_field`/`cook_volume`/`cook_mesh`、
没有 `&[&a, &b]`、没有描述符、没有注册表、没有字符串 id。域、相机口径、编解码、身份全从算子类型推。

三件事因此是**编译期**的：

| | 靠什么 | 报错长什么样 |
|---|---|---|
| 输入个数/形状 | `O::Inputs`（关联类型钉死） | `expected MixInput, found TwoInput` —— 见 §3.3b |
| 输出域 | `O::Payload` | 把体积喂给声明 `In<Field>` 的结构就编不过 |
| 参数类型 | `O::Params` | 参数文件字段写错当场报（`deny_unknown_fields`） |

---

## 1. 三层分工

```text
px_graph_schema          ① 契约：键 / 载荷格式 / 参数规范化
        ↑
px_<域>_schema           ② 数据：超参数 struct（serde）+ 载荷类型 + 序列化
        ↑
px_<域>_op               ③ 实现 + 声明（`typed.rs` 里那几行宏）—— 一个普通 rlib
        ↑
px_graphs                ④ 图脚本：普通 Rust 调用链
```

**一句话判据**：算子就是图程序**静态链**进来的一个 Rust 库 ⇒ 参数类型、输入个数、
输出域全在**编译期**判。

⚠ **这里从前还有第二层**：算子编成 dylib、驱动按描述符表运行时装载，于是"改实现不重编
图程序"。那一层**整个删掉了**（原型期决定）—— 连同 `OpDescriptor` / `OpTable` / `OpCall` /
`ParamsCanonical` / loader / `mono-gen` / 生成的单态化实例。
代价是明摆着的：**改算子实现、或改图脚本里现写的场函数，都要重编图程序**。
换回来的是：没有运行时分派、没有字符串 id、没有"两份驱动"的风险。

---

## 2. 全部要写的文件

### 2.1 算子库作者

| # | 文件 | 写什么 |
|---|---|---|
| 1 | `px_<域>_schema/src/params.rs` | 超参数 struct：`#[derive(PxParams, Serialize, Deserialize, Default)]` |
| 2 | `px_<域>_schema/src/payload.rs` | 载荷 ↔ 字节（**缓存里存的就是这一份**） |
| 3 | `px_<域>_op/Cargo.toml` | `crate-type = ["rlib"]` + `px_cook` / `px_derive` |
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

pub use typed::{Constant, Fbm, Gradient, Mix, Remap, Ridged, Warp};
```

**就这些。** ⚠ 从前这里还有三行 dylib 管道（`px_canonical_params!` / `px_dylib_call!` /
`px_op_table!`）—— 它们只为"驱动按描述符表装载"服务，已经随那一层删掉。

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
use px_cook::{Cooked, px_op};

pub struct Fbm;
pub struct Mix;

px_op! { Fbm = params::FBM, 4, params::fbm::Params, (), Field,

         [include_str!("fbm.rs"),
          include_str!("../noise.rs"),
          include_str!("../../px_field_schema/src/field.rs"),
          include_str!("../../px_field_schema/src/params.rs")],
         |p, _i, g| crate::ops::fbm::eval(p, &[], g) }

px_op! { Mix = params::MIX, 1, params::mix::Params, MixInput, Field,

         [include_str!("mix.rs"),
          include_str!("../../px_field_schema/src/field.rs"),
          include_str!("../../px_field_schema/src/params.rs")],
         |p, i, g| crate::ops::mix::eval(p, &[i.a.sample(), i.b.sample(), i.c.sample()], g) }
```

参数顺序：`名字 = id, 超参数类型, 输入形状, 输出域, |超参数, 输入, 画布| 怎么算`。

⚠ **没有「版本」这一栏，也没有「源码清单」那一栏** —— 身份两半都是自动的，见 §3.3c。

| 字段 | 是什么 |
|---|---|
| `id` | 键认得出的那一半（图侧按它找节点） |
| `超参数类型` | 从 `art/<图>/<节点>.toml` 解出来的那个 struct |
| `输入形状` | 这个算子**自己定义**的输入 struct（`()` = 不吃上游）—— 见 §3.3b |
| `输出域` | `Field` / `VolumeData` / `MeshData` —— 相机口径与编解码都从它推 |
| `输出域` | `Field` / `VolumeData` / `MeshData`。⚠ **域就是这个类型** —— 不再有并行的 `OpKind`：相机掺不掺、画布算不算分辨率都由 `Payload` 自己声明（`WITH_CAMERAS` / `RESOLUTION_IS_CANVAS`）。一个声明，没有能对不上的第二处 |
| `输入形状` | 这个算子**自己定义**的输入 struct —— 输入个数与域都在里面 |

### 3.3b 图参数的形状：**算子自己定义**，两条 impl 由宏生成

形状是这个算子**接口的一部分**，所以住 `typed.rs`，字段名就是它吃的东西的名字：

```rust
/// 三张场（两张待混 + 一张权重）。**字段名有语义。**
#[derive(px_derive::PxInputs)]
pub struct MixInput {
    pub a: Cooked<Field>,
    pub b: Cooked<Field>,
    pub mask: Cooked<Field>,
}
```

`#[derive(PxInputs)]` 生成**两条 impl**，与超参数那边"键用宏、编解码用 serde"是同一套口径：

| | 超参数（`#[derive(PxParams)]`） | 图参数（`#[derive(PxInputs)]`） |
|---|---|---|
| 键 | 宏：字段名 + `HashField`（按类型写字节） | 宏：字段名 + `Cooked::key`（上游的键） |
| 编解码 | serde：`toml` → 结构 → 规范 JSON | 宏：`Cooked::from_bytes`（按**字段顺序**解上游字节） |

**为什么不给图参数用 serde 编解码**：超参数是**文本**（`art/<图>/<节点>.toml`），而图参数是内存里
的 `Cooked<T>` —— 它没有文本形式，重算那一侧拿到的只是字节 + 它自己那把键。所以编解码走
`Cooked::from_bytes`，但**同样是机械生成的**。

⚠⚠ **字段顺序 = 上游顺序**。这是"按位置解字节"的代价（手写版也是，只是 `let [a, b, mask] = inputs`
摆在眼前）。**改字段顺序等于换接口** —— 接口哈希会自动变，所以「记不记得升版本」不是问题。

**为什么不是 `px_cook` 给一组通用的 `Unary1/2/3`**（那一版我做过，删了）：字段名会变成
`a`/`b`/`c` —— **没有语义**，而且形状就"谁都不属于"了。**为什么也不加泛型**：那只为"多个
算子共用同一个形状"，而仓里吃三张场的算子只有一个 —— 换不来什么，却要多一个类型参数到处传。

**"接错就是编译错"一条不丢**：`O::Inputs` 是具体类型，少一个字段、给错域都编不过。

### 3.3c 身份：两半都自动（你什么都不用记）

```text
源码指纹 = build.rs 遍历「这个 crate 编译进去的全部源码」（自己 src/ + 所有 path 依赖的 src/ + 本文件）
接口哈希 = interface_hash([type_name::<Params>, type_name::<Inputs>, type_name::<Payload>])
```

| 你改了什么 | 源码指纹 | 接口哈希 | 结果 |
|---|---|---|---|
| 算法体一行 | 变 | 不变 | 重算（键变） |
| 参数 struct 加/删/改字段 | 变 | **变** | 重算 |
| 输入 struct 改 | 变 | **变** | 重算 |
| 输出域改 | 变 | **变** | 重算 |
| 只改了图脚本 | 不变 | 不变 | 全命中 |

**两半都不需要人维护**：不用列清单（`build.rs` 自己遍历），不用升版本（接口变了哈希自己变）。
`build.rs` 只 include 那份共享助手（`build/fingerprint.rs`），三个算子库共用一份。

⚠ **逃生门**：真要按类型之外的理由强制失效，给 `px_op!` 末尾加
`, interface = "n"` —— 它混进接口哈希。

⚠ 接口哈希参的是 `core::any::type_name`，**不保证跨编译器稳定**。我们的键只要求
「同一台机器上前后一致」，所以够用；跨机器共享缓存的场景要另想办法。

### 3.4 输出域与相机

**域（`Payload` 类型）决定键里掺不掺评审相机**：场/网格掺、体积**不掺**（相机是「怎么看」）。
这一条由 `cook` 统一处理，算子不用管 —— 而且**没有第二个地方要写它**：
域就是 `Payload` 类型，`WITH_CAMERAS` 由那个类型自己声明。

### 3.5 图脚本

```rust
use px_cook::cook;

// 无上游
let clusters = cook::<field::Fbm>(&cache, "clusters", ())?;
// 两个上游：具名字段
let carved = cook::<field::Warp>(&cache, "carved",
    field::FieldPairInput { field: billows, offset: flow })?;
// 三个
let mixed = cook::<field::Mix>(&cache, "mixed",
    field::MixInput { a: clusters, b: carved, mask: weight })?;
// 体积与网格
let coarse = cook::<volume::CloudCoarse>(&cache, "coarse",
    volume::CloudCoarseInput { coverage: mixed.clone() })?;
let proxy = cook::<mesh::Proxy>(&cache, "proxy",
    mesh::ProxyInput { volume: coarse })?;
```

- **上游是值，不是引用**：`Cooked<T>` 里是值 ⇒ 同一份被多处用就 `.clone()`。
  深拷一次比借用的 `'a` 链条干净（实测：引用版会把整个调用链拖进生命周期标注）。
- `cook` 的第二个类型参数**不用写** —— `inputs` 收的是 `O::Inputs`，关联类型会反推。
- 参数文件按**节点名**取（`art/<图>/mixed.toml`）。同一个算子在不同节点下可以有不同参数。

### 3.6 参数文件 `art/<图>/mixed.toml`

> 跑完看一眼 `target/pcg/<图>/params.json`：它是**每个节点实际生效的参数值 + 字段名**。
> 缺参数文件的节点会在末行被点出来（"N 个节点走默认值：…"）—— 照着 JSON 补文件即可。

```toml
frequency = 1.7
octaves = 6
```

超参数**留在这里**（不在 Rust 里）：改调参只需要重跑图，不重编。

---

## 3.7 键的语义：**一个节点 = 产出它的那些东西**

```text
节点键 = op_id + 接口哈希 + 源码指纹 + 规范参数 + 上游的键 [+ 相机（那一档才掺）]
                            └─ 该域的分辨率就是画布时，画布也在这里 ─┘
```

**不在键里的**（都曾经在里面，都删了）：

| 删掉的 | 为什么它不该在 |
|---|---|
| `graph_version` | 它是**图的属性**。改图脚本里别处一行代码，不该让这个节点的产物作废 |
| 画布尺寸（体积/网格） | 它们的分辨率由**参数**给，与画布无关。一刀切会让"改画布"连带重烘它们 |
| 投影 | 它只影响编解码的字节布局，那件事该由 `Payload` 的 `encode`/`decode` 承担 |

场的画布**要**在键里 —— 场的分辨率就是画布。这条由域自己声明
（`Payload::RESOLUTION_IS_CANVAS`），`px_cook` 按它决定，不在这里一刀切。

**这条改动的实际好处**（实测）：改 `art/clouds/weight.toml` 一个值 ⇒ 只有
`weight` 与它的 8 个下游重算，`clusters` / `billows` / `flow` / `carved` 照旧命中。
从前任何一个坐标变了都可能把整张图的键掀掉。

⇒ 同样的算子、同样的参数、同样的上游 ⇒ **同一个产物**，不管图脚本长什么样。

## 4. 命令

```bash
cargo build -p px_graphs --bin <图>     # 类型化那一路
cargo build -p px_<域>_op              # 一个普通的 Rust 库
cargo run   -q -p px_graphs --bin <图>  # 跑图
cargo test  -p px_graphs                # 两条门（见 §5）
```

---

## 5. 泛型算子的实例落在哪

泛型的实例化要求「`bake<F>` 的定义」与「类型参数 `F`」在**同一个编译单元**里。
实例现在只有**一份**：图程序自己 —— `cook::<volume::CloudCoarse>(…)` 时静态链接进去。

⚠ **从前还有一份**：图侧现写场函数（`ClosedForm { f: |cloud, dir| … }`）编成一个
dylib，图程序按 op id 运行时装上，于是**改场函数不必重编图程序**。
那一整套（`mono-gen` + 四份模板 + 描述符表 + loader + `--closed-cover`）**已删**。
`px_cook::field_fn` 的模块文档里记着这件事与代价，别把它当成"还没做"。

---

## 6. 会咬人的地方（都踩过）

1. **算子库不许再声明 `dylib`/`cdylib`** —— 那会让"运行时按描述符装载"悄悄长回来。
   `px_graphs/tests/crate_graph.rs` 有一道门看着。

2. **`px_graph` 不许静态依赖任何算子**（它只认 `Cache`）；schema 层同理。同一个门看着。

3. **参数写错字段名**：靠 `deny_unknown_fields` 当场报。**缺文件仍是静默用默认值**
   （老口径没动），但跑完会写一份 `<图>/params.json`：**每个节点实际生效的参数值 + 字段名**，
   外加一句"N 个节点走默认值：<名字>"。想补参数文件，照着那份 JSON 抄字段名即可。

4. **身份两半都是自动的**，所以「漏一份清单」「忘了升版本」这两类错**没有载体**了：
   源码指纹由 `build.rs` 遍历源码树算，接口哈希由三个类型名推。
   `px_graph/tests/source_hash.rs` 那道门现在守的是「**没有人再把清单加回来**」。

5. ~~`OpKind` 必须与 `Payload` 一致~~ —— **这一条已经不存在了**。不再有 `OpKind`：
   域就是 `Payload` 类型。从前那条 `const` 断言守的是“你写了个冗余字段又写错了”，
   现在连字段都没有（`WITH_CAMERAS` / `RESOLUTION_IS_CANVAS` 直接挂在 `Payload` 上）。

6. **载荷里别放节点名**：节点名与相机是**驱动**写盘时补的（`Driver::store` 的 `bundling`）。
   算子只回 `bundle.placeholder()`。

7. ~~生成物与主 workspace 各用各的 `target/`~~ —— **这条随 dylib 一起没了**：
   算子不再产出 dylib，也就没有“两个构建图互相覆盖同名 DLL”这件事。

8. **生成的实例是自足的**（实测：只导入系统库，`px_` 前缀的导入 0 个）——
   cargo 对 path 依赖选 rlib ⇒ 上游静态链进去。所以**永远不要**从 `target/mono/` 往
   `target/debug/` 拷任何 `px_*.dll`：那是另一个构建图的产物，拷了就把正确的覆盖掉
   （这条错只在运行期冒出来，`cargo build` 还判它 fresh）。生成器每次都会读 PE 导入表
   验一遍（`assert_self_contained`）。
