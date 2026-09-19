//! 渲染作业的形状（`Request` / `Report` / `Lease`…），原 `px_protocol::render`。
//!
//! ⚠ 它**不属于**那张跨进程边界：`px_protocol` 只认 px-scene ⇄ px-pass 的交换，而作业请求
//! 是宿主自己的事（谁能渲、渲完回什么话）。留着的代价不是洁癖 —— 它是**循环依赖**：
//! `px_render` 本来就依赖 `px_protocol`，作业形状再留在那边就编不过。
//!
//! ⚠ 这里住的是**形状**，跨进程的信封与握手仍然在 `px_protocol`（`stream` / `wire`）。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Scene {
    World {
        stream: String,
        round: Option<u32>,
    },
    /// 渲染一份场景产物（`scene::SceneSpec`）：行星、云、大气、shader 槽、渲染参数
    /// 全在它里面。请求只说「渲哪一份」，不再逐项传内容。
    Artifact {
        scene: String,
    },
    /// 一串场景产物，一步一步出图。批量不是「一个请求渲多种内容」——
    /// 每一步仍然是一份完整的场景产物，请求只再多给「存哪儿」和这一步的仪器档。
    Sequence {
        shots: Vec<Shot>,
    },
}

/// 批量里的一步。`cam` 是「从哪个角度看」（怎么看），所以留在请求上；
/// 「渲的是哪个变体」是内容，归场景产物。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shot {
    /// 场景产物路径。
    pub scene: String,
    /// 这一步的图存哪儿。
    pub out: String,
    /// 相机 `yaw,pitch,dist`；给了就盖掉请求上的那一档。
    #[serde(default)]
    pub cam: Option<[f32; 3]>,
}

/// 请求里**怎么看**的那一档：相机与对照图排布。内容（壳半径、密度、云参数、消融档、
/// 大气、shader）一律住在场景产物里，这里一个都不留 —— 两处都能改内容就是两处会漂开的默认值。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct View {
    #[serde(default)]
    pub cam: Option<[f32; 3]>,
    /// 用产物自带的相机表出「一张多视角对照图」（相机表住在 `.pxart` 里）。
    #[serde(default)]
    pub sheet: bool,
    /// 对照图的列数（sheet 为真时有效）。
    #[serde(default)]
    pub columns: u32,
}

/// 一次请求要的是哪一路活。**每一路都要一份结构化 JSON 报告**（落盘 + 回给调用方），
/// 见 `Report`。分开的理由：截图要的是"每档一张可判据的图"，性能要的是"每档一串帧"，
/// 混在一条路上时前者会被后者的等待/丢窗污染（P32）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Job {
    /// 截图：一串场景各出一张图就**立刻切下一个**，不做等待统计。
    #[default]
    Shots,
    /// 老性能路（v10）：每个场景先出一张图，再收 `windows` 个干净窗口（前面丢 `drop` 个）。
    /// **回退路径**：`--windows` 显式给了才走它，老脚本因此一个字节都不用改。
    Perf {
        windows: u32,
        drop: u32,
    },
    /// 新性能主路径（v11）：等到「管线就绪 + 资产装完 + 重建后已渲染 K 帧」成立，
    /// 再**逐帧**采 `frames` 帧。窗口、丢窗、热身这三个经验常数在这一路上不存在。
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
    /// 报告 JSON 落盘到哪儿。空 = 不落盘（仍然回给调用方）。
    #[serde(default)]
    pub report: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    /// 这一张。批量时 = 最后一张。
    pub out: String,
    pub scene: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub millis: u64,
    pub warm: bool,
    /// 这次请求写出来的全部图，按顺序。单张时长度 1。
    #[serde(default)]
    pub shots: Vec<String>,
    /// 这一次请求的**结构化报告**（JSON 文本，与落盘的那份逐字节相同）。空 = 这条路没有报告。
    #[serde(default)]
    pub report: String,
    /// 报告落盘路径。空 = 没落盘。
    #[serde(default)]
    pub report_path: String,
}

/// 一次请求的完整报告。字段名是**契约**，改了就要升 `SCHEMA_VERSION`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub schema_version: u32,
    pub protocol_hash: String,
    /// `shots` / `perf`。
    pub job: String,
    pub width: u32,
    pub height: u32,
    pub millis: u64,
    pub shots: Vec<ShotReport>,
    /// 只有 `perf` / `stable` 那两路非空。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub perf: Vec<PerfReport>,
    /// 配对差：`perf` 里第一项（被测档）减第二项（参照档）。
    /// **单看绝对值没意义** —— 要判的是配对差，所以这一步在服务端就算好。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pair: Option<Pair>,
}

/// 一张图的账 + 可用性标记。
///
/// `has_cloud` 的口径（阈值写死在实现里，这里只是承诺语义）：
/// **场景声明了 clouds part** 且 `placeholder_px == 0` 且 `diff_vs_ref_grid ≥ 0.5% × 总像素`。
/// 参考图 = 这一批的**第一张**（调用方按约定把无云档放在第一个）。
/// 它挡的是两次真事故：装的是占位 shader（整块洋红）、以及云壳没有管线（图与无云档逐字节相同）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShotReport {
    pub scene: String,
    pub label: String,
    pub out: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub sha256: String,
    /// 纯洋红（占位 shader 的指纹）像素数。
    pub placeholder_px: u64,
    /// 亮像素（r+g+b > 72/765）数。
    pub bright_px: u64,
    /// 与参考图的 **16×10 网格差分**：逐格**光通量和**（r+g+b）之差的绝对值之和。
    /// 参考图自己是 0。用网格而不是逐像素：它对"相机末位抖动"那类底噪免疫；
    /// 用电通量而不是"亮像素计数"：云画在行星**上面**，亮像素总数几乎不动，判不出来（P32 实测）。
    pub diff_vs_ref_grid: u64,
    pub declared_clouds: bool,
    pub has_cloud: bool,
    pub verdict: String,
}

/// 一档的账。老路（`windows`/`dropped`）与新主路径（`frames`/分位数）共用这个壳：
/// **原始值全打**，汇总只是附带的；哪一路非空由 `Report.job` 说了算。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerfReport {
    pub scene: String,
    pub label: String,
    /// 老路：干净窗口（每窗 `FRAME_PROBE_WINDOW` 帧的平均帧时间）。
    #[serde(default)]
    pub windows: Vec<f64>,
    /// 老路：被丢弃的窗口（重建跨过的那几个），也全打出来。
    #[serde(default)]
    pub dropped: Vec<f64>,
    pub min: f64,
    pub median: f64,
    pub n: u32,
    pub error_bar: ErrorBar,
    /// 老路：逐窗口的卡采样（每 ~2 s 一次，与窗口对齐）。
    #[serde(default)]
    pub gpu: Vec<GpuSample>,
    /// 新主路径：**逐帧** app 循环毫秒（原始值全打）。
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
    /// 这一步产物自身的内容键（十六进制，`0` = 拿不到）。`--changed` 拿它比"这一档真变了吗"。
    #[serde(default)]
    pub key: String,
    /// 新主路径：每一次等待的**实际耗时**（老路为 `null`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waits: Option<Waits>,
    /// 新主路径：GPU 时间戳（设备不支持时为 `null`，那时只有 app 侧的数）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_ms: Option<GpuMs>,
    /// 新主路径：GPU 中位 vs app 循环中位。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compare: Option<Compare>,
}

/// 每一次等待的实际耗时。老协议里这些位置全是经验常数（热身 4 s / 丢 1 窗 / 40 帧上限）；
/// 新主路径把它们换成**量出来的数** —— 这才是"热身/丢窗"原本想替掉的东西。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Waits {
    /// 等到「管线全部就绪」（`PipelineCache` 里没有 Queued/Creating 且失败为 0）。
    pub pipelines_ms: f64,
    /// 等到「这一步看的资产都装完」（`AssetServer` 的递归依赖装载态）。
    pub assets_ms: f64,
    /// 等到「重建后已有 K 帧被提取渲染」。
    pub settle_ms: f64,
    /// 采 N 帧 + 把 GPU 那一串对齐的收尾帧。
    pub sample_ms: f64,
    /// 从重建完成到采完。
    pub total_ms: f64,
}

/// GPU 时间戳的逐帧序列与分位数（`source` = 哪几条诊断路径求和来的）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GpuMs {
    pub source: String,
    /// 读回是**异步环形**的：这一串与 app 那串不逐帧对齐。这个数是"为了凑够样本多等了
    /// 几帧" —— 它同时含读回滞后与**到达率不足**（GPU 饱和时实测每 4 帧才到 1 条，
    /// 因为 Bevy 的 `RenderDiagnosticsMutex` 只有一个槽，一批完成多条只留最后一条）。
    pub lag_frames: u32,
    /// 真的拿到几条 GPU 样本（可能少于请求的帧数）。
    pub n: u32,
    pub min: f64,
    pub p50: f64,
    pub p90: f64,
    pub p99: f64,
    pub max: f64,
    pub frames: Vec<f64>,
    /// 主循环里把诊断取出来这件事本身的平均代价（µs/帧）——读回不在主循环里等。
    pub poll_us: f64,
}

/// GPU 中位 vs app 循环中位 —— 直接量化"拿 app 循环周期当 GPU 时间的代理量"偏了多少。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Compare {
    pub app_p50_ms: f64,
    /// app 逐帧序列的**均值**。它才是老协议那个"120 帧一窗的平均帧时间"的同类量：
    /// 逐帧序列在无窗口这条路里是**双峰**的（每 `GPU_IN_FLIGHT_FRAMES` 帧一次
    /// `poll(wait_indefinitely)`，那一帧特别长），所以 p50 与均值差得远。
    pub app_mean_ms: f64,
    pub gpu_p50_ms: f64,
    /// gpu / app（都用**中位**）。> 1 = GPU 比主循环慢；< 1 = 主循环被 CPU 或别的东西顶住了。
    pub ratio: f64,
    /// gpu / app（app 用**均值**）。
    pub ratio_mean: f64,
    pub note: String,
}

/// 配对差：被测档 − 参照档。判据是它，不是任何一档的绝对值。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pair {
    pub measured: String,
    pub reference: String,
    /// app **中位**之差。中位在双峰的 app 序列上没有意义 —— 留着是为了让人**看见**它错。
    pub app_delta_ms: f64,
    /// app **均值**之差（老窗口协议那个量的同类）。
    pub app_mean_delta_ms: f64,
    /// 设备不支持时间戳时为 `null`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_delta_ms: Option<f64>,
    pub rule: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorBar {
    pub rule: String,
    pub value_ms: f64,
}

/// 一段窗口期里的卡读数。`[min, max]`；采不到就是空数组。
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
    pub fn matches(&self, id: &px_handshake::ProtocolId) -> bool {
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





