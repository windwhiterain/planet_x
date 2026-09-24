//! **格点遍历 + 值域映射/钳制**：场域算子共用那一条计算路径。
//!
//! 从 `px_field_op/src/ops/remap.rs` 搬进来的 —— 搬的理由不是"文件太长"，而是
//! **泛型实例要能只靠这一份 rlib 就把体算出来**（`19-generic-inst.md` §177）：
//! 各档之间必须走**同一条**路径，否则同一份参数会算出两种结果，而两者共用的缓存键
//! 分不出这个差别。
//!
//! ⚠ 这一份的算术**逐字**沿自搬出前的那一份（`Remap` 的读数就是靠这一条逐位不变的）：
//!   钳到 `[0,1]` → 平滑（可选）→ 映到 `[out_min, out_max]`。改这里的任何一行，
//!   场侧全部实例的键**一起**变（它们链的是同一个文件）。
//!
//! ## 一处尺子、四个用户
//!
//! | 用户 | 参数类型（住哪） | 谁提供"这一格的值" |
//! |---|---|---|
//! | element 那一档（`elem::Remap` / `elem::Fuse` 的体文件） | `px_elem::RemapParams` / `FuseParams` | 上游那张场，照抄 |
//! | 泛型实例（`art/inst/waves.rs` / `latbands.rs`） | `px_field_schema::params::RemapParams` | 图侧现写的 [`FieldFn`] |
//!
//! 两者共用的那一段是 [`Scale`]（`[in_min, in_max]` → `[0,1]` → `[out_min, out_max]`，带可选平滑）
//! 与**唯一那条循环** [`map_grid`]。⚠ 两套参数类型**不该合并**：element 那一档的尺子由它自己的
//! `art/<图>/<节点>.toml` 给（`in_*` / `out_*` / `smooth` / `gamma`），而泛型那一档的 `gain` /
//! `bias` / `bands` 是**给图侧函数调的**（见 `px_field_schema::params::RemapParams` 为什么单独存在）。
//! 泛型那一档的尺子因此固定为 [`identity`]（`[0,1] → [0,1]`）—— 改 TOML 里的对比度不会
//! 顺手改掉值域口径。
//!
//! ⚠ **搬出去之后预置算法的算术逐位不变**：从前的 `remap::eval` 只是把
//!   `params::remap::Params` 搭成一个 [`Scale`] 再交给同一个 [`map_grid`]；那一档 2026-09-27
//!   收进了 element，而 element 的体文件**搭的是同一个 [`Scale`]、走的是同一个 [`map_grid`]**
//!   —— 于是 `moon` / `desert` 那些图迁移前后的产物逐字节相同（planet 系列图另有一条：
//!   那三档的**默认值**原本就不是一套，见 `44-elem-generic-op.md`）。

use px_field_schema::field::{Field, Projection};
use px_field_schema::params::RemapParams;

use crate::field_fn::FieldFn;

/// **这一格的值怎么算**：给"这个节点的参数"、"已经归一化过的上游值"、"这一格的纹素中心坐标"
/// 与"这一格的球面方向"，回这一格的值。
///
/// ⚠ 有它才有"**同一条计算路径**"：预设那一档与图侧现写的那一档都走 [`map_grid`]，
///   差别只在 `C` 是谁 —— 于是"同一份参数、同一个上游 ⇒ 同一个场"不靠人工同步两份循环，
///   而靠**只有一份循环**。`FieldFn` 自动就是 `Cell`（两者今天逐字同一件事，分开是为了让
///   "这一格的值怎么算"与"图侧那个接口叫什么"各自可以被替换）。
///
/// ⚠ `params` 是**节点的参数**（`art/<图>/<节点名>.toml` 那一份）：2026-09-20 之前它在共享路径
///   里被丢掉（`let _ = params`），于是图侧函数只能读编译期默认值 ⇒ **改参数只换键、不换内容**。
///   今天它是 `Cell` 的第一栏（见 [`crate::field_fn::FieldFn`]）。
/// ⚠ `direction` 是**球面上的单位方向**：行星美术里"纬向条带 / 极冠 / 陨坑"这类函数
///   只能拿它算（`uv` 在 `CubeMap` 投影下不是经纬度）。
pub trait Cell {
    fn value(
        &self,
        params: &RemapParams,
        normalized_upstream: f32,
        uv: [f32; 2],
        direction: [f32; 3],
    ) -> f32;
}

impl<F: FieldFn> Cell for F {
    fn value(
        &self,
        params: &RemapParams,
        normalized_upstream: f32,
        uv: [f32; 2],
        direction: [f32; 3],
    ) -> f32 {
        FieldFn::value(self, params, normalized_upstream, uv, direction)
    }
}

/// 那个**单位**尺子：`[0,1] → [0,1]`，带平滑。
///
/// ⚠ 它正是 [`crate::remap_with`]（泛型实例那一档）用的那一把：`RemapParams` 那三栏
///   （`gain` / `bias` / `bands`）没有值域的意思，"归一化 + 钳制"这一段由恒等尺子跑
///   —— 于是图侧函数只管"这一格的值怎么算"，改 TOML 里的对比度不会顺手改掉值域口径。
///   ⚠ **element 那一档不用它**：`elem::Remap` / `elem::Fuse` 的尺子由函数自己的
///   `RemapParams` / `FuseParams` 给（体文件里现搭一个 [`Scale`]）——两者走的是**同一个**
///   `Scale` 类型与**同一个** [`map_grid`]，只是值域参数不同。
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

/// **唯一那条循环**：按格点遍历**上游那张场的形状**，每一格问上游要 `(值, 坐标, 方向)`、
/// 问 [`Cell`] 要这一格的值，再按尺子映到输出值域。
///
/// ⚠ 输出**与上游同形**（从前由调用点给一张"画布"，还要求它必须与上游同形 ——
///   那条要求现在是结构性的：形状只有一个来源）。
///
/// ⚠ 它不认识"这一格的值是什么"：`cell` 是个口子（预置那一档是"照抄上游"，实例那一档是
///   `art/inst/*.rs` 里那个函数）；"上游从哪来"也是 —— 但今天两条入口的上游**都是**那张
///   采样好的场（[`Sampled`]），这一条与体积域不同，见 [`crate::remap_with`] 的注释。
/// ⚠ `params` 原样递给 `cell`：它是**节点参数到达图侧函数的唯一那条路**
///   （2026-09-20 之前这条路是断的，见 [`crate::field_fn::FieldFn`]）。
pub fn map_grid<C: Cell>(
    scale: &Scale,
    params: &RemapParams,
    gamma: f32,
    upstream: &Field,
    cell: &C,
) -> Field {
    let mut field = upstream.like(0.0);
    // ⚠ **体网格没有"一个方向"这回事**（`Domain::Volume` 的每一格多一维径向层）⇒
    //   `uv` / `direction` 在工作量上是"逐格白算"，在契约上是**当场炸**。
    //   这一档是**逐格值域映射**（点态）：它根本用不到这两个量 ⇒ 体网格上给 (0,0)/+Y 占位。
    //   球面那几个域照旧（行星美术里"纬向条带 / 极冠"那类函数只能拿方向算）。
    let spherical = upstream.projection != Projection::Volume;
    for y in 0..field.height {
        for x in 0..field.width {
            let t = scale.normalize(upstream.at(x, y));
            let (u, v) = if spherical {
                upstream.uv(x, y)
            } else {
                (0.0, 0.0)
            };
            let direction = if spherical {
                upstream.direction(x, y)
            } else {
                [0.0, 1.0, 0.0]
            };
            field.set(
                x,
                y,
                px_field_schema::params::bend(
                    scale.map(cell.value(params, t, [u, v], direction)),
                    gamma,
                ),
            );
        }
    }
    field
}

/// **实例库入口**：用一个**图侧给的**场函数重映射上游那张场
/// （实例的体模板里这样用：
/// `px_field_alg::remap_with(&px_field_alg::identity(), p, 1.0, i.input.value(), g, $arg)`）。
///
/// ⚠ 第三栏 `gamma` 实例那一档固定给 `1.0`（= 不弯，见 [`px_field_schema::params::bend`]）
///   —— 实例的参数结构（`RemapParams`：`gain` / `bias` / `bands`）里**没有**非线性那一栏
///   （那个弯曲是 element 那一档 `elem::RemapParams::gamma` 的事，
///   `px_elem/body/remap.rs` 里自己取幂）。
///   要实例也弯，就在 `RemapParams` 里加一栏、并把这个 `1.0` 换成 `p.gamma`
///   —— 两处必须一起改（否则键变了而算法没变，或反过来）。
///
/// ⚠ 它与 element 那一档的体文件走**同一条**路径（都是 [`map_grid`] 与同一个 [`Scale`]），
///   否则同一份参数会算出两种结果 —— 两条路共用的缓存键分不出这个差别
///   （体积域 `coarse_with` / `eval_sampled` 是同一条规矩）。
///
/// ⚠ `upstream`（上游那张场）**参与计算**：它就是场函数收到的 `upstream` 那一栏
///   （由 [`Sampled`] 采样，与图函数自己再采一次是同一个值）。
///   ⚠ 这一点与体积域的 `coarse_with` **有意不同**：那边具名场函数**自己**就是覆盖度的
///   来源，收 `coverage` 只为让模板逐字套上；场域的"重映射"语义本来就有**上游**，
///   图侧函数是**加在**上游之上的（它拿到 `upstream` 才能谈"对比度/条带"）。
///
/// ⚠ `params` **递给场函数**（`art/<图>/<节点名>.toml` 那一份）：它是"这个节点怎么算"的另一半，
///   与 `scale` 各管一摊 —— `scale` 管归一化/钳制（共享路径），`params` 管图侧函数怎么用它
///   （`gain` / `bias` / `bands`）。⚠ 从前这里写的是 `let _ = params;`，于是**参数只进键、
///   不进计算**（改 TOML 重算出一份逐字节相同的产物）—— 2026-09-20 修掉。
///
/// ⚠ `scale` 是**显式传**的，不从 `params` 推：`RemapParams` 那三栏没有值域的意思
///   （它们是给场函数读的），硬凑一个"参数 → 值域"的映射只会把两件事搅在一起。
///   恒等档就是 [`identity`]。
#[allow(clippy::too_many_arguments)]
pub fn remap_with<F: Cell>(
    scale: &Scale,
    params: &RemapParams,
    gamma: f32,
    upstream: &Field,
    field_fn: &F,
) -> Field {
    map_grid(scale, params, gamma, upstream, field_fn)
}

#[cfg(test)]
mod tests {
    use super::*;
    // ⚠ `Sampled`（"上游照抄"那一档的场函数）在**非测试**代码里已经没有发送者了（element 那一档
    //   在体文件里自己逐格算），判据这一侧才用它当"恒等图侧函数" ⇒ 只在测试模块里引它。
    use crate::field_fn::Sampled;
    use px_field_schema::field::Projection;

    /// 一张 37×23 的对照场（形状就是判据里那一档 —— 画布没了，形状跟着场走）。
    fn sample_field() -> Field {
        Field::filled_with(37, 23, 0.0, Projection::Equirect)
    }

    /// **`gamma` 把中灰压下去、把尖峰留住**（"大片空 + 少数浓"那个形状）。
    ///
    /// ⚠ 这条判的是"非线性真的接上了"，而不是"参数被读了"：`gamma > 1` 必须让**低值掉得
    ///   比高值多**（`0.5³ = 0.125` 掉了 4 倍，而 `0.9³ = 0.729` 只掉 1.2 倍）。
    ///   线性拉伸做不到这件事 —— 那正是为什么单靠收窄 `in_*` 窗口出不来星云
    ///   （实测 fbm 的均值挤在 0.5 附近，收窄之后仍是一片中灰）。
    #[test]
    fn gamma_pushes_the_middle_down_and_keeps_the_peaks() {
        let mut input = Field::filled_with(6, 6, 0.0, Projection::Equirect);
        for x in 0..6 {
            for y in 0..6 {
                input.set(x, y, (x as f32 + y as f32) / 10.0);
            }
        }
        // 上游照抄那一档（`Sampled`）：`gamma` 那一段的对照就只差非线性这一轴。
        let scale = Scale {
            in_min: 0.0,
            in_max: 1.0,
            out_min: 0.0,
            out_max: 1.0,
            smooth: false,
        };
        let remap = |input: &Field, gamma: f32| {
            remap_with(
                &scale,
                &RemapParams::default(),
                gamma,
                input,
                &Sampled { field: input },
            )
        };
        // ⚠ 输出与输入**同形**（形状只有一个来源：上游那张场）。
        let plain = remap(&input, 1.0);
        let bent = remap(&input, 3.0);
        let mut worst_low = f32::INFINITY;
        let mut worst_high = 0.0_f32;
        for index in 0..plain.data.len() {
            let (p, b) = (plain.data[index], bent.data[index]);
            assert!(b <= p + 1e-6, "弯折不该把值抬高：{p} → {b}");
            if p > 0.02 && p < 0.5 {
                worst_low = worst_low.min(b / p);
            }
            if p > 0.55 {
                worst_high = worst_high.max(b / p);
            }
        }
        assert!(
            worst_high > worst_low + 0.2,
            "低值该掉得更多：低值最大比值 {worst_low:.3}，高值最大比值 {worst_high:.3}"
        );
    }

    /// `(值, 坐标)` 的**对照输入**：故意让值越过 `[0,1]` 的边界。
    fn input(field: &Field) -> Field {
        let mut field = field.like(0.0);
        for y in 0..field.height {
            for x in 0..field.width {
                let (u, v) = field.uv(x, y);
                field.set(x, y, (u * 3.0 - v * 1.5).sin() * 0.5 + 0.5);
            }
        }
        field
    }

    /// 这条共享路径**不该碰不该碰的东西** —— 逐位钉两档。
    ///
    /// ⚠ 从前这一条叫 `an_identity_field_function_is_the_preset_entry_bit_for_bit`：那时对照的
    ///   另一半是预置 `Remap` 的入口 `remap_sampled`（"照抄上游"）。那一档 2026-09-27 收进了
    ///   element（`px_elem/body/remap.rs`）⇒ 对照的靶子没了，但这一条钉的东西一个字没变：
    ///   **这条共享路径把输入搬到输出上，除了尺子与 `gamma` 之外一个 ulp 都不许动**。
    ///   两档各钉一半：
    ///
    ///   1. `smooth = false` 的恒等尺子 + `gamma = 1` ⇒ 输出**逐位等于**上游（连值域边界
    ///      `[0,1]` 之外的取样点一起：钳制在两端是幂等的，`0 → 0`、`1 → 1` 逐位成立）；
    ///   2. `identity()`（**带平滑**）+ 照抄上游 ⇒ 输出逐位等于 `map_grid` 里的闭式
    ///      （`3t² − 2t³`）—— 这一半钉的是"平滑确实在共享路径里、且只有这一处"。
    ///
    ///   ⚠ element 那一档的逐格数值由 `px_graphs/tests/elem.rs` 真编库、真装载地钉。
    #[test]
    fn the_shared_path_moves_the_input_without_touching_it() {
        let grid = sample_field();
        let params = RemapParams::default();
        let input = input(&grid);

        // ① 不平滑的恒等尺子：输出就是上游，逐位。
        let plain = Scale {
            in_min: 0.0,
            in_max: 1.0,
            out_min: 0.0,
            out_max: 1.0,
            smooth: false,
        };
        let copied = remap_with(&plain, &params, 1.0, &input, &Sampled { field: &input });
        assert_eq!(
            copied, input,
            "恒等尺子（不平滑）+ 照抄上游，却没算出上游那张场"
        );

        // ② 带平滑的恒等尺子：输出逐位等于"尺子那条闭式"（`map` ∘ `normalize`），并且
        //    **确实与上游不同** —— 否则这一条就退化成"什么也没测"（`identity()` 的 `smooth`
        //    是 `true`，它对绝大多数格子都会真的弯一下）。
        let smooth = identity();
        let bent = remap_with(&smooth, &params, 1.0, &input, &Sampled { field: &input });
        let mut field = input.like(0.0);
        for y in 0..field.height {
            for x in 0..field.width {
                field.set(x, y, smooth.map(smooth.normalize(input.at(x, y))));
            }
        }
        assert_eq!(
            bent, field,
            "平滑那一档与 `Scale::map ∘ Scale::normalize` 那条闭式不是逐位相同"
        );
        assert_ne!(
            bent, input,
            "`identity()` 是 `smooth = true` 的尺子 ⇒ 输出不该与上游逐位相同（尺子没生效？）"
        );
    }

    /// 场函数拿到的那三栏就是**上游归一化后的值**、**这一格的纹素中心坐标**与
    /// **这一格的球面方向**。
    #[test]
    fn the_field_function_sees_the_normalized_upstream_and_the_texel_center() {
        let grid = sample_field();
        let scale = identity();
        let params = RemapParams::default();
        let input = input(&grid);

        let seen = std::cell::RefCell::new(Vec::new());
        struct Record<'a> {
            seen: &'a std::cell::RefCell<Vec<(f32, [f32; 2], [f32; 3])>>,
        }
        impl FieldFn for Record<'_> {
            fn value(
                &self,
                _params: &RemapParams,
                upstream: f32,
                uv: [f32; 2],
                direction: [f32; 3],
            ) -> f32 {
                self.seen.borrow_mut().push((upstream, uv, direction));
                upstream
            }
        }
        let _ = remap_with(&scale, &params, 1.0, &input, &Record { seen: &seen });

        let seen = seen.into_inner();
        assert_eq!(seen.len(), (grid.width * grid.height) as usize);
        // 坐标 = `Field::uv`（纹素中心），值 = 上游过了**同一把尺子**，方向 = `Field::direction`。
        let sample = seen[(2 * grid.width + 3) as usize];
        let (u, v) = input.uv(3, 2);
        assert_eq!(sample.1, [u, v], "坐标不是纹素中心口径");
        assert_eq!(
            sample.0,
            scale.normalize(input.at(3, 2)),
            "场函数拿到的值不是'上游过同一把尺子'之后的"
        );
        assert_eq!(
            sample.2,
            input.direction(3, 2),
            "场函数拿到的方向不是这一格的球面方向（纬向条带这类函数只能拿它算）"
        );
    }

    /// **节点参数必须到得了图侧函数**（2026-09-20 修的那个缺陷）。
    ///
    /// 病：`remap_with` 里那句 `let _ = params;` 把参数丢在了共享路径上，而图侧函数只能自己读
    /// `RemapParams::default()` ⇒ 改 `art/<图>/<节点>.toml` **只换节点键、不换内容**
    /// （实测 `bands = 3 / gain = 0` 与不写文件写出同一份字节）。
    /// 这一条把那条路钉住：场函数回什么，输出就得是什么。
    #[test]
    fn the_node_params_reach_the_field_function() {
        let grid = sample_field();
        let scale = identity();
        let input = input(&grid);
        let params = RemapParams {
            gain: 0.0,
            bias: 0.25,
            bands: 5.0,
        };

        struct ReadsParams;
        impl FieldFn for ReadsParams {
            fn value(
                &self,
                params: &RemapParams,
                _upstream: f32,
                _uv: [f32; 2],
                _direction: [f32; 3],
            ) -> f32 {
                params.bias
            }
        }

        // 恒等尺子 ⇒ 输出就是场函数回的那个数（每一格都一样）。
        let field = remap_with(&scale, &params, 1.0, &input, &ReadsParams);
        assert!(
            field.data.iter().all(|value| (value - 0.25).abs() < 1e-9),
            "节点参数没到图侧函数（输出不是 `bias`）"
        );
        // 换一个值必须换内容 —— 键本来就跟着它换，两者不许再分家。
        let other = RemapParams {
            bias: 0.75,
            ..params
        };
        let field = remap_with(&scale, &other, 1.0, &input, &ReadsParams);
        assert!(
            field.data.iter().all(|value| (value - 0.75).abs() < 1e-9),
            "换了参数内容却没换"
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
