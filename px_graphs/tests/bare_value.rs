//! **裸值进图**那一档：不缓存的产物（直接调算子拿到的裸值）用 `Cooked::of` 包一下就能当上游。
//!
//! 守三件事（都是 2026-09-20 那次"图里只有一个 `cached`"改出来的新性质）：
//!
//! 1. 裸值包装出来的键是**内容**键 —— 同一份内容 ⇒ 同一个键，与它是怎么来的无关；
//! 2. 它真的能当上游：下游照算、照落盘、再跑一次命中；
//! 3. **内容变了 ⇒ 下游的键就变**（键跟着内容走，不是跟着"上游是哪个节点"走）。
//!
//! ⚠ 它**不碰** `art/` 下任何既有图：自己的图名（`bare-value`），缓存落在
//! `target/pcg/bare-value/`。⚠ 形状参数取 8×4：这一篇判的是机制，不是数值。

use px_cook::{Cooked, Domain, Graph, GraphSpec, begin, cached, field, field_params, node_params};
use px_field_schema::field::Field;
use px_graph_schema::{PxKeyed, PxOp};

fn graph() -> Graph {
    begin(GraphSpec {
        name: "bare-value".to_string(),
    })
}

/// **不缓存**那一档的产物：直接调那个普通函数（`render`），再在脚本里包成可以进图的东西。
///
/// ⚠ 尺寸/投影从前由 `graph.grid()` 递进来，现在是**参数**（生成类算子的 `Shape`）。
fn bare(value: f32) -> Cooked<Field> {
    let params = field_params::constant::Params {
        shape: field_params::Shape {
            width: 8,
            height: 4,
            projection: Domain::CubeMap,
        },
        value,
    };
    let raw = field::Constant
        .render(&params, &())
        .expect("直接调算子（不缓存）应当算得出来");
    Cooked::of(raw).expect("裸值应当能包成可进图的包装对象")
}

/// ⚠ 三组断言合在**一个**测试里：一个测试二进制里的多个测试是并行的，而它们会写
/// 同一个 `target/pcg/bare-value/manifest.json` —— 分开写会变成一场竞态。
#[test]
fn a_bare_value_is_a_content_keyed_input() {
    let graph = graph();

    // 1) 键是**内容**键：同一份内容同一个键；内容不同键就不同。
    assert_eq!(
        bare(0.25).key,
        bare(0.25).key,
        "同一份内容必须是同一个键（与它是怎么来的无关）"
    );
    assert_ne!(
        bare(0.25).key,
        bare(0.5).key,
        "内容变了，裸值包装出来的键必须跟着变"
    );

    // 2) 它真的能当上游（`FieldInput` 那一栏收的就是 `Cooked<Field>`）。
    let params =
        || node_params::<px_field_schema::params::remap::Params>(&graph, "shade").expect("参数");
    let first = cached(
        &graph,
        "shade",
        field::Remap,
        params(),
        field::FieldInput { field: bare(0.25) },
    )
    .expect("裸值当上游应当算得出来");
    assert_eq!((first.value().width, first.value().height), (8, 4));

    // 3) 再算一次（**重新**造一份同样内容的裸值）⇒ 必须命中：键只由内容决定。
    let again = cached(
        &graph,
        "shade",
        field::Remap,
        params(),
        field::FieldInput { field: bare(0.25) },
    )
    .expect("第二次");
    assert!(again.hit, "同一份内容的裸值 ⇒ 下游必须命中");
    assert_eq!(first.key, again.key);

    // 4) 内容换了 ⇒ 下游的键就换（内容寻址那一半）。
    let other = cached(
        &graph,
        "shade_other",
        field::Remap,
        params(),
        field::FieldInput { field: bare(0.5) },
    )
    .expect("换一份上游");
    assert_ne!(
        first.key, other.key,
        "上游内容变了而下游的键没变 ⇒ 上游的键没真的进下游的键"
    );

    graph.finish();
}

/// **包装对象嵌进参数结构里也成立**：`HashField for Cooked<T>` 写的是它的**键**
/// ⇒ `PxKeyed` 那一套（`#[derive(PxParams)]` 按字段名 + 字段值）对 `Cooked<T>` 字段照样成立，
/// 而**递归**也就跟着成立（外面那层写的是里面那层的键，不关心里面是什么）。
///
/// ⚠ 这条**不碰盘**（纯键），所以与上面那个测试并行跑也不会打架。
#[test]
fn a_wrapper_nests_inside_a_params_struct() {
    #[derive(px_derive::PxParams)]
    struct Wrapped {
        inner: Cooked<Field>,
    }

    let key_of = |wrapped: &Wrapped| {
        let mut hasher = px_cook::blake3::Hasher::new();
        wrapped.key(&mut hasher);
        *hasher.finalize().as_bytes()
    };

    assert_eq!(
        key_of(&Wrapped { inner: bare(0.25) }),
        key_of(&Wrapped { inner: bare(0.25) }),
        "同一份内容的包装对象嵌进参数里 ⇒ 同一个键"
    );
    assert_ne!(
        key_of(&Wrapped { inner: bare(0.25) }),
        key_of(&Wrapped { inner: bare(0.5) }),
        "内容变了 ⇒ 嵌在参数里的那个键也必须变"
    );
}
