//! 逐条 pass 的 **GPU 编码器级时间戳**读数 —— J4 的仪器。
//!
//! ## 它量的是什么（一个名字一个意思，§104 第 4 条那条教训）
//!
//! 每一帧里**每条声明过的 pass 各写两对**时间戳（槽的下标由 `px_pass::PassTimestamps` 定，
//! 写的人与读的人调的是同一个函数）：
//!
//! | 名字 | 边界 | 与谁可比 |
//! |---|---|---|
//! | `inside_ms` | pass **里面**的第一条/最后一条命令 | **Bevy 的 `elapsed_gpu` 就是这一套**：`bevy_render-0.19.1/src/diagnostic/internal.rs:455-478` 的 `begin_pass`/`end_pass` 都是 `pass.write_timestamp` |
//! | `envelope_ms` | 含 `vkCmdBegin/EndRenderPass` 的那一对（`RenderPassTimestampWrites`，`wgpu-hal-29.0.4/src/vulkan/command.rs:883`/`:918`） | 谁都不是：它是**口径差**，专门用来量"边界那点开销值多少毫秒" |
//! | `frame_ms` | 整条编码器：宿主目标清屏 → 最后一格的最后一条 pass | 宿主这一帧 GPU 忙时的**下界**（回读那一次拷贝在另一条提交里，不算） |
//!
//! ⚠⚠ **不许把这个数叫 `gpu_ms`。** Bevy 报告里的 `gpu_ms` 是 `render/**/elapsed_gpu`
//! **七段之和**（§147.4 记下来的 `source` 字符串），而这里报的是**我们文档里那些 pass 的
//! 编码器级 span**。同一个名字装两个不同的量，正是 §147 记下的那个陷阱：
//! *"一个数放进一个语义不同的字段里，比没有这个数坏"* ⇒ 它有自己的名字。
//!
//! ## 对照表是**政策**，所以它写死在仪器里、而且**没有缺省**
//!
//! 见 [`SPAN_MATCH`]：每一条都在注释里指得出 Bevy 那一侧的**出处**（哪一条 pass、哪一行）。
//! 表里没有的标签 ⇒ [`bevy_span_of`] 返回 `Err`（**当场拒**），不是"没有对应"：
//! 一个**算错的匹配子集**比没有子集更坏 —— 而"表里没有"与"查过了、Bevy 没有这一段"
//! 是两件事，它们必须长得不一样。
//!
//! ## 稳定化（§147.4：参照图自己在一次会话里漂过 4.5×）
//!
//! 预热帧与测量帧**走同一条路**（同一份代码、同样写时间戳），所以"时钟被拉起来"
//! 这件事在两段里是同一件；测量帧的读数由调用方按 p50/min/max 报，**漂移照实报**。
//!
//! ⚠ 帧级那两格的定义在 `px_pass::TIMESTAMP_FRAME_SLOTS`：它属于**排布**（谁排谁读），
//! 这里只排布、不重复定义一遍 —— 两个地方各写一份"两格"就是两处会漂开的真相。

/// 一条 pass 标签 → Bevy 那一段的对照。**这是一张政策表，不是推导出来的。**
pub struct SpanMatch {
    /// 我们文档里那条 pass 的 `label`。
    pub label: &'static str,
    /// Bevy 那七段里的名字。`None` = **查过 Bevy 源码了，它没有这一段**（不是"没查"）。
    pub bevy: Option<&'static str>,
    /// 凭什么这么说：一手源码的出处（相对 cargo registry 的路径 + 行号）。
    pub why: &'static str,
}

/// 我们的 pass 标签 → Bevy 那七个 span。**七条，一条不缺**（`point_shadow` 也在里面，
/// 它对应 `None` —— 那也是这张表必须说的话）。
///
/// ⚠ 两个"没有对应"的性质**不一样**，注释里分开了：
/// - `copy_depth`：Bevy **自己也做这件事**，但它把那条拷贝放在 `pass_span.end()` **之后**
///   ⇒ 两边都不在任何 span 里（对称地缺席）；
/// - `point_shadow`：Bevy 画影子图时**只有一条 CPU 的 `info_span!`**，不写 GPU 诊断
///   ⇒ 它的 `gpu_ms` 里**根本不含**影子图那一笔（这是"Bevy 的读数少了一块"，
///   不是"我们多了一块"）。
pub const SPAN_MATCH: &[SpanMatch] = &[
    SpanMatch {
        label: "prepass",
        bevy: Some("early prepass"),
        why: "bevy_core_pipeline-0.19.1/src/prepass/node.rs:82（early_prepass 传给 run_prepass_system 的 label）",
    },
    SpanMatch {
        label: "copy_depth",
        bevy: None,
        why: "bevy_core_pipeline-0.19.1/src/prepass/node.rs:249：Bevy 自己的深度拷贝在 pass_span.end()（:242）之后 ⇒ 两边都不在 span 里",
    },
    SpanMatch {
        label: "point_shadow",
        bevy: None,
        why: "bevy_pbr-0.19.1/src/render/light.rs:2880：只有 CPU 的 info_span!，没有 pass_span ⇒ Bevy 的 gpu_ms 不含影子图",
    },
    SpanMatch {
        label: "opaque",
        bevy: Some("main_opaque_pass_3d"),
        why: "bevy_core_pipeline-0.19.1/src/core_3d/main_opaque_pass_3d_node.rs:75",
    },
    SpanMatch {
        label: "sky",
        bevy: Some("main_opaque_pass_3d"),
        why: "bevy_core_pipeline-0.19.1/src/core_3d/main_opaque_pass_3d_node.rs:99-107：天空盒**画在同一条 render pass 里**",
    },
    SpanMatch {
        label: "transparent",
        bevy: Some("main_transparent_pass_3d"),
        why: "bevy_core_pipeline-0.19.1/src/core_3d/main_transparent_pass_3d_node.rs:87",
    },
    SpanMatch {
        label: "blit",
        bevy: Some("upscaling"),
        why: "bevy_core_pipeline-0.19.1/src/upscaling/node.rs:86（time_span，编码器级）",
    },
];

/// Bevy 那七段里**没有我们这一侧**的三段（宿主按构造不做的活）。
///
/// ⚠ 它们在 `pair`（**两档之差**）里是**公共项**：两档都要做 GPU 预处理与聚类
/// ⇒ 差值里大体相消。这条推理成立的前提写在 [`crate::main`] 的 `--spans` 输出里
/// （两档的物体数/灯数要摆出来），因为"相消"是个**假定**，得让人能自己核。
pub const BEVY_ONLY: &[(&str, &str)] = &[
    (
        "bin_unpacking",
        "bevy_pbr-0.19.1/src/render/gpu_preprocess.rs:545（compute）",
    ),
    (
        "early_mesh_preprocessing",
        "bevy_pbr-0.19.1/src/render/gpu_preprocess.rs:630（compute）",
    ),
    (
        "clustering",
        "bevy_pbr-0.19.1/src/cluster/gpu.rs:872（编码器级 time_span）",
    ),
];

/// 这一条标签在 Bevy 侧对应哪一段。
///
/// - `Ok(Some(name))`：对应**那七段**里的 `name`；
/// - `Ok(None)`：查过了，Bevy 没有这一段（见 [`SPAN_MATCH`] 的注释）；
/// - `Err`：**这张表里没有这条标签** —— 那是一个表洞，不许当成"没有对应"。
pub fn bevy_span_of(label: &str) -> Result<Option<&'static str>, String> {
    if let Some(entry) = SPAN_MATCH.iter().find(|entry| entry.label == label) {
        return Ok(entry.bevy);
    }
    // ⚠ 影子那一族是**按形状**认的，不是前缀匹配：`point_shadow_0_+x` 这一族说得出来
    //    （灯号是数字、面是那六个之一），而"形状说不出来"的标签照旧当场拒。
    //    为什么不能只做前缀匹配：那会让"表里没有"与"查过了、Bevy 没有这一段"**又长得一样**
    //    —— 而上面那条注释正是不许这样。
    if let Some(family) = SPAN_FAMILIES.iter().find(|family| (family.matches)(label)) {
        return Ok(family.bevy);
    }
    Err(format!(
        "pass '{label}' 不在这张对照表里（表里只有：{}；另有一族 {}）—— \
         匹配子集因此算不出来，而**算错的子集比没有子集更坏**。\
         要加一条，得先在 `spans.rs::SPAN_MATCH` 里指得出 Bevy 那一侧的出处",
        SPAN_MATCH
            .iter()
            .map(|entry| entry.label)
            .collect::<Vec<_>>()
            .join(" / "),
        SPAN_FAMILIES
            .iter()
            .map(|family| family.shape)
            .collect::<Vec<_>>()
            .join(" / ")
    ))
}

/// 一条**按形状**认的标签家族（不是前缀匹配：形状要说得出来，说不出来的照旧拒）。
pub struct SpanFamily {
    /// 形状的人话名字（进 `[span-map]` 那一行）。
    pub shape: &'static str,
    pub bevy: Option<&'static str>,
    pub why: &'static str,
    /// 形状本身。
    pub matches: fn(&str) -> bool,
}

/// 一族：**点光 cube 影图的六个面**（`point_shadow_<灯号>_<面>`）。
///
/// 为什么它是一族而不是表里的六条：灯的**个数**是内容（这一帧有几盏投影的灯），
/// 而六面是**渲染器的常数** —— 把 `point_shadow_0_+x` 到 `point_shadow_7_-z` 逐条写进表里
/// 是把"内容的势"抄进政策表；按形状认才是那句"这一族我查过"。
///
/// ⚠ 它对应 `None`（Bevy 侧没有这一段），理由与基标签 `point_shadow` 那条**同一条**：
/// `bevy_pbr-0.19.1/src/render/light.rs:2880` 只有 CPU 的 `info_span!`，不写 GPU 诊断。
pub const SPAN_FAMILIES: &[SpanFamily] = &[SpanFamily {
    shape: "point_shadow_{灯号}_{面}",
    bevy: None,
    why: "bevy_pbr-0.19.1/src/render/light.rs:2880（同 `point_shadow`：只有 CPU info_span!）",
    matches: is_point_shadow_face,
}];

/// `point_shadow_<灯号>_<面>`：灯号是非空数字、面是那六个之一。**多一段、少一段都不算**。
fn is_point_shadow_face(label: &str) -> bool {
    let Some(rest) = label.strip_prefix("point_shadow_") else {
        return false;
    };
    let Some((light, face)) = rest.split_once('_') else {
        return false;
    };
    !light.is_empty()
        && light.bytes().all(|byte| byte.is_ascii_digit())
        && matches!(face, "+x" | "-x" | "+y" | "-y" | "+z" | "-z")
}

/// **一帧**的时间戳槽：帧级一对 + 每条 pass 四格。
///
/// ⚠ 单张那条路才有它：12 格的对照图**不排槽**（`Session::draw_stamps` 当场拒）——
/// 12 格 × 每条 pass 四格是"12 份 span"，与"一帧一条 pass 一份 span"不是同一个量，
/// 而判据要比的正是后者。
///
/// ⚠⚠ **每帧用的是同一段格**（第 0 帧的格 = 第 1 帧的格）：这一帧的值在**这一帧的编码器
/// 里**就被 resolve 走了（[`FrameStamps::resolve_now`]）。这不是省事，是**实测**出来的形状：
/// 一帧一帧写进同一张查询集、事后再另起一条提交去 resolve，读到的是**全 0**
/// （见 `Recorder` 顶上那段）。
pub struct FrameStamps<'a> {
    query_set: &'a wgpu::QuerySet,
    /// 每条 pass 的格（**由 `px_pass::frame_slots` 排一次**，执行器拿的就是它）。
    layout: &'a [px_pass::PassSlots],
    /// 帧级那一对（在**这一帧那段格**里的位置）。
    frame: (u32, u32),
    /// 这一帧的格 resolve 到哪一块（每帧一块，偏移按 `QUERY_RESOLVE_BUFFER_ALIGNMENT` 对齐）。
    resolve: &'a wgpu::Buffer,
    resolve_offset: u64,
}

impl<'a> FrameStamps<'a> {
    /// 这一帧一共几格。
    pub fn stride(&self) -> u32 {
        self.frame.1 + 1
    }

    /// **一份计划要几格**（不用建 FrameStamps 也能算：拒词里要报出来）。
    pub fn stride_of(passes: u32) -> u32 {
        px_pass::TIMESTAMP_SLOTS_PER_PASS * passes + px_pass::TIMESTAMP_FRAME_SLOTS
    }

    /// 这一帧有几条 pass（槽是按它排的）。
    pub fn passes(&self) -> u32 {
        self.layout.len() as u32
    }

    /// 帧级 `begin`：**宿主目标清屏之前**（含清屏才是"这一帧 GPU 忙时"）。
    pub fn begin_frame(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.write_timestamp(self.query_set, self.frame.0);
    }

    /// 帧级 `end`：最后一格的最后一条 pass **之后**。
    pub fn end_frame(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.write_timestamp(self.query_set, self.frame.1);
    }

    /// **把这一帧那几格当场搬走**（写在同一条编码器里，紧跟在 `end_frame` 之后）。
    ///
    /// ⚠ 这一步是这台仪器能不能出数的分界（实测，见 `Recorder`）：查询集**每条命令缓冲
    /// 用完就还**（wgpu-core 在每条 pass 的开头为它要用的那几格发一条
    /// `vkCmdResetQueryPool`，`wgpu-core-29.0.4/src/command/render.rs:2300`）——
    /// 于是"先写完所有帧、最后再 resolve"这个看着更省事的形状会读到全 0。
    pub fn resolve_now(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.resolve_query_set(
            self.query_set,
            0..self.stride(),
            self.resolve,
            self.resolve_offset,
        );
    }

    /// 交给执行器的那一份（**同一张排布**：写的人与读的人不可能漂开）。
    pub fn executor_view(&self) -> px_pass::PassTimestamps<'_> {
        px_pass::PassTimestamps::new(self.query_set, self.layout)
    }

    /// 第 `index` 条 pass 的格。
    pub fn pass_slots(&self, index: usize) -> Option<px_pass::PassSlots> {
        self.layout.get(index).copied()
    }
}

/// 这一台设备缺哪一个 feature 就量不了（空 = 三个都有）。
///
/// ⚠ 缺了**不退化**：宿主当场拒。理由与 §147 那条一样 —— 少一套边界就变成
/// "一个数放进一个语义不同的字段里"。`TIMESTAMP_QUERY_INSIDE_PASSES` 是 `inside` 那
/// 一套的前提（`wgpu-core-29.0.4/src/command/query.rs:410` 要的是 `INSIDE_ENCODERS`，
/// 而 `RenderPass::write_timestamp` 要的是 `INSIDE_PASSES`）。
pub fn missing_features(adapter: &wgpu::Adapter) -> Vec<&'static str> {
    let features = adapter.features();
    let mut missing = Vec::new();
    for (feature, name) in [
        (wgpu::Features::TIMESTAMP_QUERY, "TIMESTAMP_QUERY"),
        (
            wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS,
            "TIMESTAMP_QUERY_INSIDE_ENCODERS",
        ),
        (
            wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES,
            "TIMESTAMP_QUERY_INSIDE_PASSES",
        ),
    ] {
        if !features.contains(feature) {
            missing.push(name);
        }
    }
    missing
}

/// 一个读数的三格：p50 / 最小 / 最大（毫秒）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stats {
    pub p50: f64,
    pub min: f64,
    pub max: f64,
}

impl Stats {
    fn of(mut values: Vec<f64>) -> Stats {
        values.sort_by(|one, two| one.partial_cmp(two).expect("毫秒不会是 NaN"));
        let mid = values.len() / 2;
        // 偶数个取**下中位**（与 Bevy 那份 `quantile` 同一套：`px_render/src/main.rs:355`
        // 用的是同一个下标算法）—— 取哪一个中位是**口径**，换了就不是同一个数。
        Stats {
            p50: values[mid],
            min: values[0],
            max: values[values.len() - 1],
        }
    }
}

/// 一条 pass 的读数。
pub struct PassReading {
    pub index: usize,
    pub label: String,
    pub kind: &'static str,
    pub bevy: Option<&'static str>,
    pub envelope: Stats,
    /// `None` = 这条 pass 不开 render pass（copy）⇒ 它没有"pass 内"那一对。
    pub inside: Option<Stats>,
}

/// 一份完整读数：**一轮**（一个"预热 + 测量"块）的全部产物。
pub struct Reading {
    /// 第几轮（0 起）。
    pub block: u32,
    /// 预热帧数：**不进读数**（它们只用来把时钟拉起来）。
    pub warm: u32,
    /// **进读数**的帧数（不含预热）。
    pub frames: u32,
    pub passes: Vec<PassReading>,
    /// 每帧：**匹配子集**（`inside` 那一套，Bevy 侧有对应的那些 pass 之和）的 p50。
    ///
    /// ⚠ 这是**逐帧先求和、再在帧之间取中位** —— 与 Bevy 的 `gpu_ms.p50` 同一套口径
    /// （`px_render/src/main.rs:2792` 那一带：先按帧求和，再对帧取分位）。
    /// 把每条 pass 各自的 p50 加起来是**另一个量**（分布的分位不满足可加性），
    /// 所以两个数分别报，谁也不冒名顶替谁。
    pub matched: Stats,
    /// 每帧：**全部**声明过的 pass 之和（多含 `copy_depth`；有影子时还含 `point_shadow`）。
    pub inside_all: Stats,
    /// 每帧：全部 pass 的**包络**之和 —— 口径差就在它与 `inside_all` 之间。
    pub envelope_all: Stats,
    /// 每帧：整条编码器（含宿主目标清屏）。
    pub frame: Stats,
    /// 这一帧执行器发了几条时间戳命令（每一帧都相同才报得出来）。
    pub timestamp_calls: u32,
}

impl Reading {
    /// 读数的文本（`[span-map]` / `[span]` / `[span-sum]` 三种行，`key=value` 用空格分开，
    /// 值里不含空格 —— 外部的脚本按这个形状取数）。
    pub fn report(&self, scene: &str, width: u32, height: u32) -> Vec<String> {
        let mut lines = Vec::new();
        for entry in SPAN_MATCH {
            lines.push(format!(
                "[span-map] pass={} bevy={} why={}",
                entry.label,
                entry.bevy.unwrap_or("none"),
                entry.why.replace(' ', "_")
            ));
        }
        for (name, why) in BEVY_ONLY {
            lines.push(format!(
                "[span-map] pass=none bevy={name} why={}",
                why.replace(' ', "_")
            ));
        }
        for family in SPAN_FAMILIES {
            lines.push(format!(
                "[span-map] pass={} bevy={} why={}",
                family.shape,
                family.bevy.unwrap_or("none"),
                family.why.replace(' ', "_")
            ));
        }
        for pass in &self.passes {
            let mut line = format!(
                "[span] scene={scene} size={width}x{height} block={} warm={} frames={} pass={}:{} kind={} bevy={} envelope_p50_ms={:.4} envelope_min_ms={:.4} envelope_max_ms={:.4}",
                self.block,
                self.warm,
                self.frames,
                pass.index,
                pass.label,
                pass.kind,
                pass.bevy.unwrap_or("none"),
                pass.envelope.p50,
                pass.envelope.min,
                pass.envelope.max
            );
            match &pass.inside {
                Some(inside) => line.push_str(&format!(
                    " inside_p50_ms={:.4} inside_min_ms={:.4} inside_max_ms={:.4}",
                    inside.p50, inside.min, inside.max
                )),
                // ⚠ 明写"没有"，不给一个数：copy 的开 pass 内那一对**根本不存在**，
                //    填 0 或填包络都会让读的人以为它是量出来的。
                None => line.push_str(" inside_p50_ms=none(copy：不开 render pass)"),
            }
            lines.push(line);
        }
        lines.push(format!(
            "[span-sum] scene={scene} size={width}x{height} block={} warm={} frames={} matched_inside_p50_ms={:.4} matched_inside_min_ms={:.4} matched_inside_max_ms={:.4}",
            self.block, self.warm, self.frames, self.matched.p50, self.matched.min, self.matched.max
        ));
        lines.push(format!(
            "[span-sum] scene={scene} block={} inside_all_p50_ms={:.4} envelope_all_p50_ms={:.4} frame_p50_ms={:.4} frame_min_ms={:.4} frame_max_ms={:.4} timestamp_calls_per_frame={}",
            self.block, self.inside_all.p50, self.envelope_all.p50, self.frame.p50, self.frame.min, self.frame.max, self.timestamp_calls
        ));
        lines
    }
}

/// 回读缓冲里**每帧占多少格**：一帧一块，块的大小按 resolve 的对齐要求向上取整
/// （`QUERY_RESOLVE_BUFFER_ALIGNMENT` = 256 字节）。
///
/// ⚠ 它是**算出来的**、不是常数：一帧的格数随文档变（影子那一族场景有 12 条 pass ⇒ 50 格
/// = 400 字节 > 256）。写死 256 字节的下场是"resolve 越界"——实测报的是
/// `Resolving queries 0..48 will end up overrunning the bounds of the destination buffer`。
fn region_bytes(stride: u32) -> u64 {
    let raw = u64::from(stride) * u64::from(wgpu::QUERY_SIZE);
    let align = u64::from(wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT);
    raw.div_ceil(align) * align
}

/// 一帧一帧地收时间戳，**每帧就地 resolve**，最后回读一次。
///
/// ⚠⚠ **为什么每帧就地 resolve**（这一条是实测出来的，不是设计出来的）：
///
/// 更省事的形状是"把所有帧写进同一张查询集（第 n 帧用 `n × 每帧格数` 那一段），
/// 跑完再另起一条提交一次 resolve 收全部"。**那个形状读到的是全 0**
/// （实测，本机 RTX 3060 / 596.36，`--spans 0,1` 与 `--spans 2,3` 都是
/// `非零 0/32 格`）—— 而同一套写入在 `px_pass` 的 GPU 判据里（那条判据的形状是
/// "写完就在**同一条编码器**里 resolve"）读出的是 4096 格 ≈ 4 µs 的实数。
/// wgpu-core 在**每条 pass 的开头**为它要用的那几格发一条 `vkCmdResetQueryPool`
/// （`wgpu-core-29.0.4/src/command/render.rs:2291-2300` 的 `pending_query_resets`），
/// 于是那些格"用完就还"；把 resolve 推到很多条提交之后，读到的是被还回去的格。
/// ⇒ 这台仪器把"读"钉在**写的那一条编码器里**（[`FrameStamps::resolve_now`]），
/// 每帧一块区域，于是查询集只需要**一帧**那么大 —— 帧数不再受格数上限约束。
///
/// ⚠ 记账要说清：这条结论是**实测 + 一处 wgpu-core 源码**，我没有把 reset 的
/// 提交次序完全读通（那需要读通 wgpu 29 新的多命令缓冲架构）；但**仪器按能出数的
/// 形状写**，而"另起一条 resolve 会读到全 0"这件事本身是可复现的读数。
pub struct Recorder {
    /// 查询集只排**一帧**那几格（每帧复用同一段 —— 值在帧末就被搬走了）。
    query_set: wgpu::QuerySet,
    /// 每条 pass 的格 + 帧级那一对（**排一次**，写与读共用）。
    layout: Vec<px_pass::PassSlots>,
    frame: (u32, u32),
    /// resolve 落点（`QUERY_RESOLVE | COPY_SRC`）—— 它不能直接映射，见 `Recorder::new`。
    resolve: wgpu::Buffer,
    /// 能从 CPU 看见的那一块（`COPY_DST | MAP_READ`）。
    buffer: wgpu::Buffer,
    /// 一帧一块区域（**按这一帧的格数**向上取整到 resolve 的对齐），一共 `frames` 块。
    bytes: u64,
    /// 一块区域里有多少格（读的时候按它跨帧）。
    region_ticks: u32,
    frames: u32,
    /// 每一块（一轮）里预热几帧、测量几帧。
    warm: u32,
    measured: u32,
    /// 每条 pass 的 `(label, kind)`（按计划里的次序）。
    labels: Vec<(String, px_pass::PassKind)>,
    period_ns: f32,
    calls: Vec<u32>,
}

impl Recorder {
    /// 建一套槽。帧的排布是**若干轮**：每轮 `warm + measured` 帧；轮与轮之间怎么交错由调用方
    /// 决定（本宿主在 `run_spans` 里按轮交错几份文档 —— §147.4 那条"参照图自己漂了 4.5×"
    /// 要求在同一个进程里轮流画几档，而每份文档只 `open` 一次）。
    ///
    /// ⚠ 预热帧**也写**时间戳（只是不进读数）：这样两段走的是同一条路，
    /// "时钟被拉起来"在两段里是同一件事。
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        labels: Vec<(String, px_pass::PassKind)>,
        blocks: u32,
        warm: u32,
        measured: u32,
    ) -> Result<Recorder, String> {
        if labels.is_empty() {
            return Err("这份文档一条 pass 都没有：没有 span 可量".to_string());
        }
        if measured == 0 {
            return Err("测量帧是 0 帧：那一位必须是正的（预热帧不算数）".to_string());
        }
        if blocks == 0 {
            return Err("轮数是 0：那一位必须是正的".to_string());
        }
        let kinds: Vec<px_pass::PassKind> = labels.iter().map(|(_, kind)| *kind).collect();
        let (layout, frame_begin) = px_pass::frame_slots(&kinds);
        let stride = frame_begin + px_pass::TIMESTAMP_FRAME_SLOTS;
        if stride > wgpu::QUERY_SET_MAX_QUERIES {
            return Err(format!(
                "一帧 {stride} 格，超过一台设备最多能有的 {} 格（wgpu 的 QUERY_SET_MAX_QUERIES）：\
                 这份文档有 {} 条 pass",
                wgpu::QUERY_SET_MAX_QUERIES,
                labels.len()
            ));
        }
        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("px_render 逐条 pass 的时间戳（一帧那几格）"),
            ty: wgpu::QueryType::Timestamp,
            count: stride,
        });
        let region = region_bytes(stride);
        let frames = blocks
            .checked_mul(warm + measured)
            .ok_or_else(|| format!("{blocks} 轮 × {} 帧：帧数溢出了", warm + measured))?;
        let bytes = region * u64::from(frames);
        // ⚠ **两块缓冲，不能合成一块**（实测，wgpu 29 的硬规则）：
        //    `MAP_READ` 只许与**相反的那一个** COPY 组合，而 resolve 要的是 `QUERY_RESOLVE`
        //    ⇒ 一块 `MAP_READ | COPY_DST | QUERY_RESOLVE` 当场被拒
        //    （"`MAP` usage can only be combined with the opposite `COPY`"）。
        //    这与 Bevy 的诊断记录器同一个形状（`diagnostic/internal.rs:420`：
        //    resolve 那块是 `COPY_SRC`，落盘那块才是 `COPY_DST | MAP_READ`）。
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("px_render 时间戳 resolve"),
            size: bytes,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("px_render 时间戳回读"),
            size: bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Ok(Recorder {
            query_set,
            layout,
            frame: (frame_begin, frame_begin + 1),
            resolve,
            buffer,
            bytes,
            region_ticks: (region / u64::from(wgpu::QUERY_SIZE)) as u32,
            frames,
            warm,
            measured,
            labels,
            // 周期是"一格多少纳秒"：本机 RTX 3060 / 596.36 实测 1 ns（§104 第 12 条）。
            period_ns: queue.get_timestamp_period(),
            calls: Vec::with_capacity(frames as usize),
        })
    }

    /// 第 `frame` 帧的槽（格的下标从 0 起 —— 每帧都用同一段）。
    pub fn stamps(&self, frame: u32) -> FrameStamps<'_> {
        FrameStamps {
            query_set: &self.query_set,
            layout: &self.layout,
            frame: self.frame,
            resolve: &self.resolve,
            resolve_offset: u64::from(frame)
                * u64::from(self.region_ticks)
                * u64::from(wgpu::QUERY_SIZE),
        }
    }

    /// 这一帧有几条 pass（读数里要报出来：槽是按它排的）。
    pub fn passes(&self) -> u32 {
        self.layout.len() as u32
    }

    /// 记下这一帧执行器报的"发了几条"（读数的一部分：它证明槽真的被写了）。
    pub fn note_calls(&mut self, calls: u32) {
        self.calls.push(calls);
    }

    /// resolve + 回读 + 算读数：**每一轮一份**（`Reading::block` 是第几轮）。
    ///
    /// `warm` 帧只用来把时钟拉起来，不进读数 —— 而它**每一轮都有**：
    /// 轮与轮之间调用方可能去画了别的文档（交错取样），所以"上一轮结束时时钟的状态"
    /// 不是这一轮的起点。
    pub fn finish(
        self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<Vec<Reading>, String> {
        // 整块搬一次、映射一次：每帧的值在**它自己那条编码器里**就已经 resolve 好了
        // （`FrameStamps::resolve_now`），这里只是一次搬运。
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("px_render 时间戳回读"),
        });
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.buffer, 0, Some(self.bytes));
        queue.submit(Some(encoder.finish()));

        let slice = self.buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(120)),
            })
            .map_err(|err| format!("等时间戳回读超时：{err}"))?;
        receiver
            .recv()
            .map_err(|err| format!("时间戳映射没有回调：{err}"))?
            .map_err(|err| format!("时间戳缓冲区映射失败：{err}"))?;

        let ticks: Vec<u64> = {
            let data = slice.get_mapped_range();
            data.chunks_exact(wgpu::QUERY_SIZE as usize)
                .map(|chunk| u64::from_le_bytes(chunk.try_into().expect("8 字节一格")))
                .collect()
        };
        let _ = self.buffer.unmap();

        if self.calls.len() != self.frames as usize {
            return Err(format!(
                "记下来的是 {} 帧、而这一套槽排的是 {} 帧：帧数与槽对不上，读数不可信",
                self.calls.len(),
                self.frames
            ));
        }
        let calls = self.calls[0];
        if let Some(other) = self.calls.iter().find(|value| **value != calls) {
            return Err(format!(
                "每一帧该发同样多条时间戳，实测 {} 与 {calls} 两种 —— \
                 条数一变，说明这两帧跑的不是同一条路，读数不可比",
                other
            ));
        }

        // 一格的纳秒数 × 前后之差 = 这一段多久（格是 u64，减法天然处理回绕）。
        let to_ms = |begin: u64, end: u64| -> f64 {
            end.wrapping_sub(begin) as f64 * f64::from(self.period_ns) / 1e6
        };

        // ⚠ 下面两段**不自己算下标**：它们调 `FrameStamps::pass_slots` —— 执行器写的时候
        //    走的是同一个函数。自己再算一遍就是"写的人"与"读的人"两处会漂开的真相，
        //    而漂开的那天读数照样打得出来（只是量的是别人的格）。
        //
        // ⚠ 帧与帧之间隔的是 `region_ticks`（每帧一块、按 resolve 的对齐向上取整）——
        //    那是 resolve 的偏移必须对齐到 256 字节的直接后果。
        let layout = self.stamps(0);
        let region = self.region_ticks;
        let at = |frame: u32, slot: u32| -> u64 { ticks[(frame * region + slot) as usize] };
        let per_block = self.warm + self.measured;

        let mut out = Vec::with_capacity((self.frames / per_block) as usize);
        for block in 0..(self.frames / per_block) {
            // 这一轮要读的那几帧（**跳过它自己的预热帧**）。
            let measured: Vec<u32> = (0..self.measured)
                .map(|index| block * per_block + self.warm + index)
                .collect();

            let mut passes = Vec::with_capacity(self.labels.len());
            for (index, (label, kind)) in self.labels.iter().enumerate() {
                let slots = layout
                    .pass_slots(index)
                    .ok_or_else(|| format!("第 {index} 条 pass '{label}' 的格不在排布里"))?;
                let mut envelope = Vec::with_capacity(measured.len());
                let mut inside = Vec::with_capacity(measured.len());
                for frame in &measured {
                    envelope.push(to_ms(
                        at(*frame, slots.envelope.0),
                        at(*frame, slots.envelope.1),
                    ));
                    if let Some((begin, end)) = slots.inside {
                        inside.push(to_ms(at(*frame, begin), at(*frame, end)));
                    }
                }
                let bevy = bevy_span_of(label)?;
                passes.push(PassReading {
                    index,
                    label: label.clone(),
                    kind: kind.name(),
                    bevy,
                    envelope: Stats::of(envelope),
                    inside: if inside.is_empty() {
                        // copy 不开 render pass ⇒ 它**没有**"pass 内"那一对（不是"没量到"）。
                        None
                    } else {
                        Some(Stats::of(inside))
                    },
                });
            }

            // ---- 逐帧求和，再在帧之间取分位（与 Bevy 的 `gpu_ms.p50` 同一套口径）----
            let mut matched = Vec::with_capacity(measured.len());
            let mut inside_all = Vec::with_capacity(measured.len());
            let mut envelope_all = Vec::with_capacity(measured.len());
            let mut frame = Vec::with_capacity(measured.len());
            for at_frame in &measured {
                let mut sum_matched = 0.0;
                let mut sum_inside = 0.0;
                let mut sum_envelope = 0.0;
                for (index, (_, _)) in self.labels.iter().enumerate() {
                    let slots = layout
                        .pass_slots(index)
                        .ok_or_else(|| format!("第 {index} 条 pass 的格不在排布里"))?;
                    sum_envelope += to_ms(
                        at(*at_frame, slots.envelope.0),
                        at(*at_frame, slots.envelope.1),
                    );
                    // ⚠ copy 没有"pass 内"那一对（`PassSlots::inside` 是 `None`）⇒ 它既不进
                    //    `inside_all`、也不进匹配子集。这不是"漏了它"，是它本来就没有这个量。
                    if let Some((begin, end)) = slots.inside {
                        let value = to_ms(at(*at_frame, begin), at(*at_frame, end));
                        sum_inside += value;
                        // 匹配子集 = **Bevy 侧查得到对应段**的那些 pass 之和。
                        if passes[index].bevy.is_some() {
                            sum_matched += value;
                        }
                    }
                }
                let (frame_begin, frame_end) = layout.frame;
                matched.push(sum_matched);
                inside_all.push(sum_inside);
                envelope_all.push(sum_envelope);
                frame.push(to_ms(at(*at_frame, frame_begin), at(*at_frame, frame_end)));
            }

            out.push(Reading {
                block,
                warm: self.warm,
                frames: self.measured,
                passes,
                matched: Stats::of(matched),
                inside_all: Stats::of(inside_all),
                envelope_all: Stats::of(envelope_all),
                frame: Stats::of(frame),
                timestamp_calls: calls,
            });
        }
        Ok(out)
    }
}
