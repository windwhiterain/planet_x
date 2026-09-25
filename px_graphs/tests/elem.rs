use px_cook::inst::{self, BuildGraph};
use px_cook::{Cooked, Domain, GraphSpec, begin, cached};
use px_elem::specs::Constant;
use px_elem::{ConstantParams, ElementFn, Shape};
use px_field_schema::field::Field;
use px_graph_schema::PxOp;
use px_graphs::elem;

fn shape() -> Shape {
    Shape {
        width: 8,
        height: 4,
        projection: Domain::Cube,
    }
}

fn flat(graph: &px_cook::Graph, node: &str, value: f32) -> Cooked<Field> {
    cached(
        graph,
        node,
        elem::Constant,
        ConstantParams {
            shape: shape(),
            value,
        },
        (),
    )
    .unwrap_or_else(|err| panic!("铺 `{node}`：{err}"))
}

#[test]
fn a_generic_element_op_runs_and_is_shared_by_two_graphs() {
    let mut instances = BuildGraph::new();
    px_graphs::insts::build(&mut instances);
    let plan = instances.missing();
    if !plan.complete() {
        let stage = instances
            .compile_missing(px_graphs::insts::codegen())
            .unwrap_or_else(|err| panic!("编 element 实例库：{err}"));
        println!("element 实例库：{}", stage.summary(0));
    }

    let shape = shape();
    let expected = 0.375_f32;
    let params = || ConstantParams {
        shape,
        value: expected,
    };
    let key = elem::key_of::<Constant>().expect("element 实例的内容键应当算得出来");
    let library = inst::library_path(&key);
    println!(
        "element 实例：key {key}｜库 {}（{} 字节）",
        library.display(),
        std::fs::metadata(&library)
            .map(|meta| meta.len())
            .unwrap_or(0),
    );

    let one = begin(GraphSpec {
        name: "elem-one".to_string(),
    });
    let cooked = cached(&one, "constant", elem::Constant, params(), ())
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

    let two = begin(GraphSpec {
        name: "elem-two".to_string(),
    });
    let again =
        cached(&two, "constant", elem::Constant, params(), ()).expect("第二个图里的同一份内容");
    assert_eq!(
        again.key, cooked.key,
        "两个图名算出了两个节点键 ⇒ 图名（或图程序自己的源码指纹）在实例身份里"
    );
    assert!(
        again.hit,
        "换一个图名算同一份内容没有命中 ⇒ 两份库/两份产物，共享那一条没成立"
    );

    let base = flat(&one, "base", -0.25);
    let other = flat(&one, "other", 0.8);
    let mask = flat(&one, "mask", 0.6);
    struct Window {
        in_min: f32,
        in_max: f32,
        out_min: f32,
        out_max: f32,
    }
    let scale = Window {
        in_min: 0.1,
        in_max: 0.9,
        out_min: -1.0,
        out_max: 2.0,
    };

    for value in [0.0_f32, -0.25, 0.8, 1.5] {
        let node = format!("const_{}", value.to_bits());
        let tile = flat(&one, &node, value);
        for y in 0..tile.value().height {
            for x in 0..tile.value().width {
                assert_eq!(
                    tile.value().at(x, y),
                    value,
                    "`elem::Constant(value = {value})` 在 ({x}, {y}) 上不是那个值"
                );
            }
        }
    }

    let mut mix_worst = 0.0_f32;
    for bias in [-1.0_f32, 0.0, 0.6] {
        let node = format!("mixed_{}", bias.to_bits());
        let mixed = cached(
            &one,
            &node,
            elem::Mix,
            elem::MixParams { bias },
            elem::MixInput {
                a: base.clone(),
                b: other.clone(),
                mask: mask.clone(),
            },
        )
        .unwrap_or_else(|err| panic!("`elem::Mix(bias = {bias})`：{err}"));
        for y in 0..mixed.value().height {
            for x in 0..mixed.value().width {
                let weight = (mask.value().at(x, y) + bias).clamp(0.0, 1.0);
                let want = base.value().at(x, y) * (1.0 - weight) + other.value().at(x, y) * weight;
                assert_eq!(
                    mixed.value().at(x, y),
                    want,
                    "`elem::Mix(bias = {bias})` 在 ({x}, {y}) 上与手算的 `a·(1−w)+b·w` 不是同一位"
                );
                mix_worst = mix_worst.max((mixed.value().at(x, y) - want).abs());
            }
        }
        println!(
            "elem::Mix bias={bias}：{}×{}｜最大 Δ {mix_worst}（逐位）",
            mixed.value().width,
            mixed.value().height
        );
    }

    let mut remap_worst = 0.0_f32;
    for (smooth, gamma) in [(false, 1.0_f32), (true, 2.5)] {
        let node = format!("remap_{}_{}", smooth as u8, gamma.to_bits());
        let remapped = cached(
            &one,
            &node,
            elem::Remap,
            elem::RemapParams {
                in_min: scale.in_min,
                in_max: scale.in_max,
                out_min: scale.out_min,
                out_max: scale.out_max,
                smooth,
                gamma,
            },
            elem::RemapInput {
                field: base.clone(),
            },
        )
        .unwrap_or_else(|err| panic!("`elem::Remap(smooth = {smooth}, gamma = {gamma})`：{err}"));
        let inv_span = {
            let span = scale.in_max - scale.in_min;
            if span.abs() < f32::EPSILON {
                0.0
            } else {
                1.0 / span
            }
        };
        for y in 0..remapped.value().height {
            for x in 0..remapped.value().width {
                let mut t = ((base.value().at(x, y) - scale.in_min) * inv_span).clamp(0.0, 1.0);
                if smooth {
                    t = t * t * (3.0 - 2.0 * t);
                }
                let lift = scale.out_min + t * (scale.out_max - scale.out_min);
                let want = if gamma > 0.0 && (gamma - 1.0).abs() >= f32::EPSILON && lift > 0.0 {
                    lift.powf(gamma)
                } else if gamma > 0.0 && (gamma - 1.0).abs() >= f32::EPSILON {
                    0.0
                } else {
                    lift
                };
                assert_eq!(
                    remapped.value().at(x, y),
                    want,
                    "`elem::Remap(smooth = {smooth}, gamma = {gamma})` 在 ({x}, {y}) 上与 \
                     手写的那份闭式算出来的不是同一位"
                );
                assert!(
                    remapped.value().at(x, y).is_finite(),
                    "`elem::Remap` 在 ({x}, {y}) 上算出了非有限值 —— `gamma` 那一步又去对\
                     负底数取幂了（`out_min = {} < 0` 时负值不弯）",
                    scale.out_min
                );
                remap_worst = remap_worst.max((remapped.value().at(x, y) - want).abs());
            }
        }
        let moved = (0..remapped.value().data.len())
            .any(|index| remapped.value().data[index] != base.value().data[index]);
        assert!(
            moved || gamma == 1.0,
            "`gamma = {gamma}` 却与上游逐格相同 ⇒ 非线性那一步没接上"
        );
        println!(
            "elem::Remap smooth={smooth} gamma={gamma}：{}×{}｜最大 Δ {remap_worst}（逐位）",
            remapped.value().width,
            remapped.value().height
        );
    }

    let fuse_params = elem::FuseParams {
        in_min: scale.in_min,
        in_max: scale.in_max,
        out_min: scale.out_min,
        out_max: scale.out_max,
        smooth: true,
        gamma: 2.5,
        bias: 0.0,
    };
    let mid = cached(
        &one,
        "fuse_mid",
        elem::Remap,
        elem::RemapParams {
            in_min: fuse_params.in_min,
            in_max: fuse_params.in_max,
            out_min: fuse_params.out_min,
            out_max: fuse_params.out_max,
            smooth: fuse_params.smooth,
            gamma: fuse_params.gamma,
        },
        elem::RemapInput {
            field: base.clone(),
        },
    )
    .unwrap_or_else(|err| panic!("融合链的中间 `elem::Remap`：{err}"));
    let two_nodes = cached(
        &one,
        "fuse_two_nodes",
        elem::Mix,
        elem::MixParams {
            bias: fuse_params.bias,
        },
        elem::MixInput {
            a: mid.clone(),
            b: other.clone(),
            mask: mask.clone(),
        },
    )
    .unwrap_or_else(|err| panic!("融合链的第二个节点 `elem::Mix`：{err}"));
    let fused = cached(
        &one,
        "fuse_one_node",
        elem::Fuse,
        fuse_params,
        elem::FuseInput {
            a: base.clone(),
            b: other,
            mask,
        },
    )
    .unwrap_or_else(|err| panic!("融合那一个节点 `elem::Fuse`：{err}"));

    let (left, right) = (fused.value(), two_nodes.value());
    let mid_path = px_cook::artifact_path_of(&mid.key);
    assert!(
        mid_path.is_file(),
        "中间场（`elem::Remap` 那一份，键 {}）不在盘上：{} ⇒ 融合的对照没有靶子",
        px_cook::hex_short(&mid.key),
        mid_path.display()
    );
    assert_eq!(
        (right.width, right.height),
        (left.width, left.height),
        "融合那条算出来的形状与两个节点那条不一致（`shape` 都取上游 `a`，不该差）"
    );
    for y in 0..left.height {
        for x in 0..left.width {
            assert_eq!(
                left.at(x, y),
                right.at(x, y),
                "({x}, {y})：`elem::Fuse` 一个节点的产物与 `elem::Remap` + `elem::Mix` 两个节点\
                 接起来的产物不是同一位 f32 —— 融合换了算法（它只该省下中间那张场）"
            );
        }
    }
    let moved = (0..left.data.len()).any(|index| left.data[index] != base.value().data[index]);
    assert!(
        moved,
        "融合那条的产物与上游 `a` 逐格相同 ⇒ 这条判据没在测融合（参数取成了恒等档？）"
    );
    println!(
        "融合：1 个节点 vs 2 个节点 ⇒ {}×{} 逐格逐位相同｜中间场（`elem::Remap` 那份）{} ms｜\
         融合 {} ms",
        left.width, left.height, mid.millis, fused.millis,
    );

    one.finish();
    two.finish();
}

#[test]
fn the_key_is_the_body_bytes_and_the_symbol_is_the_same_string() {
    let spec = px_elem::ELEM_SPECS
        .iter()
        .find(|spec| spec.ty == "Constant")
        .expect("ELEM_SPECS 里没有 `Constant` 这一条");

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

    let key = elem::key_of::<Constant>().expect("内容键");
    let by_facts = |source: &str, body: &str| {
        inst::key_of_facts(
            "",
            facts.interface,
            facts.decl_hash,
            &px_elem::all_roots(spec),
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
    edited.extend_from_slice("\n// target/ 下的副本：多这一行就该换键\n".as_bytes());
    let changed = probe.join("constant_edited.rs");
    std::fs::write(&changed, &edited).unwrap_or_else(|err| panic!("写不了副本：{err}"));
    assert_ne!(
        by_facts(&changed.to_string_lossy(), spec.body),
        key,
        "体文件字节变了而键没变 ⇒ 键没绑在体文件上（改了算法会命中旧库）"
    );

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

    assert_eq!(
        <Constant as ElementFn>::SYMBOL,
        inst::symbol(spec.ty),
        "element 函数的 `SYMBOL` 与 `inst::symbol` 拼出来的不是同一个字符串"
    );
    assert_eq!(
        <elem::Constant as PxOp>::SYMBOL,
        inst::symbol(spec.ty),
        "图侧那个算子类型报的符号名与工具那一侧不一致 ⇒ 装载必然找不到符号"
    );
    assert!(
        <elem::Constant as PxOp>::LIB.is_empty(),
        "element 算子的实现不在任何预置库里（它按内容键装载）⇒ `LIB` 必须是空串"
    );

    let roots = px_elem::all_roots(spec);
    for forbidden in [
        "px_graph",
        "px_cook",
        "px_render",
        "px-scene",
        "px_pass",
        "px_graphs",
    ] {
        assert!(
            !roots.contains(&forbidden),
            "element 实例的根里有 `{forbidden}` ⇒ 那份实例库会把驱动/烘图层链进去：{roots:?}"
        );
    }

    let _ = std::fs::remove_dir_all(&probe);
}
