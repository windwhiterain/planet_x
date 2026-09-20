//! 载荷的线格式**住在 `px_protocol`**：`PayloadBundle` 就是 CAS 里那份文件的形状，
//! 各域的 `Build` 实现也跟着**它们的载荷类型**走（孤儿规则那条口径）。
//!
//! 这里只把它接到契约层的路径上 —— `px_graph_schema::payload::{Build, PayloadBundle}`
//! 这两个名字照旧可用，别处的 import 不必改。

pub use px_protocol::payload::{Build, PayloadBundle};
