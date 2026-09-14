use px_ops::field::{Field, Projection};
use px_ops::ops::fbm::Params;
use px_ops::{canonical_params, node_key};

fn key_of(params: &Params, graph_version: u32, inputs: &[[u8; 32]]) -> [u8; 32] {
    node_key(
        "field.fbm",
        1,
        graph_version,
        (384, 192),
        Projection::Equirect,
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
        key_of(&Params::default(), 1, &[]),
        key_of(&explicit, 1, &[])
    );
}

#[test]
fn comments_and_spacing_do_not_change_the_key() {
    let plain: Params = toml::from_str("frequency = 6.0\noctaves = 4\n").expect("解析失败");
    let noisy: Params =
        toml::from_str("# 注释\nfrequency   =   6.0   # 行内注释\n\noctaves = 4\n").expect("解析失败");
    assert_eq!(canonical_params(&plain), canonical_params(&noisy));
    assert_eq!(key_of(&plain, 1, &[]), key_of(&noisy, 1, &[]));
}

#[test]
fn changing_a_value_changes_the_key() {
    let a: Params = toml::from_str("frequency = 6.0\n").expect("解析失败");
    let b: Params = toml::from_str("frequency = 6.5\n").expect("解析失败");
    assert_ne!(key_of(&a, 1, &[]), key_of(&b, 1, &[]));
}

#[test]
fn bumping_the_version_changes_the_key() {
    let params = Params::default();
    let v1 = node_key("field.fbm", 1, 1, (384, 192), Projection::Equirect, &canonical_params(&params), &[]);
    let v2 = node_key("field.fbm", 2, 1, (384, 192), Projection::Equirect, &canonical_params(&params), &[]);
    assert_ne!(v1, v2, "算子版本必须进键");
}

#[test]
fn the_canvas_size_is_part_of_the_key() {
    let params = Params::default();
    let small = node_key("field.fbm", 1, 1, (384, 192), Projection::Equirect, &canonical_params(&params), &[]);
    let large = node_key("field.fbm", 1, 1, (768, 384), Projection::Equirect, &canonical_params(&params), &[]);
    assert_ne!(small, large, "画布尺寸必须进键，否则改分辨率会命中旧尺寸的产物");
}

#[test]
fn bumping_the_graph_version_changes_the_key() {
    let params = Params::default();
    let g1 = node_key("field.fbm", 1, 1, (384, 192), Projection::Equirect, &canonical_params(&params), &[]);
    let g2 = node_key("field.fbm", 1, 2, (384, 192), Projection::Equirect, &canonical_params(&params), &[]);
    assert_ne!(g1, g2, "图版本必须进键");
}

#[test]
fn changing_an_input_changes_the_key() {
    let params = Params::default();
    let a = key_of(&params, 1, &[[1u8; 32]]);
    let b = key_of(&params, 1, &[[2u8; 32]]);
    let c = key_of(&params, 1, &[[1u8; 32], [1u8; 32]]);
    assert_ne!(a, b, "输入产物变了，下游必须重算");
    assert_ne!(a, c, "输入个数不同应当是不同的键");
}

#[test]
fn an_unknown_parameter_is_rejected() {
    let parsed: Result<Params, _> = toml::from_str("frequncy = 4.0\n");
    assert!(parsed.is_err(), "拼错的参数名必须报错，而不是被静默忽略");
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


