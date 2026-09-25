use px_field_schema::field::{Field, Projection};
use px_field_schema::params::fbm::Params;
use px_graph::{OpId, canonical_params, node_key, shader_key};

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
    let params = Params::default();
    let same = key_of(&params, &[]);
    let other_interface = node_key(
        &OpId {
            id: "field.fbm",
            interface: !INTERFACE,
            source_hash: SOURCE,
        },
        &canonical_params(&params),
        |_| {},
    );
    assert_ne!(same, other_interface, "接口形状哈希必须进键");
}

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
        |_| {},
    );
    assert_ne!(before, after, "改了实现就必须换键");
}

#[test]
fn nothing_about_the_graph_itself_is_in_the_key() {
    let params = Params::default();
    let once = key_of(&params, &[]);
    let twice = key_of(&params, &[]);
    assert_eq!(
        once, twice,
        "键只由算子身份 + 参数 + 上游决定（形状在参数里，不在图里）"
    );
}

#[test]
fn the_shape_is_a_parameter_of_the_generators_so_it_is_in_the_key() {
    let mut wider = Params::default();
    wider.shape.width = 780;
    wider.shape.height = 520;
    assert_ne!(
        key_of(&Params::default(), &[]),
        key_of(&wider, &[]),
        "改了形状参数（宽高）却没换键 ⇒ 会命中按旧尺寸烘出来的产物"
    );

    let mut projected = Params::default();
    projected.shape.projection = Projection::Equirect;
    assert_ne!(
        key_of(&Params::default(), &[]),
        key_of(&projected, &[]),
        "改了投影却没换键"
    );

    const EXPLICIT: &str = "shape = { width = 512, height = 256, projection = \"CubeMap\" }\n";
    let explicit: Params = toml::from_str(EXPLICIT).expect("解析失败");
    assert_eq!(
        canonical_params(&Params::default()),
        canonical_params(&explicit),
        "显式写出的默认形状应当等于省略"
    );
    assert_eq!(
        key_of(&Params::default(), &[]),
        key_of(&explicit, &[]),
        "同一份形状 ⇒ 同一个键"
    );
}

#[test]
fn a_filter_node_has_no_shape_axis_to_carry_into_its_key() {
    let parsed: Result<px_field_schema::params::gradient::Params, _> =
        toml::from_str("shape = { width = 8, height = 4, projection = \"CubeMap\" }\n");
    assert!(
        parsed.is_err(),
        "过滤类节点的参数表里冒出了 `shape` ⇒ 别的节点的尺寸会顺着参数表进它的键"
    );
    let remap = px_field_schema::params::gradient::Params::default();
    let json = canonical_params(&remap);
    assert!(
        !json.contains("\"shape\""),
        "过滤类节点的规范参数里出现了 `shape`（它就是要进键的那一份）：{json}"
    );
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
