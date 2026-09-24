//! **`px`**：stage 1（build graph）与 stage 2（数据图）的**唯一 driver**
//! （`20-build-graph.md` §182 里那两个词，终于有对应的 bin 了）。
//!
//! ```text
//! px list                              计划：逐条打印 id / 声明 / 根 / 源 / key / 有|缺
//! px build [--gc] [--deep] [--target]  stage 1：编缺的那些；（--gc 顺手回收非活实例库）
//! px run <图> [--build] [--store <目录>] [图自己的参数…]
//!                                      两个 stage 顺序执行
//! ```
//!
//! ⚠ `px run --store <目录>` 是**给调参面板用的**（`px_render/src/edit.rs`）：它把这一趟
//!   读参数的目录从 `art/` 换到别处（会话副本），再原样转交给图 exe。它**不进键** ——
//!   产物键跟参数的字节走，不跟目录走（`px_graph::driver` 的模块文档有整段）。
//!
//! ⚠ `list` 与 `build`（含 `--gc`）**不需要图名**：stage 1 是 **crate** 的属性，不是哪张图的
//!   —— 这正是把两个 driver 并成一个的理由。
//!
//! ⚠ 节点从哪儿来：跑一遍 `px_graphs::insts::build()` 建图（`20` §184/§190 的 stage 1 声明）。
//!   而**图里那些事实**（op id / 根 / 源 / 体）今天来自 `inst_recipe.rs` 那张数据表：
//!   生成器（`px_graphs/build.rs`）把它翻成 `OUT_DIR/insts_gen.rs` 的类型 + 一张
//!   `insts_gen_catalogue.rs` 的事实表（`21-codegen-types.md`）。**文本扫描不再参与执行**。
//!
//! ⚠ 这里**没有一行"怎么编"**：都在 `px_cook::inst::BuildGraph` 上（`missing()` /
//!   `compile_missing()` / `compile_one()`）—— 这一层只管"取节点、打印、拼汇总行、删垃圾、
//!   起图 exe"。编译失败的 **stderr 原样透出**，失败**不写缓存**、直接非零退出（`19` §176）。

use std::path::{Path, PathBuf};
use std::process::Command;

use px_cook::inst::{self, BuildGraph, InstCodegen};

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

/// **stage 1 的节点从哪儿来**：跑一遍 `px_graphs::insts::build`（build graph 的声明），
/// 收它建出来的节点 —— 不读任何手写清单（`20` §184/§190）。
fn graph() -> BuildGraph {
    let mut graph = BuildGraph::new();
    px_graphs::insts::build(&mut graph);
    graph
}

/// 生成期事实（每条实例的 op id / key / 体住哪）：**错误映射**与"点名"用它。
fn codegen() -> &'static px_cook::inst::InstCatalogue {
    px_graphs::insts::codegen()
}

/// `px list`：`id / 声明 / 根 / 源 / key / 有|缺`。
fn list() -> Result<(), String> {
    let graph = graph();
    let instances = graph.instances();
    println!("实例 {} 条：", instances.len());
    for info in instances {
        let key = inst::key_of_info(info);
        let present = Path::new(&info.library).is_file();
        println!(
            "  {:<20} {:<12} {:<16} {:<18} {} {}",
            info.op_id,
            decl_short(&info.decl_hash),
            info.alg_roots.join(","),
            info.source,
            short(&key),
            if present { "有" } else { "缺" },
        );
    }
    Ok(())
}

/// `px build [--gc] [--deep] [--target]`：只编**缺**的那些（`--gc` 顺手回收）。
///
/// ⚠ 一条一条编（不是 `compile_missing()` 那样"第一条失败就回"）：这样**剩下的也编得完**，
///   而汇总那行仍报得出 `失败 c`。两处用的是**同一个** `compile_one`。
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
    let mut failed = 0_u32;
    for info in instances {
        let key = inst::key_of_info(info);
        if Path::new(&info.library).is_file() {
            already += 1;
            println!("已有 {}（{}）", info.op_id, short(&key));
            continue;
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
        "共 {} 条：已编 {built}、已有 {already}、失败 {failed}",
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

/// `--gc`：删掉**非活**的实例构件（实例库是**纯缓存**）。
///
/// **为什么安全**：实例库的**文件名就是它的构建指纹**（`19` §179.2），而那个指纹由
/// `px_cook::inst::key` 从盘上重算得出来、`px build` 随手重编得回来 ⇒ 误删的代价上限是
/// **一次重编**，不伤任何正确性：产物键里进的是算出来的 key，不是"盘上有没有这个文件"。
///
/// ⚠ **活键集 = 本 crate 的 build graph 里那些节点**（跑 `insts::build`）—— 判活**只按本 crate**。
///   若将来有**另一份图程序 crate**也声明实例，它的库会被判成非活（照样可重建，但白编一次）。
///   到那时 gc 得把**多份 build graph** 合起来判活（或按 key 去问每个图程序），别只改这里一半。
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
        // ⚠ 扫的是 `target/jit/` **底下的每一格**，不只是"这次删掉的那些 dll 对应的 key"：
        //   dll 早被手工删掉、或从没拷过去的 key，也会在 `target/jit/<key>/` 里留下源码目录
        //   （实测踩过：那种目录在盘上留了五六份）。非活 = 不在活键集里，与"dll 在不在"无关。
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
                // 共享的编译中间物不按 key 分（由 `--target` 管），别在这儿误删。
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
        // ⚠ 共享的编译中间物：**所有**实例共用 `target/jit/target/` ⇒ 它不属于某一条 key，
        //   进不了上面那个按 key 的循环。`--target` 就是"顺手把它清掉"的那一档。
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

/// `px run <图> [--build] [-- <图参数…>]`：两个 stage 顺序执行（`20` §182）。
///
/// ⚠ **默认"运行只读"**（`20` §186）：stage 1 有缺**且没给** `--build` ⇒ 停下来报"该跑哪条命令"、
///   **非零退出**。不许让 stage 2 偷偷触发编译 —— 那会把"先证明状态是静止的，再量"那条纪律
///   （`18` §171.5 用血换的）一并弄丢。
/// ⚠ stage 2 **不嵌套 cargo**：图 exe 就在**本 exe 旁边**（同一个 `target/<profile>/`），
///   直接起它，退出码与输出原样透出去。
///
/// ⚠ `--store <目录>` 由**本函数**从自己的命令行摘走、再原样喂给图 exe（见
///   `parse_run`）。它不进 stage 1，也不许进键：产物键只跟参数的**字节**有关，
///   与"那些字节住在哪个目录"无关（`px_graph::driver` 的模块文档有整段）。
fn run(args: &[String]) -> Result<(), String> {
    let (graph_name, build, store, passthrough) = parse_run(args)?;

    // ── stage 1：计划（不编译）──────────────────────────────────────────────
    let graph = graph();
    let plan = graph.missing();
    println!(
        "stage 1｜实例 {} 条：命中 {}、缺 {}",
        plan.total,
        plan.present,
        plan.missing.len()
    );
    if !plan.complete() {
        // `--build` 才是显式请求编译（§186）。
        if build {
            println!("stage 1｜--build：编缺的那些");
            let stage = graph.compile_missing(codegen())?;
            println!("stage 1｜{}", stage.summary(0));
        } else {
            return Err(plan.hint(&graph_name));
        }
    }

    // ── stage 2：跑那张图（只读装载）────────────────────────────────────────
    let exe = graph_exe(&graph_name)?;
    println!("stage 2｜{}", exe.display());
    let mut command = Command::new(&exe);
    // ⚠ `--store` 排在**最前**：图 exe 那几支自己按位置读参数（`scene` 的配方名、
    //   `passes` 的三个位置参数），而它们都走 `px_cook::args_without_store()` 把它摘掉
    //   —— 排在前面只是让"它属于 px，不属于图"在命令行上一眼看得见。
    if let Some(store) = &store {
        command.arg("--store").arg(store);
    }
    let status = command
        .args(&passthrough)
        .status()
        .map_err(|err| format!("起不了 {}：{err}", exe.display()))?;
    // ⚠ 退出码**原样**传出去：图的判据就是退出码（`tools/px.ps1` 的文件头那条）。
    std::process::exit(status.code().unwrap_or(1));
}

/// `px run <图> [--build] [--store <目录>] [-- 图参数… | 图参数…]`。
///
/// ⚠ 头一个 `--`（`tools/px.ps1 -Target run -Graph <图>` 那条路径可能带进来）只是参数
///   **分隔符**：PowerShell 的 `-` 会被它自己的参数绑定吃掉（`-Level`），所以那一边用 `-Graph`。
/// ⚠ `--` 之后一律**原样**交给图 exe（图自己的参数，这里不解释它们）。
///
/// ⚠ **不带 `--` 的位置参数也算图参数**（`px run scene orbit-bare`）：`graph_exe` 旁边
///   那几支图程序自己按位置读参数，而 `px run scene -- orbit-bare` 与
///   `px run scene orbit-bare` 在这里是**同一件事**。收下它不放松任何一条：
///   px 自己的开关全部以 `-` 开头，所以"不带 `-` 的东西"不可能是 px 的，只可能是图的。
///   （从前那种写法只认 `--` 之后，症状是 `px run scene orbit-bare` 报"不认识的参数
///   `orbit-bare`" —— 那是一句**指错方向**的话：`orbit-bare` 本来就不是给 px 的。）
///
/// ⚠ `--store <目录>` 是 px **自己**的开关（所以它在 `--` **之前**），用处是"这一趟从
///   哪个目录读参数"：产物键跟字节走、不跟目录走（`px_graph::driver` 那一整段）。
///   它由 px 原样转交给图 exe —— 图 exe 的 `px_cook::apply_store_args` 认的就是它。
///   ⚠ `--store <目录>` 与 `--store=<目录>` **两种写法都收**：那是同一个开关的两种写法，
///     只收一种的话，另一种会在**离病因最远的地方**报"不认识的参数"。
fn parse_run(args: &[String]) -> Result<(String, bool, Option<String>, Vec<String>), String> {
    let usage = || {
        "用法：px run <图> [--build] [--store <目录>] [-- 图自己的参数…]\
         \n  --build        stage 1 有缺时**编**它们（默认不编：运行只读）\
         \n  --store <目录> 参数从哪个目录读（默认 art/；图侧同一个开关见 px_graph::driver）\
         \n  图自己的参数（`scene` 的配方名、`passes` 那三个位置参数）直接跟在图名后面"
            .to_string()
    };
    // 头一个分隔符（如果有）不算参数。
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
            // 不带 `-` 的一律是图的（见上面那段）：`px run scene orbit-bare`。
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

/// 图 exe 在**本 exe 旁边**（同一个 `target/<profile>/`）—— stage 2 不嵌套 cargo。
fn graph_exe(name: &str) -> Result<PathBuf, String> {
    let here = std::env::current_exe().map_err(|err| format!("问不到自己在哪：{err}"))?;
    let dir = here
        .parent()
        .ok_or_else(|| format!("{} 没有父目录？", here.display()))?;
    let exe = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    if exe.is_file() {
        return Ok(exe);
    }
    // ⚠ 兜一个底：`px` 是从 `cargo run` 起的（那时它在同一个目录里），但直接跑
    //   `target/debug/px.exe` 也一样。两条都不在 ⇒ 说清楚该编什么。
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

/// 一棵目录树的总字节数（量不出来就回 0：gc 不该因为"量不动"而失败）。
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
