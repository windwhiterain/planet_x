//! 键的门：**一个节点的键 = 产出这个节点的那些东西**。
//!
//! ⚠ 这里调的是**唯一那个算键的函数**（`px_graph_schema::node_key`）—— 生产路径
//! （`px_cook::cached`）调的是同一个。从前有两份（`node_key` 只在测试里活着、`cached` 自己
//! 内联算），于是"文档里的键定义"与"真正的键"可以悄悄漂开；现在漂不开。

use px_field_schema::field::Field;
use px_field_schema::params::fbm::Params;
use px_graph::{Grid, OpId, canonical_params, node_key, shader_key};
use px_protocol::art::Domain;

const INTERFACE: u64 = 0x0123_4567_89ab_cdef;
const SOURCE: &str = "0123456789abcdef";

fn key_of(params: &Params, inputs: &[[u8; 32]]) -> [u8; 32] {
    node_key(
        &OpId {
            id: "field.fbm",
            interface: INTERFACE,
            source_hash: SOURCE,
        },
        &canonical_params(params),
        None,
        |hasher| {
            for key in inputs {
                hasher.update(key);
            }
        },
    )
}

#[test]
fn the_same_values_give_the_same_key_however_they_were_written() {
    let explicit: Params = toml::from_str("frequency = 4.0\n").expect("解析失败");
    assert_eq!(
        canonical_params(&Params::default()),
        canonical_params(&explicit),
        "省略的字段应当等于默认值"
    );
    assert_eq!(key_of(&Params::default(), &[]), key_of(&explicit, &[]));
}

#[test]
fn comments_and_spacing_do_not_change_the_key() {
    let plain: Params = toml::from_str("frequency = 6.0\noctaves = 4\n").expect("解析失败");
    let noisy: Params = toml::from_str("# 注释\nfrequency   =   6.0   # 行内注释\n\noctaves = 4\n")
        .expect("解析失败");
    assert_eq!(canonical_params(&plain), canonical_params(&noisy));
    assert_eq!(key_of(&plain, &[]), key_of(&noisy, &[]));
}

#[test]
fn changing_a_value_changes_the_key() {
    let a: Params = toml::from_str("frequency = 6.0\n").expect("解析失败");
    let b: Params = toml::from_str("frequency = 6.5\n").expect("解析失败");
    assert_ne!(key_of(&a, &[]), key_of(&b, &[]));
}

#[test]
fn a_different_interface_changes_the_key() {
    // ⚠ 这里原来是"升 `VERSION` 必须换键"。`version` 已经**删掉**了 ——
    //   它由算子的**接口形状哈希**取代（`px_cook::interface_hash`，从参数/输入/输出
    //   三个类型名推出来）。所以这条门守的变成"接口哈希进键"：
    //   改了参数 struct、改了输入 struct、改了输出域 ⇒ 哈希变 ⇒ 键变，没人需要记得升版本。
    let params = Params::default();
    let same = key_of(&params, &[]);
    let other_interface = node_key(
        &OpId {
            id: "field.fbm",
            interface: !INTERFACE,
            source_hash: SOURCE,
        },
        &canonical_params(&params),
        None,
        |_| {},
    );
    assert_ne!(same, other_interface, "接口形状哈希必须进键");
}

/// **实现的源码指纹进键** —— 这是"改了实现却命中旧产物"唯一的解药。
///
/// 它与接口哈希是两半：接口哈希管"形状变了"，源码指纹管"形状没变而算法变了"。
#[test]
fn a_different_implementation_source_changes_the_key() {
    let params = Params::default();
    let before = key_of(&params, &[]);
    let after = node_key(
        &OpId {
            id: "field.fbm",
            interface: INTERFACE,
            source_hash: "fedcba9876543210",
        },
        &canonical_params(&params),
        None,
        |_| {},
    );
    assert_ne!(before, after, "改了实现就必须换键");
}

/// ⚠ 这条门是**反的**（原来是"图版本必须进键"）。
///
/// `graph_version` 是**图的属性** —— 改图脚本里别处一行代码，不该让这个节点的产物作废。
/// 签名里根本没有它，所以"它不影响键"是**结构性**的，不靠自觉。
#[test]
fn nothing_about_the_graph_itself_is_in_the_key() {
    let params = Params::default();
    let once = key_of(&params, &[]);
    let twice = key_of(&params, &[]);
    assert_eq!(
        once, twice,
        "键只由算子身份 + 参数 + 上游（+ 该域的画布）决定"
    );
}

/// 画布**由调用点**决定给不给（域自己声明 `RESOLUTION_IS_CANVAS`）：
/// 场给 `Some`，体积/网格给 `None` —— 一刀切两头都会错。
#[test]
fn the_canvas_is_a_coordinate_the_caller_opts_into() {
    let params = Params::default();
    let canvas = Grid {
        width: 780,
        height: 520,
        projection: Domain::Cube,
    };
    let without = key_of(&params, &[]);
    let with = node_key(
        &OpId {
            id: "field.fbm",
            interface: INTERFACE,
            source_hash: SOURCE,
        },
        &canonical_params(&params),
        Some(canvas),
        |_| {},
    );
    assert_ne!(without, with, "给了画布就该是另一个键");
    let same_canvas = node_key(
        &OpId {
            id: "field.fbm",
            interface: INTERFACE,
            source_hash: SOURCE,
        },
        &canonical_params(&params),
        Some(Grid {
            projection: Domain::Cube,
            ..canvas
        }),
        |_| {},
    );
    assert_eq!(with, same_canvas, "同一份画布 ⇒ 同一个键");
}

#[test]
fn changing_an_input_changes_the_key() {
    let params = Params::default();
    let a = key_of(&params, &[[1u8; 32]]);
    let b = key_of(&params, &[[2u8; 32]]);
    let c = key_of(&params, &[[1u8; 32], [1u8; 32]]);
    assert_ne!(a, b, "输入产物变了，下游必须重算");
    assert_ne!(a, c, "输入个数不同应当是不同的键");
}

#[test]
fn an_unknown_parameter_is_rejected() {
    let parsed: Result<Params, _> = toml::from_str("frequncy = 4.0\n");
    assert!(parsed.is_err(), "拼错的参数名必须报错，而不是被静默忽略");
}

/// Shader 的键 = 入口字节 + **include 闭包**指纹（§17.1、§52.3）。
/// 这条钉的就是那个洞：入口一个字没动、被 import 的模块改了 ⇒ 必须换键，
/// 否则键不动、场景键不动、槽版本不动，而画出来的东西变了。
#[test]
fn a_shader_key_follows_its_include_closure() {
    let entry = "#import planet_x::noise::fbm_3\nfn f() -> f32 { fbm_3() }\n";
    let edited_entry = "#import planet_x::noise::fbm_3\nfn f() -> f32 { fbm_3() + 1.0 }\n";
    let library = "#define_import_path planet_x::noise\nfn fbm_3() -> f32 { 1.0 }\n";
    let edited_library = "#define_import_path planet_x::noise\nfn fbm_3() -> f32 { 2.0 }\n";

    let base = closure_of(entry, library);
    let same = closure_of(entry, library);
    let include_changed = closure_of(entry, edited_library);

    assert_eq!(
        shader_key(entry, &base),
        shader_key(entry, &same),
        "同一份入口 + 同一份闭包 ⇒ 同一个键"
    );
    assert_ne!(
        shader_key(entry, &base),
        shader_key(entry, &include_changed),
        "入口没动、include 变了 ⇒ 必须换键（这就是原来漏掉的那一维）"
    );
    assert_ne!(
        shader_key(entry, &base),
        shader_key(edited_entry, &base),
        "入口改了也必须换键"
    );
}

fn closure_of(entry: &str, library: &str) -> px_shader::Closure {
    let modules: px_shader::ModuleTable = [("planet_x::noise".to_string(), library.to_string())]
        .into_iter()
        .collect();
    px_shader::closure(entry, &modules)
}

#[test]
fn a_field_survives_the_blob_round_trip() {
    let mut field = Field::filled(8, 4, 0.0);
    for y in 0..4 {
        for x in 0..8 {
            field.set(x, y, (x as f32) * 0.5 - (y as f32));
        }
    }
    let restored = Field::from_blob(&field.to_blob()).expect("往返失败");
    assert_eq!(field, restored);
}
