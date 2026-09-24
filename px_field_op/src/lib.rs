//! **场域**的算子**实现**：六个预置算子，编成一份 dylib。
//!
//! ⚠ 这一份是**实现**：声明在 `px_field_schema::ops` 里（图侧编译的是那一份）。
//!   每个 `ops/<算子>.rs` 顶上那一行 `px_body!` 就是"声明 ↔ 实现"的那根线。
//!   ⚠ 从前是九个：`constant` / `mix` / `remap` 三档 2026-09-27 收进了 element
//!   （`px_elem` 的规格表 —— 纯 pointwise 那一档不再各占一个预置算子）。
//!
//! ⚠ **算法里可复用的那一半住在 `px_field_alg`（rlib）**：格点遍历 + 值域映射/钳制
//!   （`map_grid` / `remap_with`）。这一份只是**薄壳**：把那些入口接到声明上。
//!   为什么算法要搬出去：泛型实例（今天的 `art/inst/*.rs`）的 key 必须覆盖 alg crate
//!   的源码名册（`.agents/notes/art/19-generic-inst.md` §177），而名册要的是一个能被
//!   **从盘上**收的目录 ⇒ 算法得有一个自己的 rlib crate（与体积域的 `px_volume_alg` 同一形状）。
//!   这份预置库与实例库（element 那一档 + `art/inst/*.rs`）链的是**同一份** `px_field_alg`
//!   —— "同一份参数不会算出两种结果"就靠这一条。
//!
//! ⚠ 它不认识驱动、不认识 CAS、不认识 `art/`：进来的是**类型化的值**（参数 + 上游），
//!   出去的是同一个形状 —— 因为这一份与图侧编译的是同一份契约，装载时握过手。
//!
//! ⚠ 它**不是**任何人的 cargo 依赖：图程序按身份在运行期装载它。

pub mod noise;
pub mod ops;
pub mod parallel;

// 这个库的身份：`…__source_hash`（进键的"实现是哪一份"）与 `…__contract_hash`
// （装载时与图程序对账"我们是不是同一份契约编出来的"）。
px_graph_schema::px_impl_lib!();
