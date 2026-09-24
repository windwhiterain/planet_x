//! **NURBS 声明的源码指纹** —— `px_op!` 里那句 `DECL_HASH = env!("PX_SOURCE_HASH")` 要它。
//!
//! 与 `px_field_schema` / `px_volume_schema` / `px_mesh_schema` 逐字同一条口径：
//! 泛型实例的 key 必须覆盖"**声明所在的这一份源码**"（`docs/system/generic-instances.md` §177）。

fn main() {
    px_fingerprint::cargo_fingerprint_for_crate(&[]);
}
