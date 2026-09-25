//! See docs/programs.md
pub mod cloud_proxy;
pub mod elem;
pub mod insts;

use std::path::Path;

use px_cook::inst::InstInfo;

pub const SOURCE_HASH: &str = env!("PX_SOURCE_HASH");

/// The build that is running now, which is what every present library must have been compiled by.
pub fn current_toolchain() -> &'static str {
    px_graph_schema::TOOLCHAIN_HASH
}

/// The first 12 hex digits, which is what an operator row prints; comparisons use the full hash.
fn short(hash: &str) -> &str {
    &hash[..hash.len().min(12)]
}

/// The toolchain recorded beside a library, or `None` when there is no sidecar or it will not parse.
/// A library with no record is not treated as belonging to another build: this gate exists to stop two
/// **known** builds from being confused, and inventing a mismatch where nothing is recorded would
/// refuse libraries that were merely compiled before sidecars existed.
pub fn recorded_toolchain(library: &str) -> Option<String> {
    let sidecar = Path::new(library).with_extension("json");
    let text = std::fs::read_to_string(sidecar).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value.get("toolchain")?.as_str().map(str::to_string)
}

/// The present libraries built by another toolchain, each named with both sides. Empty means the plan
/// may be loaded. A missing library is not this gate's business: absence is what `stage 1` counts and
/// `px build` fills.
pub fn toolchain_mismatches(instances: &[InstInfo]) -> Vec<String> {
    let current = current_toolchain();
    instances
        .iter()
        .filter(|info| Path::new(&info.library).is_file())
        .filter_map(|info| {
            let recorded = recorded_toolchain(&info.library)?;
            if recorded == current {
                return None;
            }
            Some(format!(
                "  {} 的库记着工具链 {}，本进程是 {}（{}）",
                info.op_id,
                short(&recorded),
                short(current),
                info.library,
            ))
        })
        .collect()
}

/// Refuses a plan whose present libraries were compiled by another build. The refusal names both sides
/// and the command that resolves it, because switching level is recoverable: `px build` rewrites the
/// libraries (and their sidecars) with the current toolchain.
pub fn assert_toolchain_matches(instances: &[InstInfo]) -> Result<(), String> {
    let mismatched = toolchain_mismatches(instances);
    if mismatched.is_empty() {
        return Ok(());
    }
    Err(format!(
        "盘上有 {} 条实现库不是当前这份构建编的 ⇒ 先 `px build` 按当前档重编它们：\n{}\n\
         ⚠ 实例键不含 `-Level`，所以键看不出是哪一档编的；这是**拒绝加载**，不是缓存命中。",
        mismatched.len(),
        mismatched.join("\n"),
    ))
}
