//! **格点遍历 + 值域映射/钳制**：场域算子共用那一条计算路径。
//!
//! 从 `px_field_op/src/ops/remap.rs` 搬进来的 —— 搬的理由不是"文件太长"，而是
//! **泛型实例要能只靠这一份 rlib 就把体算出来**（`19-generic-inst.md` §177）：
//! 预设那一档（`px_field_op` 的 `Remap`）与图侧现写的那一档（`px_inst!` 的实例）
//! 必须走**同一条**路径，否则同一份参数会算出两种结果，而两者共用的缓存键分不出这个差别。
//!
//! ⚠ 这一份的算术**逐字**沿自搬出前的那一份（`Remap` 的读数就是靠这一条逐位不变的）：
//!   钳到 `[0,1]` → 平滑（可选）→ 映到 `[out_min, out_max]`。改这里的任何一行，
//!   预设算子与全部场侧实例的键**一起**变（它们链的是同一个文件）。
//!
//! ## 两条入口、两套参数、**一份**尺子
//!
//! | 入口 | 参数类型（住 schema） | 谁提供"这一格的值" |
//! |---|---|---|
//! | [`remap_sampled`]（预置 `Remap`） | `params::remap::Params` | 上游那张场，照抄 |
//! | [`remap_with`]（`px_inst!` 的实例） | `params::RemapParams` | 图侧现写的 [`FieldFn`] |
//!
//! 两者共用的那一段是 [`Scale`]（`[in_min, in_max]` → `[0,1]` → `[out_min, out_max]`，带可选平滑）
//! 与**唯一那条循环** [`map_grid`]。⚠ 两套参数类型**不该合并**：预置那一档的尺子由它自己的
//! `art/<图>/<节点>.toml` 给（`in_*` / `out_*` / `smooth`），而泛型那一档的 `gain` / `bias` /
//! `bands` 是**给图侧函数调的**（见 `px_field_schema::params::RemapParams` 为什么单独存在）。
//! 泛型那一档的尺子因此固定为 [`identity`]（`[0,1] → [0,1]`）—— 改 TOML 里的对比度不会
//! 顺手改掉值域口径。
//!
//! ⚠ **搬出去之后预置算法的算术逐位不变**：`remap::eval` 只是把 `params::remap::Params`
//!   搭成一个 [`Scale`] 再交给同一个 [`map_grid`]。planet/desert 那 14 份产物与搬出去之前
//!   **逐字节相同**，就是这条的判据。

use px_field_schema::field::{Field, GridField};
use px_field_schema::params::RemapParams;
use px_graph_schema::Grid;

use crate::field_fn::{FieldFn, Sampled};

/// **这一格的值怎么算**：给"已经归一化过的上游值"与"这一格的纹素中心坐标"，回这一格的值。
///
/// ⚠ 有它才有"**同一条计算路径**"：预设那一档与图侧现写的那一档都走 [`map_grid`]，
///   差别只在 `C` 是谁 —— 于是"同一份参数、同一个上游 ⇒ 同一个场"不靠人工同步两份循环，
///   而靠**只有一份循环**。`FieldFn` 自动就是 `Cell`（两者今天逐字同一件事，分开是为了让
///   "这一格的值怎么算"与"图侧那个接口叫什么"各自可以被替换）。
pub trait Cell {
    fn value(&self, normalized_upstream: f32, uv: [f32; 2]) -> f32;
}

impl<F: FieldFn> Cell for F {
    fn value(&self, normalized_upstream: f32, uv: [f32; 2]) -> f32 {
        FieldFn::value(self, normalized_upstream, uv)
    }
}

/// 那个**单位**尺子：`[0,1] → [0,1]`，带平滑。
///
/// ⚠ 它正是 [`crate::remap_with`]（泛型实例那一档）用的那一把：`RemapParams` 那三栏
///   （`gain` / `bias` / `bands`）没有值域的意思，"归一化 + 钳制"这一段由恒等尺子跑
///   —— 于是图侧函数只管"这一格的值怎么算"，改 TOML 里的对比度不会顺手改掉值域口径。
///   ⚠ **预置 `Remap` 不用它**：预置那一档的尺子由 `params::remap::Params` 给
///   （`px_field_op::ops::remap::eval` 现搭一个 [`Scale`]）——两者走的是**同一个** `Scale`
///   类型与**同一个** [`map_grid`]，只是值域参数不同。
pub fn identity() -> Scale {
    Scale {
        in_min: 0.0,
        in_max: 1.0,
        out_min: 0.0,
        out_max: 1.0,
        smooth: true,
    }
}

/// 值域那一把尺子：`[in_min, in_max] → [0,1] → [out_min, out_max]`。
///
/// ⚠ 这一份从 `px_field_op` 的 `Remap` 里搬出来时**逐字照抄**（连"退化区间 ⇒ `0`"这条边界
///   口径一起）：预设算子与场侧实例共用它 ⇒ 两条路不会各漂各的。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scale {
    pub in_min: f32,
    pub in_max: f32,
    pub out_min: f32,
    pub out_max: f32,
    pub smooth: bool,
}

impl Scale {
    /// 把上游值钳到 `[in_min, in_max]` 并归一化到 `[0,1]`（可选平滑）。
    ///
    /// ⚠ 退化区间（`in_max == in_min`）回 `0` —— 与搬出前那一份同一个口径（是 `0`，不是 `1`）。
    pub fn normalize(&self, value: f32) -> f32 {
        let span = self.in_max - self.in_min;
        let inv_span = if span.abs() < f32::EPSILON {
            0.0
        } else {
            1.0 / span
        };
        let mut t = ((value - self.in_min) * inv_span).clamp(0.0, 1.0);
        if self.smooth {
            t = t * t * (3.0 - 2.0 * t);
        }
        t
    }

    /// 把 `[0,1]` 里的 `t` 映到 `[out_min, out_max]`。
    ///
    /// ⚠ **不钳制**：`out_min` / `out_max` 是算子作者写的值域，搬出前那一份也是照直映的
    ///   —— 擅自在末尾加一次 `clamp(0,1)` 会让 `out_*` 取到 `[0,1]` 之外的图**静默**换产物
    ///   （搬出前它映到 `[-1,2]` 就是 `[-1,2]`）。"算出来必须落在 `[0,1]`"是**图侧函数**
    ///   自己的承诺（`art/inst/*.rs` 末尾那次 `clamp`），不是这把尺子替它兜的底。
    pub fn map(&self, t: f32) -> f32 {
        self.out_min + t * (self.out_max - self.out_min)
    }
}

/// **唯一那条循环**：按格点遍历 `grid`，每一格问上游要 `(值, 坐标)`、问 [`Cell`] 要这一格的值，
/// 再按尺子映到输出值域。
///
/// ⚠ 它不认识"这一格的值是什么"：`cell` 是个口子（预置那一档是"照抄上游"，实例那一档是
///   `art/inst/*.rs` 里那个函数）；"上游从哪来"也是 —— 但今天两条入口的上游**都是**那张
///   采样好的场（[`Sampled`]），这一条与体积域不同，见 [`crate::remap_with`] 的注释。
pub fn map_grid<C: Cell>(scale: &Scale, upstream: &Field, cell: &C, grid: Grid) -> Field {
    let mut field = grid.filled(0.0);
    for y in 0..grid.height {
        for x in 0..grid.width {
            let t = scale.normalize(upstream.at(x, y));
            let (u, v) = upstream.uv(x, y);
            field.set(x, y, scale.map(cell.value(t, [u, v])));
        }
    }
    field
}

/// **预设那一档的入口**：上游那张场照抄、只过尺子。
///
/// ⚠ 它就是 [`remap_with`] 在"恒等场函数"上的实例 —— `px_field_op` 的 `Remap` 调的就是它
///   （**没有第二条计算路径**）。
///
/// ⚠ 它收 [`Scale`] 而**不收** `RemapParams`：预置那一档的尺子是它自己那份
///   `params::remap::Params` 给的，`RemapParams`（`gain` / `bias` / `bands`）是**给图侧函数
///   读的**、在共享路径里不参与 —— 硬收一个"用不到的参数"只会让签名骗人。
pub fn remap_sampled(scale: &Scale, input: &Field, grid: Grid) -> Field {
    remap_with(scale, &RemapParams::default(), input, grid, &Sampled { field: input })
}

/// **实例库入口**：用一个**图侧给的**场函数重映射上游那张场
/// （`px_inst!` 的体模板里这样用：
/// `px_field_alg::remap_with(&px_field_alg::identity(), p, i.input.value(), g, $arg)`）。
///
/// ⚠ 它必须与 [`remap_sampled`] 走**同一条**路径（都是 [`map_grid`]），否则同一份参数会算出
///   两种结果 —— 预设实现那一档与泛型实例那一档就再也对不上，而两者共用的缓存键分不出这个
///   差别（体积域 `coarse_with` / `eval_sampled` 是同一条规矩）。
///
/// ⚠ `upstream`（上游那张场）**参与计算**：它就是场函数收到的 `upstream` 那一栏
///   （由 [`Sampled`] 采样，与图函数自己再采一次是同一个值）。
///   ⚠ 这一点与体积域的 `coarse_with` **有意不同**：那边具名场函数**自己**就是覆盖度的
///   来源，收 `coverage` 只为让模板逐字套上；场域的"重映射"语义本来就有**上游**，
///   图侧函数是**加在**上游之上的（它拿到 `upstream` 才能谈"对比度/条带"）。
///
/// ⚠ `scale` 是**显式传**的，不从 `params` 推：`RemapParams` 那三栏没有值域的意思
///   （它们是给场函数读的），硬凑一个"参数 → 值域"的映射只会把两件事搅在一起。
///   恒等档就是 [`identity`]。
#[allow(clippy::too_many_arguments)]
pub fn remap_with<F: Cell>(
    scale: &Scale,
    params: &RemapParams,
    upstream: &Field,
    grid: Grid,
    field_fn: &F,
) -> Field {
    // ⚠ `params` 今天**不参与**共享路径：归一化与钳制是尺子的事，`gain` / `bias` / `bands`
    //   是场函数自己读的（`art/inst/*.rs` 里 `RemapParams::default()` 那一行）。
    //   收它是为了两件事：① 让"这一档的图参数"在签名里明明白白（图侧 `cook` 给的就是它）；
    //   ② 与被复用的那条声明（`FieldRemap` 的 `Params`）对齐。
    let _ = params;
    map_grid(scale, upstream, field_fn, grid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use px_field_schema::field::Projection;

    fn grid() -> Grid {
        Grid {
            width: 37,
            height: 23,
            projection: Projection::Equirect,
        }
    }

    /// `(值, 坐标)` 的**对照输入**：故意让值越过 `[0,1]` 的边界。
    fn input(grid: Grid) -> Field {
        let mut field = grid.filled(0.0);
        for y in 0..grid.height {
            for x in 0..grid.width {
                let (u, v) = field.uv(x, y);
                field.set(x, y, (u * 3.0 - v * 1.5).sin() * 0.5 + 0.5);
            }
        }
        field
    }

    /// 恒等的图侧函数（`(upstream, uv) ↦ upstream`）与预设入口必须**逐个 f32 相同** ——
    /// 那是"同一条计算路径"的判据（两条入口共用 `map_grid`）。
    #[test]
    fn an_identity_field_function_is_the_preset_entry_bit_for_bit() {
        let grid = grid();
        let scale = identity();
        let params = RemapParams::default();
        let input = input(grid);

        let preset = remap_sampled(&scale, &input, grid);
        let generic = remap_with(&scale, &params, &input, grid, &Sampled { field: &input });
        assert_eq!(preset, generic, "同一条路径上两种入口算出了不同的场");
    }

    /// 场函数拿到的那两栏就是**上游归一化后的值**与**这一格的纹素中心坐标**。
    #[test]
    fn the_field_function_sees_the_normalized_upstream_and_the_texel_center() {
        let grid = grid();
        let scale = identity();
        let params = RemapParams::default();
        let input = input(grid);

        let seen = std::cell::RefCell::new(Vec::new());
        struct Record<'a> {
            seen: &'a std::cell::RefCell<Vec<(f32, [f32; 2])>>,
        }
        impl FieldFn for Record<'_> {
            fn value(&self, upstream: f32, uv: [f32; 2]) -> f32 {
                self.seen.borrow_mut().push((upstream, uv));
                upstream
            }
        }
        let _ = remap_with(&scale, &params, &input, grid, &Record { seen: &seen });

        let seen = seen.into_inner();
        assert_eq!(seen.len(), (grid.width * grid.height) as usize);
        // 坐标 = `Field::uv`（纹素中心），值 = 上游过了**同一把尺子**。
        let sample = seen[(2 * grid.width + 3) as usize];
        let (u, v) = input.uv(3, 2);
        assert_eq!(sample.1, [u, v], "坐标不是纹素中心口径");
        assert_eq!(
            sample.0,
            scale.normalize(input.at(3, 2)),
            "场函数拿到的值不是'上游过同一把尺子'之后的"
        );
    }

    /// 尺子本身：退化区间回 `0`、平滑确实弯了中点、`out_*` 确实缩放。
    #[test]
    fn the_scale_keeps_the_pre_move_arithmetic() {
        let plain = Scale {
            in_min: 0.0,
            in_max: 1.0,
            out_min: 0.0,
            out_max: 1.0,
            smooth: false,
        };
        assert_eq!(plain.normalize(-3.0), 0.0);
        assert_eq!(plain.normalize(0.25), 0.25);
        assert_eq!(plain.normalize(3.0), 1.0);
        assert_eq!(plain.map(0.25), 0.25);

        // 退化区间：`0`（不是 `1`）—— 搬出前那一份的口径。
        let degenerate = Scale {
            in_min: 0.5,
            in_max: 0.5,
            ..plain
        };
        assert_eq!(degenerate.normalize(0.5), 0.0);

        // 平滑：中点不变、两端不变，四分之一处被推离 0.5（`3t² - 2t³`）。
        let smooth = Scale {
            smooth: true,
            ..plain
        };
        assert_eq!(smooth.normalize(0.5), 0.5);
        assert!((smooth.normalize(0.25) - 0.15625).abs() < 1e-6);

        // 输出值域：照直映，**不钳制**（搬出前那一份也是照直映的）。
        let wide = Scale {
            out_min: -1.0,
            out_max: 2.0,
            ..plain
        };
        assert_eq!(wide.map(0.0), -1.0);
        assert_eq!(wide.map(1.0), 2.0);
        assert_eq!(wide.map(0.5), 0.5);
    }
}
