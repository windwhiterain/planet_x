//! **element 函数的规格表** —— 一行一个函数，唯一那份清单。
//!
//! ⚠ 加一个 element 函数 = **这里加一行** + 写两个文件（`src/<函数>.rs` 参数与上游、
//!   `body/<函数>.rs` 算法）。
//! ⚠ **体文件住在 `body/` 下**（不在 `src/`）：它不进本 crate 的源码指纹 ⇒ 改一行算法
//!   只换**那一条**实例的键、只重编那一份库（R1）。
//!
//! 每行的五栏是：**类型名**（图脚本写 `elem::<它>`）、**参数类型**、**上游类型**、
//! **人读名**，以及 `source` / `roots` / `shape` 三条。
//! ⚠ `shape` 拿得到参数与上游两样：生成类从参数来（`params.shape`），过滤类从上游来
//!   （`inputs.<上游>.value()` 那张场的宽高与投影）—— "输出与上游同形"因此是**构造上**的。

use crate::constant::ConstantParams;
use crate::fuse::{FuseInput, FuseParams};
use crate::mix::{MixInput, MixParams};
use crate::remap::{RemapInput, RemapParams};

crate::px_elem_specs! {
    /// 一整张常值场：一条链的起点，也是"最简 element 函数"那一档（参数只有形状与值）。
    Constant, ConstantParams, (), "field.constant",
        source: "px_elem/body/constant.rs", roots: &[],
        shape: |params: &ConstantParams, _inputs: &()| params.shape;

    /// 按权重场在两份场之间插值（`a·(1-w) + b·w`）。
    Mix, MixParams, MixInput, "field.mix",
        source: "px_elem/body/mix.rs", roots: &[],
        shape: |_params: &MixParams, inputs: &MixInput| crate::like(inputs.a.value());

    /// 把上游的值域钳/归一化、映到新值域、再按 `gamma` 弯一次。
    Remap, RemapParams, RemapInput, "field.remap",
        source: "px_elem/body/remap.rs", roots: &["px_field_alg"],
        shape: |_params: &RemapParams, inputs: &RemapInput| crate::like(inputs.field.value());

    /// **融合**：`remap` 之后接着 `mix`，一个循环算完（三个上游、一份产物、上游只读一次）。
    Fuse, FuseParams, FuseInput, "field.fuse",
        source: "px_elem/body/fuse.rs", roots: &["px_field_alg"],
        shape: |_params: &FuseParams, inputs: &FuseInput| crate::like(inputs.a.value());
}
