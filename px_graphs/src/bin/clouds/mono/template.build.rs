// 这个 crate 编译进去**全部源码**的指纹 —— 身份里"改实现必然重算"那一半。
//
// 算的是：自己 `src/`（stage 1 的场函数 + stage 2 的模板）+ 下面这些契约文件。
// 加文件、改实现都不用记任何清单。
//
// ⚠ 它被生成器放到 `target/mono/<库名>/crate/build.rs`。`include!` 的路径由生成器
//   填（它知道这个 crate 落在哪）—— 用**编译期已知**的路径，不是运行期 cwd。
include!("@FINGERPRINT@");

fn main() {
    px_mono_fingerprint(&[
        @INGREDIENTS@
    ]);
}
