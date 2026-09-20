//! **生成物那两道门 + 一条端到端**（`21-codegen-types.md`）。
//!
//! 1. **图不漏**：`inst_recipe.rs` 那张表里的条数 == 图（`insts::build`）里声明的节点数
//!    —— 从前这是"数 `px_inst!` 宏调用"，今天宏调用没有了，改数**表里的条目**：
//!    生成物（`insts_gen.rs`）是按那张表生成出来的，表里加一条而生成物不认识它、
//!    或者反过来，都在这里当场红。
//! 2. **生成物与 recipe 一致**：生成物里的 `INST_TEMPLATE`（= **key 的那一轴**）必须与
//!    recipe 里那一栏**逐字相同**；`INST_BODY`（抄进实例库的那一份）必须是把 `ARG`
//!    换成 `&<类型名>` 之后的结果。对不上的话 key 会按旧模板算、复用错的构件（真缺陷的形状）。
//! 3. **端到端**：实例库在盘上时，用 `cook` 算一个**自己的图名**（`inst-op`）—— 能算、能再命中、
//!    身份就是实例 key。⚠ 库不在盘上时**只报一行、不红**（不许让没有实例库的机器红）。

use std::path::Path;

use px_cook::inst;
use px_cook::inst::BuildGraph;
use px_cook::{Cooked, Domain, GraphSpec, begin, cook, volume};
use px_field_schema::field::Field;
use px_graph_schema::PxOp;
use px_graphs::insts::{Band, Waves, build};

/// 图从 **build graph** 来（`20-build-graph.md` §190）：跑一遍 `build()`，不读任何手写清单。
fn graph() -> BuildGraph {
    let mut graph = BuildGraph::new();
    build(&mut graph);
    graph
}

#[test]
fn every_recipe_row_is_in_the_graph() {
    let listed = px_graphs::insts::recipe::INSTANCES.len();
    let in_graph = graph().instances().len();
    assert_eq!(
        listed,
        in_graph,
        "`inst_recipe.rs` 里有 {listed} 条实例，而 build graph 里声明了 {in_graph} 条\
         （差 {}）—— 每条 recipe 都要能在 `insts::build` 里登记（表里加一条就该多一条节点）",
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
fn a_real_instance_cooks_end_to_end() {
    let info = graph()
        .into_nodes()
        .into_iter()
        .next()
        .expect("build graph 里至少要有一条实例");
    let key = Band::source_hash().expect("图侧算得出实例 key");
    assert_eq!(
        Path::new(&info.library)
            .file_stem()
            .and_then(|stem| stem.to_str()),
        Some(key),
        "`info_of_facts` 给的库路径与 `source_hash()`（实例 key）不是同一个 key"
    );

    if !Path::new(&info.library).is_file() {
        eprintln!(
            "⚠ 跳过端到端：实例库不在盘上（{}）⇒ 先跑 `cargo run -p px_graphs --bin px -- build`",
            info.library
        );
        return;
    }

    // ⚠ 自己的图名（`inst-op`），不碰 `art/` 下任何既有图。
    let graph = begin(GraphSpec {
        name: "inst-op".to_string(),
        width: 8,
        height: 4,
        projection: Domain::Cube,
        cameras: Vec::new(),
    });
    // 上游那张覆盖度场：实例复用的是 `CloudCoarse` 的**声明** ⇒ 输入形状就是它那个
    // `CloudCoarseInput { coverage }`。⚠ `Band` 自己就是覆盖度的来源，这张场**不参与计算**，
    // 但接口要它在场（`19` §179.1：体逐字套在复用的声明上）。
    let coverage = Cooked::new(
        *px_cook::blake3::hash(b"inst-op/coverage").as_bytes(),
        Field::filled_with(8, 4, 1.0, Domain::Cube),
        false,
        0,
        0,
    );
    // `CloudCoarseInput` 不吃 `Clone` ⇒ 两条输入各建一次（同一份上游 ⇒ 同一个键）。
    let inputs = || volume::CloudCoarseInput {
        coverage: coverage.clone(),
    };
    let first = cook::<Band>(&graph, "band", inputs()).expect("实例算子应当能算");
    let again = cook::<Band>(&graph, "band", inputs()).expect("第二次");
    println!(
        "实例算子：{}×{}×{}｜命中={} → {}｜key {}",
        first.value().res,
        first.value().layers,
        first.value().data.len(),
        first.hit,
        again.hit,
        px_cook::hex_short(&first.key),
    );
    assert!(again.hit, "同一个节点再算一次没命中 ⇒ 实例 key 不稳定");
    assert_eq!(first.key, again.key);
    assert_eq!(
        Band::source_hash().expect("身份"),
        key,
        "`source_hash()` 两次算出来不一样"
    );
    assert!(
        !first.value().data.is_empty(),
        "实例算子算出了一份空体积"
    );
    graph.finish();
}
