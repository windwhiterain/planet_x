//! **声明表那道门**（`docs/system/codegen-types.md`："每加一个 `px_op!` 就要在表里加一行"）。
//!
//! 三件事，缺一就红：
//!
//! 1. **表不缺**：各 schema crate（`px_decls::SCHEMAS`）的 `src/**` 里 `px_op!` 的**处数** == 表里条数；
//! 2. **表不虚**：表里每个名字都能 `decl()` 查到（且 `decl()` 只认它一个）；
//! 3. **事实是活的**：每一条的 `interface` / `decl_hash` 都非空、三个类型路径都含自己的 crate
//!    —— 于是"表里写死了一条过期的声明"这件事跑一次就看得见。
//!
//! ⚠ 数调用走的是 **`px_cook::inst_scan`**（与从前那道实例计数门**同一份**口径）：两份解析器
//!   读同一份源码却数出不同的数（实测 21 vs 1）正是从前那道门失效的原因（`19` §180）。
//! ⚠ 顺序与 `px_decls::SCHEMAS` 一致 ⇒ 数出来的处数是**声明侧**的真数。

use px_decls::{SCHEMAS, decl, entries, names, workspace_root};

/// 各 schema crate（`px_decls::SCHEMAS`）的 `src/**` 里 `px_op!` 的**处数**。
///
/// ⚠ 走的是 **`px_cook::inst_scan`**（与从前那道实例计数门**同一份**口径）：两份解析器
///   读同一份源码却数出不同的数（实测 21 vs 1）正是从前那道门失效的原因（`19` §180）。
/// ⚠ 它住在**门这一侧**（`px_cook` 只是本 crate 的 dev-dependency）：`px_decls` 的正常依赖图
///   里不许有 `px_cook` —— 那会把驱动拖进 `px_graphs/build.rs`。
fn px_op_count() -> usize {
    let root = workspace_root();
    let mut total = 0;
    for schema in SCHEMAS {
        total += px_cook::inst_scan::count_named(&root.join(schema).join("src"), "px_op!")
            .unwrap_or_else(|err| panic!("扫描器应当读得完 {schema}/src：{err}"));
    }
    total
}

#[test]
fn the_table_has_one_row_per_px_op_declaration() {
    let found = px_op_count();
    let listed = names().len();
    assert_eq!(
        found,
        listed,
        "`SCHEMAS` 那几份 schema 里有 {found} 处 `px_op!`，而 `px_decls::TABLE` 里登记了 {listed} 条\
         （差 {}）—— 每加一个声明就要在 `px_decls/src/lib.rs` 的 `TABLE` 里加一行（\
         `TABLE` 是唯一那份清单）",
        found as i64 - listed as i64
    );
}

#[test]
fn every_row_resolves_by_name() {
    let roots = entries();
    assert_eq!(
        roots.len(),
        names().len(),
        "`TABLE` 里的名字在 `decl()` 里查不到"
    );
    for name in names() {
        let facts = decl(name).unwrap_or_else(|| panic!("表里有 {name}，而 `decl({name})` 查不到"));
        // 每一条都必须能指回自己的 schema crate（生成物的 `[dependencies]` 与 `pub use` 用它）。
        assert!(
            SCHEMAS.contains(&facts.schema),
            "{name} 的 schema 是 {}，不在 {:?} 里",
            facts.schema,
            SCHEMAS
        );
        assert_eq!(
            facts.module, "ops",
            "{name} 的声明不在 `ops` 模块里（生成物的 `pub use` 会写错路径）"
        );
    }
}

#[test]
fn the_table_names_are_unique() {
    // ⚠ 单表之后新长出来的一条判据：`decl()` 找的是**第一个**同名的行 ⇒ 重名会让后面那行
    //   永远查不到（静默），而条数门照样绿（两行算两条）。各 schema 的命名空间是同一个，
    //   重名本来就是"生成器不知道该写哪个路径"。
    let mut seen: Vec<&str> = Vec::new();
    for (name, _) in entries() {
        assert!(!seen.contains(&name), "`TABLE` 里有两行同名：{name}");
        seen.push(name);
    }
}

#[test]
fn every_row_resolves_to_the_type_it_names() {
    // ⚠ 2026-09-20：这一条补的正是"配错行"那个洞 —— 条数门与"指回自己 schema"那道门都
    //   拦不住 `("Fbm", || facts::<Ridged>())`。判定只用**类型自己报的名字**，不认人写的
    //   注释、也不要第二份清单。
    for (name, facts) in entries() {
        assert_eq!(
            facts.type_name, name,
            "`px_decls::TABLE` 里 `{name}` 这一行指的是类型 `{}` ⇒ 名字与类型配错了",
            facts.type_name
        );
    }
}

#[test]
fn the_facts_are_alive() {
    for (name, facts) in entries() {
        assert_ne!(
            facts.interface, 0,
            "{name} 的 interface 是 0（没编译过类型？）"
        );
        assert!(
            !facts.decl_hash.is_empty(),
            "{name} 的 decl_hash 是空的（声明的 crate 没有 build.rs？）"
        );
        // ⚠ 载荷类型**不保证**住在自己的 schema crate 里（`VolumeData` 是
        //   `px_protocol::art` 里定义的、各 schema re-export）⇒ 那一条只做**形状**检查：
        //   生成物要照原样写 `type Payload = <这一份>;`，所以它必须是一条干净的类型路径
        //   （无空格、无泛型参数）；**它解析得开**由图程序编译保证（写错就编不过）。
        for (label, path) in [
            ("params", facts.params),
            ("inputs", facts.inputs),
            ("payload", facts.payload),
        ] {
            assert!(
                !path.is_empty() && !path.contains(' ') && !path.contains('<'),
                "{name} 的 {label} 路径 `{path}` 不是一条干净的类型路径（生成物会写不合法）"
            );
        }
        // 参数与输入是**声明自己**那一档的形状 ⇒ 它们必须住在声明所在的 schema 里。
        // ⚠ 例外：`()`（不吃上游那一档）没有 crate 可指。
        for (label, path) in [("params", facts.params), ("inputs", facts.inputs)] {
            assert!(
                path == "()" || path.contains(facts.schema),
                "{name} 的 {label} 路径 `{path}` 不在它自己的 crate `{}` 里",
                facts.schema
            );
        }
    }
}
