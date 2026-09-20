//! 图侧的**实例类型** —— 今天它们**是生成出来的**（不再是宏调用）。
//!
//! 分工（`.agents/notes/art/21-codegen-types.md`）：
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
//!   只有**编译过类型**的那一侧算得出（`19-generic-inst.md` §179.5）。从前这是"图侧写一行宏、
//!   工具问类型"；今天改成"**build script 用 `px_decls` 那张声明表**在编译图程序之前就把
//!   事实拿齐、写成 const" —— 于是图侧一个字面量都不用写，而 stage 2 拿到的仍是**类型**。
//!
//! ⚠ **改 `art/inst/*.rs` 不会让这个生成物变**（它的每一栏都不含源文件字节）：key 是运行期用
//!   `key_of_facts` 算的（要读那些字节）⇒ **图程序不重编、七个 exe 一位不动**，
//!   只有那一条实例库要重编（`20-build-graph.md` §186 的 R1）。

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

/// **stage 1 的声明**（`20-build-graph.md` §184/§189）：这里点名"要编哪些代码单元"。
///
/// ⚠ 事实（op id / 根 / 源文件 / 体）**只在 `inst_recipe.rs` 那一张表里**；两个只有编译过类型的
///   那一侧算得出的事实（`interface` / `decl_hash`）按 `type_name` 从**生成物**取 ——
///   生成器把它们写成了 const 字面量。加一条实例 = 表里加一条（**本函数不改**）。
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
}

/// 生成物的**旁挂件**：每条实例的 op id / key / 体住哪（`px build` 的错误映射与门读它）。
///
/// ⚠ 它**不参与任何 key**、也不进图程序（只在工具与测试里被 include）。
pub mod catalogue {
    include!(concat!(env!("OUT_DIR"), "/insts_gen_catalogue.rs"));
}

/// 每条实例的生成期事实（工具与门用）。
pub fn codegen() -> &'static px_cook::inst::InstCatalogue {
    &catalogue::INST_CODEGEN
}

// ⚠ **图侧的 `px_inst` 宏调用没有了**（`21-codegen-types.md`）：类型由 build.rs 生成，
//   体由 `inst_recipe.rs` 给。那条旧路径（"扫文本找宏调用、抠模板、替换占位符"）连同
//   `inst::claim` / `inst::check_template` 一起删掉了 —— 一处声明只有一个来源。
