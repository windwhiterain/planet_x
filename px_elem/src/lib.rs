//! **element-wise 算子的作者面**：一个泛型算子 + 每个函数自己的类型。
//!
//! 用户的裁定（2026-09-27）："一个 generic 的 element wise Op 在图脚本测单态化成任意
//! element wise 算子，还能融合算子" / "用 rust 泛型" / "实例必须要 dylib，否则拖慢编译速度，
//! 泛型是对脚本编写者的**手感**要求"。这一份就是那句话的落点：
//!
//! ```text
//! 脚本作者写：  cached(&graph, "constant", Elementwise::<Constant>::new(), params, ())
//! 手感来自：    `params` 是 **ConstantParams**（每个函数自己的类型）—— 字段名错、少给一个
//!               上游、形状类型不对，全是**编译错**；不再是"三个 float 撑所有函数"的口袋。
//! 实现住在哪：  体文件（`px_elem/body/*.rs`）编成**内容寻址的 dylib**（`px build`）——
//!               改一行算法只重编那一份库，图程序一位不动（R1）。
//! 身份：        **内容**（声明指纹 ‖ 根名册 ‖ 接口 ‖ 体模板 ‖ 体文件字节）⇒ 两个图用同一个
//!               函数就共享同一份库与同一份产物。
//! ```
//!
//! ⚠ 上面那个写法里的 `::new()` **不是可省的**：`Elementwise<F>` 带着一个 `PhantomData` 字段
//!   （类型参数要在字段里出现），而那个字段是私有的 ⇒ 图脚本**写不出** `Elementwise::<Constant>`
//!   这样一个值（那是"类型"不是"值"），只能走 `PxOp::new()`。预置那一档（`field::Fbm`）是单元
//!   结构体，所以它们能裸写 —— 两档在这点上手感不同，改起来要动这个结构体的形状。
//!
//! ⚠ **它为什么不自己算**：`Elementwise<F>::render` 只是"按内容键把那一份库装进来、调它的
//!   符号" —— 算法一个字都不在本 crate 里（否则改算法就要重编它，而它是**每个图程序都依赖**
//!   的 crate）。这条与 `px_op!` 那一档（预置库）同构，只是"哪个库"从常量变成了内容键。
//!
//! ⚠ **它省掉的是"图侧"那一份代码生成**（不是"这一档不代码生成" —— 那句说法 2026-09-27 当场
//!   被问住、已纠正，见 `44` §8）：`px build` 那一侧**照样**写一个极小的具体 crate
//!   （`target/jit/<内容键>/src/lib.rs`：类型别名 + `include!(体文件)` + `px_body!` +
//!   `px_impl_lib!()`）再编成 dylib —— **单态化就发生在编它的时候**（Rust 的泛型只在编译期
//!   单态化，dylib 里装不下泛型函数，这正是"实例必须要 dylib"那条硬约束）。
//!   省掉的是图侧那份类型 + 事实表（`OUT_DIR/insts_gen.rs`）：内容键可以在**运行期**算
//!   （`px_cook::inst::key_of_facts` 读体文件字节 + 数各根的源码名册）—— 与 `px build` 判断
//!   "缺哪些库"走的是**同一个函数**。⇒ 没有"把事实烘成 const"这一步，也就没有孤儿规则那道墙
//!   （`impl Facts for <px_elem 的类型>` 写在图程序里是非法的）。代价：每个节点算一次键
//!   （一次名册读取 + 哈希，与既有实例那一档同一量级；见 `px_graphs/src/insts.rs` 的缓存）。
//!
//! ⚠ **融合就是"一个函数一次拿到全部上游"**：`F::Inputs` 自己声明几个上游（`()/FieldInput/
//!   FieldPairInput/…`），整条链在一个循环里算完 ⇒ 一个节点、一个产物、上游只读一次。

pub use px_field_schema::params::Shape;
use px_graph_schema::{PxInputs, PxKeyed};
use serde::Serialize;
use serde::de::DeserializeOwned;

pub mod constant;
pub mod fuse;
pub mod mix;
pub mod remap;

pub mod specs;

/// **各函数的参数类型**从它们的模块提到 crate 根。
///
/// ⚠ 为什么要这一行：体文件（`px_elem/body/*.rs`）里参数类型写的是 `px_elem::<Params>`
///   （它们被 `include!` 进生成的实例库 —— 那儿只有"外部 crate 名"可用，见那个文件的头一段），
///   而**同一个类型在两处必须能写**：作者面（图脚本给参数）与体文件。
///   ⚠ 加一个 element 函数 = 这里再 re-export 它那一个参数 struct：`px_elem_specs!` 那一行
///   给的是**类型**（不是路径），宏没法替任意类型发一句 `use`。
pub use constant::ConstantParams;
pub use fuse::{FuseInput, FuseParams};
pub use mix::{MixInput, MixParams};
pub use remap::{RemapInput, RemapParams};
pub use px_field_schema::field::Field;
pub use px_graph_schema::interface_hash;
pub use specs::ELEM_SPECS;

/// 一张场**自己的形状**（"输出与上游同形"那条口径的落点）。
///
/// ⚠ 过滤类的 `shape` 一律走它：形状只有一个来源（上游那张场），于是"输入与输出形状不一致"
///   这类缺陷在 element 这一档**不可表达**（从前那是调用点的纪律）。
pub fn like(field: &Field) -> Shape {
    Shape {
        width: field.width,
        height: field.height,
        projection: field.projection,
    }
}

/// 一条 element 函数的**全部根**：作者声明的那些 ∪ **两个固定的**。
///
/// ⚠ 为什么固定的两个也在里面：参数 / 上游那些**类型**住在那两个 crate 里
///   （`px_elem` 的 `ConstantParams`、`px_field_schema` 的 `Shape`/`Field`）——
///   它们的源码变了（比如 `Shape` 加一栏）也必须换实例键，否则会命中一份按旧布局编的库。
/// ⚠ 这一条口径**只有这一处**：生成器（build script）与图侧运行期算键都调它，
///   两边各写一份就会出"库名与图谱算的键对不上"那种最难查的错。
pub fn all_roots(spec: &ElemSpec) -> Vec<&'static str> {
    let mut roots = vec!["px_elem", "px_field_schema"];
    for each in spec.roots {
        if !roots.contains(each) {
            roots.push(each);
        }
    }
    roots
}

/// 一个 element 函数的**类型级事实**（生成器 / `px build` / 门读它）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElemFacts {
    pub interface: u64,
    pub decl_hash: &'static str,
    pub params: &'static str,
    pub inputs: &'static str,
    pub payload: &'static str,
}

/// 一条 element 函数的**规格**（[`crate::px_elem_specs!`] 一行展开成一条）。
pub struct ElemSpec {
    /// 类型名（`Constant`）—— 图脚本写 `Elementwise::<Constant>`。
    pub ty: &'static str,
    /// 人读名（`field.constant`）。
    pub name: &'static str,
    /// 体文件（相对 workspace 根）—— **内容**进身份，路径不进。
    pub source: &'static str,
    /// 体编译时链的 crate（build graph 的边）。
    pub roots: &'static [&'static str],
    /// `px_body_raw!` 的体表达式（生成物逐字抄它）。
    pub body: &'static str,
    /// 类型级事实（从**真类型**取）。
    pub facts: fn() -> ElemFacts,
}

/// `px_elem` 那一份源码指纹（element 函数的家）—— 实例键的一轴（`decl_hash`）。
///
/// ⚠ `env!` 在**本 crate** 展开 ⇒ 图侧那一边（`px_graphs::elem` 的生成物）读到的就是它：
///   往参数 struct 里加一栏、或改规格表 ⇒ 换 `decl_hash` ⇒ 换实例键（`19-generic-inst.md` §177）。
pub const DECL_HASH: &str = env!("PX_SOURCE_HASH");

/// 一个 element 函数**对脚本作者的那一面**：他的参数类型、他的上游、他的名字、他要多大。
///
/// ⚠ `SOURCE` / `ROOTS` / `BODY` 三栏**由 [`crate::px_elem_specs!`] 填**（作者不写它们）：
///   它们是"这一份实现在哪、链了谁、怎么装进声明"，是身份与构建的输入，不是艺术参数。
pub trait ElementFn: 'static {
    /// ⚠ **参数类型是关联类型**："用 rust 泛型"落在这里 —— 每个函数有自己的参数 struct。
    type Params: Serialize + DeserializeOwned + Default + PxKeyed;
    /// **多上游**：几个由函数自己声明（融合 = 输入多几个）。
    type Inputs: PxInputs;
    /// 人读名（`field.constant`）—— 进 `PxOp::ID`（读数与报错），**不进实例身份**。
    const NAME: &'static str;
    /// 体文件（相对 workspace 根）：**内容**进身份，路径不进。
    const SOURCE: &'static str;
    /// 体编译时链的 crate（build graph 的边）。
    const ROOTS: &'static [&'static str];
    /// `px_body!` 的体表达式（生成物逐字抄它）—— 由宏从 `NAME`/`SOURCE` 推出来。
    const BODY: &'static str;
    /// 生成的那一份库里的符号名（`px_inst__<类型名>`）。
    ///
    /// ⚠ 与 `px_cook::inst::symbol` 同一口径（生成物那句 `px_body! { <类型名>, … }` 是它的来源）
    ///   —— `tests` 里有一道判据逐条对过：符号名与库那边**不可能**漂开。
    const SYMBOL: &'static str;
    /// 输出那张场的形状：生成类从**参数**来（`params.shape`），过滤类从**上游**来。
    fn shape(params: &Self::Params, inputs: &Self::Inputs) -> Shape;
}

/// 从一个**真类型**取事实（`F` 是作者面那个类型，不是 `Elementwise<F>`）。
pub fn facts_of<F: ElementFn>() -> ElemFacts {
    ElemFacts {
        interface: interface_hash(&[
            ::core::any::type_name::<F::Params>(),
            ::core::any::type_name::<F::Inputs>(),
            ::core::any::type_name::<Field>(),
        ]),
        decl_hash: DECL_HASH,
        params: ::core::any::type_name::<F::Params>(),
        inputs: ::core::any::type_name::<F::Inputs>(),
        payload: ::core::any::type_name::<Field>(),
    }
}

/// **element 算子的唯一那条循环**（与 `px_field_alg::map_grid` 同一档）。
///
/// 逐格把 `(这一格的纹素中心坐标, 这一格的球面方向)` 交给 `cell`，形状由 [`ElementFn::shape`]
/// 给。⚠ **只有这一条循环**：预置那一档、融合那一档、每个函数 —— 全走它（"同一份参数在两条
/// 路径上算出两种结果"这类缺陷因此不可表达）。
pub fn fill<F: ElementFn>(
    params: &F::Params,
    inputs: &F::Inputs,
    cell: impl Fn(u32, u32, [f32; 2], [f32; 3]) -> f32,
) -> Field {
    let mut out = F::shape(params, inputs).filled(0.0);
    for y in 0..out.height {
        for x in 0..out.width {
            let uv = out.uv(x, y);
            let direction = out.direction(x, y);
            // ⚠ 这一格的四样东西一起给：坐标（读上游）、纹素中心（径向条带那种）、球面方向
            //   （纬向条带那种）。函数按需取，不用的写 `_`。
            out.set(x, y, cell(x, y, [uv.0, uv.1], direction));
        }
    }
    out
}

/// **规格表**：一行一个 element 函数 —— 这是"有哪些 element 函数"的**唯一**那处清单。
///
/// ```ignore
/// px_elem_specs! {
///     /// 一整张常值场。
///     Constant, ConstantParams, (), "field.constant",
///         source: "px_elem/body/constant.rs", roots: &["px_elem"],
///         shape: |params, _inputs| params.shape;
/// }
/// ```
///
/// 每行五栏：**类型名**（图脚本写 `Elementwise::<它>`）、**参数类型**、**上游类型**、
/// **人读名**，以及 `source` / `roots` / `shape` 三条。
///
/// 它一次展开出三样东西（**一处声明**）：
/// 1. 那个标记类型本身（`pub struct Constant;`）与它的 [`ElementFn`] 实现；
/// 2. [`ElemSpec`] 那一行（生成物与 `px build` 读它）；
/// 3. 事实（[`ElemFacts`]，从**真类型**取）—— 挂在 [`ElemSpec::facts`] 上（没有第二张表）。
#[macro_export]
macro_rules! px_elem_specs {
    ($(
        $(#[$meta:meta])*
        $ty:ident, $params:ty, $inputs:ty, $name:literal,
            source: $source:literal, roots: $roots:expr, shape: $shape:expr;
    )*) => {
        $(
            $(#[$meta])*
            #[derive(Debug, Clone, Copy, PartialEq, Eq)]
            pub struct $ty;

            impl $crate::ElementFn for $ty {
                type Params = $params;
                type Inputs = $inputs;
                const NAME: &'static str = $name;
                const SOURCE: &'static str = $source;
                const ROOTS: &'static [&'static str] = $roots;
                // ⚠ 生成物那句 `px_body! { <类型名>, … }` 定下符号名（`<包名>__<类型名>`），
                //   而生成的 crate 一律叫 `px_inst`（`px_cook::inst` 那一栏）——
                //   两处口径由 `tests` 里那道判据逐条对（漂开就红）。
                const SYMBOL: &'static str =
                    ::core::concat!("px_inst__", ::core::stringify!($ty));
                // ⚠ 体模板由宏推出来（人只写"哪个文件"）：逐格那条循环只有 `px_elem::fill` 一条。
                //   ⚠ 那个泛型实参写的是**全路径**（`px_elem::specs::<ty>`，那个标记类型）：
                //   生成物里那个裸名（`<ty>`）是**图侧那个算子类型**（`px_body_raw!` 的符号名，
                //   与 `fill` 要的 `ElementFn` 不是一回事）⇒ 体里走全路径，
                //   与"生成物里的 `use` 一律全路径"同一条规矩。
                const BODY: &'static str = ::core::concat!(
                    "px_elem::fill::<px_elem::specs::",
                    ::core::stringify!($ty),
                    ">(p, i, |x, y, uv, direction| value(p, i, x, y, uv, direction))"
                );
                fn shape(params: &Self::Params, inputs: &Self::Inputs) -> $crate::Shape {
                    ($shape)(params, inputs)
                }
            }
        )*

        /// **全部 element 函数**（顺序即声明顺序）。
        pub const ELEM_SPECS: &[$crate::ElemSpec] = &[
            $(
                $crate::ElemSpec {
                    ty: ::core::stringify!($ty),
                    name: $name,
                    source: $source,
                    roots: $roots,
                    body: <$ty as $crate::ElementFn>::BODY,
                    facts: || $crate::facts_of::<$ty>(),
                },
            )*
        ];
    };
}




