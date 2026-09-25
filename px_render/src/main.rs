//! See docs/renderer.md

mod art;
mod camera;
mod client;
mod cook;
mod diff;
mod digest;
mod edit;
mod gpu;
mod group0;
mod icosphere;
mod mat4;
mod material;
mod mesh;
mod panel;
mod plan;
mod render;
mod report;
mod serve;
mod shader;
mod shot;
mod spans;
mod stubs;
mod vec;
mod viewer;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use px_protocol::render::{Job, Request, Scene, Shot, View};

fn usage() -> String {
    [
        "用法：",
        "  px_render --serve [--port N] [--pcg-root DIR] [--width W] [--height H]",
        "      常驻渲染服务：写租约（target/render-server.json）、按请求出图。",
        "      ⚠ 尺寸以**请求里的**为准（--width/--height 只收不用）。",
        "  px_render --scene A.pxart --out a.png [--scene B.pxart --out b.png]",
        "            [--cam YAW,PITCH,DIST] [--report r.json] [--width W] [--height H] [--autostart]",
        "      客户端：把一条请求交给在跑的服务。给了多个 --scene 就是**批量**（一个请求出一串图）。",
        "      --report 让服务端把结构化报告落到那个路径（同一份也回给调用方）。",
        "  px_render --scene 文档.pxart --out PNG [--width W] [--height H] [--offline] [--stats]",
        "      **离线**出图（要显式写 --offline）：不开服务、不走协议，直接用本进程的设备画一帧。",
        "      ⚠ 为什么它不是缺省：`--scene --out` 在 bevy 宿主那里的语义是**请求**（交给服务），",
        "        而 `tools/` 那套仪器（frame-probe / harness）就是这么调本 exe 的。",
        "        两套语义共用一个写法，等于让'这张图是谁画的'变成一条要靠猜的事。",
        "      --stats：报回读字节的逐通道 min/max 与颜色数（平场那种判据靠它）。",
        "      --image-hash（--view）：窗口每帧把回读出来那张图的 sha16 与墙钟时刻打进日志。",
        "        它服务的是 S7 那条「改一个 .wgsl ⇒ 约 1 秒内**画面**变」：判据的尺子是图，不是日志。",
        "      --time N：**保留态的读数** —— 准备一次、连画 N 帧，报「准备 / 第 1 帧（含建管线）/",
        "        第 2..N 帧（中位）」三段墙钟；落盘的是最后一帧（与不带 --time 的那张逐字节相同）。",
        "      --spans 预热,测量：**逐条 pass 的 GPU 编码器级时间戳**（J4 的仪器）。",
        "        一条保留态会话 + 预热帧 + 测量帧；每条 pass 报两套边界（包络 / pass 内 ——",
        "        后者与 Bevy 的 `elapsed_gpu` 同一套），另有帧级一条与匹配子集之和。",
        "        ⚠ 两个数都必须显式写（不给缺省）；⚠ 与 --sheet 互斥（槽按单张排）。",
        "        ⚠ 这个数**不叫** `gpu_ms`：那个名字在 Bevy 那边是七段之和（见 src/spans.rs）。",
        "  px_render --scene 文档.pxart --out sheet.png --sheet [--columns N] [--offline]",
        "      **对照图**（J2）：12 格 × 960×640 拼成一张 3840×1920 —— 相机表来自产物",
        "      （`.pxart` 的 `cameras`），格子的排布是渲染器的事（缺省 4 列）。",
        "      ⚠ 与 --cam 互斥：相机表与 --cam 是两处会漂开的真相。",
        "  px_render --diff A.png B.png",
        "      两张 PNG 逐像素比（不要 GPU）：差异像素数 / 最大与平均通道差 / 差异区域 /",
        "      剪影内的像素是不是逐位相同。",
        "      ⚠ **按位置读，不按角色读**：背景色与剪影**只按第一个参数（左图）**定，",
        "        约定是『左 = 本宿主那张 / 右 = oracle』。传反了整篇读数就反着读 ——",
        "        本工具不认识角色，所以报告开头会把两条路径连同左右一起打出来。",
        "  px_render --device [--shot PNG] [--width W] [--height H]",
        "      建实例/适配器/设备（Vulkan 锁死），报「到设备就绪」的读数；",
        "      给了 --shot 就再清一张纯色图、回读、落 PNG。",
        "  px_render --shaders",
        "      把四份内容 shader 用**本宿主的桩表**组装出来并 naga 校验（不要 GPU）。",
        "  px_render --view [--scene 文档.pxart] [--cam YAW,PITCH,DIST] [--shot PNG]",
        "                   [--width W] [--height H] [--novsync] [--edit <场景配方名>]",
        "      **常驻预览窗口**（S7）：1 台轨道相机（左键拖 = 转、滚轮 = 缩放），",
        "      相机一变、场景一换、窗口一改大小才重画一帧（本宿主是同步建管线，按需渲染）。",
        "      --shot：**开窗之后的第一帧**顺手存一张 PNG（与 `--offline` 同一行代码）。",
        "      --cam：窗口的起始方位；不给就是**不给 --cam 那一档**（探针机位，与 J1 那张图同一台）。",
        "      --width/--height：窗口的初始尺寸（离线那条路是图的尺寸，同一个口径）。",
        "      --edit：窗口里那块**调参面板**（S9）编辑的是**哪份场景配方**引用到的图。",
        "        不给就按**窗口正在显示的那份产物名**推（产物名 = 配方名）；推不出来时面板",
        "        只说「给 --edit 哪个名」。改的是参数，烘图走 `px run`（子进程），画面随即重载；",
        "        编辑落在会话副本 `target/pcg/edit/` 上，`art/` 要按面板里的 Save 才动。",
        "        Tab / F1 收起·展开面板。⚠ 只有 `--view` 有它（面板住在窗口里）。",
        "      --ui-shot PNG：把**屏幕上那一张**（画面 + 面板）读回来存成 PNG（窗口里按 `u` 也行）。",
        "        ⚠ 它与 `--shot` 是**两件事**：`--shot` 写的是判据那张图（**不含**面板），",
        "          这一格写的是人眼看到的东西 —— 它是「面板长什么样」的证据。",
        "        ⚠ 没给 `--width/--height` 时窗口按**逻辑尺寸**开（960×640 pt ⇒ 本机 1680×1120 px，",
        "          缩放因子 1.75）：egui 量的都是逻辑点，拿 960 当物理像素的话面板会显得吃掉半个窗口。",
        "          给了 `--width/--height` 就照旧是**那张图的像素**（判据那条路要的意思）。",
        "  px_render --show --scene 文档.pxart [--shot PNG]",
        "      把一份场景**推给**在跑的窗口（写 target/viewer-scene.json）；它自己不渲染。",
        "  px_render --where ｜ px_render --place YAW,PITCH,DIST",
        "      问 / 摆**常驻窗口**的相机（一问一答，3 s 超时；不换场景、不重烘）。",
        "      ⚠ (yaw,pitch,distance) 的含义与 `--cam` **同一套数**（同一个 `probe_camera`）：",
        "        窗口在某个方位看到的，就是 `--offline --cam 同一个三元组` 画出来的那一张。",
        "  px_render --help",
        "",
        "⚠ 这一版没有的（各自都会**当场拒**，而且拒词指路）：",
        "  --perf/--windows/--frames",
        "                     性能那两路要的是**计时用的帧循环**（逐帧采样 / 丢窗 / 等 K 帧 +",
        "                     逐段 GPU 时间戳），viewer 那个是按需渲染的交互循环，不是它",
        "  shader 热重载      预览窗口里改一个 .wgsl 存盘、1 秒内画面变 —— S7 的后半",
    ]
    .join("\n")
}

struct ShotArgs {
    scene: PathBuf,
    out: Option<PathBuf>,
    cam: Option<[f32; 3]>,
}

struct Options {
    device: bool,
    shaders: bool,
    shot: Option<PathBuf>,
    diff: Option<(PathBuf, PathBuf)>,
    offline: bool,
    stats: bool,
    time: u32,
    spans: Option<(u32, u32, u32)>,
    image_hash: bool,
    serve: bool,
    port: u16,
    pcg_root: PathBuf,
    pcg_root_given: bool,
    fps: bool,
    novsync: bool,
    shots: Vec<ShotArgs>,
    out: Option<PathBuf>,
    cam: Option<[f32; 3]>,
    width: u32,
    height: u32,
    report: String,
    sheet: bool,
    columns: u32,
    perf: bool,
    view: bool,
    show: bool,
    ask_where: bool,
    place: Option<[f32; 3]>,
    edit: Option<String>,
    ui_shot: Option<PathBuf>,
    windows: u32,
    windows_given: bool,
    drop_windows: u32,
    frames: u32,
    stream: PathBuf,
    round: Option<u32>,
    autostart: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            device: false,
            shaders: false,
            shot: None,
            diff: None,
            offline: false,
            stats: false,
            time: 0,
            spans: None,
            image_hash: false,
            serve: false,
            port: 0,
            pcg_root: art::default_pcg_root(),
            pcg_root_given: false,
            fps: false,
            novsync: false,
            shots: Vec::new(),
            out: None,
            cam: None,
            width: 960,
            height: 640,
            report: String::new(),
            sheet: false,
            columns: 4,
            perf: false,
            view: false,
            show: false,
            ask_where: false,
            place: None,
            edit: None,
            ui_shot: None,
            windows: 4,
            windows_given: false,
            drop_windows: 1,
            frames: 60,
            stream: PathBuf::from("target/world.pxstream"),
            round: None,
            autostart: false,
        }
    }
}

fn parse_cam(flag: &str, text: &str) -> Result<[f32; 3], String> {
    let parts: Vec<f32> = text
        .split(',')
        .map(|part| part.trim().parse::<f32>())
        .collect::<Result<_, _>>()
        .map_err(|_| format!("{flag} 要 yaw,pitch,dist 三个数"))?;
    if parts.len() != 3 {
        return Err(format!("{flag} 要 yaw,pitch,dist 三个数"));
    }
    Ok([parts[0], parts[1], parts[2]])
}

fn parse_spans(text: &str) -> Result<(u32, u32, u32), String> {
    let parts: Vec<&str> = text.split(',').map(str::trim).collect();
    let [warm, measured, rounds] = parts.as_slice() else {
        return Err(
            "--spans 要 `预热,测量,轮数` 三个数（例如 --spans 8,24,4）：\
                    ⚠ 三个都**必须**写 —— 不给缺省，因为「预热几帧、交错几轮」是这个读数的一部分，\
                    而轮数还是整批代价的一部分"
                .to_string(),
        );
    };
    let warm: u32 = warm
        .parse()
        .map_err(|_| format!("--spans 的预热帧 '{warm}' 不是个非负整数"))?;
    let measured: u32 = measured
        .parse()
        .map_err(|_| format!("--spans 的测量帧 '{measured}' 不是个正整数"))?;
    let rounds: u32 = rounds
        .parse()
        .map_err(|_| format!("--spans 的轮数 '{rounds}' 不是个正整数"))?;
    if measured == 0 {
        return Err("--spans 的测量帧是 0：那一位必须是正的（预热帧不进读数）".to_string());
    }
    if rounds == 0 {
        return Err("--spans 的轮数是 0：那一位必须是正的".to_string());
    }
    Ok((warm, measured, rounds))
}

impl Options {
    fn parse() -> Result<Self, String> {
        let mut options = Self::default();
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            let mut next = |what: &str| -> Result<String, String> {
                args.next().ok_or_else(|| format!("{what} 后面要跟一个值"))
            };
            match arg.as_str() {
                "--device" => options.device = true,
                "--shaders" => options.shaders = true,
                "--offline" => options.offline = true,
                "--stats" => options.stats = true,
                "--image-hash" => options.image_hash = true,
                "--time" => {
                    options.time = next("--time")?
                        .parse()
                        .map_err(|_| "--time 需要一个整数（连画几帧）".to_string())?
                }
                "--spans" => options.spans = Some(parse_spans(&next("--spans")?)?),
                "--serve" => options.serve = true,
                "--autostart" => options.autostart = true,
                "--fps" => options.fps = true,
                "--novsync" => options.novsync = true,
                "--sheet" => options.sheet = true,
                "--shot" => options.shot = Some(PathBuf::from(next("--shot")?)),
                "--diff" => {
                    let left = PathBuf::from(next("--diff")?);
                    let right = PathBuf::from(next("--diff（第二张）")?);
                    options.diff = Some((left, right));
                }
                "--scene" => options.shots.push(ShotArgs {
                    scene: PathBuf::from(next("--scene")?),
                    out: None,
                    cam: None,
                }),
                "--out" => {
                    let out = PathBuf::from(next("--out")?);
                    match options.shots.last_mut() {
                        Some(shot) => shot.out = Some(out),
                        None => options.out = Some(out),
                    }
                }
                "--cam" => {
                    let cam = parse_cam("--cam", &next("--cam")?)?;
                    match options.shots.last_mut() {
                        Some(shot) => shot.cam = Some(cam),
                        None => options.cam = Some(cam),
                    }
                }
                "--report" => options.report = next("--report")?,
                "--pcg-root" => {
                    options.pcg_root = PathBuf::from(next("--pcg-root")?);
                    options.pcg_root_given = true;
                }
                "--port" => {
                    options.port = next("--port")?
                        .parse()
                        .map_err(|_| "--port 需要一个整数".to_string())?
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
                "--columns" => {
                    options.columns = next("--columns")?
                        .parse()
                        .map_err(|_| "--columns 需要一个整数".to_string())?
                }
                "--shots" => options.perf = false,
                "--perf" => options.perf = true,
                "--windows" => {
                    options.windows = next("--windows")?
                        .parse()
                        .map_err(|_| "--windows 需要一个整数".to_string())?;
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
                "--stream" => options.stream = PathBuf::from(next("--stream")?),
                "--round" => {
                    options.round = Some(
                        next("--round")?
                            .parse()
                            .map_err(|_| "--round 需要一个整数".to_string())?,
                    )
                }
                "--view" => options.view = true,
                "--show" => options.show = true,
                "--where" => options.ask_where = true,
                "--place" => options.place = Some(parse_cam("--place", &next("--place")?)?),
                "--edit" => {
                    let recipe = next("--edit")?;
                    if recipe.is_empty() {
                        return Err("--edit 要一个场景配方名（--edit orbit-bare）".to_string());
                    }
                    options.edit = Some(recipe);
                }
                "--ui-shot" => options.ui_shot = Some(PathBuf::from(next("--ui-shot")?)),
                "--help" | "-h" => {
                    println!("{}", usage());
                    std::process::exit(0);
                }
                other => return Err(format!("不认识的参数 '{other}'\n{}", usage())),
            }
        }
        if options.width == 0 || options.height == 0 {
            return Err("尺寸里有 0".to_string());
        }
        if options.sheet && (options.cam.is_some() || options.shots.iter().any(|s| s.cam.is_some()))
        {
            return Err("--sheet 用的是产物自带的相机表，不要再给 --cam".to_string());
        }
        options.check_viewer()?;
        Ok(options)
    }

    fn check_viewer(&self) -> Result<(), String> {
        let asked = [self.view, self.show, self.ask_where, self.place.is_some()]
            .iter()
            .filter(|flag| **flag)
            .count();
        if asked == 0 {
            return Ok(());
        }
        if self.serve {
            return Err(
                "--serve（渲染服务）与 --view/--show/--where/--place（预览窗口）是**两条常驻路**：\
                 一个进程只能当一个（各自的租约文件也不同：target/render-server.json 与 target/viewer.json）"
                    .to_string(),
            );
        }
        if self.offline {
            return Err(
                "--offline 是「本进程画一帧就退出」，与常驻窗口那几路（--view/--show/--where/--place）不相干"
                    .to_string(),
            );
        }
        if asked > 1 {
            return Err(format!(
                "--view / --show / --where / --place 一次只能走一条（这次给了 {asked} 条）：\
                 起窗口、推场景、问相机、摆相机是四件事"
            ));
        }
        if self.show {
            if self.shots.is_empty() {
                return Err("--show 需要一份场景产物：--scene <SCENE.pxart>".to_string());
            }
        } else if self.view {
            if self.shots.len() > 1 {
                return Err(
                    "--view 只要**一份**起始场景（窗口是常驻的，其余场景用 --show 推）：\
                     一次只能显示一份文档"
                        .to_string(),
                );
            }
        } else if self.shots.iter().any(|shot| shot.out.is_some()) || self.out.is_some() {
            return Err(
                "--where/--place 只跟常驻窗口的相机说话，不渲染任何东西：--out 在这儿没有意义"
                    .to_string(),
            );
        }
        if self.edit.is_some() && !self.view {
            return Err("--edit（调参面板）住在预览窗口里，得与 --view 一起给：\
                 px_render --view --edit <场景配方名>"
                .to_string());
        }
        Ok(())
    }

    fn view_cam(&self) -> Option<[f32; 3]> {
        match self.shots.as_slice() {
            [shot] => shot.cam.or(self.cam),
            _ => self.cam,
        }
    }

    fn out_path(&self) -> PathBuf {
        self.shots
            .last()
            .and_then(|shot| shot.out.clone())
            .or_else(|| self.out.clone())
            .unwrap_or_else(|| PathBuf::from("target/shot.png"))
    }

    fn job(&self) -> Job {
        if !self.perf {
            return Job::Shots;
        }
        if self.windows_given {
            Job::Perf {
                windows: self.windows,
                drop: self.drop_windows,
            }
        } else {
            Job::Stable {
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
        let outs_needed = !matches!(self.job(), Job::Stable { .. });
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
            shots.push(Shot {
                scene: shot.scene.display().to_string(),
                out,
                cam: shot.cam,
            });
        }
        Ok(Scene::Sequence { shots })
    }

    fn request(&self) -> Result<Request, String> {
        Ok(Request {
            scene: self.scene()?,
            view: View {
                cam: self.view_cam(),
                sheet: self.sheet,
                columns: self.columns,
            },
            width: self.width,
            height: self.height,
            out: self.out_path().display().to_string(),
            job: self.job(),
            report: self.report.clone(),
        })
    }
}

fn check_content_shaders() -> i32 {
    let modules = shader::modules();
    let mut failed = 0;
    for name in shader::CONTENT_SHADERS {
        let (source, path) = shader::source_of(name);
        let assembled = shader::assemble(&source, &modules, stubs::stubs);
        match shader::validate(name, &assembled) {
            Ok(module) => {
                let rows = shader::bindings(&module);
                println!(
                    "✓ {name}｜{}｜组装后 {} 字节｜全局绑定 {} 个",
                    path.display(),
                    assembled.len(),
                    rows.len()
                );
                for (group, binding, space, var) in rows {
                    println!("      @group({group}) @binding({binding}) {space} {var}");
                }
            }
            Err(message) => {
                eprintln!("✗ {message}");
                failed += 1;
            }
        }
    }
    if failed > 0 {
        eprintln!("四份内容 shader 里有 {failed} 份组装/校验不过");
        return 1;
    }
    println!("四份内容 shader 全部组装并校验通过（桩表 = 本宿主的 group 0 契约）");
    0
}

fn executed_summary(executed: &[String]) -> String {
    const HEAD: usize = 8;
    let total = executed.len();
    if std::env::var_os("PX_PASS_LIST").is_some() || total <= HEAD {
        return executed.join(" → ");
    }
    format!(
        "{} → …（共 {total} 条；要看全表设 PX_PASS_LIST=1）",
        executed[..HEAD].join(" → ")
    )
}

fn run_scene(
    scene: &Path,
    out: &Path,
    width: u32,
    height: u32,
    stats: bool,
    time: u32,
    views: render::Views,
) -> i32 {
    let gpu = gpu::connect();
    let rendered = if time == 0 {
        render::run(&gpu, scene, &art::default_pcg_root(), views, width, height)
    } else {
        run_frames(&gpu, scene, views, width, height, time)
    };
    let rendered = match rendered {
        Ok(rendered) => rendered,
        Err(message) => {
            eprintln!("渲染失败：{message}");
            return 1;
        }
    };
    for line in &rendered.audit {
        println!("{line}");
    }
    let width = rendered.width;
    let height = rendered.height;
    let first = [
        rendered.pixels[0],
        rendered.pixels[1],
        rendered.pixels[2],
        rendered.pixels[3],
    ];
    let stats_text = if stats {
        Some(describe(&rendered.pixels))
    } else {
        None
    };
    let bytes = match shot::write_png(out, width, height, rendered.pixels) {
        Ok(bytes) => bytes,
        Err(message) => {
            eprintln!("{message}");
            return 1;
        }
    };
    println!(
        "写出：{} → {}（{}×{}，{} 字节，sha256 {}）",
        scene.display(),
        out.display(),
        width,
        height,
        bytes,
        digest::short(out)
    );
    println!(
        "首像素：R={} G={} B={} A={}",
        first[0], first[1], first[2], first[3]
    );
    if let Some(text) = stats_text {
        println!("{text}");
    }
    println!("执行了：{}", executed_summary(&rendered.executed));
    for (label, why) in &rendered.skipped {
        println!("⚠ 没有执行 '{label}'：{why}");
    }
    println!("（离线：不碰交换链，渲染目标是本进程自己的 Rgba8UnormSrgb）");
    0
}

fn describe(pixels: &[u8]) -> String {
    let mut low = [255_u8; 4];
    let mut high = [0_u8; 4];
    let mut counts: BTreeMap<[u8; 4], usize> = BTreeMap::new();
    for chunk in pixels.chunks_exact(4) {
        let texel = [chunk[0], chunk[1], chunk[2], chunk[3]];
        for channel in 0..4 {
            low[channel] = low[channel].min(texel[channel]);
            high[channel] = high[channel].max(texel[channel]);
        }
        *counts.entry(texel).or_insert(0) += 1;
    }
    let (mode, count) = counts
        .iter()
        .max_by_key(|(color, count)| (**count, std::cmp::Reverse(**color)))
        .map(|(color, count)| (*color, *count))
        .unwrap_or(([0; 4], 0));
    format!(
        "读数：{} 个像素｜逐通道 min R={} G={} B={} A={}｜max R={} G={} B={} A={}\
         ｜不同颜色 {} 种｜众数 R={} G={} B={} A={}（{} 个像素，{:.2}%）",
        pixels.len() / 4,
        low[0],
        low[1],
        low[2],
        low[3],
        high[0],
        high[1],
        high[2],
        high[3],
        counts.len(),
        mode[0],
        mode[1],
        mode[2],
        mode[3],
        count,
        if pixels.is_empty() {
            0.0
        } else {
            100.0 * count as f64 / (pixels.len() / 4) as f64
        }
    )
}

fn run_diff(left: &Path, right: &Path) -> i32 {
    match diff::compare(left, right) {
        Ok(report) => {
            println!("比对（左 / 右 = 第一个 / 第二个参数）：");
            println!("{}", report.report());
            0
        }
        Err(message) => {
            eprintln!("比不了：{message}");
            1
        }
    }
}

pub use serve::WORLD_REFUSAL;

fn main() {
    let options = match Options::parse() {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(64);
        }
    };

    if options.shaders {
        std::process::exit(check_content_shaders());
    }

    if let Some((left, right)) = &options.diff {
        std::process::exit(run_diff(left, right));
    }

    if options.view {
        if let Err(message) = viewer::view(&options) {
            eprintln!("{message}");
            std::process::exit(1);
        }
        return;
    }
    if options.image_hash {
        eprintln!(
            "--image-hash 是预览窗口（--view）的读数：它每帧把回读出来那批字节的 sha16 与墙钟打出来，\
             服务与离线那两条路没有「每帧」可报"
        );
        std::process::exit(64);
    }
    if options.show {
        let scene = options.shots[0].scene.clone();
        if let Err(message) = viewer::show(&scene, options.shot.clone()) {
            eprintln!("{message}");
            std::process::exit(1);
        }
        return;
    }
    if options.ask_where || options.place.is_some() {
        if let Err(message) = viewer::camera_query(options.place) {
            eprintln!("{message}");
            std::process::exit(1);
        }
        return;
    }

    if options.serve {
        if options.fps || options.novsync {
            println!(
                "（--fps/--novsync 在**服务**这条路上收下但不生效：服务按需渲染，没有帧循环、也不碰交换链。\
                 窗口那条路（--view）上 --novsync 是真的（交换链的 present mode））"
            );
        }
        if let Err(message) = serve::serve(
            options.port,
            options.pcg_root.clone(),
            options.width,
            options.height,
        ) {
            eprintln!("{message}");
            std::process::exit(1);
        }
        return;
    }

    if options.pcg_root_given {
        eprintln!(
            "⚠ --pcg-root 只在渲染进程（--serve）生效：这次请求的内容由服务进程按它自己的 CAS 根解析"
        );
    }

    if options.device || options.shot.is_some() {
        std::process::exit(run_device(&options));
    }

    if options.offline {
        std::process::exit(run_offline(&options));
    }

    if options.stats {
        eprintln!("--stats 是离线那条路（--offline）的读数；走服务那条路的同类读数在 --report 里");
        std::process::exit(64);
    }

    if options.time > 0 {
        eprintln!(
            "--time 是离线那条路（--offline）的读数：它量的是「准备一次、连画 N 帧」，而服务那条路一条请求只画一帧"
        );
        std::process::exit(64);
    }

    if options.spans.is_some() {
        eprintln!(
            "--spans 是离线那条路（--offline）的仪器：它量的是本进程那些编码器上的 GPU 时间戳，而服务那条路一条请求只画一帧"
        );
        std::process::exit(64);
    }

    if options.shots.is_empty() {
        eprintln!("{WORLD_REFUSAL}");
        std::process::exit(64);
    }

    let request = match options.request() {
        Ok(request) => request,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(64);
        }
    };
    std::process::exit(client::request(request, options.autostart));
}

fn run_offline(options: &Options) -> i32 {
    if !options.report.is_empty() {
        eprintln!(
            "--report 是**服务端**产出的（sha256 / 网格差分 / 兜底像素数都由它算）；离线那条路要读数就加 --stats"
        );
        return 64;
    }
    if options.perf || options.windows_given {
        eprintln!(
            "--perf/--windows 要的是**计时用的帧循环**（服务那条路的活），而离线这条路一次只画一帧"
        );
        return 64;
    }
    if options.spans.is_some() && options.sheet {
        eprintln!(
            "--spans 与 --sheet 不能同时用：时间戳槽是按**单张**排的（每条 pass 四格 + 帧级两格），\
             12 格会往同一批格上写 12 遍，读到的只是其中一遍。\
             要对对照图计时，请把 12 格拆成 12 次单张（那也是 J2 判据量过的事）"
        );
        return 64;
    }
    if let Some((warm, measured, rounds)) = options.spans {
        return run_spans(options, warm, measured, rounds, views_of(options));
    }
    let [shot] = options.shots.as_slice() else {
        eprintln!(
            "离线那条路一次只画**一份**文档（给了 {} 份）：批量是请求的概念，交给服务去做。\
             ⚠ 例外是 `--spans`：那条路**要**几份文档（交错取样是那台仪器的一部分）",
            options.shots.len()
        );
        return 64;
    };
    let Some(out) = shot.out.as_ref().or(options.out.as_ref()) else {
        eprintln!("给了 --scene 却没给 --out：图写到哪里去？\n{}", usage());
        return 64;
    };
    run_scene(
        &shot.scene,
        out,
        options.width,
        options.height,
        options.stats,
        options.time,
        views_of(options),
    )
}
fn run_frames(
    gpu: &gpu::Gpu,
    scene: &Path,
    views: render::Views,
    width: u32,
    height: u32,
    frames: u32,
) -> Result<render::Rendered, String> {
    use std::time::Instant;
    let started = Instant::now();
    let mut session =
        render::Session::open(gpu, scene, &art::default_pcg_root(), views, width, height)?;
    println!(
        "[计时] 准备 = {:.1} ms（文档 → CAS 成员 → 计划 → 句柄 → 第一层）",
        started.elapsed().as_secs_f64() * 1e3
    );
    let mut drawn: Vec<f64> = Vec::with_capacity(frames as usize);
    let mut last: Option<render::Rendered> = None;
    for index in 0..frames {
        let started = Instant::now();
        let rendered = session.draw(gpu, views, width, height)?;
        let ms = started.elapsed().as_secs_f64() * 1e3;
        drawn.push(ms);
        if index == 0 {
            println!("[计时] 第 1 帧 = {ms:.1} ms（含执行器**第一次建管线**：缓存是空的）");
        }
        last = Some(rendered);
    }
    if let Some(rest) = drawn.get(1..).filter(|rest| !rest.is_empty()) {
        let mut sorted = rest.to_vec();
        sorted.sort_by(|one, two| one.partial_cmp(two).expect("墙钟不会是 NaN"));
        println!(
            "[计时] 第 2..{frames} 帧（n = {}）：中位 {:.1} ms｜最小 {:.1}｜最大 {:.1}",
            rest.len(),
            sorted[sorted.len() / 2],
            sorted[0],
            sorted[sorted.len() - 1]
        );
    }
    println!(
        "⚠ 落盘的是**最后一帧**那张图，而它必须与不带 --time 的那一次逐字节相同（同一条渲染路）"
    );
    last.ok_or_else(|| "--time 至少要 1 帧".to_string())
}

fn run_spans(
    options: &Options,
    warm: u32,
    measured: u32,
    rounds: u32,
    views: render::Views,
) -> i32 {
    use std::time::Instant;
    let gpu = gpu::connect();
    let missing = spans::missing_features(&gpu.adapter);
    if !missing.is_empty() {
        eprintln!(
            "这一台设备缺 {} ⇒ 逐条 pass 的 GPU 时间戳量不了。\
             ⚠ 不许退化成「只量包络」那种数：它与 Bevy 的 `elapsed_gpu` 不是同一套边界，\
             放进同一个字段比没有这个数坏（§147）",
            missing.join(" / ")
        );
        return 1;
    }
    let width = options.width;
    let height = options.height;
    let mut shots: Vec<(PathBuf, PathBuf)> = Vec::with_capacity(options.shots.len());
    for shot in &options.shots {
        match shot.out.as_ref().or(options.out.as_ref()) {
            Some(out) => shots.push((shot.scene.clone(), out.clone())),
            None => {
                eprintln!(
                    "`--spans` 那一档每一份 --scene 都要有 --out（这一份是 {}）：\
                     落盘那张图是判据的一部分 —— 它必须与不带 --spans 的那一次逐字节相同",
                    shot.scene.display()
                );
                return 64;
            }
        }
    }
    if shots.is_empty() {
        eprintln!("`--spans` 至少要一份 --scene");
        return 64;
    }

    println!(
        "[spans-plan] 文档 {} 份｜轮数 {rounds}｜每轮每档 {warm} 预热 + {measured} 测量帧｜\
         一共 {} 次 draw｜尺寸 {width}x{height}",
        shots.len(),
        rounds as usize * shots.len() * (warm + measured) as usize
    );
    let started = Instant::now();
    let mut sessions: Vec<(
        String,
        render::Session,
        spans::Recorder,
        Option<render::Rendered>,
    )> = Vec::with_capacity(shots.len());
    for (scene, _) in &shots {
        let opened = Instant::now();
        let session = match render::Session::open(
            &gpu,
            scene,
            &art::default_pcg_root(),
            views,
            width,
            height,
        ) {
            Ok(session) => session,
            Err(message) => {
                eprintln!("打开 {} 失败：{message}", scene.display());
                return 1;
            }
        };
        let labels = session.pass_kinds();
        let recorder =
            match spans::Recorder::new(&gpu.device, &gpu.queue, labels, rounds, warm, measured) {
                Ok(recorder) => recorder,
                Err(message) => {
                    eprintln!("{} 的时间戳槽建不出来：{message}", scene.display());
                    return 1;
                }
            };
        println!(
            "[spans-plan] 档 {}｜pass {} 条｜一帧 {} 格｜准备 {:.1} ms｜时间戳周期 {} ns",
            scene.display(),
            recorder.passes(),
            spans::FrameStamps::stride_of(recorder.passes()),
            opened.elapsed().as_secs_f64() * 1e3,
            gpu.queue.get_timestamp_period()
        );
        sessions.push((scene.display().to_string(), session, recorder, None));
    }
    let prepare_ms = started.elapsed().as_secs_f64() * 1e3;
    println!(
        "[spans-plan] 全部准备完 = {prepare_ms:.1} ms（{} 份文档，**每份只付一次**）",
        shots.len()
    );

    let draws_started = Instant::now();
    for round in 0..rounds {
        for (slot, (name, session, recorder, last)) in sessions.iter_mut().enumerate() {
            for index in 0..(warm + measured) {
                let frame = round * (warm + measured) + index;
                let stamps = recorder.stamps(frame);
                match session.draw_stamps(&gpu, views, width, height, &stamps) {
                    Ok(rendered) => {
                        recorder.note_calls(rendered.timestamp_calls);
                        *last = Some(rendered);
                    }
                    Err(message) => {
                        eprintln!("画 {name} 失败：{message}");
                        return 1;
                    }
                }
            }
            let _ = slot;
        }
    }
    let draw_ms = draws_started.elapsed().as_secs_f64() * 1e3;
    println!(
        "[spans-plan] 画完 = {draw_ms:.1} ms（{} 次 draw）｜整批墙钟 {:.1} ms",
        rounds as usize * sessions.len() * (warm + measured) as usize,
        started.elapsed().as_secs_f64() * 1e3
    );

    let mut failed = 0;
    for (slot, (name, _session, recorder, last)) in sessions.into_iter().enumerate() {
        let readings = match recorder.finish(&gpu.device, &gpu.queue) {
            Ok(readings) => readings,
            Err(message) => {
                eprintln!("{name} 的时间戳读数算不出来：{message}");
                failed += 1;
                continue;
            }
        };
        for reading in &readings {
            for line in reading.report(name.as_str(), width, height) {
                println!("{line}");
            }
        }
        let Some(rendered) = last else {
            eprintln!("{name} 一帧都没画出来");
            failed += 1;
            continue;
        };
        for line in &rendered.audit {
            println!("{line}");
        }
        let (_, out) = &shots[slot];
        match shot::write_png(out, rendered.width, rendered.height, rendered.pixels) {
            Ok(bytes) => {
                println!(
                    "写出：{} → {}（{}×{}，{} 字节，sha256 {}）",
                    shots[slot].0.display(),
                    out.display(),
                    rendered.width,
                    rendered.height,
                    bytes,
                    digest::short(out)
                );
            }
            Err(message) => {
                eprintln!("{message}");
                failed += 1;
            }
        }
    }
    println!(
        "⚠ 这个数**不是** Bevy 报告里的 `gpu_ms`（那个是 `render/**/elapsed_gpu` 七段之和）：\
         这里报的是本文档那些 pass 的编码器级 span，同名会让人以为它们是一回事（§147）"
    );
    if failed > 0 { 1 } else { 0 }
}

fn views_of(options: &Options) -> render::Views {
    if options.sheet {
        render::Views::Sheet {
            columns: options.columns,
        }
    } else {
        render::Views::Single(options.view_cam())
    }
}

fn run_device(options: &Options) -> i32 {
    let gpu = gpu::connect();
    let limits = gpu.adapter.limits();
    println!(
        "适配器能力：max_bind_groups={} max_texture_dimension_2d={} max_texture_array_layers={}",
        limits.max_bind_groups, limits.max_texture_dimension_2d, limits.max_texture_array_layers
    );
    if gpu
        .adapter
        .features()
        .contains(wgpu::Features::TIMESTAMP_QUERY)
    {
        println!("时间戳周期：{} ns", gpu.queue.get_timestamp_period());
    } else {
        println!("时间戳周期：（这一台不可用 ⇒ gpu_ms 将报 null，不做替代读数）");
    }

    let Some(path) = &options.shot else {
        return 0;
    };

    let color = wgpu::Color {
        r: 0.25,
        g: 0.5,
        b: 0.75,
        a: 1.0,
    };
    let target = shot::Target::new(&gpu.device, options.width, options.height);
    shot::clear(&gpu.device, &gpu.queue, &target, color);

    let pixels = match shot::read_back(&gpu.device, &gpu.queue, &target) {
        Ok(pixels) => pixels,
        Err(message) => {
            eprintln!("回读失败：{message}");
            return 1;
        }
    };
    let first = [pixels[0], pixels[1], pixels[2], pixels[3]];
    let bytes = match shot::write_png(path, options.width, options.height, pixels) {
        Ok(bytes) => bytes,
        Err(message) => {
            eprintln!("{message}");
            return 1;
        }
    };

    println!(
        "写出：{} → {}（{}×{}，{} 字节，sha256 {}）",
        "纯色",
        path.display(),
        options.width,
        options.height,
        bytes,
        digest::short(path)
    );
    println!(
        "清屏值 (0.25, 0.5, 0.75, 1.0) 的落盘首像素：R={} G={} B={} A={}",
        first[0], first[1], first[2], first[3]
    );

    println!("（离线：不碰交换链，渲染目标是本进程自己的 Rgba8UnormSrgb）");
    let _ = &gpu.instance;
    0
}
