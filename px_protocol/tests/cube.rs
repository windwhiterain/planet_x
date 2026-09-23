use px_protocol::art::{CUBE_FACES, cube_atlas_uv, cube_direction, cube_face_of};

#[test]
fn each_face_centre_is_its_axis() {
    let axes = [
        [1.0, 0.0, 0.0],
        [-1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, -1.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, -1.0],
    ];
    for face in 0..CUBE_FACES {
        let centre = cube_direction(face, 0.5, 0.5);
        let axis = axes[face as usize];
        for axis_index in 0..3 {
            assert!(
                (centre[axis_index] - axis[axis_index]).abs() < 1e-4,
                "面 {face} 的中心应当是 {axis:?}，实际 {centre:?}"
            );
        }
    }
}

#[test]
fn a_direction_lands_on_the_face_that_produced_it() {
    let mut worst = 0.0_f32;
    for face in 0..CUBE_FACES {
        for i in 0..32 {
            for j in 0..32 {
                let s = (i as f32 + 0.5) / 32.0;
                let t = (j as f32 + 0.5) / 32.0;
                let direction = cube_direction(face, s, t);
                let (back_face, back_s, back_t) = cube_face_of(direction);
                assert_eq!(back_face, face, "方向 {direction:?} 落到了面 {back_face}");
                worst = worst.max((back_s - s).abs()).max((back_t - t).abs());
            }
        }
    }
    assert!(worst < 1e-4, "面内坐标往返误差 {worst}");
}

#[test]
fn the_gutter_extends_into_the_neighbouring_direction() {
    let face_size = 256;
    let gutter = 2;
    let inside = cube_atlas_uv(0, 0.0, 0.5, face_size, gutter);
    let outside = cube_atlas_uv(
        0,
        -(gutter as f32) / face_size as f32,
        0.5,
        face_size,
        gutter,
    );
    assert!(outside[0] < inside[0], "gutter 应当落在面的外侧");

    let extended = cube_direction(0, -0.5 / face_size as f32, 0.5);
    let (neighbour, s, t) = cube_face_of(extended);
    assert_eq!(neighbour, 4, "越过 +X 面的 s<0 边界应当进入 +Z 面");
    assert!(
        s.max(t) > 0.99,
        "gutter 应当落在邻面贴边处，实际 ({s}, {t})"
    );
}
