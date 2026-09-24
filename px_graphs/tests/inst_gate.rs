//! **生成物那两道门 + 一条端到端**（`21-codegen-types.md`）。
//!
//! 1. **图不漏**：`inst_recipe.rs` 那张表（+ `px_elem::ELEM_SPECS` 那张表）里的条数 ==
//!    图（`insts::build`）里声明的节点数 —— 从前这是"数 `px_inst!` 宏调用"，今天宏调用没有了，
//!    改数**表里的条目**：声明那一档的生成物（`insts_gen.rs`）是按 recipe 生成出来的、
//!    element 那一档的规格是 `px_elem` 里编译进来的常量表（**图侧没有生成物**，见 `44` §8）
//!    —— 两张表里加一条而 `insts::build` 不认识它，都在这里当场红。
//! 2. **生成物与 recipe 一致**：生成物里的 `INST_TEMPLATE`（= **key 的那一轴**）必须与
//!    recipe 里那一栏**逐字相同**；`INST_BODY`（抄进实例库的那一份）必须是把 `ARG`
//!    换成 `&<类型名>` 之后的结果。对不上的话 key 会按旧模板算、复用错的构件（真缺陷的形状）。
//! 3. **库路径 ↔ 身份**：每条实例的库路径（`<key>.dll`）必须与它自己算出来的 key 是同一个
//!    —— 纯事实，不需要任何产物，任何 checkout 都断言得动。⚠ 名单从 build graph 与
//!    `codegen()` 来（**两档合流之后的那一份**），所以 element 那几条也在这里被判。
//!    ⚠ **运行那一半**（真装载、真 cached、真命中）住在 `tests/elem.rs`（element 那一档）
//!    与**探针** `--bin inst_probe`（声明那一档）：它们要 `px build` 的产物才能跑，
//!    而测试不该替构建产物负责。搬出去之前那一段在库里写着
//!    "库不在盘上就打印一行、然后 return" —— 那就是"跳过"，本仓不吃这一套。
//!    ⚠ element 那一档留在 `tests/elem.rs` 里的理由是**它自己会编**（`missing()` +
//!    `compile_missing()`，缺才编、一次约 2 秒）—— 声明那一档编一次要几十秒，所以只在探针里。

use std::path::Path;

use px_cook::inst;
use px_cook::inst::BuildGraph;
use px_graph_schema::PxOp;
use px_graphs::insts::{Band, LatBands, Waves, build};

/// 图从 **build graph** 来（`20-build-graph.md` §190）：跑一遍 `build()`，不读任何手写清单。
fn graph() -> BuildGraph {
    let mut graph = BuildGraph::new();
    build(&mut graph);
    graph
}

#[test]
fn every_recipe_row_is_in_the_graph() {
    // ⚠ 两张表**都数**：声明那一档（recipe）与 element 那一档（`px_elem::ELEM_SPECS`）。
    //   等式仍然是等式（不是"至少"）—— 少一条、多一条都当场红。
    let listed = px_graphs::insts::recipe::INSTANCES.len() + px_elem::ELEM_SPECS.len();
    let in_graph = graph().instances().len();
    assert_eq!(
        listed,
        in_graph,
        "`inst_recipe.rs` 与 `px_elem::ELEM_SPECS` 里一共 {listed} 条实例，而 build graph 里\
         声明了 {in_graph} 条（差 {}）—— 两张表都要能在 `insts::build` 里登记\
         （哪张表加一条就该多一条节点）",
        listed as i64 - in_graph as i64
    );
}

#[test]
fn the_generated_body_is_the_substituted_recipe_body() {
    for item in px_graphs::insts::recipe::INSTANCES {
        // 生成物里那两个常量（按图侧类型名取，`insts.rs` 的 `generated` 那一格）。
        let (interface, decl_hash) = px_graphs::insts::generated::facts_of(item.type_name);
        assert_ne!(interface, 0, "{} 的 interface 没生成出来", item.type_name);

        // 体：`ARG` → `&<类型名>`（**整词**替换），其余逐字。
        let expected = item.generated_body();
        assert!(
            expected.contains(&format!("&{}", item.type_name)),
            "{} 的体里没有真类型名：{expected}",
            item.type_name
        );
        assert!(
            !expected.contains("ARG"),
            "{} 的体里还留着占位符（生成物编不过）：{expected}",
            item.type_name
        );

        // 身份：实例 key 由**生成物里的那两个事实 + recipe 其余几栏**算出来。
        let key = inst::key_of_facts(
            item.op_id,
            interface,
            decl_hash,
            item.roots,
            item.source,
            item.body,
        )
        .expect("实例 key 应当算得出来（源文件在盘上）");
        assert_eq!(
            key.len(),
            64,
            "{} 的 key 不是 64 位十六进制：{key}",
            item.type_name
        );
    }
}

#[test]
fn the_symbol_name_is_the_same_string_on_both_sides() {
    // 工具那一侧（`inst::symbol` 照它拼生成的 crate 的包名）。
    assert_eq!(
        inst::symbol("CloudCoarse"),
        format!("{}__CloudCoarse", inst::PACKAGE),
        "`inst::symbol` 与 `inst::PACKAGE` 的口径不一致"
    );
    // 图侧生成出来的那个常量（装载时就是照它去 DLL 里找）。
    assert_eq!(
        <Band as PxOp>::SYMBOL,
        inst::symbol("CloudCoarse"),
        "生成物的 `SYMBOL` 与 `inst::symbol` 拼出来的不是同一个字符串 ⇒ 装载必然找不到符号"
    );
    // ⚠ 第二条实例（场域那条）**复用同一个声明**（`FieldRemap`）—— 它的 `SYMBOL` 与
    //   `Band` 那条是**同一个字符串**（符号名只由"包名 + 声明名"决定，与图侧类型名无关）。
    assert_eq!(
        <Waves as PxOp>::SYMBOL,
        inst::symbol("FieldRemap"),
        "复用声明的实例拼出来的符号名与工具那一侧不一致 ⇒ 装载必然找不到符号"
    );
    assert!(
        <Band as PxOp>::LIB.is_empty() && <Waves as PxOp>::LIB.is_empty(),
        "实例库是运行期按 key 装载的 ⇒ `LIB` 必须是空串（见 `19` §179.1）"
    );
}

#[test]
fn every_instance_library_path_is_the_key_the_generator_planned() {
    // ⚠ 这条测试**只判计划那一半**（纯事实，不需要任何产物）。运行那一半（真装载、真 cached、
    //   真命中）搬进了**探针** `--bin inst_probe`：它要 `px build` 烘出来的库才能跑，而那是
    //   **构建产物**，`cargo test` 不保证它在盘上、也不该替它去编。
    //   搬出去之前那一段写的是"**库不在盘上就打印一行、然后 return**" —— 那就是"跳过"，
    //   而本仓的不变式是"任何『跳过』都是判据的敌人"（一条可能什么都不查的判据比没有更坏）。
    //   现在两半**各自都硬**：这一半任何 checkout 都断言得了、且一定会断言；那一半缺库就报错并给命令。
    //
    // ⚠ 这里**不写手维护的实例清单**：名单从 build graph 与生成物（`codegen()`）来，
    //   两边的 op id 集合必须一模一样（多一条、少一条都当场红）。
    let catalogue = px_graphs::insts::codegen();
    let nodes = graph().into_nodes();
    for node in &nodes {
        let entry = catalogue
            .for_op(&node.op_id)
            .unwrap_or_else(|err| panic!("{err}"));
        assert_eq!(
            Path::new(&node.library)
                .file_stem()
                .and_then(|stem| stem.to_str()),
            Some(entry.key),
            "实例 `{}` 的库路径不是生成器算出来的那个 key（计划的两半分家了）",
            node.op_id,
        );
    }
    assert_eq!(
        nodes.len(),
        catalogue.entries.len(),
        "build graph 里 {} 条实例、生成物里 {} 条 —— 名单不同步",
        nodes.len(),
        catalogue.entries.len(),
    );
    // 逐类型再钉一遍"类型自己报的身份"（生成物的 const 与 `source_hash()` 是两条路）。
    for (op_id, hash) in [
        ("cloud.coarse/band", Band::source_hash()),
        ("field.remap/waves", Waves::source_hash()),
        ("field.remap/latbands", LatBands::source_hash()),
    ] {
        let entry = catalogue
            .for_op(op_id)
            .unwrap_or_else(|err| panic!("{err}"));
        assert_eq!(
            hash.as_deref(),
            Ok(entry.key),
            "`{op_id}` 的类型自报身份与生成器算的不是同一个"
        );
    }
}
