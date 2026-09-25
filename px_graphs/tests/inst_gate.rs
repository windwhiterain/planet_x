use px_cook::inst;
use px_cook::inst::BuildGraph;
use px_graph_schema::PxOp;
use px_graphs::insts::{Band, LatBands, Waves, build};

fn graph() -> BuildGraph {
    let mut graph = BuildGraph::new();
    build(&mut graph);
    graph
}

#[test]
fn every_recipe_row_is_in_the_graph() {
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
        let (interface, decl_hash) = px_graphs::insts::generated::facts_of(item.type_name);
        assert_ne!(interface, 0, "{} 的 interface 没生成出来", item.type_name);

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
    assert_eq!(
        inst::symbol("CloudCoarse"),
        format!("{}__CloudCoarse", inst::PACKAGE),
        "`inst::symbol` 与 `inst::PACKAGE` 的口径不一致"
    );
    assert_eq!(
        <Band as PxOp>::SYMBOL,
        inst::symbol("CloudCoarse"),
        "生成物的 `SYMBOL` 与 `inst::symbol` 拼出来的不是同一个字符串 ⇒ 装载必然找不到符号"
    );
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
    let catalogue = px_graphs::insts::codegen();
    let nodes = graph().into_nodes();
    for node in &nodes {
        let entry = catalogue
            .for_op(&node.op_id)
            .unwrap_or_else(|err| panic!("{err}"));
        assert_eq!(
            entry.source, node.source,
            "实例 `{}`：工具编的源文件与图脚本键里的不是同一个",
            node.op_id
        );
        let expected = match &entry.kind {
            px_cook::inst::InstKind::Decl { decl: _, .. } => px_graphs::insts::recipe::INSTANCES
                .iter()
                .find(|each| each.op_id == node.op_id)
                .unwrap_or_else(|| panic!("recipe 里没有 `{}` 这一条", node.op_id))
                .generated_body(),
            px_cook::inst::InstKind::Element { ty, .. } => px_elem::ELEM_SPECS
                .iter()
                .find(|each| each.ty == *ty)
                .unwrap_or_else(|| panic!("ELEM_SPECS 里没有 `{ty}` 这一条"))
                .body
                .to_string(),
        };
        assert_eq!(
            entry.body, expected,
            "实例 `{}`：工具编的体与声明那一份对不上（`px build` 会编出别的东西）",
            node.op_id
        );
    }
    assert_eq!(
        nodes.len(),
        catalogue.entries.len(),
        "build graph 里 {} 条实例、生成物里 {} 条 —— 名单不同步",
        nodes.len(),
        catalogue.entries.len(),
    );
    for (op_id, hash) in [
        ("cloud.coarse/band", Band::source_hash()),
        ("field.remap/waves", Waves::source_hash()),
        ("field.remap/latbands", LatBands::source_hash()),
    ] {
        let hash = hash.unwrap_or_else(|err| panic!("{op_id} 算不出身份：{err}"));
        assert_eq!(hash.len(), 64, "`{op_id}` 的身份不是 64 位十六进制：{hash}");
        assert!(
            hash.chars().all(|c| c.is_ascii_hexdigit()),
            "`{op_id}` 的身份不是十六进制：{hash}"
        );
        let entry = catalogue
            .for_op(op_id)
            .unwrap_or_else(|err| panic!("{err}"));
        let node = nodes
            .iter()
            .find(|node| node.op_id == op_id)
            .unwrap_or_else(|| panic!("build graph 里没有 `{op_id}`"));
        let expected = px_cook::inst::library_path(&hash);
        assert_eq!(
            std::path::Path::new(&node.library),
            expected,
            "`{op_id}` 的库路径不是按它的身份算出来的那一个"
        );
        assert_eq!(
            entry.source, node.source,
            "`{op_id}`：工具编的源与图脚本键的源不是同一份"
        );
    }
}
