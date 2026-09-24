//! **element 那一档端到端**：`Elementwise::<Constant>` 真编库、真装载、真算出来。
//!
//! 这一篇钉四件事（"一个泛型 element-wise 算子在图脚本里单态化成任意 element-wise 算子"
//! 那条裁定落地的判据）：
//!
//! 1. **端到端**：缺库就编（只编缺的那几条）、`cached` 算得出、逐格等于参数里的值；
//! 2. **跨图共享**：同一个规格 + 同一份参数，在两个**不同图名**的图里算出**同一个节点键**
//!    （"实例身份 = 内容"：键里没有图名、也没有图程序自己的源码指纹）；
//! 3. **内容身份**：键 == `key_of_facts("", …)` 的返回值，且它绑在**体文件字节**上；
//! 4. **符号名口径**：`<Constant as ElementFn>::SYMBOL == inst::symbol(spec.ty)`
//!    —— 生成物那句 `px_body!` 定的符号与装载器拼的必须是同一个字符串。
//!
//! ⚠ 第 1、2 条要**真的编一次库**（约 2 秒）：走 `missing()`/`compile_missing()`（与
//!   `px build` 同一对函数）⇒ `px build` 编过就一条都不编，缺才编。
//! ⚠ 这个二进制里所有**碰图**的断言合在**一个**测试里：多个测试是并行的，而它们会各自写
//!   `target/pcg/<图名>/manifest.json`（本仓那条老规矩：并行写同一份清单就是一场竞态）。
//! ⚠ 判据**不许依赖缓存是空的**（CAS 是持久的、测试会重跑）：所以判"键稳定 ⇒ 第二次必命中"，
//!   不判"第一次一定重算"。

use px_cook::inst::{self, BuildGraph};
use px_cook::{Domain, GraphSpec, begin, cached};
use px_elem::specs::Constant;
use px_elem::{ConstantParams, ElementFn, Elementwise, Shape};
use px_graph_schema::PxOp;

/// 端到端 + 跨图共享（共用一份库，所以合在一个测试里 —— 见文件头那条并行规矩）。
#[test]
fn a_generic_element_op_runs_and_is_shared_by_two_graphs() {
    // ① 那一份实例库在盘上（**缺才编**：`px build` 刚跑过就一条都不编）。
    let mut instances = BuildGraph::new();
    px_graphs::insts::build(&mut instances);
    let plan = instances.missing();
    if !plan.complete() {
        let stage = instances
            .compile_missing(px_graphs::insts::codegen())
            .unwrap_or_else(|err| panic!("编 element 实例库：{err}"));
        println!("element 实例库：{}", stage.summary(0));
    }

    let shape = Shape {
        width: 8,
        height: 4,
        projection: Domain::Cube,
    };
    let expected = 0.375_f32;
    // ⚠ 参数是一份**普通值**（`ConstantParams` 是这个函数自己的类型，不是"三个 float 的口袋"）。
    let params = || ConstantParams {
        shape,
        value: expected,
    };
    let key = px_elem::key_of::<Constant>().expect("element 实例的内容键应当算得出来");
    let library = inst::library_path(&key);
    println!(
        "element 实例：key {key}｜库 {}（{} 字节）",
        library.display(),
        std::fs::metadata(&library).map(|meta| meta.len()).unwrap_or(0),
    );

    // ② 端到端：真装载那份库、真算出一张场。
    let one = begin(GraphSpec {
        name: "elem-one".to_string(),
    });
    let cooked = cached(
        &one,
        "constant",
        Elementwise::<Constant>,
        params(),
        (),
    )
    .expect("element 实例应当能算（库不在盘上时这里会带命令报错）");
    let field = cooked.value();
    assert_eq!(
        (field.width, field.height),
        (shape.width, shape.height),
        "算出来的场尺寸与参数里的 `shape` 不一致"
    );
    assert_eq!(
        field.projection, shape.projection,
        "算出来的投影与参数里的 `shape` 不一致"
    );
    for y in 0..field.height {
        for x in 0..field.width {
            assert_eq!(
                field.at(x, y),
                expected,
                "({x}, {y}) 那一格不是参数里那个值 ⇒ 体文件没按参数算"
            );
        }
    }
    println!(
        "element 实例：{}×{}｜节点键 {}｜命中={}｜{} ms",
        field.width,
        field.height,
        px_cook::hex_short(&cooked.key),
        cooked.hit,
        cooked.millis,
    );
    one.finish();

    // ③ **跨图共享**：另一个图名、同一份规格与参数 ⇒ 同一个节点键，而且**必命中**
    //    （同一个键 ⇒ 同一份 CAS 产物 —— 这就是"两个图用同一个函数共享"那句话的读数）。
    let two = begin(GraphSpec {
        name: "elem-two".to_string(),
    });
    let again = cached(&two, "constant", Elementwise::<Constant>, params(), ())
        .expect("第二个图里的同一份内容");
    assert_eq!(
        again.key, cooked.key,
        "两个图名算出了两个节点键 ⇒ 图名（或图程序自己的源码指纹）还在实例身份里"
    );
    assert!(
        again.hit,
        "换一个图名算同一份内容没有命中 ⇒ 两份库/两份产物，共享那一条没成立"
    );
    two.finish();
}

/// 内容身份 + 符号名口径（纯事实：不碰盘上的实例库、不碰图，也不动工作树里那个体文件）。
#[test]
fn the_key_is_the_body_bytes_and_the_symbol_is_the_same_string() {
    let spec = px_elem::ELEM_SPECS
        .iter()
        .find(|spec| spec.ty == "Constant")
        .expect("ELEM_SPECS 里没有 `Constant` 这一条");

    // ⚠ 类型自己那一份事实与规格表那一份必须是同一份：两处来源（`facts_of::<F>()` 与
    //   `ElemSpec::facts`），一个口径 —— 漂开的话图脚本装载用的键与 `px build` 编的库不是一个。
    let facts = (spec.facts)();
    assert_eq!(
        facts,
        px_elem::facts_of::<Constant>(),
        "规格表里的类型级事实与类型自己报的不是同一份"
    );
    assert_eq!(
        facts.decl_hash,
        px_elem::DECL_HASH,
        "事实里的声明指纹不是 `px_elem` 那一份（改参数 struct 就不会换键了）"
    );

    // **同一个函数、同一个输入**：图脚本（`px_elem::key_of`）与这里（显式事实）算出来的键。
    let key = px_elem::key_of::<Constant>().expect("内容键");
    let by_facts = |source: &str, body: &str| {
        inst::key_of_facts(
            // ⚠ 空 op id：手写名不进身份（"实例身份 = 内容"）—— 与 `px_elem::key_of` 同参。
            "",
            facts.interface,
            facts.decl_hash,
            spec.roots,
            source,
            body,
        )
        .unwrap_or_else(|err| panic!("算键：{err}"))
    };
    assert_eq!(
        by_facts(spec.source, spec.body),
        key,
        "`px_elem::key_of` 与按显式事实算的不是同一个键（两处各写了一份算法？）"
    );
    assert_eq!(key.len(), 64, "键不是 64 位十六进制：{key}");
    assert!(
        key.chars().all(|c| c.is_ascii_hexdigit()),
        "键不是十六进制：{key}"
    );

    // ① **键绑在体文件字节上** —— 而**不**真的动工作树里那个体文件（那是要被审的东西）：
    //    同一份字节抄到另一个路径 ⇒ 还是同一个键（路径不进身份）；改一行 ⇒ 换键。
    //    ⚠ 副本住在 `target/` 下：本仓"禁止往项目外写临时文件"那条，而 `target/` 本来就可删。
    let root = px_cook::workspace_root();
    let probe = root.join("target/elem_key_probe");
    std::fs::create_dir_all(&probe)
        .unwrap_or_else(|err| panic!("建不了 {}：{err}", probe.display()));
    let bytes = std::fs::read(root.join(spec.source))
        .unwrap_or_else(|err| panic!("读不了体文件 {}：{err}", spec.source));
    assert!(!bytes.is_empty(), "体文件 {} 是空的", spec.source);

    let copy = probe.join("constant_same.rs");
    std::fs::write(&copy, &bytes).unwrap_or_else(|err| panic!("写不了副本：{err}"));
    assert_eq!(
        by_facts(&copy.to_string_lossy(), spec.body),
        key,
        "同一份字节、另一个路径就算出另一个键 ⇒ 路径进了身份（换一个 checkout 就会全部换键）"
    );

    let mut edited = bytes.clone();
    edited.extend_from_slice(b"\n// target/ 下的副本：多这一行就该换键\n");
    let changed = probe.join("constant_edited.rs");
    std::fs::write(&changed, &edited).unwrap_or_else(|err| panic!("写不了副本：{err}"));
    assert_ne!(
        by_facts(&changed.to_string_lossy(), spec.body),
        key,
        "体文件字节变了而键没变 ⇒ 键没绑在体文件上（改了算法会命中旧库）"
    );

    // ② 体**模板**也是身份的一轴。⚠ 去空白只去空白 ⇒ 多一个空格**不**该换键（那是归一的口径），
    //    多一个真 token 才该换。
    assert_eq!(
        by_facts(spec.source, &format!("{} ", spec.body)),
        key,
        "模板只多了一个空白 ⇒ 键不该变（`normalize_template` 那一套没生效？）"
    );
    assert_ne!(
        by_facts(spec.source, &format!("{} // 多一个 token", spec.body)),
        key,
        "模板换了而键没变 ⇒ 模板没进身份（同一个函数两种装法会复用同一份库）"
    );

    // ③ **符号名口径**：生成物那句 `px_body! { <ty>, … }` 定下的符号（`<包名>__<ty>`）与
    //    装载器手里的 `SYMBOL` 必须是同一个字符串。⚠ 两处是**各自拼**的（宏里一次、
    //    `inst::symbol` 一次）⇒ 只有判据钉得住；漂开的症状是"库编出来了、装载找不到符号"。
    assert_eq!(
        <Constant as ElementFn>::SYMBOL,
        inst::symbol(spec.ty),
        "element 函数的 `SYMBOL` 与 `inst::symbol` 拼出来的不是同一个字符串"
    );
    assert_eq!(
        <Elementwise<Constant> as PxOp>::SYMBOL,
        inst::symbol(spec.ty),
        "`Elementwise<F>` 报的符号名与工具那一侧不一致 ⇒ 装载必然找不到符号"
    );
    assert!(
        <Elementwise<Constant> as PxOp>::LIB.is_empty(),
        "element 算子的实现不在任何预置库里（它按内容键装载）⇒ `LIB` 必须是空串"
    );

    // 副本用完就收：`target/` 下不留垃圾（断言失败时留着 —— 那正好是现场）。
    let _ = std::fs::remove_dir_all(&probe);
}
