use px_cook::{Cooked, Domain, Graph, GraphSpec, begin, cached, field_params, node_params};
use px_field_schema::field::Field;
use px_graph_schema::{PxKeyed, PxOp};
use px_graphs::elem;

fn graph() -> Graph {
    begin(GraphSpec {
        name: "bare-value".to_string(),
    })
}

fn bare(value: f32) -> Cooked<Field> {
    let params = elem::ConstantParams {
        shape: field_params::Shape {
            width: 8,
            height: 48,
            projection: Domain::CubeMap,
        },
        value,
    };
    let raw = elem::Constant
        .render(&params, &())
        .expect("直接调算子（不缓存）应当算得出来");
    Cooked::of(raw).expect("裸值应当能包成可进图的包装对象")
}

#[test]
fn a_bare_value_is_a_content_keyed_input() {
    let graph = graph();

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

    let params = || node_params::<elem::RemapParams>(&graph, "shade").expect("参数");
    let first = cached(
        &graph,
        "shade",
        elem::Remap,
        params(),
        elem::RemapInput { field: bare(0.25) },
    )
    .expect("裸值当上游应当算得出来");
    assert_eq!((first.value().width, first.value().height), (8, 48));

    let again = cached(
        &graph,
        "shade",
        elem::Remap,
        params(),
        elem::RemapInput { field: bare(0.25) },
    )
    .expect("第二次");
    assert!(again.hit, "同一份内容的裸值 ⇒ 下游必须命中");
    assert_eq!(first.key, again.key);

    let other = cached(
        &graph,
        "shade_other",
        elem::Remap,
        params(),
        elem::RemapInput { field: bare(0.5) },
    )
    .expect("换一份上游");
    assert_ne!(
        first.key, other.key,
        "上游内容变了而下游的键没变 ⇒ 上游的键没真的进下游的键"
    );

    graph.finish();
}

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
