//! 算子库的**源码指纹** —— 身份的自动来源，别手维护清单。
//!
//! 算的是"这个 crate 编译进去的全部源码"：自己 `src/` + 所有 path 依赖的 `src/` + 本文件。
//! 于是**加一个新文件不用改任何清单**，改任何一行实现都自动变键。
//!
//! 实现在 `build/fingerprint.rs`（三个算子库共用一份）。
include!("../build/fingerprint.rs");

fn main() {
    px_fingerprint_for_crate();
}
