use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};

use crate::wire::{self, WireError};
use crate::{ArtBundle, Blob, ProtocolId, WorldView};

pub const MAGIC: [u8; 4] = *b"PXST";
pub const STREAM_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    Protocol(ProtocolId),
    World(WorldView),
    Art(ArtBundle),
    Blob(Blob),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "frame", rename_all = "snake_case")]
enum TextFrame {
    Protocol(ProtocolId),
    World(WorldView),
    Art(ArtBundle),
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
            Self::World(inner) => Self::encode_text(TextFrame::World(inner.clone())),
            Self::Art(inner) => Self::encode_text(TextFrame::Art(inner.clone())),
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
                    TextFrame::World(inner) => Self::World(inner),
                    TextFrame::Art(inner) => Self::Art(inner),
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

impl From<WorldView> for Frame {
    fn from(world: WorldView) -> Self {
        Self::World(world)
    }
}

pub fn write_stream<W: Write>(writer: &mut W, frames: &[Frame]) -> Result<(), WireError> {
    writer.write_all(&MAGIC).map_err(io_error)?;
    writer
        .write_all(&STREAM_VERSION.to_le_bytes())
        .map_err(io_error)?;
    for frame in frames {
        let payload = frame.encode()?;
        writer
            .write_all(&(payload.len() as u32).to_le_bytes())
            .map_err(io_error)?;
        writer.write_all(&payload).map_err(io_error)?;
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
    loop {
        let mut len = [0u8; 4];
        match reader.read_exact(&mut len) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(err) => return Err(io_error(err)),
        }
        let len = u32::from_le_bytes(len) as usize;
        let mut payload = vec![0u8; len];
        reader.read_exact(&mut payload).map_err(io_error)?;
        frames.push(Frame::decode(&payload)?);
    }
    Ok(frames)
}

fn io_error(err: io::Error) -> WireError {
    WireError::Io(err.to_string())
}
