use super::*;

fn assert_near(actual: f32, expected: f32, tolerance: f32) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "期望 {expected}±{tolerance}，实际 {actual}",
    );
}

fn response() -> Response {
    Response::new(Response::DEFAULT_FORGETTING)
}

fn true_dealt(volume: f32, aggressiveness: f32) -> f32 {
    let share = 1.5 * aggressiveness.powf(0.5);
    let depth = 2.0 * aggressiveness.powf(0.5);
    (volume * share / (1.0 + volume / depth)).min(volume)
}

fn sample(index: usize) -> (f32, f32) {
    let volume = 0.25 + (index % 24) as f32 * 0.5;
    let aggressiveness = 0.25 * 2.0f32.powf((index % 11) as f32 * 0.5);
    (volume, aggressiveness)
}

#[test]
fn response_never_deals_more_than_declared() {
    let estimator = response();
    for volume in [1e-4, 0.5, 1.0, 40.0, 1e4] {
        for aggressiveness in [1e-3, 0.5, 1.0, 8.0, 1e3] {
            let dealt = estimator.get(volume, aggressiveness);
            assert!(
                dealt <= volume + 1e-4,
                "申报 {volume} 力度 {aggressiveness} 预测成交 {dealt}",
            );
        }
    }
}

#[test]
fn response_is_monotone_in_aggressiveness() {
    let mut estimator = response();
    for index in 0..400 {
        let (volume, aggressiveness) = sample(index);
        estimator.update(volume, aggressiveness, true_dealt(volume, aggressiveness));
    }
    let mut previous = 0.0;
    for step in 0..40 {
        let aggressiveness = 0.125 * 1.15f32.powf(step as f32);
        let dealt = estimator.get(2.0, aggressiveness);
        assert!(
            dealt >= previous - 1e-4,
            "力度 {aggressiveness} 下预测成交反而下降：{previous} -> {dealt}",
        );
        previous = dealt;
    }
}

#[test]
fn response_recovers_a_known_surface() {
    let mut estimator = response();
    for epoch in 0..40 {
        for index in 0..264 {
            let (volume, aggressiveness) = sample(index + epoch * 7);
            estimator.update(volume, aggressiveness, true_dealt(volume, aggressiveness));
        }
    }
    for index in 0..24 {
        let volume = 0.25 + index as f32 * 0.5;
        for aggressiveness in [0.25, 0.5, 1.0, 2.0, 4.0] {
            let expected = true_dealt(volume, aggressiveness);
            let predicted = estimator.get(volume, aggressiveness);
            assert_near(predicted, expected, 0.25 * expected.max(0.2));
        }
    }
}

#[test]
fn response_matches_the_small_and_large_limits() {
    let mut estimator = response();
    for index in 0..4000 {
        let (volume, aggressiveness) = sample(index);
        estimator.update(volume, aggressiveness, true_dealt(volume, aggressiveness));
    }
    let aggressiveness = 1.0;
    let shallow = estimator.get(1e-4, aggressiveness) / 1e-4;
    assert_near(shallow, estimator.share(aggressiveness).min(1.0), 1e-3);
    let deep = estimator.get(1e6, aggressiveness);
    assert_near(
        deep,
        estimator.share(aggressiveness) * estimator.depth(aggressiveness),
        0.2 * deep.max(1.0),
    );
}

#[test]
fn response_learns_from_missing_fills() {
    let mut estimator = response();
    let before = estimator.fill_ratio(1.0, 0.5);
    for _ in 0..200 {
        estimator.update(1.0, 0.5, 0.0);
    }
    let after = estimator.fill_ratio(1.0, 0.5);
    assert!(
        after < before,
        "零成交应当压低预测成交比例：{before} -> {after}",
    );
    assert!(after >= 0.0 && after.is_finite(), "预测比例越界：{after}");
}

#[test]
fn response_ignores_degenerate_observations() {
    let mut estimator = response();
    estimator.update(1.0, 1.0, 0.5);
    let share = estimator.share(1.0);
    let depth = estimator.depth(1.0);
    let noise = estimator.noise();
    estimator.update(0.0, 1.0, 0.5);
    estimator.update(-2.0, 1.0, 0.5);
    estimator.update(1.0, 0.0, 0.5);
    estimator.update(1.0, -1.0, 0.5);
    estimator.update(1.0, 1.0, -0.5);
    estimator.update(f32::NAN, 1.0, 0.5);
    estimator.update(1.0, f32::NAN, 0.5);
    estimator.update(1.0, 1.0, f32::NAN);
    estimator.update(1.0, 1.0, f32::INFINITY);
    assert_eq!(estimator.share(1.0), share);
    assert_eq!(estimator.depth(1.0), depth);
    assert_eq!(estimator.noise(), noise);
}

#[test]
fn response_quotes_nothing_for_an_empty_size() {
    let estimator = response();
    assert_eq!(estimator.get(0.0, 1.0), 0.0);
    assert_eq!(estimator.get(-3.0, 1.0), 0.0);
    assert_eq!(estimator.get(f32::NAN, 1.0), 0.0);
    assert_eq!(estimator.get(f32::INFINITY, 1.0), 0.0);
    assert_eq!(estimator.get(1.0, 0.0), 0.0);
    assert_eq!(estimator.get(1.0, f32::NAN), 0.0);
}

#[test]
fn response_keeps_its_surface_positive_and_bounded() {
    let estimator = response();
    for volume in [1e-6, 1e-3, 1.0, 1e3, 1e6] {
        for aggressiveness in [1e-6, 1e-3, 1.0, 1e3, 1e6] {
            let dealt = estimator.get(volume, aggressiveness);
            assert!(dealt.is_finite() && dealt >= 0.0);
            assert!(dealt <= volume * 1.0001);
            assert!(estimator.share(aggressiveness).is_finite());
            assert!(estimator.depth(aggressiveness).is_finite());
        }
    }
}

#[test]
fn response_tracks_a_regime_change() {
    let mut estimator = response();
    for index in 0..800 {
        let (volume, aggressiveness) = sample(index);
        estimator.update(volume, aggressiveness, true_dealt(volume, aggressiveness));
    }
    let volume = 1.5;
    let aggressiveness = 1.0;
    let before = estimator.get(volume, aggressiveness);
    for _ in 0..400 {
        estimator.update(volume, aggressiveness, 0.0);
    }
    let after = estimator.get(volume, aggressiveness);
    assert!(
        after < 0.5 * before,
        "长期零成交后预测应当明显下降：{before} -> {after}",
    );
}

#[test]
fn response_noise_grows_with_the_miss() {
    let mut clean = response();
    let mut noisy = response();
    for index in 0..600 {
        let (volume, aggressiveness) = sample(index);
        let expected = true_dealt(volume, aggressiveness);
        clean.update(volume, aggressiveness, expected);
        let wobble = 1.0 + 0.6 * (index as f32 * 0.7).sin();
        noisy.update(volume, aggressiveness, expected * wobble);
    }
    assert!(
        noisy.noise() > clean.noise(),
        "噪声估计应当随偏差上升：{} / {}",
        noisy.noise(),
        clean.noise(),
    );
    assert!(clean.noise() >= Response::MIN_NOISE);
    assert!(
        noisy.noise().is_finite() && noisy.noise() > 0.0,
        "噪声不再有上限（那是策略），但必须有限且为正：{}",
        noisy.noise(),
    );
}
