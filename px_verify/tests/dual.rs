use px_verify::dual::{Dual64, DualNum, first_derivative};

fn dmax<D: DualNum<Primitive = f64> + Copy>(one: D, two: D) -> D {
    if one.re() >= two.re() { one } else { two }
}

fn dmin<D: DualNum<Primitive = f64> + Copy>(one: D, two: D) -> D {
    if one.re() <= two.re() { one } else { two }
}

fn dclamp<D: DualNum<Primitive = f64> + Copy>(value: D, low: D, high: D) -> D {
    dmin(dmax(value, low), high)
}

fn dsmoothstep<D: DualNum<Primitive = f64> + Copy>(low: D, high: D, value: D) -> D {
    let t = dclamp(
        (value - low) / (high - low),
        D::from(0.0f64),
        D::from(1.0f64),
    );
    t * t * (D::from(3.0f64) - t - t)
}

fn slope<F>(f: F, at: f64) -> f64
where
    F: Fn(Dual64) -> Dual64,
{
    first_derivative(f, at).1
}

fn smoothstep(value: f64, low: f64, high: f64) -> f64 {
    let t = ((value - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[test]
fn a_dual_carries_the_derivative_of_the_branch_it_actually_took() {
    let taken = slope(|value| dmax(value, Dual64::from_re(2.0)), 3.0);
    assert!(
        (taken - 1.0).abs() < 1e-12,
        "取到上支时导数应当是 1，实测 {taken}"
    );

    let flat = slope(|value| dmax(value, Dual64::from_re(2.0)), 1.0);
    assert!(flat.abs() < 1e-12, "取到常数支时导数应当是 0，实测 {flat}");
}

#[test]
fn clamping_is_flat_outside_and_linear_inside() {
    let value = |x: f64| x.clamp(0.0, 1.0);
    assert!(
        slope(|x| dclamp(x, Dual64::from_re(0.0), Dual64::from_re(1.0)), -1.0).abs() < 1e-12,
        "下界之外应当是平的"
    );
    assert!(
        slope(|x| dclamp(x, Dual64::from_re(0.0), Dual64::from_re(1.0)), 2.0).abs() < 1e-12,
        "上界之外应当是平的"
    );
    let inside = slope(
        |x| dclamp(x, Dual64::from_re(0.0), Dual64::from_re(1.0)),
        0.5,
    );
    assert!((inside - 1.0).abs() < 1e-12, "界内斜率应当是 1，实测 {inside}");
    assert!(value(0.5) > 0.0);
}

#[test]
fn smoothstep_kills_its_own_clamp_at_both_ends() {
    let dual = |at: f64| {
        slope(
            |value| {
                dsmoothstep(
                    Dual64::from_re(0.0),
                    Dual64::from_re(1.0),
                    value,
                )
            },
            at,
        )
    };
    assert!(dual(0.0).abs() < 1e-12, "起点斜率应当是 0");
    assert!(dual(1.0).abs() < 1e-12, "终点斜率应当是 0");
    let middle = dual(0.5);
    assert!((middle - 1.5).abs() < 1e-12, "中点斜率应当是 1.5，实测 {middle}");
}

#[test]
fn the_dual_smoothstep_matches_the_closed_form_derivative() {
    let dual = |at: f64| {
        slope(
            |value| {
                dsmoothstep(
                    Dual64::from_re(0.0),
                    Dual64::from_re(0.3),
                    value,
                )
            },
            at,
        )
    };
    let closed = |at: f64| {
        let t = (at / 0.3).clamp(0.0, 1.0);
        if t > 0.0 && t < 1.0 {
            6.0 * t * (1.0 - t) / 0.3
        } else {
            0.0
        }
    };

    let mut worst = 0.0_f64;
    for step in 0..=200 {
        let at = step as f64 / 200.0 * 0.3;
        worst = worst.max((dual(at) - closed(at)).abs());
    }
    assert!(
        worst < 1e-12,
        "对偶数求得的 smoothstep 导数与闭式不符，最大偏差 {worst}"
    );
}

#[test]
fn a_product_of_clamped_factors_differentiates_through_every_piece() {
    let value = |x: f64| {
        smoothstep(x, 0.0, 0.3) * (1.0 - smoothstep(x, 0.5, 0.7)) * x.clamp(0.1, 0.9)
    };
    let dual = |at: f64| {
        slope(
            |x| {
                dsmoothstep(Dual64::from_re(0.0), Dual64::from_re(0.3), x)
                    * (Dual64::from_re(1.0)
                        - dsmoothstep(Dual64::from_re(0.5), Dual64::from_re(0.7), x))
                    * dclamp(x, Dual64::from_re(0.1), Dual64::from_re(0.9))
            },
            at,
        )
    };

    let kinks = [0.1, 0.3, 0.5, 0.7, 0.9];
    let mut worst = 0.0_f64;
    let mut counted = 0usize;
    for step in 15..185 {
        let at = step as f64 / 200.0;
        if kinks.iter().any(|kink| (at - kink).abs() < 1e-3) {
            continue;
        }
        counted += 1;
        let numeric = (value(at + 1e-6) - value(at - 1e-6)) / 2e-6;
        worst = worst.max((dual(at) - numeric).abs());
    }
    assert!(
        counted > 100,
        "有效采样点只有 {counted} 个，这个测试没在测东西"
    );
    assert!(
        worst < 1e-5,
        "离开折点后对偶数导数与中心差分不符，最大偏差 {worst}（折点邻域已排除：那里对偶数给单侧导数，中心差分跨折点，本来就没有可比性）"
    );
}
