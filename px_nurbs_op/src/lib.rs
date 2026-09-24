//! **NURBS 域**的算子**实现**：曲线（圆 / 求值 / 插入 / 升阶 / 细分）与曲面
//! （球 / 求值 / 插入 / 升阶 / 细分），编成一份 dylib。
//!
//! ⚠ 这一份是**实现**：声明在 `px_nurbs_schema::ops` 里（图侧编译的是那一份）。
//!   每个 `ops/<算子>.rs` 顶上那一行 `px_body!` 就是"声明 ↔ 实现"的那根线。
//!
//! ⚠ 算法本身住在 `px_nurbs_schema`（`curve.rs` / `surface.rs`）—— **故意的**：
//!   那些算法是"这个域是什么"的一部分（`insert_knot` / `elevate_degree` 的几何不变性
//!   是判据直接量的东西），而声明那一层必须能离开实现库单独编译与判据
//!   （`px_field_schema` 把可复用那一半放 `px_field_alg` 是同一条理由的另一半：
//!   那一半要给**泛型实例**链，这一半不用）。
//!
//! ⚠ 它**不是**任何人的 cargo 依赖：图程序按身份在运行期装载它。
//!   也不认识驱动、不认识 CAS、不认识 `art/`。

pub mod ops;
pub mod tessellate;

// 这个库的身份：`…__source_hash`（进键的"实现是哪一份"）与 `…__contract_hash`
// （装载时与图程序对账"我们是不是同一份契约编出来的"）。
px_graph_schema::px_impl_lib!();
