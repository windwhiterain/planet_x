//! 算子库的**源码指纹** —— 身份的自动来源，别手维护清单。
//!
//! 算的是"这个 crate 编译进去的全部源码"：自己 `src/` + 所有 path 依赖的 `src/` + 本文件。
//! 于是**加一个新文件不用改任何清单**，改任何一行实现都自动变键。
//!
//! 实现在 `px_fingerprint`（**rlib**：build.rs 与运行期代码共用同一份算法）。
//! 见 `docs/operators.md`（身份怎么算、进哪些键）。

fn main() {
    px_fingerprint::cargo_fingerprint_for_crate();
}
