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
//! * `PLANET_X_WEB_PORT`     listen port. **不设 = 自动**：从 [`DEFAULT_PORT`] 起向上扫
//!   一个空闲端口（`3000` 被占就 `3001`、`3002`…），全满则交给 OS 挑临时端口。
//!   设成一个数字 = **就要这个**：被占直接报错（不会被悄悄换掉）。设成 `auto` 同上。
//! * `PLANET_X_WEB_STATIC`   static dir to serve (default `<crate>/static`).

use planet_x::config::{load_config, load_state, parse_seed};
use planet_x::prng::{Prng, random_seed};
use planet_x::world;
use planet_x_web::{GameWorld, Shared, bind_auto, router};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

/// 自动模式从哪里开始找端口（也是从前的固定端口，空着时行为与从前一致）。
const DEFAULT_PORT: u16 = 3000;
/// 自动模式最多向上试几个端口；全满才退到 OS 临时端口。
const AUTO_SCAN: u16 = 100;

/// 用户对端口的意图：`PLANET_X_WEB_PORT` 的解析结果。
#[derive(Debug, PartialEq)]
enum PortChoice {
    /// 没设 / 设成 `auto`：向上扫一个空闲端口。
    Auto,
    /// 明确要这个端口（`0` = 让 OS 挑）：绑不上就失败。
    Exact(u16),
}

/// 解析 `PLANET_X_WEB_PORT`。`Err` 带上原文，好让人一眼看出写错了什么。
fn parse_port_choice(raw: Option<&str>) -> Result<PortChoice, String> {
    match raw {
        None => Ok(PortChoice::Auto),
        Some(s) if s.trim().is_empty() || s.trim().eq_ignore_ascii_case("auto") => {
            Ok(PortChoice::Auto)
        }
        Some(s) => s
            .trim()
            .parse::<u16>()
            .map(PortChoice::Exact)
            .map_err(|_| s.to_string()),
    }
}

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

    let host = "127.0.0.1";
    let raw_port = std::env::var("PLANET_X_WEB_PORT").ok();
    let listener = match parse_port_choice(raw_port.as_deref())
        .map_err(|bad| format!("PLANET_X_WEB_PORT=`{bad}` 不是端口号（1-65535，或 `auto`）"))?
    {
        PortChoice::Exact(port) => TcpListener::bind((host, port)).await.map_err(|e| {
            format!(
                "绑不上 {host}:{port}：{e}（端口是你点名的，所以不会自动换；\
                 去掉 PLANET_X_WEB_PORT 或设成 `auto` 就会自动挑一个空闲端口）"
            )
        })?,
        PortChoice::Auto => bind_auto(host, DEFAULT_PORT, AUTO_SCAN)
            .await
            .map_err(|e| {
                format!("{host} 上从 {DEFAULT_PORT} 起扫了 {AUTO_SCAN} 个端口都绑不上：{e}")
            })?,
    };
    let port = listener.local_addr()?.port();

    let world = GameWorld::new(state, config, Prng::new(seed));
    let shared: Shared = Arc::new(Mutex::new(world));

    let app = router(shared);
    println!("行星X WebUI 运行于 http://{host}:{port}");
    if port != DEFAULT_PORT {
        println!(
            "  （{DEFAULT_PORT} 已被占用 → 自动改用 {port}；钉死端口用 PLANET_X_WEB_PORT={port}）"
        );
    }
    println!("  种子 : {}", seed);
    // 给脚本/agent 一行可以直接抠的地址（人看的那行保持原样，别让它们互相猜）。
    println!("PLANET_X_WEB_URL=http://{host}:{port}");
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 没设 = 自动；`auto`（含大小写/空白）= 自动；数字 = 点名；别的 = 报错。
    #[test]
    fn port_env_parsing() {
        assert_eq!(parse_port_choice(None), Ok(PortChoice::Auto));
        assert_eq!(parse_port_choice(Some("")), Ok(PortChoice::Auto));
        assert_eq!(parse_port_choice(Some(" auto ")), Ok(PortChoice::Auto));
        assert_eq!(parse_port_choice(Some("AUTO")), Ok(PortChoice::Auto));
        assert_eq!(parse_port_choice(Some("3011")), Ok(PortChoice::Exact(3011)));
        assert_eq!(parse_port_choice(Some(" 0 ")), Ok(PortChoice::Exact(0)));
        assert_eq!(parse_port_choice(Some("70000")), Err("70000".to_string()));
        assert_eq!(
            parse_port_choice(Some("http://x")),
            Err("http://x".to_string())
        );
    }
}
