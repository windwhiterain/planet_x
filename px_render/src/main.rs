use std::collections::VecDeque;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender, TryRecvError, sync_channel};
use std::time::{Duration, Instant};

use bevy::app::{AppExit, ScheduleRunnerPlugin};
use bevy::asset::LoadState;
use bevy::camera::{RenderTarget, Viewport};
use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::light::Skybox;
use bevy::prelude::*;
use bevy::render::diagnostic::RenderDiagnosticsPlugin;
use bevy::render::render_resource::{
    CachedPipelineState, PipelineCache, TextureFormat, TextureUsages,
};
use bevy::render::renderer::{RenderAdapterInfo, RenderDevice, RenderQueue};
use bevy::render::settings::{Backends, RenderCreation, WgpuSettings};
use bevy::render::view::screenshot::{Capturing, Screenshot, save_to_disk};
use bevy::render::{Render, RenderApp, RenderPlugin};
use bevy::window::ExitCondition;
use bevy::window::PrimaryWindow;
use bevy::winit::WinitPlugin;

use px_protocol::client;
use px_protocol::render::{
    Compare, ErrorBar, GpuMs, GpuSample, Job as JobKind, Lease, Pair, PerfReport, Report, Request,
    Response, Scene, ShotReport, Waits,
};
use px_protocol::sim::WorldView;
use px_protocol::stream::{self, Frame};
use px_protocol::ProtocolId;
use px_render::{
    Canvas, OrbitCamera, ScenePart, art_cache, asset_root, material, passes, scene, shaders, slots,
};

const GOOD_COLORS: [Srgba; 3] = [
    Srgba::new(0.86, 0.72, 0.34, 1.0),
    Srgba::new(0.52, 0.68, 0.88, 1.0),
    Srgba::new(0.74, 0.54, 0.82, 1.0),
];

const SPACING: f32 = 3.0;
const MAX_STACK: f32 = 5.0;
const MAX_BAR: f32 = 4.0;
const PIPELINE_WAIT_BUDGET: Duration = Duration::from_secs(30);
const FRAMES_AFTER_JOB: u32 = 6;
const LEASE_CHECK_INTERVAL: u32 = 120;
// 星空（`STAR_WIDTH` / `STAR_HEIGHT`）从这里删掉了：它现在是产物（`generated::stars`），
// 尺寸由烘图侧说了算，渲染器里一个常数都不留。

#[derive(Resource, Clone, Copy)]
struct InitialSize(u32, u32);

/// 「管线全部就绪」是渲染世界给主世界的一个**瞬时**断言（见 `watch_pipelines`）。
/// 它带一个 epoch：每次重建 +1，渲染世界靠它从头数干净帧。
#[derive(Resource, Clone)]
struct RenderReady {
    state: Arc<AtomicU8>,
    /// 每次重建 +1。为什么不用 `state != READY` 去推断"该从头数"：主世界把状态打回
    /// PENDING 之后状态会一直是 PENDING，用状态反推就等于每帧把计数清零 ⇒ 永远数不到 2。
    epoch: Arc<AtomicU32>,
    /// 已知坏在哪：**第一条**证到的原因留着不改，直到那批失败消失（`clear_failure`）
    /// 或重建（新 epoch）。
    failure: Arc<std::sync::Mutex<Option<String>>>,
}

impl RenderReady {
    fn new() -> Self {
        Self {
            state: Arc::new(AtomicU8::new(PIPELINES_PENDING)),
            epoch: Arc::new(AtomicU32::default()),
            failure: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    fn get(&self) -> u8 {
        self.state.load(Ordering::Relaxed)
    }

    fn store(&self, value: u8) {
        self.state.store(value, Ordering::Relaxed);
    }

    fn failure(&self) -> Option<String> {
        self.failure
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    /// 证到坏了：留原因 + 置 FAILED。原因只认第一条（根因），后来的不覆盖。
    fn fail(&self, detail: String) {
        let mut slot = self.failure.lock().unwrap_or_else(|err| err.into_inner());
        if slot.is_none() {
            *slot = Some(detail);
        }
        drop(slot);
        self.state.store(PIPELINES_FAILED, Ordering::Relaxed);
    }

    /// 那批失败没了（管线重新编过了）：明细跟着走，状态由 `watch_pipelines` 接着算。
    /// 少了这一步，一次坏管线会把服务**钉死到重启**：拒绝的理由永远留着，新的请求
    /// 在 `accept_jobs` 就被挡回去。
    fn clear_failure(&self) {
        *self.failure.lock().unwrap_or_else(|err| err.into_inner()) = None;
    }

    /// 重建 = 那个瞬时断言当场失效：epoch 加一 + 状态打回 PENDING。
    fn invalidate(&self) {
        *self.failure.lock().unwrap_or_else(|err| err.into_inner()) = None;
        self.epoch.fetch_add(1, Ordering::Relaxed);
        self.state.store(PIPELINES_PENDING, Ordering::Relaxed);
    }
}

const PIPELINES_PENDING: u8 = 0;
const PIPELINES_READY: u8 = 1;
const PIPELINES_FAILED: u8 = 2;

// ---------------------------------------------------------------------------
// 报告：截图统计 + 卡读数
// ---------------------------------------------------------------------------

/// 网格差分用的格数。16×10 对"整块云没了"足够敏感，又对相机末位抖动那类底噪免疫。
const GRID_COLS: u32 = 16;
const GRID_ROWS: u32 = 10;
/// 「亮像素」的阈值：r+g+b > 72（即三通道均值 > 24/255）。天空接近全黑，行星/壳远超它。
const BRIGHT_SUM: u32 = 72;
/// `has_cloud` 的差分阈值：网格差分 ÷ 参考图的总光通量。
///
/// ⚠ 网格里存的是**逐格光通量和**（r+g+b），不是"亮像素计数"。用亮像素计数判不出来：
/// 云是画在行星**上面**的，云换了行星，亮像素总数几乎不动（实测 1 249 627 → 1 257 629），
/// 只有边缘那一圈会跨过阈值 ⇒ 差分小得没法判。光通量对"颜色/明暗整体变了"敏感得多。
const CLOUD_DIFF_RATIO: f64 = 0.01;

/// 截图观察者算完往这里塞，`drive` 取走。
#[derive(Resource, Default, Clone)]
struct ShotStats(Arc<Mutex<Vec<ShotStat>>>);

#[derive(Clone)]
struct ShotStat {
    /// 纯洋红像素数 —— 占位 shader 的指纹。
    placeholder_px: u64,
    bright_px: u64,
    /// 逐格的**光通量和**（r+g+b 累加，长度 = GRID_COLS × GRID_ROWS）。
    grid: Vec<u64>,
}

/// 卡读数。**绝不在主循环里调 nvidia-smi**：那会卡住一整帧、把那个窗口的帧时间顶高。
/// 后台线程每 2 s 采一次，主循环只读快照。
#[derive(Resource, Default, Clone)]
struct GpuLog(Arc<Mutex<Vec<GpuReading>>>);

#[derive(Clone, Copy, Default)]
struct GpuReading {
    sm: f64,
    power: f64,
    util: f64,
    vram: f64,
    temp: f64,
}

/// `perf` 那一路的窗口收集状态机。
struct PerfGather {
    want: u32,
    drop: u32,
    seen: u32,
    dropped: Vec<f64>,
    windows: Vec<f64>,
    /// 每个窗口结束时卡读数已有几条 —— 用它把采样切给窗口。
    marks: Vec<usize>,
    /// 图还没出完就先收到的窗口数。正常情况 ≤ 1（重建跨过的那一窗）；
    /// 一直涨说明出图卡住了，别在这儿无限等（P32 真踩过：113 窗）。
    before_shot: u32,
}

impl PerfGather {
    fn new(windows: u32, drop: u32) -> Self {
        Self {
            want: windows,
            drop,
            seen: 0,
            dropped: Vec::new(),
            windows: Vec::new(),
            marks: vec![0],
            before_shot: 0,
        }
    }

    fn done(&self) -> bool {
        self.windows.len() as u32 >= self.want
    }

    fn push(&mut self, millis: f64, sampled: usize) {
        if self.seen < self.drop {
            self.dropped.push(millis);
        } else {
            self.windows.push(millis);
        }
        self.seen += 1;
        self.marks.push(sampled);
    }
}

// ---------------------------------------------------------------------------
// 新主路径：等条件成立 + 逐帧采样
// ---------------------------------------------------------------------------

/// 重建之后要跑过多少**渲染帧**才认为画面稳定。它替掉的是「热身 4 s」：
/// 前几帧还在付新管线的首次绑定/上传，数它们就是量仪器自己。
const STABLE_SETTLE_FRAMES: u32 = 8;

/// 渲染世界跑过多少帧（`Render` 调度每帧 +1）。它是"重建后已有 K 帧被提取渲染"的
/// **直接读数** —— 不是猜，也不是拿 app 帧计数冒充。
#[derive(Resource, Clone, Default)]
struct RenderFrames(std::sync::Arc<std::sync::atomic::AtomicU32>);

fn count_render_frames(frames: Res<RenderFrames>) {
    frames.0.fetch_add(1, Ordering::Relaxed);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StablePhase {
    /// 等管线全部就绪。
    Pipelines,
    /// 等这一步看的资产装完。
    Assets,
    /// 等重建后已渲染 K 帧。
    Settle,
    /// 采 N 帧。
    Sample,
    /// 收尾：多等几帧把 GPU 那一串凑满 —— 差值就是 GPU 读回的滞后帧数。
    Flush,
}

struct StableGather {
    want: u32,
    phase: StablePhase,
    /// 重建完成那一刻。
    started: Instant,
    /// 每一个阶段的起点，用来量各段等待。
    mark: Instant,
    pipelines_ms: f64,
    assets_ms: f64,
    settle_ms: f64,
    /// 逐帧 app 循环毫秒。
    frames: Vec<f64>,
    /// GPU 时间戳逐帧毫秒（读回是异步环形的，整体滞后几帧）。
    gpu: Vec<f64>,
    /// 上一条 GPU 诊断的测量时刻：认"来了一批新的"用。
    gpu_seen: Option<std::time::Instant>,
    /// 哪几条诊断路径求和来的（报告里列出，读数的人能自己核）。
    gpu_source: String,
    /// 主循环里取诊断这件事本身的耗时（µs/帧）。
    poll_us_total: f64,
    poll_us_count: u32,
    /// 收到过多少条 GPU 样本（含采样窗之前的）。
    gpu_lag_frames: u32,
    flush_frames: u32,
    /// 资产那一段只报一次（每一步都要经历它，不刷屏）。
    assets_logged: bool,
    /// 采样 + 对齐那一段的实测耗时。
    sample_ms: f64,
    /// 采样段的起点（`sample_ms` 从它算）。
    sample_started: Instant,
    /// 从重建完成到采完。
    total_ms: f64,
    done: bool,
}

impl StableGather {
    fn new(want: u32) -> Self {
        let now = Instant::now();
        Self {
            want,
            phase: StablePhase::Pipelines,
            started: now,
            mark: now,
            pipelines_ms: f64::NAN,
            assets_ms: f64::NAN,
            settle_ms: f64::NAN,
            frames: Vec::with_capacity(want as usize),
            gpu: Vec::with_capacity(want as usize),
            gpu_seen: None,
            gpu_source: String::new(),
            poll_us_total: 0.0,
            poll_us_count: 0,
            gpu_lag_frames: 0,
            flush_frames: 0,
            assets_logged: false,
            sample_ms: f64::NAN,
            sample_started: now,
            total_ms: f64::NAN,
            done: false,
        }
    }

    fn phase_ms(&self) -> f64 {
        self.mark.elapsed().as_secs_f64() * 1000.0
    }

    fn enter(&mut self, phase: StablePhase) {
        self.phase = phase;
        self.mark = Instant::now();
    }
}

/// 分位数口径：线性插值（numpy 的 `linear`，即 R 的 type 7）。
/// `n=60` 时 p99 落在第 58.41 个序位上 —— 所以样本数本身就是**口径的一部分**，报告里带着 `n`。
fn quantile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let position = q.clamp(0.0, 1.0) * (sorted.len() - 1) as f64;
    let low = position.floor() as usize;
    let high = position.ceil() as usize;
    if low == high {
        return sorted[low];
    }
    let weight = position - low as f64;
    sorted[low] * (1.0 - weight) + sorted[high] * weight
}

/// 均值。**无窗口那条路的 app 逐帧序列是双峰的**（每 `GPU_IN_FLIGHT_FRAMES` 帧一次
/// `poll(wait_indefinitely)`），所以均值才是与老协议"120 帧一窗"同类的量，中位不是。
fn mean_of(values: &[f64]) -> f64 {
    let live: Vec<f64> = values.iter().cloned().filter(|value| value.is_finite()).collect();
    if live.is_empty() {
        return f64::NAN;
    }
    live.iter().sum::<f64>() / live.len() as f64
}

fn quantiles(values: &[f64]) -> (f64, f64, f64, f64, f64) {
    let mut sorted: Vec<f64> = values
        .iter()
        .cloned()
        .filter(|value| value.is_finite())
        .collect();
    if sorted.is_empty() {
        return (f64::NAN, f64::NAN, f64::NAN, f64::NAN, f64::NAN);
    }
    sorted.sort_by(|one, two| one.partial_cmp(two).unwrap_or(std::cmp::Ordering::Equal));
    (
        sorted[0],
        quantile(&sorted, 0.50),
        quantile(&sorted, 0.90),
        quantile(&sorted, 0.99),
        sorted[sorted.len() - 1],
    )
}

/// 把 `DiagnosticsStore` 里 `render/**/elapsed_gpu` 全部加起来 —— 这就是"这一帧 GPU 忙了多久"。
///
/// 为什么能求和：Bevy 给每个 render pass 各写一对时间戳（`main_opaque_pass_3d` /
/// `main_transparent_pass_3d` / `prepass` / mip 生成），外加 `tonemapping` / `upscaling`
/// 两条编码器级 `time_span`。它们互不嵌套，所以和 ≈ 一帧的 GPU 忙时；`source` 里把
/// 参与的路径列全，读数的人可以自己核是不是有哪一段被算了两次。
fn gpu_snapshot(store: &DiagnosticsStore) -> Option<(String, f64, std::time::Instant)> {
    let mut parts: Vec<(String, f64, std::time::Instant)> = Vec::new();
    for diagnostic in store.iter() {
        let path = diagnostic.path();
        let components: Vec<&str> = path.components().collect();
        if components.first() != Some(&"render") {
            continue;
        }
        if components.last() != Some(&"elapsed_gpu") {
            continue;
        }
        let Some(measurement) = diagnostic.measurement() else {
            continue;
        };
        if !measurement.value.is_finite() {
            continue;
        }
        parts.push((components.join("/"), measurement.value, measurement.time));
    }
    if parts.is_empty() {
        return None;
    }
    parts.sort_by(|one, two| one.0.cmp(&two.0));
    let newest = parts
        .iter()
        .map(|(_, _, time)| *time)
        .max()
        .unwrap_or_else(Instant::now);
    let sum: f64 = parts.iter().map(|(_, value, _)| *value).sum();
    let source = parts
        .iter()
        .map(|(name, value, _)| format!("{name}={value:.3}"))
        .collect::<Vec<_>>()
        .join(" + ");
    Some((source, sum, newest))
}

fn median_of(values: &[f64]) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|one, two| one.partial_cmp(two).unwrap_or(std::cmp::Ordering::Equal));
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[mid]
    } else {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    }
}

fn parse_gpu_line(line: &str) -> Option<GpuReading> {
    let number = |text: &str| -> f64 {
        let clean: String = text
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        clean.parse::<f64>().unwrap_or(f64::NAN)
    };
    let fields: Vec<&str> = line.split(',').map(|field| field.trim()).collect();
    if fields.len() < 5 {
        return None;
    }
    Some(GpuReading {
        sm: number(fields[0]),
        power: number(fields[1]),
        util: number(fields[2]),
        vram: number(fields[3]),
        temp: number(fields[4]),
    })
}

fn spawn_gpu_sampler(log: GpuLog) {
    std::thread::spawn(move || {
        let smi = std::env::var("SystemRoot")
            .map(|root| format!("{root}\\System32\\nvidia-smi.exe"))
            .unwrap_or_else(|_| "nvidia-smi".to_string());
        if !std::path::Path::new(&smi).exists() {
            println!("（没有 nvidia-smi，报告里的卡读数会是空数组）");
            return;
        }
        loop {
            let output = std::process::Command::new(&smi)
                .args([
                    "--query-gpu=clocks.current.sm,power.draw,utilization.gpu,memory.used,temperature.gpu",
                    "--format=csv,noheader",
                ])
                .output();
            if let Ok(output) = output
                && output.status.success()
                && let Ok(text) = String::from_utf8(output.stdout)
                && let Some(reading) = parse_gpu_line(text.trim())
            {
                if let Ok(mut readings) = log.0.lock() {
                    readings.push(reading);
                }
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

/// 截图统计：与 `save_to_disk` 挂在同一个实体上的第二个观察者。
/// 从**捕获到的图**算，不从磁盘上的 PNG 算 —— PNG 是有损压缩后的东西，判据要看原始像素。
fn collect_shot_stat(
    screenshot: On<bevy::render::view::screenshot::ScreenshotCaptured>,
    stats: Res<ShotStats>,
) {
    let Ok(dynamic) = screenshot.image.clone().try_into_dynamic() else {
        return;
    };
    let rgb = dynamic.to_rgb8();
    let (width, height) = (rgb.width(), rgb.height());
    let mut grid = vec![0u64; (GRID_COLS * GRID_ROWS) as usize];
    let mut placeholder_px = 0u64;
    let mut bright_px = 0u64;
    for (x, y, pixel) in rgb.enumerate_pixels() {
        let [r, g, b] = pixel.0;
        if r > 250 && g < 8 && b > 250 {
            placeholder_px += 1;
        }
        let sum = u32::from(r) + u32::from(g) + u32::from(b);
        if sum > BRIGHT_SUM {
            bright_px += 1;
        }
        let cell = (y * GRID_ROWS / height.max(1)) * GRID_COLS + (x * GRID_COLS / width.max(1));
        if let Some(slot) = grid.get_mut(cell as usize) {
            *slot += u64::from(sum);
        }
    }
    if let Ok(mut all) = stats.0.lock() {
        all.push(ShotStat {
            placeholder_px,
            bright_px,
            grid,
        });
    }
}

fn take_shot_stat(stats: &ShotStats) -> Option<ShotStat> {
    stats.0.lock().ok().and_then(|mut all| all.pop())
}

#[derive(Resource)]
struct Inbox(std::sync::Mutex<Receiver<Job>>);

#[derive(Resource, Default)]
struct Active(Option<ActiveJob>);

#[derive(Resource, Default)]
struct Ticks(u32);

#[derive(Resource)]
struct LeaseWatch {
    path: PathBuf,
    pid: u32,
}

struct Job {
    request: Request,
    reply: SyncSender<Frame>,
}

/// 一次请求要出的那一串图里的一步。内容仍然全在场景产物里：这一步只是
/// 「渲哪一份 + 存哪儿 + 从哪个角度看」。
struct Step {
    scene: StepScene,
    out: PathBuf,
    cam: Option<[f32; 3]>,
}

enum StepScene {
    World {
        stream: String,
        round: Option<u32>,
    },
    Artifact(String),
}

/// 一步搭出来的东西：进 `Response.scene` 的标签、环境（环境光 + 天空盒）以及产物自带的相机表。
struct Built {
    label: String,
    ambient: f32,
    /// 天空盒是**内容**（cube 贴图产物）：没有就不挂，背景就是清屏色。
    skybox: Option<Skybox>,
    cameras: Vec<px_protocol::art::Camera>,
    /// 这一步往槽里装了新 shader（要等管线重编，见 `drive`）。
    installed_shaders: bool,
    /// 这一步的内容声明了 `clouds` 这个期望标签（报告里"该有云"的判据）。
    declared_clouds: bool,
    /// 这一步看的资产（槽里装的 shader 句柄）：等"资产装完"看的就是它们。
    watch: Vec<Handle<Shader>>,
    /// 可以现场改的仪器参数（窗口那一套键）。
    instruments: Vec<scene::Instrument>,
    /// 这一步文档声明的 pass 表（None = 没有这一节）。
    passes: Option<Arc<px_pass::Plan>>,
}

/// 对照图里的一格（列/行 + 格子尺寸）。相机表来自 `.pxart`，格子的排布是渲染器的事。
#[derive(Debug, Clone, Copy)]
struct SheetCell {
    column: u32,
    row: u32,
    width: u32,
    height: u32,
}

struct ActiveJob {
    /// 这一步的标签。
    label: String,
    /// 这一步的图存哪儿。
    out: PathBuf,
    width: u32,
    height: u32,
    /// 整个请求的起点（批量时毫秒数算全部，不是最后一步）。
    started: Instant,
    reply: SyncSender<Frame>,
    warm: u32,
    requested: bool,
    /// 这一步的图写出来了：`drive` 置位，`accept_jobs` 接着切下一步或回话。
    finished: bool,
    /// 还没轮到的那几步。
    queue: VecDeque<Step>,
    /// 已经写出来的图，按顺序。
    shots: Vec<String>,
    /// 请求里「怎么看」那一档（对照图开关与列数）。每一步都沿用。
    view: px_protocol::render::View,
    /// 一格视口的尺寸（对照图会乘上行列）。
    cell: (u32, u32),
    /// 这一次请求是哪一路活（截图 / 性能）。
    job_kind: JobKind,
    /// 报告 JSON 落盘路径（空 = 不落盘）。
    report_path: String,
    /// 已经出过的图各自的那份账。
    shot_reports: Vec<ShotReport>,
    /// 这一批第一张图的网格签名（`diff_vs_ref_grid` 的参考）。
    ref_grid: Option<Vec<u64>>,
    /// 这一步的场景产物声明了 clouds part。
    declared_clouds: bool,
    /// 这一步的图与统计都收好了。
    shot_done: bool,
    /// 已经收完的档（只有 perf 那一路会非空）。
    perf_reports: Vec<PerfReport>,
    /// 正在收窗口（`perf` 那一路）。
    perf: Option<PerfGather>,
    /// 正在等条件 + 逐帧采样（`stable` 那一路，新主路径）。
    stable: Option<StableGather>,
    /// 这一步看的资产（槽里装的 shader + shader 库）：`AssetServer` 的装载态只看它们。
    watched: Vec<Handle<Shader>>,
    /// 这一步的场景产物路径（报告里对账用）。
    scene_path: String,
    /// 这一步产物的内容键（十六进制；`0` = 拿不到）。`--changed` 拿它比"这一档真变了吗"。
    content_key: String,
    /// 重建完成那一刻的挂钟（等条件的耗时从这里起算）。
    rebuilt_instant: Instant,
    /// 重建完成那一刻渲染世界跑过多少帧（等"K 帧被提取渲染"的起点）。
    rebuilt_render_frame: u32,
    /// 这一步还没建（在等新 shader 版本落地）。`drive` 这时候**一个字都不许做** ——
    /// 忘了这条就会拿一个空的 `out` 去出图（P32 真踩过：截图存盘报"格式认不出来"，
    /// 然后一直等一个永远不出现的文件）。
    pending_preload: bool,
}

impl ActiveJob {
    /// 把一次请求摊成一队「要出的图」。World 只有一张；场景产物一张或一串。
    fn steps(request: &Request) -> Result<VecDeque<Step>, String> {
        let out = PathBuf::from(&request.out);
        let cam = request.view.cam;
        Ok(match &request.scene {
            Scene::World { stream, round } => VecDeque::from([Step {
                scene: StepScene::World {
                    stream: stream.clone(),
                    round: *round,
                },
                out,
                cam,
            }]),
            Scene::Artifact { scene } => VecDeque::from([Step {
                scene: StepScene::Artifact(scene.clone()),
                out,
                cam,
            }]),
            Scene::Sequence { shots } => {
                if shots.is_empty() {
                    return Err("批量请求一步都没有".to_string());
                }
                shots
                    .iter()
                    .map(|shot| Step {
                        scene: StepScene::Artifact(shot.scene.clone()),
                        out: PathBuf::from(&shot.out),
                        // 这一步给了相机就用它的，没给就沿用请求上那一档。
                        cam: shot.cam.or(cam),
                    })
                    .collect()
            }
        })
    }

    fn new(job: Job, queue: VecDeque<Step>) -> Self {
        Self {
            label: String::new(),
            out: PathBuf::new(),
            width: 0,
            height: 0,
            started: Instant::now(),
            reply: job.reply,
            warm: 0,
            requested: false,
            finished: false,
            queue,
            shots: Vec::new(),
            view: job.request.view.clone(),
            cell: (job.request.width, job.request.height),
            job_kind: job.request.job,
            report_path: job.request.report.clone(),
            shot_reports: Vec::new(),
            ref_grid: None,
            declared_clouds: false,
            shot_done: false,
            perf_reports: Vec::new(),
            perf: None,
            stable: None,
            watched: Vec::new(),
            scene_path: String::new(),
            content_key: String::new(),
            rebuilt_instant: Instant::now(),
            rebuilt_render_frame: 0,
            pending_preload: false,
        }
    }

    /// 一次请求的结算。`out` / `bytes` / `width` / `height` 说的是**最后一张**。
    fn response(&self, warm: bool, report: String) -> Response {
        Response {
            out: self.out.display().to_string(),
            scene: self.label.clone(),
            width: self.width,
            height: self.height,
            bytes: std::fs::metadata(&self.out)
                .map(|meta| meta.len())
                .unwrap_or(0),
            millis: self.started.elapsed().as_millis() as u64,
            warm,
            shots: self.shots.clone(),
            report,
            report_path: self.report_path.clone(),
        }
    }
}

#[derive(Debug, Clone)]
struct ShotArgs {
    scene: PathBuf,
    out: Option<PathBuf>,
    cam: Option<[f32; 3]>,
}

#[derive(Debug)]
struct Options {
    serve: bool,
    view: bool,
    show: bool,
    shot: bool,
    autostart: bool,
    port: u16,
    width: u32,
    height: u32,
    stream: PathBuf,
    round: Option<u32>,
    /// 一次请求要出的那一串图。`--scene` 起一步，`--out` / `--cam` 配到**它前面那一步**；
    /// 一步都没有就是经济世界那条路。
    shots: Vec<ShotArgs>,
    /// 没有 `--scene` 时（经济世界）这一张存哪儿。
    out: Option<PathBuf>,
    pcg_root: PathBuf,
    pcg_root_given: bool,
    fps: bool,
    novsync: bool,
    /// 用产物自带的相机表出一张多视角对照图。只给开关：相机表住在 `.pxart` 里。
    sheet: bool,
    columns: u32,
    /// 没跟在哪一步后面的 `--cam`：整批通用。
    cam: Option<[f32; 3]>,
    /// `--perf`：这一路请求要收帧（默认是 `--shots`）。
    perf: bool,
    /// 性能那一路要收的**干净**窗口数（老路）。
    windows: u32,
    /// 调用方**显式**给了 `--windows`：那就走老路（老脚本一个字节都不用改）。
    windows_given: bool,
    /// 性能那一路先丢几个窗口（重建跨过的那几个，老路）。
    drop_windows: u32,
    /// 新主路径要采的**帧数**（逐帧，不再折成 120 帧一个窗）。60 帧够算 p99。
    frames: u32,
    /// 报告 JSON 写哪儿（空 = 只回给调用方，不落盘）。
    report: String,
    /// `--where`：问常驻窗口"相机现在在哪儿"（**只读**，不动画面、不换场景）。§64.8
    ask_where: bool,
    /// `--place yaw,pitch,distance`：把常驻窗口的相机摆到某个方位（复现某个视角用）。
    place: Option<[f32; 3]>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            serve: false,
            view: false,
            show: false,
            shot: false,
            autostart: false,
            port: 0,
            width: 960,
            height: 640,
            stream: PathBuf::from("target/world.pxstream"),
            round: None,
            shots: Vec::new(),
            out: None,
            pcg_root: PathBuf::from("target/pcg"),
            pcg_root_given: false,
            fps: false,
            novsync: false,
            sheet: false,
            columns: 4,
            cam: None,
            perf: false,
            windows: 4,
            windows_given: false,
            drop_windows: 1,
            frames: 60,
            report: String::new(),
            ask_where: false,
            place: None,
        }
    }
}

fn parse_cam(text: &str) -> Result<[f32; 3], String> {
    let parts: Vec<f32> = text
        .split(',')
        .map(|part| part.trim().parse::<f32>())
        .collect::<Result<_, _>>()
        .map_err(|_| "--cam 要 yaw,pitch,dist 三个数".to_string())?;
    if parts.len() != 3 {
        return Err("--cam 要 yaw,pitch,dist 三个数".to_string());
    }
    Ok([parts[0], parts[1], parts[2]])
}

impl Options {
    fn parse() -> Result<Self, String> {
        let mut options = Self::default();

        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            let mut next = |needed: &str| args.next().ok_or_else(|| format!("{needed} 需要一个值"));
            match arg.as_str() {
                "--serve" => options.serve = true,
                "--view" => options.view = true,
                "--show" => options.show = true,
                "--shot" => options.shot = true,
                // 窗口相机的一问一答（§64.8）：只跟常驻窗口说话，不换场景、不重烘。
                "--where" => options.ask_where = true,
                "--place" => options.place = Some(parse_cam(&next("--place")?)?),
                "--autostart" => options.autostart = true,
                "--fps" => options.fps = true,
                "--novsync" => options.novsync = true,
                "--sheet" => options.sheet = true,
                // 两路活：默认截图（每个 --scene 一张图就切下一个）；--perf 改成收窗口。
                "--shots" => options.perf = false,
                "--perf" => options.perf = true,
                "--windows" => {
                    options.windows = next("--windows")?
                        .parse()
                        .map_err(|_| "--windows 需要一个整数".to_string())?;
                    // 给了窗口数就是走老路：老脚本这么写的，它的语义一个字节都不许变。
                    options.windows_given = true;
                }
                "--drop" => {
                    options.drop_windows = next("--drop")?
                        .parse()
                        .map_err(|_| "--drop 需要一个整数".to_string())?;
                }
                "--frames" => {
                    options.frames = next("--frames")?
                        .parse()
                        .map_err(|_| "--frames 需要一个整数".to_string())?;
                }
                "--report" => options.report = next("--report")?,
                "--stream" => options.stream = PathBuf::from(next("--stream")?),
                "--scene" => options.shots.push(ShotArgs {
                    scene: PathBuf::from(next("--scene")?),
                    out: None,
                    cam: None,
                }),
                "--pcg-root" => {
                    options.pcg_root = PathBuf::from(next("--pcg-root")?);
                    options.pcg_root_given = true;
                }
                "--out" => {
                    let out = PathBuf::from(next("--out")?);
                    match options.shots.last_mut() {
                        Some(shot) => shot.out = Some(out),
                        None => options.out = Some(out),
                    }
                }
                "--cam" => {
                    let cam = parse_cam(&next("--cam")?)?;
                    match options.shots.last_mut() {
                        Some(shot) => shot.cam = Some(cam),
                        None => options.cam = Some(cam),
                    }
                }
                "--columns" => {
                    options.columns = next("--columns")?
                        .parse()
                        .map_err(|_| "--columns 需要一个整数".to_string())?;
                }
                "--round" => {
                    options.round = Some(
                        next("--round")?
                            .parse()
                            .map_err(|_| "--round 需要一个整数".to_string())?,
                    );
                }
                "--port" => {
                    options.port = next("--port")?
                        .parse()
                        .map_err(|_| "--port 需要一个整数".to_string())?;
                }
                "--width" => {
                    options.width = next("--width")?
                        .parse()
                        .map_err(|_| "--width 需要一个整数".to_string())?;
                }
                "--height" => {
                    options.height = next("--height")?
                        .parse()
                        .map_err(|_| "--height 需要一个整数".to_string())?;
                }
                "--help" | "-h" => {
                    println!("{}", usage());
                    std::process::exit(0);
                }
                other => return Err(format!("未知参数：{other}\n{}", usage())),
            }
        }

        if options.sheet && (options.cam.is_some() || options.shots.iter().any(|s| s.cam.is_some())) {
            return Err("--sheet 用的是产物自带的相机表，不要再给 --cam".to_string());
        }
        Ok(options)
    }

    fn scene_path(&self) -> Result<Option<&std::path::Path>, String> {
        if self.shots.len() > 1 {
            return Err("这条命令一次只吃一份场景：批量是 `--serve` 那边的事".to_string());
        }
        Ok(self.shots.first().map(|shot| shot.scene.as_path()))
    }

    /// 单张时的相机：`--scene A --cam …` 是「这一步的相机」，单张请求里它就是 `View.cam`。
    fn view_cam(&self) -> Option<[f32; 3]> {
        match self.shots.as_slice() {
            [shot] => shot.cam.or(self.cam),
            _ => self.cam,
        }
    }

    /// 这一次请求的 `out`：单张就是它，批量时是最后一张。
    fn out_path(&self) -> PathBuf {
        self.shots
            .last()
            .and_then(|shot| shot.out.clone())
            .or_else(|| self.out.clone())
            .unwrap_or_else(|| PathBuf::from("target/shot.png"))
    }

    /// 这一次请求是哪一路活。
    /// **默认是新主路径**（等条件成立 + 逐帧采样）；`--windows` 显式给了才回退到老窗口路。
    fn job(&self) -> JobKind {
        if !self.perf {
            return JobKind::Shots;
        }
        if self.windows_given {
            JobKind::Perf {
                windows: self.windows,
                drop: self.drop_windows,
            }
        } else {
            JobKind::Stable {
                frames: self.frames,
            }
        }
    }

    fn scene(&self) -> Result<Scene, String> {
        if self.shots.is_empty() {
            return Ok(Scene::World {
                stream: self.stream.display().to_string(),
                round: self.round,
            });
        }
        if let [shot] = self.shots.as_slice() {
            return Ok(Scene::Artifact {
                scene: shot.scene.display().to_string(),
            });
        }
        let mut shots = Vec::with_capacity(self.shots.len());
        // 新主路径**不出图**（见 `accept_jobs`），所以它不要求每一步给 `--out`；
        // 截图与老性能路要落图，那两路少一个 `--out` 就是少一张图，必须报错。
        let outs_needed = !matches!(self.job(), JobKind::Stable { .. });
        for (index, shot) in self.shots.iter().enumerate() {
            let out = match (&shot.out, outs_needed) {
                (Some(out), _) => out.display().to_string(),
                (None, true) => {
                    return Err(format!(
                        "批量请求的第 {} 步没给 --out：{} 要存哪儿？",
                        index + 1,
                        shot.scene.display()
                    ));
                }
                (None, false) => String::new(),
            };
            shots.push(px_protocol::Shot {
                scene: shot.scene.display().to_string(),
                out,
                cam: shot.cam,
            });
        }
        Ok(Scene::Sequence { shots })
    }
}

fn usage() -> String {
    [
        "用法：",
        "  px_render --serve [--port N] [--width W] [--height H] [--fps]",
        "      常驻渲染服务：启动时预热管线，之后按请求出图。",
        "      ⚠ --fps 去掉 60 Hz 上限；**性能请求（--perf）要求服务带 --fps**，否则拒收。",
        "  px_render --scene A.pxart --out a.png [--scene B.pxart --out b.png] [--report r.json]",
        "      截图请求（默认）：每个场景出一张图就立刻切下一个；报告是 JSON。",
        "  px_render --perf --scene A.pxart [--scene B.pxart] --frames 60 --report r.json",
        "      **性能主路径**：每一步等到「管线就绪 + 资产装完 + 重建后已渲染 K 帧」成立，",
        "      再逐帧采 60 帧，报 min/p50/p90/p99/max + 原始逐帧序列 + 每次等待的实测耗时。",
        "      给两档就是**配对**：报告里的 pair 直接给「被测档 − 参照档」的差值。",
        "  px_render --perf --windows 4 --drop 1 --scene A.pxart --out a.png --report r.json",
        "      **回退路径**（老协议 v10 语义）：每个场景先出一张图，再收 4 个干净窗口（先丢 1 个）。",
        "      给了 --windows 就自动走它 —— 老脚本不用改。",
        "      （两路的字段见 .agents/notes/art/09-instruments.md §57 的 schema 与样例。）",
        "  px_render --stream PATH [--round N] [--out PNG] [--width W] [--height H]",
        "      经济世界：三根部门库存柱 + 三根价格柱",
        "  px_render --scene SCENE.pxart [--out PNG] [--cam YAW,PITCH,DIST] [--sheet]",
        "            [--pcg-root DIR] [--width W] [--height H]",
        "      场景产物：行星、云、大气、shader 槽、消融档全部由 .pxart 决定",
        "      （成员按内容键去 CAS 取）。--scene 可以给多次，一次出一整串图：",
        "        --scene a.pxart --out a.png --scene b.pxart --out b.png",
        "      每一步的 --out / --cam 配在它前面那个 --scene 上。",
        "      --sheet：用产物自带的相机表（AssetManifest.cameras）出一张多视角对照图，",
        "      默认 4 列（--columns 可改），相机表是烘图时用 px_ops::cameras::review() 灌的。",
        "      --pcg-root 是 CAS 根（默认 target/pcg）—— 它只在服务端生效：解析成员的是服务进程。",
        "  px_render --view [--scene SCENE.pxart] [--fps] [--novsync]",
        "      常驻预览窗口：1 台自由相机，场景产物变了才重建",
        "  px_render --show --scene SCENE.pxart [--shot]",
        "      把一份场景推到常驻窗口；--shot 顺便存一张 target/viewer-shot.png",
        "  服务没在跑时会提示；要顺手拉起一个就加 --autostart",
    ]
    .join("\n")
}

fn main() {
    let options = match Options::parse() {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(64);
        }
    };

    // 解析成员的是渲染进程，它有自己的 CAS 根；客户端这一份只报一声，不能装作生效。
    if !options.serve && !options.view && options.pcg_root_given {
        eprintln!(
            "⚠ --pcg-root 只在渲染进程（--serve / --view）生效：这次请求的内容由服务进程按它自己的 CAS 根解析"
        );
    }

    if options.view {
        if let Err(message) = view(options) {
            eprintln!("{message}");
            std::process::exit(1);
        }
    } else if options.ask_where || options.place.is_some() {
        if let Err(message) = viewer_camera(&options) {
            eprintln!("{message}");
            std::process::exit(1);
        }
    } else if options.show {
        if let Err(message) = show(&options) {
            eprintln!("{message}");
            std::process::exit(1);
        }
    } else if options.serve {
        if let Err(message) = serve(options) {
            eprintln!("{message}");
            std::process::exit(1);
        }
    } else {
        std::process::exit(request_once(options));
    }
}

fn request_once(options: Options) -> i32 {
    let out = options.out_path();
    let scene = match options.scene() {
        Ok(scene) => scene,
        Err(message) => {
            eprintln!("{message}");
            return 64;
        }
    };
    let request = Request {
        scene,
        view: px_protocol::render::View {
            cam: options.view_cam(),
            sheet: options.sheet,
            columns: options.columns,
        },
        width: options.width,
        height: options.height,
        out: out.display().to_string(),
        job: options.job(),
        report: options.report.clone(),
    };

    match client::request_with(request, options.autostart) {
        Ok(response) => {
            for shot in &response.shots {
                println!("写出：{shot}");
            }
            println!(
                "{} → {}（{}×{}，{} 字节，耗时 {} ms，{}，共 {} 张）",
                response.scene,
                response.out,
                response.width,
                response.height,
                response.bytes,
                response.millis,
                if response.warm { "服务已热" } else { "服务刚起" },
                response.shots.len().max(1),
            );
            if !response.report_path.is_empty() {
                println!("报告：{}", response.report_path);
            }
            // 报告也**回给调用方**（与落盘那份逐字节相同）：调用方不必再去读文件。
            if !response.report.is_empty() {
                println!("{}", response.report);
            }
            0
        }
        Err(px_protocol::ClientError::NoServer) => {
            eprintln!("没有在跑的渲染服务。先起一个：");
            eprintln!("    px_render --serve");
            eprintln!("租约文件：{}", client::lease_path().display());
            1
        }
        Err(err) => {
            eprintln!("{err}");
            1
        }
    }
}

fn serve(options: Options) -> Result<(), String> {
    let listener = TcpListener::bind(("127.0.0.1", options.port))
        .map_err(|err| format!("绑定端口失败：{err}"))?;
    let port = listener
        .local_addr()
        .map_err(|err| format!("拿不到端口：{err}"))?
        .port();

    let id = ProtocolId::local();
    let lease_path = client::lease_path();
    // 卡读数从**后台线程**采：主循环里调 nvidia-smi 会卡住一整帧，把那个窗口顶高。
    let gpu_log = GpuLog::default();
    spawn_gpu_sampler(gpu_log.clone());
    let lease = Lease {
        pid: std::process::id(),
        port,
        protocol_hash: id.protocol_hash,
        git_rev: id.git_rev.clone(),
        exe: std::env::current_exe()
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
    };
    client::write_lease(&lease_path, &lease).map_err(|err| err.to_string())?;

    println!(
        "渲染服务已启动：端口 {port}、协议指纹 {:016x}、git {}、pid {}",
        id.protocol_hash, id.git_rev, lease.pid,
    );
    println!("租约：{}", lease_path.display());

    let (job_tx, job_rx) = std::sync::mpsc::channel::<Job>();
    spawn_listener(listener, job_tx);

    let ready = RenderReady::new();
    let render_frames = RenderFrames::default();
    let mut app = App::new();
    // slots 资产源必须在 DefaultPlugins 之前注册：AssetPlugin 在它自己那一步就把
    // 资产源建完了，之后注册的源在 AssetServer 里根本不存在（槽会一直读不到 shader）。
    app.add_plugins(slots::SlotsPlugin)
        .insert_resource(ClearColor(Color::srgb(0.004, 0.005, 0.010)))
        .add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    file_path: asset_root(),
                    watch_for_changes_override: Some(true),
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                })
                .set(render_plugin())
                .disable::<WinitPlugin>(),
        )
        .add_plugins(material::DocMaterialPlugin)
        .add_plugins(shaders::ShaderLibraryPlugin)
        .add_plugins(passes::PassPlugin)
        // 渲染侧的 GPU 时间戳/管线统计。Bevy 只在开了 `tracing-tracy` 时自己加它
        // （`bevy_render/src/lib.rs` 381 行），所以这里显式加一次：我们不用 tracy，
        // 但要它写进 `DiagnosticsStore` 的 `render/**/elapsed_gpu`。
        .add_plugins(RenderDiagnosticsPlugin)
        .insert_resource(FrameProbe(options.fps))
        .add_plugins(ScheduleRunnerPlugin::run_loop(if options.fps {
            Duration::ZERO
        } else {
            Duration::from_secs_f64(1.0 / 60.0)
        }))
        .insert_resource(ready.clone())
        .insert_resource(render_frames.clone())
        .insert_resource(Inbox(std::sync::Mutex::new(job_rx)))
        .insert_resource(InitialSize(options.width, options.height))
        .init_resource::<Active>()
        .init_resource::<Ticks>()
        .init_resource::<art_cache::ArtCache>()
        .init_resource::<ShotStats>()
        // `accept_jobs` 会按产物刷新"可现场改的仪器参数"那张表（窗口的 v/m/n 用它）。
        .init_resource::<SceneInstruments>()
        .insert_resource(gpu_log.clone())
        .insert_resource(PcgRoot(options.pcg_root.clone()))
        .insert_resource(LeaseWatch {
            path: lease_path,
            pid: lease.pid,
        })
        .add_systems(Startup, (assert_vulkan_backend, warm_up).chain())
        .add_systems(Update, (accept_jobs, drive, watch_lease))
        .add_systems(Update, report_frame_time)
        .add_systems(Update, sample_gpu.before(drive))
        .add_systems(Update, tick_frame_clock.before(drive))
        .add_systems(Update, report_device_features)
        .add_systems(Update, idle_between_jobs.after(accept_jobs).before(drive));

    if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
        render_app
            .insert_resource(ready.clone())
            .insert_resource(render_frames.clone())
            // ⚠ 必须排在 `RenderSystems::Cleanup`：`PipelineCache::process_pipeline_queue_system`
            // 在 `RenderSystems::Render` 里（`bevy_render/src/lib.rs` 430 行），排在它之前
            // 看到的是上一帧的队列 —— 那正是"新排队的管线看不见"那条缝。
            .add_systems(
                Render,
                (
                    watch_pipelines.in_set(bevy::render::RenderSystems::Cleanup),
                    gpu_backpressure,
                    drain_gpu_callbacks,
                    count_render_frames,
                ),
            );
    }

    app.run();
    Ok(())
}

fn spawn_listener(listener: TcpListener, jobs: Sender<Job>) {
    std::thread::spawn(move || {
        for connection in listener.incoming() {
            let Ok(mut stream) = connection else {
                continue;
            };
            let jobs = jobs.clone();
            std::thread::spawn(move || {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(300)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(300)));

                let local = ProtocolId::local();
                match stream::read_frame(&mut stream) {
                    Ok(Some(Frame::Protocol(remote))) => {
                        if remote.schema_version != local.schema_version
                            || remote.protocol_hash != local.protocol_hash
                        {
                            let reason = format!(
                                "协议不一致：对端 schema {} / 指纹 {:016x} / git {}，本服务 schema {} / 指纹 {:016x} / git {}",
                                remote.schema_version,
                                remote.protocol_hash,
                                remote.git_rev,
                                local.schema_version,
                                local.protocol_hash,
                                local.git_rev,
                            );
                            let _ = stream::write_frame(&mut stream, &Frame::Refused(reason.clone()));
                            eprintln!("拒绝连接：{reason}");
                            return;
                        }
                    }
                    other => {
                        let reason = format!("握手指望 Protocol 帧，收到 {other:?}");
                        let _ = stream::write_frame(&mut stream, &Frame::Refused(reason));
                        return;
                    }
                }

                if stream::write_frame(&mut stream, &Frame::Protocol(local)).is_err() {
                    return;
                }

                let request = match stream::read_frame(&mut stream) {
                    Ok(Some(Frame::Request(request))) => request,
                    Ok(other) => {
                        let reason = format!("指望 Request 帧，收到 {other:?}");
                        let _ = stream::write_frame(&mut stream, &Frame::Refused(reason));
                        return;
                    }
                    Err(err) => {
                        eprintln!("读请求失败：{err}");
                        return;
                    }
                };

                let (reply_tx, reply_rx) = sync_channel::<Frame>(1);
                if jobs
                    .send(Job {
                        request,
                        reply: reply_tx,
                    })
                    .is_err()
                {
                    return;
                }
                match reply_rx.recv_timeout(Duration::from_secs(300)) {
                    Ok(frame) => {
                        let _ = stream::write_frame(&mut stream, &frame);
                    }
                    Err(err) => {
                        let _ = stream::write_frame(
                            &mut stream,
                            &Frame::Refused(format!("渲染超时或服务已退出：{err}")),
                        );
                    }
                }
            });
        }
    });
}

fn pipeline_label(descriptor: &bevy::material::descriptor::PipelineDescriptor) -> String {
    match descriptor {
        bevy::material::descriptor::PipelineDescriptor::RenderPipelineDescriptor(render) => render
            .label
            .as_deref()
            .unwrap_or("<无名渲染管线>")
            .to_string(),
        bevy::material::descriptor::PipelineDescriptor::ComputePipelineDescriptor(compute) => compute
            .label
            .as_deref()
            .unwrap_or("<无名计算管线>")
            .to_string(),
    }
}

/// 「管线全部就绪」要连着这么多**渲染帧**没有新排队/在编的管线才算数。
///
/// 为什么是"连续 N 帧"而不是"某一帧看到 pending == 0"：`PipelineCache` 里
/// `pipelines()` 只列**已经过 `process_queue`** 的那批，新排队的躺在私有的 `new_pipelines`
/// 里外部看不见（`render_resource/pipeline_cache.rs` 221 行 vs 210/645 行）。所以
/// 「某一帧 pending == 0」可能只是还没轮到它 —— 老代码用「每 20 帧轮询一次 + 重建后先等
/// 一个轮询周期」来盖这个盲区，那两个常数就是它的代价。现在改成**每帧轮询 + 连续 2 帧干净**：
/// 重建后主世界把状态打回 PENDING（见 `accept_jobs`），此后只要有任何一条管线入队，
/// 它在**同一帧**就会被本系统看见（本系统在 `RenderSystems::Cleanup`，排在
/// `process_pipeline_queue_system` 之后）并清零计数 —— 盲区从 20 帧缩到 0 帧。
const PIPELINES_CLEAN_FRAMES: u32 = 2;

fn watch_pipelines(
    cache: Res<PipelineCache>,
    ready: Res<RenderReady>,
    mut announced: Local<bool>,
    mut reported: Local<Vec<String>>,
    mut clean: Local<u32>,
    mut total_seen: Local<usize>,
    mut seen_epoch: Local<u32>,
) {
    let state = ready.get();
    // 主世界每次重建都会把 epoch 加一：那是「全部就绪」这个**瞬时**断言的失效信号，
    // 计数必须从零开始数，不能拿重建前那几帧的干净当证据。
    let now_epoch = ready.epoch.load(Ordering::Relaxed);
    if now_epoch != *seen_epoch {
        *seen_epoch = now_epoch;
        *clean = 0;
    }
    let total = cache.pipelines().count();
    let mut pending = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for pipeline in cache.pipelines() {
        match &pipeline.state {
            CachedPipelineState::Ok(_) => {}
            CachedPipelineState::Err(err) => {
                // ⚠ `ShaderNotLoaded` / `ShaderImportNotYetAvailable` 是**可重试**的：
                // `PipelineCache::process_pipeline` 下一帧就把它们打回 Queued
                // （`pipeline_cache.rs` 685~690 行）。所以它们在 `Err` 里只停留一帧 ——
                // 老代码每 20 帧才轮询一次，看不见；现在每帧看，就必须把它们当"在编"
                // 而不是"失败"，否则会把一次正常的 shader 迟到报成管线编译失败。
                if matches!(
                    err,
                    bevy::shader::ShaderCacheError::ShaderNotLoaded(_)
                        | bevy::shader::ShaderCacheError::ShaderImportNotYetAvailable
                ) {
                    pending += 1;
                } else {
                    failures.push(format!("{}｜{}", pipeline_label(&pipeline.descriptor), err));
                }
            }
            _ => pending += 1,
        }
    }
    if total > 0 && !*announced {
        *announced = true;
        println!("首个渲染管线入队：当前 {total} 条，待编译 {pending} 条");
    }
    if failures != *reported {
        if failures.is_empty() {
            println!("渲染管线恢复正常");
        } else {
            for line in &failures {
                eprintln!("管线编译失败｜{line}");
            }
        }
        *reported = failures.clone();
    }
    if !failures.is_empty() {
        if state != PIPELINES_FAILED {
            eprintln!("⚠ 有管线编译失败，在这修好之前拒绝一切出图任务");
        }
        ready.fail(failures.join("\n"));
        return;
    }
    ready.clear_failure();
    // 缓存条数变了 = 这一帧有管线刚进缓存（多半刚排队）。也算"不干净"。
    let grew = total != *total_seen;
    *total_seen = total;
    if pending > 0 || grew || total == 0 {
        // ⚠ 这里**必须无条件回到 PENDING**，不能写成"只在还不是 READY 时才置位"。
        // 老写法一旦到过 READY 就再也不认新排队的管线 ⇒ 换场景时新材质要现编的那条管线
        // 对 `drive` 完全隐形 ⇒ 会在"管线还没编出来"的帧上截图 ⇒ 那一张云壳是空的，
        // 而且与无云档**逐字节相同**（P32 实测：批量第二步 1 312 701 字节 / e1f99e4b…，
        // 与第一步一模一样）。「全部就绪」是个**瞬时**断言，不是一次性结论。
        if state != PIPELINES_PENDING {
            ready.store(PIPELINES_PENDING);
        }
        return;
    }
    *clean += 1;
    if *clean < PIPELINES_CLEAN_FRAMES {
        return;
    }
    if state != PIPELINES_READY {
        ready.store(PIPELINES_READY);
        println!("渲染管线全部就绪：共 {total} 条，失败 0 条");
    }
}

fn watch_lease(watch: Res<LeaseWatch>, mut ticks: ResMut<Ticks>, mut exit: MessageWriter<AppExit>) {
    ticks.0 += 1;
    if ticks.0 % LEASE_CHECK_INTERVAL != 0 {
        return;
    }
    match client::read_lease(&watch.path) {
        Some(lease) if lease.pid == watch.pid => {}
        _ => {
            println!("租约已易主，渲染服务退出");
            exit.write(AppExit::Success);
        }
    }
}

fn new_target(images: &mut Assets<Image>, width: u32, height: u32) -> Handle<Image> {
    let mut target = Image::new_target_texture(width, height, TextureFormat::Rgba8UnormSrgb, None);
    target.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    images.add(target)
}

fn warm_up(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    size: Res<InitialSize>,
    existing: Option<Res<Canvas>>,
) {
    if existing.is_some() {
        return;
    }
    let handle = new_target(&mut images, size.0, size.1);
    commands.insert_resource(Canvas {
        size: (size.0, size.1),
        target: handle.clone(),
    });

    commands.spawn((
        ScenePart,
        Camera3d::default(),
        DepthPrepass,
        Msaa::Off,
        RenderTarget::Image(handle.into()),
        Transform::from_xyz(0.0, 6.0, 16.0).looking_at(Vec3::new(0.0, 2.0, 0.0), Vec3::Y),
    ));
    commands.spawn((
        ScenePart,
        AmbientLight {
            brightness: 200.0,
            ..default()
        },
    ));
    // ⚠ 这里**故意不摆方向光**（§64.5）：场景的太阳由产物里的灯表摆（点光源）。
    // 再留一盏 9000 lx 的方向光，`sun_light` 的兜底分支就会在出图路上"悄悄换个太阳"
    // （窗口路没有它 ⇒ 同一个 shader 两条路含义不同）；删掉它，两条路的灯清单就一致了。
    commands.spawn((
        ScenePart,
        Mesh3d(meshes.add(Cuboid::new(1.0, 1.0, 1.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.5, 0.5, 0.5),
            ..default()
        })),
        Transform::from_xyz(0.0, 1.0, 0.0),
    ));
}

fn accept_jobs(
    mut commands: Commands,
    inbox: Res<Inbox>,
    ready: Res<RenderReady>,
    frames: Res<RenderFrames>,
    libraries: Res<shaders::ShaderLibraries>,
    mut active: ResMut<Active>,
    mut cache: ResMut<art_cache::ArtCache>,
    mut canvas: ResMut<Canvas>,
    mut scene_assets: SceneAssets,
    mut instruments: ResMut<SceneInstruments>,
    parts: Query<Entity, With<ScenePart>>,
    mut tools: SceneTools,
) {
    // 这条系统一次走完「这一步」：上一步的图写完了就接着搭下一步，队空才回话；
    // 手上没有活儿才去收件箱拿新请求。两条路共用下面这一份搭场景的代码。
    let mut job = match active.0.take() {
        // `pending_preload` 也要接着往下走：那一步正等着"下一帧再建"，不是干完了。
        // （只认 `finished` 的话，推迟的那一步就永远躺在队里没人再取 —— 死锁，而且看起来
        // 像"出图卡住"：帧时间一直在打、一个出图行都没有。）
        Some(job) if job.finished || job.pending_preload => job,
        Some(job) => {
            active.0 = Some(job);
            return;
        }
        None => match inbox.0.lock().expect("收件箱锁坏了").try_recv() {
            Ok(job) => {
                if let Some(detail) = ready.failure() {
                    let _ = job
                        .reply
                        .send(Frame::Refused(format!("渲染管线失败，拒绝出图：{detail}")));
                    return;
                }
                if let Some(detail) = shader_load_failure(&tools.assets, &libraries.0, &[]) {
                    let _ = job.reply.send(Frame::Refused(format!(
                        "shader 资产装载失败，拒绝出图：{detail}"
                    )));
                    return;
                }
                // 性能那两路都要**去掉 60 Hz 上限**：不给 --fps 量到的是帧率上限，不是成本。
                // 与其量出一个假数，不如当场拒掉。
                if !matches!(job.request.job, JobKind::Shots) && !tools.probe.0 {
                    let _ = job.reply.send(Frame::Refused(
                        "性能请求要服务是用 --serve --fps 起的：不给 --fps 主循环会睡觉凑 60 Hz，\
                         量到的是上限不是成本"
                            .to_string(),
                    ));
                    return;
                }
                let queue = match ActiveJob::steps(&job.request) {
                    Ok(queue) => queue,
                    Err(err) => {
                        let _ = job.reply.send(Frame::Refused(err));
                        return;
                    }
                };
                ActiveJob::new(job, queue)
            }
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => return,
        },
    };

    // 队空 ⇒ 这一批出完了：落报告 + 回话（`out` / `bytes` 说的是最后一张）。
    let Some(step) = job.queue.pop_front() else {
        let report = build_report(&job);
        let text = serde_json::to_string_pretty(&report).unwrap_or_else(|err| {
            eprintln!("报告序列化失败：{err}");
            String::new()
        });
        if !job.report_path.is_empty() {
            let path = PathBuf::from(&job.report_path);
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match std::fs::write(&path, format!("{text}\n")) {
                Ok(()) => println!("报告：{}", path.display()),
                Err(err) => eprintln!("报告写不进去（{}）：{err}", path.display()),
            }
        }
        let response = job.response(ready.get() == PIPELINES_READY, text);
        let _ = job.reply.send(Frame::Response(response));
        return;
    };

    // 这一步要的 shader 版本**先挂上一帧**，再建材质（`preload_step_shaders` 里写了为什么）。
    match preload_step_shaders(&step, &tools, &mut cache) {
        Ok(true) => {
            // 本帧只做"把资产挂上"：下一帧资产才在渲染世界的 ShaderCache 里可见。
            job.queue.push_front(step);
            job.pending_preload = true;
            active.0 = Some(job);
            return;
        }
        Ok(false) => {}
        Err(err) => {
            let _ = job.reply.send(Frame::Refused(err));
            return;
        }
    }
    job.pending_preload = false;

    cache.begin();

    for entity in parts.iter() {
        commands.entity(entity).despawn();
    }

    // 单张的尺寸先摆好：World 那条路搭场景时就要拿到 target。
    // 对照图要等产物相机表读出来才知道几张、几行，所以它在搭完之后再定尺寸。
    if !job.view.sheet && canvas.size != job.cell {
        let handle = new_target(&mut scene_assets.images, job.cell.0, job.cell.1);
        canvas.size = job.cell;
        canvas.target = handle;
    }

    let built = match &step.scene {
        StepScene::World { stream, round } => build_world_scene(
            &mut commands,
            &mut scene_assets.meshes,
            &mut scene_assets.materials,
            &canvas.target,
            stream,
            *round,
        )
        .map(|label| Built {
            label,
            ambient: DEFAULT_AMBIENT,
            skybox: None,
            cameras: Vec::new(),
            installed_shaders: false,
            declared_clouds: false,
            watch: Vec::new(),
            instruments: Vec::new(),
            passes: None,
        }),
        StepScene::Artifact(path) => scene::spawn_document(
            &mut cache,
            &mut commands,
            &mut scene_assets.meshes,
            &mut scene_assets.images,
            &mut scene_assets.doc_materials,
            &tools.assets,
            &tools.root.0,
            path,
        )
        .map(|document| {
            let skybox = scene::skybox_of(&document);
            Built {
                label: document.label,
                ambient: document.ambient,
                skybox,
                cameras: document.cameras,
                installed_shaders: document.installed_shaders,
                declared_clouds: document.declared_clouds,
                watch: document.watched,
                instruments: document.instruments,
                passes: document.passes,
            }
        }),
    };
    let built = match built {
        Ok(built) => built,
        Err(err) => {
            let _ = job.reply.send(Frame::Refused(err));
            return;
        }
    };
    // 只有真把场景搭起来了才结算缓存：一次被拒的请求不该让任何条目变冷。
    println!("{}", cache.sweep());
    println!("{}", cache.stats());

    // 对照图：相机表住在产物里（`.pxart` 的 AssetManifest.cameras）。
    // 一次请求 → 一个场景 → N 个视口 → 一张图，替掉以前「12 个进程 + 12 次全量重建 + CPU 拼图」。
    let columns = job.view.columns.max(1);
    let sheet = if job.view.sheet {
        if built.cameras.is_empty() {
            let _ = job.reply.send(Frame::Refused(
                "--sheet 用的是产物自带的相机表：这一步没带（烘场景时用 px_ops::cameras::review() 灌进 .pxart）"
                    .to_string(),
            ));
            return;
        }
        Some(built.cameras)
    } else {
        None
    };
    let rows = sheet
        .as_ref()
        .map(|cameras| (cameras.len() as u32).div_ceil(columns))
        .unwrap_or(1);
    let target = match &sheet {
        Some(_) => (job.cell.0 * columns, job.cell.1 * rows),
        None => job.cell,
    };
    if canvas.size != target {
        let handle = new_target(&mut scene_assets.images, target.0, target.1);
        canvas.size = target;
        canvas.target = handle;
    }

    let placements: Vec<(Transform, Option<SheetCell>)> = match &sheet {
        Some(cameras) => cameras
            .iter()
            .enumerate()
            .map(|(index, camera)| {
                (
                    scene::camera_for(camera),
                    Some(SheetCell {
                        column: index as u32 % columns,
                        row: index as u32 / columns,
                        width: job.cell.0,
                        height: job.cell.1,
                    }),
                )
            })
            .collect(),
        None => vec![(scene::probe_camera(step.cam), None)],
    };
    for (order, (transform, cell)) in placements.into_iter().enumerate() {
        let camera = commands
            .spawn((
                ScenePart,
                Camera3d::default(),
                DepthPrepass,
                Msaa::Off,
                RenderTarget::Image(canvas.target.clone().into()),
                AmbientLight {
                    brightness: built.ambient,
                    ..default()
                },
                transform,
            ))
            .id();
        if let Some(skybox) = &built.skybox {
            commands.entity(camera).insert(skybox.clone());
        }
        if let Some(cell) = cell {
            // 多相机共用一张 target：各自占一格视口，而且**只有第 0 台清屏**，
            // 否则后面的相机会把前面已经画好的格子擦掉。
            commands.entity(camera).insert(Camera {
                viewport: Some(Viewport {
                    physical_position: UVec2::new(
                        cell.column * cell.width,
                        cell.row * cell.height,
                    ),
                    physical_size: UVec2::new(cell.width, cell.height),
                    depth: 0.0..1.0,
                }),
                order: order as isize,
                clear_color: if order == 0 {
                    ClearColorConfig::Default
                } else {
                    ClearColorConfig::None
                },
                ..default()
            });
        }
    }

    job.label = built.label;
    job.scene_path = match &step.scene {
        StepScene::Artifact(path) => path.clone(),
        StepScene::World { stream, .. } => stream.clone(),
    };
    // 产物**内容键**：`.pxart` 的载荷指纹（与预览窗口判"该不该重建"用的是同一个口径）。
    job.content_key = match &step.scene {
        StepScene::Artifact(_) => art_cache::fingerprint_of(&job.scene_path)
            .map(|key| format!("{key:016x}"))
            .unwrap_or_else(|_| "0".to_string()),
        StepScene::World { .. } => "0".to_string(),
    };
    job.watched = built.watch;
    // 这一步声明的 pass 表进渲染世界（`extract_passes` 抄过去，`run_passes` 逐条录）。
    // 上一步证到的坏跟着重建一起清：明细不是锁（§62）。
    tools.failure.clear();
    tools.passes.0 = built.passes;
    // 窗口那一套键要改的就是它们（渲染器不认识这些参数是什么意思，只按名字与偏移改）。
    instruments.0 = built.instruments;
    if built.installed_shaders {
        println!("这一步装了以前没见过的 shader 版本：它的管线要现编，等 `watch_pipelines` 重新置 READY");
    }
    job.out = step.out;
    job.width = target.0;
    job.height = target.1;
    job.warm = 0;
    job.requested = false;
    job.finished = false;
    // 重建 = 「全部就绪」这个**瞬时**断言当场失效：epoch 加一让渲染世界从头数干净帧，
    // 状态同时打回 PENDING 让 `drive` 这一帧就别往下走。
    // 少了这一对，`drive` 会拿着**重建前**那个 READY 一路往下跑（P32 那个"静默出无云图"）。
    ready.invalidate();
    // 这一步刚重建：图与统计都还没收。
    job.declared_clouds = built.declared_clouds;
    job.shot_done = false;
    // 等条件的耗时从**重建完成**这一刻起算，所以两个起点都记在这里。
    job.rebuilt_instant = Instant::now();
    job.rebuilt_render_frame = frames.0.load(Ordering::Relaxed);
    match job.job_kind {
        JobKind::Perf { windows, drop } => {
            // 老路：先出一张判据图，再从图后面收窗口。
            job.perf = Some(PerfGather::new(windows, drop));
        }
        JobKind::Stable { frames: want } => {
            // 新主路径**不出图**：出图是一次 2240×1400 的回读（几十毫秒），就在要量的东西旁边。
            // 它本来是为了兜"静默出无云图"，而那个兜底换成了更硬的判据：
            // 管线全部就绪（瞬时断言）+ 槽里装的是这一版 shader + 资产装载态。
            job.shot_done = true;
            job.stable = Some(StableGather::new(want));
        }
        JobKind::Shots => {}
    }
    active.0 = Some(job);
}

/// 这一步要的 shader 版本**先挂上（不建材质）**。返回 `true` = 有新版本刚挂上。
///
/// 为什么必须分成两帧：新建的 shader 资产要经过「主世界加进去 → 渲染世界抽走 →
/// `PipelineCache` 的 `ShaderCache` 认得它」之后才可用。材质若在同一帧就建，
/// 它的管线会在 shader 进 `ShaderCache` 之前被特化，Bevy 判 `ShaderNotLoaded` ——
/// 那是**终态**，资产后来到了也不会重试 ⇒ 整条管线永久失败、之后每个请求都被拒。
/// 多花一帧（约 20 ms）换掉一整类随机崩，值。
///
/// 这里**只挂不建**：场景解析要重读一遍产物（~1 ms），但真正的装配一行都没跑，
/// 所以没有"建了一半又要重来"的副作用。
fn preload_step_shaders(
    step: &Step,
    tools: &SceneTools,
    cache: &mut art_cache::ArtCache,
) -> Result<bool, String> {
    let StepScene::Artifact(path) = &step.scene else {
        return Ok(false);
    };
    let spec = px_protocol::scene::read_scene(std::path::Path::new(path))?;
    let mut fresh = false;
    for object in &spec.objects {
        // 槽判定的错**留给装配那一步报**（口径只有一处）；这里只要决定"要不要先挂资产"。
        let member = &object.material.shader;
        let key_path = member.resolve(&tools.root.0)?;
        let entry = cache.shader(&key_path.display().to_string())?;
        let version = slots::version_of(&member.key)?;
        if slots::activate(
            &tools.assets,
            slots::MATERIAL,
            version,
            &entry.value.source,
        ) {
            println!("shader 槽：版本 {version:016x} 第一次见，先挂上，下一帧再建材质");
            fresh = true;
        }
    }
    Ok(fresh)
}

/// 一次请求的报告。各路活共用外壳，各自的账分开列。
fn build_report(job: &ActiveJob) -> Report {
    let protocol = ProtocolId::local();
    Report {
        schema_version: protocol.schema_version,
        protocol_hash: format!("{:016x}", protocol.protocol_hash),
        job: job.job_kind.name().to_string(),
        width: job.shot_reports.last().map(|shot| shot.width).unwrap_or(0),
        height: job.shot_reports.last().map(|shot| shot.height).unwrap_or(0),
        millis: job.started.elapsed().as_millis() as u64,
        shots: job.shot_reports.clone(),
        perf: job.perf_reports.clone(),
        pair: build_pair(&job.perf_reports),
    }
}

/// 配对差：`perf[0] − perf[1]`。**单看绝对值没意义** —— 要判的是"改这一档比参照档贵多少"，
/// 而两台机器/两次会话的绝对值没法比。所以两档一起测、差值在服务端就算好。
fn build_pair(reports: &[PerfReport]) -> Option<Pair> {
    if reports.len() != 2 {
        return None;
    }
    let measured = &reports[0];
    let reference = &reports[1];
    if measured.waits.is_none() || reference.waits.is_none() {
        // 老路两个窗口批也能凑成两档，但那是窗口不是配对，别硬算。
        return None;
    }
    let gpu_delta = match (&measured.gpu_ms, &reference.gpu_ms) {
        (Some(one), Some(two)) => Some(one.p50 - two.p50),
        _ => None,
    };
    let (Some(measured_mean), Some(reference_mean)) = (
        measured.compare.as_ref().map(|compare| compare.app_mean_ms),
        reference.compare.as_ref().map(|compare| compare.app_mean_ms),
    ) else {
        return None;
    };
    Some(Pair {
        measured: measured.label.clone(),
        reference: reference.label.clone(),
        app_delta_ms: measured.p50 - reference.p50,
        app_mean_delta_ms: measured_mean - reference_mean,
        gpu_delta_ms: gpu_delta,
        rule: "配对差 = perf[0] − perf[1]（被测档 − 参照档）；同一个会话里先后测，不是同时测。\
               ⚠ `app_delta_ms` 用的是 app 的中位，那个量在无窗口这条路里没有意义（双峰），\
               要看 `app_mean_delta_ms` 或 `gpu_delta_ms`"
            .to_string(),
    })
}

/// 场景产物里的成员键从哪个 CAS 根解析（`--pcg-root`，默认 `target/pcg`）。
/// 只有服务端用它：解析成员的是服务进程，客户端那份旗标最多报一声。
#[derive(Resource, Clone)]
struct PcgRoot(PathBuf);

/// 服务侧给装配用的那几样本事。打包成一个 `SystemParam` ——
/// `accept_jobs` 的参数表已经挨着上限（Bevy 的元组最多 16 个），再摊平摆不下。
#[derive(bevy::ecs::system::SystemParam)]
struct SceneTools<'w> {
    assets: Res<'w, AssetServer>,
    root: Res<'w, PcgRoot>,
    /// 帧时间探针开着没有（`--fps`）：性能请求没有它就是在量 60 Hz 的上限。
    probe: Res<'w, FrameProbe>,
    /// 这一步声明的 pass 表（空 = 只有主 pass）。
    passes: ResMut<'w, passes::DocumentPasses>,
    /// pass 执行器证到的坏（有它就不出图，与 `RenderReady.failure` 同一条路）。
    failure: Res<'w, passes::PassFailure>,
}

/// 搭一个场景要往里塞的那些资产。同样的理由打包（§ 上面那条）：
/// 通用渲染把"每样东西一个材质类型"变成"一个材质类型装所有东西"，
/// 但几何、贴图、材质三张资产表还是要一起拿。
#[derive(bevy::ecs::system::SystemParam)]
struct SceneAssets<'w> {
    images: ResMut<'w, Assets<Image>>,
    meshes: ResMut<'w, Assets<Mesh>>,
    materials: ResMut<'w, Assets<StandardMaterial>>,
    doc_materials: ResMut<'w, Assets<material::DocMaterial>>,
}

/// 场景装配的中间态：每个 kind 的装配器只填自己那一段。
fn build_world_scene(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    target: &Handle<Image>,
    stream_path: &str,
    round: Option<u32>,
) -> Result<String, String> {
    let path = PathBuf::from(stream_path);
    let worlds = stream::read_worlds(&path)
        .map_err(|err| format!("读 {} 失败：{err}", path.display()))?;
    if worlds.is_empty() {
        return Err(format!("{} 里没有 World 帧", path.display()));
    }
    if let Some(declared) = stream::declared_protocol(&path).map_err(|err| err.to_string())? {
        let local = ProtocolId::local();
        if declared.protocol_hash != local.protocol_hash {
            return Err(format!(
                "{} 是 {:016x}（git {}）录的，本服务是 {:016x}；用同一份代码重录",
                path.display(),
                declared.protocol_hash,
                declared.git_rev,
                local.protocol_hash,
            ));
        }
    }

    let world: WorldView = match round {
        Some(round) => worlds
            .iter()
            .find(|world| world.round == round)
            .cloned()
            .ok_or_else(|| format!("{} 里没有第 {round} 轮", path.display()))?,
        None => worlds.last().cloned().unwrap(),
    };

    commands.spawn((
        ScenePart,
        Mesh3d(meshes.add(Plane3d::default().mesh().size(28.0, 28.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.10, 0.12, 0.17),
            perceptual_roughness: 0.9,
            ..default()
        })),
    ));

    let departments = world.departments.len().max(1);
    let peak_stack = world
        .departments
        .iter()
        .map(|department| {
            department
                .holdings
                .iter()
                .map(|holding| holding.max(0.0))
                .sum::<f32>()
        })
        .fold(0.0_f32, f32::max)
        .max(1e-3);

    for (index, department) in world.departments.iter().enumerate() {
        let x = (index as f32 - (departments as f32 - 1.0) / 2.0) * SPACING;
        let mut level = 0.0_f32;
        for (good, holding) in department.holdings.iter().enumerate() {
            let height = (holding.max(0.0) / peak_stack * MAX_STACK).max(0.04);
            commands.spawn((
                ScenePart,
                Mesh3d(meshes.add(Cuboid::new(1.5, height, 1.5))),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: Color::Srgba(GOOD_COLORS[good % GOOD_COLORS.len()]),
                    perceptual_roughness: 0.5,
                    ..default()
                })),
                Transform::from_xyz(x, level + height / 2.0, -2.0),
            ));
            level += height;
        }
    }

    let goods = world.goods.len().max(1);
    let peak_price = world
        .goods
        .iter()
        .map(|good| good.price.max(0.0))
        .fold(0.0_f32, f32::max)
        .max(1e-3);

    for (good, view) in world.goods.iter().enumerate() {
        let x = (good as f32 - (goods as f32 - 1.0) / 2.0) * SPACING;
        let height = (view.price.max(0.0) / peak_price * MAX_BAR).max(0.04);
        commands.spawn((
            ScenePart,
            Mesh3d(meshes.add(Cuboid::new(0.7, height, 0.7))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::Srgba(GOOD_COLORS[good % GOOD_COLORS.len()]),
                perceptual_roughness: 0.25,
                metallic: 0.6,
                ..default()
            })),
            Transform::from_xyz(x, height / 2.0, 3.0),
        ));
    }

    commands.spawn((
        ScenePart,
        AmbientLight {
            brightness: 200.0,
            ..default()
        },
    ));
    // ⚠ 这里**故意不摆方向光**（§64.5）：场景的太阳由 `spawn_lights` 摆成 `PointLight`。
    // 再留一盏 9000 lx 的方向光，`sun_light` 的兜底分支就会在出图路上"悄悄换个太阳"
    // （窗口路没有它 ⇒ 同一个 shader 两条路含义不同）；删掉它，两条路的灯清单就一致了。
    commands.spawn((
        ScenePart,
        Camera3d::default(),
        DepthPrepass,
        Msaa::Off,
        RenderTarget::Image(target.clone().into()),
        Transform::from_xyz(0.0, 6.0, 16.0).looking_at(Vec3::new(0.0, 2.0, 0.0), Vec3::Y),
    ));

    Ok(format!("第 {} 轮", world.round))
}

#[derive(Resource)]
struct FrameProbe(bool);

fn report_frame_time(
    probe: Res<FrameProbe>,
    gpu: Res<GpuLog>,
    mut active: ResMut<Active>,
    mut since: Local<Option<Instant>>,
    mut frames: Local<u32>,
) {
    if !probe.0 {
        return;
    }
    let now = Instant::now();
    let Some(previous) = *since else {
        *since = Some(now);
        return;
    };
    *frames += 1;
    if *frames < FRAME_PROBE_WINDOW {
        return;
    }
    let elapsed = now.duration_since(previous);
    let count = f64::from(*frames);
    let millis = elapsed.as_secs_f64() * 1000.0 / count;
    println!(
        "帧时间 {:.2} ms（{:.1} fps，{} 帧平均）",
        millis,
        count / elapsed.as_secs_f64(),
        *frames,
    );
    // perf 那一路在这里收窗口：窗口的边界就是这一行，别处再切一次就是第二个定义。
    if let Some(job) = active.0.as_mut()
        && !job.pending_preload
        && let Some(gather) = job.perf.as_mut()
    {
        let sampled = gpu.0.lock().map(|all| all.len()).unwrap_or(0);
        gather.push(millis, sampled);
        if !job.shot_done {
            gather.before_shot += 1;
        }
    }
    *since = Some(now);
    *frames = 0;
}

/// 无窗口这条路没有任何东西替我们压住 CPU → GPU 的队列：`--fps` 把主循环设成
/// `Duration::ZERO`，跑多快就提交多快。wgpu 要等一次提交真完成才回收它那一帧的临时资源
/// （间接绘制校验缓冲每帧 2×1 MiB、staging、聚类回读缓冲…），队列无限深 ⇒ 这些临时资源
/// 一帧一帧堆着不还，显存线性涨到 `0x887A0005`。
/// 有窗口那条路不需要这个：present（尤其 vsync）本身就是节流，在飞的帧数天然有界 ——
/// 所以同样的云在预览窗口里不漏。
/// 每 4 帧等一次「最后一次提交」：在飞的帧数因此有界（约 4 帧的临时资源），
/// 同时留住流水线重叠 —— 每帧都等会把 CPU 编码与 GPU 渲染串起来，帧时间被撑大约 25%，
/// 那是测量失真（仪器量帧时间，不能引进这种偏差）。画面一个像素都不动。
const GPU_IN_FLIGHT_FRAMES: u32 = 4;

fn gpu_backpressure(device: Res<bevy::render::renderer::RenderDevice>, mut ticks: Local<u32>) {
    *ticks += 1;
    if *ticks % GPU_IN_FLIGHT_FRAMES != 0 {
        return;
    }
    let _ = device.poll(bevy::render::render_resource::PollType::wait_indefinitely());
}

#[derive(Component)]
struct FpsReadout;
#[derive(Resource, Clone, Copy)]
struct ShowFps(bool);

/// 产物里那个可以被窗口现场改的整数参数叫什么。**渲染器不认识它是什么意思** ——
/// 云拿它当消融档（`clouds.wgsl` 的 `ABLATE_*`），换个 shader 拿它当别的都行。
/// 只有"照名字改那一格"这条路是渲染器提供的（`scene::Instrument` 记了在哪份材质的第几字节）。
const INSTRUMENT_PARAM: &str = "ablate";
/// 三个键对应的档位码。它们是**那份 shader 定的**（`clouds.wgsl`：0 = 体积、5 = 硬表面、6 = 法线）。
const INSTRUMENT_HEAVY: u32 = 0;
const INSTRUMENT_SURFACE: u32 = 5;
const INSTRUMENT_NORMALS: u32 = 6;

/// 这一步搭出来的可改参数（重建时按新产物换一份）。
#[derive(Resource, Default)]
struct SceneInstruments(Vec<scene::Instrument>);

/// 窗口现场生效的那一档。`None` = 跟着产物走（还没被键覆盖过）。
/// 它**不是**命令行的初值 —— 命令行那份已经随 `--cloud-ablate` 删了。
#[derive(Resource, Default, Clone, Copy)]
struct InstrumentView(Option<u32>);

/// 「用户刚按了 v/m/n」这件事单独记成一次性请求：只认资源 `Changed` 不行 ——
/// 构建期插入的资源第一次 run 也算 changed，那会让窗口一开就被默认值顶掉产物里的档。
#[derive(Resource, Default)]
struct InstrumentKey(Option<u32>);

fn instrument_keys(keys: Res<ButtonInput<KeyCode>>, mut requested: ResMut<InstrumentKey>) {
    let wanted = if keys.just_pressed(KeyCode::KeyV) {
        Some(INSTRUMENT_HEAVY)
    } else if keys.just_pressed(KeyCode::KeyM) {
        Some(INSTRUMENT_SURFACE)
    } else if keys.just_pressed(KeyCode::KeyN) {
        Some(INSTRUMENT_NORMALS)
    } else {
        None
    };
    if let Some(wanted) = wanted {
        requested.0 = Some(wanted);
        println!("仪器档切到：{wanted}");
    }
}

/// 临时覆盖只在**真按了键的那一帧**动手：请求取走就没有了，不和产物抢。
fn apply_instrument(
    mut requested: ResMut<InstrumentKey>,
    mut view: ResMut<InstrumentView>,
    instruments: Res<SceneInstruments>,
    mut materials: ResMut<Assets<material::DocMaterial>>,
) {
    let Some(code) = requested.0.take() else {
        return;
    };
    view.0 = Some(code);
    let mut touched = 0;
    for instrument in instruments.0.iter().filter(|item| item.param == INSTRUMENT_PARAM) {
        let Some(mut material) = materials.get_mut(&instrument.material) else {
            continue;
        };
        let at = instrument.offset as usize;
        let Some(cell) = material.params.get_mut(at..at + 4) else {
            eprintln!(
                "⚠ 仪器参数 '{}' 落在第 {at} 字节，超出参数块（{} 字节）",
                instrument.param,
                material.params.len()
            );
            continue;
        };
        cell.copy_from_slice(&code.to_le_bytes());
        touched += 1;
    }
    println!("应用仪器档 {code}（改了 {touched} 份材质）");
}

/// 重建之后窗口那一档必须与材质里那一格同码：不同码就是「按一下键弹回上一份产物」那条缝
/// 又开了。只在状态变化时报一行，不刷屏。
fn check_instrument_matches(
    view: Res<InstrumentView>,
    instruments: Res<SceneInstruments>,
    materials: Res<Assets<material::DocMaterial>>,
    mut reported: Local<Option<(Option<u32>, u32)>>,
) {
    let Some(instrument) = instruments
        .0
        .iter()
        .find(|item| item.param == INSTRUMENT_PARAM)
    else {
        return;
    };
    let Some(material) = materials.get(&instrument.material) else {
        return;
    };
    let at = instrument.offset as usize;
    let Some(cell) = material.params.get(at..at + 4) else {
        return;
    };
    let actual = u32::from_le_bytes([cell[0], cell[1], cell[2], cell[3]]);
    let state = (view.0, actual);
    if *reported == Some(state) {
        return;
    }
    *reported = Some(state);
    match view.0 {
        Some(code) if code != actual => eprintln!(
            "⚠ 仪器档与材质不一致：窗口 {code}（码 {actual}）—— 这一格被别的东西改过"
        ),
        _ => println!("仪器档与材质一致：码 {actual}"),
    }
}

fn spawn_fps_readout(mut commands: Commands) {
    commands.spawn((
        FpsReadout,
        Text::new("fps --"),
        TextFont {
            font_size: bevy::text::FontSize::Px(15.0),
            ..default()
        },
        TextColor(Color::srgb(0.92, 0.96, 1.0)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(8.0),
            left: Val::Px(10.0),
            padding: UiRect::all(Val::Px(5.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
    ));
}

fn update_fps_readout(
    diagnostics: Res<DiagnosticsStore>,
    mut frames: Local<u32>,
    mut slowest: Local<f32>,
    mut readout: Query<(&mut Text, &mut Visibility), With<FpsReadout>>,
) {
    let Ok((mut text, mut visibility)) = readout.single_mut() else {
        return;
    };
    if *visibility == Visibility::Hidden {
        return;
    }
    *frames += 1;
    let Some(millis) = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FRAME_TIME)
        .and_then(|diagnostic| diagnostic.value())
    else {
        return;
    };
    let millis = millis as f32;
    *slowest = slowest.max(millis);
    if *frames % FPS_REFRESH_FRAMES != 0 {
        return;
    }
    text.0 = format!(
        "{:.1} fps  {:.1} ms  worst {:.1} ms",
        1000.0 / millis.max(1e-3),
        millis,
        *slowest,
    );
    if *frames % FPS_WORST_FRAMES == 0 {
        *slowest = 0.0;
    }
}

fn idle_between_jobs(
    probe: Res<FrameProbe>,
    ready: Res<RenderReady>,
    active: Res<Active>,
    mut cameras: Query<&mut Camera, With<Camera3d>>,
) {
    let wanted = probe.0 || active.0.is_some() || ready.get() == PIPELINES_PENDING;
    for mut camera in cameras.iter_mut() {
        if camera.is_active != wanted {
            camera.is_active = wanted;
        }
    }
}

enum PipelineGate {
    Ready,
    Waiting,
    Refuse(String),
}

/// 出图前那道闸：**能证明坏了就当场拒**，只是还没编完就等（有界），就绪才放行。
///
/// 为什么不是"等到超时再放行"：放行出一张缺材质的图，退出码 / 颜色 / 字节数 / 编译 /
/// 单测全绿，只有哈希看得出（P32 就是这么丢的）。所以超预算的出路是**不出图**，
/// 而不是"带警告照出"。超预算时**不动管线**、不锁失败 ⇒ 下一次请求照样有机会拿到。
///
/// 老写法把"等管线"的预算记在全局 `Ticks` 上（服务启动以来的帧数，`watch_lease` 每帧 +1），
/// 于是服务起来满 1800 帧之后这道闸整个失效；`--fps` 下 1800 帧只要几秒。现在按
/// **这一步**（`rebuilt_instant`）起算。
fn pipeline_gate(
    job: &ActiveJob,
    ready: &RenderReady,
    failure: &passes::PassFailure,
    assets: &AssetServer,
    libraries: &[Handle<Shader>],
) -> PipelineGate {
    if let Some(detail) = ready.failure() {
        return PipelineGate::Refuse(format!("渲染管线失败，拒绝出图：{detail}"));
    }
    if let Some(detail) = failure.get() {
        return PipelineGate::Refuse(format!("pass 表失败，拒绝出图：{detail}"));
    }
    let state = ready.get();
    if state == PIPELINES_FAILED {
        return PipelineGate::Refuse(
            "渲染管线编译失败，拒绝出图（细节见服务端日志的「管线编译失败」行）".to_string(),
        );
    }
    if let Some(detail) = shader_load_failure(assets, libraries, &job.watched) {
        return PipelineGate::Refuse(format!("shader 资产装载失败，拒绝出图：{detail}"));
    }
    if state != PIPELINES_READY {
        if job.rebuilt_instant.elapsed() < PIPELINE_WAIT_BUDGET {
            return PipelineGate::Waiting;
        }
        return PipelineGate::Refuse(format!(
            "⚠ 等渲染管线就绪超过 {PIPELINE_WAIT_BUDGET:?}：这一步不出图（管线还在编，修好或稍后再请求一次就能拿到）"
        ));
    }
    PipelineGate::Ready
}

/// `AssetServer` 那一侧能**证明**的坏：资产（或它的依赖）装载失败。
///
/// 槽里的 shader 走 `AssetServer::add`（同步、当场就有），它编不出来是 `ProcessShaderError`，
/// 由渲染世界的 `watch_pipelines` 报；这里管的是走路径装载的 shader 库。
/// 少了这道检查，`ShaderNotLoaded` / `ShaderImportNotYetAvailable` 在 Bevy 里是**无限重试**
/// 的（`pipeline_cache.rs` 685~690 行），`pending` 永远 > 0 ⇒ 永远不就绪。
fn shader_load_failure(
    assets: &AssetServer,
    libraries: &[Handle<Shader>],
    watched: &[Handle<Shader>],
) -> Option<String> {
    for handle in libraries.iter().chain(watched.iter()) {
        let Some((state, deps, recursive)) = assets.get_load_states(handle) else {
            continue;
        };
        if !(state.is_failed() || deps.is_failed() || recursive.is_failed()) {
            continue;
        }
        let name = handle
            .path()
            .map(|path| path.to_string())
            .unwrap_or_else(|| format!("{:?}", handle.id()));
        let detail = match &state {
            LoadState::Failed(err) => format!("{err}"),
            other => format!("{other:?} / 依赖 {deps:?} / 递归 {recursive:?}"),
        };
        return Some(format!("{name}｜{detail}"));
    }
    None
}

fn drive(
    mut commands: Commands,
    mut active: ResMut<Active>,
    canvas: Res<Canvas>,
    ready: Res<RenderReady>,
    capturing: Query<Entity, With<Capturing>>,
    stats: Res<ShotStats>,
    gpu: Res<GpuLog>,
    frames: Res<RenderFrames>,
    assets: Res<AssetServer>,
    libraries: Res<shaders::ShaderLibraries>,
    store: Res<DiagnosticsStore>,
    pass_failure: Res<passes::PassFailure>,
) {
    let Some(job) = active.0.as_mut() else {
        return;
    };
    // 这一步还没建（在等 shader 版本落地）：`out` 还是空的，什么都别做。
    if job.pending_preload {
        return;
    }

    match pipeline_gate(job, &ready, &pass_failure, &assets, &libraries.0) {
        PipelineGate::Ready => {}
        PipelineGate::Waiting => return,
        PipelineGate::Refuse(reason) => {
            eprintln!("拒绝任务：{reason}");
            let _ = job.reply.send(Frame::Refused(reason));
            active.0 = None;
            return;
        }
    }
    let state = ready.get();

    // 到这里 `ready` 一定是**重建之后**由渲染世界给出的 READY：`accept_jobs` 每次重建都把
    // 状态打回 PENDING，而 `watch_pipelines` 只有连着 `PIPELINES_CLEAN_FRAMES` 个渲染帧
    // 没看见新排队/在编的管线才会重新置 READY。所以老代码那两道路障
    // （「重建后先等 20 帧轮询」与「装完新 shader 先等看见 PENDING」）在这里退休了 ——
    // 它们盖的是同一个盲区，而盲区已经由「每帧轮询 + 连续干净」消掉。
    // ⚠ 只在**第一次**见某一版 shader 时那个盲区才真会咬人（同一版再来是缓存命中）。

    // 新主路径：等条件成立 → 逐帧采样。它**不出图**（理由见 `accept_jobs` 里那一段）。
    if job.stable.is_some() {
        let render_frames = frames.0.load(Ordering::Relaxed);
        if drive_stable(job, state, &assets, &libraries.0, render_frames, &store) {
            let label = job.label.clone();
            let scene = job.scene_path.clone();
            let key = job.content_key.clone();
            let gather = job.stable.take().expect("上面刚查过它还在");
            job.perf_reports.push(build_stable_report(&label, &scene, &key, gather));
            job.finished = true;
        }
        return;
    }

    if !job.requested {
        job.warm += 1;
        if job.warm >= FRAMES_AFTER_JOB {
            // 两个观察者挂同一个实体：一个落盘（Bevy 自带的），一个算这张图的账。
            commands
                .spawn(Screenshot::image(canvas.target.clone()))
                .observe(save_to_disk(job.out.clone()))
                .observe(collect_shot_stat);
            job.requested = true;
        }
        return;
    }

    if !capturing.is_empty() {
        return;
    }

    // 统计与落盘是同一个事件上的两个观察者，执行次序不保证 ⇒ 收不到就再等一帧。
    if !job.shot_done {
        // 出图卡住的兜底：正常最多跨过 1 个窗口，涨到 30 个就是真卡了（文件写不出去之类），
        // 与其无限等一个永远不出现的文件，不如拒掉并说清。
        if let Some(gather) = job.perf.as_ref()
            && gather.before_shot > 30
        {
            let reason = format!(
                "出图卡住了：等了 {} 个窗口（约 {} s）也没等到 {}；服务端日志里找截图那行报错",
                gather.before_shot,
                (gather.before_shot as f64 * 120.0 * 0.05) as u64,
                job.out.display()
            );
            eprintln!("拒绝任务：{reason}");
            let _ = job.reply.send(Frame::Refused(reason));
            active.0 = None;
            return;
        }
        let Some(stat) = take_shot_stat(&stats) else {
            return;
        };
        let path = job.out.clone();
        if std::fs::metadata(&path).is_err() {
            // 落盘那个观察者还没写完（或写失败）：把统计塞回去，下一帧再试。
            if let Ok(mut all) = stats.0.lock() {
                all.push(stat);
            }
            return;
        }
        let bytes = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
        let sha256 = px_render::digest::sha256_file(&path).unwrap_or_default();
        // 这一张是不是**参考图**（= 这一批的第一个）。参考图没人可比，判据降一档：
        // 只能判"不是占位、而且真画出了东西"；要判"丢云壳"必须把无云档放这一批第一个。
        let is_reference = job.ref_grid.is_none();
        let reference = job.ref_grid.get_or_insert_with(|| stat.grid.clone());
        let diff: u64 = reference
            .iter()
            .zip(stat.grid.iter())
            .map(|(one, two)| one.abs_diff(*two))
            .sum();
        // 阈值按**参考图的总光通量**按比例给：差分与阈值同量纲（亮度×像素），
        // 换个分辨率/换个相机都对得上，不用手调。
        let reference_luma: u64 = reference.iter().sum();
        let threshold = ((CLOUD_DIFF_RATIO * reference_luma as f64) as u64).max(1);
        let declared = job.declared_clouds;
        let has_cloud = declared
            && stat.placeholder_px == 0
            && if is_reference {
                stat.bright_px > 0
            } else {
                diff >= threshold
            };
        let verdict = if stat.placeholder_px > 0 {
            format!(
                "⚠ 画的是占位 shader：{} 个洋红像素（这一张不算数）",
                stat.placeholder_px
            )
        } else if !declared {
            "无云档（产物没声明 clouds part）".to_string()
        } else if is_reference {
            format!(
                "有云？（这一张是参考图，只能判「不是占位、亮像素 {} > 0」；要判丢云壳得把无云档放这一批第一个）",
                stat.bright_px
            )
        } else if has_cloud {
            format!("有云 ✓（与参考图的网格差分 {diff} ≥ {threshold}）")
        } else {
            format!("⚠ 该有云却没画出来（网格差分 {diff} < {threshold}）")
        };
        println!("出图：{} → {}（{}×{}，{} 字节，{} ms，sha256 {}）", job.label, path.display(), job.width, job.height, bytes, job.started.elapsed().as_millis() as u64, &sha256[..sha256.len().min(16)]);
        println!("  可用性：{verdict}");
        job.shots.push(path.display().to_string());
        job.shot_reports.push(ShotReport {
            scene: job.scene_path.clone(),
            label: job.label.clone(),
            out: path.display().to_string(),
            width: job.width,
            height: job.height,
            bytes,
            sha256,
            placeholder_px: stat.placeholder_px,
            bright_px: stat.bright_px,
            diff_vs_ref_grid: diff,
            declared_clouds: declared,
            has_cloud,
            verdict,
        });
        job.shot_done = true;
        if job.perf.is_some() {
            println!(
                "  开始收帧窗口（丢 {} 个 + 收 {} 个，每窗 {} 帧）",
                job.perf.as_ref().map(|g| g.drop).unwrap_or(0),
                job.perf.as_ref().map(|g| g.want).unwrap_or(0),
                FRAME_PROBE_WINDOW
            );
            return;
        }
    }

    // perf 那一路：还要等窗口收够。
    if let Some(gather) = job.perf.as_ref()
        && !gather.done()
    {
        return;
    }
    if let Some(gather) = job.perf.take() {
        let mut report = build_perf_report(&job.label, &job.scene_path, gather, &gpu);
        // 老路也把产物键带上：`--changed` 对新旧两路的报告都能用。
        report.key = job.content_key.clone();
        job.perf_reports.push(report);
    }
    job.finished = true;
}

/// 把一串窗口折成 `PerfReport`。**原始值全打**，汇总只是附带的；
/// 误差棒口径写进报告里，免得看数的人猜。
fn build_perf_report(label: &str, scene: &str, gather: PerfGather, gpu: &GpuLog) -> PerfReport {
    let readings = gpu.0.lock().map(|all| all.clone()).unwrap_or_default();
    let mut samples = Vec::new();
    for (index, _window) in gather.windows.iter().enumerate() {
        let from = *gather.marks.get(index).unwrap_or(&0);
        let to = *gather.marks.get(index + 1).unwrap_or(&readings.len());
        let slice = readings.get(from.min(readings.len())..to.min(readings.len())).unwrap_or(&[]);
        let values = |pick: fn(&GpuReading) -> f64| -> Vec<f64> {
            slice
                .iter()
                .map(pick)
                .filter(|value| !value.is_nan())
                .collect()
        };
        let range = |mut values: Vec<f64>| -> Vec<f64> {
            if values.is_empty() {
                return Vec::new();
            }
            values.sort_by(|one, two| one.partial_cmp(two).unwrap_or(std::cmp::Ordering::Equal));
            vec![values[0], values[values.len() - 1]]
        };
        let max = |values: Vec<f64>| values.iter().cloned().fold(f64::NAN, f64::max);
        samples.push(GpuSample {
            window: (index as u32) + 1,
            sm_mhz: range(values(|r| r.sm)),
            power_w: range(values(|r| r.power)),
            util_pct: range(values(|r| r.util)),
            vram_mib_max: max(values(|r| r.vram)),
            temp_c_max: max(values(|r| r.temp)),
            samples: slice.len() as u32,
        });
    }
    let min = gather
        .windows
        .iter()
        .cloned()
        .fold(f64::INFINITY, f64::min);
    let max = gather.windows.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let bar = if gather.windows.is_empty() { f64::NAN } else { (max - min) / 2.0 };
    PerfReport {
        scene: scene.to_string(),
        label: label.to_string(),
        min: if min.is_finite() { min } else { f64::NAN },
        median: median_of(&gather.windows),
        n: gather.windows.len() as u32,
        error_bar: ErrorBar {
            rule: "单次请求内：干净窗口的半极差 (max−min)/2；跨轮/跨请求的误差棒由调用方按各次中位数另算"
                .to_string(),
            value_ms: bar,
        },
        windows: gather.windows,
        dropped: gather.dropped,
        gpu: samples,
        // 老路不采逐帧、不算分位数、没有 GPU 时间戳：这几项留空/零，`Report.job` 说了算。
        frames: Vec::new(),
        p50: f64::NAN,
        p90: f64::NAN,
        p99: f64::NAN,
        max: if max.is_finite() { max } else { f64::NAN },
        key: String::new(),
        waits: None,
        gpu_ms: None,
        compare: None,
    }
}

/// 新主路径的状态机：每一步都**等条件**成立，并把等的时间量出来。
/// 返回 `true` = 采完了，调用方去结算。
///
/// 三个条件各自替掉一个经验常数：
///   · 管线就绪 ← 替掉「热身 4 s」与「丢 1 个 120 帧的窗」；
///   · 资产装完 ← 替掉「再等一会儿」；
///   · 重建后已渲染 K 帧 ← 替掉「丢窗」的剩下那一半。
fn drive_stable(
    job: &mut ActiveJob,
    state: u8,
    assets: &AssetServer,
    libraries: &[Handle<Shader>],
    render_frames: u32,
    store: &DiagnosticsStore,
) -> bool {
    let Some(gather) = job.stable.as_mut() else {
        return false;
    };
    if gather.done {
        return true;
    }
    match gather.phase {
        StablePhase::Pipelines => {
            if state != PIPELINES_READY {
                return false;
            }
            gather.pipelines_ms = gather.phase_ms();
            gather.enter(StablePhase::Assets);
            false
        }
        StablePhase::Assets => {
            // 场景内容是**同步** `Assets::add` 进来的（`images.add` / `meshes.add`），
            // 没有 `AssetServer` 的装载态可言；真在 `AssetServer` 上挂过号的只有
            // 槽里的 shader（`AssetServer::add`）与 shader 库（`AssetServer::load`）。
            // 所以这一条**只对它们**成立，不是"整个场景装完了"的证明 —— 报告里就这么写。
            let handles: Vec<(&str, &Handle<Shader>)> = libraries
                .iter()
                .map(|handle| ("库", handle))
                .chain(job.watched.iter().map(|handle| ("槽", handle)))
                .collect();
            if handles
                .iter()
                .any(|(_, handle)| !assets.is_loaded_with_dependencies(*handle))
            {
                return false;
            }
            let pending = handles
                .iter()
                .filter(|(_, handle)| {
                    !matches!(assets.load_state(*handle), bevy::asset::LoadState::Loaded)
                })
                .count();
            gather.assets_ms = gather.phase_ms();
            if !gather.assets_logged {
                gather.assets_logged = true;
                println!(
                    "等资产：{} 个句柄全部 `Loaded`（其中「非 Loaded」{pending} 个），等了 {:.1} ms",
                    handles.len(),
                    gather.assets_ms
                );
                for (kind, handle) in &handles {
                    if let Some((state, deps, rec)) = assets.get_load_states(*handle) {
                        println!("  资产 {kind} {:?}：{state:?} / 依赖 {deps:?} / 递归 {rec:?}", handle.id());
                    }
                }
            }
            gather.enter(StablePhase::Settle);
            false
        }
        StablePhase::Settle => {
            if render_frames.saturating_sub(job.rebuilt_render_frame) < STABLE_SETTLE_FRAMES {
                return false;
            }
            gather.settle_ms = gather.phase_ms();
            // GPU 那一串从"这一刻之后到的批次"算起：之前的批次属于还没数进采样窗的帧。
            gather.gpu_seen = gpu_snapshot(store).map(|(_, _, time)| time);
            gather.enter(StablePhase::Sample);
            // 采样段从这一刻起算（含后面为了对齐 GPU 而多等的那几帧）。
            gather.sample_started = Instant::now();
            false
        }
        StablePhase::Sample => {
            // 逐帧 app 循环毫秒由 `tick_frame_clock` 推进，这里只看够没够。
            if gather.frames.len() as u32 >= gather.want {
                gather.enter(StablePhase::Flush);
            }
            false
        }
        StablePhase::Flush => {
            if gather.gpu.len() as u32 >= gather.want {
                gather.gpu_lag_frames = gather.flush_frames;
                gather.sample_ms = gather.sample_started.elapsed().as_secs_f64() * 1000.0;
                gather.total_ms = gather.started.elapsed().as_secs_f64() * 1000.0;
                gather.done = true;
                return true;
            }
            // 设备没有 `Features::TIMESTAMP_QUERY` 时这一串永远是空的：
            // 不为一个拿不到的信号死等 —— 那时报告里 `gpu_ms` 是 `null`，app 侧的数照给。
            if gather.gpu_seen.is_none() {
                gather.sample_ms = gather.sample_started.elapsed().as_secs_f64() * 1000.0;
                gather.total_ms = gather.started.elapsed().as_secs_f64() * 1000.0;
                gather.done = true;
                return true;
            }
            gather.flush_frames += 1;
            if gather.flush_frames > STABLE_FLUSH_LIMIT {
                eprintln!(
                    "⚠ GPU 时间戳等不到第 {} 条，照现有 {} 条结算",
                    gather.want,
                    gather.gpu.len()
                );
                gather.sample_ms = gather.sample_started.elapsed().as_secs_f64() * 1000.0;
                gather.total_ms = gather.started.elapsed().as_secs_f64() * 1000.0;
                gather.done = true;
                return true;
            }
            false
        }
    }
}

/// GPU 那一串凑不满时最多再等几帧。
///
/// GPU 时间戳的**到达率不是 1 帧 1 条**：Bevy 的 `RenderDiagnosticsMutex` 只有一个槽
/// （`bevy_render/src/diagnostic/internal.rs` 120 行 `*mutex = Some(diagnostics)`），
/// 一次 `begin_frame` 里完成多条时只留最后一条；而 `gpu_backpressure` 每 4 帧
/// `poll(wait_indefinitely)` 一次，那一次会把好几条一起放出来。实测：GPU 不饱和时
/// 1 帧 1 条，饱和时 **1 帧 1/4 条**。所以这里按时**帧数上限**兜底，拿多少算多少，
/// 报告里 `gpu_ms.n` 说了实际条数。
const STABLE_FLUSH_LIMIT: u32 = 240;

/// 让 Bevy 的 GPU 诊断读回**及时**回调。
///
/// `DiagnosticsRecorder` 的 `map_async` 回调只在 `device.poll` 里被处理，而
/// `gpu_backpressure` 每 `GPU_IN_FLIGHT_FRAMES`（= 4）帧才 poll 一次。实测后果：
/// **每 4 个渲染帧才拿到 1 条 GPU 时间戳**（`smoke` 那次 30 帧样本要再等 88 帧）。
/// 这里补一个**非阻塞** poll：只处理回调，不等任何提交完成，也不动在飞帧数的上界。
/// 有了它，GPU 那一串的采样率回到 1 帧 1 条，滞后只剩读回本身那几帧。
fn drain_gpu_callbacks(device: Res<RenderDevice>) {
    let _ = device.poll(bevy::render::render_resource::PollType::Poll);
}

/// 逐帧 app 循环毫秒。老协议是"120 帧平均成一个窗口"——分位数与长尾只能从**逐帧**序列上算，
/// 所以新主路径直接存原始帧。测量点在 `Update` 里的固定一处，与前后的系统无关。
fn tick_frame_clock(mut active: ResMut<Active>, mut last: Local<Option<Instant>>) {
    let now = Instant::now();
    let Some(previous) = *last else {
        *last = Some(now);
        return;
    };
    *last = Some(now);
    let millis = now.duration_since(previous).as_secs_f64() * 1000.0;
    let Some(job) = active.0.as_mut() else {
        return;
    };
    let Some(gather) = job.stable.as_mut() else {
        return;
    };
    if gather.phase == StablePhase::Sample && (gather.frames.len() as u32) < gather.want {
        gather.frames.push(millis);
    }
}

/// 把 GPU 时间戳捞进帧序列。
///
/// **读回不在主循环里等**：Bevy 的 `RenderDiagnosticsPlugin` 每帧把时间戳 query set
/// resolve 进一个 buffer、再 `copy_buffer_to_buffer` 进 staging，然后 `map_async`；
/// 数据到达后在**后面某一帧**的 `begin_frame` 里被 `run_mapped_callback` 取走，
/// 由 `sync_diagnostics`（`PreUpdate`）写进 `DiagnosticsStore`
/// （`bevy_render/src/diagnostic/internal.rs` 的 `submitted_frames` / `finish` /
/// `run_mapped_callback`）。所以这里读到的是**几帧之前**那一帧的 GPU 时间，代价只有
/// 一次 `DiagnosticsStore` 遍历（报告里的 `poll_us` 就是它）。同步等会 stall，
/// 而 stall 正好污染要量的东西 —— 这条环形缓冲就是为它存在的。
fn sample_gpu(store: Res<DiagnosticsStore>, mut active: ResMut<Active>) {
    let started = Instant::now();
    let snapshot = gpu_snapshot(&store);
    let poll_us = started.elapsed().as_secs_f64() * 1e6;
    let Some(job) = active.0.as_mut() else {
        return;
    };
    let Some(gather) = job.stable.as_mut() else {
        return;
    };
    if !matches!(gather.phase, StablePhase::Sample | StablePhase::Flush) {
        return;
    }
    gather.poll_us_total += poll_us;
    gather.poll_us_count += 1;
    let Some((source, sum, newest)) = snapshot else {
        return;
    };
    gather.gpu_source = source;
    // 同一个批次在连续几帧里都还在 `DiagnosticsStore` 里；只认"来了新的"。
    if gather.gpu_seen == Some(newest) {
        return;
    }
    gather.gpu_seen = Some(newest);
    if (gather.gpu.len() as u32) < gather.want {
        gather.gpu.push(sum);
    }
}

/// 后端锁死 Vulkan。为什么必须在代码里写死、不能只靠环境变量：
/// `WgpuSettings::default()` 会读 `WGPU_BACKEND`（`bevy_render-0.19.1/src/settings.rs:84`
/// → `wgpu-types-29.0.4/src/backend.rs:156`），而全仓唯一读它的就是那一处；这里显式给出
/// `backends` 之后，Bevy 直接拿它去建 `InstanceDescriptor`（`renderer/mod.rs:188`），
/// 环境里的值就再也盖不过来了（实测见 `tools/harness.ps1` 头注）。
/// 值：同一 exe/场景/shader 下 DX12 40.1 ms vs Vulkan 17.5 ms（2.3×，长尾 3.6×）。
fn wgpu_settings() -> WgpuSettings {
    WgpuSettings {
        backends: Some(Backends::VULKAN),
        ..default()
    }
}

fn render_plugin() -> RenderPlugin {
    RenderPlugin {
        render_creation: RenderCreation::Automatic(Box::new(wgpu_settings())),
        ..default()
    }
}

/// 后端的断言退出码。非 0 是为了让 shell 侧"起服务"这一步也判失败，不必只靠日志。
const BACKEND_EXIT_CODE: i32 = 2;

/// 只清**自己**那份租约：`serve` 是写完租约才起 app 的，断言在这里退出就会把它留在盘上，
/// 下一次合法启动会被单例闸拦下 —— 那是**假故障**，比原故障更难查。
/// pid 对不上（`view` 那一侧看到的可能是别人服务的租约）就一概不碰。
fn clear_own_lease(path: &std::path::Path) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(lease) = serde_json::from_str::<Lease>(&text) else {
        return;
    };
    if lease.pid == std::process::id() {
        let _ = std::fs::remove_file(path);
    }
}

/// 启动时把**实际**后端喊出来；不是 Vulkan 就硬失败退出。
/// 为什么要有这条门：环境事故（某支脚本 dot-source 后留下 `WGPU_BACKEND=dx12`）会静默把
/// 整轮测量的量级换掉，而日志里只有一个不显眼的 `AdapterInfo`。喊出来 + 失败才是可判的。
/// stderr 那行的固定前缀 `后端断言失败` 被 `tools/harness.ps1` 的 fail-fast 正则认。
fn assert_vulkan_backend(adapter: Option<Res<RenderAdapterInfo>>, mut done: Local<bool>) {
    if *done {
        return;
    }
    // 渲染资源要等 RenderPlugin 走完 finish；拿不到就下一帧再看。
    let Some(adapter) = adapter else {
        return;
    };
    *done = true;
    println!(
        "后端：{:?}｜适配器 {}｜驱动 {} {}｜类型 {:?}",
        adapter.backend, adapter.name, adapter.driver, adapter.driver_info, adapter.device_type,
    );
    if !Backends::from(adapter.backend).contains(Backends::VULKAN) {
        eprintln!(
            "后端断言失败：实际 {:?}，要求 Vulkan（WGPU_BACKEND={:?}）—— 后端决定帧时间量级，不许静默换",
            adapter.backend,
            std::env::var("WGPU_BACKEND").ok(),
        );
        clear_own_lease(&client::lease_path());
        std::process::exit(BACKEND_EXIT_CODE);
    }
}

/// 设备到底给不给时间戳 —— 这是 A 档要的"本机能不能拿到"的**实测**出处。
/// 一次就够，所以只打一遍。
fn report_device_features(
    device: Option<Res<RenderDevice>>,
    adapter: Option<Res<RenderAdapterInfo>>,
    queue: Option<Res<RenderQueue>>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let (Some(device), Some(adapter)) = (device, adapter) else {
        return;
    };
    *done = true;
    let features = device.features();
    let inside = features.contains(bevy::render::render_resource::WgpuFeatures::TIMESTAMP_QUERY_INSIDE_PASSES);
    let encoders =
        features.contains(bevy::render::render_resource::WgpuFeatures::TIMESTAMP_QUERY_INSIDE_ENCODERS);
    let timestamps = features.contains(bevy::render::render_resource::WgpuFeatures::TIMESTAMP_QUERY);
    let stats = features.contains(bevy::render::render_resource::WgpuFeatures::PIPELINE_STATISTICS_QUERY);
    println!("GPU 设备：{}", adapter.name);
    println!(
        "GPU 时间戳能力：TIMESTAMP_QUERY={timestamps} INSIDE_ENCODERS={encoders} INSIDE_PASSES={inside} PIPELINE_STATISTICS={stats}｜时间戳周期 {} ns",
        queue.as_ref().map(|queue| queue.get_timestamp_period()).unwrap_or(f32::NAN)
    );
}

/// 新主路径的结算：分位数 + 原始逐帧序列 + 每次等待的实测耗时 + GPU 那一路。
fn build_stable_report(label: &str, scene: &str, key: &str, gather: StableGather) -> PerfReport {
    let (min, p50, p90, p99, max) = quantiles(&gather.frames);
    let gpu_ms = if gather.gpu.is_empty() {
        None
    } else {
        let (gmin, gp50, gp90, gp99, gmax) = quantiles(&gather.gpu);
        Some(GpuMs {
            source: gather.gpu_source.clone(),
            lag_frames: gather.gpu_lag_frames,
            n: gather.gpu.len() as u32,
            min: gmin,
            p50: gp50,
            p90: gp90,
            p99: gp99,
            max: gmax,
            frames: gather.gpu.clone(),
            poll_us: if gather.poll_us_count > 0 {
                gather.poll_us_total / f64::from(gather.poll_us_count)
            } else {
                0.0
            },
        })
    };
    let compare = gpu_ms.as_ref().map(|gpu| Compare {
        app_p50_ms: p50,
        app_mean_ms: mean_of(&gather.frames),
        gpu_p50_ms: gpu.p50,
        ratio: if p50 > 0.0 { gpu.p50 / p50 } else { f64::NAN },
        ratio_mean: {
            let mean = mean_of(&gather.frames);
            if mean > 0.0 { gpu.p50 / mean } else { f64::NAN }
        },
        note: "以前拿 app 循环周期当 GPU 时间的代理量。⚠ app 逐帧序列在无窗口这条路里是\
               **双峰**的：`gpu_backpressure` 每 4 帧一次 poll(wait_indefinitely)，那一帧特别长 ——\
               所以看 app 的均值，别拿它的中位当「每帧成本」"
            .to_string(),
    });
    PerfReport {
        scene: scene.to_string(),
        label: label.to_string(),
        windows: Vec::new(),
        dropped: Vec::new(),
        min,
        median: p50,
        n: gather.frames.len() as u32,
        error_bar: ErrorBar {
            rule: "新主路径：逐帧分位数（线性插值，numpy 'linear' / R type 7，n 带着）；\
                   value_ms = p99 − p50 = 长尾"
                .to_string(),
            value_ms: if p99.is_finite() && p50.is_finite() {
                p99 - p50
            } else {
                f64::NAN
            },
        },
        gpu: Vec::new(),
        frames: gather.frames.clone(),
        p50,
        p90,
        p99,
        max,
        key: key.to_string(),
        waits: Some(Waits {
            pipelines_ms: gather.pipelines_ms,
            assets_ms: gather.assets_ms,
            settle_ms: gather.settle_ms,
            sample_ms: gather.sample_ms,
            total_ms: gather.total_ms,
        }),
        gpu_ms,
        compare,
    }
}

const DEFAULT_AMBIENT: f32 = 80.0;
const FRAME_PROBE_WINDOW: u32 = 120;
const FPS_REFRESH_FRAMES: u32 = 10;
const FPS_WORST_FRAMES: u32 = 120;

const VIEW_REQUEST: &str = "target/viewer-scene.json";
const VIEW_LEASE: &str = "target/viewer.json";
/// 窗口对"相机在哪儿"的回话（§64.8）。
const VIEW_CAMERA: &str = "target/viewer-camera.json";

/// 推给常驻窗口的东西：**哪一份场景产物** + 它的内容键。内容一个字段都不进来 ——
/// 窗口自己去 CAS 取同一份产物。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ViewRequest {
    scene: String,
    /// 场景产物的内容键（`read_manifest` 的载荷指纹）。窗口只在它变了才重建。
    #[serde(default)]
    key: u64,
    at: u64,
    #[serde(default)]
    shot: bool,
    /// 窗口不开对照图（多视口是 `--serve` 出图的事），只用来打一行说明。
    #[serde(default)]
    sheet: bool,
    /// 问一句"相机在哪儿"：窗口把方位写进 `VIEW_CAMERA`（§64.8）。
    /// ⚠ 与 `shot` 一样是**附带动作**：`scene` / `key` 沿用上一次请求那份，不为问一句话换场景。
    #[serde(default)]
    ask_camera: bool,
    /// 顺手把相机摆到这个方位（`yaw,pitch,distance`）：复现某个视角用。`None` = 不动。
    #[serde(default)]
    set_camera: Option<[f32; 3]>,
}

/// 窗口回话：**复现一个视角要的那三个数**，外加位置与视口尺寸（对账用）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CameraReply {
    /// 回的是哪一次请求（`ViewRequest.at`）。
    at: u64,
    yaw: f32,
    pitch: f32,
    distance: f32,
    position: [f32; 3],
    size: [u32; 2],
}

#[derive(Resource)]
struct Viewer {
    scene: String,
    /// 这次请求说的内容键。
    key: u64,
    /// 上次真拿它重建过时的载荷指纹（`poll_scene` 靠它分辨「动过」和「变了」）。
    fingerprint: Option<u64>,
    scene_modified: Option<std::time::SystemTime>,
    request_at: u64,
    sheet: bool,
}

#[derive(Resource)]
struct Orbit {
    yaw: f32,
    pitch: f32,
    distance: f32,
}

#[derive(Resource)]
struct Rebuild(bool);

#[derive(Resource, Default)]
struct PendingShot(Option<u32>);

fn keep_rendering(
    error: &bevy::render::error_handler::RenderError,
    _main: &mut World,
    _render: &mut World,
) -> bevy::render::error_handler::RenderErrorPolicy {
    eprintln!("预览窗口遇到渲染错误，选择继续（不退出）：{:?}", error.ty);
    bevy::render::error_handler::RenderErrorPolicy::Ignore
}

fn now_nanos() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos() as u64)
        .unwrap_or(0)
}

/// 窗口标题里那一行：场景产物路径 + 内容键前几位。
fn describe(viewer: &Viewer) -> String {
    format!("{}｜键 {:016x}", viewer.scene, viewer.key)
}

fn lease_age() -> Option<Duration> {
    let stamp: u64 = std::fs::read_to_string(VIEW_LEASE).ok()?.trim().parse().ok()?;
    Some(Duration::from_nanos(now_nanos().saturating_sub(stamp)))
}

fn show(options: &Options) -> Result<(), String> {
    let Some(scene) = options.scene_path()? else {
        return Err("--show 需要一份场景产物：--scene <SCENE.pxart>".to_string());
    };
    let scene = scene.display().to_string();
    let key = art_cache::fingerprint_of(&scene)?;
    let request = ViewRequest {
        scene: scene.clone(),
        key,
        at: now_nanos(),
        shot: options.shot,
        sheet: options.sheet,
        ask_camera: false,
        set_camera: None,
    };
    let text = serde_json::to_string_pretty(&request).map_err(|err| err.to_string())?;
    std::fs::write(VIEW_REQUEST, text).map_err(|err| format!("写 {VIEW_REQUEST} 失败：{err}"))?;

    println!("已推给常驻窗口：{scene}（键 {key:016x}）");
    match lease_age() {
        Some(age) if age < Duration::from_secs(5) => {
            println!("窗口在线（心跳 {:.1} s 前）", age.as_secs_f32());
        }
        _ => println!("⚠ 没检测到在跑的窗口；先执行 `px_render --view` 开一个，它会一直留着"),
    }
    Ok(())
}

/// `--where` / `--place`：**只问/只摆窗口的相机**，不换场景、不重烘（§64.8）。
///
/// 为什么需要这条 API：窗口的轨道相机只有鼠标能改，而"缝在不在"依赖那个视角 ——
/// 量的时候必须能把当时的方位**读回来**（写进命令行的 `--place`），才算有了确定性复现。
fn viewer_camera(options: &Options) -> Result<(), String> {
    let previous = read_view_request().ok_or_else(|| {
        "读不到 target/viewer-scene.json：先 `px_render --show --scene …` 推一份，或 `px_render --view --scene …`"
            .to_string()
    })?;
    let at = now_nanos();
    let request = ViewRequest {
        scene: previous.scene,
        key: previous.key,
        at,
        shot: false,
        sheet: previous.sheet,
        ask_camera: true,
        set_camera: options.place,
    };
    let text = serde_json::to_string_pretty(&request).map_err(|err| err.to_string())?;
    std::fs::write(VIEW_REQUEST, text).map_err(|err| format!("写 {VIEW_REQUEST} 失败：{err}"))?;
    let _ = std::fs::remove_file(VIEW_CAMERA);

    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        if let Ok(text) = std::fs::read_to_string(VIEW_CAMERA) {
            if let Ok(reply) = serde_json::from_str::<CameraReply>(&text) {
                if reply.at == at {
                    println!(
                        "相机 yaw {:.4}｜pitch {:.4}｜distance {:.4}｜位置 ({:.3}, {:.3}, {:.3})｜视口 {}×{}",
                        reply.yaw,
                        reply.pitch,
                        reply.distance,
                        reply.position[0],
                        reply.position[1],
                        reply.position[2],
                        reply.size[0],
                        reply.size[1],
                    );
                    // 这一行能直接粘回命令行：换个窗口也能摆到同一个视角。
                    println!("--place {:.4},{:.4},{:.4}", reply.yaw, reply.pitch, reply.distance);
                    return Ok(());
                }
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err("窗口没回话（3 s）：确认 `px_render --view` 在跑".to_string())
}

/// 窗口那一侧的起始场景：命令行给的 `--scene`，否则读 `--show` 推过来的那一份。
fn read_view_request() -> Option<ViewRequest> {
    let text = std::fs::read_to_string(VIEW_REQUEST).ok()?;
    serde_json::from_str(&text).ok()
}

fn view(options: Options) -> Result<(), String> {
    let request = match options.scene_path()? {
        Some(scene) => {
            let scene = scene.display().to_string();
            // 命令行起的窗口：键当场算（读不到清单就大声报错，别开着窗一片黑）。
            let key = art_cache::fingerprint_of(&scene)?;
            ViewRequest {
                scene,
                key,
                at: now_nanos(),
                shot: options.shot,
                sheet: options.sheet,
                ask_camera: false,
                set_camera: None,
            }
        }
        None => read_view_request().ok_or_else(|| {
            "--view 需要一个起始场景：给 --scene <SCENE.pxart>，或先用 --show 推一份".to_string()
        })?,
    };
    let scene_modified = std::fs::metadata(&request.scene)
        .and_then(|meta| meta.modified())
        .ok();
    // P31：按逻辑帧时间推进的自转**没了**。它有两个害处：① 逻辑帧率与渲染帧率不一致时
    // 每一帧的步长都不一样 ⇒ 画面一顿一顿；② 每帧内容都在变 ⇒ 同一场景连续帧不逐字节相同，
    // 截图不可复现、帧时间也没法比。场景产物里的 `spin` 现在只当**静态相位**用。
    let ready = RenderReady::new();

    let mut app = App::new();
    // 同上：槽资产源要在 AssetPlugin 之前注册，否则预览窗口里的云/大气也读不到 shader。
    app.add_plugins(slots::SlotsPlugin).add_plugins(
        DefaultPlugins
            .set(AssetPlugin {
                file_path: asset_root(),
                ..default()
            })
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: format!("px_render 预览 — {}", request.scene),
                    resolution: (1280_u32, 800_u32).into(),
                    present_mode: if options.novsync {
                        bevy::window::PresentMode::AutoNoVsync
                    } else {
                        bevy::window::PresentMode::AutoVsync
                    },
                    ..default()
                }),
                ..default()
            })
            .set(render_plugin()),
    )
    .add_plugins(material::DocMaterialPlugin)
    .add_plugins(shaders::ShaderLibraryPlugin)
    .add_plugins(passes::PassPlugin)
    .insert_resource(ClearColor(Color::srgb(0.004, 0.005, 0.010)))
    .insert_resource(PcgRoot(options.pcg_root.clone()))
    .insert_resource(Viewer {
        scene: request.scene.clone(),
        key: request.key,
        fingerprint: Some(request.key),
        scene_modified,
        request_at: request.at,
        sheet: request.sheet,
    })
    .insert_resource(Orbit {
        yaw: 0.0,
        pitch: 0.17,
        distance: 3.2,
    })
    .insert_resource(Rebuild(true))
    .insert_resource(PendingShot(request.shot.then_some(12)))
    .init_resource::<art_cache::ArtCache>()
    .insert_resource(FrameProbe(options.fps))
    .insert_resource(ShowFps(true))
    .init_resource::<InstrumentView>()
    .init_resource::<InstrumentKey>()
    .init_resource::<SceneInstruments>()
    .init_resource::<ShotStats>()
    .init_resource::<GpuLog>()
    // `report_frame_time` 会去看"有没有正在收窗口的任务"（`Active`），预览窗口没有任务但那
    // 个资源得在 —— 少了它这一条系统一跑就 panic（P32 真踩过：窗口起来了、核验三行都对了，
    // 14 s 后崩在 "Resource does not exist"）。
    .init_resource::<Active>()
    .add_plugins(FrameTimeDiagnosticsPlugin::default())
    .insert_resource(ready.clone())
    .insert_resource(bevy::render::error_handler::RenderErrorHandler(
        keep_rendering,
    ))
    .add_systems(Startup, (assert_vulkan_backend, view_startup, spawn_fps_readout).chain())
    .add_systems(
        Update,
        (
            orbit_camera,
            viewer_keys,
            poll_request,
            poll_scene,
            rebuild_scene,
            auto_shot,
            heartbeat,
            update_title,
            report_frame_time,
            update_fps_readout,
            instrument_keys,
            apply_instrument,
            check_instrument_matches,
        )
            .chain(),
    );

    println!("预览窗口已开：左键拖动转视角、滚轮缩放；场景产物一换（键变了）就重建");
    println!("  空格 自转｜f 帧率开关｜s 存图｜q 退出");
    println!("  v/m/n 切产物里那个仪器参数（云拿它当消融档：体积 / 硬表面 / 法线）—— 随时来回切，不用重开");
    println!("  推一份新场景进来：px_render --show --scene <SCENE.pxart> [--shot]");

    if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
        render_app
            .insert_resource(ready.clone())
            .add_systems(Render, watch_pipelines.in_set(bevy::render::RenderSystems::Cleanup));
    }

    app.run();
    let _ = std::fs::remove_file(VIEW_LEASE);
    Ok(())
}

fn view_startup(mut commands: Commands) {
    // 天空盒与灯都**由产物决定**：窗口起来时还没有场景，所以这里一个都不摆 ——
    // 第一份场景重建时才把天空盒与环境光按产物挂上（`rebuild_scene`）。
    commands.spawn((
        OrbitCamera,
        Camera3d::default(),
        DepthPrepass,
        Msaa::Off,
        AmbientLight {
            brightness: DEFAULT_AMBIENT,
            ..default()
        },
        Transform::from_xyz(0.0, 0.55, 3.2).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

fn orbit_camera(
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut orbit: ResMut<Orbit>,
    mut camera: Query<&mut Transform, With<OrbitCamera>>,
) {
    if buttons.pressed(MouseButton::Left) {
        orbit.yaw -= motion.delta.x * 0.006;
        orbit.pitch = (orbit.pitch - motion.delta.y * 0.006).clamp(-1.25, 1.25);
    }
    if scroll.delta.y.abs() > 0.0 {
        orbit.distance = (orbit.distance * (1.0 - scroll.delta.y * 0.08)).clamp(1.5, 14.0);
    }
    let Ok(mut transform) = camera.single_mut() else {
        return;
    };
    let rotation = Quat::from_rotation_y(orbit.yaw) * Quat::from_rotation_x(orbit.pitch);
    transform.translation = rotation * Vec3::new(0.0, 0.0, orbit.distance);
    transform.look_at(Vec3::ZERO, Vec3::Y);
}

fn viewer_keys(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut show_fps: ResMut<ShowFps>,
    readout: Query<Entity, With<FpsReadout>>,
    mut exit: MessageWriter<AppExit>,
) {
    // 内容型的快捷键（色板 / 海平面 / 位移 / 环系）随内容旗标一起没了：
    // 那些数现在只有场景产物说得算，窗口里改一个数就是第二份会漂开的默认值。
    // 空格以前切"自转"；自转按逻辑帧时间推进 ⇒ 逻辑帧率与渲染帧率一对不上就卡，
    // 而且每帧内容都在变 ⇒ 截图不可复现、帧时间不可比。功能已删（P31），键位不再占用。
    if keys.just_pressed(KeyCode::KeyF) {
        show_fps.0 = !show_fps.0;
        for entity in readout.iter() {
            commands.entity(entity).insert(if show_fps.0 {
                Visibility::Visible
            } else {
                Visibility::Hidden
            });
        }
    }
    if keys.just_pressed(KeyCode::KeyQ) || keys.just_pressed(KeyCode::Escape) {
        exit.write(AppExit::Success);
    }
}

fn poll_request(
    mut viewer: ResMut<Viewer>,
    mut rebuild: ResMut<Rebuild>,
    mut shot: ResMut<PendingShot>,
    mut orbit: ResMut<Orbit>,
    windows: Query<&Window>,
) {
    let Some(request) = read_view_request() else {
        return;
    };
    if request.at == viewer.request_at {
        return;
    }
    viewer.request_at = request.at;
    // 换场景之前先把相机摆好：`--place` 与场景无关，只是"把镜头挪过去"。
    if let Some(place) = request.set_camera {
        orbit.yaw = place[0];
        orbit.pitch = place[1];
        orbit.distance = place[2];
        println!(
            "相机被摆到 yaw {:.4}｜pitch {:.4}｜distance {:.4}",
            place[0], place[1], place[2]
        );
    }
    if request.ask_camera {
        // 位置与 `orbit_camera` 里那份算法逐字一致：不依赖"变换这一帧更新了没有"。
        let rotation = Quat::from_rotation_y(orbit.yaw) * Quat::from_rotation_x(orbit.pitch);
        let position = rotation * Vec3::new(0.0, 0.0, orbit.distance);
        let size = windows
            .iter()
            .next()
            .map(|window| {
                let size = window.physical_size();
                [size.x, size.y]
            })
            .unwrap_or([0, 0]);
        let reply = CameraReply {
            at: request.at,
            yaw: orbit.yaw,
            pitch: orbit.pitch,
            distance: orbit.distance,
            position: position.to_array(),
            size,
        };
        match serde_json::to_string_pretty(&reply) {
            Ok(text) => {
                let _ = std::fs::write(VIEW_CAMERA, text);
            }
            Err(err) => eprintln!("写相机回话失败：{err}"),
        }
    }
    // 换了场景，或者内容键变了 ⇒ 重建；同一份产物再推一次只是「看见了」，不动。
    let changed = request.scene != viewer.scene
        || request.key == 0
        || Some(request.key) != viewer.fingerprint;
    viewer.scene = request.scene;
    viewer.key = request.key;
    viewer.sheet = request.sheet;
    viewer.scene_modified = std::fs::metadata(&viewer.scene)
        .and_then(|meta| meta.modified())
        .ok();
    if changed {
        viewer.fingerprint = Some(request.key);
        rebuild.0 = true;
        println!("窗口切到：{}", describe(&viewer));
    } else {
        println!("同一份产物（键 {:016x}）再推一次 ⇒ 不重建", viewer.key);
    }
    if request.shot {
        shot.0 = Some(12);
    }
}

fn auto_shot(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    ready: Res<RenderReady>,
    mut pending: ResMut<PendingShot>,
) {
    if keys.just_pressed(KeyCode::KeyS) {
        pending.0 = Some(3);
    }
    let Some(frames) = pending.0 else {
        return;
    };
    if frames > 0 {
        pending.0 = Some(frames - 1);
        return;
    }
    if ready.get() == PIPELINES_PENDING {
        return;
    }

    pending.0 = None;
    let path = "target/viewer-shot.png";
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path));
    println!("预览截图 → {path}");
}

/// 场景产物变了吗 —— 判据是 mtime 闹钟 + **载荷指纹**（§50）。
///
/// mtime 只是「该去看一眼」的闹钟（一次 metadata 调用，便宜）；真正的判据是清单里的
/// 载荷指纹：重新烘一份内容一模一样的场景（或者只是 touch 了一下）不该让窗口重建。
/// 指纹为 0 的旧产物照样重建 —— 判不了就照旧。
fn poll_scene(mut viewer: ResMut<Viewer>, mut rebuild: ResMut<Rebuild>, mut ticks: Local<u32>) {
    *ticks += 1;
    if *ticks % 20 != 0 {
        return;
    }
    let Ok(meta) = std::fs::metadata(&viewer.scene) else {
        return;
    };
    let Ok(modified) = meta.modified() else {
        return;
    };
    if viewer.scene_modified == Some(modified) {
        return;
    }
    viewer.scene_modified = Some(modified);

    match art_cache::fingerprint_of(&viewer.scene) {
        Ok(fingerprint) if fingerprint != 0 && Some(fingerprint) == viewer.fingerprint => {
            println!(
                "场景产物动过，但载荷指纹没变（{:016x}）⇒ 不重建",
                fingerprint
            );
            return;
        }
        Ok(fingerprint) => {
            viewer.fingerprint = Some(fingerprint);
            viewer.key = fingerprint;
        }
        Err(err) => println!("场景产物动过，读不到清单：{err}"),
    }
    rebuild.0 = true;
    println!("场景产物更新，重载：{}", viewer.scene);
}

fn rebuild_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut doc_materials: ResMut<Assets<material::DocMaterial>>,
    mut cache: ResMut<art_cache::ArtCache>,
    viewer: Res<Viewer>,
    mut rebuild: ResMut<Rebuild>,
    mut instruments: ResMut<SceneInstruments>,
    mut view: ResMut<InstrumentView>,
    camera: Query<(Entity, Option<&Skybox>), With<Camera3d>>,
    parts: Query<Entity, With<ScenePart>>,
    tools: SceneTools,
) {
    if !rebuild.0 {
        return;
    }
    rebuild.0 = false;
    cache.begin();

    // 预览窗口只有一个视口：产物相机表是 `--serve` 出对照图用的，这里说明一声就好，
    // 不装作摆了 12 台相机。
    if viewer.sheet {
        println!("⚠ 预览窗口不出多视口对照图（那是 --serve 出图的事）：这一份的相机表只当参考");
    }

    // 先只读一遍产物：读不出来就留着窗口里现在这张图 ——
    // 拆完旧场景才发现产物坏了，窗口就空了。
    let path = std::path::Path::new(&viewer.scene);
    if let Err(message) = px_protocol::scene::read_scene(path).and_then(|spec| spec.check()) {
        eprintln!("⚠ 新场景读不出来，保留窗口里现在这张图：{message}");
        return;
    }

    for entity in parts.iter() {
        commands.entity(entity).despawn();
    }
    match scene::spawn_document(
        &mut cache,
        &mut commands,
        &mut meshes,
        &mut images,
        &mut doc_materials,
        &tools.assets,
        &tools.root.0,
        &viewer.scene,
    ) {
        Ok(built) => {
            // 环境只换"挂在相机上"的那一样：天空盒按新产物换（没有就摘掉）。
            // 相机的**位置与朝向是用户拖着**的，重建一根手指都不许碰 —— 窗口的轨道相机
            // 是唯一的"怎么看"状态（§64 那条方法论：漂移过的相机量出来的数全部作废）。
            let skybox = scene::skybox_of(&built);
            for (entity, _) in camera.iter() {
                match &skybox {
                    Some(wanted) => {
                        commands.entity(entity).insert(wanted.clone());
                    }
                    None => {
                        commands.entity(entity).remove::<Skybox>();
                    }
                }
            }
            instruments.0 = built.instruments.clone();
            view.0 = None;
            println!("{}", built.label);
            println!("{}", cache.sweep());
        }
        Err(message) => eprintln!("重建场景失败：{message}"),
    }
}

fn heartbeat(mut ticks: Local<u32>) {
    *ticks += 1;
    if *ticks % 30 != 0 {
        return;
    }
    let _ = std::fs::write(VIEW_LEASE, format!("{}", now_nanos()));
}

fn update_title(viewer: Res<Viewer>, mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    let wanted = format!("px_render 预览 — {}", describe(&viewer));
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    if window.title != wanted {
        window.title = wanted;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use px_protocol::scene::SceneSpec;

    /// **文档引用的成员一个都不能少**：几何、shader 都在 `members()` 里 ——
    /// 少一个就是"场景要的东西没人去取"（缓存与 diff 也按这张表走）。
    #[test]
    fn a_document_declares_every_member_it_needs() {
        let member = |node: &str| px_protocol::Member::new("generated", node, &"a".repeat(64));
        let document = SceneSpec {
            schema: px_protocol::SCENE_SCHEMA,
            name: "夹具".to_string(),
            environment: px_protocol::Environment {
                ambient: 80.0,
                skybox: Some(member("stars")),
                skybox_brightness: 900.0,
            },
            cameras: Vec::new(),
            expects: vec!["clouds".to_string()],
            resources: Vec::new(),
            passes: Vec::new(),
            lights: vec![px_protocol::Light::point("sun", [1.0, 2.0, 3.0], [1.0; 3], 1.0)],
            // 帧自有材质（§135）：Bevy 宿主**不消费**这一节（它是锚，吃的是冻件），
            // 这里留空只为把结构体补齐。
            frame_materials: Vec::new(),
            objects: vec![px_protocol::Object {
                id: "云".to_string(),
                geometry: px_protocol::Geometry::mesh(member("shell")),
                material: px_protocol::Material::new(member("clouds"))
                    .with_texture("coverage", px_protocol::TextureRef::new(5, member("coverage"), Default::default())),
                transform: px_protocol::Transform::default(),
                cast_shadow: false,
            }],
        };
        document.check().expect("这份文档是成形的");
        let names: Vec<String> = document
            .members()
            .iter()
            .map(|entry| entry.node.clone())
            .collect();
        assert_eq!(names, vec!["shell", "clouds", "coverage", "stars"]);
        assert!(document.expects.iter().any(|tag| tag == "clouds"));
    }

    /// 失败明细是「**当前这一批**失败」的读数：置上去就带着原因，同一轮里后来的不覆盖
    /// （根因第一条），管线重新编过（`clear_failure`）或重建（`invalidate`）才清。
    /// `accept_jobs` 与 `pipeline_gate` 靠它把"拒绝出图"的原因说清楚 —— 而不是只留一句
    /// 「细节见服务端日志」。
    #[test]
    fn a_proved_failure_keeps_its_first_reason_until_the_pipelines_recover() {
        let ready = RenderReady::new();
        assert!(ready.failure().is_none());
        assert_eq!(ready.get(), PIPELINES_PENDING);

        ready.fail("管线 A｜少了分号".to_string());
        assert_eq!(ready.get(), PIPELINES_FAILED);
        assert!(ready.failure().unwrap().contains("管线 A"));

        ready.fail("管线 B｜另一条也坏了".to_string());
        assert_eq!(
            ready.failure().unwrap(),
            "管线 A｜少了分号",
            "根因只认第一条，后来的不覆盖"
        );

        ready.clear_failure();
        assert!(ready.failure().is_none(), "管线编回来了：明细跟着走");
        assert_eq!(
            ready.get(),
            PIPELINES_FAILED,
            "状态由 watch_pipelines 接着算"
        );

        ready.fail("管线 C｜又坏了".to_string());
        assert!(ready.failure().unwrap().contains("管线 C"));

        ready.invalidate();
        assert_eq!(ready.get(), PIPELINES_PENDING);
        assert!(ready.failure().is_none());
    }

    /// 构建期插入的资源**第一次 run 也算 changed**（`FunctionSystem::initialize` 把 `last_run`
    /// 摆到相对 `Tick::MAX` 的位置）—— 窗口「一开就换档」正是踩了这个。
    /// 产物里那一格是什么，没人按键时一个字节都不许动；真按了键才临时覆盖。
    #[test]
    fn the_instrument_only_overrides_after_a_key_request() {
        use bevy::ecs::system::RunSystemOnce;

        let mut world = World::new();
        world.insert_resource(Assets::<material::DocMaterial>::default());
        world.insert_resource(InstrumentView::default());
        world.init_resource::<InstrumentKey>();
        world.init_resource::<SceneInstruments>();

        let handle = {
            let mut materials = world.resource_mut::<Assets<material::DocMaterial>>();
            let mut params = vec![0_u8; 32];
            // 产物给的那一档：硬表面（5）。
            params[16..20].copy_from_slice(&INSTRUMENT_SURFACE.to_le_bytes());
            materials.add(material::DocMaterial {
                params,
                textures: Vec::new(),
                alpha: AlphaMode::Opaque,
                cull: px_protocol::scene::CullMode::Back,
                depth_bias: 0.0,
                shader: 0,
            })
        };
        world.resource_mut::<SceneInstruments>().0 = vec![scene::Instrument {
            material: handle.clone(),
            param: INSTRUMENT_PARAM.to_string(),
            offset: 16,
        }];

        let code = |world: &World, handle: &Handle<material::DocMaterial>| {
            let material = world
                .resource::<Assets<material::DocMaterial>>()
                .get(handle)
                .unwrap();
            u32::from_le_bytes([
                material.params[16],
                material.params[17],
                material.params[18],
                material.params[19],
            ])
        };

        world.run_system_once(apply_instrument).unwrap();
        assert_eq!(
            code(&world, &handle),
            INSTRUMENT_SURFACE,
            "没有按键请求就不该动产物给的材质"
        );

        world.resource_mut::<InstrumentKey>().0 = Some(INSTRUMENT_NORMALS);
        world.run_system_once(apply_instrument).unwrap();
        assert_eq!(code(&world, &handle), INSTRUMENT_NORMALS);
        assert_eq!(world.resource::<InstrumentView>().0, Some(INSTRUMENT_NORMALS));
    }
}










































