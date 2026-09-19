//! **宿主协议面**：渲染宿主 ⇄ 它的客户端之间那一份形状契约（作业请求 / 回读报告 / 租约 /
//! 四种帧与它的长度前缀）。搬出来的理由有两条，缺一条这个 crate 都不该存在：
//!
//! - ⚠ **不能住在 `px_protocol` 里**：那一份的职责只有一句话 —— px-scene ⇄ px-pass 的交换边界。
//!   作业请求与回读报告是宿主**自己**的事（谁能渲、渲完回什么话），只有宿主与它的客户端认；
//!   边界每多装一样，跨进程握手就要为一个只有一边认的形状背一次版本。更要紧的是方向：
//!   `px_render` 本来就依赖 `px_protocol`，这些形状再留在那边就是循环依赖，编都编不过。
//! - ⚠ **也不能住在 `px_render` 里**（那是上一版的摆法）：`px_protocol` 的快照测试必须按**真类型**
//!   构造这些形状 —— 快照逐字节钉着它们，而 `protocol_hash()` 就是那份快照的哈希。把它们放进
//!   宿主，那条 dev 边就是 `px_protocol` → `px_render` → wgpu / naga / winit：**协议测试要编一整套
//!   GPU 栈**，依赖方向也整个反过来。落在这里，`px_protocol` 那条 dev 边上只剩纯数据。
//!
//! 三个模块与它们在 `px_render` 里的旧名字一一对应：`render`（旧 `render_proto`：作业形状）、
//! `frame`（旧 `render_frame`：`Protocol` / `Request` / `Response` / `Refused` 与
//! `[u32 小端长度][载荷]` 的信封）、`client`（旧 `client_proto`：租约路径 + 连接/握手/请求）。
//!
//! ⚠ **搬的是位置，不是字节**：JSON 标签、字段名与信封逐字照旧 ——
//! `px_protocol/snapshots/protocol.snapshot.json` 与 `tools/harness.ps1` 认的就是它们。

pub mod client;
pub mod frame;
pub mod render;
