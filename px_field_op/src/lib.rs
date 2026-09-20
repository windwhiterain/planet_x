//! **场域**的算子**实现**：八个预置算子，编成一份 dylib。
//!
//! ⚠ 这一份是**实现**：声明在 `px_field_schema::ops` 里（图侧编译的是那一份）。
//!   每个 `ops/<算子>.rs` 顶上那一行 `px_body!` 就是"声明 ↔ 实现"的那根线。
//!
//! ⚠ **算法里可复用的那一半住在 `px_field_alg`（rlib）**：格点遍历 + 值域映射/钳制
//!   （`remap_with` / `remap_sampled`）。这一份只是**薄壳**：把那些入口接到声明上。
//!   为什么算法要搬出去：泛型实例（`px_inst!`）的 key 必须覆盖 alg crate 的源码名册
//!   （`.agents/notes/art/19-generic-inst.md` §177），而名册要的是一个能被**从盘上**收的目录
//!   ⇒ 算法得有一个自己的 rlib crate（与体积域的 `px_volume_alg` 同一形状）。
//!   这个壳与实例库链的是**同一份** `px_field_alg`，于是"预置那一档"与"泛型那一档"
//!   算出来的场必然一致（`remap` 就是这样：它直接调 `px_field_alg::remap_sampled`）。
//!
//! ⚠ 它不认识驱动、不认识 CAS、不认识 `art/`：进来的是**类型化的值**（参数 + 上游），
//!   出去的是同一个形状 —— 因为这一份与图侧编译的是同一份契约，装载时握过手。
//!
//! ⚠ 它**不是**任何人的 cargo 依赖：图程序按身份在运行期装载它。

pub mod noise;
pub mod ops;

// 这个库的身份：`…__source_hash`（进键的"实现是哪一份"）与 `…__contract_hash`
// （装载时与图程序对账"我们是不是同一份契约编出来的"）。
px_graph_schema::px_impl_lib!();
