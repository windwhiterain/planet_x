use px_cook::{Domain, Graph, GraphSpec, begin, cached, field_params, node_params, px_local_op};
use px_field_schema::field::Field;
use px_graph_schema::PxOp;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
#[serde(default, deny_unknown_fields)]
struct BandParams {
    frequency: f32,
    gain: f32,
    axis: u32,
    shape: field_params::Shape,
}

impl Default for BandParams {
    fn default() -> Self {
        Self {
            frequency: 6.0,
            gain: 1.0,
            axis: 2,
            shape: field_params::Shape::default(),
        }
    }
}

fn bake<F: Fn([f32; 3]) -> f32>(shape: field_params::Shape, sample: F) -> Field {
    let mut field = shape.filled(0.0);
    for y in 0..shape.height {
        for x in 0..shape.width {
            field.set(x, y, sample(field.direction(x, y)));
        }
    }
    field
}

struct Band;

px_local_op! { Band, "local.band", BandParams, (), Field, |p, _i| bake(p.shape, |d| {
    let axis = (p.axis as usize).min(2);
    ((d[axis] * p.frequency).sin() * 0.5 + 0.5) * p.gain
}) }

struct Rings;

px_local_op! { Rings, "local.rings", BandParams, (), Field, |p, _i| bake(p.shape, |d| {
    let radial = (d[0] * d[0] + d[1] * d[1]).sqrt();
    ((radial * p.frequency).cos() * 0.5 + 0.5) * p.gain
}) }

fn graph() -> Graph {
    begin(GraphSpec {
        name: "local-op".to_string(),
    })
}

#[test]
fn a_graph_local_operator_is_a_first_class_operator() {
    let shape = field_params::Shape {
        width: 8,
        height: 48,
        projection: Domain::CubeMap,
    };
    let one = bake(shape, |_| 1.0).stats().mean;
    let two = bake(shape, |_| 2.0).stats().mean;
    assert!(
        (one - 1.0).abs() < 1e-6 && (two - 2.0).abs() < 1e-6,
        "同一份泛型函数对两个闭包算出 {one:.4} / {two:.4} ⇒ 泛型实例没被真的分开"
    );

    let graph = graph();

    let band = cached(
        &graph,
        "band",
        Band,
        BandParams {
            shape,
            ..node_params(&graph, "band")
                .expect("参数（这张图没有 art/local-op/band.toml ⇒ 走 Default）")
        },
        (),
    )
    .expect("现写的算子应当能算");
    let value = band.value();
    assert_eq!((value.width, value.height), (8, 48));
    let stats = value.stats();
    println!(
        "图侧算子：{}×{}｜值域 {:.4}..{:.4}｜均值 {:.4}（命中={}）",
        value.width, value.height, stats.min, stats.max, stats.mean, band.hit
    );
    assert!(
        stats.max - stats.min > 1e-3,
        "现写的算子算出了常数场 ⇒ 闭包没被真的调用"
    );

    let hash = Band::source_hash().expect("图侧算子应当有身份");
    assert!(Band::LIB.is_empty(), "图侧算子不该宣称自己住在一个实现库里");
    assert_eq!(
        hash,
        px_graphs::SOURCE_HASH,
        "图侧算子的身份必须是本图程序这一份源码的指纹"
    );
    assert_eq!(hash.len(), 64);

    let rings = cached(
        &graph,
        "rings",
        Rings,
        BandParams {
            shape,
            ..node_params(&graph, "rings").expect("参数")
        },
        (),
    )
    .expect("第二个现写算子");
    assert_ne!(band.key, rings.key, "两个现写算子算出同一个键 ⇒ id 没进键");
    let again = cached(
        &graph,
        "band",
        Band,
        BandParams {
            shape,
            ..node_params(&graph, "band").expect("参数")
        },
        (),
    )
    .expect("第二次");
    assert!(
        again.hit,
        "同一个节点再算一次没命中 ⇒ 键不稳定（现写算子的身份没钉住）"
    );
    assert_eq!(band.key, again.key);

    graph.finish();
}
