//! px_render_wgpu：**不依赖 bevy** 的渲染宿主（`art/15-render-wgpu.md` 的 S0）。
//!
//! S0 只证明一件事，但证到底：**设备 → 自己的 `Rgba8UnormSrgb` → 回读 → PNG** 这条路径
//! 通，而且**哈希稳定**。后面每一档的判据都从这条路径上取数（§105）。
//!
//! ⚠ 这一步**故意**不碰 `.pxart`、不碰材质、不碰灯：判据要一处一处地加，
//! 「一次改两个变量」在这张图上会让读数无法归因（§107）。

mod digest;
mod gpu;
mod mesh;
mod shader;
mod shot;
mod stubs;
mod vec;

use std::path::PathBuf;

fn usage() -> String {
    [
        "用法：",
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
    width: u32,
    height: u32,
}

fn parse() -> Result<Options, String> {
    let mut options = Options {
        device: false,
        shaders: false,
        shot: None,
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
            "--shot" => options.shot = Some(PathBuf::from(next("--shot")?)),
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
        println!(
            "时间戳周期：{} ns",
            gpu.queue.get_timestamp_period()
        );
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
