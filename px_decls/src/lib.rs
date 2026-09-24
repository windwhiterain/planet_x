//! **声明表**：声明名 → 类型级事实（`docs/system/codegen-types.md` 的"四件新东西 ②"）。
//!
//! 它为什么存在：stage 1 的**生成器**是 `px_graphs/build.rs`，而 build script **编译不了算子类型**
//! （它只看得见 `[build-dependencies]`）。可生成物里那三句
//! `type Params = <路径>;` 与 key 的两轴（`interface()` / `decl_hash()`）**只有编译过类型的那一侧**
//! 才算得出（`docs/system/generic-instances.md` §179.5）。⇒ 把"编译过类型"这件事放进一个**小 rlib**，
//! 由它把事实交出来。
//!
//! ⚠ **它为什么不是 `px_*_schema` 里的一个函数**（那是第一版设计）：`decl_hash()` 取的是
//!   **声明所在 crate 的全量源码指纹**（`px_fingerprint`，刻意粗粒度，§177）。往 schema 里加一行
//!   就换掉它的 `SOURCE_HASH` ⇒ 换掉全部实例 key 与节点键 + `art/anchor` 要重登记
//!   —— **为工具付产品级的账**。而 `px_decls` 不进任何实现库的名册 ⇒ 这类代价归零
//!   （实测：加它前后两条实例 key 逐位相同）。
//!
//! ⚠ **表里引用的是真类型**：`interface()` / `decl_hash()` / `type_name::<O::Params>()` 全从
//!   `<O>`（声明类型本身）取；`schema` / `module` 从 `<O>` 的**全路径**切出来。
//!   ⇒ 表里**没有一处人写的路径**。那三个类型路径由**生成物**再写一次
//!   （`type Params = <路径>;`）⇒ 名字改错/类型删掉是**图程序编译错**（当场可见），
//!   不是运行期才发现。
//!
//! ⚠ **每加一个 `px_op!` 就要在 [`TABLE`] 里加一行**：由 `tests/inst_gate.rs` 那道门看着
//!   （"`px_op!` 的处数 == 表里条数"，且表里每个名字都查得到）。
//!   ⚠ 2026-09-20 之前这里有**两份**会漂开的清单（`decl()` 的 match 臂 + `NAMES` 数组），
//!   而门只能数条数 ⇒ 配错行（`"Fbm" => facts::<Ridged>()`）每一道门都过。今天只有 [`TABLE`] 一份。

use px_graph_schema::PxOp;

/// 一个**声明**的类型级事实。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeclFacts {
    /// `<声明 as PxOp>::interface()` —— 三个类型名的哈希（key 的一轴）。
    pub interface: u64,
    /// `<声明 as PxOp>::decl_hash()` —— **声明所在 crate** 的源码指纹（key 的一轴）。
    pub decl_hash: &'static str,
    /// 声明的三个类型，**全路径**（`core::any::type_name` 取）。
    ///
    /// ⚠ 生成器照它们写 `type Params = <这一份>;` ⇒ 不需要人写任何路径字符串
    ///   （写字符串就是一处会烂的清单）。那三条路径**一定是合法路径文本**：它们就是
    ///   rustc 自己给的名字（`px_volume_schema::params::Params` 这种），实测没有空格、没有泛型参数
    ///   （见 `tests/inst_gate.rs` 那道"三个路径都含自己的 crate"的门）。
    pub params: &'static str,
    pub inputs: &'static str,
    pub payload: &'static str,
    /// 声明住的 crate（包名）—— 生成物的 `[dependencies]` 与 `pub use` 用它。
    pub schema: &'static str,
    /// 声明在它 crate 里的模块路径（`ops`）—— 生成物那句
    /// `pub use <schema>::<module>::<decl>;` 用它。
    pub module: &'static str,
    /// 声明**自己的类型名**（`CloudCoarse` 这种，不带路径）。
    ///
    /// ⚠ 它**不参与生成物**：唯一的消费者是那道"这一行指的是不是它自己写的那个类型"的门
    ///   （`tests/inst_gate.rs`）—— 表里配错一行（`("Fbm", || facts::<Ridged>())`）在别处
    ///   是查不出来的（条数对得上、schema 也对得上），只有拿类型名对名字才当场红。
    pub type_name: &'static str,
}

/// **声明表**：一行一个声明 —— **唯一**那份"有哪些声明"的清单。
///
/// ⚠ 2026-09-20 收口：从前这里有两份会漂开的清单（`decl()` 的 match 臂 + [`NAMES`] 那张字符串
///   数组），而门只能数**条数**（`px_op!` 处数 == 表里条数）。条数对得上、配错行的那种写错
///   （`"Fbm" => facts::<Ridged>()`）**每一道门都过**，直到某条实例拿它去编译才炸 —— 而且
///   炸在"生成物里的类型与体对不上"这种离现场很远的地方。今天只有这一张表：
///   [`decl`] / [`names`] / [`entries`] 全从它派生，**没有第二处可以写名字**。
///   ⚠ 加一个声明就是**加一行**（`px_op!` 那处 + 这一行，门数条数）。
pub const TABLE: &[(&str, fn() -> DeclFacts)] = &[
    // ⚠ `Constant` / `Remap` / `Mix` 三行已删（2026-09-27）：那三处 `px_op!` 收进了 element
    //   那一档（`px_elem` 的规格表 + `px_graphs::elem`）—— 它们**不是 `px_op!` 声明**，
    //   所以既不该、也不能进这张表（这道门数的是"各 schema 里 `px_op!` 的处数"）。
    //   那三条实例由 `px_graphs::insts::build` 照 `ELEM_SPECS` 登记（`InstKind::Element`）。
    ("Fbm", || facts::<px_field_schema::ops::Fbm>()),
    ("Ridged", || facts::<px_field_schema::ops::Ridged>()),
    ("Gradient", || facts::<px_field_schema::ops::Gradient>()),
    ("Warp", || facts::<px_field_schema::ops::Warp>()),
    ("Craters", || facts::<px_field_schema::ops::Craters>()),
    ("Stamps", || facts::<px_field_schema::ops::Stamps>()),
    ("FieldRemap", || facts::<px_field_schema::ops::FieldRemap>()),
    ("CloudCoarse", || {
        facts::<px_volume_schema::ops::CloudCoarse>()
    }),
    ("CubeSphere", || facts::<px_mesh_schema::ops::CubeSphere>()),
    ("Proxy", || facts::<px_mesh_schema::ops::Proxy>()),
    // ⚠ 2026-09-25 补：这一晚新增/漏登记的 7 处声明。`inst_gate` 那道门要求
    //   "`px_op!` 的处数 == 表里条数"，它就是这么发现少了 7 条的（20 vs 13）。
    ("Fbm3", || facts::<px_field_schema::ops::Fbm3>()),
    ("Ridged3", || facts::<px_field_schema::ops::Ridged3>()),
    ("Warp3", || facts::<px_field_schema::ops::Warp3>()),
    ("Density", || facts::<px_volume_schema::ops::Density>()),
    ("Emission", || facts::<px_volume_schema::ops::Emission>()),
    ("SkyNebula", || facts::<px_volume_schema::ops::SkyNebula>()),
    ("Stars", || facts::<px_volume_schema::ops::Stars>()),
    // ⚠ NURBS 域（`px_nurbs_schema`）：这十五条与 `ops.rs` 里的十五处 `px_op!` 一一对应。
    ("Circle", || facts::<px_nurbs_schema::ops::Circle>()),
    ("CurveEval", || facts::<px_nurbs_schema::ops::CurveEval>()),
    ("CurveAt", || facts::<px_nurbs_schema::ops::CurveAt>()),
    ("CurveHodograph", || {
        facts::<px_nurbs_schema::ops::CurveHodograph>()
    }),
    ("CurveInsert", || {
        facts::<px_nurbs_schema::ops::CurveInsert>()
    }),
    ("CurveElevate", || {
        facts::<px_nurbs_schema::ops::CurveElevate>()
    }),
    ("CurveTessellate", || {
        facts::<px_nurbs_schema::ops::CurveTessellate>()
    }),
    ("Sphere", || facts::<px_nurbs_schema::ops::Sphere>()),
    ("SurfaceEval", || {
        facts::<px_nurbs_schema::ops::SurfaceEval>()
    }),
    ("SurfaceAt", || facts::<px_nurbs_schema::ops::SurfaceAt>()),
    ("SurfaceInsert", || {
        facts::<px_nurbs_schema::ops::SurfaceInsert>()
    }),
    ("SurfaceElevate", || {
        facts::<px_nurbs_schema::ops::SurfaceElevate>()
    }),
    ("SurfaceTessellate", || {
        facts::<px_nurbs_schema::ops::SurfaceTessellate>()
    }),
    // ⚠ 这两个的实现在**另一个库**里（`px_nurbs_gpu_op`）：声明仍然住 `px_nurbs_schema`
    //   （图侧编译的是那一份），"声明 ↔ 实现"那根线是 `px_op!` 里的库名字符串。
    ("SurfaceTessellateGpu", || {
        facts::<px_nurbs_schema::ops::SurfaceTessellateGpu>()
    }),
    ("CurveTessellateGpu", || {
        facts::<px_nurbs_schema::ops::CurveTessellateGpu>()
    }),
];

/// **声明名 → 事实**。查不到回 `None`（生成器据此报错并点名是 recipe 第几条）。
///
/// ⚠ 各 schema 的命名空间是**同一个**（`FieldRemap` 与 `CloudCoarse` 都只是类型名）：
///   重名在 [`TABLE`] 里当场看得见 —— 那说明生成器不知道该写哪个路径。
/// ⚠ 它是**运行期**函数（不是 `const fn`）：`any::type_name` 与 `PxOp::interface()` 今天都不是
///   const（实测 `is not yet stable as a const fn` / "const traits are not yet supported"）。
///   调用它的是 **build script 与测试**，一次编译各跑一遍 ⇒ 这点开销无关紧要。
pub fn decl(name: &str) -> Option<DeclFacts> {
    TABLE
        .iter()
        .find(|(listed, _)| *listed == name)
        .map(|(_, facts)| facts())
}

/// 表里全部声明名（门与工具用它，不必自己再抄一遍）。
pub fn names() -> Vec<&'static str> {
    TABLE.iter().map(|(name, _)| *name).collect()
}

/// 表里全部登记（名字 + 事实）—— 与 [`TABLE`] 逐行同序（**没有第二份名字清单**）。
pub fn entries() -> Vec<(&'static str, DeclFacts)> {
    TABLE.iter().map(|(name, facts)| (*name, facts())).collect()
}

/// 各 schema 的 crate 目录名（相对 workspace 根）。
pub const SCHEMAS: &[&str] = &[
    "px_field_schema",
    "px_volume_schema",
    "px_mesh_schema",
    "px_nurbs_schema",
];

/// workspace 根（`CARGO_MANIFEST_DIR` 的上一级）—— 与 `px_graph::workspace_root` 同一口径。
///
/// ⚠ 门的左半边（"扫各 schema 数 `px_op!` 的处数"）走 [`workspace_root`] + `SCHEMAS` +
///   **`px_cook::inst_scan`**，而那个扫描器只是 **dev-dependency** ⇒ `px_cook` **不进本 crate
///   的正常依赖图**（它是 `px_graphs` 的 build script 的唯一消费者，不许被拖重）。
///   调用点：`tests/inst_gate.rs`。
pub fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

/// 一个声明的全部事实 —— **全部取自真类型**。
///
/// ⚠ `schema` / `module` / `type_name` 从**声明自己的全路径**
/// （`px_volume_schema::ops::CloudCoarse`）里切出来：第一段是 crate、最后一段是类型名、
/// 中间那一段是模块。⇒ 表里没有一处"人写的路径"。
/// ⚠ `type_name` 是 2026-09-20 补的：在那之前这张表**只数条数**，于是
///   `("Fbm", || facts::<Ridged>())` 这种**配错行**每一道门都过（条数对得上、每条也都能
///   指回自己的 schema crate），直到某条实例拿它去编译才炸 —— 而炸在离现场很远的地方。
///   有了这一栏，门就能判"**这一行指的是不是它自己写的那个类型**"（`inst_gate` 那条门）。
fn facts<O: PxOp>() -> DeclFacts {
    let path = ::core::any::type_name::<O>();
    let (schema, module) = split_schema_module(path).unwrap_or_else(|| {
        panic!("声明类型的全路径 `{path}` 不是 <crate>::<模块>::<类型> 形状（生成物写不出路径）")
    });
    let type_name = path.rsplit_once("::").map(|(_, name)| name).unwrap_or(path);
    DeclFacts {
        interface: O::interface(),
        decl_hash: O::decl_hash(),
        params: ::core::any::type_name::<O::Params>(),
        inputs: ::core::any::type_name::<O::Inputs>(),
        payload: ::core::any::type_name::<O::Payload>(),
        schema,
        module,
        type_name,
    }
}

/// `a::b::C` → `("a", "b")`；`a::C`（类型就在 crate 根）→ `("a", "")`。
fn split_schema_module(path: &str) -> Option<(&str, &str)> {
    let (schema, rest) = path.split_once("::")?;
    let module = match rest.rsplit_once("::") {
        Some((head, _type_name)) => head,
        None => rest,
    };
    Some((schema, module))
}
