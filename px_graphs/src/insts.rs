//! 图侧的**实例类型** —— 今天它们**是生成出来的**（不再是宏调用）。
//!
//! 分工（`docs/system/codegen-types.md`）：
//!
//! ```text
//! src/inst_recipe.rs   数据表（op id / 声明 / 图侧类型名 / 根 / 源文件 / 体）—— 图侧零宏
//!         ↓ cargo build 的 build.rs（stage 1 的**计划**段：不调 cargo、不编任何东西）
//! OUT_DIR/insts_gen.rs    每条实例一个**生成出来的类型**（PxOp / InstNode impl，事实为 const）
//!         ↓ include!（下面 `generated` 那一格）+ re-export
//! 本模块              stage 2 用的类型就是上面那一份
//! ```
//!
//! ⚠ **为什么生成物能进这一侧**：`PxOp` 的 `interface()` / `decl_hash()` / 三个关联类型
//!   只有**编译过类型**的那一侧算得出（`docs/system/generic-instances.md` §179.5）。从前这是"图侧写一行宏、
//!   工具问类型"；今天改成"**build script 用 `px_decls` 那张声明表**在编译图程序之前就把
//!   事实拿齐、写成 const" —— 于是图侧一个字面量都不用写，而 stage 2 拿到的仍是**类型**。
//!
//! ⚠ **改 `art/inst/*.rs` 不会让这个生成物变**（它的每一栏都不含源文件字节）：key 是运行期用
//!   `key_of_facts` 算的（要读那些字节）⇒ **图程序不重编、七个 exe 一位不动**，
//!   只有那一条实例库要重编（`docs/system/build-graph.md` §186 的 R1）。
//!
//! ⚠ **上面这条只描述"声明那一档"**：element 那一档（`px_elem` 的规格表，`Elementwise::<F>`）
//!   **图侧没有生成物**，它的规格是普通 Rust 代码（`ELEM_SPECS`）⇒ 本模块多出"照表登记节点与
//!   catalogue 条目"的一段（`build` / `codegen` 里各一个循环）。生成的 crate 本身照样有
//!   （`px build` 写 `target/jit/<key>/`、在那儿单态化）—— 省掉的只是图侧那份类型 + 事实表。
//!   两档的**身份还是同一个函数**算的（`key_of_facts`），只是表从"生成物里那串字面量"
//!   换成了"编译进图程序的常量表"。
//!   ⚠ 于是"改一行 element 算法只重编那一份库"仍成立：算法住在 `px_elem/body/*.rs`
//!   （**不在** `px_elem/src/` 下 ⇒ 不进本图程序的依赖指纹）。

/// 实例那张**数据表**（只许有类型定义 + 字面量 —— build script 会读同一份）。
///
/// ⚠ 路径基准是**这一格**（`src/`）；`build.rs` 那一侧写的是 `src/inst_recipe.rs`。
#[path = "inst_recipe.rs"]
pub mod recipe;

/// **生成物那一格**：`OUT_DIR/insts_gen.rs`。
///
/// ⚠ `include!` 而不是 `mod`：生成物住在 `OUT_DIR`，它的路径只有 build script 知道。
///   它只写"用的那些名字"的类型与 impl，不写 `super::` ⇒ 也能被测试 `include!`。
/// ⚠ 它 `include!` 一次就够（下面 re-export 那两行把类型提到本模块的名字空间里）——
///   两处 `include!` 会造出**两个** `Band`，那是两份类型、两份 impl。
pub mod generated {
    include!(concat!(env!("OUT_DIR"), "/insts_gen.rs"));
}

// ⚠ 生成物里的类型在本模块的**名字空间**里（`px_graphs::insts::Band`）——
//   图程序与测试都从这一路径取它，别多一层 `generated::` 的噪音。
pub use generated::{Band, LatBands, Waves};

/// **stage 1 的声明**（`docs/system/build-graph.md` §184/§189）：这里点名"要编哪些代码单元"。
///
/// ⚠ 事实（op id / 根 / 源文件 / 体）**只在 `inst_recipe.rs` 那一张表里**；两个只有编译过类型的
///   那一侧算得出的事实（`interface` / `decl_hash`）按 `type_name` 从**生成物**取 ——
///   生成器把它们写成了 const 字面量。
/// ⚠ **element 那一档图侧没有生成物**（内容键在运行期算，见 `px_elem` 的模块文档）：它的规格
///   （类型名 / 人读名 / 根 / 体文件 / 体模板 / 类型级事实）由 `px_elem_specs!` 展开进
///   `px_elem::ELEM_SPECS` —— 那是同一件事的另一种落法：**表是编译进图程序的**，
///   于是"生成类型"那一步不需要（类型本来就编译过）。这里照它逐条登记，与 recipe 那几条
///   走**同一个** `facts()`（两条路只有一个入口）。
/// ⚠ 加一条 element 函数 = `px_elem/src/specs.rs` 里加一行（**本函数不改**）。
pub fn build(g: &mut px_cook::inst::BuildGraph) {
    for item in recipe::INSTANCES {
        let (interface, decl_hash) = generated::facts_of(item.type_name);
        g.facts(
            item.type_name,
            item.op_id,
            interface,
            decl_hash,
            item.roots,
            item.source,
            item.body,
        );
    }
    // ⚠ op id 用**规格的人读名**（`field.constant`）：它是读数与报错用的标签，不进身份
    //   （身份 = 内容，见 `px_cook::inst::key` 里那段裁定）—— 但 `BuildGraph::facts` 要求它唯一。
    // ⚠ 根用 `px_elem::all_roots(spec)`（作者声明的 ∪ `px_elem`/`px_field_schema`）：
    //   它同时是**算键的输入**与**生成物 `[dependencies]` 的来源** ⇒ 两边各写一份就会出
    //   "键对不上"或"库引不到类型"（后者实测过一次：payload 写的 `px_field_schema::field::Field`
    //   而 manifest 里没有这个依赖）。口径只在 `px_elem::all_roots` 一处。
    for spec in px_elem::ELEM_SPECS {
        let facts = (spec.facts)();
        let roots = px_elem::all_roots(spec);
        g.facts(
            spec.ty,
            spec.name,
            facts.interface,
            facts.decl_hash,
            &roots,
            spec.source,
            spec.body,
        );
    }
}

/// 生成物的**旁挂件**：每条实例的 op id / key / 体住哪（`px build` 的错误映射与门读它）。
///
/// ⚠ 它**不参与任何 key**、也不进图程序（只在工具与测试里被 include）。
/// ⚠ 这里是**条目切片**（不是 `InstCatalogue`），**两档都在里面**（声明那一档与 element
///   那一档 —— 后者的键由生成器读体文件字节算出来，与图侧运行期算的是同一个值）。
pub mod catalogue {
    include!(concat!(env!("OUT_DIR"), "/insts_gen_catalogue.rs"));
}

/// 每条实例的生成期事实（工具与门用）—— **两档已经在生成物里合流**。
///
/// ⚠ element 那一档从前由这里在运行期现造（那要 `OnceLock` + `Box::leak` 两份字符串），
///   2026-09-27 收掉了：生成器读得到 `px_elem::ELEM_SPECS`（它是 build-dependency），
///   于是两档都由生成物出 ⇒ 这一层只剩一层转发，调用方拿到的仍是 `&'static`。
pub fn codegen() -> &'static px_cook::inst::InstCatalogue {
    static ALL: std::sync::OnceLock<px_cook::inst::InstCatalogue> = std::sync::OnceLock::new();
    ALL.get_or_init(|| px_cook::inst::InstCatalogue::from_generated(catalogue::INST_CODEGEN))
}

// ⚠ **图侧的 `px_inst` 宏调用没有了**（`docs/system/codegen-types.md`）：类型由 build.rs 生成，
//   体由 `inst_recipe.rs` 给。那条旧路径（"扫文本找宏调用、抠模板、替换占位符"）连同
//   `inst::claim` / `inst::check_template` 一起删掉了 —— 一处声明只有一个来源。
