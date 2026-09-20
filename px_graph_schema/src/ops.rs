//! 算子实现的**装载**：契约住在这里（rlib），实现在实现库里（dylib，运行时装载）。
//!
//! ⚠ 这里**没有描述符表、没有注册表、没有字符串 id 分派**：每个算子在编译期就钉死了
//!   「去哪个库、取哪个符号」（`PxOp::LIB` / `PxOp::SYMBOL` 都是常量），运行期只做一次
//!   `GetProcAddress`。类型检查全在编译期 —— `Body<O>` 的签名从 `O` 的三个关联类型推。
//!
//! ⚠ 为什么是**运行期装载**而不是链接期导入：图程序**不许 cargo 依赖实现库**。
//!   只要依赖了，cargo 的源码指纹就盯着它，改一行实现就会重编重链图程序（实测：
//!   静态链 1.91 s、直接依赖 dylib 3.41 s，而运行期装载 **0.44 s 且图 exe 字节不变**）。
//!   代价值得认：跑图之前实现库得先在盘上（缺了**当场拒**，并把该跑的命令打出来）。

use std::collections::HashMap;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use libloading::Library;

use crate::{Grid, PxOp};

/// 实现库里那个函数的签名 —— 从算子的三个关联类型推，**编译期就检查得住**。
pub type Body<O> = fn(
    &<O as PxOp>::Params,
    &<O as PxOp>::Inputs,
    Grid,
) -> Result<<O as PxOp>::Payload, String>;

/// 取 `O` 的实现函数。装载失败当场说清楚该跑什么命令，不静默。
pub fn body<O: PxOp>() -> Result<Body<O>, String> {
    // ⚠ `::<O>` 不能省：`Body<O>` 是 `fn` 指针的**别名**，投影类型（`<O as PxOp>::Params`）
    //   推不出 `O` 来（实测 E0283）。
    load_at::<O>(O::LIB, O::SYMBOL)
}

/// 从**显式**的库（库名或路径）取一个符号。
///
/// ⚠ 给"**内容寻址的实例库**"用（`px_cook::px_inst!`）：实例库的文件名就是它的内容键，
///   运行期才算得出来 ⇒ 编译期写不进 `const LIB`。图侧算子覆盖 `render` 调它。
/// ⚠ 它与 `body::<O>()` 走**同一条**查表/握手/告警路径（不许有第二条装载路径）。
pub fn load_at<O: PxOp>(library: &str, symbol: &str) -> Result<Body<O>, String> {
    // ⚠ 身份符号（`__contract_hash` 那三个）的前缀是**包名**，不是库名/路径：
    //   实例库的文件名是内容键（`<key>.dll`），而它导出的身份符号叫 `px_inst__…`
    //   （`env!("CARGO_PKG_NAME")`）。前缀从`SYMBOL`里取（它就是 `包名__类型名`）。
    let prefix = symbol.split("__").next().unwrap_or(library);
    let opened = open(library, prefix)?;
    let pointer = raw::<*mut c_void>(opened, symbol)
        .map_err(|err| format!("{err}{}", hint(library)))?;
    // ⚠ 全仓**唯一**一处 `transmute`：把符号地址当成"签名由算子钉死的函数"。
    //   它不是类型擦除（那会丢类型检查）—— `Body<O>` 的签名是编译期写死的，
    //   运行期只解析"这个地址在不在"。
    Ok(unsafe { std::mem::transmute::<*mut c_void, Body<O>>(pointer) })
}

/// `lib` 这份实现的**源码指纹** —— 从实现库自己的身份符号取。
///
/// ⚠ 它是"实现换了"这一维进入缓存键的唯一来源（图程序不重编也拿得到）。
pub fn source_hash(lib: &'static str) -> Result<&'static str, String> {
    // 这一档 `lib` **就是包名** ⇒ 库名与身份前缀同值。
    let library = open(lib, lib)?;
    let name = format!("{lib}__source_hash");
    let text = raw::<extern "Rust" fn() -> &'static str>(library, &name)
        .map_err(|err| format!("{err}{}", hint(lib)))?;
    Ok(text())
}

/// 已经装载的实现库（进程内一份）。`name` 可以是库名或路径；`prefix` 是**身份符号的前缀**（包名）。
fn open(name: &str, prefix: &str) -> Result<&'static Library, String> {
    static LIBS: OnceLock<Mutex<HashMap<String, &'static Library>>> = OnceLock::new();
    let mut libs = LIBS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("算子库表锁坏了");
    if let Some(found) = libs.get(name) {
        return Ok(found);
    }

    // ⚠ `name` 有两种，**都要走同一道握手**（不新增装载路径）：
    //   * **包名**（`px_volume_op`）：按平台加前后缀、在几个目录里搜；
    //   * **路径**（内容寻址的实例库：文件名就是它的内容键，编译期写不进 `const LIB`）：原样用。
    //   实测过的真缺陷：把路径也当包名拼一次 `lib….dll` ⇒ 拼成 `libC:\…\x.dll`，永远找不到。
    let path = if Path::new(name).is_absolute() || name.contains(['/', '\\']) {
        PathBuf::from(name)
    } else {
        library_path(name).ok_or_else(|| format!("找不到实现库 {name}{}", hint(name)))?
    };
    if !path.is_file() {
        return Err(format!("实现库不在盘上：{}{}", path.display(), hint(name)));
    }
    let loaded = unsafe { Library::new(&path) }
        .map_err(|err| format!("装载 {} 失败：{err}{}", path.display(), hint(name)))?;

    // ⚠ **契约握手**：DLL 与图程序必须是同一份契约编出来的（类型布局才谈得上一致）。
    //   对不上说明改了 `px_graph_schema` 而 DLL 没重编 —— 当场拒，绝不拿错的布局去调。
    let contract = raw::<extern "Rust" fn() -> &'static str>(&loaded, &format!("{prefix}__contract_hash"))
        .map_err(|err| {
            format!(
                "{} 不是一份实现库（没有身份符号）：{err}{}",
                path.display(),
                hint(name)
            )
        })?;
    if contract() != crate::SOURCE_HASH {
        return Err(format!(
            "{} 与契约**不是同一份**编出来的（DLL {} / 图程序 {}）—— 先 `cargo build -p {name}`",
            path.display(),
            contract(),
            crate::SOURCE_HASH,
        ));
    }

    // ⚠ **工具链握手**：契约一致还不够 —— `extern "Rust"` 的 ABI 由**编译器**定。
    //   实例库（`px_jit build` 生成的）尤其需要这一道：它不是 cargo 沿主 workspace 编的。
    let toolchain =
        raw::<extern "Rust" fn() -> &'static str>(&loaded, &format!("{prefix}__toolchain_hash"))
            .map_err(|err| {
                format!(
                    "{} 没有工具链身份符号（{err}）—— 它是旧形状的实现库，重编：cargo build -p {name}",
                    path.display(),
                )
            })?;
    if toolchain() != crate::TOOLCHAIN_HASH {
        return Err(format!(
            "{} 与图程序**不是同一套工具链**编出来的\n  DLL {} / 图程序 {}\n  \
             ⇒ 用同一套 rustc/target/RUSTFLAGS/profile 重编（实例库：`px_jit build`）",
            path.display(),
            toolchain(),
            crate::TOOLCHAIN_HASH,
        ));
    }

    // ⚠ 陈旧告警只对**包名**那一档有意义（它要找 `<root>/<包名>/src` 比 mtime）；
    //   实例库是内容寻址的：它的新鲜度由 key + 工具链握手管，没有"同名目录"可查。
    if !name.contains(['/', '\\']) {
        warn_if_stale(name, &path);
    }
    let leaked: &'static Library = Box::leak(Box::new(loaded));
    libs.insert(name.to_string(), leaked);
    Ok(leaked)
}

fn raw<T: Copy>(library: &Library, symbol: &str) -> Result<T, String> {
    unsafe { library.get::<T>(symbol.as_bytes()) }
        .map(|found| *found)
        .map_err(|err| format!("装入符号 '{symbol}' 失败：{err}"))
}

/// 实现库在哪儿：`PX_OP_DIR` > exe 同目录 > exe 的上一级（测试 exe 在 `deps/` 里）> 当前目录。
fn library_path(name: &str) -> Option<PathBuf> {
    let file = format!(
        "{}{name}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let mut dirs: Vec<PathBuf> = Vec::new();
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

fn hint(lib: &str) -> String {
    format!(
        "\n  ⚠ 先把实现编出来：cargo build -p {lib}（或者 `cargo build` 编全部 workspace 成员；\
         跑图的 exe 与实现库必须在同一个 target 目录下）"
    )
}

/// 实现库比它的源码旧 ⇒ 这一趟跑的是**旧实现**。说出来，别让人以为改了没用。
///
/// ⚠ 这件事**不会**让产物认错：库的身份（源码指纹）进键，跑的是哪一份实现就出哪一份
///   产物 —— 只是"你以为在跑新的"。所以这里只告警，不拒绝（拒绝会把
///   "只想跑一遍老结果"的人也挡住）。
fn warn_if_stale(name: &str, library: &Path) {
    let Ok(modified) = library.metadata().and_then(|meta| meta.modified()) else {
        return;
    };
    let Some(source) = source_dir(library, name) else {
        return;
    };
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    collect_newest(&source, &mut newest);
    let Some((time, file)) = newest else { return };
    if time > modified {
        eprintln!(
            "⚠ {name} 的实现比库新（{}）⇒ 这一趟跑的是**旧实现**；先 `cargo build -p {name}`",
            file.display()
        );
    }
}

/// 从实现库的位置反推它的 crate 目录：`<root>/target/<profile>/[deps/]<name>.dll`。
fn source_dir(library: &Path, name: &str) -> Option<PathBuf> {
    let parent = library.parent()?;
    for up in [2, 3] {
        let mut root = parent.to_path_buf();
        for _ in 0..up {
            root = root.parent()?.to_path_buf();
        }
        let source = root.join(name).join("src");
        if source.is_dir() {
            return Some(source);
        }
    }
    None
}

fn collect_newest(dir: &Path, newest: &mut Option<(std::time::SystemTime, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_newest(&path, newest);
            continue;
        }
        if path.extension().and_then(|value| value.to_str()) != Some("rs") {
            continue;
        }
        let Ok(modified) = path.metadata().and_then(|meta| meta.modified()) else {
            continue;
        };
        if newest.as_ref().is_none_or(|(time, _)| modified > *time) {
            *newest = Some((modified, path));
        }
    }
}
