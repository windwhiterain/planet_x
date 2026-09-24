//! **element 那一档端到端**：`Elementwise::<Constant>` 真编库、真装载、真算出来。
//!
//! 这一篇钉五件事（"一个泛型 element-wise 算子在图脚本里单态化成任意 element-wise 算子"
//! 那条裁定落地的判据）：
//!
//! 1. **端到端**：缺库就编（只编缺的那几条）、`cached` 算得出、逐格等于参数里的值；
//! 2. **跨图共享**：同一个规格 + 同一份参数，在两个**不同图名**的图里算出**同一个节点键**
//!    （"实例身份 = 内容"：键里没有图名、也没有图程序自己的源码指纹）；
//! 3. **内容身份**：键 == `key_of_facts("", …)` 的返回值，且它绑在**体文件字节**上；
//! 4. **符号名口径**：`<Constant as ElementFn>::SYMBOL == inst::symbol(spec.ty)`
//!    —— 生成物那句 `px_body!` 定的符号与装载器拼的必须是同一个字符串。
//! 5. **三条迁移过来的数值判据 + 融合那条**（2026-09-27 收掉预置 `constant`/`mix`/`remap`
//!    时把它们原件里的东西搬到这一档，一条都没丢）：`Constant` 逐格等于 `value`；
//!    `Mix` 在 `bias` 各档上等于手算的 `a·(1−w)+b·w`（含 `w` 被钳到 `[0,1]` 的两端）；
//!    `Remap` 在 `gamma` 各档上等于**判据自己手写的那份闭式**；
//!    以及**融合**—— `elem::Fuse` 一个节点的产物与"先 `elem::Remap` 再 `elem::Mix`"两个节点
//!    接起来的产物**逐格逐位相同**（`assert_eq!` 到 f32 位）。
//!
//! ⚠ 第 1、2、5 条要**真的编一次库**（实测从零编一份约 3 秒）：走 `missing()`/`compile_missing()`
//!   （与 `px build` 同一对函数）⇒ `px build` 编过就一条都不编，缺才编。
//! ⚠⚠ 这个二进制里所有**碰图**的断言合在**一个**测试里（第 1、2、5 条都在
//!   `a_generic_element_op_runs_and_is_shared_by_two_graphs` 里）：多个测试是并行的，而它们
//!   会各自写 `target/pcg/<图名>/manifest.json`（本仓那条老规矩：并行写同一份清单就是一场竞态）。
//! ⚠ 判据**不许依赖缓存是空的**（CAS 是持久的、测试会重跑）：所以判"键稳定 ⇒ 第二次必命中"，
//!   不判"第一次一定重算"。
//! ⚠ 判据**不许放宽容差**：手算那一侧是**独立手写的闭式**（不借实现那一侧的算术），
//!   于是 `assert_eq!`（逐位）成立；融合那一条更是**必须**逐位 —— 它的全部卖点就是
//!   "少一张中间场、算法一个字不换"。

use px_cook::inst::{self, BuildGraph};
use px_cook::{Cooked, Domain, GraphSpec, begin, cached};
use px_elem::specs::Constant;
use px_elem::{ConstantParams, ElementFn, Shape};
use px_field_schema::field::Field;
use px_graph_schema::PxOp;
use px_graphs::elem;

/// 判据用的小形状（8×4）：这一篇判的是**算法接没接对**，不是数值规模。
fn shape() -> Shape {
    Shape {
        width: 8,
        height: 4,
        projection: Domain::Cube,
    }
}

/// 一张可复用的上游：`elem::Constant` 铺一个值（形状由它自己那份参数给）。
///
/// ⚠ 上游走的是**图里唯一那个 `cached`**（没有第二条造场的路）。
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

/// 端到端 + 跨图共享 + 三条数值判据 + 融合（共用一份库，所以合在一个测试里 —— 见文件头）。
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

    let shape = shape();
    let expected = 0.375_f32;
    // ⚠ 参数是一份**普通值**（`ConstantParams` 是这个函数自己的类型，不是"三个 float 的口袋"）。
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

    // ② 端到端：真装载那份库、真算出一张场。
    //    ⚠ 写法是 `Elementwise::<Constant>::new()`（`PxOp::new()`）而**不是**裸的
    //    `Elementwise::<Constant>`：后者是个带 `PhantomData` 字段的元组结构体、字段私有
    //    ⇒ 图脚本**写不出**一个值来（这条手感差别记在 `px_elem` 的模块文档里）。
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

    // ③ **跨图共享**：另一个图名、同一份规格与参数 ⇒ 同一个节点键，而且**必命中**
    //    （同一个键 ⇒ 同一份 CAS 产物 —— 这就是"两个图用同一个函数共享"那句话的读数）。
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

    // ④ **数值判据**（迁移自被删掉的预置实现那一档；参数在这一侧显式给，不让 TOML 决定）。
    //
    //    ⚠ 上游都从 `elem::Constant` 来（同一张 `Cube` 投影的 8×4 场），三张互不相同。
    let base = flat(&one, "base", -0.25);
    let other = flat(&one, "other", 0.8);
    let mask = flat(&one, "mask", 0.6);
    // ⚠ 这一组数是**判据自己写的**（`in_min`/`out_min` 特意含"负的下界"那一档：
    //   它正是 `powf` 那个 NaN 缺陷的现场）。
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

    // ④a `Constant`：逐格等于 `value`（② 那一格就是这个；这里再走一遍**另一份**参数，
    //     免得"只有 0.375 那个值对得上"这种巧合）。
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

    // ④b `Mix`：`a·(1−w) + b·w`，`w = clamp(mask + bias, 0, 1)`。
    //     ⚠ `bias` 三档都要过 `clamp` 的**两端**（−1 ⇒ w=0 全取 a；+0.6 ⇒ mask+bias=1.2
    //     ⇒ w=1 全取 b）；0 那一档是中间的普通插值。
    //     ⚠ 算术**逐字照抄体文件那一行**（先 `(mask + bias).clamp(0,1)`，再 `a*(1-w)+b*w`）
    //     ⇒ 下面的 `assert_eq!` 是**逐位**的，不是"差不多"。
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

    // ④c `Remap`：钳 → 归一化（可选平滑）→ 映值域 → 按 `gamma` 弯。
    //     ⚠ `gamma` 那一口径是"非正数不弯"（`px_elem/body/remap.rs` 与 `px_field_alg` 同一句话）；
    //     这里 `gamma = 1.0` 是**恒等档**、`2.5` 是**真弯**那一档（`out_max = 2` ⇒ 映出来的值
    //     可能是负的 ⇒ 负底数不进 `powf`，口径与 `params::bend` 同一条）。
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
        // ⚠ 手算那一侧是**独立写的一份闭式**（不是去调 `px_field_alg::Scale`）：拿实现自己
        //   当 oracle 抓不出实现错 —— `powf` 那个"只判指数不判底数"的缺陷就是这么露头的
        //   （它靠的是另一条路径的对照）。公式按**同一个运算次序**写，好让"逐位相等"这条
        //   硬判据成立：`t = clamp((v - in_min)·inv_span)`（可选 smoothstep）→
        //   `out_min + t·(out_max - out_min)` → 非正数不弯。
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
        // ⚠ 顺带钉住这条判据不是空转：`gamma = 1` 是恒等档（不许弯），`gamma > 1` 必须真的弯了
        //   ——两个档的产物只要不同就说明 `gamma` 真的接上了（"六面都一样"那种 bug 才看不出来）。
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

    // ⑤ **融合那条**（用户裁定"还能融合算子"的落点）：`elem::Fuse` 一个节点 **与**
    //    "先 `elem::Remap` 再 `elem::Mix`"两个节点接起来，在**同一份上游与参数**下必须
    //    **逐格逐位相同**。
    //    ⚠ 两个节点那一条的中间产物就是 `Mix` 的 `a`（`elem::Remap` 的输出是 `Cooked<Field>`
    //    ⇒ 直接喂给下一个节点，没有"再包一层"这种中间步骤）。
    //    ⚠ 这一条**真跑 dylib**：两个节点那条走**两份**实例库（`Remap` 一份、`Mix` 一份），
    //    融合那条走**第三份**（`Fuse` 自己）—— 三份都在 ① 那一步的同一张表里。
    let fuse_params = elem::FuseParams {
        in_min: scale.in_min,
        in_max: scale.in_max,
        out_min: scale.out_min,
        out_max: scale.out_max,
        // ⚠ 平滑那一档取 `true`：融合与两个节点在这条路上都要经过 smoothstep。
        smooth: true,
        gamma: 2.5,
        // ⚠ 取 **0.0**（不取 ④b 里那个 0.6）：`mask + 0.6` 会被钳到 1 ⇒ 输出就是 `b` 那张
        //    常数场，融合的其余部分全被盖住。这一条判据要的是**两边真的在插值**。
        bias: 0.0,
    };
    // 中间产物（两个节点那条的 `a`）。
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
    // ⚠ 先钉住"这条链真的算过中间那一张场"：`mid` 是 `elem::Remap` 的输出喂给 `Mix` 的 `a`，
    //   它必须**在盘上、且与那个节点的键对得上** —— 否则下面那个循环比的是两份都还没算的东西。
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
    // 顺带钉住"这条判据不是空转"：两个节点那条的中间场与融合那条的输入 `a` 出自同一份上游，
    // 而融合的产物必须真的**动过**值（`gamma` / `bias` 都不取恒等档）。
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
    let key = elem::key_of::<Constant>().expect("内容键");
    let by_facts = |source: &str, body: &str| {
        inst::key_of_facts(
            // ⚠ 空 op id：手写名不进身份（"实例身份 = 内容"）—— 与 `px_elem::key_of` 同参。
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
    edited.extend_from_slice("\n// target/ 下的副本：多这一行就该换键\n".as_bytes());
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
        <elem::Constant as PxOp>::SYMBOL,
        inst::symbol(spec.ty),
        "图侧那个算子类型报的符号名与工具那一侧不一致 ⇒ 装载必然找不到符号"
    );
    assert!(
        <elem::Constant as PxOp>::LIB.is_empty(),
        "element 算子的实现不在任何预置库里（它按内容键装载）⇒ `LIB` 必须是空串"
    );

    // ④ **算法那一侧不许认识驱动**（2026-09-27 的尺寸问题）：`roots` 就是那份实例库编译时
    //    链的 crate 集合 ⇒ 它一旦包含驱动/烘图层，那份库就会把驱动静态链进去
    //    （实测 15.0 MB vs 声明档 5.3 MB），而且"实现库不许依赖 `px_graph`/`px_cook`"那条
    //    不变式在这一档就没有门看着。⚠ `crate_graph.rs` 只管五份预置库，管不到这里。
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

    // 副本用完就收：`target/` 下不留垃圾（断言失败时留着 —— 那正好是现场）。
    let _ = std::fs::remove_dir_all(&probe);
}
