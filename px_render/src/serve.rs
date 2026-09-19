//! `--serve`：**租约 + `ProtocolId` 握手 + 批量请求**。
//!
//! 这一半的逻辑**逐字照搬** Bevy 宿主（`px_render/src/main.rs`：`serve` :1139、
//! `spawn_listener` :1253、`watch_lease` :1449、`accept_jobs` 里的出图结算 :2500-2557）。
//! 不另发明一份协议（§102）：`tools/harness.ps1` / `tools/frame-probe.ps1` 那整套仪器
//! 认的就是 `px_protocol` 那一份，换一个字节它们就全废。
//!
//! ## 与 Bevy 宿主的**形状差别**（只有一处，而它是本宿主的性质，不是偷工）
//!
//! Bevy 那边是"主世界发任务 → 渲染世界逐帧推进"的两段式：一条请求会被摊到好几帧上
//! （等管线、等资产、等 K 帧），所以必须有 `Job` 队列、`ActiveJob` 与"到哪一步了"的状态机。
//! 本宿主**没有渲染世界**：`create_render_pipeline` 是同步的（§104 第 5 条），一帧在一次
//! `render::run` 里从头画到尾。于是：
//!
//! - **没有队列**：一次请求在**一个函数调用**里跑完（`execute`），跑完就回话；
//! - **没有"瞬时就绪断言"要等**：管线编不出来就**当场拒这一条请求**，而不是排队等它——
//!   §104 第 5 条那条判据（"坏管线当场拒、不静默出缺材质的图"）在这里是**结构性**成立的，
//!   不是靠一个 gate 兜的；
//! - **没有跨请求的保留状态**：每一条请求自己 `render::run`（读文档 → 建计划 → 画 → 回读）。
//!   ⚠ 这条是**故意的**，而且它正是判据要的：Bevy 宿主"渲染相位常驻"⇒ 拿同一个服务连着出
//!   几份文档，后一份会被前一份的残留污染（起新服务才干净）。本宿主按构造就没有这个污染源，
//!   所以 J3 的 **R｜不重启**（一个 pid 吃下 4 份文档 + 2 份坏的）不必靠"每次都重启"来保证
//!   干净 —— 但**代价**是没有管线缓存，每一条请求都要重编一次它的管线。那一笔账属于计时
//!   那一档（J4/S7），不属于这里。

use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use px_host_protocol::client as client;
use px_host_protocol::frame::{self, Frame};
use px_host_protocol::render::{Job, Lease, Report, Request, Response, Scene, ShotReport};
use px_protocol::ProtocolId;

use crate::digest;
use crate::gpu::Gpu;
use crate::render;
use crate::report;
use crate::shot;

/// 租约自查的间隔。Bevy 那边是"每 120 帧看一次"（60 Hz ⇒ 约 2 s），口径是
/// **删掉租约 ⇒ 4 秒内自查退出**（§104 第 10 条）。这里没有帧循环，所以直接按秒给。
const LEASE_CHECK_INTERVAL: Duration = Duration::from_secs(2);

/// 一条请求的读/写超时（Bevy 的 `spawn_listener` 给的是 300 s，同一档）。
const IO_TIMEOUT: Duration = Duration::from_secs(300);

/// 经济世界（`Scene::World`）那条路的拒词。**一份真本、两处引用**：服务端对协议客户端拒它，
/// 客户端在没有 `--scene` 时也用它 —— 抄成两份措辞就会漂开，而漂开的那一天读的人会被指向
/// 错的原因（"没有在跑的渲染服务" ≠ "这条路本宿主没有"）。
pub const WORLD_REFUSAL: &str =
    "经济世界（`Scene::World` / `--stream`）那一路没有搬到这个宿主：\
     它画的是 sim 的世界视图，不是渲染文档（`.pxart`）";

/// 常驻服务：一台设备 + 一个 CAS 根。**没有别的状态**（见模块头那段）。
pub struct Server {
    gpu: Gpu,
    pcg_root: PathBuf,
}

/// 起服务：绑端口 → 建设备 → 写租约 → 等请求。
///
/// ⚠ **次序与 Bevy 宿主相反，而理由是硬的**：Bevy 那边是"先写租约、后起 app"，
/// 于是 app 启动期的断言失败（非 Vulkan ⇒ `exit(2)`）必须**清掉自己那份租约**
/// （`clear_own_lease`，§104 第 9 条；否则下一次合法启动会被单例闸拦成假故障）。
/// 本宿主的设备是在**一个函数调用**里建的，把它放在写租约**之前**，那条"写完租约才可能
/// 硬退出"的窗口就**不存在** —— 不需要清理代码，因为不存在要清理的东西。
/// （`gpu::connect` 失败时打的是 `后端断言失败` 前缀并 `exit(2)`，此刻租约还没落盘。）
pub fn serve(port: u16, pcg_root: PathBuf, width: u32, height: u32) -> Result<(), String> {
    let listener =
        TcpListener::bind(("127.0.0.1", port)).map_err(|err| format!("绑定端口失败：{err}"))?;
    let port = listener
        .local_addr()
        .map_err(|err| format!("拿不到端口：{err}"))?
        .port();

    let gpu = crate::gpu::connect();

    let id = ProtocolId::local();
    let lease_path = client::lease_path();
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
    // ⚠ 这一行**必须在**：`tools/harness.ps1::Start-RenderServer` 等的就是它
    // （`Select-String -Pattern '渲染管线全部就绪'`）。措辞里的"共 0 条"不是省事：
    // 本宿主此刻确实一条待编的管线都没有，而"编不出来"这件事在请求里当场就拒了。
    println!(
        "渲染管线全部就绪：本宿主是**同步**建管线（没有队列可等），此刻待编 0 条、失败 0 条"
    );
    // 尺寸这一栏在本宿主里**不参与出图**（每条请求自带 width/height）。与其让它变成一个
    // 谁也不看的旗标，不如当场说清：§106 那条"文档里写着的命令必须真的被跑过一次"。
    println!("尺寸以请求里的为准（--width/--height 是 Bevy 宿主那一路的初始画布，这里只收不用）：{width}×{height}");

    spawn_lease_watch(lease_path, lease.pid);

    let mut server = Server { gpu, pcg_root };
    for connection in listener.incoming() {
        let Ok(stream) = connection else {
            continue;
        };
        // 串行服务：**同一时刻只有一个渲染循环**（§104 第 10 条）——
        // 两个并列的循环会让双方的读数都作废，所以这里连线程都不开。
        serve_connection(&mut server, stream);
    }
    Ok(())
}

/// 租约没了 / 易主 ⇒ 自己退出。
///
/// ⚠ 为什么是**另一个线程**而不是主循环里的一步：本宿主的主循环会**阻塞**在一条请求上
/// （最长一次 `render::run` 几秒）。塞在循环里的话，"删掉租约"要等这条请求跑完才生效，
/// 而那正是仪器收尾时要等的东西。Bevy 那边不存在这个问题（它的 Update 每帧都跑）。
fn spawn_lease_watch(path: PathBuf, pid: u32) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(LEASE_CHECK_INTERVAL);
            match client::read_lease(&path) {
                Some(lease) if lease.pid == pid => {}
                _ => {
                    println!("租约已易主，渲染服务退出");
                    std::process::exit(0);
                }
            }
        }
    });
}

/// 一条连接：握手 → 收请求 → 出图/拒绝 → 回话。整个进程**只在这里**碰 GPU。
fn serve_connection(server: &mut Server, mut stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_nodelay(true);

    let local = ProtocolId::local();
    match frame::read_frame(&mut stream) {
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
                let _ = frame::write_frame(&mut stream, &Frame::Refused(reason.clone()));
                eprintln!("拒绝连接：{reason}");
                return;
            }
        }
        other => {
            let reason = format!("握手指望 Protocol 帧，收到 {other:?}");
            let _ = frame::write_frame(&mut stream, &Frame::Refused(reason));
            return;
        }
    }
    if frame::write_frame(&mut stream, &Frame::Protocol(local)).is_err() {
        return;
    }

    let request = match frame::read_frame(&mut stream) {
        Ok(Some(Frame::Request(request))) => request,
        Ok(other) => {
            let reason = format!("指望 Request 帧，收到 {other:?}");
            let _ = frame::write_frame(&mut stream, &Frame::Refused(reason));
            return;
        }
        Err(err) => {
            eprintln!("读请求失败：{err}");
            return;
        }
    };

    // 拒绝**不杀服务**：这是 R｜不重启 那条判据的一半（一个 pid 吃下 4 份好文档 + 2 份坏的）。
    let frame = match server.execute(&request) {
        Ok(response) => Frame::Response(response),
        Err(reason) => {
            eprintln!("拒绝任务：{reason}");
            Frame::Refused(reason)
        }
    };
    let _ = frame::write_frame(&mut stream, &frame);
}

/// 请求里的一步：渲哪一份文档、存哪儿、从哪个角度看。
///
/// 与 Bevy 的 `Step` 是同一件事（那边还有 `StepScene::World` 那一支，本宿主没有）。
#[derive(Debug)]
struct Step {
    scene: String,
    out: String,
    /// 这一步**怎么看**：`--cam` 那一台，或者产物自带的相机表（`--sheet` / J2）。
    ///
    /// ⚠ 它是 `render::Views`，不是 `Option<[f32; 3]>`：对照图不是"另一台相机"，
    /// 而是"12 台相机 + 12 块格子"（格子的排布是渲染器的事，见 `SheetCell` 的注释）。
    views: render::Views,
}

impl Server {
    /// 一次请求：翻成若干步 → 逐步出图 → 落报告 → 回话。
    fn execute(&mut self, request: &Request) -> Result<Response, String> {
        let started = Instant::now();
        let steps = steps_of(request)?;

        let mut shot_reports: Vec<ShotReport> = Vec::with_capacity(steps.len());
        let mut shots: Vec<String> = Vec::with_capacity(steps.len());
        // 这一批第一张图的网格（`diff_vs_ref_grid` 的参考）。**按顺序**取第一张，
        // 与 Bevy 一致：调用方按约定把无云档放第一个（`ShotReport` 的文档注释）。
        let mut reference: Option<Vec<u64>> = None;
        let mut last = (String::new(), 0_u32, 0_u32, 0_u64);

        for step in &steps {
            let path = Path::new(&step.scene);
            let rendered = render::run(
                &self.gpu,
                path,
                &self.pcg_root,
                step.views,
                request.width,
                request.height,
            )?;
            for line in &rendered.audit {
                println!("{line}");
            }
            let label = rendered.audit.join("\n");
            // ⚠ 三个读数从**回读出来的字节**算，尺寸也要用**画出来的那张**的
            //    （对照图是 3840×1920，而请求给的 960×640 只是**一格**的大小）。
            let width = rendered.width;
            let height = rendered.height;

            let stat = report::measure(&rendered.pixels, width, height);
            let declared = rendered.declared_clouds;
            let is_reference = reference.is_none();
            let reference_grid = reference.get_or_insert_with(|| stat.grid.clone());
            let diff = report::grid_diff(reference_grid, &stat.grid);
            let threshold = report::cloud_threshold(reference_grid.iter().sum());
            let (has_cloud, verdict) = report::cloud_verdict(&report::Verdict {
                declared,
                placeholder_px: stat.placeholder_px,
                bright_px: stat.bright_px,
                diff,
                threshold,
                is_reference,
            });

            let out = PathBuf::from(&step.out);
            let bytes = shot::write_png(&out, width, height, rendered.pixels)?;
            let sha256 = digest::sha256_file(&out)?;
            println!(
                "出图：{label} → {}（{}×{}，{} 字节，{} ms，sha256 {}）",
                out.display(),
                width,
                height,
                bytes,
                started.elapsed().as_millis(),
                &sha256[..sha256.len().min(16)],
            );
            println!("  可用性：{verdict}");
            println!("  执行了：{}", rendered.executed.join(" → "));
            for (skipped, why) in &rendered.skipped {
                println!("⚠ 没有执行 '{skipped}'：{why}");
            }

            shots.push(out.display().to_string());
            shot_reports.push(ShotReport {
                // ⚠ 与 Bevy 一致：`ShotReport.scene` 是**路径**、`label` 是那一串说明。
                // （`Response.scene` 恰好相反 —— 那里放的是 label。两处别"理顺"。）
                scene: step.scene.clone(),
                label: label.clone(),
                out: out.display().to_string(),
                width,
                height,
                bytes,
                sha256,
                placeholder_px: stat.placeholder_px,
                bright_px: stat.bright_px,
                diff_vs_ref_grid: diff,
                declared_clouds: declared,
                has_cloud,
                verdict,
            });
            last = (
                label,
                width,
                height,
                std::fs::metadata(&out).map(|meta| meta.len()).unwrap_or(0),
            );
        }

        // 报告：与落盘那份**逐字节相同**（落盘多一个换行，与 Bevy 的 `write` 一样）。
        let protocol = ProtocolId::local();
        let report = Report {
            schema_version: protocol.schema_version,
            protocol_hash: format!("{:016x}", protocol.protocol_hash),
            job: request.job.name().to_string(),
            // 宽高说的是**最后一张**（Bevy 的 `build_report` 取 `shot_reports.last()`）。
            width: last.1,
            height: last.2,
            millis: started.elapsed().as_millis() as u64,
            shots: shot_reports,
            // 性能那两路不在这一版（见 [`steps_of`] 之前那段）：报告里这两栏因此是空的，
            // 而不是"填了但没有可比对象"。
            perf: Vec::new(),
            pair: None,
        };
        let text = serde_json::to_string_pretty(&report).unwrap_or_else(|err| {
            eprintln!("报告序列化失败：{err}");
            String::new()
        });
        if !request.report.is_empty() {
            let path = PathBuf::from(&request.report);
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match std::fs::write(&path, format!("{text}\n")) {
                Ok(()) => println!("报告：{}", path.display()),
                Err(err) => eprintln!("报告写不进去（{}）：{err}", path.display()),
            }
        }

        Ok(Response {
            out: shots.last().cloned().unwrap_or_default(),
            scene: last.0,
            width: last.1,
            height: last.2,
            bytes: last.3,
            millis: started.elapsed().as_millis() as u64,
            // ⚠ 恒 `true`，而这不是"填一个缺省值"：这个字段的口径是"回话那一刻还有没有
            // **待编**的管线"（Bevy 那边 = `ready.get() == PIPELINES_READY`）。本宿主的管线
            // 是在这条请求里**同步**建完的，建不出来就已经回 `Refused` 了 ⇒ 能走到这里，
            // "待编 0 条"按构造成立。给它写 `false` 才会是一句假话（客户端会打成"服务刚起"）。
            warm: true,
            shots,
            report: text,
            report_path: request.report.clone(),
        })
    }
}

/// 把一次请求摊成一队「要出的图」，并把**能力之外**的东西当场拒掉。
///
/// 一条拒词的理由要说对（§146.3 ③：「拦住了不等于说对了」）：
///
/// - **性能那两路**（`Perf` / `Stable`）：它们要的是**计时用的帧循环**——逐帧采样、丢窗、等
///   "重建后已渲染 K 帧"，外加**每条 pass 的编码器级 GPU 时间戳**。本宿主现在**按需渲染**
///   （一条请求画一帧就回话），而 S7 前半那个预览窗口的循环是**交互循环**：画面变了才画一帧
///   （相机一动、场景一换），它给不出逐帧序列，也没有那七段 span。
///   ⇒ `gpu_ms` / `pair` 这两个量**没有可比对象**：
///   Bevy 那边的 `gpu_ms` 是 `main_opaque_pass_3d` / `main_transparent_pass_3d` /
///   `prepass` / mip / tonemapping / upscaling 那几条编码器级 span 的**求和**（§104 第 4 条），
///   而 app 逐帧毫秒是"GPU 在飞 4 帧"那条流水线的周期。在按需渲染上凑一个同名的数
///   = 一个数放进一个语义不同的字段里，比没有这个数坏。
///
/// ⚠ `view.sheet`（对照图）**不再是拒词**（J2 那一档已经落地）：它现在是**一步怎么看**
/// 的一档，等价于 Bevy 那边 `View { sheet, columns }` —— 相机表住在产物里（`.pxart`），
/// 格子的排布是渲染器的事（`SheetCell` 的注释）。产物没带相机表时由
/// `render::placements` 当场拒，而且那句话说的是"这一步没带相机表"（与 Bevy 逐字同一条）。
fn steps_of(request: &Request) -> Result<Vec<Step>, String> {
    if !matches!(request.job, Job::Shots) {
        return Err(format!(
            "`{}` 那一路不在这一版：它要的是**计时用的帧循环**（逐帧采样 / 丢窗 / 等 K 帧 \
             + 每条 pass 的编码器级 GPU 时间戳），而本宿主现在**按需渲染** —— \
             预览窗口（S7 前半）那个循环只在画面变了时画一帧，给不出逐帧序列、也没有那七段 span。\
             报告里的 `gpu_ms` / `pair` 因此没有可比对象（Bevy 那几段按 §104 第 4 条切，\
             与「按需画 N 次」不是同一个量）。要计时读数请走 `--spans 预热,测量`：\
             它量的是**逐条 pass** 的编码器级时间戳（§153 的 J4 仪器）—— \
             ⚠ 那个数**不叫** `gpu_ms`，与这一路说的 `gpu_ms` 不是同一个量。",
            request.job.name()
        ));
    }
    if request.width == 0 || request.height == 0 {
        return Err(format!(
            "请求的尺寸是 {}×{}：宽高都得大于 0",
            request.width, request.height
        ));
    }
    match &request.scene {
        Scene::Artifact { scene } => {
            if request.out.trim().is_empty() {
                return Err(format!("请求没给 out：{scene} 这一张图存哪儿？"));
            }
            Ok(vec![Step {
                scene: scene.clone(),
                out: request.out.clone(),
                views: views_of(request, request.view.cam),
            }])
        }
        Scene::Sequence { shots } => {
            if shots.is_empty() {
                return Err("批量请求一步都没有".to_string());
            }
            let mut steps = Vec::with_capacity(shots.len());
            for (index, shot) in shots.iter().enumerate() {
                if shot.out.trim().is_empty() {
                    return Err(format!(
                        "批量请求的第 {} 步没给 out：{} 要存哪儿？",
                        index + 1,
                        shot.scene
                    ));
                }
                steps.push(Step {
                    scene: shot.scene.clone(),
                    out: shot.out.clone(),
                    // 这一步给了相机就用它的，没给就沿用请求上那一档（Bevy 的 `ActiveJob::steps`）。
                    views: views_of(request, shot.cam.or(request.view.cam)),
                });
            }
            Ok(steps)
        }
        Scene::World { .. } => Err(WORLD_REFUSAL.to_string()),
    }
}

/// 这一次请求**怎么看**：`--sheet` 那一档优先（它本来就是"用产物自带的相机表"，
/// 所以两步都不会带 `cam` —— 客户端那一侧已经在本地拒了 `--cam` + `--sheet`）。
///
/// ⚠ `columns` 在这里**不兜底**：0 交给 `render::placements` 按 Bevy 同一条 `.max(1)` 处理，
/// 两处各兜一次就是"同一个数两个来源"。
fn views_of(request: &Request, cam: Option<[f32; 3]>) -> render::Views {
    if request.view.sheet {
        render::Views::Sheet {
            columns: request.view.columns,
        }
    } else {
        render::Views::Single(cam)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use px_host_protocol::render::{Shot, View};

    fn request(scene: Scene, out: &str) -> Request {
        Request {
            scene,
            view: View::default(),
            width: 960,
            height: 640,
            out: out.to_string(),
            job: Job::Shots,
            report: String::new(),
        }
    }

    /// 批量：每一步各自给 `out`，没给相机的沿用请求上那一档。
    #[test]
    fn a_batch_keeps_step_order_and_inherits_the_camera() {
        let mut request = request(
            Scene::Sequence {
                shots: vec![
                    Shot {
                        scene: "a.pxart".to_string(),
                        out: "a.png".to_string(),
                        cam: Some([1.0, 2.0, 3.0]),
                    },
                    Shot {
                        scene: "b.pxart".to_string(),
                        out: "b.png".to_string(),
                        cam: None,
                    },
                ],
            },
            "ignored.png",
        );
        request.view.cam = Some([9.0, 9.0, 9.0]);
        let steps = steps_of(&request).expect("批量请求应当成立");
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].scene, "a.pxart");
        assert_eq!(steps[1].out, "b.png");
        assert_eq!(
            steps[0].views,
            render::Views::Single(Some([1.0, 2.0, 3.0])),
            "这一步自己的相机"
        );
        assert_eq!(
            steps[1].views,
            render::Views::Single(Some([9.0, 9.0, 9.0])),
            "没给就沿用请求上那一档"
        );
    }

    /// `--sheet` 从**拒词**变成了**一档怎么看**：它落到每一步上是 `Views::Sheet`，
    /// 而且**吞掉 `cam`** —— 产物自带的相机表与 `--cam` 是两处会漂开的真相
    /// （客户端那一侧已经在本地拒了同时给两个，服务端这一侧不能"cam 赢"或者"sheet 赢"）。
    #[test]
    fn a_sheet_request_becomes_the_artifact_camera_table() {
        let sheet = Request {
            view: View {
                sheet: true,
                columns: 4,
                cam: Some([1.0, 2.0, 3.0]),
            },
            ..request(
                Scene::Artifact {
                    scene: "a.pxart".to_string(),
                },
                "a.png",
            )
        };
        let steps = steps_of(&sheet).expect("对照图这一档已经落地，不该再拒");
        assert_eq!(
            steps[0].views,
            render::Views::Sheet { columns: 4 },
            "对照图这一步是「按产物自带的相机表排格子」"
        );
    }

    /// 能力拒词钉的是"理由对不对"，不是"有没有拒"：性能那两路要说的必须是
    /// "需要帧循环"、并且指路 S7。
    ///
    /// ⚠ S8-a 补一条：拒词**不许再指路一个已经不存在的宿主**。它原来收在
    /// "要计时读数请走 Bevy 宿主" —— 而 `px_render` 已经删了，那句话会把人指进空处。
    /// 现在它指向本宿主真有的那件仪器（`--spans`），这条断言就是不让死指针长回来。
    #[test]
    fn the_capability_refusals_name_the_real_reason() {
        let perf = Request {
            job: Job::Stable { frames: 60 },
            ..request(
                Scene::Artifact {
                    scene: "a.pxart".to_string(),
                },
                "a.png",
            )
        };
        let why = steps_of(&perf).unwrap_err();
        assert!(why.contains("帧循环"), "拒词要说对理由：{why}");
        assert!(why.contains("S7"), "而且要指路：{why}");
        assert!(
            !why.contains("Bevy 宿主"),
            "拒词不许指路一个已经删掉的宿主（S8-a）：{why}"
        );
        assert!(
            why.contains("--spans"),
            "要指路到本宿主**真有的**那件计时仪器 —— 而且要说清它量的是别的量：{why}"
        );
    }

    /// 一次都没有的批量、少 `out` 的一步、0 尺寸：三条都是**当场拒**，不是"画一半"。
    #[test]
    fn malformed_batches_are_refused_before_anything_is_drawn() {
        let empty = request(Scene::Sequence { shots: Vec::new() }, "a.png");
        assert!(steps_of(&empty).unwrap_err().contains("一步都没有"));

        let no_out = request(
            Scene::Sequence {
                shots: vec![
                    Shot {
                        scene: "a.pxart".to_string(),
                        out: "a.png".to_string(),
                        cam: None,
                    },
                    Shot {
                        scene: "b.pxart".to_string(),
                        out: String::new(),
                        cam: None,
                    },
                ],
            },
            "a.png",
        );
        let why = steps_of(&no_out).unwrap_err();
        assert!(why.contains("第 2 步") && why.contains("b.pxart"), "{why}");

        let zero = Request {
            width: 0,
            ..request(
                Scene::Artifact {
                    scene: "a.pxart".to_string(),
                },
                "a.png",
            )
        };
        assert!(steps_of(&zero).unwrap_err().contains("大于 0"));
    }
}
