use std::collections::HashMap;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use libloading::Library;

use crate::PxOp;

pub type Body<O> =
    fn(&<O as PxOp>::Params, &<O as PxOp>::Inputs) -> Result<<O as PxOp>::Payload, String>;

pub fn body<O: PxOp>() -> Result<Body<O>, String> {
    load_at::<O>(O::LIB, O::SYMBOL)
}

pub fn load_at<O: PxOp>(library: &str, symbol: &str) -> Result<Body<O>, String> {
    let prefix = symbol.split("__").next().unwrap_or(library);
    let opened = open(library, prefix)?;
    let pointer =
        raw::<*mut c_void>(opened, symbol).map_err(|err| format!("{err}{}", hint(library)))?;
    Ok(unsafe { std::mem::transmute::<*mut c_void, Body<O>>(pointer) })
}

pub fn source_hash(lib: &'static str) -> Result<&'static str, String> {
    let library = open(lib, lib)?;
    let name = format!("{lib}__source_hash");
    let text = raw::<extern "Rust" fn() -> &'static str>(library, &name)
        .map_err(|err| format!("{err}{}", hint(lib)))?;
    Ok(text())
}

fn open(name: &str, prefix: &str) -> Result<&'static Library, String> {
    static LIBS: OnceLock<Mutex<HashMap<String, &'static Library>>> = OnceLock::new();
    let mut libs = LIBS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("算子库表锁坏了");
    if let Some(found) = libs.get(name) {
        return Ok(found);
    }

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

    let contract =
        raw::<extern "Rust" fn() -> &'static str>(&loaded, &format!("{prefix}__contract_hash"))
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

    let toolchain = raw::<extern "Rust" fn() -> &'static str>(
        &loaded,
        &format!("{prefix}__toolchain_hash"),
    )
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
