use super::*;

fn assert_near(actual: f32, expected: f32, tolerance: f32) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "期望 {expected}±{tolerance}，实际 {actual}",
    );
}

#[test]
fn a_flat_price_keeps_a_zero_drift() {
    let mut state = MarketState::new(&[1.0]);
    for _ in 0..50 {
        state.observe(0, 1.0);
    }
    assert_eq!(state.level(0), 1.0);
    assert_eq!(state.drift(0), 0.0);
    assert_eq!(state.volatility(0), 0.0);
}

#[test]
fn a_geometric_decay_shows_up_as_a_negative_drift() {
    let mut state = MarketState::new(&[1.0]);
    let mut price = 1.0;
    for _ in 0..200 {
        price *= 0.9;
        state.observe(0, price);
    }
    assert_near(state.drift(0), 0.9f32.ln(), 1e-3);
    assert_near(state.level(0), price, 1e-6);
    assert!(
        state.volatility(0) < 1e-3,
        "单调衰减没有波动：{}",
        state.volatility(0)
    );
}

#[test]
fn a_growing_price_shows_up_as_a_positive_drift() {
    let mut state = MarketState::new(&[1.0]);
    let mut price = 1.0;
    for _ in 0..200 {
        price *= 1.05;
        state.observe(0, price);
    }
    assert_near(state.drift(0), 1.05f32.ln(), 1e-3);
}

#[test]
fn alternating_prices_raise_the_volatility() {
    let mut state = MarketState::new(&[1.0]);
    for step in 0..200 {
        state.observe(0, if step % 2 == 0 { 1.1 } else { 0.9 });
    }
    assert!(
        state.volatility(0) > 0.05,
        "来回震荡应当有波动：{}",
        state.volatility(0)
    );
    assert!(state.drift(0).abs() < state.volatility(0));
}

#[test]
fn every_merchandise_is_tracked_on_its_own() {
    let mut state = MarketState::new(&[1.0, 1.0]);
    state.observe(0, 2.0);
    for _ in 0..50 {
        state.observe(1, 1.0);
    }
    assert!(state.drift(0) > 0.0);
    assert_eq!(state.drift(1), 0.0);
    assert_eq!(state.level(1), 1.0);
    assert_eq!(state.level(5), 0.0);
    assert_eq!(state.drift(5), 0.0);
}

#[test]
fn degenerate_prices_are_ignored() {
    let mut state = MarketState::new(&[1.0]);
    for _ in 0..50 {
        state.observe(0, 1.0);
    }
    state.observe(0, 0.0);
    state.observe(0, -3.0);
    state.observe(0, f32::NAN);
    state.observe(0, f32::INFINITY);
    assert_eq!(state.level(0), 1.0);
    assert_eq!(state.drift(0), 0.0);
    assert_eq!(state.volatility(0), 0.0);
}

#[test]
fn an_unset_merchandise_adopts_the_first_price() {
    let mut state = MarketState::new(&[0.0]);
    for _ in 0..20 {
        state.observe(0, 5.0);
    }
    assert_eq!(state.drift(0), 0.0);
    assert_eq!(state.level(0), 5.0);
}

#[test]
fn drift_and_volatility_stay_finite() {
    let mut state = MarketState::new(&[1.0]);
    for step in 0..400 {
        let price = if step % 2 == 0 { 1e6 } else { 1e-6 };
        state.observe(0, price);
    }
    let largest = (1e6f32 / 1e-6).ln();
    assert!(state.drift(0).is_finite());
    assert!(state.volatility(0).is_finite());
    assert!(state.drift(0).abs() <= largest);
    assert!(state.volatility(0) <= largest);
}
