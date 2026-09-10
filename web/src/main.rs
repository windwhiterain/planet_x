//! `planet_x_web` — the WebUI server for Planet X.
//!
//! Serves a JSON API plus the static frontend (`web/static/`) from the crate's
//! `static/` directory. It holds one authoritative world in memory. Run it and
//! open the printed URL.
//!
//! **起它请用 `scripts/web.ps1`**（git_bash 用 `scripts/web.sh`）：那个脚本会在
//! `cargo` 之前先清掉本 worktree 里还在跑的旧实例（旧进程会锁住
//! `target\debug\planet_x_web.exe`，让重新编译换不掉顶层二进制），并把
//! `PLANET_X_WEB_OWNER_PID` 交给本进程当租约。
//!
//! Environment overrides:
//! * `PLANET_X_CONFIG`       path to `config/game.ron`.
//! * `PLANET_X_START`        optional initial-state RON file.
//! * `PLANET_X_SEED`         deterministic seed (a number or `random`).
//! * `PLANET_X_WEB_PORT`     listen port. **不设 = 自动**：从 [`DEFAULT_PORT`] 起向上扫
//!   一个空闲端口（`3000` 被占就 `3001`、`3002`…），全满则交给 OS 挑临时端口。
//!   设成一个数字 = **就要这个**：被占直接报错（不会被悄悄换掉）。设成 `auto` 同上。
//! * `PLANET_X_WEB_STATIC`   static dir to serve (default `<crate>/static`).
//! * `PLANET_X_WEB_OWNER_PID` **启动者租约**：这个 pid 一结束，本服务立刻自退
//!   （见 [`planet_x_web::owner`]）。没设 = 没人看护（手动直接跑 exe 就是这种）。
//! * `PLANET_X_WEB_CLOSE_EXIT` `0`/`false`/`off` = 关掉「最后一个页面关闭后自退」。
//! * `PLANET_X_WEB_CLOSE_GRACE_MS` 关掉最后一个页面后的**刷新窗口**（默认 `500`）：
//!   只为吸收「旧页面注销先到、新页面登记后到」这个交错；`0` = 立刻退（刷新会带走服务）。
//!
//! 生命周期只有这两把锁，**都没有空闲计时器**：启动者一没就退，最后一个页面走了就退。

use planet_x::config::{load_config, load_state, parse_seed};
use planet_x::prng::{Prng, random_seed};
use planet_x::world;
use planet_x_web::{GameWorld, Shared, WebCtx, bind_auto, owner, router};
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

/// 秒 → 人看的一句话（`42s` / `3m20s` / `2h05m`）。
///
/// 用在启动横幅与 `/api/ping` 的「二进制构建于多久前」上——那正是识别「页面里跑的
/// 是不是刚编的那个」的关键线索（旧实例锁住 exe 时，症状就是这里显示一个很大的数）。
fn human_age(secs: u64) -> String {
    match secs {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m{:02}s", s / 60, s % 60),
        s => format!("{}h{:02}m", s / 3600, (s % 3600) / 60),
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

    // 生命周期：身份 + 页面登记 + 退出闸门。`exit_rx` 就是给 axum 的优雅停机信号。
    let owner_pid =
        owner::parse_owner_pid(std::env::var("PLANET_X_WEB_OWNER_PID").ok().as_deref());
    let (web, exit_rx) = WebCtx::new(port, owner_pid);
    let app = router(shared, web.clone());

    // 启动者租约：拉我起来的那个进程一结束就退。这是「会话/job 没了，exe 却继续监听」
    // 的正解——Windows 不会替你收孙进程（本仓真丢过一个 10:43 起的孤儿）。
    if let Some(pid) = owner_pid {
        let gate = web.exit_gate();
        std::thread::spawn(move || {
            owner::wait_for_exit(pid);
            gate.request(format!("启动它的进程 pid {pid} 已退出"));
        });
    }

    let id = web.identity();
    println!("行星X WebUI 运行于 http://{host}:{port}");
    if port != DEFAULT_PORT {
        println!(
            "  （{DEFAULT_PORT} 已被占用 → 自动改用 {port}；钉死端口用 PLANET_X_WEB_PORT={port}）"
        );
    }
    println!("  种子 : {}", seed);
    match id.exe_age_secs {
        Some(age) => println!(
            "  pid  : {}   二进制 : {}（构建于 {} 前）",
            id.pid,
            id.exe,
            human_age(age)
        ),
        None => println!("  pid  : {}   二进制 : {}", id.pid, id.exe),
    }
    match owner_pid {
        Some(p) => {
            println!("  自退 : 启动它的 pid {p} 一退出就退；最后一个页面关闭后也退（无空闲计时器）")
        }
        None => {
            println!("  自退 : 最后一个页面关闭后退出（没设 PLANET_X_WEB_OWNER_PID，无启动者租约）")
        }
    }
    // 给脚本/agent 一行可以直接抠的地址（人看的那行保持原样，别让它们互相猜）。
    println!("PLANET_X_WEB_URL=http://{host}:{port}");
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            if let Ok(why) = exit_rx.await {
                println!("行星X WebUI 退出：{why}");
            }
        })
        .await?;
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

    /// 「构建于多久前」那行要一眼能读：秒 / 分秒 / 小时分。
    #[test]
    fn human_age_reads_at_a_glance() {
        assert_eq!(human_age(0), "0s");
        assert_eq!(human_age(42), "42s");
        assert_eq!(human_age(200), "3m20s");
        assert_eq!(human_age(7500), "2h05m");
    }
}
