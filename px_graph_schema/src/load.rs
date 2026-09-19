//! **dyn lib 的调用**：装载、取描述符表、把算子叫起来。
//!
//! 用户裁决把这一层放在契约 crate 里（而不是驱动里）——理由站得住：这里定义的是
//! 「算子是什么、怎么叫」，驱动只是它的一个调用方。

use std::path::{Path, PathBuf};

use crate::op::{OpDescriptor, OpTable, table_symbol};
use crate::protocol::Grid;

pub struct OpLibrary {
    /// 句柄必须活着：`table` 指着它里面的静态数据。
    library: libloading::Library,
    table: &'static OpTable,
    path: PathBuf,
    /// dll 文件字节的 blake3 前 8 字节 —— 「这份产物是哪个二进制算的」（不进键）。
    fingerprint: u64,
}

impl OpLibrary {
    pub fn load(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path)
            .map_err(|err| format!("读不了算子库 {}：{err}", path.display()))?;
        let digest = blake3::hash(&bytes);
        let fingerprint =
            u64::from_le_bytes(digest.as_bytes()[..8].try_into().expect("blake3 是 32 字节"));
        // SAFETY：装载一个本仓自己编出来的 Rust dylib，入口按 §契约 取。
        // 句柄在 `Self` 里活到进程结束；不卸载（`table` 是它里面的静态数据）。
        let library = unsafe { libloading::Library::new(path) }
            .map_err(|err| format!("装载算子库 {} 失败：{err}", path.display()))?;
        let symbol = table_symbol(path);
        let table: &'static OpTable = unsafe {
            let entry: libloading::Symbol<extern "Rust" fn() -> &'static OpTable> =
                library.get(&symbol).map_err(|err| {
                    format!(
                        "{} 里没有入口 {}（它不是算子库？）：{err}",
                        path.display(),
                        String::from_utf8_lossy(&symbol),
                    )
                })?;
            entry()
        };
        Ok(Self {
            library,
            table,
            path: path.to_path_buf(),
            fingerprint,
        })
    }

    pub fn handle(&self) -> &libloading::Library {
        &self.library
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// dll 文件字节的指纹（blake3 前 8 字节，小端）。
    pub fn fingerprint(&self) -> u64 {
        self.fingerprint
    }

    pub fn descriptors(&self) -> &'static [OpDescriptor] {
        self.table.ops
    }

    pub fn descriptor(&self, op_id: &str) -> Option<OpDescriptor> {
        self.table.ops.iter().copied().find(|op| op.id == op_id)
    }

    pub fn canonical_params(&self, op_id: &str, toml: Option<&str>) -> Result<String, String> {
        (self.table.canonical_params)(op_id, toml)
    }

    pub fn call(
        &self,
        op_id: &str,
        params: &str,
        grid: Grid,
        inputs: &[&[u8]],
    ) -> Result<Vec<u8>, String> {
        (self.table.call)(op_id, params, grid, inputs)
    }
}

/// 从目录里把算子库都装进来：认名字（`px_*_op` + 平台动态库后缀）。
///
/// 认名字而不是写死一张表，是为了让**图自建的 op crate**（需要单态化那一支）只要
/// 编出来放在同一个目录里就会被接上 —— 它跟内置的算子走的是同一个入口。
pub fn load_directory(dir: &Path) -> Result<Vec<OpLibrary>, String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|err| format!("读不了算子库目录 {}：{err}", dir.display()))?;
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let Some(extension) = path.extension().and_then(|ext| ext.to_str()) else {
            continue;
        };
        if !matches!(extension, "dll" | "so" | "dylib") {
            continue;
        }
        // `libpx_field_op` 是 Unix 上的前缀形态。
        let stem = stem.strip_prefix("lib").unwrap_or(stem);
        if stem.starts_with("px_") && stem.ends_with("_op") {
            paths.push(path);
        }
    }
    paths.sort();
    paths.iter().map(|path| OpLibrary::load(path)).collect()
}
