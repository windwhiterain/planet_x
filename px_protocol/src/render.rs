use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Scene {
    World { stream: String, round: Option<u32> },
    Artifact { scene: String },
    Sequence { shots: Vec<Shot> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shot {
    pub scene: String,
    pub out: String,
    #[serde(default)]
    pub cam: Option<[f32; 3]>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct View {
    #[serde(default)]
    pub cam: Option<[f32; 3]>,
    #[serde(default)]
    pub sheet: bool,
    #[serde(default)]
    pub columns: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Job {
    #[default]
    Shots,
    Perf {
        windows: u32,
        drop: u32,
    },
    Stable {
        frames: u32,
    },
}

impl Job {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Shots => "shots",
            Self::Perf { .. } => "perf",
            Self::Stable { .. } => "stable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub scene: Scene,
    #[serde(default)]
    pub view: View,
    pub width: u32,
    pub height: u32,
    pub out: String,
    #[serde(default)]
    pub job: Job,
    #[serde(default)]
    pub report: String,
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
    #[serde(default)]
    pub shots: Vec<String>,
    #[serde(default)]
    pub report: String,
    #[serde(default)]
    pub report_path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub schema_version: u32,
    pub protocol_hash: String,
    pub job: String,
    pub width: u32,
    pub height: u32,
    pub millis: u64,
    pub shots: Vec<ShotReport>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub perf: Vec<PerfReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pair: Option<Pair>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShotReport {
    pub scene: String,
    pub label: String,
    pub out: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub sha256: String,
    pub placeholder_px: u64,
    pub bright_px: u64,
    pub diff_vs_ref_grid: u64,
    pub declared_clouds: bool,
    pub has_cloud: bool,
    pub verdict: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerfReport {
    pub scene: String,
    pub label: String,
    #[serde(default)]
    pub windows: Vec<f64>,
    #[serde(default)]
    pub dropped: Vec<f64>,
    pub min: f64,
    pub median: f64,
    pub n: u32,
    pub error_bar: ErrorBar,
    #[serde(default)]
    pub gpu: Vec<GpuSample>,
    #[serde(default)]
    pub frames: Vec<f64>,
    #[serde(default)]
    pub p50: f64,
    #[serde(default)]
    pub p90: f64,
    #[serde(default)]
    pub p99: f64,
    #[serde(default)]
    pub max: f64,
    #[serde(default)]
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waits: Option<Waits>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_ms: Option<GpuMs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compare: Option<Compare>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Waits {
    pub pipelines_ms: f64,
    pub assets_ms: f64,
    pub settle_ms: f64,
    pub sample_ms: f64,
    pub total_ms: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GpuMs {
    pub source: String,
    pub lag_frames: u32,
    pub n: u32,
    pub min: f64,
    pub p50: f64,
    pub p90: f64,
    pub p99: f64,
    pub max: f64,
    pub frames: Vec<f64>,
    pub poll_us: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Compare {
    pub app_p50_ms: f64,
    pub app_mean_ms: f64,
    pub gpu_p50_ms: f64,
    pub ratio: f64,
    pub ratio_mean: f64,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pair {
    pub measured: String,
    pub reference: String,
    pub app_delta_ms: f64,
    pub app_mean_delta_ms: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_delta_ms: Option<f64>,
    pub rule: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorBar {
    pub rule: String,
    pub value_ms: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GpuSample {
    pub window: u32,
    pub sm_mhz: Vec<f64>,
    pub power_w: Vec<f64>,
    pub util_pct: Vec<f64>,
    pub vram_mib_max: f64,
    pub temp_c_max: f64,
    pub samples: u32,
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
