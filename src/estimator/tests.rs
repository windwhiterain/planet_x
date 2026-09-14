use super::*;

#[test]
fn power_law_keeps_a_fixed_slope_while_learning_the_level() {
    let mut estimator = PowerLaw::new(-1.0, 0.0, PowerLaw::DEFAULT_FORGETTING).with_fixed_slope();
    for step in 0..500 {
        let volume = 0.5 + (step % 40) as f32 * 0.25;
        estimator.update(volume, 3.0 / volume);
    }
    assert!(estimator.is_fixed_slope());
    assert_eq!(estimator.slope(), -1.0);
    assert_near(estimator.get(2.0), 1.5, 0.05);
}

#[test]
fn a_fixed_slope_does_not_drift_on_unidentifiable_data() {
    let mut estimator = PowerLaw::new(-1.0, 0.0, PowerLaw::DEFAULT_FORGETTING).with_fixed_slope();
    for step in 0..500 {
        let volume = 2.0 + (step % 8) as f32;
        estimator.update(volume, 1.0);
    }
    assert_eq!(estimator.slope(), -1.0, "阶数被钉住就不该漂");
    assert_near(estimator.get(4.0), 1.0, 0.4);
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
            assert!(quote <= PowerLaw::LOG_LIMIT.exp());
            assert!(quote >= (-PowerLaw::LOG_LIMIT).exp());
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
