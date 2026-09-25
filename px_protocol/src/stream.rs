use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};

use crate::scene::SceneSpec;
use crate::wire::{self, Blob, WireError};
use crate::{ArtBundle, ProtocolId};

pub const MAGIC: [u8; 4] = *b"PXST";
pub const STREAM_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    Protocol(ProtocolId),
    Art(ArtBundle),
    Scene(SceneSpec),
    Blob(Blob),
    Refused(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "frame", rename_all = "snake_case")]
enum TextFrame {
    Protocol(ProtocolId),
    Art(ArtBundle),
    Scene(SceneSpec),
    Refused { reason: String },
}

impl Frame {
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        match self {
            Self::Blob(blob) => {
                let mut out = vec![b'B'];
                out.extend_from_slice(&blob.encode()?);
                Ok(out)
            }
            Self::Protocol(inner) => Self::encode_text(TextFrame::Protocol(inner.clone())),
            Self::Art(inner) => Self::encode_text(TextFrame::Art(inner.clone())),
            Self::Scene(inner) => Self::encode_text(TextFrame::Scene(inner.clone())),
            Self::Refused(reason) => Self::encode_text(TextFrame::Refused {
                reason: reason.clone(),
            }),
        }
    }

    pub fn decode(buf: &[u8]) -> Result<Self, WireError> {
        match buf.split_first() {
            Some((b'B', rest)) => Ok(Self::Blob(Blob::decode(rest)?)),
            Some((b'J', rest)) => {
                let text =
                    std::str::from_utf8(rest).map_err(|err| WireError::Json(err.to_string()))?;
                Ok(match wire::from_text::<TextFrame>(text)? {
                    TextFrame::Protocol(inner) => Self::Protocol(inner),
                    TextFrame::Art(inner) => Self::Art(inner),
                    TextFrame::Scene(inner) => Self::Scene(inner),
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

impl From<ProtocolId> for Frame {
    fn from(id: ProtocolId) -> Self {
        Self::Protocol(id)
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

pub fn write_stream<W: Write>(writer: &mut W, frames: &[Frame]) -> Result<(), WireError> {
    writer.write_all(&MAGIC).map_err(io_error)?;
    writer
        .write_all(&STREAM_VERSION.to_le_bytes())
        .map_err(io_error)?;
    for frame in frames {
        write_frame(writer, frame)?;
    }
    Ok(())
}

pub fn read_stream<R: Read>(reader: &mut R) -> Result<Vec<Frame>, WireError> {
    let mut magic = [0u8; 4];
    reader.read_exact(&mut magic).map_err(io_error)?;
    if magic != MAGIC {
        return Err(WireError::BadMagic);
    }
    let mut version = [0u8; 4];
    reader.read_exact(&mut version).map_err(io_error)?;
    let version = u32::from_le_bytes(version);
    if version != STREAM_VERSION {
        return Err(WireError::BadStreamVersion(version));
    }
    let mut frames = Vec::new();
    while let Some(frame) = read_frame(reader)? {
        frames.push(frame);
    }
    Ok(frames)
}

pub fn declared_protocol(path: &std::path::Path) -> Result<Option<ProtocolId>, WireError> {
    let bytes = std::fs::read(path).map_err(io_error)?;
    Ok(read_stream(&mut bytes.as_slice())?
        .into_iter()
        .find_map(|frame| match frame {
            Frame::Protocol(id) => Some(id),
            _ => None,
        }))
}

fn io_error(err: io::Error) -> WireError {
    WireError::Io(err.to_string())
}
