use px_field_schema::field::Field;
use px_field_schema::params::fbm::Params;
use px_graph::{canonical_params, node_key, shader_key};

fn key_of(params: &Params, inputs: &[[u8; 32]]) -> [u8; 32] {
    node_key(
        "field.fbm",
        "0123456789abcdef",
        &canonical_params(params),
        inputs,
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
    assert_eq!(
        key_of(&Params::default(), &[]),
        key_of(&explicit, &[])
    );
}

#[test]
fn comments_and_spacing_do_not_change_the_key() {
    let plain: Params = toml::from_str("frequency = 6.0\noctaves = 4\n").expect("解析失败");
    let noisy: Params =
        toml::from_str("# 注释\nfrequency   =   6.0   # 行内注释\n\noctaves = 4\n").expect("解析失败");
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
    let before = node_key(
        "field.fbm",
        "0123456789abcdef",
        &canonical_params(&params),
        &[],
    );
    let after = node_key(
        "field.fbm",
        "fedcba9876543210",
        &canonical_params(&params),
        &[],
    );
    assert_ne!(before, after, "接口形状哈希必须进键");
}

/// ⚠ 这条门是**反的**（原来是"图版本必须进键"）。
///
/// 一个节点的键 = 产出这个节点的那些东西：算子身份 + 规范参数 + 上游的键。
/// `graph_version` 是**图的属性** —— 改图脚本里别处一行代码，不该让这个节点的产物作废。
#[test]
fn the_graph_version_is_not_part_of_the_key() {
    let params = Params::default();
    // `node_key` 的签名里已经没有 `graph_version` 了 —— 这条测试记录的就是那个决定。
    // 想证明"它不影响键"，最直接的办法是同一个调用给出同一个键（没有可变的第三个数）。
    let once = key_of(&params, &[]);
    let twice = key_of(&params, &[]);
    assert_eq!(once, twice, "键只由算子身份 + 参数 + 上游决定");

    // ⚠ 画布同理：它对**场**是真的，但那件事由 `px_cook` 按域决定
    //   （`Payload::RESOLUTION_IS_CANVAS`），不在 `node_key` 里一刀切。
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
    let modules: px_shader::ModuleTable =
        [("planet_x::noise".to_string(), library.to_string())]
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


