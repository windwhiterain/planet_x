# 彻底剥离 Bevy：一份带实测的调研（2026-09-16，**代码未动**）

> 起因：用户问「**彻底剥离 bevy，换上更轻量级、编译速度 / 启动速度更快的渲染框架**」，
> 并追加一条放宽 —— **不必须保证可以动态加载 pass**（`slots://` 那套内容版本化的 shader 槽不是硬约束）。
>
> 这一篇只做三件事：**把今天的代价量出来**、**把候选逐个否掉或留下**、**给出分期与风险**。
> 所有数字都是本轮在 `feature/pass-table` worktree 上现测的（机器：i7-12700H / RTX 3060 Laptop /
> Windows 11 / rustc 1.97.1 / `[profile.dev] debug = "line-tables-only"`）。
> 复现脚本与原始产物在 `target/research/`（不入 git）。

> ⚠ **S8-c 标注（2026-09-19）：这一篇是"离开 Bevy"这条决定的证据链，整篇保留、一个字不删。**
> 它是 §154 那次剥离**唯一**的代价依据（冷编 366.4 → 54.7 s、增量 13–16 → 1.7 s、
> 产物 156.6 → 8.45 MB、依赖树 358 → 53），而"代价量出来了"正是那件事能被用户裁定的前提。
> ⚠ 两处读法：
> 1. `target/research/` 里的复现脚本与原始产物**在 `target/` 下**（不入 git）⇒ 多半已不在；
>    **读数仍然有效**，它们是当时现测的（本篇头两行就写着机器与 profile）。
> 2. 表里 `px_render` 那一列描述的是**被剥掉的那一支**：它已删（`f121ee3`，`15-render-wgpu.md` §154）。
>    ⇒ 这一篇回答的是"**为什么要走**"，不是"今天还有什么"。

---

## §91 先划线：bevy 的边界其实只有两个 crate

`grep -rn bevy` 的结论比预期干净：

| crate | 依赖 bevy？ | 说明 |
|---|---|---|
| `px_protocol` | **否**（注释里出现 4 次） | 只有类型 + serde |
| `px_shader` | **否**（注释里出现 9 次） | 无依赖 |
| `px_ops` / `px_graphs` / `px_verify` / `px_mc` / `game` / `src`(sim) | **否** | — |
| **`px_pass`** | **否**（`Cargo.toml` 里只有 `wgpu`） | §85 那条编译期保证今天仍然成立 |
| **`px_render`** | **是**（`bevy = "0.19"` + `file_watcher`） | 唯一的宿主 |
| `px_probe` | 只为了 `px_render::shaders::assemble` 而依赖 `px_render` | 自己已经是裸 wgpu（`probe.rs` 460 行） |

⇒ **剥离面 = `px_render` 一个 crate**（外加把 `assemble` 从 `px_render` 挪到一个不含 bevy 的落点）。
PCG 侧、协议侧、判据侧（`px_probe`）、sim 侧**一行都不用改**。

这一条是有前提的：§65「通用渲染」已经把渲染器变成了**只认产物**的形状（`objects[]` / `material.shader`
/ 反射出来的参数块 / 固定超集绑定），`material.rs` 与 `reflect.rs` 里几乎没有 Bevy 特有语义。
**通用渲染是这次剥离能成立的结构基础**；如果还在 `KINDS = ["planet","clouds","atmosphere"]` 那个形状上，
这笔账会难看一个量级。

---

## §92 编译代价：冷编、增量、体积（实测）

### §92.1 依赖树

| | 唯一 crate 数 | `bevy_*` crate 数 |
|---|---|---|
| `px_render`（今天） | **358** | 63 |
| `px_pass`（只有 wgpu） | **53** | 0 |
| 裸 wgpu 基线（`target/research/render-min`，见 §93） | **170 个编译单元** | 0 |

bevy 拖来的、本项目**一个字节都用不到**的：文字 / 字体 / ICU 一族 **47 个 crate**（`read-fonts` ×2、
`skrifa` ×2、`swash`、`parley`、`fontique`、`harfrust`、`icu_*` ×12…，全部只为 viewer 里那行 FPS 读数）、
glTF 3 个、音频 7 个、手柄 4 个、`accesskit` 4 个、UI 布局 6 个。

### §92.2 冷编（全新 target dir，`cargo build -p px_render --timings`）

| 读数 | 值 |
|---|---|
| 墙钟 | **366.4 s**（cargo 自报 6m05s） |
| 单元时间合计 | 1484.1 s / 431 个单元（并行度 ≈ 4×） |
| `bevy_*` 合计 | **524.2 s = 35.3%** |
| **关键路径** | **`bevy_pbr` 一个单元：起于 208.9 s、耗时 134.2 s、止于 343.1 s** —— 紧接着 `px_render` 才开始（343.1 s），止于 365.7 s |
| 其它大件 | `windows 0.62.2` 95.8 s｜`bevy_render` 61.9 s｜`ash` 36.3 s｜`read-fonts` 27.7+24.6 s｜`bevy_core_pipeline` 26.6 s｜`glam` 24.1 s｜`naga` 23.5 s｜`px_render` 自己 22.6 s |

⇒ **冷编的墙钟基本就是 `bevy_pbr` 的编译时间**（它独占关键路径 134 s / 366 s）。
这和上游 [#23642](https://github.com/bevyengine/bevy/issues/23642)（OPEN，"渲染 crate 合起来 91.2 s
= 总编译时间 76%，其中约一半是 `bevy_pbr`"）量级一致。

### §92.3 增量（这才是日常）

| 动作 | 今天（bevy） | 裸 wgpu 基线 |
|---|---|---|
| 无改动 `cargo build -p px_render` | 0.6–3.5 s | 0.24 s |
| **改 `px_render/src/main.rs` 一行** | **13.0 / 15.7 / 16.4 / 28.6 s**（稳态 ~13–16 s） | **1.67 / 1.70 / 1.77 s** |
| 同上，但代码量补到 ~3100 行平凡函数 | — | **2.80 / 3.10 / 3.44 s** |
| 同一行 `cargo check -p px_render`（只前端） | **1.43 s** | — |
| 产物 | `px_render.exe` **156.6 MB** | `render_min.exe` **8.45 MB** |

**这一组读数是本轮最值钱的东西**，它推翻了两个想当然：

1. **链接不是大头。** 同一行的 `--timings` 里只有一个单元：`px_render 14.80 s`，
   而墙钟 16.38 s ⇒ **链接 + cargo 开销只有 ~1.6 s**。
2. **"4500 行代码"也不是大头。** 前端（`cargo check`）只值 **1.43 s**；
   补 3000 行平凡函数后裸基线的增量只从 1.7 s 涨到 3.1 s。
   ⇒ 那 13 s 几乎全是 **LLVM 为 bevy 泛型产码**（`Query<...>` / `Res<...>` / `Commands` /
   `MaterialPlugin` 的 `AsBindGroup` / `MeshVertexBufferLayoutRef` 在 `px_render` 这边被单态化）。
   `bevy_pbr` 自己在冷编里也是同样的指纹：上游 `--timings` 显示它 **95% 的时间花在 rustc 前端 + MIR**，
   而不是 codegen。

⇒ **推论（重要，与直觉相反）：只把 bevy 的功能特性砍掉（`default-features = false`）几乎不改善日常循环。**
砍掉 `bevy_text` / `bevy_gltf` / `bevy_audio` 不会让 `px_render` 的 `main.rs` 少单态化一个泛型，
改一行仍然 ~13–16 s。**要动的是那 13 s，只有把 bevy 从 `px_render` 的依赖里拿掉才动得了。**

---

## §93 裸 wgpu 基线的实测（`target/research/render-min`）

一个 70 行的 bin，依赖 = `wgpu 29`(vulkan+wgsl) + `winit 0.30` + `naga 29` + `serde`/`serde_json` +
`image` + `notify` + `glam` + `encase` + `bytemuck` + `pollster` + `blake3`：

| 读数 | 值 |
|---|---|
| 冷编墙钟 | **54.74 s** |
| 单元数 / 单元时间 | 170 / 507.4 s（并行度 ≈ 9×） |
| 末单元结束 | 52.9 s |
| 改一行的增量 | 1.67–1.77 s；+3000 行平凡码后 2.8–3.4 s |
| 产物 | 8.45 MB |

最贵的 8 个：`ash` 26.2 s｜`syn` 13.2+9.7 s｜`glam` 11.5 s｜`moxcms` 10.4 s｜`naga` 10.0 s｜
`windows-sys` 9.2+6.3 s｜`wgpu-core` 7.7 s。

**注意这不是"省 366−55 = 311 s"的天真账**：wgpu 仍然是重依赖（`naga` 10 s + `wgpu-core` 7.7 s +
`wgpu-hal` + `wgpu-types` + `ash` 26 s），基线里还有一半时间花在与 3D 无关的 `image`/`moxcms`/`png`
（可以砍到 `png` 一个）。**但是 6.7× 的冷编差和 9× 的增量差是实测的。**

---

## §94 启动代价（`--serve` headless，Vulkan，热文件缓存）

外部仪器（一行代码都不改产品）：`target/research/startup-probe.ps1`，按日志行到达时间打戳。

| 里程碑 | 第 2 次 | 第 3 次 | 冷文件缓存第 1 次 |
|---|---|---|---|
| 进程起 → `后端：Vulkan…`（适配器 + 设备就绪） | **1 419 ms** | **1 312 ms** | 10 235 ms |
| → `GPU 时间戳能力` | 1 877 ms | 1 656 ms | 10 631 ms |
| → `渲染管线全部就绪：共 42 条` | **3 102 ms** | **2 902 ms** | 11 911 ms |
| → 第一张图（含搭场景 + 画 + readback + PNG） | +1 465 ms | +1 471 ms | +1 777 ms |

对照（同一台机器、同一个 wgpu 29、同一把 Vulkan 后端）：

| 进程 | 体积 | 进程起 → 设备就绪 |
|---|---|---|
| `px_probe --bin device`（DX12） | 55.4 MB | **418 / 425 ms** |
| `px_probe --bin device`（Vulkan） | 55.4 MB | **455 / 483 ms**（首跑 1 649 ms） |
| `render_min --device`（裸 wgpu，Vulkan） | 8.45 MB | **329 / 458 / 570 ms**（首跑 5 683 ms） |
| `px_render --serve`（bevy） | 156.6 MB | **1 312 / 1 419 ms**（首跑 10 235 ms） |

### §94.1 三条结论

1. **"Vulkan 枚举 2.8 s"是过期读数（§44.4 那条）。** 今天 Vulkan 与 DX12 的设备就绪都是 ~450 ms。
   `target/research/startup/serve-*.err` 里仍能看到成因（Vulkan loader 去找不存在的 layer JSON：
   `...\WeGame\...\CrossVulkanLayer64.json`、`...\Epic Games\...\EOSOverlayVkLayer-Win32/64.json`，
   4 条 `Failed to open JSON file`），但代价已经降到几百毫秒量级。**不必为此改框架。**
2. **bevy 在"进程 → 设备就绪"这一段值 ~0.85–0.9 s**（1 312 − 450），并且它随产物体积走：
   `--serve` 冷文件缓存首跑要 10.2 s 才到同一个里程碑，而 8.45 MB 的裸基线是 0.33–0.57 s。
3. **剩下 1.6 s（`设备就绪 → 42 条管线全就绪`）与框架无关**，是 42 条管线的 naga + 驱动编译。
   §51.20 已经量过：47 条 ≈ 3.4 ms/条、净编译 ≈ 162 ms。这里面有可省的部分
   （P6 早说过：42 条里混着 `StandardMaterial` / `Skybox` 的变体）。
   **换框架能把这一段从"bevy 的 42 条"降到"我们自己的几条 + pass 表"，但不是数量级。**

⇒ **启动这一项的真实收益是「~3.0 s → ~1.2–1.6 s」这个量级，而不是「3 s → 0.3 s」。**
冲这一项去做剥离，性价比不高；**冲增量编译（16 s → 3 s）和冷编（366 s → 55 s，17 个 worktree 各付一次）
才是划算的**。

---

## §95 候选逐个过（含子 agent 的 crates.io / GitHub 一手核查）

筛选的第一道闸是 **wgpu 版本对齐**：今天锁 `wgpu 29.0.4` + `naga 29`（`bevy_render 0.19.1` 自己就钉
`wgpu ^29.0.3` + `naga ^29.0.3`），而 **wgpu 每三个月一个 breaking 版本**（上游 README 原话）。

| 候选 | 版本 / 状态 | wgpu | 判决 |
|---|---|---|---|
| **`winit` + `wgpu` +（可选 `egui-wgpu`）** | winit 0.30.13（`bevy_winit` 也钉 ^0.30）；wgpu 29.0.4 | **29** ✅ | **留下**（唯一三头都占的） |
| `bevy` 最小特性集 | 0.19.1 | 29 ✅ | **不影响日常循环**（§92.3），只省冷编 |
| `bevy_*` 子 crate 替代 umbrella | — | 29 ✅ | **无意义**：`bevy` 本体 19 行、唯一依赖 `bevy_internal` |
| `kiss3d` 0.46 | 2026-08-15，活跃 | **30** | 否：差一个大版；**API 里没有任何 shadow map 类型**；自述"不是为功能完整或快而设计" |
| `three-d` 0.19 | 活跃 | 无（`glow` OpenGL + **winit ^0.28**） | 否：后端不对；`PointLight` 没有影子 |
| `macroquad` / `miniquad` | 活跃 | 无（OpenGL/GLES） | 否：后端不对、2D 优先、无 WGSL |
| `rend3` | **GitHub archived**，"MAINTENCE MODE"，最后 release 0.3.0 / **2022-02-12**，钉 wgpu 0.12 | 0.12 | **死** |
| `luminance` | 最后 release **2022-04-19** | 无 | **死** |
| `sokol` / `rokol` / `sokol-rust` | 2019-04-29 / 2022-01-06 / **crates.io 上不存在** | 无 | **死** |
| `pixels` 0.17.2 | 活跃 | 29 ✅ | 只值"2D 帧缓冲 / readback 便利"，3D 用不上 |
| `vello` 0.10 / `vello_hybrid` 0.2 | 活跃（Linebender） | 29 ✅ | **只有 2D 矢量**；最多当 HUD 叠层 |
| `femtovg` 0.27 / `nanovg` | femtovg 活跃；`nanovg` 最后 **2018-08-27** | 默认 glow；wgpu 后端是 **30** | 否（2D + 版本偏） |
| `myth_render` 0.3.0 | 2026-07-31，**单人、~154 下载** | **29.0.4** ✅ | 架构最像（render graph），但太年轻、关键能力未证 ⇒ **观察，不采用** |
| `nightshade` 0.57 | 9 个月 148 个 release；**metadata 里的仓库 URL 404** | 29 | 否：源码不可审计 |
| `renderling` | 仓库活着，crate 停在 0.4.9 / 2024-09-20 | **26**（且要 rust-gpu = nightly） | 否 |
| `blade` / `screen-13` / `kajiya` | blade 活跃 | 无（Vulkan/ash） | 否：不是 wgpu |

**榜上没有一个"更轻的现成 3D 框架"。** 落点只有两个：**留下的 bevy**，或**自己写宿主**。

---

## §96 自己写宿主：能得到什么、要写什么

### §96.1 今天 bevy 替我们做的事（逐条对照）

| Bevy 提供的 | 裸 wgpu 侧要写的 | 量 |
|---|---|---|
| `App` / ECS / 调度 / `Commands` / `Query` | 普通结构体 + 函数（`main.rs` 的 3553 行本来就该这么组织） | 重写，但机械 |
| `RenderPlugin` / `RenderDevice` / `RenderQueue` | `InstanceDescriptor::new_without_display_handle()` + `request_adapter` + `request_device`（`px_probe/src/common.rs` **已有 60 行现成**） | **已有** |
| `Camera3d` / `RenderTarget` / `Viewport` / `DepthPrepass` / `Msaa::Off` | 自己的深度纹理 + 逐相机 render pass（`--sheet` 12 格视口 = 12 个 viewport） | ~250 行 |
| `MaterialPlugin` / `AsBindGroup` / `specialize` / `Material` | 管线缓存（键 = shader 内容版本 × cull × alpha）+ 固定超集布局 + 逐材质 bind group。**`material.rs` 的 `AsBindGroup` impl 与 `reflect.rs`（641 行，纯 naga）逐条可搬** | ~350 行（多为搬迁） |
| `Mesh` 资产 / `Mesh3d` / `MeshMaterial3d` | 顶点/索引缓冲 + 一个 draw 结构（`mesh.rs` 259 行可搬） | ~200 行 |
| `PointLight` + **cube shadow map** + clustered forward + `fetch_point_shadow` | **真正的新活**：6 面深度图（`Depth32Float` + `depth_or_array_layers: 6`）、`TextureViewDimension::Cube`、`sampler_comparison` + `textureSampleCompare`。Vulkan 上可用 **multiview**（`multiview_mask = 0b111111` + WGSL `@builtin(view_index) -> u32`）一次画 6 面 —— 官方 `multiview` 例子在 v29.0.4 里 | **~450 行** |
| `Skybox` / tonemapping / upscaling | 天空盒（官方 `skybox.rs` 例子有）+ 一个 fullscreen tonemap pass（**wgpu 官方没有例子**，但就是一段 WGSL；`px_pass` 天生就是干这个的） | ~250 行 |
| `Screenshot` + `save_to_disk` + `ScreenshotCaptured` | 渲染到自己的 `Rgba8UnormSrgb`（**不要读 swapchain**：surface 只保证 BGRA）+ `copy_texture_to_buffer`（256 字节行对齐）+ `map_async` + `poll(PollType::wait_indefinitely())` + `png`。官方 `render_to_texture.rs` 就是这个 | ~120 行 |
| `RenderDiagnosticsPlugin` 的 `render/**/elapsed_gpu` | `TIMESTAMP_QUERY` + `QuerySet` + `resolve_query_set` + 自己的账 | ~200 行 |
| `AssetServer` 的热重载（`notify`） | `notify 8.2` + 重建 `ShaderModule`/管线（**官方没有例子**）。⚠ 用户已放宽"动态加载 pass"，所以 `slots://` 那套内容版本化槽**可以整段删掉**，改成启动时组装一次 | ~100 行 |
| `WinitPlugin` / `Window` / `AccumulatedMouseMotion` / `ButtonInput` | `winit 0.30` 事件循环 + orbit 相机（官方 `standalone/02_hello_window` 有模板） | ~250 行 |
| `bevy::text` 的 FPS 读数 | 删掉（打 stdout）或 egui | **负成本** |
| `bevy_shader` / naga_oil 的 `#import` | **已经有一份离线组装器**（`shaders.rs::assemble` + `px_shader`）⇒ 运行期不再需要 naga_oil | **已有** |
| `art_cache` / `scene`（文档 → 实体）/ `passes`（宿主侧） | 大部分与框架无关，可搬 | ~400 行（搬迁） |

**估算：新写 + 改写 ≈ 3000–3500 行，其中真正全新的 ≈ 1200–1500 行**（影子、tonemap、截图回读、
时间戳、winit 循环）。§4 当初记的"自研 wgpu + naga 要 3–6 人月才到 parity"今天**不成立**了 ——
通用渲染（§65）与 `px_pass`（§85）已经把中间的活干掉了大半，而且本仓已有 `px_probe` 这份裸 wgpu 先例。

### §96.2 内容 shader 要改的地方（这是最容易被低估的一项）

`art/shaders/*.wgsl` 里有 5 类 `bevy_pbr::*` 外部符号，**它们同时是 group 0 的绑定契约**：

```
bevy_pbr::forward_io::VertexOutput                 ← 顶点输出结构（位置/世界位置/法线/uv）
bevy_pbr::mesh_view_bindings::{view, lights, depth_prepass_texture, globals}
bevy_pbr::mesh_view_types::POINT_LIGHT_FLAGS_SHADOWS_ENABLED_BIT
bevy_pbr::shadows::fetch_point_shadow
bevy_pbr::view_transformations::depth_ndc_to_view_z
```

影响面：`clouds.wgsl`(722 行) / `surface.wgsl` / `atmosphere.wgsl` / `ring.wgsl` / `light.wgsl` +
`px_render/src/shaders.rs` 里那 10 个手写桩 + 离线门 `tests/shaders.rs`。
**好消息**：光照数学（Burley、`F_AB`/`EnvBRDFApprox`、`getDistanceAttenuation`）**已经逐字抄在本仓**
（`surface.wgsl` / `light.wgsl` 的注释自己写着），从来没有真的依赖 bevy 的实现；
真正只有 bevy 有的，是 **group 0 的布局 + 点光 cube shadow map 的采样**。换个绑定表就搬完了。

### §96.3 判据的风险（这是最大的那一项，不是代码量）

这个仓的判据体系是**逐字节**的：

- §12：同一轮跑两次逐字节相同；§65 迁移的判据是 `0 / 614400` 不同像素；§86 的 pass 表四条判据
  里两条是"像素差 0"。
- §51.18 那条铁律：**判据一律读 `pair.gpu_delta_ms`** —— 而那个 `gpu_ms` 今天来自
  `RenderDiagnosticsPlugin` 写进 `DiagnosticsStore` 的 `render/**/elapsed_gpu`。
- `--sheet` 的 12 格视口、px_render 报告里的 `placeholder_px` / `bright_px` / grid，全部从
  `ScreenshotCaptured` 的 `Image` 上算。

⇒ **剥离之后必须逐字节复现到今天为止的所有对照图，否则一整轮云优化的基线全部作废。**
这不是"尽量"，而是这件事能不能做的**验收条件**：
不逐字节相同的剥离 = 把 §51 那一整套性能账（软档 −10.4%、代理 −13.4%…）清零。

同样地，`gpu_delta_ms` 换实现之后**读数必须与 Bevy 的旧读数可比**，否则 §51 的历史表全变成不可比的数。
这一条要在动第一行代码之前先定口径。

---

## §97 分期与代价

### 阶段 0（零风险、当天见效，建议先做）
1. **`.cargo/config.toml` 打开 `rust-lld`**（工具链里已经有 `rust-lld.exe`，§4.5 记过它只是不在 PATH）。
   上游实测：增量 14.5 → 10.5 s（−28%），冷编**略慢**。
2. **nightly + cranelift codegen backend**（`[profile.dev] codegen-backend = "cranelift"`，
   依赖侧留 LLVM）。上游实测增量 **14.5 → ~5 s**，冷编 2m16 → 1m45。
   这是**唯一一条直接打中"改一行 16 s"这个痛点的现成手段**，且与是否剥离无关。
3. **清掉 Vulkan loader 的坏 layer 注册**（WeGame / Epic overlay 那 4 条 JSON）——启动省几百 ms。
4. **sccache / 共享 `CARGO_TARGET_DIR`**：本仓有 17 个 worktree，**每个一份自己的 `target/`**
   （`pass-table` 这一份 5.97 GB）。跨 worktree 复用同一个冷编是纯浪费。
   ⚠ 这一条是**推断**，本仓没有实测过 sccache 与 MSVC + bevy 的配合，要用先量。

### 阶段 1 ｜新宿主落地（**不删 bevy**，两条路并排跑）
- 新 crate（建议 `px_gpu`：实例/设备/交换链/深度/timestamp/截图回读 + `px_host`：材质/网格/灯/相机）。
- `px_render` 保留为 bevy 宿主，**两个宿主消费同一份 `.pxart`**。
- **验收判据 = 逐字节**：`orbit-bare` / `orbit-rings` / `orbit-soft` / `orbit-proxy-fine` 四档
  与今天 bevy 宿主的图逐字节相同；`--sheet` 12 格逐字节相同；pass 表四份文档（§86）四条判据全过。
- 这一阶段结束时，两条路都活着，`cargo build -p px_render` 仍然是 bevy 的 16 s —— **不赚，但不赔**。

### 阶段 2 ｜切换与拆除
- 判据全绿后，把 bevy 从 `px_render/Cargo.toml` 摘掉，删 `slots.rs`（用户已放宽）、
  `DocMaterialPlugin`、`WgpuSettings`/`Backend` 那条、`bevy_*` 全部引用。
- `px_probe` 改成依赖 `px_shader` 而不是 `px_render`（`assemble` 搬过去）。
- 记账：冷编 366 → ~55–90 s，改一行 16 → ~3–5 s，产物 156.6 → ~10–15 MB，
  `--serve` 到设备 1.31 → ~0.45 s，到管线全就绪 2.9 → ~1.2–1.6 s。

### 代价（要认的三条）
1. **wgpu 三个月一个 breaking**（上游 README 原话）。今天这个版本漂移是由 `bevy_render`
   替我们吸收的；剥离之后 **`wgpu 29 → 30` 的迁移由我们自己付**。29.0.0 已经改掉了
   `SurfaceError` 消失、`InstanceDescriptor::default` 消失、`bind_group_layouts: &[Option<..>]`、
   `DepthStencilState` 变 `Option<..>`、`MapMode::Write` 走 `WriteOnly<[u8]>` 等等。
2. **点光 cube shadow map 是本轮唯一没有现成参考的活**（wgpu 官方例子里的 `shadow.rs` 是
   **2D array 方向光**，不是 cube；`gl_Layer` 那套在 WGSL 里不存在）。要么 6 个 pass，要么 multiview。
3. **`px_render` 那 3553 行不是能一次搬完的**：ECS 系统边界（`Res`/`Query`/`Local`/观察者）
   改成普通函数时，`watch_pipelines` 那类"读渲染世界状态"的系统要重新设计
   （Bevy 的 `PipelineCache` 给的 `Queued/Creating/Ok/Err` 是 §62 那条 fail-fast 闸门的全部依据）。

---

## §98 结论（一句话）

- **"换框架"能买到的是：冷编 366 → ~55 s、改一行 16 → ~3 s、产物 156 → ~10 MB、
  启动到设备 1.31 → ~0.45 s。**
- **买不到的是：一个更轻的现成 3D 框架（榜上没有）**，所以落点必然是**自己写宿主**，
  新写 ≈ 1200–1500 行（影子 / tonemap / 回读 / 时间戳 / winit）。
- **只砍 bevy 的特性集不解决日常循环**（§92.3：前端 1.43 s、链接 1.6 s，剩下 13 s 是 bevy 泛型的 LLVM 产码）。
- **最大的风险不是代码量，是判据**：不逐字节复现，§51 那一整套性能账全部作废（§96.3）。
- 因此建议的次序是 **阶段 0（cranelift/lld/loader，当天见效、与剥离无关）→ 阶段 1（双宿主 + 逐字节判据）
  → 阶段 2（拆除）**，而不是直接开一个"删掉 bevy"的分支。

---

## §99 没做 / 没验（明账）

- **没验**：`--view`（窗口那条路）的启动耗时；本轮只量了 `--serve` headless。
- **没验**：`default-features = false` 的最小特性集能不能让 `px_render` 编过 —— §92.3 的推论
  （"不影响日常循环"）是从"前端 1.43 s / 链接 1.6 s / 其余为泛型产码"这三个读数推出来的，
  **没有真的裁一遍再量**。
- **没验**：sccache 在本机 MSVC + bevy 上的命中率与开销（§97 阶段 0 第 4 条是推断）。
- **没验**：nightly + cranelift 在本仓的实际增益（引的是上游 Bevy 的实测，不是本机的）。
- **没做**：`target/research/render-min` 只是个**量编译与设备启动的基线**，它不画东西。
- **没动**：`px_render` 一行代码没改；本篇是调研。
