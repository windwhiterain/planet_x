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

fn assert_near(actual: f32, expected: f32, tolerance: f32) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "期望 {expected}±{tolerance}，实际 {actual}",
    );
}

fn power_law(slope: f32) -> PowerLaw {
    PowerLaw::new(slope, 0.0, PowerLaw::DEFAULT_FORGETTING)
}

#[test]
fn power_law_priors_reproduce_the_volume_to_scale_convention() {
    let sell = power_law(-1.0);
    let buy = power_law(1.0);
    for volume in [0.25, 0.5, 1.0, 2.0, 6.0, 25.0] {
        assert_near(sell.get(volume), 1.0 / volume, 1e-5);
        assert_near(buy.get(volume), volume, 1e-4);
    }
}

#[test]
fn power_law_recovers_a_known_exponent() {
    let mut estimator = PowerLaw::new(-1.0, 0.0, 1.0);
    for step in 0..400 {
        let volume = 0.5 + (step % 60) as f32 * 0.25;
        estimator.update(volume, 0.5 * volume.powf(-1.5));
    }
    assert_near(estimator.slope(), -1.5, 0.05);
    assert_near(estimator.get(2.0), 0.5 * 2.0f32.powf(-1.5), 0.01);
}

#[test]
fn power_law_holds_its_estimate_when_the_observation_matches() {
    let mut estimator = power_law(-2.0);
    let volume = 4.0;
    let predicted = estimator.get(volume);
    estimator.update(volume, predicted);
    assert_near(estimator.slope(), -2.0, 1e-3);
    assert_near(estimator.get(volume), predicted, 1e-5);
}

#[test]
fn power_law_quotes_nothing_for_an_empty_size() {
    let estimator = power_law(-1.0);
    assert_eq!(estimator.get(0.0), 0.0);
    assert_eq!(estimator.get(-3.0), 0.0);
    assert_eq!(estimator.get(f32::NAN), 0.0);
    assert_eq!(estimator.get(f32::INFINITY), 0.0);
}

#[test]
fn power_law_ignores_degenerate_observations() {
    let mut estimator = power_law(-1.0);
    let before = estimator.get(3.0);
    estimator.update(0.0, 1.0);
    estimator.update(-2.0, 1.0);
    estimator.update(2.0, 0.0);
    estimator.update(2.0, -1.0);
    estimator.update(f32::NAN, 1.0);
    estimator.update(2.0, f32::INFINITY);
    assert_eq!(estimator.slope(), -1.0);
    assert_eq!(estimator.intercept(), 0.0);
    assert_eq!(estimator.get(3.0), before);
}

#[test]
fn power_law_keeps_its_quote_positive_and_bounded() {
    for slope in [-40.0, -1.0, 1.0, 40.0] {
        let estimator = power_law(slope);
        for volume in [1e-6, 1e-3, 1.0, 1e3, 1e6] {
            let quote = estimator.get(volume);
            assert!(
                quote > 0.0 && quote.is_finite(),
                "斜率 {slope}、规模 {volume} 的报价越界：{quote}",
            );
            assert!(quote <= PowerLaw::MAX_LOG_SCALE.exp() + 1e-3);
            assert!(quote >= PowerLaw::MIN_LOG_SCALE.exp() - 1e-6);
        }
    }
}

#[test]
fn power_law_follows_a_regime_change() {
    let mut estimator = PowerLaw::new(-1.5, 0.0, 0.9);
    for step in 0..80 {
        let volume = 0.5 + (step % 40) as f32 * 0.25;
        estimator.update(volume, 0.5 * volume.powf(-1.5));
    }
    for step in 0..400 {
        let volume = 0.5 + (step % 40) as f32 * 0.25;
        estimator.update(volume, 0.5 * volume.powf(-0.5));
    }
    assert!(
        estimator.slope() > -1.0,
        "遗忘因子下应当追上新指数 -0.5，实际 {}",
        estimator.slope(),
    );
}
