//! **场域**的算法：上游那张场 → 一张新场（格点遍历 + 值域映射/钳制）。
//!
//! ⚠ 这一份是**算法**（rlib）：element 那一档的体文件（`px_elem/body/remap.rs` / `fuse.rs`）
//!   把它链进**内容寻址的实例库**，泛型实例（`art/inst/*.rs`）也把**同一个入口**链进实例库
//!   —— 两边算出来的必须是同一个东西。⇒ 两条路今天共用的入口就是这里的 [`remap_with`]
//!   （走**唯一那条** [`map_grid`]），只有"这一格的值怎么算"不同
//!   （element 那一档在体文件里现搭 [`Scale`]、逐格自己算；实例由 `art/inst/*.rs` 现写）。
//!
//! ⚠ 为什么算法要单独一个 crate（而不是留在 `px_field_op` 里）：泛型实例的键必须覆盖
//!   **alg crate 的源码名册**（`docs/system/generic-instances.md` §177），而名册要的是一个能被**从盘上**
//!   收的目录；算法住在 dylib crate 里就没有那个目录可收。element 那一档的体文件也正是
//!   靠 `ROOTS` 里那一栏 `px_field_alg` 才拿得到同一把尺子（`px_elem::specs` 那几行）。
//!
//! ⚠ `field_fn` 是**对外**的接口：`FieldFn` / `Upstream` / `Sampled` 是给
//!   `art/inst/waves.rs` 那种**外部 impl** 用的（那份源码会被 `px` **`include!`** 进实例库，
//!   不是本 crate 的模块）。而 `dead_code` **只看本 crate 自己用没用** —— 本 crate 只在
//!   判据里用 `Sampled` ⇒ 替它收声。这正是这个机制的性质：泛型参数住在被 include 进来的外部源码里。

#[allow(dead_code)]
pub mod field_fn;
pub mod noise;
pub mod remap;

pub use field_fn::{FieldFn, Sampled, Upstream};
pub use remap::{Cell, Scale, identity, map_grid, remap_with};

// ⚠ 这里**没有** `SOURCE_HASH` 常量：泛型实例的 key 里覆盖的是**从盘上算的**这一步的源码
//   名册（`px_cook::inst::key` + `px_fingerprint::roster`），不是某个编译期嵌进来的常量。
//   本 crate 的 `build.rs` 仍然必须存在 —— 它给的 `cargo:rerun-if-changed=<每个源文件>`
//   是"改了 alg 就跑得动重编"那条机关（cargo 只对自己 package 的 `src/` 有自动指纹，
//   path 依赖的改动**不会**自动重跑 build script）。
