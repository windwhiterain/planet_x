//! 渲染服务的信封：`Protocol` / `Request` / `Response` / `Refused` 四种帧与它的长度前缀。
//!
//! ⚠ 这一份原来住在 `px_protocol::stream::Frame`（那张表当时八种帧）。跨进程边界一收窄成
//! "px-scene ⇄ px-pass"，作业那两路就跟着作业形状搬来这里 —— 留在 `px_protocol` 只会让它
//! 反向依赖 `px_render`，那是编都编不过的循环。
//!
//! ⚠ **搬的是位置，不是字节**：JSON 标签（`"frame"` = `protocol` / `request` / `response` /
//! `refused`）与信封（`[u32 小端长度][载荷]`）逐字照旧 —— `tools/harness.ps1` 那套仪器认的就是它。

use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};

use px_protocol::ProtocolId;
use px_protocol::wire::{self, WireError};

use crate::render::{Request, Response};

#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    Protocol(ProtocolId),
    Request(Request),
    Response(Response),
    Refused(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "frame", rename_all = "snake_case")]
enum TextFrame {
    Protocol(ProtocolId),
    Request(Request),
    Response(Response),
    Refused { reason: String },
}

impl Frame {
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        match self {
            Self::Protocol(inner) => Self::encode_text(TextFrame::Protocol(inner.clone())),
            Self::Request(inner) => Self::encode_text(TextFrame::Request(inner.clone())),
            Self::Response(inner) => Self::encode_text(TextFrame::Response(inner.clone())),
            Self::Refused(reason) => Self::encode_text(TextFrame::Refused {
                reason: reason.clone(),
            }),
        }
    }

    pub fn decode(buf: &[u8]) -> Result<Self, WireError> {
        match buf.split_first() {
            Some((b'J', rest)) => {
                let text =
                    std::str::from_utf8(rest).map_err(|err| WireError::Json(err.to_string()))?;
                Ok(match wire::from_text::<TextFrame>(text)? {
                    TextFrame::Protocol(inner) => Self::Protocol(inner),
                    TextFrame::Request(inner) => Self::Request(inner),
                    TextFrame::Response(inner) => Self::Response(inner),
                    TextFrame::Refused { reason } => Self::Refused(reason),
                })
            }
            _ => Err(WireError::TruncatedFrame),
        }
    }

    fn encode_text(frame: TextFrame) -> Result<Vec<u8>, WireError> {
        let mut out = vec![b'J'];
        out.extend_from_slice(wire::to_text(&frame)?.as_bytes());
        Ok(out)
    }
}

pub fn write_frame<W: Write>(writer: &mut W, frame: &Frame) -> Result<(), WireError> {
    let payload = frame.encode()?;
    writer
        .write_all(&(payload.len() as u32).to_le_bytes())
        .map_err(io_error)?;
    writer.write_all(&payload).map_err(io_error)?;
    Ok(())
}

pub fn read_frame<R: Read>(reader: &mut R) -> Result<Option<Frame>, WireError> {
    let mut len = [0u8; 4];
    match reader.read_exact(&mut len) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(err) => return Err(io_error(err)),
    }
    let len = u32::from_le_bytes(len) as usize;
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).map_err(io_error)?;
    Ok(Some(Frame::decode(&payload)?))
}

fn io_error(err: io::Error) -> WireError {
    WireError::Io(err.to_string())
}
