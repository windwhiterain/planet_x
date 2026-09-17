//! px_render_wgpu：**不依赖 bevy** 的渲染宿主（`art/15-render-wgpu.md`）。
//!
//! 几条路，一条比一条走得远：
//!
//! - `--device [--shot PNG]`：S0。设备 → 自己的 `Rgba8UnormSrgb` → 回读 → PNG 这条路径
//!   通，而且哈希稳定（§105）。
//! - `--shaders`：离线门。四份内容 shader 按**本宿主的桩表**组装 + naga 校验，不要 GPU。
//! - `--scene <文档> --out PNG`：**按文档里的帧表画一帧**（切片 1：背景 + 行星）。
//!   判据取的是**像素**，不是"跑完了"：出图之后拿 `--diff` 对着 oracle 量差异。
//! - `--serve`：常驻服务（租约 + `ProtocolId` 握手 + 批量请求），协议逐字照搬 bevy 宿主。
//!   没有 `--scene` 的调用是**客户端**：把一条请求交给在跑的服务，并把回话打出来。
//!
//! ⚠ 每一步都**不许多画一样东西**：切片 1 故意不画天空盒与大气，好让差异的归因只有一种
//! 解释（§107：「一次改两个变量」在这张图上会让读数无法归因）。

mod art;
mod camera;
mod client;
mod diff;
mod digest;
mod gpu;
mod group0;
mod icosphere;
mod mat4;
mod material;
mod mesh;
mod plan;
mod render;
mod report;
mod serve;
mod shader;
mod shot;
mod stubs;
mod vec;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use px_protocol::render::{Job, Request, Scene, Shot, View};

fn usage() -> String {
    [
        "用法：",
        "  px_render_wgpu --serve [--port N] [--pcg-root DIR] [--width W] [--height H]",
        "      常驻渲染服务：写租约（target/render-server.json）、按请求出图。",
        "      ⚠ 尺寸以**请求里的**为准（--width/--height 只收不用）。",
        "  px_render_wgpu --scene A.pxart --out a.png [--scene B.pxart --out b.png]",
        "            [--cam YAW,PITCH,DIST] [--report r.json] [--width W] [--height H] [--autostart]",
        "      客户端：把一条请求交给在跑的服务。给了多个 --scene 就是**批量**（一个请求出一串图）。",
        "      --report 让服务端把结构化报告落到那个路径（同一份也回给调用方）。",
        "  px_render_wgpu --scene 文档.pxart --out PNG [--width W] [--height H] [--offline] [--stats]",
        "      **离线**出图（要显式写 --offline）：不开服务、不走协议，直接用本进程的设备画一帧。",
        "      ⚠ 为什么它不是缺省：`--scene --out` 在 bevy 宿主那里的语义是**请求**（交给服务），",
        "        而 `tools/` 那套仪器（frame-probe / harness）就是这么调本 exe 的。",
        "        两套语义共用一个写法，等于让'这张图是谁画的'变成一条要靠猜的事。",
        "      --stats：报回读字节的逐通道 min/max 与颜色数（平场那种判据靠它）。",
        "  px_render_wgpu --scene 文档.pxart --out sheet.png --sheet [--columns N] [--offline]",
        "      **对照图**（J2）：12 格 × 960×640 拼成一张 3840×1920 —— 相机表来自产物",
        "      （`.pxart` 的 `cameras`），格子的排布是渲染器的事（缺省 4 列）。",
        "      ⚠ 与 --cam 互斥：相机表与 --cam 是两处会漂开的真相。",
        "  px_render_wgpu --diff A.png B.png",
        "      两张 PNG 逐像素比（不要 GPU）：差异像素数 / 最大与平均通道差 / 差异区域 /",
        "      剪影内的像素是不是逐位相同。",
        "      ⚠ **按位置读，不按角色读**：背景色与剪影**只按第一个参数（左图）**定，",
        "        约定是『左 = 本宿主那张 / 右 = oracle』。传反了整篇读数就反着读 ——",
        "        本工具不认识角色，所以报告开头会把两条路径连同左右一起打出来。",
        "  px_render_wgpu --device [--shot PNG] [--width W] [--height H]",
        "      建实例/适配器/设备（Vulkan 锁死），报「到设备就绪」的读数；",
        "      给了 --shot 就再清一张纯色图、回读、落 PNG。",
        "  px_render_wgpu --shaders",
        "      把四份内容 shader 用**本宿主的桩表**组装出来并 naga 校验（不要 GPU）。",
        "  px_render_wgpu --help",
        "",
        "⚠ 这一版没有的（各自都会**当场拒**，而且拒词指路）：",
        "  --perf/--windows/--frames",
        "                     性能那两路要的是**帧循环**（逐帧采样 / 丢窗 / 等 K 帧），",
        "                     而帧循环是 viewer（S7）的交付物 —— 在那之前 gpu_ms/pair 没有可比对象",
        "  --view/--show      预览窗口（S7）",
    ]
    .join("\n")
}

/// 一次请求里的一步：`--scene` 起一步，`--out` / `--cam` 配到**它前面那一步**。
struct ShotArgs {
    scene: PathBuf,
    out: Option<PathBuf>,
    cam: Option<[f32; 3]>,
}

struct Options {
    // ---- 离线那几条路（S0/S1/S3/S5 的仪器）----
    device: bool,
    shaders: bool,
    shot: Option<PathBuf>,
    diff: Option<(PathBuf, PathBuf)>,
    /// `--offline`：`--scene --out` 由**本进程**画，不走服务、不走协议。
    ///
    /// ⚠ 缺省是"客户端"（交给在跑的服务），与 bevy 宿主那条语义一致 —— `tools/harness.ps1`
    /// 与 `tools/frame-probe.ps1` 就是把本 exe 当**客户端**调的（`Invoke-Client`）。
    /// 两套语义共用 `--scene --out` 这个写法的话，"这张图是谁画的"就要靠猜。
    offline: bool,
    /// `--stats`：把回读到的 RGBA 的**逐通道最小/最大值**与"整幅是不是纯色"打出来。
    ///
    /// 为什么它是宿主的一个开关、而不是一个读 PNG 的脚本：读数要的是**回读出来的那些字节**，
    /// 而"PNG 解码器"是这条判据链上一个新的、会自己出错的环节（本仓已经为"仪器验错了产物"
    /// 付过学费，§122）。宿主手里本来就有那些字节 —— 顺手报一下，就不必再写第二个解码器。
    ///
    /// 判据 §86.4 用的正是它：`grade_half` 的 `strength = 0.5` 在纯反相上是**数学上的平场**
    /// （`mix(x, 1−x, 0.5) ≡ 0.5`）⇒ `min = max = 188 = sRGB(0.5)`，于是"uniform 里到的
    /// 到底是不是 0.5"从一个推断变成一个读数。
    stats: bool,
    // ---- 服务 ----
    serve: bool,
    port: u16,
    pcg_root: PathBuf,
    pcg_root_given: bool,
    /// `--fps` / `--novsync`：**收下但不生效**（本宿主没有帧循环、也没有交换链）。
    ///
    /// ⚠ 为什么是"收下 + 说明"而不是"不认识的参数"：这两个开关是
    /// `tools/harness.ps1::Start-RenderServer` 给**性能那两路**传的（`-Extra @('--fps')`），
    /// 而它等的就绪信号是日志里那行"渲染管线全部就绪"。当场拒 ⇒ 进程立刻退出，可 harness
    /// 的等待循环**看不见**"它死了"，要空等到超时才报一句"服务没在 180 s 内就绪" ——
    /// 那句话指向**错的原因**（§146.3 ③：拦住了不等于说对了）。
    /// 收下之后，真正的拒词由**服务端**在收到性能请求时说出来（"要的是帧循环，那是 S7 的"）。
    fps: bool,
    novsync: bool,
    // ---- 请求 ----
    /// 一次请求要出的那一串图。**空 = 经济世界那条路**（本宿主没有它，服务端当场拒）。
    shots: Vec<ShotArgs>,
    /// 没有 `--scene` 时这一张存哪儿。
    out: Option<PathBuf>,
    /// 没跟在哪一步后面的 `--cam`：整批通用。
    cam: Option<[f32; 3]>,
    width: u32,
    height: u32,
    /// 报告 JSON 写哪儿（空 = 只回给调用方，不落盘）。
    report: String,
    /// 用产物自带的相机表出一张多视角对照图（J2：12 格 × 960×640 ⇒ 3840×1920）。
    ///
    /// ⚠ 它**不是"另一台相机"**：相机表住在产物里（`.pxart` 的 `cameras`），
    /// 格子的排布（4 列、行优先、目标尺寸）是**渲染器的事**（Bevy 的 `SheetCell` 就是这句话）。
    /// 所以 `--cam` 与它互斥：两个来源就是两处会漂开的真相（`Options::parse` 当场拒）。
    sheet: bool,
    /// 对照图的列数（`sheet` 为真时有效）。缺省 4 —— 与 Bevy 宿主同一条（那是**策略**，
    /// 不是内容：产物里只有相机表，没有"排几列"）。
    columns: u32,
    /// `--perf`：这一路请求要收帧（默认是 `--shots`）。
    perf: bool,
    /// 性能那一路要收的**干净**窗口数（老路）。
    windows: u32,
    /// 调用方**显式**给了 `--windows`（决定走老路还是新主路径）。
    windows_given: bool,
    drop_windows: u32,
    /// 新主路径要采的帧数。
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
            serve: false,
            port: 0,
            // ⚠ 缺省 CAS 根**不是** `PathBuf::from("target/pcg")`：那是**当前目录**，
            // 而 cargo 的当前目录是包目录（`px_render_wgpu/`）。`art::default_pcg_root()`
            // 从可执行文件的位置反推工作区。这里只在"服务端"生效。
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
            let mut next = |what: &str| -> Result<String, String> {
                args.next().ok_or_else(|| format!("{what} 后面要跟一个值"))
            };
            match arg.as_str() {
                "--device" => options.device = true,
                "--shaders" => options.shaders = true,
                "--offline" => options.offline = true,
                "--stats" => options.stats = true,
                "--serve" => options.serve = true,
                "--autostart" => options.autostart = true,
                // 收下但不生效：见 `Options::fps` 那段（harness 起性能那两路时会传它）。
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
                    let cam = parse_cam(&next("--cam")?)?;
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
                        .map_err(|_| "--width 需要一个整数".to_string())?
                }
                "--height" => {
                    options.height = next("--height")?
                        .parse()
                        .map_err(|_| "--height 需要一个整数".to_string())?
                }
                "--columns" => {
                    options.columns = next("--columns")?
                        .parse()
                        .map_err(|_| "--columns 需要一个整数".to_string())?
                }
                // ---- 两路活：语义照搬 bevy 宿主（`--shots` 是缺省，`--windows` 显式给了走老路）----
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
                // ---- 这一版没有的路：**当场拒，且说对理由**（不是"不认识的参数"）----
                "--view" | "--show" | "--where" | "--place" => {
                    return Err(format!(
                        "{arg} 不在这一版：预览窗口（常驻相机 / 输入 / 热重载 / 一问一答）是 S7 的交付物。\n{}",
                        usage()
                    ));
                }
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
        // 与 bevy 宿主同一条：对照图用的是产物自带的相机表，两处都给就是两处会漂开的真相。
        if options.sheet && (options.cam.is_some() || options.shots.iter().any(|s| s.cam.is_some()))
        {
            return Err("--sheet 用的是产物自带的相机表，不要再给 --cam".to_string());
        }
        Ok(options)
    }

    /// 单张时的相机：`--scene A --cam …` 是「这一步的相机」，单张请求里它就是 `View.cam`。
    fn view_cam(&self) -> Option<[f32; 3]> {
        match self.shots.as_slice() {
            [shot] => shot.cam.or(self.cam),
            _ => self.cam,
        }
    }

    /// 这一次请求的 `out`：单张就是它，批量时是**最后一张**。
    fn out_path(&self) -> PathBuf {
        self.shots
            .last()
            .and_then(|shot| shot.out.clone())
            .or_else(|| self.out.clone())
            .unwrap_or_else(|| PathBuf::from("target/shot.png"))
    }

    /// 这一次请求是哪一路活。**默认截图**；`--windows` 显式给了才是老性能路。
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
        // ⚠ 新主路径**不出图**（那一路只采样），所以它不要求每一步给 `--out`；
        //    截图与老性能路要落图，少一个 `--out` 就是少一张图，必须报错（照搬 bevy 宿主）。
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

/// 四份内容 shader 组装 + 校验（S1 判据 ③）。**不要 GPU** —— 它是离线门，不是探针。
///
/// 顺带把每份的 `(group, binding)` **反射**出来：那是 group 0 契约的唯一真本，
/// 也是下一步建 bind group layout 的依据（在 Rust 侧再抄一份就是两个数字开始漂）。
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

/// `--scene`（离线）：按文档画一帧、回读、落 PNG。**失败就大声说**（返回非 0）。
///
/// ⚠ 落盘的尺寸用 `rendered.width/height`，不是命令行的 `--width/--height`：
/// 对照图那一档命令行给的是**一格**的尺寸（960×640），而落盘那张是 3840×1920。
fn run_scene(
    scene: &Path,
    out: &Path,
    width: u32,
    height: u32,
    stats: bool,
    views: render::Views,
) -> i32 {
    let gpu = gpu::connect();
    let rendered = match render::run(&gpu, scene, &art::default_pcg_root(), views, width, height) {
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
    // 首像素要在把 pixels 交出去之前抄下来：写 PNG 会把它移走。
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
    println!("首像素：R={} G={} B={} A={}", first[0], first[1], first[2], first[3]);
    if let Some(text) = stats_text {
        println!("{text}");
    }
    println!("执行了：{}", rendered.executed.join(" → "));
    for (label, why) in &rendered.skipped {
        println!("⚠ 没有执行 '{label}'：{why}");
    }
    println!("（离线：不碰交换链，渲染目标是本进程自己的 Rgba8UnormSrgb）");
    0
}

/// 回读到的 RGBA 的逐通道读数：最小 / 最大 / 不同颜色数 / 出现最多的那种颜色。
///
/// ⚠ 它读的是**回读出来的字节**，不是 PNG 解码的结果 —— 见 [`Options::stats`]。
/// 用 `BTreeMap` 计颜色数：平场那种判据下颜色只有一两种，而它是稳定的（读数可复现）。
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

/// `--diff`：两张 PNG 的实测差异。**不要 GPU**（它是读数，不是渲染）。
///
/// ⚠ 参数是**位置**，不是角色（`(左, 右)`）。报告开头会把这一点与两条路径一起打出来 ——
/// 这个工具不知道哪一张是 oracle，而"它替使用者猜角色"正是 §136 那次读反的根因。
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

/// 没有 `--scene` 时那条经济世界路（本宿主没有它）：说清是哪一条路没有。
///
/// ⚠ 它是**一份**真本、**两处**引用（客户端在本地就拒，服务端对协议客户端也拒）：
/// 抄成两份措辞就会漂开，而漂开的那一天读的人会被指向错的原因。
pub use serve::WORLD_REFUSAL;

fn main() {
    let options = match Options::parse() {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(64);
        }
    };

    // 组装那一路在建设备**之前**返回：它是离线的，不该为它付一次 Vulkan 初始化。
    if options.shaders {
        std::process::exit(check_content_shaders());
    }

    // 比对那一档同理：它是**读数**，一张 PNG 都不想重画。
    if let Some((left, right)) = &options.diff {
        std::process::exit(run_diff(left, right));
    }

    // 服务那一路：它自己建设备、写租约、等请求。
    if options.serve {
        // `--fps` / `--novsync` 收下但不生效：说一声，别让它变成一个谁也不看的旗标。
        if options.fps || options.novsync {
            println!(
                "（--fps/--novsync 收下但不生效：本宿主按需渲染，没有帧循环、也不碰交换链 —— 那两个开关属于 S7）"
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

    // ⚠ `--pcg-root` 只在**服务端**生效：解析成员的是服务进程，它有自己的 CAS 根。
    //    客户端这一份只报一声，不能装作生效（照搬 bevy 宿主那句）。
    if options.pcg_root_given {
        eprintln!(
            "⚠ --pcg-root 只在渲染进程（--serve）生效：这次请求的内容由服务进程按它自己的 CAS 根解析"
        );
    }

    // `--device` / `--shot`：S0 那条路（不要文档、不要服务）。它天然是离线的。
    if options.device || options.shot.is_some() {
        std::process::exit(run_device(&options));
    }

    if options.offline {
        std::process::exit(run_offline(&options));
    }

    // ⚠ `--stats` 是**离线**那条路的读数（它读的是本进程回读出来的字节）：
    //    服务那条路的同类读数住在报告里（服务端算的 sha256 / 网格差分 / 兜底像素数）。
    //    两处混用会让"这个数是哪台机器算的"变成一条要靠猜的事。
    if options.stats {
        eprintln!("--stats 是离线那条路（--offline）的读数；走服务那条路的同类读数在 --report 里");
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

/// `--offline --scene 文档 --out PNG`：本进程画一帧、回读、落 PNG。
///
/// 三条"不适用"当场拒 —— 它们要的都是**服务端**的东西，而这条路没有服务端：
/// `--report`（报告是服务算的）、`--perf`/`--windows`/`--frames`（要帧循环）、多份 `--scene`（批量是请求的概念）。
fn run_offline(options: &Options) -> i32 {
    if !options.report.is_empty() {
        eprintln!("--report 是**服务端**产出的（sha256 / 网格差分 / 兜底像素数都由它算）；离线那条路要读数就加 --stats");
        return 64;
    }
    if options.perf || options.windows_given {
        eprintln!("--perf/--windows 要的是**帧循环**（服务那条路的活），而离线这条路一次只画一帧");
        return 64;
    }
    let [shot] = options.shots.as_slice() else {
        eprintln!(
            "离线那条路一次只画**一份**文档（给了 {} 份）：批量是请求的概念，交给服务去做",
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
        // ⚠ 离线这条路**也要走 `--sheet`**：判据那一张对照图就是它出的（J2）。
        //    两处各写一份"怎么看"的翻译就是两处会漂开的真相（服务那条路在 `serve::views_of`）。
        views_of(options),
    )
}

/// 命令行那一档「怎么看」→ `render::Views`（**离线**那条路的翻译；服务那条路在
/// `serve::views_of`，因为那里拿的是请求而不是命令行）。
fn views_of(options: &Options) -> render::Views {
    if options.sheet {
        render::Views::Sheet {
            columns: options.columns,
        }
    } else {
        render::Views::Single(options.view_cam())
    }
}

/// `--device [--shot PNG]`：报设备就绪读数，给了 `--shot` 就再清一张纯色图。
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
        // §104 第 12 条：不可用就降级（gpu_ms 报 null），**不许 panic**。
        println!("时间戳周期：（这一台不可用 ⇒ gpu_ms 将报 null，不做替代读数）");
    }

    let Some(path) = &options.shot else {
        return 0;
    };

    // 纯色是 S0 唯一能单独验这条路径的机会：清一个**非平凡**的值（三个通道都不一样，
    // 这样通道序错了、sRGB 编码错了都看得出来），再原样走到 PNG 上。
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
    // 首像素要在把 pixels 交出去之前抄下来：写 PNG 会把它移走。
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

    // 离线那条路不需要 surface（§104 第 13 条），所以这里显式说明一声：
    // 少了这一句，下一次有人会以为"没开窗口所以没画"。
    println!("（离线：不碰交换链，渲染目标是本进程自己的 Rgba8UnormSrgb）");
    let _ = &gpu.instance;
    0
}
