//! **中档（T2，49–480 回合）**的端到端行为用例。
//!
//! 快档（默认 `cargo nextest run`）不选这个二进制；`-P mid` / `-P full` 选。
//! 长档（T3，>480 回合）在 `tests/horizon_long.rs`，探针诊断在 `tests/trade_probe.rs`。

use planet_x::config::load_config;
use planet_x::model::*;
use planet_x::prng::Prng;
use planet_x::sim;
use planet_x::world;

/// 确定性复现：同一种子 + 相同配置 + 相同回合数 → 输出逐字节一致（spec 的硬性要求）。
/// 这也锁定了新增机制（治理/重建/本土防御/MOND）不会破坏可复现性。
#[test]
fn same_seed_reproduces_identically() {
    let config = load_config();
    let mut a = world::default_state(&config, 42);
    let mut ra = Prng::new(42);
    let mut b = world::default_state(&config, 42);
    let mut rb = Prng::new(42);
    let mut derived_a = Derived::default();
    let mut derived_b = Derived::default();
    for _ in 0..200 {
        derived_a = sim::advance(&mut a, &config, &mut ra);
        derived_b = sim::advance(&mut b, &config, &mut rb);
    }
    // 用最后一回合的 Derived 渲染（携带产出/维护/治理流 + 总结指标），验证这些中间量同样可复现。
    let sa = planet_x::agent::render_state(&a, &derived_a);
    let sb = planet_x::agent::render_state(&b, &derived_b);
    assert_eq!(
        sa, sb,
        "same seed 42 at round 200 must reproduce identical agent state"
    );
    assert_eq!(a.round, b.round);
}
