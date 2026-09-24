//! NURBS 的算子：**一个文件一条声明**（`px_body!` 在模块里只定义一次 `__px_body`，
//! 所以两条实现不能住同一个模块 —— 这是契约层的形状，不是这里的偏好）。

pub mod circle;
pub mod curve_at;
pub mod curve_elevate;
pub mod curve_eval;
pub mod curve_hodograph;
pub mod curve_insert;
pub mod curve_tessellate;
pub mod sphere;
pub mod surface_at;
pub mod surface_elevate;
pub mod surface_eval;
pub mod surface_insert;
pub mod surface_tessellate;
