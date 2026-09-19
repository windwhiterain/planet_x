# 写一个泛型算子库，再接进 graph scripts

> 这份文档是**操作清单**：从一个空目录到图脚本跑出产物，一共要写哪些文件、敲哪些命令、
> 每处约束是为什么。所有代码片段都与仓里现有代码对得上（`px_volume_op` 是已落地的实例）。
>
> 例子用 `mesh.iso`（**吃一个场函数、吐一张网格**）—— 它比现有的体积算子更"泛型"，
> 正好把三条边界（schema / dylib / 图脚本）都走一遍。⚠ 这个算子本身**没落地**，
> 是文档例子；`px_volume_op::CloudCoarse` 是它的已落地对应物。

---

## 0. 先分清三层

```text
px_graph_schema::op      ① 契约：OpTable / OpDescriptor / OpCall / ParamsCanonical
        ↑
px_*_schema              ② 数据：载荷类型 + 参数字段 + 序列化（**跨 dylib 边界只走这里**）
        ↑
px_*_op                  ③ 实现：算法体 + dylib 入口 + **类型化契约**（rlib 那一半）
        ↑
px_graphs                ④ 图脚本：普通 Rust 调用链（静态链 ③ 的 rlib，运行时装载 ③ 的 dylib）
```

**一句话判据**：算子的**实现**住在 dylib ⇒ 改实现不重编图程序；
图脚本**静态链**算子的 rlib ⇒ 参数类型/输入个数/输出域在编译期判得出来。
两条同时成立的关键是：**rlib 那一半只做接线，不含实现**。

---

## 1. 全部要写的文件

| # | 文件 | 谁写 | 干什么 |
|---|---|---|---|
| 1 | `px_<域>_schema/src/params.rs`（改） | 加一段 | 这个算子的参数字段 + 默认值 + `canonical` 分支 |
| 2 | `px_<域>_schema/src/payload.rs`（改） | 加两个函数 | 载荷 ↔ 字节（跨边界只走它） |
| 3 | `px_<域>_op/Cargo.toml`（改） | 加一行 | `crate-type = ["dylib", "rlib"]` |
| 4 | `px_<域>_op/src/lib.rs`（改） | 加一段 | 实现体 + `DESCRIPTOR` + `call` 分支 + `eval_*` 入口 |
| 5 | `px_<域>_op/src/typed.rs`（改） | 加一段 | `impl Op` —— **接线，不含实现** |
| 6 | `px_graphs/Cargo.toml`（改） | 加一行 | `px_<域>_op = { path = "../px_<域>_op" }` |
| 7 | `px_graphs/src/bin/<图>.rs`（改） | 加几行 | `cook_*::<算子>(...)` 调用 |
| 8 | `art/<图>/<节点名>.toml` | 新建 | 这个节点自己的参数 |
| 9 | `px_<域>_op/src/op.rs`（可选） | 新建 | **泛型缝**：只在这一个算子泛型时才拆出来 |

**只有 #9 与"泛型"有关**，其余 1–8 是任何算子都要走的。下面逐个写。

---

## 2. 逐文件

### ① `px_<域>_schema/src/params.rs`

```rust
pub const ISO: &str = "mesh.iso";

pub mod iso {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(default, deny_unknown_fields)]
    pub struct Params {
        /// 等值面的阈值。场值 ≥ 它的地方算"在实体里"。
        pub level: f32,
        /// 参数空间里每个轴切多少段（越大越细，也越慢）。
        pub subdivisions: u32,
    }

    impl Default for Params {
        fn default() -> Self {
            Self { level: 0.5, subdivisions: 96 }
        }
    }
}
```

⚠ `deny_unknown_fields` **要开**：图侧参数文件写错字段名时，这是唯一能在"读参数"那一刻
就报出来的机制（不然会静默用默认值）。见 §5 第 3 条。

然后在同文件的 `canonical(op_id, toml_text)` 里加一条分支：

```rust
pub fn canonical(op_id: &str, toml_text: Option<&str>) -> Result<String, String> {
    match op_id {
        // …已有的…
        ISO => Ok(px_graph_schema::canonical_params(&parse::<iso::Params>(toml_text)?)),
        other => Err(format!("不认识算子 {other}")),
    }
}
```

⚠ 这一条是**契约要求**：`ParamsCanonical` 必须由算子那一侧实现（默认值与字段集在它手里），
而且**不求值** —— 所以驱动的"先 key 后 cook"不变。

### ② `px_<域>_schema/src/payload.rs`

```rust
pub fn encode(mesh: &MeshData) -> PayloadBundle { /* … */ }

pub fn decode(bytes: &[u8]) -> Result<MeshData, String> {
    let bundle = PayloadBundle::from_bytes(bytes)?;
    // …
}
```

⚠ 载荷里**存的是内容，不是身份**：`PayloadBundle::new(kind, manifest_params, blobs)` 的
清单 `params` 只放形状数（顶点数/三角形数那种），**别把节点名或参数写进去** ——
节点名是驱动在写盘时补的（`Driver::store` 里的 `bundling`）。这一条踩过：漏了它
产物里 `id` 会是空的，而"逐字节对账"才抓得到。

### ③ `px_<域>_op/Cargo.toml`

```toml
[lib]
# ⚠ `dylib` 是给驱动装载的那一份；`rlib` 是给图脚本静态拿类型的那一份。
# 两个都要 —— 只留 rlib 就没法动态装载，只留 dylib 图脚本就没有类型检查。
crate-type = ["dylib", "rlib"]

[dependencies]
px_cook = { path = "../px_cook" }          # 类型化契约（`Op` / `FieldFn` / `Cooked`）
px_graph_schema = { path = "../px_graph_schema" }
px_<域>_schema = { path = "../px_<域>_schema" }
serde_json = { version = "1", features = ["float_roundtrip"] }
```

⚠ `float_roundtrip` **必须开**：默认的浮点解析不是正确舍入的，"读进来再写回去"不是恒等，
而本仓库的判据是逐字节的。

### ④ `px_<域>_op/src/lib.rs` —— 实现 + dylib 入口

```rust
pub mod typed;

use px_cook::field_fn::{FieldFn, SampleField};
use px_field_schema::field::Field;
use px_graph_schema::identity::fnv1a_sources;
use px_graph_schema::{
    Grid, OpCall, OpDescriptor, OpKind, OpTable, ParamsCanonical, PayloadBundle,
};
use px_<域>_schema::{params, payload, MeshData};

pub const VERSION: u32 = 1;

/// ⚠ 身份覆盖**共享依赖**：`lib.rs` 不动而 `noise.rs` 改了，也必须让身份变（§28.2）。
pub const SOURCE_HASH: u64 = fnv1a_sources(&[
    include_str!("lib.rs"),
    include_str!("op.rs"),
    include_str!("../../px_<域>_schema/src/mesh.rs"),
    include_str!("../../px_<域>_schema/src/params.rs"),
    include_str!("../../px_<域>_schema/src/payload.rs"),
    include_str!("../../px_field_schema/src/field.rs"),
]);

pub const INPUTS: &[&str] = &["field"];

pub const DESCRIPTOR: OpDescriptor = OpDescriptor {
    id: params::ISO,
    version: VERSION,
    source_hash: SOURCE_HASH,
    inputs: INPUTS,
    kind: OpKind::Mesh,
};

/// **泛型方法**：算法体对"场是什么"没有假设，只要求它能按方向取值。
///
/// ⚠ 这个签名就是"泛型算子"的全部含义。`F` 的实例在**调用方**（图侧 dylib 或
/// 图脚本）单态化 —— 见 §4。
pub fn eval<F: FieldFn>(params: &params::iso::Params, field: &F) -> MeshData {
    // …按 params.level 抽等值面，每一步问 field.cover(cloud, direction)…
}

/// 老路径（dylib 那一半）用的入口：上游那张**采样好的场**。
pub fn eval_sampled(params: &params::iso::Params, field: &Field) -> MeshData {
    eval(params, &SampleField { field })
}

#[unsafe(no_mangle)]
pub extern "Rust" fn px_<域>_op_table() -> &'static OpTable {
    static TABLE: OpTable = OpTable {
        ops: &[DESCRIPTOR],
        canonical_params: params::canonical as ParamsCanonical,
        call: call as OpCall,
    };
    &TABLE
}

extern "Rust" fn call(
    op_id: &str,
    params_json: &str,
    grid: Grid,
    inputs: &[&[u8]],
) -> Result<Vec<u8>, String> {
    match op_id {
        params::ISO => {
            let params: params::iso::Params = serde_json::from_str(params_json)
                .map_err(|err| format!("参数 JSON 解不开：{err}"))?;
            let field = px_field_schema::payload::decode(inputs[0], grid.projection)?;
            let mesh: MeshData = eval_sampled(&params, &field);
            payload::encode(&mesh).placeholder()
        }
        other => Err(format!("px_<域>_op 不认识算子 {other}")),
    }
}
```

⚠ `#[unsafe(no_mangle)]` 的函数名**必须**是 `<库名>_table`：驱动按**文件名词干**算入口名
（`px_mesh_op.dll` → `px_mesh_op_table`），名字不对就是运行期
`GetProcAddress failed`，没有任何编译期提示。

⚠ `OpKind` 决定图脚本该用哪个入口，也决定**键里掺不掺评审相机**：

| `OpKind` | 图脚本入口 | 键里掺相机？ |
|---|---|---|
| `Field` | `cook_field::<Op>(...)` | 掺（产物里带相机表） |
| `Mesh` | `cook_mesh::<Op>(...)` | 掺 |
| `Volume` | `cook_volume::<Op>(...)` | **不掺**（相机是"怎么看"，体积没人看） |

掺了相机就意味着产物内容随相机变 ⇒ 相机必须进键，否则会出现"同一个键、不同内容"。
这一条由 `cook_*` 那一层统一处理，算子不用管。

### ⑤ `px_<域>_op/src/typed.rs` —— 类型化契约（**接线，不含实现**）

```rust
use px_cook::{Cooked, Identity, Op};
use px_field_schema::field::Field;
use px_graph_schema::Grid;
use px_<域>_schema::{params, MeshData};

/// 一个具体算子：`impl Op` 之后，图脚本就能 `cook_mesh::<Iso>(...)`。
///
/// ⚠ 这里**一行实现都没有** —— `cook` 转发到 `crate::eval_sampled`。
/// 一旦把实现搬进来，图程序就静态链住了算法体 ⇒ 改算法要重编图程序
/// ——「类型检查」与「改实现不重编」两条会互斥。
pub struct Iso;

impl Op for Iso {
    const IDENTITY: Identity = Identity {
        id: params::ISO,
        version: crate::VERSION,
        // ⚠ 与 `lib.rs` 的 `SOURCE_HASH` **同一份清单、同一份算法**。
        // 漏一份就会「改了它而身份没变」⇒ 缓存静默给旧产物。
        source_hash: crate::SOURCE_HASH,
    };

    type Params = params::iso::Params;
    // 输入形态在**类型里**：个数接错、域接错都编译不过。
    type Inputs<'a> = &'a Cooked<Field>;
    type Payload = MeshData;

    fn cook(params: &Self::Params, field: &Self::Inputs<'_>, _grid: Grid) -> MeshData {
        crate::eval_sampled(params, field.field())
    }
}
```

⚠ `type Inputs<'a>` 的三种写法（对应"几个上游"）：

| 几个上游 | 写法 |
|---|---|
| 0 | `()` |
| 1 | `&'a Cooked<Field>` |
| N | `&'a [&'a Cooked<Field>; N]`（**数组长度就是个数**，写进类型里） |

### ⑥⑦ 图脚本侧

`px_graphs/Cargo.toml`：

```toml
[dependencies]
px_cook = { path = "../px_cook" }
px_<域>_op = { path = "../px_<域>_op" }   # ← 静态拿类型；实现仍在 dylib

[dev-dependencies]
px_<域>_op = { path = "../px_<域>_op" }   # ← 只为把 dylib 编出来给测试/老路径用
```

⚠ **这两条不能合并**：合并进 `[dependencies]` 就不会再编出 dylib；
合并进 `[dev-dependencies]` 图脚本就拿不到类型。`tests/crate_graph.rs` 那道门看着这件事。

`px_graphs/src/bin/<图>.rs`：

```rust
use px_cook::cook_mesh;
use px_<域>_op::typed as iso;

let height = cook_field::<field::Fbm>(&cache, "continents", (), canvas)?;
let mesh = cook_mesh::<iso::Iso>(&cache, "relief", &height, canvas)?;
```

⚠ 参数文件按**节点名**取（`art/<图>/relief.toml`）—— `cook_*` 的第二个参数既是节点名、
也是参数文件名。同一个算子在不同图/不同节点下可以有不同的参数文件。

### ⑧ `art/<图>/relief.toml`

```toml
level = 0.52
subdivisions = 128
```

（`deny_unknown_fields` 开着 ⇒ 写错字段名会当场报，不会静默用默认值。）

---

## 3. 命令

```bash
# 1) 类型化那一路直接编（图脚本会静态拿 op 的 rlib）
cargo build -p px_graphs --bin <图>

# 2) 把 dylib 编出来，供**老路径**与测试用
cargo build -p px_<域>_op

# 3) 跑图
cargo run -q -p px_graphs --bin <图>

# 4) 测试（含两条门：算子不许依赖 px_graph、图库不许静态链算子）
cargo test -p px_graphs
```

---

## 4. 泛型算子额外要做的一步（可选，只有泛型才需要）

上面 ①②③④⑤ 已经支持"参数类型 + 输入个数 + 输出域"的编译期检查。
**但 `eval<F: FieldFn>` 的实例还没有落点** —— 若要让图侧**现写**一个场函数
（不先栅格化成一张场），就得让实例住在**图侧自建的 dylib** 里：

```text
px_graphs/src/bin/<图>/mono/
  fields.rs            ⭐ stage 1：图自己的场函数（唯一编辑面）
  fields.mono          ⭐ 声明：lib / id / version / ingredient
  template.Cargo.toml / template.lib.rs / template.identity.rs   stage 2 模板
```

```bash
cargo run -q -p px_graphs --bin mono-gen -- px_graphs/src/bin/<图>/mono/fields.rs
```

生成器只吃 **stage 1 的路径**，其余全部推出来（stage 1 可以在任何位置）：
同目录同主名的 `.mono` 是声明，同目录的 `template.*` 是模板，
产物落 `target/debug/<库名>_op.dll`。图脚本用 `node(<id>, <节点名>, &[&上游])` 接上。

读数（已实测）：改一行场函数 → 生成 + 编 **1.7 s**，同期图程序 exe 的 sha256 **不变**，缓存键必变。

**为什么必须这个形状**：泛型的实例化要求「`eval<F>` 的定义」与「类型参数 `F`」在**同一个
编译单元**里；`F` 是图侧的东西、图程序是 `bin` ⇒ 实例只能落在图侧自建的 dylib 里。

---

## 5. 会咬人的地方（都踩过）

1. **入口名由库名派生**（`<文件名词干>_table`）。改名 DLL 而不改导出函数名 ⇒
   运行期 `GetProcAddress failed`，编译期毫无提示。

2. **`typed.rs` 里不许放实现**。放了就失去「改算法不重编图程序」；而 `px_graph` 也
   不许静态依赖算子（那道门看住）。
   反过来说：**dylib 那一半不许依赖 `px_graph`** —— 否则运行时装载会让**同一个进程里
   出现两份驱动**（CAS / 清单 / 索引各一份）。这比"重编"严重得多，门也看着。

3. **参数文件写错字段名是运行期才报**，除非 `deny_unknown_fields` 开着。
   而且缺文件是**静默用默认值** —— 设计师看不到自己少写了什么。这是当前最扎的一处。

4. **身份清单（`SOURCE_HASH` 的 `include_str!` 列表）漏一份** ⇒ 改了那个共享依赖而身份没变
   ⇒ 缓存静默给旧产物。`px_graph/tests/source_hash.rs` 那道门在看着它（至少两段、点到共享依赖）。

5. **载荷里别放节点名**。节点名与相机表是**驱动**在写盘时补的（`Driver::store::bundling`）；
   算子只回一个占位载荷（`bundle.placeholder()`）。漏了这一步产物里 `id` 会是空的。

6. **生成物与主 workspace 不能同时污染同一个 `target/`**。混过之后 `target/debug/` 里的
   算子 DLL 会变成半成品，而 `cargo build` **判它 fresh 不重编** ⇒ 莫名
   `LoadLibraryExW failed`。恢复只能 `cargo clean`。**未根治**（见笔记 §166.5）。

7. **`dylib` 这个 crate-type 的 ABI 是递归的**：生成的实例在运行时还要它自己那一套上游 DLL。
   所以"把生成的 dylib 单独拷到别处跑"不成 —— 见第 6 条与 §166.5。
