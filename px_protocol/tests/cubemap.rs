use px_protocol::art::{
    CUBE_FACES, Domain, cube_direction, cube_face_of, cube_map_extent, direction_at, uv_of,
};

#[test]
fn the_six_faces_are_the_six_axes_in_layer_order() {
    let axes = [
        [1.0, 0.0, 0.0],
        [-1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, -1.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, -1.0],
    ];
    let face_size = 16;
    let (width, height) = cube_map_extent(face_size);
    for face in 0..CUBE_FACES {
        let centre = direction_at(
            Domain::CubeMap,
            width,
            height,
            face_size / 2,
            face * face_size + face_size / 2,
        );
        let axis = axes[face as usize];
        for index in 0..3 {
            assert!(
                (centre[index] - axis[index]).abs() < 0.11,
                "第 {face} 层的中心应当是 {axis:?}，实际 {centre:?}"
            );
        }
    }
}

#[test]
fn a_face_occupies_exactly_its_own_rows() {
    let face_size = 24;
    let (width, height) = cube_map_extent(face_size);
    assert_eq!((width, height), (face_size, face_size * CUBE_FACES));

    for face in 0..CUBE_FACES {
        for row in [0, 1, face_size / 2, face_size - 1] {
            let y = face * face_size + row;
            for x in [0, width / 2, width - 1] {
                let direction = direction_at(Domain::CubeMap, width, height, x, y);
                let (back, _, _) = cube_face_of(direction);
                assert_eq!(back, face, "第 {y} 行落在面 {back}，应当是面 {face}");

                let expected = cube_direction(
                    face,
                    (x as f32 + 0.5) / face_size as f32,
                    (row as f32 + 0.5) / face_size as f32,
                );
                for index in 0..3 {
                    assert!(
                        (direction[index] - expected[index]).abs() < 1e-5,
                        "({x},{y}) 的方向 {direction:?} 与 cube_direction 给的 {expected:?} 不一致"
                    );
                }
            }
        }
    }
}

#[test]
fn a_direction_comes_back_to_the_row_that_produced_it() {
    let face_size = 32;
    let (width, height) = cube_map_extent(face_size);
    for face in 0..CUBE_FACES {
        for row in 0..face_size {
            for column in 0..face_size {
                let y = face * face_size + row;
                let direction = direction_at(Domain::CubeMap, width, height, column, y);
                let uv = uv_of(Domain::CubeMap, direction, width, height);
                let back_x = ((uv[0] * width as f32) as u32).min(width - 1);
                let back_y = ((uv[1] * height as f32) as u32).min(height - 1);
                assert_eq!(back_x, column, "面 {face} 第 {row} 行第 {column} 列回到了第 {back_x} 列");
                assert_eq!(back_y, y, "面 {face} 第 {row} 行回到了第 {back_y} 行");
            }
        }
    }
}
