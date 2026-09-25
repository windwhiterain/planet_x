use std::path::{Path, PathBuf};
use std::process::Command;

use px_cook::inst::{self, BuildGraph, InstCodegen};
use px_graphs::{assert_toolchain_matches, current_toolchain, recorded_toolchain};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("list") => list(),
        Some("build") => build(&args[1..]),
        Some("run") => run(&args[1..]),
        Some(other) => Err(format!("不认识的子命令 `{other}`\n{}", usage())),
        None => Err(usage()),
    };
    if let Err(err) = result {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn usage() -> String {
    "用法：px <list|build|run>\n  \
     list                              计划：逐条打印 id / 声明 / 根 / 源 / key / 有|缺\n  \
     build [--gc] [--deep] [--target]  stage 1：编缺的那些（--gc 顺手回收非活实例库）\n  \
     run <图> [--build] [--store <目录>] [图自己的参数…]\n  \
                                     两个 stage 顺序执行（默认不编：运行只读）"
        .to_string()
}

fn graph() -> BuildGraph {
    let mut graph = BuildGraph::new();
    px_graphs::insts::build(&mut graph);
    graph
}

fn codegen() -> &'static px_cook::inst::InstCatalogue {
    px_graphs::insts::codegen()
}

fn list() -> Result<(), String> {
    let graph = graph();
    let instances = graph.instances();
    println!("实例 {} 条：", instances.len());
    let mut stale = 0_u32;
    let mut unreadable = 0_u32;
    let mut unrecorded = 0_u32;
    for info in instances {
        let key = inst::key_of_info(info);
        let present = Path::new(&info.library).is_file();
        // The toolchain axis is not part of the key: `-Level opt` shares keys with `dev`, so a
        // library on disk can be from another build of the same identity. The sidecar is the only
        // record of which build compiled it, so read it back here.
        let status = if present {
            sidecar_status(&info.library)
        } else {
            Toolchain::NotBuilt
        };
        let stale_here = matches!(status, Toolchain::Old { .. } | Toolchain::Invalid { .. });
        if stale_here {
            stale += 1;
        }
        if matches!(status, Toolchain::Invalid { .. }) {
            unreadable += 1;
        }
        if matches!(status, Toolchain::Unrecorded) {
            unrecorded += 1;
        }
        println!(
            "  {:<20} {:<12} {:<16} {:<18} {} {}{}",
            info.op_id,
            decl_short(&info.decl_hash),
            info.alg_roots.join(","),
            info.source,
            short(&key),
            if present { "有" } else { "缺" },
            if stale_here { " !" } else { "" },
        );
        match status {
            Toolchain::Current | Toolchain::NotBuilt => {}
            Toolchain::Old { recorded } => println!(
                "      ⚠ {} 记着工具链 {}，不是当前这份构建 {} ⇒ 库在盘上，但可能是别的档编的",
                info.op_id,
                short(&recorded),
                short(current_toolchain()),
            ),
            Toolchain::Unrecorded => println!(
                "      ⚠ {} 没有 sidecar：库在盘上，但没有任何东西记着它是哪一档编的",
                info.op_id
            ),
            Toolchain::Invalid { why } => println!(
                "      ✗ {} 的 sidecar 解不开（{why}）⇒ 这一档记不上，别把它当成新鲜库",
                info.op_id
            ),
        }
    }
    if stale > 0 {
        let broken = if unreadable > 0 {
            format!("，其中 {unreadable} 条的 sidecar 解不开")
        } else {
            String::new()
        };
        let absent = if unrecorded > 0 {
            format!("；另有 {unrecorded} 条没有 sidecar")
        } else {
            String::new()
        };
        println!(
            "陈旧 {stale} 处（`!`：库在盘上，但不是当前这份构建编的）{broken}{absent}—— \
             实例键不含 `-Level`，`opt` 与 `dev` 共用键；要按当前档重编就 `px build`"
        );
    } else if unrecorded > 0 {
        println!("{unrecorded} 条没有 sidecar ⇒ 判断不了是不是当前这份构建编的");
    }
    Ok(())
}

/// What the sidecar next to a compiled library says about the build that wrote it. Three outcomes
/// that must not collapse into one, because each calls for a different reaction: there is no
/// sidecar, there is one that will not parse, and there is one from another build.
enum Toolchain {
    Current,
    Old {
        recorded: String,
    },
    /// No library on disk, so there is nothing to ask about.
    NotBuilt,
    /// A library on disk with no sidecar beside it.
    Unrecorded,
    /// A sidecar is there and `serde_json` rejects it.
    Invalid {
        why: String,
    },
}

#[derive(serde::Deserialize)]
struct Sidecar {
    toolchain: String,
}

fn sidecar_status(library: &str) -> Toolchain {
    let sidecar = Path::new(library).with_extension("json");
    let text = match std::fs::read_to_string(&sidecar) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Toolchain::Unrecorded,
        Err(err) => {
            return Toolchain::Invalid {
                why: err.to_string(),
            };
        }
    };
    match serde_json::from_str::<Sidecar>(&text) {
        Ok(read) if read.toolchain == current_toolchain() => Toolchain::Current,
        Ok(read) => Toolchain::Old {
            recorded: read.toolchain,
        },
        Err(err) => Toolchain::Invalid {
            why: err.to_string(),
        },
    }
}

fn build(flags: &[String]) -> Result<(), String> {
    let gc = flags.iter().any(|flag| flag == "--gc");
    let deep = flags.iter().any(|flag| flag == "--deep");
    let target = flags.iter().any(|flag| flag == "--target");
    for flag in flags {
        if !matches!(flag.as_str(), "--gc" | "--deep" | "--target") {
            return Err(format!("不认识的开关 `{flag}`\n{}", usage()));
        }
    }
    if (deep || target) && !gc {
        return Err(format!(
            "`--deep` / `--target` 是 `--gc` 的细化开关，得一起给：px build --gc [--deep] [--target]\n{}",
            usage()
        ));
    }

    let graph = graph();
    let instances = graph.instances();
    let catalogue = codegen();

    let mut built = 0_u32;
    let mut already = 0_u32;
    let mut stale = 0_u32;
    let mut failed = 0_u32;
    for info in instances {
        let key = inst::key_of_info(info);
        let present = Path::new(&info.library).is_file();
        let recorded = if present {
            recorded_toolchain(&info.library)
        } else {
            None
        };
        // A library recorded by another build is *work*, not an error: this is the command that makes
        // the plan true again. Refusing here would leave no way to switch levels.
        let stale_here = present && recorded.as_deref() != Some(current_toolchain());
        if present && !stale_here {
            already += 1;
            println!("已有 {}（{}）", info.op_id, short(&key));
            continue;
        }
        if let Some(recorded) = &recorded {
            stale += 1;
            println!(
                "重编 {}（{}）：盘上记着工具链 {}，当前是 {}",
                info.op_id,
                short(&key),
                short(recorded),
                short(current_toolchain()),
            );
        }
        let entry: &InstCodegen = catalogue.for_op(&info.op_id)?;
        match inst::compile_one(info, &key, entry) {
            Ok(()) => {
                built += 1;
                println!("已编 {} → {}", info.op_id, short(&key));
            }
            Err(err) => {
                failed += 1;
                eprintln!("失败 {}（key {}）：\n{err}", info.op_id, key);
            }
        }
    }
    println!(
        "共 {} 条：已编 {built}（其中换档重编 {stale}）、已有 {already}、失败 {failed}",
        instances.len()
    );

    if gc {
        collect_garbage(&graph, deep, target)?;
    }
    if failed > 0 {
        return Err(format!("{failed} 条实例没编出来"));
    }
    Ok(())
}

fn collect_garbage(graph: &BuildGraph, deep: bool, target: bool) -> Result<(), String> {
    let root = px_cook::workspace_root();
    let inst_dir = root.join("target").join("pcg").join("inst");
    let instances = graph.instances();
    let live = inst::live_keys(graph);

    let mut deleted = 0_u32;
    let mut skipped = 0_u32;
    let mut freed = 0_u64;
    println!("活 {} 条：{}", instances.len(), live.join(", "));

    if inst_dir.is_dir() {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&inst_dir)
            .map_err(|err| format!("读不了 {}：{err}", inst_dir.display()))?
            .flatten()
            .map(|entry| entry.path())
            .collect();
        entries.sort();
        for path in entries {
            let key = stem(&path);
            let suffix = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("");
            if suffix != "dll" && suffix != "json" {
                println!("  跳过 {}（不是实例构件）", path.display());
                skipped += 1;
                continue;
            }
            if live.contains(&key) {
                skipped += 1;
                continue;
            }
            let size = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
            std::fs::remove_file(&path)
                .map_err(|err| format!("删不了 {}：{err}", path.display()))?;
            println!(
                "  删 {}（{:.1} MB）",
                path.display(),
                size as f64 / 1_048_576.0
            );
            deleted += 1;
            freed += size;
        }
    }

    if deep {
        let jit = root.join("target/jit");
        if jit.is_dir() {
            let mut dirs: Vec<PathBuf> = std::fs::read_dir(&jit)
                .map_err(|err| format!("读不了 {}：{err}", jit.display()))?
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .collect();
            dirs.sort();
            for dir in dirs {
                if dir.file_name().and_then(|name| name.to_str()) == Some("target") {
                    continue;
                }
                if live.contains(&stem(&dir)) {
                    continue;
                }
                let size = dir_size(&dir);
                std::fs::remove_dir_all(&dir)
                    .map_err(|err| format!("删不了 {}：{err}", dir.display()))?;
                println!(
                    "  删源码目录 {}（{:.1} MB）",
                    dir.display(),
                    size as f64 / 1_048_576.0
                );
                deleted += 1;
                freed += size;
            }
        }
    }

    if target {
        let cache = root.join("target/jit/target");
        match std::fs::metadata(&cache) {
            Ok(_) => {
                let size = dir_size(&cache);
                std::fs::remove_dir_all(&cache)
                    .map_err(|err| format!("删不了 {}：{err}", cache.display()))?;
                println!(
                    "  清共享编译中间物 {}（{:.1} MB；下次 build 会重编）",
                    cache.display(),
                    size as f64 / 1_048_576.0
                );
                deleted += 1;
                freed += size;
            }
            Err(_) => println!("  共享编译中间物 {} 本来就不在", cache.display()),
        }
    }

    println!(
        "共 {} 条：活 {}、删 {deleted}（释放 {:.1} MB）、跳过 {skipped}",
        instances.len(),
        instances.len(),
        freed as f64 / 1_048_576.0,
    );
    Ok(())
}

fn run(args: &[String]) -> Result<(), String> {
    let (graph_name, build, store, passthrough) = parse_run(args)?;

    let graph = graph();
    let plan = graph.missing();
    println!(
        "stage 1｜实例 {} 条：命中 {}、缺 {}",
        plan.total,
        plan.present,
        plan.missing.len()
    );
    if !plan.complete() {
        if build {
            println!("stage 1｜--build：编缺的那些");
            let stage = graph.compile_missing(codegen())?;
            println!("stage 1｜{}", stage.summary(0));
        } else {
            return Err(plan.hint(&graph_name));
        }
    }
    // The plan is complete, which says every library is on disk — not that it came from this build.
    assert_toolchain_matches(&graph.instances())?;

    let exe = graph_exe(&graph_name)?;
    println!("stage 2｜{}", exe.display());
    let mut command = Command::new(&exe);
    if let Some(store) = &store {
        command.arg("--store").arg(store);
    }
    let status = command
        .args(&passthrough)
        .status()
        .map_err(|err| format!("起不了 {}：{err}", exe.display()))?;
    std::process::exit(status.code().unwrap_or(1));
}

fn parse_run(args: &[String]) -> Result<(String, bool, Option<String>, Vec<String>), String> {
    let usage = || {
        "用法：px run <图> [--build] [--store <目录>] [-- 图自己的参数…]\
         \n  --build        stage 1 有缺时**编**它们（默认不编：运行只读）\
         \n  --store <目录> 参数从哪个目录读（默认 art/；图侧同一个开关见 px_graph::driver）\
         \n  图自己的参数（`scene` 的配方名、`passes` 那三个位置参数）直接跟在图名后面"
            .to_string()
    };
    let args: &[String] = match args.first().map(String::as_str) {
        Some("--") => &args[1..],
        _ => args,
    };
    let Some(name) = args.first().filter(|arg| !arg.starts_with('-')) else {
        return Err(usage());
    };
    let mut build = false;
    let mut store: Option<String> = None;
    let mut passthrough = Vec::new();
    let mut after_separator = false;
    let mut index = 1;
    while index < args.len() {
        let arg = args[index].as_str();
        if after_separator {
            passthrough.push(arg.to_string());
            index += 1;
            continue;
        }
        match arg {
            "--" => after_separator = true,
            "--build" => build = true,
            "--store" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(format!("--store 后面要跟一个目录\n{}", usage()));
                };
                store = Some(value.clone());
                index += 1;
            }
            other if !other.starts_with('-') => passthrough.push(other.to_string()),
            other => match other.strip_prefix("--store=") {
                Some(value) => store = Some(value.to_string()),
                None => return Err(format!("不认识的参数 `{other}`\n{}", usage())),
            },
        }
        index += 1;
    }
    Ok((name.clone(), build, store, passthrough))
}

fn graph_exe(name: &str) -> Result<PathBuf, String> {
    let here = std::env::current_exe().map_err(|err| format!("问不到自己在哪：{err}"))?;
    let dir = here
        .parent()
        .ok_or_else(|| format!("{} 没有父目录？", here.display()))?;
    let exe = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    if exe.is_file() {
        return Ok(exe);
    }
    let workspace = px_cook::workspace_root();
    let fallback = [
        workspace.join("target/debug").join(format!("{name}.exe")),
        workspace.join("target/release").join(format!("{name}.exe")),
    ]
    .into_iter()
    .find(|path| path.is_file());
    fallback.ok_or_else(|| {
        format!(
            "找不到图 exe `{name}`（找过 {} 与 target/{{debug,release}}/）\
             \n  ⇒ 先把图程序编出来：cargo build -p px_graphs --bin {name}",
            dir.display()
        )
    })
}

fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut total = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            total += dir_size(&path);
        } else {
            total += std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
        }
    }
    total
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_string()
}

fn decl_short(hash: &str) -> String {
    hash.chars().take(12).collect()
}

fn short(key: &str) -> String {
    key.chars().take(12).collect()
}
