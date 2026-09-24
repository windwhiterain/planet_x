//! 载荷的两个名字**从契约层的路径上**够得到（`px_graph_schema::payload::{Build, PayloadBundle}`）：
//!
//! * `Build` 是**烘图契约**，定义在同目录的 [`crate::build`]；
//! * `PayloadBundle` 是**线格式**，定义在 `px_protocol::payload`（CAS 里那份文件的形状，
//!   渲染侧也读它 —— 那条边不该经过契约层）。
//!
//! 这一层只负责把两个名字接到一条 import 路径上，别处的 `use` 不必改。

pub use crate::build::Build;
pub use px_protocol::payload::PayloadBundle;
