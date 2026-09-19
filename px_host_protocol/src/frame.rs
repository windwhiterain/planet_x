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

use px_handshake::ProtocolId;
use px_handshake::{wire, WireError};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{Job, Scene, Shot, View};

    fn request() -> Frame {
        Frame::Request(Request {
            scene: Scene::Artifact {
                scene: "a.pxart".to_string(),
            },
            view: View::default(),
            width: 8,
            height: 8,
            out: "a.png".to_string(),
            job: Job::default(),
            report: String::new(),
        })
    }

    fn text_of(frame: &Frame) -> String {
        let payload = frame.encode().expect("编得出来");
        assert_eq!(payload.first(), Some(&b'J'), "信封前缀不是 `J`");
        String::from_utf8(payload[1..].to_vec()).expect("载荷是 JSON 文本")
    }

    /// **线格式判据**：`J` 前缀 + JSON 标签 + `[u32 小端长度]` 前缀，三样一个都不许动 ——
    /// `tools/harness.ps1` 那套仪器认的就是它们，动了就等于把在跑的旧客户端拒之门外。
    ///
    /// ⚠ 这条同时钉住"搬了家但字节没动"（上一轮把这四种帧从 `px_protocol::stream` 搬到这里）。
    #[test]
    fn the_frame_bytes_are_the_wire_format() {
        let text = text_of(&request());
        assert!(text.contains("\"frame\":\"request\""), "标签动了：{text}");
        assert!(text.contains("\"scene\":\"a.pxart\""), "载荷动了：{text}");

        // 长度前缀 = 载荷（含 `J`）的字节数，小端。
        let payload = request().encode().expect("编得出来");
        let mut wire = Vec::new();
        write_frame(&mut wire, &request()).expect("写得出去");
        assert_eq!(&wire[..4], &(payload.len() as u32).to_le_bytes());
        assert_eq!(&wire[4..], payload.as_slice());
    }

    /// 拒绝那一帧的键是 **`reason`**（`Refused { reason }`），而内存里是 `Refused(String)`
    /// —— 这一层的不对称是线格式定下来的，不是一个可以顺手"修"的瑕疵。
    #[test]
    fn the_refused_frame_carries_a_reason_key() {
        let text = text_of(&Frame::Refused("夹具拒绝".to_string()));
        assert!(text.contains("\"frame\":\"refused\""), "{text}");
        assert!(text.contains("\"reason\""), "{text}");
    }

    /// 往返恒等，而且能**连着读**（服务端就是这么用的）；读空了是 `None`，不是错误。
    #[test]
    fn frames_round_trip_and_the_end_of_stream_is_none() {
        let mut wire = Vec::new();
        write_frame(&mut wire, &request()).expect("写得出去");
        write_frame(&mut wire, &Frame::Refused("夹具拒绝".to_string())).expect("写得出去");

        let mut input = wire.as_slice();
        let first = read_frame(&mut input).expect("读得回来").expect("有第一帧");
        let second = read_frame(&mut input).expect("读得回来").expect("有第二帧");
        match first {
            Frame::Request(Request {
                scene: Scene::Artifact { scene },
                width,
                ..
            }) => {
                assert_eq!(scene, "a.pxart");
                assert_eq!(width, 8);
            }
            other => panic!("读回来变了：{other:?}"),
        }
        assert_eq!(second, Frame::Refused("夹具拒绝".to_string()));
        assert!(read_frame(&mut input).expect("干净结束").is_none());
    }

    /// 握手那一帧（两端各发一份 `Protocol`）。
    #[test]
    fn the_handshake_frame_round_trips() {
        let frame = Frame::Protocol(ProtocolId {
            schema_version: px_handshake::SCHEMA_VERSION,
            protocol_hash: 0xfeed_face_dead_beef,
            git_rev: "abc123".to_string(),
        });
        let text = text_of(&frame);
        assert!(text.contains("\"frame\":\"protocol\""), "{text}");
        let mut wire = Vec::new();
        write_frame(&mut wire, &frame).expect("写得出去");
        assert_eq!(
            read_frame(&mut wire.as_slice()).expect("读得回来"),
            Some(frame)
        );
    }

    /// 批量那一档（`Scene::Sequence`）也要过这条线，`Shot` 整份往返。
    #[test]
    fn a_sequence_request_round_trips_with_its_shots() {
        let batch = Frame::Request(Request {
            scene: Scene::Sequence {
                shots: vec![
                    Shot {
                        scene: "one.pxart".to_string(),
                        out: "one.png".to_string(),
                        cam: None,
                    },
                    Shot {
                        scene: "two.pxart".to_string(),
                        out: "two.png".to_string(),
                        cam: Some([0.0, 5.0, 3.2]),
                    },
                ],
            },
            view: View::default(),
            width: 4,
            height: 4,
            out: "unused.png".to_string(),
            job: Job::Shots,
            report: "r.json".to_string(),
        });
        let mut wire = Vec::new();
        write_frame(&mut wire, &batch).expect("写得出去");
        match read_frame(&mut wire.as_slice()).expect("读得回来") {
            Some(Frame::Request(Request {
                scene: Scene::Sequence { shots },
                ..
            })) => {
                assert_eq!(shots.len(), 2);
                assert_eq!(shots[1].cam, Some([0.0, 5.0, 3.2]));
            }
            other => panic!("读回来变了：{other:?}"),
        }
    }

    /// 截断的载荷要报错（不是当成"没有帧"）—— 否则半截报文会被静默丢掉。
    #[test]
    fn a_truncated_payload_is_an_error_not_an_end() {
        let mut wire = Vec::new();
        write_frame(&mut wire, &request()).expect("写得出去");
        wire.truncate(wire.len() - 2);
        read_frame(&mut wire.as_slice()).expect_err("载荷短了");
    }

    /// 认不出的信封前缀 / 标签都是错误（这一族是**闭合**的：加一种要同时改这条判据）。
    #[test]
    fn an_unknown_prefix_or_tag_is_refused() {
        assert!(matches!(
            Frame::decode(b"X{}"),
            Err(WireError::TruncatedFrame)
        ));
        assert!(matches!(
            Frame::decode(br#"J{"frame":"telemetry","x":1}"#),
            Err(WireError::Json(_))
        ));
    }
}
