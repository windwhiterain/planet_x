//! 渲染服务的客户端那一半（读租约 → 连端口 → 握手 → 发请求），原 `px_protocol::client`。
//!
//! ⚠ 与 `render` 同一条理由：租约里写着 `ProtocolId`，而作业请求是宿主自己的形状 ——
//! 它留在 `px_protocol` 就是宿主 ⇄ 协议的循环依赖。**跨进程的握手本身没有搬**：
//! `ProtocolId` / `Frame::Protocol` / 信封长度前缀仍然来自 `px_protocol`。

use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::ProtocolId;

use crate::frame::{self, Frame};
use crate::render::{ClientError, Lease, Request, Response};

pub const LEASE_PATH: &str = "target/render-server.json";
const CONNECT_TIMEOUT: Duration = Duration::from_millis(400);
const IO_TIMEOUT: Duration = Duration::from_secs(180);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(90);

pub fn lease_path() -> PathBuf {
    PathBuf::from(LEASE_PATH)
}

pub fn read_lease(path: &Path) -> Option<Lease> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn write_lease(path: &Path, lease: &Lease) -> Result<(), ClientError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io)?;
    }
    let text =
        serde_json::to_string_pretty(lease).map_err(|err| ClientError::Wire(err.to_string()))?;
    std::fs::write(path, text).map_err(io)
}

fn handshake(stream: &mut TcpStream) -> Result<(), ClientError> {
    let local = ProtocolId::local();
    frame::write_frame(stream, &Frame::Protocol(local.clone())).map_err(wire)?;
    match frame::read_frame(stream).map_err(wire)? {
        Some(Frame::Protocol(remote)) => {
            if remote.schema_version != local.schema_version
                || remote.protocol_hash != local.protocol_hash
            {
                return Err(ClientError::Refused(format!(
                    "协议不一致：对端 schema {} / 指纹 {:016x} / git {}",
                    remote.schema_version, remote.protocol_hash, remote.git_rev
                )));
            }
            Ok(())
        }
        Some(Frame::Refused(reason)) => Err(ClientError::Refused(reason)),
        other => Err(ClientError::Wire(format!(
            "握手时期望 Protocol 帧，收到 {other:?}"
        ))),
    }
}

fn spawn_server() -> Result<(), ClientError> {
    let exe = std::env::current_exe().map_err(io)?;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("target/render-server.log")
        .map_err(io)?;
    let log_err = log.try_clone().map_err(io)?;
    std::process::Command::new(exe)
        .arg("--serve")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(log))
        .stderr(std::process::Stdio::from(log_err))
        .spawn()
        .map_err(io)?;
    Ok(())
}

pub fn connect() -> Result<TcpStream, ClientError> {
    let path = lease_path();
    let local = ProtocolId::local();

    let Some(lease) = read_lease(&path) else {
        return Err(ClientError::NoServer);
    };
    if !lease.matches(&local) {
        return Err(ClientError::Refused(format!(
            "在跑的服务是 git {} / 指纹 {:016x}，本进程是 git {} / 指纹 {:016x}",
            lease.git_rev, lease.protocol_hash, local.git_rev, local.protocol_hash,
        )));
    }
    let Some(mut stream) = connect_port(lease.port) else {
        return Err(ClientError::NoServer);
    };
    handshake(&mut stream)?;
    Ok(stream)
}

pub fn connect_or_start() -> Result<TcpStream, ClientError> {
    if let Ok(stream) = connect() {
        return Ok(stream);
    }

    spawn_server()?;
    let local = ProtocolId::local();
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(150));
        let Some(lease) = read_lease(&lease_path()) else {
            continue;
        };
        if !lease.matches(&local) {
            continue;
        }
        if let Some(mut stream) = connect_port(lease.port) {
            if handshake(&mut stream).is_ok() {
                return Ok(stream);
            }
        }
    }
    Err(ClientError::NoServer)
}

fn connect_port(port: u16) -> Option<TcpStream> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).ok()?;
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    Some(stream)
}

pub fn request(request: Request) -> Result<Response, ClientError> {
    request_with(request, false)
}

pub fn request_with(request: Request, autostart: bool) -> Result<Response, ClientError> {
    let mut stream = if autostart {
        connect_or_start()?
    } else {
        connect()?
    };
    frame::write_frame(&mut stream, &Frame::Request(request)).map_err(wire)?;
    match frame::read_frame(&mut stream).map_err(wire)? {
        Some(Frame::Response(response)) => Ok(response),
        Some(Frame::Refused(reason)) => Err(ClientError::Refused(reason)),
        other => Err(ClientError::Wire(format!(
            "请求时期望 Response 帧，收到 {other:?}"
        ))),
    }
}

fn io(err: std::io::Error) -> ClientError {
    ClientError::Io(err.to_string())
}

fn wire(err: crate::WireError) -> ClientError {
    ClientError::Wire(err.to_string())
}
