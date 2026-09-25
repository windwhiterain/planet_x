use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use px_protocol::ProtocolId;
use px_protocol::client;
use px_protocol::frame::{self, Frame};
use px_protocol::render::{Job, Lease, Report, Request, Response, Scene, ShotReport};

use crate::digest;
use crate::gpu::Gpu;
use crate::render;
use crate::report;
use crate::shot;

const LEASE_CHECK_INTERVAL: Duration = Duration::from_secs(2);

const IO_TIMEOUT: Duration = Duration::from_secs(300);

pub const WORLD_REFUSAL: &str = "经济世界（`Scene::World` / `--stream`）那一路没有搬到这个宿主：\
     它画的是 sim 的世界视图，不是渲染文档（`.pxart`）";

pub struct Server {
    gpu: Gpu,
    pcg_root: PathBuf,
}

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
    println!("渲染管线全部就绪：本宿主是**同步**建管线（没有队列可等），此刻待编 0 条、失败 0 条");
    println!(
        "尺寸以请求里的为准（--width/--height 只在别的入口有意义，这里只收不用）：{width}×{height}"
    );

    spawn_lease_watch(lease_path, lease.pid);

    let mut server = Server { gpu, pcg_root };
    for connection in listener.incoming() {
        let Ok(stream) = connection else {
            continue;
        };
        serve_connection(&mut server, stream);
    }
    Ok(())
}

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

    let frame = match server.execute(&request) {
        Ok(response) => Frame::Response(response),
        Err(reason) => {
            eprintln!("拒绝任务：{reason}");
            Frame::Refused(reason)
        }
    };
    let _ = frame::write_frame(&mut stream, &frame);
}

#[derive(Debug)]
struct Step {
    scene: String,
    out: String,
    views: render::Views,
}

impl Server {
    fn execute(&mut self, request: &Request) -> Result<Response, String> {
        let started = Instant::now();
        let steps = steps_of(request)?;

        let mut shot_reports: Vec<ShotReport> = Vec::with_capacity(steps.len());
        let mut shots: Vec<String> = Vec::with_capacity(steps.len());
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
            println!("  执行了：{}", crate::executed_summary(&rendered.executed));
            for (skipped, why) in &rendered.skipped {
                println!("⚠ 没有执行 '{skipped}'：{why}");
            }

            shots.push(out.display().to_string());
            shot_reports.push(ShotReport {
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

        let protocol = ProtocolId::local();
        let report = Report {
            schema_version: protocol.schema_version,
            protocol_hash: format!("{:016x}", protocol.protocol_hash),
            job: request.job.name().to_string(),
            width: last.1,
            height: last.2,
            millis: started.elapsed().as_millis() as u64,
            shots: shot_reports,
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
            warm: true,
            shots,
            report: text,
            report_path: request.report.clone(),
        })
    }
}

fn steps_of(request: &Request) -> Result<Vec<Step>, String> {
    if !matches!(request.job, Job::Shots) {
        return Err(format!(
            "`{}` 那一路不在这一版：它要的是**计时用的帧循环**（逐帧采样 / 丢窗 / 等 K 帧 \
             + 每条 pass 的编码器级 GPU 时间戳），而本宿主现在**按需渲染** —— \
             预览窗口（S7 前半）那个循环只在画面变了时画一帧，给不出逐帧序列、也没有那七段 span。\
             报告里的 `gpu_ms` / `pair` 因此没有可比对象（逐条 pass 的编码器级时间戳，\
             与「按需画 N 次」不是同一个量）。要计时读数请走 `--spans 预热,测量`：\
             它量的是**逐条 pass** 的编码器级时间戳—— \
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
                    views: views_of(request, shot.cam.or(request.view.cam)),
                });
            }
            Ok(steps)
        }
        Scene::World { .. } => Err(WORLD_REFUSAL.to_string()),
    }
}

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
    use px_protocol::render::{Shot, View};

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
