#![allow(unused_imports)]
//! 只剩「按文本组装 shader」这一层：GPU 探针已经搬到 `px_probe`（§47）。
//!
//! 实现住在 `px_render::shaders`，这里只转出，免得测试侧再抄一份。
//!
//! ⚠️ 它比运行时宽松：这里递归展开整个 `#import` 模块，而 Bevy 只内联点名的符号
//!    ⇒「测试能过」给不了「运行时能过」的保证（§46.4）。

pub use px_render::shaders::{
    assemble, expand, import_path_of, module_sources, render_source, shader_files,
};
