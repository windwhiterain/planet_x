use super::*;

fn assert_close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 1e-5,
        "期望 {expected}，实际 {actual}",
    );
}

#[test]
fn same_signature_groups_positive_and_negative() {
    assert!(same_signature(1.0, 2.0));
    assert!(same_signature(-1.0, -2.0));
    assert!(!same_signature(1.0, -2.0));
    assert!(!same_signature(-1.0, 2.0));
}

#[test]
fn same_signature_treats_zero_as_both_sides() {
    assert!(same_signature(0.0, 5.0));
    assert!(same_signature(0.0, -5.0));
    assert!(same_signature(0.0, 0.0));
}

#[test]
fn nan_is_not_same_signature_as_anything() {
    assert!(!same_signature(f32::NAN, f32::NAN));
    assert!(!same_signature(f32::NAN, 1.0));
    assert!(!same_signature(1.0, f32::NAN));
}

#[test]
fn conditional_swap_swaps_only_when_asked() {
    assert_eq!(conditional_swap(1, 2, true), (2, 1));
    assert_eq!(conditional_swap(1, 2, false), (1, 2));
}

#[test]
fn conditional_swap_moves_values_that_are_not_copy() {
    let (first, second) = conditional_swap(String::from("a"), String::from("b"), true);
    assert_eq!(first, "b");
    assert_eq!(second, "a");
}

#[test]
fn similarity_is_one_for_equal_magnitudes() {
    assert_close(similarity(3.0, 3.0), 1.0);
    assert_close(similarity(-4.0, -4.0), 1.0);
    assert_close(similarity(0.5, 0.5), 1.0);
}

#[test]
fn similarity_is_symmetric_and_bounded() {
    for a in [0.25, 1.0, 3.0, 40.0] {
        for b in [0.5, 1.0, 7.0, 100.0] {
            let value = similarity(a, b);
            assert_close(value, similarity(b, a));
            assert!(
                (-1.0..=1.0).contains(&value),
                "相似度必须在 [-1, 1]，实际 {value}",
            );
        }
    }
}

#[test]
fn similarity_flips_sign_with_the_operands() {
    assert_close(similarity(1.0, -1.0), -1.0);
    assert!(similarity(6.0, -1.0) < 0.0);
    assert!(similarity(6.0, 1.0) > 0.0);
}

#[test]
fn similarity_of_two_zeros_is_nan() {
    assert!(similarity(0.0, 0.0).is_nan());
}

#[test]
fn conditional_signature_maps_bool_to_pm_one() {
    assert_close(conditional_signature(true), 1.0);
    assert_close(conditional_signature(false), -1.0);
}

#[test]
fn geometric_average_of_four_and_nine_is_six() {
    assert_close(geometric_average(4.0, 9.0), 6.0);
}

#[test]
fn geometric_average_is_symmetric_and_idempotent() {
    assert_close(geometric_average(3.0, 12.0), geometric_average(12.0, 3.0));
    assert_close(geometric_average(7.5, 7.5), 7.5);
}

#[test]
fn geometric_average_ignores_signs() {
    assert_close(geometric_average(-4.0, 9.0), 6.0);
    assert_close(geometric_average(-4.0, -9.0), 6.0);
    assert_close(geometric_average(0.0, 9.0), 0.0);
}

#[test]
fn normal_cdf_is_one_half_at_the_center() {
    assert!((normal_cdf(0.0) - 0.5).abs() < 1e-5, "{}", normal_cdf(0.0));
}

#[test]
fn normal_cdf_matches_the_known_quantiles() {
    assert!((normal_cdf(1.0) - 0.8413).abs() < 1e-3, "{}", normal_cdf(1.0));
    assert!((normal_cdf(-1.0) - 0.1587).abs() < 1e-3, "{}", normal_cdf(-1.0));
    assert!((normal_cdf(1.6449) - 0.95).abs() < 1e-3, "{}", normal_cdf(1.6449));
    assert!((normal_cdf(2.0) - 0.9772).abs() < 1e-3, "{}", normal_cdf(2.0));
}

#[test]
fn normal_cdf_is_monotone_and_bounded() {
    let mut previous = 0.0;
    for step in -60..=60 {
        let value = normal_cdf(step as f32 * 0.1);
        assert!(value >= previous - 1e-6, "非单调：{value} < {previous}");
        assert!((0.0..=1.0).contains(&value), "越界：{value}");
        previous = value;
    }
    assert_eq!(normal_cdf(1e9), 1.0);
    assert_eq!(normal_cdf(-1e9), 0.0);
    assert_eq!(normal_cdf(f32::NAN), 0.0);
}
