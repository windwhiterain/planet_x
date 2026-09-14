use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Scene {
    World {
        stream: String,
        round: Option<u32>,
    },
    Planet {
        field: String,
        #[serde(default)]
        mesh: Option<String>,
        #[serde(default)]
        clouds: Option<String>,
        #[serde(default)]
        slope: Option<[String; 3]>,
        palette: String,
        displace: f32,
        sea_level: f32,
        radius: f32,
        spin: f32,
        rings: f32,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct View {
    #[serde(default)]
    pub ambient: Option<f32>,
    #[serde(default)]
    pub cam: Option<[f32; 3]>,
    #[serde(default)]
    pub atmo: Option<f32>,
    #[serde(default)]
    pub scatter: Option<String>,
    #[serde(default)]
    pub cloud: Option<f32>,
    /// 用产物自带的相机表出「一张多视角对照图」（相机表住在 `.pxart` 里）。
    #[serde(default)]
    pub sheet: bool,
    /// 对照图的列数（sheet 为真时有效）。
    #[serde(default)]
    pub columns: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub scene: Scene,
    #[serde(default)]
    pub view: View,
    pub width: u32,
    pub height: u32,
    pub out: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub out: String,
    pub scene: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub millis: u64,
    pub warm: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lease {
    pub pid: u32,
    pub port: u16,
    pub protocol_hash: u64,
    pub git_rev: String,
    pub exe: String,
}

impl Lease {
    pub fn matches(&self, id: &crate::ProtocolId) -> bool {
        self.protocol_hash == id.protocol_hash && self.git_rev == id.git_rev
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientError {
    NoServer,
    Refused(String),
    Wire(String),
    Io(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoServer => write!(formatter, "没有在跑的渲染服务"),
            Self::Refused(reason) => write!(formatter, "渲染服务拒绝：{reason}"),
            Self::Wire(message) => write!(formatter, "协议失败：{message}"),
            Self::Io(message) => write!(formatter, "IO 失败：{message}"),
        }
    }
}

impl std::error::Error for ClientError {}





