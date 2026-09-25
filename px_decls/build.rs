//! **`px_decls` 自己的身份** —— 与各 schema / 实现库同一份算法（`px_fingerprint` rlib）。
//!
//! ⚠ 它今天不参与任何**实例** key（它不进任何实现库的名册：那些库的 manifest 由
//!   `px_cook::inst` 按边一条一条写出来，里面没有它）。留着它是为了与全仓同一口径，
//!   以及"**本 crate 改了就换一个可查的指纹**"这条性质不缺口。
//! ⚠ `px_graphs/build.rs` 重跑靠的是 **cargo 的 path 依赖**（`px_decls` 是它的
//!   `[build-dependencies]`）—— 不是这个哈希。

fn main() {
    px_fingerprint::cargo_fingerprint_for_crate();
}
