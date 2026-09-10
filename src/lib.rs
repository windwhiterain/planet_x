//! planet_x — a sandbox trajectory generator for the Planet X game.
//!
//! This library crate holds the simulation model and engine together with the
//! agent-facing CLI. It is deliberately **free of any HTTP/web dependency**
//! (no axum/tokio/tower-http): the `planet_x` CLI binary is a thin shell over
//! it, and the player-facing WebUI lives in the separate `planet_x_web` crate
//! (the `web/` workspace member) which depends on this crate as a library.

pub mod agent;
pub mod autocontrol;
pub mod config;
pub mod control;
pub mod json;
pub mod model;
pub mod projection;
pub mod prng;
pub mod sim;
pub mod visual;
pub mod world;
