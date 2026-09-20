//! **体积域**的算子**实现**（薄壳）：算法在 `px_volume_alg`（rlib）里，这里只把它接到
//! 声明上。
//!
//! ⚠ 这一份是**实现**：声明在 `px_volume_schema::ops` 里（图侧编译的是那一份）。
//!   底下那一行 `px_body!` 就是"声明 ↔ 实现"的那根线。
//!
//! ⚠ 它**不是**任何人的 cargo 依赖：图程序按身份在运行期装载它。
//!
//! ⚠ 算法**为什么搬出去**：泛型实例（`px_inst!`）的 key 必须覆盖 alg crate 的源码名册
//!   （§177），而名册要的是一个能被**从盘上**收的目录 ⇒ 算法得有一个自己的 rlib crate。
//!   这个壳与实例库链的是**同一份** `px_volume_alg`，于是"预置那一档"与"泛型那一档"
//!   算出来的体积必然一致。
//!
//! ⚠ 底下那个 `use` 只为一件事：`px_body!` 的**输入**里那个声明名要在本 crate 里解析得出来。
//!   宏展开后的代码全是 `$crate::…` / `::core::…` 全路径 ⇒ 别的 `use` 都不需要
//!   （曾经照着"声明↔实现那条线"多写了三个，rustc 判 unused 是对的，已删）。

use px_volume_schema::ops::CloudCoarse;

px_graph_schema::px_body! {
    CloudCoarse,
    |p, i, _g| px_volume_alg::eval_sampled(p, i.coverage.value())
}

// 这个库的身份：`…__source_hash`（进键的"实现是哪一份"）与 `…__contract_hash`
// （装载时与图程序对账"我们是不是同一份契约编出来的"）。
px_graph_schema::px_impl_lib!();
