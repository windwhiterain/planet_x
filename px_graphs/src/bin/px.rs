use std::path::{Path, PathBuf};
use std::process::Command;

use px_cook::inst::{self, BuildGraph, InstCodegen};
use px_graphs::{assert_toolchain_matches, current_toolchain, recorded_toolchain};

fn main() {
    px_cook::fault::install_panic_hook();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("list") => list(),
        Some("build") => build(&args[1..]),
        Some("run") => run(&args[1..]),
        Some("cost") => cost(&args[1..]),
        Some(other) => Err(px_cook::fault::line(
            "usage",
            "",
            &format!("不认识的子命令 `{other}`\n{}", usage()),
        )),
        None => Err(px_cook::fault::line("usage", "", &usage())),
    };
    if let Err(err) = result {
        px_cook::fault::report(&err);
    }
}

fn usage() -> String {
    "用法：px <list|build|run|cost>\n  \
     list                              计划：逐条打印 id / 声明 / 根 / 源 / key / 有|缺\n  \
     build [--gc] [--deep] [--target]  stage 1：编缺的那些（--gc 顺手回收非活实例库）\n  \
     run <图> [--build] [--store <目录>] [图自己的参数…]\n  \
                                     两个 stage 顺序执行（默认不编：运行只读）\n  \
     cost <图> [--runs N]              读数：最近 N 轮（默认 3）的逐节点 hit/miss、cook 毫秒、字节与参数"
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
            return Err(px_cook::fault::line(
                "usage",
                "",
                &format!("不认识的开关 `{flag}`\n{}", usage()),
            ));
        }
    }
    if (deep || target) && !gc {
        return Err(px_cook::fault::line(
            "usage",
            "",
            &format!(
                "`--deep` / `--target` 是 `--gc` 的细化开关，得一起给：px build --gc [--deep] [--target]\n{}",
                usage()
            ),
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
        return Err(px_cook::fault::line(
            "missing-instance",
            "stage=build",
            &format!("{failed} 条实例没编出来（编译报错逐条在上面）"),
        ));
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
            .map_err(|err| {
                px_graph_schema::Fault::new(
                    px_graph_schema::Kind::Library,
                    format!("读不了 {}：{err}", inst_dir.display()),
                )
            })?
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
            std::fs::remove_file(&path).map_err(|err| {
                px_graph_schema::Fault::new(
                    px_graph_schema::Kind::Write,
                    format!("删不了 {}：{err}", path.display()),
                )
            })?;
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
                .map_err(|err| {
                    px_graph_schema::Fault::new(
                        px_graph_schema::Kind::Library,
                        format!("读不了 {}：{err}", jit.display()),
                    )
                })?
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
                std::fs::remove_dir_all(&dir).map_err(|err| {
                    px_graph_schema::Fault::new(
                        px_graph_schema::Kind::Write,
                        format!("删不了 {}：{err}", dir.display()),
                    )
                })?;
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
                std::fs::remove_dir_all(&cache).map_err(|err| {
                    px_graph_schema::Fault::new(
                        px_graph_schema::Kind::Write,
                        format!("删不了 {}：{err}", cache.display()),
                    )
                })?;
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
        return Err(px_cook::fault::line("usage", "", &usage()));
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
                    return Err(px_cook::fault::line(
                        "usage",
                        "",
                        &format!("--store 后面要跟一个目录\n{}", usage()),
                    ));
                };
                store = Some(value.clone());
                index += 1;
            }
            other if !other.starts_with('-') => passthrough.push(other.to_string()),
            other => match other.strip_prefix("--store=") {
                Some(value) => store = Some(value.to_string()),
                None => {
                    return Err(px_cook::fault::line(
                        "usage",
                        "",
                        &format!("不认识的参数 `{other}`\n{}", usage()),
                    ));
                }
            },
        }
        index += 1;
    }
    Ok((name.clone(), build, store, passthrough))
}

fn graph_exe(name: &str) -> Result<PathBuf, String> {
    let here = std::env::current_exe().map_err(|err| format!("问不到自己在哪：{err}"))?;
    let dir = here.parent().ok_or_else(|| {
        px_graph_schema::Fault::internal(format!("{} 没有父目录？", here.display()))
    })?;
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
        px_cook::fault::line(
            "missing-graph",
            &format!("graph={name}"),
            &format!(
                "找不到图 exe（找过 {} 与 target/{{debug,release}}/）\
                 \n  ⇒ 先把图程序编出来：cargo build -p px_graphs --bin {name}",
                dir.display()
            ),
        )
    })
}

#[derive(serde::Deserialize)]
struct CostNode {
    seq: u64,
    node: String,
    key: String,
    hit: bool,
    cook_millis: u64,
    bytes: u64,
}

fn parse_cost(args: &[String]) -> Result<(String, usize), String> {
    let usage = || {
        "用法：px cost <图> [--runs N]\n  \
         N 缺省为 3：读最近 N 轮的逐节点 hit/miss、cook 毫秒、字节与参数"
            .to_string()
    };
    let Some(name) = args.first().filter(|arg| !arg.starts_with('-')) else {
        return Err(px_cook::fault::line("usage", "", &usage()));
    };
    let mut runs = 3_usize;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--runs" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(px_cook::fault::line(
                        "usage",
                        "",
                        &format!("--runs 后面要跟轮数\n{}", usage()),
                    ));
                };
                runs = value.parse().map_err(|_| {
                    px_cook::fault::line(
                        "usage",
                        "",
                        &format!("轮数不是正整数：{value}\n{}", usage()),
                    )
                })?;
                if runs == 0 {
                    return Err(px_cook::fault::line(
                        "usage",
                        "",
                        &format!("轮数至少为 1\n{}", usage()),
                    ));
                }
                index += 1;
            }
            other if other.starts_with("--runs=") => {
                runs = other
                    .strip_prefix("--runs=")
                    .unwrap_or("")
                    .parse()
                    .map_err(|_| {
                        px_cook::fault::line(
                            "usage",
                            "",
                            &format!("轮数不是正整数：{other}\n{}", usage()),
                        )
                    })?;
                if runs == 0 {
                    return Err(px_cook::fault::line(
                        "usage",
                        "",
                        &format!("轮数至少为 1\n{}", usage()),
                    ));
                }
            }
            other => {
                return Err(px_cook::fault::line(
                    "usage",
                    "",
                    &format!("不认识的参数 `{other}`\n{}", usage()),
                ));
            }
        }
        index += 1;
    }
    Ok((name.clone(), runs))
}

fn cost(args: &[String]) -> Result<(), String> {
    let (graph_name, runs) = parse_cost(args)?;
    let subject = format!("graph={graph_name}");
    let dir = px_cook::workspace_root()
        .join("target")
        .join("pcg")
        .join(&graph_name);
    let ledger = dir.join("metrics.jsonl");
    let text = match std::fs::read_to_string(&ledger) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            println!(
                "no ledger for {subject}（{} 不在：此图在此目录下没有烘过）",
                ledger.display()
            );
            return Ok(());
        }
        Err(err) => {
            return Err(px_cook::fault::line(
                "manifest",
                &format!("{subject} file=metrics.jsonl"),
                &format!("账本读不了 {}：{err}", ledger.display()),
            ));
        }
    };
    if dir.join("metrics.jsonl.1").is_file() {
        println!("注：上一份轮转账本（metrics.jsonl.1）在，seq 跨文件连续，这里只读当前文件");
    }
    let mut nodes: Vec<CostNode> = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(line).map_err(|err| {
            px_cook::fault::line(
                "manifest",
                &subject,
                &format!("账本第 {} 行解不开：{err}", index + 1),
            )
        })?;
        if value.get("node").is_none() {
            continue;
        }
        nodes.push(serde_json::from_value(value).map_err(|err| {
            px_cook::fault::line(
                "manifest",
                &subject,
                &format!("账本第 {} 行不是节点行：{err}", index + 1),
            )
        })?);
    }
    let mut seqs: Vec<u64> = nodes.iter().map(|node| node.seq).collect();
    seqs.sort_unstable();
    seqs.dedup();
    if seqs.is_empty() {
        return Err(px_cook::fault::line(
            "manifest",
            &subject,
            &format!("{} 里没有可读的轮次", ledger.display()),
        ));
    }
    let window: Vec<u64> = seqs.iter().rev().take(runs).rev().copied().collect();
    let manifest_path = dir.join("manifest.json");
    let manifest: Vec<serde_json::Value> =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).map_err(|err| {
            px_cook::fault::line(
                "manifest",
                &subject,
                &format!("清单读不了 {}：{err}", manifest_path.display()),
            )
        })?)
        .map_err(|err| {
            px_cook::fault::line(
                "manifest",
                &subject,
                &format!("清单解不开 {}：{err}", manifest_path.display()),
            )
        })?;
    let op_of = |node: &str| {
        manifest
            .iter()
            .find(|entry| entry.get("node").and_then(|name| name.as_str()) == Some(node))
            .and_then(|entry| entry.get("op"))
            .and_then(|op| op.as_str())
            .unwrap_or("?")
    };
    let params_path = dir.join("params.json");
    let params: Option<serde_json::Value> = match std::fs::read_to_string(&params_path) {
        Ok(text) => Some(serde_json::from_str(&text).map_err(|err| {
            px_cook::fault::line(
                "manifest",
                &subject,
                &format!("参数索引解不开 {}：{err}", params_path.display()),
            )
        })?),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => {
            return Err(px_cook::fault::line(
                "manifest",
                &subject,
                &format!("参数索引读不了 {}：{err}", params_path.display()),
            ));
        }
    };
    if params.is_none() {
        println!("注：{} 不在，参数列为空", params_path.display());
    }
    let param_of = |node: &str| {
        params
            .as_ref()
            .and_then(|index| index.get(node))
            .and_then(|entry| entry.get("params"))
            .map(|value| serde_json::to_string(value).unwrap_or_else(|_| value.to_string()))
            .unwrap_or_else(|| "—".to_string())
    };
    println!(
        "cost {graph_name}：最近 {} 轮（seq {}..{}）",
        window.len(),
        window.first().unwrap_or(&0),
        window.last().unwrap_or(&0),
    );
    for seq in &window {
        let rows: Vec<&CostNode> = nodes.iter().filter(|node| node.seq == *seq).collect();
        let hits = rows.iter().filter(|node| node.hit).count();
        let millis: u64 = rows.iter().map(|node| node.cook_millis).sum();
        let bytes: u64 = rows.iter().map(|node| node.bytes).sum();
        println!(
            "  seq={seq} 节点 {}：命中 {hits}、重算 {}，cook {millis} ms，bytes {bytes}",
            rows.len(),
            rows.len() - hits
        );
        if *seq == *window.last().unwrap_or(&0) {
            for node in &rows {
                println!(
                    "    {:<16} {:<16} {} key={} millis={} bytes={} params={}",
                    node.node,
                    op_of(&node.node),
                    if node.hit { "命中" } else { "重算" },
                    short(&node.key),
                    node.cook_millis,
                    node.bytes,
                    param_of(&node.node),
                );
            }
        }
    }
    for pair in window.windows(2) {
        let (before, after) = (pair[0], pair[1]);
        println!("  差分 seq{before}→seq{after}：");
        let mut changed = 0_usize;
        for node in nodes.iter().filter(|node| node.seq == after) {
            match nodes
                .iter()
                .find(|prev| prev.seq == before && prev.node == node.node)
            {
                Some(prev) => {
                    let key_mark = if prev.key == node.key {
                        "键同"
                    } else {
                        "键变"
                    };
                    if prev.key != node.key {
                        changed += 1;
                    }
                    let delta_millis = node.cook_millis as i64 - prev.cook_millis as i64;
                    let delta_bytes = node.bytes as i64 - prev.bytes as i64;
                    println!(
                        "    {:<16} {key_mark} millis {:+} bytes {:+}",
                        node.node, delta_millis, delta_bytes,
                    );
                }
                None => {
                    changed += 1;
                    println!(
                        "    {:<16} 新增 millis={} bytes={}",
                        node.node, node.cook_millis, node.bytes
                    );
                }
            }
        }
        for prev in nodes.iter().filter(|node| node.seq == before) {
            if !nodes
                .iter()
                .any(|node| node.seq == after && node.node == prev.node)
            {
                changed += 1;
                println!("    {:<16} 消失", prev.node);
            }
        }
        if changed == 0 {
            println!("    键与读写全同（全命中且内容不动）");
        }
    }
    Ok(())
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
