use super::*;

fn assert_close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 1e-6,
        "期望 {expected}，实际 {actual}",
    );
}

#[test]
fn scale_reproduces_the_observed_point() {
    let mut scale = Scale::new(1.0);
    scale.update(4.0, 9.0);
    assert_close(scale.get(4.0), 9.0);
}

#[test]
fn scale_is_linear_through_the_origin() {
    let mut scale = Scale::new(1.0);
    scale.update(2.0, 5.0);
    assert_close(scale.get(0.0), 0.0);
    assert_close(scale.get(4.0), 2.0 * scale.get(2.0));
    assert_close(scale.get(-3.0), -scale.get(3.0));
}

#[test]
fn scale_keeps_its_slope_when_the_target_is_its_own_output() {
    let mut scale = Scale::new(2.0);
    let observed = scale.get(3.0);
    scale.update(3.0, observed);
    assert_close(scale.get(3.0), observed);
    assert_close(scale.get(7.0), 2.0 * 7.0);
}

#[test]
fn scale_carries_the_sign_of_its_observation() {
    let mut scale = Scale::new(1.0);
    scale.update(2.0, -5.0);
    assert_close(scale.get(1.0), -2.5);
}

#[test]
fn scale_of_a_zero_observation_is_infinite() {
    let mut scale = Scale::new(1.0);
    scale.update(0.0, 5.0);
    assert!(scale.get(1.0).is_infinite());
}
