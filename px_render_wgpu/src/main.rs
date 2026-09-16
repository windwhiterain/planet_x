//! px_render_wgpu：**不依赖 bevy** 的渲染宿主（`art/15-render-wgpu.md`）。
//!
//! 三条路，一条比一条走得远：
//!
//! - `--device [--shot PNG]`：S0。设备 → 自己的 `Rgba8UnormSrgb` → 回读 → PNG 这条路径
//!   通，而且哈希稳定（§105）。
//! - `--shaders`：离线门。四份内容 shader 按**本宿主的桩表**组装 + naga 校验，不要 GPU。
//! - `--scene <文档> --out PNG`：**按文档里的帧表画一帧**（切片 1：背景 + 行星）。
//!   判据取的是**像素**，不是"跑完了"：出图之后拿 `--diff` 对着 oracle 量差异。
//!
//! ⚠ 每一步都**不许多画一样东西**：切片 1 故意不画天空盒与大气，好让差异的归因只有一种
//! 解释（§107：「一次改两个变量」在这张图上会让读数无法归因）。

mod art;
mod camera;
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
mod shader;
mod shot;
mod stubs;
mod vec;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn usage() -> String {
    [
        "用法：",
        "  px_render_wgpu --scene 文档.pxart --out PNG [--width W] [--height H] [--stats]",
        "      按文档里的帧表画一帧（切片 1：预通道 → 不透明 → blit），回读、落 PNG。",
        "      --stats 额外报回读字节的逐通道 min/max 与颜色数（平场那种判据靠它）。",
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
    ]
    .join("\n")
}

struct Options {
    device: bool,
    shaders: bool,
    shot: Option<PathBuf>,
    scene: Option<PathBuf>,
    out: Option<PathBuf>,
    diff: Option<(PathBuf, PathBuf)>,
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
    width: u32,
    height: u32,
}

fn parse() -> Result<Options, String> {
    let mut options = Options {
        device: false,
        shaders: false,
        shot: None,
        scene: None,
        out: None,
        diff: None,
        stats: false,
        width: 960,
        height: 640,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut next = |what: &str| -> Result<String, String> {
            args.next().ok_or_else(|| format!("{what} 后面要跟一个值"))
        };
        match arg.as_str() {
            "--device" => options.device = true,
            "--shaders" => options.shaders = true,
            "--stats" => options.stats = true,
            "--shot" => options.shot = Some(PathBuf::from(next("--shot")?)),
            "--scene" => options.scene = Some(PathBuf::from(next("--scene")?)),
            "--out" => options.out = Some(PathBuf::from(next("--out")?)),
            "--diff" => {
                let left = PathBuf::from(next("--diff")?);
                let right = PathBuf::from(next("--diff（第二张）")?);
                options.diff = Some((left, right));
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
    Ok(options)
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

/// `--scene`：按文档画一帧、回读、落 PNG。**失败就大声说**（返回非 0）。
fn run_scene(scene: &Path, out: &Path, width: u32, height: u32, stats: bool) -> i32 {
    let gpu = gpu::connect();
    let rendered = match render::run(&gpu, scene, width, height) {
        Ok(rendered) => rendered,
        Err(message) => {
            eprintln!("渲染失败：{message}");
            return 1;
        }
    };
    for line in &rendered.audit {
        println!("{line}");
    }
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

fn main() {
    let options = match parse() {
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

    // 渲染那一档：要 `--scene` 与 `--out` **成对**出现。
    // ⚠ 只给一个就当场拒：默认往某个文件名写图是"替调用方决定了一件它没说过的事"。
    match (&options.scene, &options.out) {
        (Some(scene), Some(out)) => {
            std::process::exit(run_scene(
                scene,
                out,
                options.width,
                options.height,
                options.stats,
            ));
        }
        (Some(_), None) => {
            eprintln!("给了 --scene 却没给 --out：图写到哪里去？\n{}", usage());
            std::process::exit(64);
        }
        (None, Some(_)) => {
            eprintln!("给了 --out 却没给 --scene：画什么？\n{}", usage());
            std::process::exit(64);
        }
        (None, None) => {}
    }

    if !options.device && options.shot.is_none() {
        eprintln!("{}", usage());
        std::process::exit(64);
    }

    let gpu = gpu::connect();

    // 适配器的那几个数后面每一档都要用（group 0 的组数、影子图的边长、时间戳周期），
    // 所以在这里一次报清楚 —— 出问题时先看这一行，不必猜能力。
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

    let Some(path) = options.shot else {
        return;
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
            std::process::exit(1);
        }
    };
    // 首像素要在把 pixels 交出去之前抄下来：写 PNG 会把它移走。
    let first = [pixels[0], pixels[1], pixels[2], pixels[3]];
    let bytes = match shot::write_png(&path, options.width, options.height, pixels) {
        Ok(bytes) => bytes,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    };

    println!(
        "写出：{} → {}（{}×{}，{} 字节，sha256 {}）",
        "纯色",
        path.display(),
        options.width,
        options.height,
        bytes,
        digest::short(&path)
    );
    println!(
        "清屏值 (0.25, 0.5, 0.75, 1.0) 的落盘首像素：R={} G={} B={} A={}",
        first[0], first[1], first[2], first[3]
    );

    // 离线那条路不需要 surface（§104 第 13 条），所以这里显式说明一声：
    // 少了这一句，下一次有人会以为"没开窗口所以没画"。
    println!("（离线：不碰交换链，渲染目标是本进程自己的 Rgba8UnormSrgb）");
    let _ = &gpu.instance;
}
