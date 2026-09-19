//! 算子契约：**描述符**（它是谁）与**调用入口**（怎么叫）。
//!
//! 一个 `px_*_op` dylib 导出一个 `&'static OpTable`——两个函数指针加一张描述符表。
//! `extern "Rust"` 是用户裁决（与 Rust dylib 同一套 ABI，不折腾 C 那一层）。

use crate::protocol::Grid;

/// 算子的产物档：决定键里要不要掺评审相机（体积不掺）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    Field,
    Mesh,
    Volume,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpDescriptor {
    pub id: &'static str,
    /// **接口形状哈希**（取代手写的 `version`）—— 改了参数/输入/输出类型就自动变。
    pub interface: u64,
    /// **源码指纹**：这个库编译进去全部源码的 blake3（`build.rs` 算的）。
    /// **不进键**，只在命中时对账（§19.1）。
    pub source_hash: &'static str,
    pub kind: OpKind,
}

/// 参数规范化：TOML 原文 → 键用的规范 JSON。`None` = 文件不存在 ⇒ 用算子的默认值。
///
/// ⚠ 它必须由**算子那一侧**实现：默认值与字段集住在算子的 `Params` 里。
/// 也因此「先 key 后 cook」不受影响 —— 规范化不求值。
pub type ParamsCanonical =
    extern "Rust" fn(op_id: &str, toml: Option<&str>) -> Result<String, String>;

/// 求值：`op_id` + 规范参数 JSON + 画布 + 上游产物的**序列化字节** → 产物的序列化字节。
pub type OpCall = extern "Rust" fn(
    op_id: &str,
    params: &str,
    grid: Grid,
    inputs: &[&[u8]],
) -> Result<Vec<u8>, String>;

pub struct OpTable {
    pub ops: &'static [OpDescriptor],
    pub canonical_params: ParamsCanonical,
    pub call: OpCall,
}

/// 每个 `px_*_op` dylib 导出的入口名**由库名派生**：`<file stem>_table`
/// （`px_field_op.dll` → `px_field_op_table`；Unix 的 `libpx_field_op.so` 先去掉 `lib`）。
///
/// ⚠ 为什么不叫一个固定的名字：三个算子库都要导出同一个契约，而「同一个名字」在
/// **静态**链到一起时（测试、dev-dependency）会当场 `LNK2005`。派生名字让每个库各叫各的，
/// 装载方按文件名词干算出来即可 —— 图自建的 op crate 也就自动适用，不必改一行。
pub fn table_symbol(path: &std::path::Path) -> Vec<u8> {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("px_op");
    let stem = stem.strip_prefix("lib").unwrap_or(stem);
    format!("{stem}_table").into_bytes()
}
