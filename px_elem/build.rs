//! 算**本 crate**（element 函数的家）那一份源码的指纹：`decl_hash()` 就是它。
//!
//! ⚠ 与各 `px_*_schema` 同一份算法（`px_fingerprint::cargo_fingerprint_for_crate`）——
//!   参数 struct 与规格表都在 `src/` 下，改它们 ⇒ 换 `PX_SOURCE_HASH` ⇒ 换实例键 ✓。
//!   ⚠ **体文件不在 `src/` 下**（`px_elem/body/*.rs`）：它们不进这一份指纹（改一行算法
//!   只该换**那一条**实例的键、只重编那一份库，不该连带所有 element 实例 —— R1）。
fn main() {
    px_fingerprint::cargo_fingerprint_for_crate();
}
