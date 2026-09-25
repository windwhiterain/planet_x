//! **契约的源码指纹** —— 装载实现库时用它握手。
//!
//! 实现库导出"我编的时候契约是哪一份"，图程序拿自己这一份比：对不上就说明 DLL 与图程序
//! 不是同一份契约编出来的（类型布局可能已经不一样了）⇒ **当场拒**，绝不拿错的布局去调。
//!
//! 实现在 `px_fingerprint`（**rlib**：build.rs 与运行期代码共用同一份算法）。
//! 见 `docs/operators.md`（契约握手那一节）。

fn main() {
    px_fingerprint::cargo_fingerprint_for_crate();
}
