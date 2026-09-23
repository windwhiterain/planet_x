//! `sky.stars`：**世界坐标里的一批点光源 → 一个 R3 星场**（`StarField` 载荷）。
//!
//! ⚠ 这一档**不是场算子**（尽管它的前身 `field.stars` 是）：输出是 `StarField`
//!   （星表 + 稀疏均匀三维格），不是一张 `width × height` 的画布。见
//!   `px_volume_alg::stars` 的文件头 —— 那里写着"为什么不能再画进一张立方图"。
//!
//! ⚠ 画布（`grid`）**不参与**：星的位置由参数里的世界半径与种子决定
//!   （`PxOp::Payload::RESOLUTION_IS_CANVAS = false`）⇒ 同一份参数在**任何**图里
//!   都解析到同一个键。这正是"两张图共用一份星场"能成立的原因。

use px_volume_schema::ops::Stars;

px_graph_schema::px_body! {
    Stars,
    |p, _i, _g| px_volume_alg::bake_stars(p)?
}
