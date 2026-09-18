//! 预览窗口（S7 前半）：**常驻的 winit 窗口** + orbit 相机 + 鼠标输入。
//!
//! 语义与 Bevy 宿主（`px_render/src/main.rs`：`view` :3204、`show` :3120、
//! `viewer_camera` :3152）**逐字对齐**：窗口是**常驻**的，场景由**别人推**进来
//! （`--show` 写 `target/viewer-scene.json`），相机的方位可以**问回来**（`--where`）
//! 也可以**摆过去**（`--place`）。三个文件名原样照抄 ——
//! `target/viewer-scene.json` / `target/viewer.json` / `target/viewer-camera.json`：
//! 那两个宿主之间唯一的约定就是这三个文件，换一个字节，"谁在线、它现在看着哪儿"
//! 就变成一条要靠猜的事。
//!
//! ## 三处与 Bevy 宿主的形状差别（都不是"简化"，各有各的理由）
//!
//! 1. **按需渲染**：裸 wgpu 建管线是同步的（§104 第 5 条），一次 `render::run` 就把一整帧
//!    从头画到尾。所以窗口里**没有**"逐帧推进"那套东西（Bevy 有，因为它的管线与资产要跨帧等）：
//!    相机一动、场景一换、窗口一改大小，才画一帧；没变的时候只是把上一张重新呈一次。
//! 2. **屏幕上那些字节 = 判据那张图的字节**：画面画进**本进程自己的** `Rgba8UnormSrgb`
//!    （`shot::Target`，§104 第 13 条：交换链不许读），回读出来的那批字节
//!    ① 原样传到屏幕（[`Present`] 那一段）② `--shot` 时原样走 `shot::write_png`。
//!    ⇒ "窗口显示的是不是我们证明过的那张图"这个问题**只有一个数据来源**，
//!    没有一个"另画一遍给屏幕看"的第二条路。
//! 3. **角度的含义由 `camera::probe_camera` 定**：窗口的 `(yaw, pitch, distance)` 与
//!    `--cam` 是**同一套数、同一个函数**，所以"窗口在某个方位看到的那张图"与
//!    "离线在同一个方位画的那张图"必须逐字节相同 —— 那就是本单元的判据。
//!    ⚠ Bevy 的窗口把 `Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch)` 乘到
//!    `(0,0,distance)` 上，它的 **pitch 正方向与它自己的 `--cam`（`camera_for`）相反**；
//!    我们不让同一个三元组有两套含义（§104 第 1 条那一族：两处会漂开的真相）。
//!
//! ## 这一版**没有**的（下一单元：shader 热重载）
//!
//! `.wgsl` 改存盘之后画面 1 秒内变 —— 那是 S7 的后半。这里只留下**接口**：
//! [`Viewer::poll_scene_file`] 已经按"mtime 闹钟 + 载荷指纹"的规矩在看产物文件，
//! 热重载接上来时只需要在同一处多问一句"shader 闭包变了吗"。
//!
//! ⚠ 起窗口必须**脱离**（`Start-Process` 不带 `-Wait`）：窗口进程会一直占着事件循环。
//! 而且 `Start-Process` 继承的是**宿主 PowerShell 的进程 cwd**（`Set-Location` 改不了它）⇒
//! 三个相对路径与 CAS 根都会落在错的目录里，`-WorkingDirectory` 一个都不能省。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use crate::gpu::Gpu;
use crate::render::{self, Views};
use crate::shot;

// ---------------------------------------------------------------------------
// 三个文件（名字与 Bevy 宿主逐字相同）
// ---------------------------------------------------------------------------

/// `--show` 推场景进来的落点（也是 `--where` / `--place` 的载体）。
pub const VIEW_REQUEST: &str = "target/viewer-scene.json";
/// 窗口的心跳（活着 = 这个文件的时间戳是新的）。
pub const VIEW_LEASE: &str = "target/viewer.json";
/// 窗口对"相机在哪儿"的回话。
pub const VIEW_CAMERA: &str = "target/viewer-camera.json";
/// 请求没给截图路径时落在哪儿。**不是我们发明的缺省**：Bevy 宿主的 `auto_shot` 写死的
/// 就是这一条（`px_render/src/main.rs:3487`）—— 照抄它是为了让"没给路径"那一档
/// 两个宿主指的是同一个文件（§104 第 3 条：缺省值也是一处会漂的真相）。
pub const VIEW_SHOT: &str = "target/viewer-shot.png";

/// 看请求文件的间隔。Bevy 那边每个逻辑帧看一次（60 Hz）；这里是**事件驱动**的窗口，
/// 只有这个周期会让它醒过来 —— 200 ms 是"人按了 `--show` 之后感觉是即时的"那一档。
const POLL_INTERVAL: Duration = Duration::from_millis(200);
/// 心跳的间隔（租约写一次）。Bevy 是每 30 帧（约 0.5 s）。
const HEARTBEAT: Duration = Duration::from_secs(1);
/// 租约自查的间隔（与 `serve::LEASE_CHECK_INTERVAL` 同一个数：2 s）。
const LEASE_CHECK_INTERVAL: Duration = Duration::from_secs(2);
/// 多久算"租约还新鲜"。**与 Bevy 的 `show()` 同一个数（5 s）**：
/// 那边的 `lease_age() < 5 s` 就是"窗口在线"的判据，这里把它同时用作单例闸。
const LEASE_FRESH: Duration = Duration::from_secs(5);
/// `--where` / `--place` 等回话的上限（Bevy 是 3 s）。
const REPLY_TIMEOUT: Duration = Duration::from_secs(3);

/// 鼠标灵敏度与轨道范围：**数值逐字照抄 Bevy 的 `orbit_camera`**（:3345-3365）。
/// 抄而不是"调一个更顺手"的：惯性是肌肉记忆，而这一版没有任何东西需要它不同。
///
/// ⚠⚠ **单位**：Bevy 那个 `Orbit` 资源整体是**弧度**，所以 `0.006` 与 `1.25` 都是弧度。
/// 我们这一侧的状态是**度** —— `--cam` / `--place` / `--where` 报的都是度，
/// 弧度只在 `camera::probe_camera` 内部出现一次（`to_radians()`）。
/// ⇒ 这两个常数**必须在用之前换成度**，不能拿弧度直接加到度上：
/// 那样"拖一下"会变成 0.006°（人眼看不见），而 pitch 的夹取会变成 ±1.25°（一碰就到底）。
/// 同一个三元组在窗口、`--cam`、`--place` 三处必须是**同一套单位**，否则"窗口看到的"
/// 与"离线画的"就不是同一台相机（本单元的判据正是那件事）。
const YAW_PER_PIXEL: f64 = 0.006;
const PITCH_PER_PIXEL: f64 = 0.006;
const ZOOM_PER_NOTCH: f64 = 0.08;
/// Bevy 的 `orbit.pitch.clamp(-1.25, 1.25)` —— **弧度**。
const PITCH_LIMIT_RADIANS: f64 = 1.25;
const DISTANCE_MIN: f32 = 1.5;
const DISTANCE_MAX: f32 = 14.0;

/// `probe_camera` 自己会把 pitch 夹到 ±89.5°（`camera.rs:48`）。窗口这一侧提前夹到**同一个数**：
/// 状态里存着一个相机根本不会用的角度，`--where` 就会报一个与画面不符的数（§146.3 ③）。
const PITCH_DEGREES_LIMIT: f32 = 89.5;

/// 鼠标拖一下 → 角度变多少（**度**）。见上面那段单位说明。
fn drag_degrees(per_pixel: f64, pixels: f64) -> f32 {
    (pixels * per_pixel).to_degrees() as f32
}

/// Bevy 那条夹取（±1.25 弧度）换成度。
fn pitch_limit_degrees() -> f32 {
    PITCH_LIMIT_RADIANS.to_degrees() as f32
}

// ---------------------------------------------------------------------------
// 请求 / 回话（schema 与 Bevy 逐字相同，只多两格**可缺省**的）
// ---------------------------------------------------------------------------

/// 推给常驻窗口的东西：**哪一份场景产物** + 它的内容键。内容本身一个字都不进来 ——
/// 窗口自己去 CAS 取同一份产物（照抄 Bevy 的 `ViewRequest`）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ViewRequest {
    scene: String,
    /// 场景产物的内容键（清单里的载荷指纹）。窗口只在它变了才重建。
    #[serde(default)]
    key: u64,
    at: u64,
    #[serde(default)]
    shot: bool,
    /// 窗口不开对照图（多视口是 `--serve` 出图的事），只用来打一行说明。
    #[serde(default)]
    sheet: bool,
    /// 问一句"相机在哪儿"：窗口把方位写进 `VIEW_CAMERA`。
    /// ⚠ 与 `shot` 一样是**附带动作**：`scene` / `key` 沿用上一次请求那份，不为问一句话换场景。
    #[serde(default)]
    ask_camera: bool,
    /// 顺手把相机摆到这个方位（`yaw,pitch,distance`）。`None` = 不动。
    #[serde(default)]
    set_camera: Option<[f32; 3]>,
    /// **我们多的那一格**：截图存哪儿。
    ///
    /// ⚠ Bevy 把它写死成 `target/viewer-shot.png`（`auto_shot`），所以那一格在文件里
    /// **没有对应的字段**；而"图存哪儿"是调用方写的一句话（策略），不是可以替它猜的东西。
    /// 加成**可缺省**的一格之后两个方向都不炸：Bevy 写的请求（没有这一格）我们照收
    /// （缺省 = 它写死的那条路径），我们写的请求它也能解（serde 默认忽略不认识的多余字段）。
    #[serde(default)]
    shot_path: Option<String>,
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
    /// **我们多的那一格**（可缺省）：这三个角是**从探针位姿反推**出来的，
    /// 不是窗口手里那份状态 —— 窗口还没被鼠标或 `--place` 接管时它手里根本没有角。
    /// 少了这一格，"`--place` 它给出来的三个数"与"现在这台相机"之间的关系就要靠猜。
    #[serde(default)]
    derived: bool,
}

fn now_nanos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos() as u64)
        .unwrap_or(0)
}

/// 租约有多旧。`None` = 没有租约文件（没有窗口在跑，或者它收工了）。
fn lease_age() -> Option<Duration> {
    let stamp: u64 = std::fs::read_to_string(VIEW_LEASE).ok()?.trim().parse().ok()?;
    Some(Duration::from_nanos(now_nanos().saturating_sub(stamp)))
}

fn write_request(request: &ViewRequest) -> Result<(), String> {
    let text = serde_json::to_string_pretty(request).map_err(|err| err.to_string())?;
    std::fs::write(VIEW_REQUEST, text).map_err(|err| format!("写 {VIEW_REQUEST} 失败：{err}"))
}

fn read_request() -> Option<ViewRequest> {
    let text = std::fs::read_to_string(VIEW_REQUEST).ok()?;
    serde_json::from_str(&text).ok()
}

/// 场景产物的**载荷指纹**（清单里那一格）。与 `px_render::art_cache::fingerprint_of`
/// 同一个口径、同一份协议（`px_protocol::art::read_manifest`）—— 两处算法不同的那天，
/// "同一份产物"会被判成两份。
fn fingerprint_of(path: &Path) -> Result<u64, String> {
    let bundle = px_protocol::art::read_manifest(path)?;
    Ok(bundle.assets.first().map(|asset| asset.fingerprint).unwrap_or(0))
}

/// 把 `--cam` / `--place` 那三个数收成窗口能用的状态：pitch 夹到**相机自己**的范围
/// （见 [`PITCH_DEGREES_LIMIT`]），并拒掉非有限的数（NaN 的 yaw 会让 `--where` 回一串 NaN，
/// 而"回了一串 NaN"看着像窗口坏了，其实是调用方给错了）。
///
/// ⚠ 拒词里带上**是哪一个开关**（照 `main.rs::parse_cam` 那条规矩）：`--view --cam` 与
/// `--place` 用的是同一套数，报错时指向另一个开关会把人引到错的地方（§146.3 ③）。
fn orbit_of(flag: &str, place: [f32; 3]) -> Result<[f32; 3], String> {
    if !place.iter().all(|value| value.is_finite()) {
        return Err(format!(
            "{flag} 要三个**有限的**数（yaw,pitch,distance），收到 {place:?}"
        ));
    }
    Ok([
        place[0],
        place[1].clamp(-PITCH_DEGREES_LIMIT, PITCH_DEGREES_LIMIT),
        place[2],
    ])
}

// ---------------------------------------------------------------------------
// 客户端那两条：`--show` 推场景；`--where` / `--place` 一问一答
// ---------------------------------------------------------------------------

/// `--show --scene 文档 [--shot PNG]`：把一份场景推给在跑的窗口。
///
/// ⚠ 它**不渲染任何东西**（本进程连设备都不建）：推完就回话。
/// "窗口在不在"由**心跳的新鲜度**判（`VIEW_LEASE`），而判不出来的那一天
/// 说的是"没检测到在跑的窗口"，不是"推失败"（§146.3 ③：拦住了不等于说对了）。
pub fn show(scene: &Path, shot: Option<PathBuf>) -> Result<(), String> {
    let scene = scene.display().to_string();
    let key = fingerprint_of(Path::new(&scene))?;
    let request = ViewRequest {
        scene: scene.clone(),
        key,
        at: now_nanos(),
        shot: shot.is_some(),
        sheet: false,
        ask_camera: false,
        set_camera: None,
        shot_path: shot.map(|path| path.display().to_string()),
    };
    write_request(&request)?;

    println!("已推给常驻窗口：{scene}（键 {key:016x}）");
    match lease_age() {
        Some(age) if age < LEASE_FRESH => println!("窗口在线（心跳 {:.1} s 前）", age.as_secs_f32()),
        _ => println!("⚠ 没检测到在跑的窗口；先执行 `px_render_wgpu --view --scene …` 开一个，它会一直留着"),
    }
    if request.shot {
        println!(
            "截图由**窗口**那边写（它每 {} ms 看一次请求文件）：{}",
            POLL_INTERVAL.as_millis(),
            request.shot_path.as_deref().unwrap_or(VIEW_SHOT)
        );
    }
    Ok(())
}

/// `--where` / `--place`：**只问/只摆窗口的相机**，不换场景、不重烘。
///
/// 为什么需要这条 API：窗口的轨道相机只有鼠标能改，而"某个视角对不对"依赖那个方位 ——
/// 量的时候必须能把当时的方位**读回来**（写进命令行的 `--place`），才算有了确定性复现。
/// ⚠ 两个开关**都**等回话（照抄 Bevy：`place` 也带 `ask_camera`）：
/// "摆到哪儿了"这句话要由**窗口**说，不能由摆的人自己复述一遍（那是同一个数的两次序列化）。
pub fn camera_query(place: Option<[f32; 3]>) -> Result<(), String> {
    let place = place.map(|place| orbit_of("--place", place)).transpose()?;
    let previous = read_request().ok_or_else(|| {
        format!(
            "读不到 {VIEW_REQUEST}：先 `px_render_wgpu --view --scene <SCENE.pxart>` 开一个窗口，\
             或 `--show --scene <SCENE.pxart>` 推一份"
        )
    })?;
    let at = now_nanos();
    let request = ViewRequest {
        scene: previous.scene,
        key: previous.key,
        at,
        shot: false,
        sheet: previous.sheet,
        ask_camera: true,
        set_camera: place,
        shot_path: None,
    };
    write_request(&request)?;
    // 先删回话再等：上一次的回话留着的话，`at` 一对不上就会白等满 3 s。
    let _ = std::fs::remove_file(VIEW_CAMERA);

    let deadline = Instant::now() + REPLY_TIMEOUT;
    while Instant::now() < deadline {
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
                    if reply.derived {
                        println!(
                            "⚠ 这三个角是**从探针位姿反推**的（窗口还没被鼠标或 --place 接管）：\
                             位置那一栏是真的，`--place` 它得到的是**轨道**那一档"
                        );
                    }
                    // 这一行能直接粘回命令行：换个窗口也能摆到同一个视角。
                    println!("--place {:.4},{:.4},{:.4}", reply.yaw, reply.pitch, reply.distance);
                    return Ok(());
                }
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Err(format!(
        "窗口没回话（{} s）：确认 `px_render_wgpu --view` 在跑（租约 {}）",
        REPLY_TIMEOUT.as_secs(),
        VIEW_LEASE
    ))
}

// ---------------------------------------------------------------------------
// 起窗口
// ---------------------------------------------------------------------------

/// `--view [--scene 文档] [--cam …] [--shot PNG] [--width W] [--height H]`。
///
/// 这条函数**不返回**直到窗口关掉（winit 的事件循环在里面）。
pub fn view(options: &crate::Options) -> Result<(), String> {
    let request = match options.shots.first() {
        Some(shot) => {
            let scene = shot.scene.display().to_string();
            // 命令行起的窗口：键当场算（读不到清单就大声报错，别开着窗一片黑）。
            let key = fingerprint_of(Path::new(&scene))?;
            ViewRequest {
                scene,
                key,
                at: now_nanos(),
                shot: options.shot.is_some(),
                sheet: options.sheet,
                ask_camera: false,
                set_camera: None,
                shot_path: options
                    .shot
                    .as_ref()
                    .map(|path| path.display().to_string()),
            }
        }
        None => read_request().ok_or_else(|| {
            format!(
                "--view 需要一个起始场景：给 --scene <SCENE.pxart>，或先用 --show 推一份（{VIEW_REQUEST}）"
            )
        })?,
    };

    // ---- 单例闸：一个窗口 ----
    //
    // §147 那条纪律（租约 = 单例）在窗口这一侧也要成立：两个窗口互相覆盖心跳，
    // 于是"谁在线"没人说得清，而 `--show` / `--where` 会随缘落到其中一个上。
    // 判据是**心跳新鲜度**，不是文件在不在：被 kill 掉的窗口会留下一份旧租约，
    // 那一份必须让路（否则下一次合法启动会被自己的残骸挡住）。
    if let Some(age) = lease_age() {
        if age < LEASE_FRESH {
            return Err(format!(
                "已经有一个预览窗口在跑（{} 的心跳 {:.1} s 前，上限 {} s）：\
                 先关掉它（q / Esc / 关窗口），或删掉那个文件再起",
                VIEW_LEASE,
                age.as_secs_f32(),
                LEASE_FRESH.as_secs()
            ));
        }
    }

    // 三处相对路径的基准是**当前目录**，而 `Start-Process` 继承的是宿主 PowerShell 的
    // **进程** cwd（`Set-Location` 改不了它）—— 所以把基准打出来，错了当场看得见。
    println!(
        "预览窗口：当前目录 {}｜场景 {}｜请求文件 {VIEW_REQUEST}｜租约 {VIEW_LEASE}",
        std::env::current_dir()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| "（拿不到）".to_string()),
        request.scene,
    );

    let orbit = match options.view_cam() {
        Some(place) => Some(orbit_of("--cam", place)?),
        // `None` = **不给 --cam 那一档**（探针机位，`camera.rs::probe_camera`）。
        // 不发明一个"窗口的初始角度"：那正是 §104 第 3 条说的那种没人写过的数。
        None => None,
    };

    let mut viewer = Viewer {
        scene: request.scene.clone(),
        key: request.key,
        request_at: request.at,
        orbit,
        pending_shot: initial_shot(&request),
        sheet: request.sheet,
        size: (options.width, options.height),
        pcg_root: options.pcg_root.clone(),
        novsync: options.novsync,
        window: None,
        gpu: None,
        surface: None,
        config: None,
        present: None,
        dragging: false,
        cursor: None,
        dirty: true,
        last: None,
        scene_modified: std::fs::metadata(&request.scene)
            .and_then(|meta| meta.modified())
            .ok(),
        first_frame: true,
        frames: 0,
        last_poll: Instant::now(),
        last_heartbeat: Instant::now(),
    };

    // 请求文件与租约都**在窗口起来之后**才写（见 `Viewer::open`）：
    // 它们说的是"窗口现在显示着什么、它还活着"，而窗口还没起来时这两句话都不成立。
    // ⚠ 反过来说：`--where` / `--place` 因此要等窗口真的起来才有东西可读 ——
    //    那是实话（"窗口没起来"与"窗口起来了但没回话"是两件事），不是缺陷。

    println!("操作：左键拖动 = 转视角、滚轮 = 缩放；s = 存一张图（落到 --shot 那条路径）；q / Esc = 退出");
    println!("  推一份新场景进来：px_render_wgpu --show --scene <SCENE.pxart> [--shot PNG]");
    println!("  问/摆相机：px_render_wgpu --where ｜ px_render_wgpu --place yaw,pitch,distance");
    println!("  截图落点：{}（--shot 不给路径时）", VIEW_SHOT);
    if options.fps {
        println!(
            "（--fps 收下但不生效：本窗口是**按需渲染**的（相机/场景一变才画一帧），\
             没有逐帧的帧率可报 —— 帧率读数属于把渲染做成逐帧的那一档）"
        );
    }

    let event_loop = EventLoop::new().map_err(|err| format!("起事件循环失败：{err}"))?;
    // 租约 = 生命周期（§147 那条纪律的窗口版）：**删掉租约 ⇒ 窗口自己退出**。
    // 服务端早就有这一条（`serve::spawn_lease_watch`，"删掉租约 ⇒ 4 秒内自查退出"）；
    // 窗口这边少了它，"把窗口关掉"就只剩鼠标一条路，于是自动化（包括本单元的验收脚本）
    // 只能 `/F` 硬杀 —— 而硬杀留下的正是一份**陈租约**（那正是要避免的东西）。
    spawn_lease_watch();
    event_loop
        .run_app(&mut viewer)
        .map_err(|err| format!("事件循环退出：{err}"))?;
    Ok(())
}

/// 租约没了 ⇒ 自己退出。**另一个线程**（照搬服务端那条的理由）：主线程会**阻塞**在
/// 一帧渲染里（实测 0.8–2.3 s），塞进事件循环的话"删掉租约"要等这一帧画完才生效。
///
/// ⚠ 连续**两个**周期都读不到才退（约 4–6 s），不是一次：心跳是 `fs::write`（不是原子替换），
/// 读的人有可能正好撞见"文件被截断成 0 字节"的那一瞬间 —— 一次就退的话，那是一次**静默自杀**。
/// 服务端那条是一锤子（它自己写自己读，窗口比它多一个风险面：写的人与读的人可能同时在不同核上）。
fn spawn_lease_watch() {
    std::thread::spawn(move || {
        let mut missing = 0;
        loop {
            std::thread::sleep(LEASE_CHECK_INTERVAL);
            if std::fs::read_to_string(VIEW_LEASE).is_ok() {
                missing = 0;
                continue;
            }
            missing += 1;
            if missing >= 2 {
                println!("租约没了（{VIEW_LEASE}），预览窗口退出");
                std::process::exit(0);
            }
        }
    });
}

/// 起始那一份请求要不要截图。`--view --shot P` 与 `--show --shot P` 走的是同一格。
fn initial_shot(request: &ViewRequest) -> Option<PathBuf> {
    if !request.shot {
        return None;
    }
    Some(PathBuf::from(
        request.shot_path.as_deref().unwrap_or(VIEW_SHOT),
    ))
}

/// 写一份新租约（**无条件**）。只在窗口刚起来那一次用。
fn write_lease() {
    let _ = std::fs::write(VIEW_LEASE, format!("{}", now_nanos()));
}

/// 心跳：**只在租约还在的时候刷新它**。
///
/// ⚠ 这一条不是"顺手防一下"，它让"删掉租约"变成一个**能用的操作**：
/// 服务端那条纪律是"删掉租约 ⇒ 4 秒内自查退出"（§104 第 10 条），而服务端那份租约
/// **没有心跳** —— 写完就不动了，所以删掉就是删掉了。窗口这份租约按 Bevy 的口径是
/// **每秒刷新**的，于是"删掉它"会被下一次心跳**原地复活**：外部就再没有任何办法
/// 把它请出场（只能 `/F` 硬杀，而硬杀留下的正是一份**陈租约** —— 那正是要避免的东西）。
///
/// ⇒ 缺了就不再续：租约的**存在**是"这个窗口还在"的唯一真本，谁都不许替它续命。
/// 于是"删掉 `target/viewer.json`"与"关窗口"是同一件事（退出由 [`spawn_lease_watch`] 做）。
fn heartbeat_now() {
    if !std::path::Path::new(VIEW_LEASE).exists() {
        return;
    }
    write_lease();
}

// ---------------------------------------------------------------------------
// 窗口本体
// ---------------------------------------------------------------------------

/// 一次渲染得到的、**也**要送到屏幕上的那批字节。
///
/// ⚠ 它就是 `render::run` 回读出来的 `pixels`，一个字节都不重新采样：
/// 屏幕上那一张与 `--shot` 写出来的那一张因此是**同一批字节**（模块头第 2 条）。
struct Present {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    bind: Option<wgpu::BindGroup>,
    /// 显示用的那张纹理（`Rgba8UnormSrgb` + **非 sRGB 视图**，见 `upload`）。
    texture: Option<wgpu::Texture>,
    size: (u32, u32),
}

impl Present {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Present {
        // ⚠ 这一份 WGSL 用 wgpu 的**缺省**编译档（`checked()`）—— 与内容 shader 那条路
        //    故意不同（`px_pass::module_of_wgsl` 抄的是 Bevy 的 `unchecked()`，§145）。
        //    理由：它的输出**永远只到交换链**，不进任何判据（连 `--shot` 都不经过它）。
        //    判据那条路上一个字节都不许走"另一个编译档"，也不该让这一份去共享那个档位。
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("px_render_wgpu 呈现"),
            source: wgpu::ShaderSource::Wgsl(PRESENT_WGSL.into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("px_render_wgpu 呈现"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("px_render_wgpu 呈现"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("px_render_wgpu 呈现"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        Present {
            layout,
            pipeline,
            // 1:1 的取样：放大缩小时宁可看到方块，也不要一个"看起来更顺眼"的重采样 ——
            // 那是屏幕上**另外**画了一遍，而这一格的全部意义是"原样"。
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("px_render_wgpu 呈现"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Nearest,
                min_filter: wgpu::FilterMode::Nearest,
                mipmap_filter: wgpu::MipmapFilterMode::Nearest,
                ..Default::default()
            }),
            bind: None,
            texture: None,
            size: (0, 0),
        }
    }

    /// 把回读出来的紧凑 RGBA8 原样搬到一张纹理上。
    ///
    /// ⚠ 两端都用**非 sRGB 的视图**：判据那张图存的是**已经 sRGB 编码过的字节**
    /// （`Rgba8UnormSrgb`，`shot::FORMAT`），而交换链那一张也是 sRGB 格式。
    /// 拿 sRGB 视图采样、再写进 sRGB 目标 ⇒ 硬件会做一次"解码再编码"，
    /// 而 8 位下那条往返**不是**恒等（有几个值差 1）。非 sRGB 视图把两端都变成
    /// "字节进、字节出"，于是屏幕上的像素与 PNG 里的字节**逐个相等**。
    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32, rgba: &[u8]) {
        if self.texture.is_none() || self.size != (width, height) {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("px_render_wgpu 呈现源"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: shot::FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                // sRGB 的格式 + 非 sRGB 的视图：**只有 sRGB 那一格可以这样换**（wgpu 的规矩）。
                view_formats: &[shot::FORMAT.remove_srgb_suffix()],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("px_render_wgpu 呈现源（非 sRGB）"),
                format: Some(shot::FORMAT.remove_srgb_suffix()),
                ..Default::default()
            });
            self.bind = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("px_render_wgpu 呈现源"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            }));
            self.texture = Some(texture);
            self.size = (width, height);
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                // 上面刚保证过 `Some`（`texture.is_none()` 那一支已经建了一张）。
                texture: self.texture.as_ref().expect("刚建过"),
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }

    /// 一个全屏三角，把那张纹理盖到交换链上。
    fn draw(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("px_render_wgpu 呈现"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let Some(bind) = &self.bind else {
            // 还没画过任何一帧（窗口刚开、第一次 `render::run` 就失败了）：黑屏，
            // 而不是"拿一张没建好的绑定组去画"。
            return;
        };
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// 全屏三角 + 一次取样。
///
/// ⚠ `uv.y = 1 − corner.y`：NDC 的 y 朝上，而纹理的第 0 行是**图的第一行**（顶行）。
/// 少了这个 `1 − `，屏幕上的图上下颠倒 —— 而"窗口能看"这条判据在人眼里**依然成立**，
/// 只是上下反了（这正是那种不会被任何门抓到的错）。照抄 `px_pass` 里同一个全屏三角。
const PRESENT_WGSL: &str = r#"
struct PxPresentOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@group(0) @binding(0) var px_source: texture_2d<f32>;
@group(0) @binding(1) var px_sampler: sampler;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> PxPresentOut {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: PxPresentOut;
    out.uv = vec2<f32>(corner.x, 1.0 - corner.y);
    out.position = vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
    return out;
}

@fragment
fn fs_main(in: PxPresentOut) -> @location(0) vec4<f32> {
    return textureSample(px_source, px_sampler, in.uv);
}
"#;

/// 窗口的全部状态。事件循环里**只有**它。
struct Viewer {
    /// 现在显示的是哪一份产物（请求文件里那一栏的真本就在窗口手里）。
    scene: String,
    key: u64,
    /// 已经吃过的那一次请求（`ViewRequest.at`）。同一个 `at` 只处理一次。
    request_at: u64,
    /// 轨道相机：`None` = 不给 `--cam` 那一档（探针机位）。
    orbit: Option<[f32; 3]>,
    /// 还要存一张图（存完就清）。
    pending_shot: Option<PathBuf>,
    /// 这一份产物声明了对照图（只用来打一行说明：窗口不出多视口）。
    sheet: bool,
    /// 窗口的初始尺寸（逻辑像素；`--width/--height`）。
    size: (u32, u32),
    pcg_root: PathBuf,
    novsync: bool,

    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    surface: Option<wgpu::Surface<'static>>,
    config: Option<wgpu::SurfaceConfiguration>,
    present: Option<Present>,

    dragging: bool,
    cursor: Option<(f64, f64)>,
    /// 画面脏了：下一次 `RedrawRequested` 要重画一帧。
    dirty: bool,
    /// 上一次画出来的尺寸（`--shot` 与"重呈一次"都要用它）。
    last: Option<(u32, u32)>,
    /// 这一份产物的审计行只在**第一次**画它的时候打：相机一动就打一遍会把日志淹掉。
    first_frame: bool,
    frames: u64,
    /// 产物文件的 mtime（`poll_scene_file` 的闹钟：§50 那条"闹钟 + 载荷指纹"）。
    scene_modified: Option<SystemTime>,
    last_poll: Instant,
    last_heartbeat: Instant,
}

impl Viewer {
    /// 当前这台的"怎么看"：`Views::Single` 那一档（与离线那条路**同一个**入口）。
    fn views(&self) -> Views {
        Views::Single(self.orbit)
    }

    /// 现在这台相机的姿态。⚠ 用**同一个** `probe_camera` 算：`--where` 报的位置
    /// 与画面上那台相机因此来自同一份算术（两处各算一遍 = 两处会漂开的真相）。
    fn camera(&self, width: u32, height: u32) -> crate::camera::Camera {
        crate::camera::probe_camera(self.orbit, width as f32 / height as f32)
    }

    /// 按当前状态把窗口补出来：窗口 → 实例 → surface → 设备 → 呈现管线 → 租约。
    ///
    /// ⚠ 次序是硬的：`Surface` 属于**建它的那个实例**，所以"实例"必须自己建、
    /// 再把它连同 surface 一起交给 `gpu::connect_with`。少了这一步，适配器/设备来自
    /// 另一个实例 —— 那是"窗口开了但画不出来"那一族里最难查的一种。
    fn open(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title(self.title())
                        // ⚠ **物理**像素，不是逻辑像素。`--width/--height` 在这个命令行上
                        // 只有一个意思：**那张图的像素尺寸**（离线那条路就是它）。
                        // 拿逻辑尺寸去开窗，"窗口画的到底是多大一张图"就多出一个
                        // **没人写过的输入** —— 显示器的缩放因子（本机 175% ⇒ 960×640
                        // 会变成 1680×1120）。那样一来"与离线那张逐字节相同"就变成
                        // "在你还得知道 DPI 的前提下相同"，而判据里不许有这种格子。
                        .with_inner_size(PhysicalSize::new(self.size.0, self.size.1)),
                )
                .map_err(|err| format!("建窗口失败：{err}"))?,
        );
        let instance = crate::gpu::instance();
        let surface = instance
            .create_surface(window.clone())
            .map_err(|err| format!("把窗口接到 wgpu 失败：{err}"))?;

        let size = window.inner_size();
        let gpu = crate::gpu::connect_with(instance, Some(&surface));
        let capabilities = surface.get_capabilities(&gpu.adapter);
        // 交换链的格式：挑一个 sRGB 的（画面本来就是 sRGB 的字节），
        // 再把它**非 sRGB 的那一面**列进 `view_formats` —— 呈现那一段要用它（见 `Present::upload`）。
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(|format| format.is_srgb())
            .ok_or_else(|| {
                format!(
                    "这块 surface 一个 sRGB 格式都不给（{:?}）：画面是 sRGB 的字节，换一个后端",
                    capabilities.formats
                )
            })?;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: if self.novsync {
                wgpu::PresentMode::AutoNoVsync
            } else {
                // 缺省是 `AutoVsync`（Bevy 那边不给 `--novsync` 时也是它）——
                // 垂直同步不是"性能开关"，它是窗口不该撕裂的那条底线。
                wgpu::PresentMode::AutoVsync
            },
            desired_maximum_frame_latency: 2,
            alpha_mode: capabilities.alpha_modes[0],
            view_formats: vec![format.remove_srgb_suffix()],
        };
        surface.configure(&gpu.device, &config);

        println!(
            "表面：{:?}｜呈现模式 {:?}｜视图格式 {:?}｜物理尺寸 {}×{}（缩放因子 {}）｜适配器 {}",
            config.format,
            config.present_mode,
            config.view_formats,
            config.width,
            config.height,
            window.scale_factor(),
            gpu.adapter.get_info().name,
        );

        self.present = Some(Present::new(&gpu.device, format.remove_srgb_suffix()));
        self.size = (config.width, config.height);
        self.gpu = Some(gpu);
        self.surface = Some(surface);
        self.config = Some(config);
        self.window = Some(window);

        // ---- 两处"我在线"的声明**都放在窗口真的起来之后** ----
        //
        // ⚠ 放在这之前的话，起窗口失败（比如 `create_surface` 被拒）会留下一份
        //    **假的心跳**：外部仪器看见"窗口在线"，而进程已经死了。
        //    写在这里之后，租约的含义是"窗口已经画得出来"，与它该有的意思一致。
        let _ = write_request(&ViewRequest {
            scene: self.scene.clone(),
            key: self.key,
            at: self.request_at,
            shot: self.pending_shot.is_some(),
            sheet: self.sheet,
            ask_camera: false,
            set_camera: None,
            shot_path: self
                .pending_shot
                .as_ref()
                .map(|path| path.display().to_string()),
        });
        // ⚠ 这一次是**无条件**写（`write_lease`，不是心跳）：窗口刚起来，租约还不存在，
        //    而心跳那条规矩是"缺了就不续" —— 用错那一个，租约永远建不出来。
        write_lease();
        Ok(())
    }

    fn title(&self) -> String {
        format!(
            "px_render_wgpu 预览 — {}｜键 {:016x}｜{}",
            self.scene,
            self.key,
            describe_orbit(self.orbit)
        )
    }

    fn update_title(&self) {
        if let Some(window) = &self.window {
            window.set_title(&self.title());
        }
    }

    /// 窗口尺寸变了：交换链重建，画面**必须**重画（长宽比变了 ⇒ 换了一台相机）。
    fn resize(&mut self, size: PhysicalSize<u32>) {
        let (Some(surface), Some(config), Some(gpu)) =
            (self.surface.as_ref(), self.config.as_mut(), self.gpu.as_ref())
        else {
            return;
        };
        if size.width == 0 || size.height == 0 {
            // 最小化：交换链不允许 0 尺寸（wgpu 会拒）。等下一次 Resized。
            return;
        }
        config.width = size.width;
        config.height = size.height;
        surface.configure(&gpu.device, config);
        self.size = (size.width, size.height);
        self.dirty = true;
    }

    /// 现在这台相机在哪 —— 大小只影响投影，位姿与它无关（`seed_orbit` 只用位置）。
    fn current_position(&self) -> [f32; 3] {
        let (width, height) = self.size;
        let camera = self.camera(width.max(1), height.max(1));
        [camera.position.x, camera.position.y, camera.position.z]
    }

    /// 鼠标左键拖着转视角。**逐字照抄** Bevy 的 `orbit_camera`（灵敏度、夹取范围都一样）。
    ///
    /// ⚠ 第一次拖动之前窗口还在**探针机位**（没有角这份状态）⇒ 先把位姿反推成角
    /// （[`seed_orbit`]），再把这一笔位移加上去。少了这一步，鼠标的**第一下**
    /// 会静默丢掉（相机没动，而"没动"看起来像卡了）。
    fn drag(&mut self, position: (f64, f64)) {
        let previous = self.cursor.replace(position);
        if !self.dragging {
            return;
        }
        let Some(previous) = previous else {
            return;
        };
        let (dx, dy) = (position.0 - previous.0, position.1 - previous.1);
        let mut state = self
            .orbit
            .unwrap_or_else(|| seed_orbit(self.current_position()));
        state[0] -= drag_degrees(YAW_PER_PIXEL, dx);
        let limit = pitch_limit_degrees();
        state[1] = (state[1] - drag_degrees(PITCH_PER_PIXEL, dy)).clamp(-limit, limit);
        self.orbit = Some(state);
        self.dirty = true;
        self.update_title();
    }

    fn wheel(&mut self, delta: MouseScrollDelta) {
        // Windows 后端给的是 `LineDelta`（一格 = 1.0）。`PixelDelta` 只在别的平台出现，
        // 而**没有任何读数依赖它** —— 除以 100 是"一格约 100 像素"这个通行口径，
        // 写出来只是为了让那一支不是"悄悄什么都不做"。
        let notches = match delta {
            MouseScrollDelta::LineDelta(_, y) => f64::from(y),
            MouseScrollDelta::PixelDelta(position) => position.y / 100.0,
        };
        if notches == 0.0 {
            return;
        }
        let mut state = self
            .orbit
            .unwrap_or_else(|| seed_orbit(self.current_position()));
        state[2] = (f64::from(state[2]) * (1.0 - notches * ZOOM_PER_NOTCH))
            .clamp(f64::from(DISTANCE_MIN), f64::from(DISTANCE_MAX)) as f32;
        self.orbit = Some(state);
        self.dirty = true;
        self.update_title();
    }

    /// `--place` / 请求里的 `set_camera`。
    fn place(&mut self, place: [f32; 3]) {
        match orbit_of("--place", place) {
            Ok(place) => {
                self.orbit = Some(place);
                self.dirty = true;
                self.update_title();
                println!(
                    "相机被摆到 yaw {:.4}｜pitch {:.4}｜distance {:.4}",
                    place[0], place[1], place[2]
                );
            }
            Err(message) => eprintln!("{message}"),
        }
    }

    /// 问一句"相机在哪儿"：把三个数与位置写进 `VIEW_CAMERA`。
    ///
    /// ⚠ 位置用**画面上那台相机**（`camera()`），不是把角再算一遍 ——
    /// 两处各算一次就是两处会漂开的真相（§146.3 ③）。
    /// 探针机位那一档的三个角也**只有一份算法**（[`seed_orbit`]）：那是鼠标接管时的同一个反推，
    /// 抄成两份的话，"`--where` 报的角"与"鼠标一动从哪个角开始"就会各自漂开。
    fn answer_camera(&self, at: u64) {
        let (width, height) = self.size;
        let camera = self.camera(width.max(1), height.max(1));
        let (yaw, pitch, distance, derived) = match self.orbit {
            Some(state) => (state[0], state[1], state[2], false),
            // 探针机位：窗口手里**没有**角这一份状态，三个数只能从位姿反推。
            // 位置那一栏仍然是真的，反推出来的角只在"想用 --place 复现"时被用到，
            // 而那一档得到的是**轨道**那一档（所以 `derived` 必须为真）。
            None => {
                let position = camera.position;
                let angles = seed_orbit([position.x, position.y, position.z]);
                (angles[0], angles[1], angles[2], true)
            }
        };
        let reply = CameraReply {
            at,
            yaw,
            pitch,
            distance,
            position: [camera.position.x, camera.position.y, camera.position.z],
            size: [width, height],
            derived,
        };
        match serde_json::to_string_pretty(&reply) {
            Ok(text) => {
                if let Err(err) = std::fs::write(VIEW_CAMERA, text) {
                    eprintln!("写相机回话失败：{err}");
                }
            }
            Err(err) => eprintln!("序列化相机回话失败：{err}"),
        }
    }

    /// 周期活之一：看请求文件。`--show` / `--where` / `--place` / `--shot` 全从这里进来。
    fn poll_request(&mut self) {
        let Some(request) = read_request() else {
            return;
        };
        if request.at == self.request_at {
            return;
        }
        self.request_at = request.at;
        // 换场景之前先把相机摆好：`--place` 与场景无关，只是"把镜头挪过去"。
        if let Some(place) = request.set_camera {
            self.place(place);
        }
        if request.ask_camera {
            self.answer_camera(request.at);
        }
        if request.shot {
            self.pending_shot = Some(PathBuf::from(
                request.shot_path.as_deref().unwrap_or(VIEW_SHOT),
            ));
            self.dirty = true;
        }
        // 换了场景，或者内容键变了 ⇒ 重画；同一份产物再推一次只是「看见了」，不动。
        let changed = request.scene != self.scene
            || request.key == 0
            || request.key != self.key
            || self.last.is_none();
        if request.scene != self.scene {
            self.scene = request.scene.clone();
            self.scene_modified = std::fs::metadata(&self.scene)
                .and_then(|meta| meta.modified())
                .ok();
            self.first_frame = true;
        }
        self.key = request.key;
        self.sheet = request.sheet;
        if request.sheet {
            println!("⚠ 预览窗口不出多视口对照图（那是 --serve 出图的事）：这一份的相机表只当参考");
        }
        if changed {
            self.dirty = true;
            self.update_title();
            println!("窗口切到：{}｜键 {:016x}", self.scene, self.key);
        } else {
            println!("同一份产物（键 {:016x}）再推一次 ⇒ 不重画", self.key);
        }
    }

    /// 周期活之二：产物文件自己动了（重烘之后窗口跟上）。
    ///
    /// 判据是 mtime 闹钟 + **载荷指纹**（§50）：重新烘一份内容一模一样的产物
    /// （或者只是 touch 了一下）不该让窗口重画。指纹为 0 的旧产物照样重画 —— 判不了就照旧。
    ///
    /// ⚠ 下一单元（shader 热重载）接的就是**这一处**：`.wgsl` 改动不在产物的指纹里，
    /// 它要在同一个 tick 里多问一句"内容闭包变了吗"，然后置 `dirty`。
    fn poll_scene_file(&mut self) {
        let Ok(meta) = std::fs::metadata(&self.scene) else {
            return;
        };
        let Ok(modified) = meta.modified() else {
            return;
        };
        if self.scene_modified == Some(modified) {
            return;
        }
        self.scene_modified = Some(modified);
        match fingerprint_of(Path::new(&self.scene)) {
            Ok(fingerprint) if fingerprint != 0 && fingerprint == self.key => {
                println!("场景产物动过，但载荷指纹没变（{fingerprint:016x}）⇒ 不重画");
            }
            Ok(fingerprint) => {
                self.key = fingerprint;
                self.dirty = true;
                self.update_title();
                println!("场景产物更新，重载：{}（键 {fingerprint:016x}）", self.scene);
            }
            Err(err) => println!("场景产物动过，读不到清单：{err}"),
        }
    }

    /// 画一帧（脏了才画），然后把它呈到屏幕上。
    ///
    /// ⚠ 全程只碰**直接字段**（不调 `&self` 的方法）拿借用：`render::run` 借的是
    /// `self.gpu`，而紧接着要改的是 `self.present` / `self.pending_shot` —— 同一结构体的
    /// 不同字段可以同时借，走一次 `&self` 就不行了。
    fn draw(&mut self) {
        // ⚠ 这里**不 `expect`**：`resumed` 之前、`open` 失败之后都可能进来一次，
        //    而 panic 会把租约留在盘上（§147：不留陈租约）。
        if self.window.is_none() || self.gpu.is_none() {
            return;
        }

        if self.dirty {
            self.dirty = false;
            // 先把"这一帧要什么"全部抄成局部量（这一批读都是 `&self`，必须在改之前做完）。
            let views = self.views();
            let scene = self.scene.clone();
            let pcg_root = self.pcg_root.clone();
            let (width, height) = self.size;
            let started = Instant::now();
            let rendered = {
                let Some(gpu) = self.gpu.as_ref() else {
                    return;
                };
                render::run(
                    gpu,
                    Path::new(&scene),
                    &pcg_root,
                    views,
                    width,
                    height,
                )
            };
            match rendered {
                Ok(rendered) => {
                    let (width, height) = (rendered.width, rendered.height);
                    if self.first_frame {
                        for line in &rendered.audit {
                            println!("{line}");
                        }
                        self.first_frame = false;
                    }
                    if let Some(gpu) = self.gpu.as_ref() {
                        if let Some(present) = self.present.as_mut() {
                            present.upload(&gpu.device, &gpu.queue, width, height, &rendered.pixels);
                        }
                    }
                    // 截图**从刚回读出来的这批字节**走同一条 PNG 路径（§104 第 3 条）：
                    // 屏幕上是它、文件里也是它，中间没有第二次渲染。
                    if let Some(path) = self.pending_shot.take() {
                        match shot::write_png(&path, width, height, rendered.pixels) {
                            Ok(bytes) => println!(
                                "预览截图 → {}（{}×{}，{} 字节，sha256 {}）",
                                path.display(),
                                width,
                                height,
                                bytes,
                                crate::digest::short(&path)
                            ),
                            Err(message) => eprintln!("{message}"),
                        }
                    }
                    self.last = Some((width, height));
                    self.frames += 1;
                    println!(
                        "第 {} 帧：{}×{}｜{}｜{} ms",
                        self.frames,
                        width,
                        height,
                        describe_orbit(self.orbit),
                        started.elapsed().as_millis()
                    );
                }
                Err(message) => {
                    // 画不出来就**留着窗口里现在这张图**（Bevy 的 `rebuild_scene` 同一条）。
                    eprintln!("⚠ 这一帧画不出来，保留窗口里现在这张图：{message}");
                }
            }
        }

        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let (device, queue) = (&gpu.device, &gpu.queue);
        let Some(surface) = self.surface.as_ref() else {
            return;
        };
        let Some(config) = self.config.as_ref() else {
            return;
        };
        let frame = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                surface.configure(device, config);
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return,
            wgpu::CurrentSurfaceTexture::Validation => {
                eprintln!("⚠ 交换链拿不到下一张（校验错）：这一帧跳过");
                return;
            }
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("px_render_wgpu 呈现（非 sRGB）"),
            format: Some(config.format.remove_srgb_suffix()),
            ..Default::default()
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("px_render_wgpu 呈现"),
        });
        if let Some(present) = self.present.as_ref() {
            present.draw(&mut encoder, &view);
        }
        queue.submit(Some(encoder.finish()));
        frame.present();
    }

    /// 只重呈一次（不重画）：窗口露出来 / 尺寸没变但要刷一下时用。
    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn shot_now(&mut self) {
        // `s` 键：存进"这一次请求说的那个路径"，没有请求给过就用 Bevy 那条固定路径。
        let path = self
            .pending_shot
            .clone()
            .unwrap_or_else(|| PathBuf::from(VIEW_SHOT));
        self.pending_shot = Some(path);
        self.dirty = true;
        self.request_redraw();
    }

    fn cleanup(&self) {
        // 收工时把租约删掉：它是一份"我在线"的声明，而进程没了还留着就是**假的**在线
        // （§147 那条纪律：不留陈租约；下一次启动的单例闸判的正是这个文件）。
        let _ = std::fs::remove_file(VIEW_LEASE);
        println!("预览窗口收工（{} 帧）", self.frames);
    }
}

/// 探针机位 → 轨道角：**只是鼠标接管那一刻的种子**。
///
/// 窗口一开始是"不给 `--cam` 那一档"（探针机位），而鼠标要改的是**角**。
/// 这两个表示之间只能反推一次（`probe_camera` 的构造反过来：`direction = position/|position|`，
/// `yaw = atan2(x, z)`、`pitch = asin(y/|position|)`、`distance = |position|`），
/// 而从这一刻起相机归那三个数管 —— 反推不是恒等（浮点），所以 [`CameraReply::derived`]
/// 那一格必须把"这三个数是反推的"说出来。
fn seed_orbit(position: [f32; 3]) -> [f32; 3] {
    let [x, y, z] = position;
    let length = (x * x + y * y + z * z).sqrt();
    [
        x.atan2(z).to_degrees(),
        if length > 0.0 {
            (y / length).asin().to_degrees()
        } else {
            0.0
        },
        length,
    ]
}

fn describe_orbit(orbit: Option<[f32; 3]>) -> String {
    match orbit {
        Some([yaw, pitch, distance]) => {
            format!("相机 yaw {yaw:.4}｜pitch {pitch:.4}｜distance {distance:.4}")
        }
        None => "相机：探针机位（不给 --cam 那一档）".to_string(),
    }
}

impl ApplicationHandler for Viewer {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        if let Err(message) = self.open(event_loop) {
            eprintln!("{message}");
            event_loop.exit();
            return;
        }
        self.update_title();
        self.request_redraw();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => self.resize(size),
            // 缩放因子变了会紧跟一次 `Resized`，尺寸与交换链都在那里处理。
            WindowEvent::ScaleFactorChanged { .. } => {}
            WindowEvent::RedrawRequested => self.draw(),
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                self.dragging = state == ElementState::Pressed;
                if !self.dragging {
                    // 松手时把方位打一行：它能直接粘回 `--place`，也是"人看到了什么"的读数。
                    println!("拖到：{}｜{}", describe_orbit(self.orbit), place_line(self.orbit));
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.drag((position.x, position.y));
                if self.dirty {
                    self.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.wheel(delta);
                if self.dirty {
                    self.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return;
                }
                match event.logical_key {
                    Key::Named(NamedKey::Escape) => event_loop.exit(),
                    Key::Character(ref text) if text.eq_ignore_ascii_case("q") => event_loop.exit(),
                    Key::Character(ref text) if text.eq_ignore_ascii_case("s") => self.shot_now(),
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        if now.duration_since(self.last_poll) >= POLL_INTERVAL {
            self.last_poll = now;
            self.poll_request();
            self.poll_scene_file();
            if self.dirty {
                self.request_redraw();
            }
        }
        if now.duration_since(self.last_heartbeat) >= HEARTBEAT {
            self.last_heartbeat = now;
            heartbeat_now();
        }
        // 事件驱动 + 一个 200 ms 的闹钟：没有帧循环要转，也不拿 `Poll` 空转烧 CPU。
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.last_poll + POLL_INTERVAL));
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.cleanup();
    }
}

/// 一行能直接粘回命令行的 `--place`。
fn place_line(orbit: Option<[f32; 3]>) -> String {
    match orbit {
        Some([yaw, pitch, distance]) => format!("--place {yaw:.4},{pitch:.4},{distance:.4}"),
        None => "（探针机位：没有对应的 --place）".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 三个文件是**两个宿主之间的全部约定**（§147 那三个文件名逐字相同），
    /// 所以 schema 得钉两件事：
    ///
    /// 1. Bevy 写的请求（**没有** `shot_path` 那一格）我们解得开；
    /// 2. 我们写的请求（多一格）它解得开 —— serde 默认忽略不认识的多余字段，
    ///    这一条**必须**有一次实测，不能靠"我记得 serde 是这样"。
    #[test]
    fn the_bevy_shaped_request_still_parses_and_ours_still_parses_for_it() {
        let bevy = r#"{
            "scene": "a.pxart",
            "key": 18,
            "at": 7,
            "shot": true,
            "sheet": false,
            "ask_camera": false,
            "set_camera": null
        }"#;
        let request: ViewRequest = serde_json::from_str(bevy).expect("Bevy 写的那一份要解得开");
        assert_eq!(request.scene, "a.pxart");
        assert_eq!(request.at, 7);
        assert!(request.shot);
        assert_eq!(
            request.shot_path, None,
            "Bevy 那一版没有这一格 ⇒ 缺省是 None ⇒ 截图落到它写死的那条路径"
        );
        assert_eq!(
            initial_shot(&request),
            Some(PathBuf::from(VIEW_SHOT)),
            "缺省必须是 Bevy 的 `target/viewer-shot.png`（不是我们发明的路径）"
        );

        // 反过来：我们写出去的那一份，Bevy 的 `ViewRequest`（无 deny_unknown_fields）解它时
        // 只会忽略 `shot_path`。这里用"照它的字段集解一遍"来钉这件事。
        let ours = serde_json::to_string(&ViewRequest {
            scene: "b.pxart".to_string(),
            key: 1,
            at: 2,
            shot: true,
            sheet: false,
            ask_camera: true,
            set_camera: Some([1.0, 2.0, 3.0]),
            shot_path: Some("target/x.png".to_string()),
        })
        .expect("序列化");
        let value: serde_json::Value = serde_json::from_str(&ours).expect("解成值");
        for field in [
            "scene",
            "key",
            "at",
            "shot",
            "sheet",
            "ask_camera",
            "set_camera",
        ] {
            assert!(
                value.get(field).is_some(),
                "Bevy 要的那一格 '{field}' 一个都不能少（少了它整份请求解不开）"
            );
        }
    }

    /// `--place` 收进来的数：pitch 夹到**相机自己**的范围（`probe_camera` 的 ±89.5°），
    /// 非有限的数当场拒。
    ///
    /// 为什么不"原样收下"：`probe_camera` 内部照样会夹，于是窗口的**状态**里存着一个
    /// 相机不会用的角度 ⇒ `--where` 报一个与画面不符的数（§146.3 ③：说错了原因，
    /// 与没拦住一样贵）。
    #[test]
    fn a_place_angle_is_clamped_to_what_the_camera_will_actually_use() {
        assert_eq!(orbit_of("--place", [0.0, 8.0, 3.15]).unwrap(), [0.0, 8.0, 3.15]);
        assert_eq!(
            orbit_of("--place", [10.0, 120.0, 5.0]).unwrap(),
            [10.0, PITCH_DEGREES_LIMIT, 5.0]
        );
        assert_eq!(
            orbit_of("--place", [10.0, -120.0, 5.0]).unwrap(),
            [10.0, -PITCH_DEGREES_LIMIT, 5.0]
        );
        assert!(orbit_of("--place", [f32::NAN, 0.0, 3.0]).is_err(), "NaN 的 yaw 要当场拒");
        assert!(orbit_of("--place", [0.0, 0.0, f32::INFINITY]).is_err(), "无穷远要当场拒");
    }

    /// 探针位姿 → 轨道角：**反推不是恒等**，但必须是那条构造的逆（位置量级上对得上）。
    ///
    /// ⚠ 为什么钉"近似"而不是"逐位"：这两个表示本来就不是双射（探针机位不在
    /// `direction × distance` 那张曲面上）。实测的差落在像素上：把反推出来的三个数
    /// `--place` 回去，960×640 的 `orbit-bare` 与探针机位那张差 **314 个像素**（最大通道差 11）
    /// ⇒ 所以回话里那一格 `derived` 必须为真（说清"这三个数是反推的"）。
    /// 这一条钉的是"反推的方向对、量级对"——**不是**"它等价于探针机位"。
    #[test]
    fn the_seed_angles_are_the_inverse_of_the_probe_pose_within_rounding() {
        let probe = crate::camera::probe_camera(None, 960.0 / 640.0);
        let position = [probe.position.x, probe.position.y, probe.position.z];
        assert_eq!(position, [0.0, 0.55, 3.15], "探针机位的位姿");

        let angles = seed_orbit(position);
        assert_eq!(angles[0], 0.0, "正前方 ⇒ yaw 0");
        assert!((angles[1] - 9.9042).abs() < 0.001, "pitch 约 9.9°：{}", angles[1]);
        assert!((angles[2] - 3.1977).abs() < 0.001, "distance 约 3.1977：{}", angles[2]);

        let back = crate::camera::probe_camera(Some(angles), 960.0 / 640.0);
        for (what, a, b) in [
            ("x", back.position.x, position[0]),
            ("y", back.position.y, position[1]),
            ("z", back.position.z, position[2]),
        ] {
            assert!((a - b).abs() < 1e-3, "{what} 反推回去差了 {}：{a} vs {b}", (a - b).abs());
        }

        // 正下方/正上方那种退化输入不该给出 NaN（`asin` 的定义域）。
        assert!(seed_orbit([0.0, 0.0, 0.0]).iter().all(|v| v.is_finite()));
        assert!(seed_orbit([0.0, 0.0, 4.0])[1].abs() < 1e-4);
    }

    /// 标题/日志那一行 `--place` 的格式：它能直接粘回命令行（4 位小数，逗号分隔）。
    /// 探针机位没有对应的三元组 —— 那一档必须**说出来**，不能编一个数。
    #[test]
    fn the_place_line_is_copy_pasteable_and_the_probe_pose_says_so() {
        assert_eq!(
            place_line(Some([35.0, 20.0, 8.0])),
            "--place 35.0000,20.0000,8.0000"
        );
        assert!(place_line(None).contains("探针机位"));
        assert!(describe_orbit(None).contains("探针机位"));
        assert!(describe_orbit(Some([1.0, 2.0, 3.0])).contains("distance 3.0000"));
    }

    /// ⚠ 鼠标那一路的**单位**：Bevy 的常数是弧度，我们的状态是度。
    ///
    /// 这一格是实测抓到的缺陷：第一版拿 `0.006` 直接往度上加，于是"拖 100 像素"只转
    /// **0.6°**（人眼看不见，看着像"鼠标没反应"），而 pitch 的夹取变成 ±1.25°（一碰就到底）。
    /// 钉住三件事：① 换算确实乘了 180/π；② 拖满一屏的量级是"几十度"；③ 夹取是 ±71.62°。
    #[test]
    fn the_mouse_deltas_are_radians_converted_to_the_degrees_we_store() {
        // 100 像素 × 0.006 rad/px = 0.6 rad = 34.377°（**不是** 0.6°）
        let turned = drag_degrees(YAW_PER_PIXEL, 100.0);
        assert!((turned - 34.377_47).abs() < 0.001, "100 像素该转约 34.38°，实得 {turned}");

        // 夹取：Bevy 的 ±1.25 rad ⇒ ±71.6197°
        let limit = pitch_limit_degrees();
        assert!((limit - 71.619_73).abs() < 0.001, "夹取该是 ±71.62°，实得 {limit}");
        assert!(
            limit < PITCH_DEGREES_LIMIT,
            "鼠标的夹取比相机自己的 ±89.5° 紧（Bevy 就是这么定的），两条不是同一个数"
        );

        // 往上/往下拖都落在同一个范围里，而且不会因为单位错了而"一碰到底"
        let mut pitch = 0.0_f32;
        pitch = (pitch + drag_degrees(PITCH_PER_PIXEL, 50.0)).clamp(-limit, limit);
        assert!((pitch - 17.188_73).abs() < 0.001, "往下拖 50 像素该到约 17.19°，实得 {pitch}");
        assert!(pitch < limit, "还没到夹取点");
    }

    /// 窗口的"怎么看"**就是**离线那条路的入口：`Views::Single(那一份角)`。
    /// 两个表示之间不许有第二套翻译（角 → 相机只走 `camera::probe_camera`）。
    #[test]
    fn the_window_asks_for_the_same_views_the_offline_path_does() {
        assert_eq!(Views::Single(None), Views::Single(None));
        assert_eq!(
            Views::Single(Some([0.0, 0.0, 3.15])),
            Views::Single(Some([0.0, 0.0, 3.15]))
        );
        assert_ne!(Views::Single(None), Views::Single(Some([0.0, 0.0, 3.15])));
    }
}
