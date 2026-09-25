//! See docs/verdicts.md
//!
//! Reading, not a gate: this binary prints cook-side process-boundary costs in
//! milliseconds and exits 0. It asserts nothing. The numbers feed the backlog
//! decision "which side owns the resident cook process".
//! Usage: cook_boundary [rounds] [spawns]
//!   rounds: in-process warm repetitions (default 3)
//!   spawns: child spawns per child mode (default 3)

use std::path::PathBuf;
use std::time::Instant;

const LIBS: [&str; 5] = [
    "px_field_op",
    "px_volume_op",
    "px_mesh_op",
    "px_nurbs_op",
    "px_nurbs_gpu_op",
];

fn ms_of(work: impl FnOnce()) -> f64 {
    let start = Instant::now();
    work();
    start.elapsed().as_secs_f64() * 1000.0
}

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len().max(1) as f64
}

fn usage() -> ! {
    eprintln!("usage: cook_boundary [rounds] [spawns]");
    std::process::exit(1);
}

/// Mirror of the library search in `px_graph_schema::ops` (that helper is
/// private): where a `<lib>.dll` would be found without opening it. Checking
/// the file keeps the cold-open readings below truly cold — asking the loader
/// first would warm its cache.
fn lib_file(lib: &str) -> Option<PathBuf> {
    let file = format!(
        "{}{lib}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let mut dirs = Vec::new();
    if let Ok(dir) = std::env::var("PX_OP_DIR") {
        dirs.push(PathBuf::from(dir));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            dirs.push(dir.to_path_buf());
            if let Some(parent) = dir.parent() {
                dirs.push(parent.to_path_buf());
            }
        }
    }
    if let Ok(dir) = std::env::current_dir() {
        dirs.push(dir);
    }
    dirs.into_iter()
        .map(|dir| dir.join(&file))
        .find(|path| path.is_file())
}

/// The production loading path (`ops::body`, same call the graph uses): open
/// the library on first use, re-resolve the symbol on every call. Returns None
/// without touching the loader when the library is not on disk.
fn open_ms(lib: &str) -> Option<f64> {
    if lib_file(lib).is_none() {
        return None;
    }
    Some(match lib {
        "px_field_op" => ms_of(|| {
            let _ = px_graph_schema::ops::body::<px_field_schema::ops::Fbm>();
        }),
        "px_volume_op" => ms_of(|| {
            let _ = px_graph_schema::ops::body::<px_volume_schema::ops::CloudCoarse>();
        }),
        "px_mesh_op" => ms_of(|| {
            let _ = px_graph_schema::ops::body::<px_mesh_schema::ops::Proxy>();
        }),
        "px_nurbs_op" => ms_of(|| {
            let _ = px_graph_schema::ops::body::<px_nurbs_schema::ops::Circle>();
        }),
        _ => ms_of(|| {
            let _ = px_graph_schema::ops::body::<px_nurbs_schema::ops::SurfaceTessellateGpu>();
        }),
    })
}

/// Child modes run inside a fresh process, so every `connect` and every first
/// library open in them is a true cold start. Not a pairing arm on their own;
/// the parent alternates the two modes (A/B/A/B) and compares the walls.
fn run_child(mode: &str) -> ! {
    match mode {
        "device" => {
            let start = Instant::now();
            let gpu = px_gpu::connect();
            let cold_ms = start.elapsed().as_secs_f64() * 1000.0;
            let Some(gpu) = gpu else {
                eprintln!("child: no adapter (require_gpu stance)");
                std::process::exit(2);
            };
            let warm_ms = ms_of(|| {
                let _ = px_gpu::connect();
            });
            let info = gpu.adapter.get_info();
            println!("child adapter={} backend={:?}", info.name, info.backend);
            println!("child device_ms={cold_ms:.3}");
            println!("child device_warm_ms={warm_ms:.3}");
            std::process::exit(0);
        }
        "device-libs" => {
            let start = Instant::now();
            let gpu = px_gpu::connect();
            let cold_ms = start.elapsed().as_secs_f64() * 1000.0;
            let Some(gpu) = gpu else {
                eprintln!("child: no adapter (require_gpu stance)");
                std::process::exit(2);
            };
            let info = gpu.adapter.get_info();
            println!("child adapter={} backend={:?}", info.name, info.backend);
            println!("child device_ms={cold_ms:.3}");
            let mut total = 0.0;
            for lib in LIBS {
                match open_ms(lib) {
                    Some(first) => {
                        total += first;
                        println!("child lib {lib} first_ms={first:.3}");
                    }
                    None => println!("child lib {lib}: absent"),
                }
            }
            println!("child lib_total_ms={total:.3}");
            std::process::exit(0);
        }
        _ => {
            eprintln!("unknown child mode: {mode}");
            std::process::exit(1);
        }
    }
}

fn parse_args() -> (usize, usize) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("usage: cook_boundary [rounds] [spawns]");
        println!("reading, not a gate: prints ms costs, exits 0 (2 when no device)");
        std::process::exit(0);
    }
    let mut numbers = Vec::new();
    for arg in &args {
        if let Some(internal) = arg.strip_prefix("--child=") {
            run_child(internal);
        }
        numbers.push(arg.parse::<usize>().unwrap_or_else(|_| usage()));
    }
    if numbers.len() > 2 {
        usage();
    }
    let rounds = numbers.first().copied().unwrap_or(3).max(1);
    let spawns = numbers.get(1).copied().unwrap_or(3).max(1);
    (rounds, spawns)
}

struct Spawned {
    wall_ms: f64,
    child_device_ms: f64,
    child_lib_total_ms: f64,
    status: Option<i32>,
}

fn parse_after(key: &str, line: &str) -> Option<f64> {
    line.split_whitespace().find_map(|token| {
        token
            .strip_prefix(key)
            .and_then(|rest| rest.parse::<f64>().ok())
    })
}

fn spawn_child(mode: &str) -> Spawned {
    let exe = std::env::current_exe().expect("child needs our own path");
    let start = Instant::now();
    let output = std::process::Command::new(exe)
        .arg(format!("--child={mode}"))
        .output()
        .expect("spawn must succeed");
    let wall_ms = start.elapsed().as_secs_f64() * 1000.0;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut child_device_ms = 0.0;
    let mut child_lib_total_ms = 0.0;
    for line in stdout.lines() {
        println!("  | {line}");
        if line.contains("device_warm_ms=") {
            continue;
        }
        if let Some(value) = parse_after("device_ms=", line) {
            child_device_ms = value;
        }
        if let Some(value) = parse_after("lib_total_ms=", line) {
            child_lib_total_ms = value;
        }
    }
    let status = output.status.code();
    if !output.status.success() {
        eprintln!(
            "  | child --child={mode} exited {status:?}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Spawned {
        wall_ms,
        child_device_ms,
        child_lib_total_ms,
        status,
    }
}

fn main() {
    let (rounds, spawns) = parse_args();
    println!("cook_boundary: rounds={rounds} spawns={spawns} (reading, not a gate)");

    // A. Device: the single in-process cold start first, then warm re-connects.
    let start = Instant::now();
    let gpu = px_gpu::connect();
    let device_cold_ms = start.elapsed().as_secs_f64() * 1000.0;
    let device_live = gpu.is_some();
    if let Some(gpu) = gpu {
        let info = gpu.adapter.get_info();
        println!(
            "adapter: {} ({:?}, backend {:?})",
            info.name, info.device_type, info.backend
        );
        let mut warm = Vec::new();
        for _ in 0..rounds {
            warm.push(ms_of(|| {
                let _ = px_gpu::connect();
            }));
        }
        println!("device_cold_in_process_ms={device_cold_ms:.3}");
        println!(
            "device_warm_in_process_ms={:.3} (mean of {rounds})",
            mean(&warm)
        );
    } else {
        println!("device: unavailable (no adapter; require_gpu stance)");
    }

    // B. Per-library open: first open (Library load) vs resident re-resolve.
    // Warm rounds alternate the library order (A/B/A/B): the resident path
    // re-resolves the symbol on every call, so a fixed order would pile any
    // order effect onto the same end.
    println!("== per-library open: first vs resident (in-process) ==");
    let mut first_total = 0.0;
    let mut present = 0_usize;
    let mut warm_samples: Vec<Vec<f64>> = vec![Vec::new(); LIBS.len()];
    for lib in LIBS.iter() {
        match open_ms(lib) {
            Some(first) => {
                present += 1;
                first_total += first;
                println!("lib {lib}: first_ms={first:.3}");
            }
            None => println!("lib {lib}: absent (not on disk; excluded from totals)"),
        }
    }
    for round in 0..rounds {
        let order: Vec<usize> = if round % 2 == 0 {
            (0..LIBS.len()).collect()
        } else {
            (0..LIBS.len()).rev().collect()
        };
        for index in order {
            if let Some(sample) = open_ms(LIBS[index]) {
                warm_samples[index].push(sample);
            }
        }
    }
    let mut warm_total = 0.0;
    for (index, lib) in LIBS.iter().enumerate() {
        if !warm_samples[index].is_empty() {
            let warm_mean = mean(&warm_samples[index]);
            warm_total += warm_mean;
            println!("lib {lib}: resident_ms={warm_mean:.3} (mean of {rounds}, alternating order)");
        }
    }
    println!(
        "lib_total_first_ms={first_total:.3} over {present}/{} present",
        LIBS.len()
    );
    println!(
        "lib_total_resident_ms={warm_total:.3} over {present}/{} present",
        LIBS.len()
    );

    // C. Process boundary: alternate device-only and device+libs children.
    if device_live {
        println!("== process boundary: alternating device / device-libs children ==");
        let mut device_walls = Vec::new();
        let mut libs_walls = Vec::new();
        let mut device_overheads = Vec::new();
        let mut libs_overheads = Vec::new();
        for index in 0..spawns {
            for mode in ["device", "device-libs"] {
                println!("spawn {index}/{mode}:");
                let child = spawn_child(mode);
                if child.status == Some(2) {
                    println!("device: unavailable in child (require_gpu stance)");
                    std::process::exit(2);
                }
                if child.status != Some(0) {
                    eprintln!("child --child={mode} failed; stopping");
                    std::process::exit(1);
                }
                let inside = child.child_device_ms + child.child_lib_total_ms;
                let overhead = (child.wall_ms - inside).max(0.0);
                println!(
                    "  wall_ms={:.3} inside_ms={inside:.3} spawn_overhead_ms={overhead:.3}",
                    child.wall_ms
                );
                if mode == "device" {
                    device_walls.push(child.wall_ms);
                    device_overheads.push(overhead);
                } else {
                    libs_walls.push(child.wall_ms);
                    libs_overheads.push(overhead);
                }
            }
        }
        println!(
            "spawn_plus_device_cold_ms={:.3} (wall mean)",
            mean(&device_walls)
        );
        let mut warm = Vec::new();
        for _ in 0..rounds {
            warm.push(ms_of(|| {
                let _ = px_gpu::connect();
            }));
        }
        println!(
            "device_warm_ms={:.3} (in-process mean of {rounds})",
            mean(&warm)
        );
        println!(
            "warm_plus_lib_open_ms={:.3} (wall mean, device+libs child)",
            mean(&libs_walls)
        );
        println!(
            "spawn_overhead_ms={:.3} / {:.3} (device child / device-libs child means)",
            mean(&device_overheads),
            mean(&libs_overheads)
        );
    }

    if !device_live {
        std::process::exit(2);
    }
}
