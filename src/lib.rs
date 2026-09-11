//! planet_x — a sandbox trajectory generator for the Planet X game.
//!
//! This library crate holds the simulation model and engine together with the
//! agent-facing CLI. It is deliberately **free of any HTTP/web dependency**
//! (no axum/tokio/tower-http): the `planet_x` CLI binary is a thin shell over
//! it, and the player-facing WebUI lives in the separate `planet_x_web` crate
//! (the `web/` workspace member) which depends on this crate as a library.

// `projection_schema()` 用 `serde_json::json!` 手写整张 schema，而设计图那一轮把 `ships`
// 的 `column_docs` 写厚了 ⇒ 宏展开的递归深度越过了默认的 128（`json!` 每多一个键就多一层
// 展开）。抬到 256 是最小改动：编译产物与行为都不变，只是允许这份数据字面量更长。
#![recursion_limit = "256"]

pub mod agent;
pub mod autocontrol;
pub mod config;
pub mod control;
pub mod json;
pub mod model;
pub mod prng;
pub mod projection;
pub mod schema;
pub mod sim;
pub mod visual;
pub mod world;
