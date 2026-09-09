//! `planet_x_web` — the WebUI server for Planet X.
//!
//! Serves a JSON API plus the static frontend (`web/static/`) from the crate's
//! `static/` directory. It holds one authoritative world in memory. Run it and
//! open the printed URL.
//!
//! Environment overrides:
//! * `PLANET_X_CONFIG`       path to `config/game.ron`.
//! * `PLANET_X_START`        optional initial-state RON file.
//! * `PLANET_X_SEED`         deterministic seed (a number or `random`).
//! * `PLANET_X_WEB_PORT`     listen port (default `3000`).
//! * `PLANET_X_WEB_STATIC`   static dir to serve (default `<crate>/static`).

use planet_x::config::{load_config, load_state, parse_seed};
use planet_x::prng::{random_seed, Prng};
use planet_x::world;
use planet_x_web::{router, GameWorld, Shared};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = load_config();

    let seed: u64 = match std::env::var("PLANET_X_SEED") {
        Ok(v) => parse_seed(&v),
        Err(_) => random_seed(),
    };

    let state = match std::env::var_os("PLANET_X_START") {
        Some(p) => load_state(&PathBuf::from(p)),
        None => world::default_state(&config, seed),
    };

    let port = std::env::var("PLANET_X_WEB_PORT").unwrap_or_else(|_| "3000".to_string());
    let addr = format!("127.0.0.1:{port}");

    let world = GameWorld { state, config, rng: Prng::new(seed) };
    let shared: Shared = Arc::new(Mutex::new(world));

    let app = router(shared);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("行星X WebUI 运行于 http://{addr}");
    println!("  种子 : {}", seed);
    axum::serve(listener, app).await?;
    Ok(())
}
