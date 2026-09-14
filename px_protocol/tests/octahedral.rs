use px_protocol::art::{
    octahedral_direction, octahedral_direction_y_up, octahedral_uv, octahedral_uv_y_up,
};

#[test]
fn the_encoding_is_the_inverse_of_the_decoding() {
    let mut worst = 0.0_f32;
    for i in 0..96 {
        for j in 0..96 {
            let u = (i as f32 + 0.5) / 96.0;
            let v = (j as f32 + 0.5) / 96.0;
            let direction = octahedral_direction(u, v);
            let back = octahedral_uv(direction);
            let again = octahedral_direction(back[0], back[1]);
            for axis in 0..3 {
                worst = worst.max((direction[axis] - again[axis]).abs());
            }
        }
    }
    assert!(worst < 1e-3, "编解码往返最大误差 {worst}");
}

#[test]
fn the_y_up_variant_keeps_the_pole_on_y() {
    let centre = octahedral_direction_y_up(0.5, 0.5);
    assert!(
        centre[1] > 0.99,
        "正方形正中应当是 +Y 极点，实际 {centre:?}"
    );

    let mut worst = 0.0_f32;
    for i in 0..64 {
        for j in 0..64 {
            let u = (i as f32 + 0.5) / 64.0;
            let v = (j as f32 + 0.5) / 64.0;
            let direction = octahedral_direction_y_up(u, v);
            let back = octahedral_uv_y_up(direction);
            let again = octahedral_direction_y_up(back[0], back[1]);
            for axis in 0..3 {
                worst = worst.max((direction[axis] - again[axis]).abs());
            }
        }
    }
    assert!(worst < 1e-3, "y_up 往返最大误差 {worst}");
}

#[test]
fn every_direction_is_a_unit_vector() {
    for i in 0..64 {
        for j in 0..64 {
            let u = (i as f32 + 0.5) / 64.0;
            let v = (j as f32 + 0.5) / 64.0;
            let d = octahedral_direction_y_up(u, v);
            let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            assert!((length - 1.0).abs() < 1e-4, "({u}, {v}) 长度 {length}");
        }
    }
}
