//! 算子库的**源码指纹** —— 身份的自动来源，别手维护清单。
//!
//! 与 `px_field_op` / `px_volume_op` / `px_mesh_op` / `px_nurbs_op` 逐字同一条口径：
//! 算的是"这个 crate 编译进去的全部源码"（自己 `src/` + 所有 path 依赖的 `src/` + 本文件）
//! —— ⚠ WGSL 也在 `src/` 里，所以**改一行着色器**同样会换掉身份与键。

fn main() {
    px_fingerprint::cargo_fingerprint_for_crate();
}
