//! PRNG 的单元测试。

use super::*;

#[test]
fn checkpoint_state_restores_identical_sequence() {
    let mut a = Prng::new(42);
    for _ in 0..100 {
        a.next_u64();
    }
    // Save the position, then keep going from the fresh original.
    let mut b = Prng::from_state(a.state());
    for _ in 0..50 {
        a.next_u64();
        b.next_u64();
    }
    assert_eq!(a.state(), b.state());
}
