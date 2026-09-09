//! planet_x — a sandbox trajectory generator for the Planet X game.
//!
//! This library crate holds the simulation model and engine together with a
//! small CLI and an HTTP API. Both the `planet_x` CLI binary and the
//! `planet_x_web` server binary are thin shells over it.

pub mod agent;
pub mod config;
pub mod model;
pub mod projection;
pub mod prng;
pub mod sim;
pub mod visual;
pub mod web;
pub mod world;
