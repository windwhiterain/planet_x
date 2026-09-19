use px_field_schema::field::{Field, Projection};
use px_protocol::art::{cube_direction, cube_map_extent};

fn axis_cube_map(face_size: u32) -> Field {
    let (width, height) = cube_map_extent(face_size);
    let mut field = Field::filled_with(width, height, 0.0, Projection::CubeMap);
    for y in 0..height {
        for x in 0..width {
            field.set(x, y, field.direction(x, y)[0]);
        }
    }
    field
}

fn normalize(vector: [f32; 3]) -> [f32; 3] {
    let length = (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt();
    [vector[0] / length, vector[1] / length, vector[2] / length]
}

#[test]
fn a_smooth_field_stays_smooth_across_a_face_edge() {
    let field = axis_cube_map(64);
    let mut worst = 0.0_f32;
    let mut biggest_step = 0.0_f32;
    let mut previous: Option<f32> = None;

    for step in 0..=600 {
        let angle = -0.9 + 1.8 * step as f32 / 600.0;
        let direction = normalize([1.0_f32.cos(), 0.0, angle.sin()]);
        let value = field.sample_direction(direction);
        worst = worst.max((value - direction[0]).abs());
        if let Some(before) = previous {
            biggest_step = biggest_step.max((value - before).abs());
        }
        previous = Some(value);
    }

    assert!(worst < 0.03, "跨 +X/+Z 棱采样偏离解析值 {worst}");
    assert!(biggest_step < 0.02, "跨棱处出现跳变，单步最大 {biggest_step}");
}

#[test]
fn the_other_edge_is_continuous_too() {
    let field = axis_cube_map(64);
    let mut worst = 0.0_f32;
    let mut biggest_step = 0.0_f32;
    let mut previous: Option<f32> = None;

    for step in 0..=600 {
        let angle = -0.9 + 1.8 * step as f32 / 600.0;
        let direction = normalize([1.0_f32.cos(), angle.sin(), 0.0]);
        let value = field.sample_direction(direction);
        worst = worst.max((value - direction[0]).abs());
        if let Some(before) = previous {
            biggest_step = biggest_step.max((value - before).abs());
        }
        previous = Some(value);
    }

    assert!(worst < 0.03, "跨 +X/+Y 棱采样偏离解析值 {worst}");
    assert!(biggest_step < 0.02, "跨棱处出现跳变，单步最大 {biggest_step}");
}

#[test]
fn every_face_is_reachable_by_its_own_direction() {
    let face_size = 32;
    let field = axis_cube_map(face_size);
    for face in 0..6 {
        for row in 0..face_size {
            for column in 0..face_size {
                let direction = cube_direction(
                    face,
                    (column as f32 + 0.5) / face_size as f32,
                    (row as f32 + 0.5) / face_size as f32,
                );
                let sampled = field.sample_direction(direction);
                assert!(
                    (sampled - direction[0]).abs() < 0.02,
                    "面 {face} 的 ({column},{row}) 采到 {sampled}，解析值 {}",
                    direction[0]
                );
            }
        }
    }
}
