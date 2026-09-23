use serde::{Serialize, de::DeserializeOwned};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    Json(String),
    Io(String),
    MissingHeader,
    BadPayloadLength { expected: usize, actual: usize },
    NotF32(DType),
    NotU32(DType),
    BadMagic,
    BadStreamVersion(u32),
    TruncatedFrame,
}

impl std::fmt::Display for WireError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(message) => write!(formatter, "JSON 编解码失败：{message}"),
            Self::Io(message) => write!(formatter, "IO 失败：{message}"),
            Self::MissingHeader => write!(formatter, "二进制块缺少头部行"),
            Self::BadPayloadLength { expected, actual } => {
                write!(
                    formatter,
                    "二进制块长度不符：头部声明 {expected} 字节，实际 {actual} 字节"
                )
            }
            Self::NotF32(dtype) => write!(formatter, "期望 f32 载荷，实际 {dtype:?}"),
            Self::NotU32(dtype) => write!(formatter, "期望 u32 载荷，实际 {dtype:?}"),
            Self::BadMagic => write!(formatter, "不是 .pxstream：魔数不符"),
            Self::BadStreamVersion(version) => write!(formatter, "流版本不支持：{version}"),
            Self::TruncatedFrame => write!(formatter, "帧被截断或标签未知"),
        }
    }
}

impl std::error::Error for WireError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
pub enum DType {
    F32,
    F64,
    U8,
    U16,
    U32,
}

impl DType {
    pub fn elem_size(self) -> usize {
        match self {
            Self::F32 | Self::U32 => 4,
            Self::F64 => 8,
            Self::U8 => 1,
            Self::U16 => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct BlobHeader {
    pub dtype: DType,
    pub shape: Vec<u32>,
}

impl BlobHeader {
    pub fn elems(&self) -> usize {
        self.shape.iter().map(|dim| *dim as usize).product()
    }

    pub fn byte_len(&self) -> usize {
        self.elems() * self.dtype.elem_size()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Blob {
    pub header: BlobHeader,
    pub bytes: Vec<u8>,
}

impl Blob {
    pub fn new(header: BlobHeader, bytes: Vec<u8>) -> Result<Self, WireError> {
        let expected = header.byte_len();
        if bytes.len() != expected {
            return Err(WireError::BadPayloadLength {
                expected,
                actual: bytes.len(),
            });
        }
        Ok(Self { header, bytes })
    }

    pub fn from_f32(shape: Vec<u32>, data: &[f32]) -> Self {
        // ⚠⚠ 这里**绕开了 `Blob::new` 的长度自检**，而绕开过一次的代价是：
        //   `VolumeData` 曾把六通道的字节塞进单通道的形状 ⇒ 产物自相矛盾、
        //   读回被拒，而症状只是"每次烘图都重算"（不报错、不崩溃）。
        //   ⇒ 自检在这里也补一道（debug + 测试里都走）：形状的元素数必须等于 `data`。
        debug_assert_eq!(
            shape.iter().product::<u32>() as usize,
            data.len(),
            "blob 形状 {shape:?} 装不下 {} 个 f32",
            data.len()
        );
        let mut bytes = Vec::with_capacity(data.len() * 4);
        for value in data {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        Self {
            header: BlobHeader {
                dtype: DType::F32,
                shape,
            },
            bytes,
        }
    }

    pub fn f32s(&self) -> Result<Vec<f32>, WireError> {
        if self.header.dtype != DType::F32 {
            return Err(WireError::NotF32(self.header.dtype));
        }
        Ok(self
            .bytes
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect())
    }

    pub fn from_u32(shape: Vec<u32>, data: &[u32]) -> Self {
        let mut bytes = Vec::with_capacity(data.len() * 4);
        for value in data {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        Self {
            header: BlobHeader {
                dtype: DType::U32,
                shape,
            },
            bytes,
        }
    }

    pub fn u32s(&self) -> Result<Vec<u32>, WireError> {
        if self.header.dtype != DType::U32 {
            return Err(WireError::NotU32(self.header.dtype));
        }
        Ok(self
            .bytes
            .chunks_exact(4)
            .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect())
    }

    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let header = to_text(&self.header)?;
        let mut out = Vec::with_capacity(header.len() + 1 + self.bytes.len());
        out.extend_from_slice(header.as_bytes());
        out.push(b'\n');
        out.extend_from_slice(&self.bytes);
        Ok(out)
    }

    pub fn decode(buf: &[u8]) -> Result<Self, WireError> {
        let split = buf
            .iter()
            .position(|byte| *byte == b'\n')
            .ok_or(WireError::MissingHeader)?;
        let header: BlobHeader = from_text(std::str::from_utf8(&buf[..split]).map_err(utf8)?)?;
        let bytes = buf[split + 1..].to_vec();
        Self::new(header, bytes)
    }
}

pub fn to_text<T: Serialize>(value: &T) -> Result<String, WireError> {
    serde_json::to_string(value).map_err(|err| WireError::Json(err.to_string()))
}

pub fn from_text<T: DeserializeOwned>(text: &str) -> Result<T, WireError> {
    serde_json::from_str(text).map_err(|err| WireError::Json(err.to_string()))
}

fn utf8(err: std::str::Utf8Error) -> WireError {
    WireError::Json(err.to_string())
}
