//! **element 函数的规格表** —— 一行一个函数，唯一那份清单。
//!
//! ⚠ 加一个 element 函数 = **这里加一行** + 写两个文件（`src/<函数>.rs` 参数结构、
//!   `body/<函数>.rs` 算法）。
//! ⚠ **体文件住在 `body/` 下**（不在 `src/`）：它不进本 crate 的源码指纹 ⇒ 改一行算法
//!   只换**那一条**实例的键、只重编那一份库（R1）。

use crate::constant::ConstantParams;

crate::px_elem_specs! {
    /// 一整张常值场：一条链的起点，也是"最简 element 函数"那一档（参数只有形状与值）。
    Constant, ConstantParams, (), "field.constant",
        source: "px_elem/body/constant.rs", roots: &["px_elem"],
        shape: |params: &ConstantParams| params.shape;
}
