# px_render_wgpu：把宿主从 Bevy 换成裸 wgpu（**开工序**）

> **这一篇是工单，不是调研。** 调研与全部实测读数在 `art/14-bevy-exit.md` §91–§99。
> 给新 session 的读法：**先读 §100（边界）、§101（判据）、§104（坑）**，再照 §105 一档一档做。
> **每一档都有自己的判据；没拿到判据不许进下一档。**

---

## §100 目标、边界、为什么

**先要知道自己在哪儿开工**（本仓的 `AGENTS.md`）：**另开一个 worktree、另起一个 feature 分支**，
不要在这个 worktree 里做。

```powershell
# 从 feature/pass-table 拉 —— 这篇工单假设你已经有了它的两样东西：
#   ① v2 的材质契约已并进来（4fc772d）  ② pass shader 统一到材质契约（e0e1355，§86.4）
git -C C:\resource\planet_x worktree add .worktrees/render-wgpu -b feature/render-wgpu feature/pass-table
```

⚠ 如果 `feature/pass-table` 那时**已经并进 `v2`**（`git log --oneline v2 | head`），就从 `v2` 拉。
两样东西都在哪条线上，用 `git log --oneline -1 --grep 'pass shader 统一到材质契约'` 查一下再决定。
⚠ 本仓的 `main` 分支**不许看**（`AGENTS.md`）。

**目标**：新 crate **`px_render_wgpu`** —— 一个**不依赖 bevy** 的宿主，消费**同一份 `.pxart` 渲染文档**，
先与今天的 `px_render`（Bevy 0.19）**逐字节同图**，再取代它。

**为什么值**（全部实测，§92/§94）：

| | 今天（bevy） | 裸 wgpu 基线 |
|---|---|---|
| 冷编（全新 target） | **366.4 s** | **54.7 s** |
| 改一行自己的代码 | **13.0 / 15.7 / 16.4 / 28.6 s** | **1.7 s**（同文本量 3.1 s） |
| 产物 | 156.6 MB | 8.45 MB |
| 进程 → 设备就绪 | 1312 / 1419 ms | 329–570 ms |
| 依赖树 | 358 个 crate | 53（wgpu 树） |

⚠ 顺带一个反直觉的读数，它决定你怎么写代码：那 13 s 里 **`cargo check`（前端）只值 1.43 s、
链接只值 1.6 s**，其余全是 **LLVM 为 bevy 泛型产码**。用户口径：
「**能动态的就动态，减少类型检查与单态化的时间**」——
枚举 + `match` 优先于泛型 trait、`Box<dyn Trait>` / 函数指针优先于泛型参数、数据表 + 循环优先于每类一份代码。
**别再造一层 ECS 式的泛型查询。**

**不许做**：

- 不许删 `px_render`（Bevy 宿主）—— 它是**判据的锚**，判据全绿之前一步都不能少。
- 不许动 `px_pass` / `px_protocol` / `px_shader` / `px_ops` / `px_graphs` / `px_verify` / `game` / sim 的**语义**。
  （`px_shader::assemble` 的桩表要加一个「宿主提供」的入口，见 §103；那是唯一允许的改动。）
- 不许先做阶段 0（cranelift / lld / 清理 Vulkan loader）——用户已裁决**直接阶段 1**。
- 不许改 `SCENE_SCHEMA` / `protocol_hash`（改了在跑的旧服务当场被握手拒，是设计但不是这一步该付的代价）。

---

## §101 判据（做完才知道对不对）

**主判据 = 逐字节。** 这个仓的判据体系是逐字节的（§65 迁移的判据是 `0 / 614400`），
**不逐字节相同的剥离 = 把 §51 那一整套性能账（软档 −10.4%、代理 −13.4%…）清零**。

| # | 判据 | 怎么取 |
|---|---|---|
| **J1** | 四档场景与 bevy 宿主**逐字节相同** | `orbit-bare` / `orbit-rings` / `orbit-soft` / `orbit-proxy-fine-bound`，同尺寸同 `--cam`，PNG sha256 相等 |
| **J2** | `--sheet` 12 格逐字节相同 | 产物自带的评审相机表 |
| **J3** | pass 表四条判据全过 | `art/13-passtable.md` §86.1 的 A/P/R/D（六张图的哈希见 §86.4） |
| **J4** | 报告口径可比 | `pair.gpu_delta_ms` 与 bevy 宿主**同量级**（不是"看着差不多"，是四档的 delta 排序不变） |
| **J5** | 编译与启动的读数达标 | 冷编 ≤ 90 s、改一行 ≤ 5 s、产物 ≤ 20 MB、进程→设备 ≤ 0.6 s、进程→管线全就绪 ≤ 1.6 s |
| **J6** | 既有门全绿 | `tools/px.ps1 -Target test`；`cargo test -p px_render` 照旧（Bevy 宿主不能被你改坏） |

**什么不算过**（写下来是因为它们最像过）：

- "图看着一样" —— 不够，要哈希。
- "只有云那几档差 20–30 个像素" —— **§65 记过同一件事**（绑定下标一变就差这么多，而且**没归因**）。
  这是**未通过**，不是"可接受的残差"。见 §104 第 1 条。
- "PASS 掉了 / 跳过了一次" —— 任何跳过都是判据的敌人；探针拿不到设备要 `exit(2)`，不许静默变绿。

---

## §102 已经给你搭好的东西（**不要重写**）

| 件 | 入口 | 说明 |
|---|---|---|
| **渲染文档** | `px_protocol::scene::{SceneSpec, Object, Material, Light, Environment, Camera, PassSpec, PassResource, VIEW_BUILTIN, read_scene}` | v2 通用文档。`objects[]` / `lights[]` / `environment` / `cameras[]` / `expects[]`。**渲染器不认识行星** |
| **材质契约** | `px_protocol::material::{MATERIAL_BIND_GROUP=3, PARAMS_BINDING=0, PARAMS_ALIGN=16, MAX_PARAMS_BYTES=4096, TEXTURE_SLOTS(12), MaterialLayout, ParamKind, pack}` | **唯一一份绑定表**。`MaterialLayout::pack(&BTreeMap<String, Value>)` 就是参数打包，三档当场报错 |
| **naga 反射** | `px_shader::reflect::reflect_assembled(&assembled, name) -> MaterialLayout` | 叶子 crate，**无依赖**。烘图侧与装载侧共用同一份 |
| **组装器** | `px_shader::assemble::{render_source, expand, bevy_stub}` | 展开 `#import` + 把 `#{MATERIAL_BIND_GROUP}` 替成 3。**桩表要改成宿主可提供，见 §103** |
| **pass 执行器** | `px_pass::{Plan, Executor, Layout, Slot, Dimension, Frame, External, Role}` | **已经是裸 wgpu**。`Layout` 由宿主从契约读出来填（`px_render::passes::executor_layout()` 就是那份代码，照搬） |
| **CAS 装载** | `px_render::art_cache::ArtCache`（`field`/`mesh`/`sphere`/`texture`/`shader` 五张表）、`px_protocol::scene::{cas_path, Member::resolve}` | 键 = 路径 + 清单载荷指纹；`art_cache::replay` 打审计 |
| **网格产物** | `px_render::mesh`（`MeshData` → 顶点/索引；含缠绕翻转与法线焊） | 逻辑可搬，落点从 Bevy `Mesh` 换成你的顶点/索引缓冲 |
| **服务协议** | `px_protocol::{client, stream, wire, render::{Request, Response, Report, ShotReport, PerfReport, GpuMs, Waits}}` + `ProtocolId`（schema + `protocol_hash` + `git rev`）握手 + 租约文件 `target/render-server.json` | **逐字照搬**，否则 `tools/` 那套仪器全废 |
| **离线门** | `px_render/tests/shaders.rs`（每份 `.wgsl` 都要解析 + 校验）、`px_probe`（梯度唯一判据） | 照旧要绿 |
| **裸 wgpu 先例** | `px_probe/src/{common.rs, probe.rs}` | 实例 / 适配器 / 设备 / 纹理 / bind group / staging + `map_async` —— **本仓已经写过一遍**，别从零学 |

`px_render/src/` 各模块的 bevy 耦合度（决定搬迁难度）：`digest` 0、`reflect` 1、`lib` 1、`scene` 3、
`shaders` 4、`art_cache` 5、`mesh` 6、`passes` 7、`slots` 12、`material` 22、**`main` 58**。

---

## §103 要新写的东西

| 件 | 量 | 备注 |
|---|---|---|
| 实例 / 适配器 / 设备 / 交换链 | ~120 行 | `px_probe/src/common.rs` 有 60 行现成；**Vulkan 锁死**（§104 第 9 条） |
| 深度纹理 + prepass + MSAA off | ~150 行 | `DepthStencilState` 的字段在 29 里是 `Option<..>` |
| **group 0 的契约**（我们自己的 `view` / `lights` / `clustered_lights` / `globals` / `depth_prepass_texture`） | ~200 行 | **照抄 Bevy 的绑定号**，理由见 §104 第 1 条 |
| 网格上传 + draw | ~200 行 | `mesh.rs` 搬迁 |
| 通用材质：管线缓存（键 = shader 内容版本 × cull × alpha）+ 固定超集 bind group | ~350 行 | `material.rs` + `reflect.rs` 搬迁，**去掉 `MaterialPlugin` 那一层** |
| 灯 + **点光 cube shadow map**（6 面或 multiview） | **~450 行** | **全局唯一没有现成参考的活**（§104 第 2 条） |
| 主 pass + 天空盒 + 色调映射 | ~250 行 | tonemap 官方没有例子，但就是一段 WGSL；`px_pass` 天生干这个 |
| **截图回读 → PNG** | ~120 行 | **必须用 `image` 的同一行代码**，否则哈希对不上（§104 第 3 条） |
| GPU 时间戳 → 报告 | ~200 行 | 段要按 Bevy 那几段切，否则读数不可比（§104 第 4 条） |
| `--serve` + 租约 + 握手 + 排队 + 报告 | ~500 行 | `px_render/src/main.rs` 的逻辑可搬，去掉 bevy 那半 |
| winit 窗口 + orbit 相机 + 输入 | ~250 行 | 官方 `examples/standalone/02_hello_window` 有模板 |
| shader 热重载（`notify 8.2`） | ~100 行 | `slots.rs` 那 287 行**大部分可以删**：裸 wgpu 的 `create_shader_module` 是**同步**的 |
| `px_shader::assemble` 的桩表改成宿主可提供 | ~40 行 | 见下 |

**合计 ≈ 3000–3500 行，其中真正全新 ≈ 1200–1500 行。**

### §103.1 桩表要改成「宿主提供」（唯一允许动的共享件）

今天 `px_shader::assemble::bevy_stub(symbol)` 是一张**写死的**表，把 `bevy_pbr::*` 替成
**最小声明**（`view` / `lights` / `clustered_lights` / `globals` / `depth_prepass_texture` / …）。

两个宿主**必须同时活着**（一个当锚），而它们对同一批 `#import bevy_pbr::*` 的兑现方式不同：

- Bevy 宿主：运行期由 **naga_oil** 用 Bevy 自己的实现兑现；
- wgpu 宿主：**没有 naga_oil**，必须自己兑。

⇒ 把桩表改成**参数**：

```rust
pub trait Stubs { fn get(&self, symbol: &str) -> Option<&'static str>; }
pub fn render_source(source: &str, modules: &ModuleTable, stubs: &dyn Stubs, seen: &mut Vec<String>) -> String
```

- `px_render`（Bevy）与 `px_probe` 传**今天那张表**（一个 `BevyStubs` 零尺寸类型），行为逐字节不变；
- `px_render_wgpu` 传**自己的表**：`view` / `lights` / `globals` / `depth_prepass_texture` / `clustered_lights`
  用**同样的绑定号**声明，而 `bevy_pbr::shadows::fetch_point_shadow` 给的是**真实现**（采样我们自己的 cube 影子图），
  不是今天那个 `return 1.0` 的桩。

⚠ **改这一处必须证明 Bevy 宿主没变**：`px_render` 重新出一张 `orbit-bare` 与旧哈希逐字节相同。

**内容 shader（`art/shaders/*.wgsl`）在这一步一个字都不改。** 把 `#import bevy_pbr::*` 改名成
`#import planet_x::*` 是**最后一步**（§105 的 S8），那时两个宿主的判据已经全绿，
改名之后**像素一个都不许动** —— 那正是"剥离"完成的判据。

---

## §104 十三个坑（每一条都付过代价）

1. **绑定号会改像素。** §65 记过：云那张 cube 从第 1 格挪到第 5 格、布局多出 6 个空格，
   `orbit-soft` 就差了 **22–33 个像素**（最大通道差 2），**至今没归因**。
   ⇒ **照抄 Bevy 的 group 0 绑定号**（`view`=0、`lights`=1、`clustered_lights`=8、`globals`=11、
   `depth_prepass_texture`=20），把变量减到一个。省下的每一步都是判据上的噪声。
2. **点光 cube shadow map 是本轮唯一没有现成参考的活。** wgpu 官方 `shadow.rs` 是**方向光的 2D array**，
   不是 cube；`gl_Layer` 那套在 WGSL 里不存在 ⇒ 要么 6 个 pass，要么 **multiview**
   （`multiview_mask = NonZero::new(0b111111)` + `@builtin(view_index)`，**u32 不是 i32**，v28 改的）。
   采样是 `texture_depth_cube` + `textureSampleCompare`（**没有 layer 参数**）。
   ⚠ 采样用的影子图**不许加 `TextureUsages::TRANSIENT`**（那是只给 RENDER_ATTACHMENT 的）。
3. **PNG 必须用 `image` 的同一行代码**，否则哈希对不上。Bevy 的 `save_to_disk` 做的是：
   ```rust
   let img = screenshot.image.clone().try_into_dynamic()?;   // Rgba8UnormSrgb 读回
   let img = img.to_rgb8();                                  // ← 丢掉 alpha
   img.save_with_format(&path, image::ImageFormat::Png)      // image 0.25 的 PNG 编码器
   ```
   实测产物：**8 位 / 颜色类型 2（真彩无 alpha）/ 单个 IDAT / 无辅助块**（`a-base.png` 300012 字节）。
   `image` 走 `default-features = false, features = ["png"]`；它拖 `moxcms`（10.4 s）、`png`、`fdeflate`、
   `miniz_oxide` —— 那是 54.7 s 基线里约 39 s 单元时间的一部分，**认了**。
4. **`gpu_ms` 的段必须与 Bevy 那几段对齐**，否则 §51 的历史表全变成不可比的数。
   Bevy 的读数是 `render/**/elapsed_gpu` 的**求和**，参与的是 `main_opaque_pass_3d` /
   `main_transparent_pass_3d` / `prepass` / mip 生成 + `tonemapping` / `upscaling` 两条编码器级 span
   （互不嵌套，所以和 ≈ 一帧 GPU 忙时）。`GpuMs.source` 那个字符串要把参与路径列全。
   ⇒ 自己开时间戳时，**切在同样的边界上**。
5. **就绪门会简单很多 —— 但别顺手删掉判据。** Bevy 那边 `PipelineCache` 的
   `Queued/Creating/Ok/Err` 是 §62 那条 fail-fast 闸门的**全部依据**；裸 wgpu 的
   `create_render_pipeline` 是**同步**的（编不出就是编不出）⇒ `RenderReady` / `watch_pipelines` /
   `PIPELINE_WAIT_BUDGET` / `pipeline_gate` 大半可以删。
   **但"坏管线当场拒、不静默出缺材质的图"这条判据要原样保留**（用 `push_error_scope` 抓，
   29 里是 guard：`let scope = device.push_error_scope(..); … scope.pop().await`，
   **没有 `Device::pop_error_scope`**）。
6. **`px_pass` 拿到的 WGSL 必须是自足且已经替过占位符的**：入口文本里还有
   `#{MATERIAL_BIND_GROUP}`，`naga::front::wgsl::parse_str` 会当场拒。先过
   `render_source`（替占位符）再喂给执行器 —— `px_render/src/passes.rs::resolve` 就是这么写的，照抄。
7. **`--sheet` 的 12 格**：Bevy 是 12 台 `Camera` 各带 `Viewport`，pass 表那边
   `target.post_process_write()` 每格一次 ping-pong。裸 wgpu 里这是**同一个 render pass 里 12 个 viewport**，
   或者 12 个 pass 写进同一张图的子矩形 —— **两条路都可能产生不同的光栅化落点**，选一条之后
   用 J2 判。零碎但会咬人。
8. **实体分类那条规则要翻译过去**（§13）：Bevy 那边是 `StageCamera` / `Stage`（灯）/ `WorldPart`，
   重建世界**只动 `WorldPart`** —— 把相机一起 despawn 会让四张图全是背景色。
   裸 wgpu 里对应的是"相机与灯是常驻状态、只有 draw list 每请求重建"。
   ⚠ **"改尺寸再改回来必须逐字节回到原样"是一条免费的强断言**，照旧要过。
9. **后端锁死 Vulkan，且要喊出来。** `Backends::VULKAN`（DX12 实测慢 2.3×、长尾 3.6×）；
   环境变量**不许盖过它**；非 Vulkan 就 `exit(2)` 并在 stderr 打固定前缀（`tools/harness.ps1` 的
   fail-fast 正则认它）。⚠ `serve` 是**写完租约才起 app** ⇒ 断言失败退出前要**清掉自己那份租约**
   （`clear_own_lease`），否则下一次合法启动会被单例闸拦成**假故障**（实测踩过）。
10. **服务是单例，且两个渲染循环并列会让双方的性能数据都作废。** 租约文件
    `target/render-server.json`（删掉它 = 停服务，4 秒内自查退出）。
    ⇒ **开发期不要同时跑 bevy 宿主与 wgpu 宿主**：先把 bevy 宿主的图**烘成文件**当锚（§105 的 S-1），
    之后同一时刻只跑一个。⚠ 改代码前先停服务，否则 exe 被占用、`cargo build` 报「拒绝访问」。
11. **`wgpu 29` 的 API 拼写**（照错一处就白烧一轮）：
    `InstanceDescriptor::new_without_display_handle`（`Default` / `from_env_or_default` **没了**）｜
    `request_adapter` 回 `Result`｜`PipelineLayoutDescriptor{ bind_group_layouts: &[Option<&BindGroupLayout>], immediate_size: 0 }`｜
    `DepthStencilState::{depth_write_enabled, depth_compare}` 是 `Option<..>`｜
    `MapMode::Write` 走 `WriteOnly<[u8]>`｜`PollType::wait_indefinitely()`｜
    `ImageCopyTexture/ImageCopyBuffer/ImageDataLayout` → **`TexelCopy*`**｜
    `create_buffer_with_data` **没了** → `wgpu::util::DeviceExt::create_buffer_init`（`BufferInitDescriptor` 在 `wgpu::util` 里，不在根）｜
    `SurfaceError` **没了** → `CurrentSurfaceTexture` 枚举｜`entry_point: Option<&str>`。
12. **GPU 时间戳可能在你的队列族上不可用**：wgpu-hal 的 Vulkan 后端要求
    `timestampValidBits >= 36` 才一起开那三个 feature。⇒ 运行期查 `adapter.features()`，
    没有就**降级**（`gpu_ms: null`），不许 panic。本机 RTX 3060 / 596.36 是三个全开、周期 1 ns。
13. **截图不要读交换链。** surface 只保证 `Bgra8Unorm(Srgb)` 且只保证 `RENDER_ATTACHMENT`
    ⇒ **渲到自己的 `Rgba8UnormSrgb`（`RENDER_ATTACHMENT | COPY_SRC`）再回读**；
    BGRA 喂给 RGBA 的 PNG 会把红蓝换掉，而 `view_formats` 只切 sRGB 不换通道序。
    离线那条路根本不需要 surface。

---

## §105 开工序（一档一判据）

### S-1｜先把锚备好（**这一步不做完不许写一行新代码**）

```powershell
# 1) 停掉可能在跑的 bevy 服务，再编出锚
Remove-Item target\render-server.json -Force -ErrorAction SilentlyContinue
cargo build -p px_render
New-Item -ItemType Directory -Force -Path target\oracle | Out-Null
Copy-Item target\debug\px_render.exe target\oracle\px_render-bevy.exe

# 2) 烘内容（顺序不能换：shaders → planet/clouds → scene）
cargo run -q -p px_graphs --bin shaders
cargo run -q -p px_graphs --bin planet
cargo run -q -p px_graphs --bin clouds
foreach ($s in 'orbit-bare','orbit-rings','orbit-soft','orbit-proxy-fine-bound') {
    cargo run -q -p px_graphs --bin scene $s      # 打印「产物 scene -> <path>」，记下来
}

# 3) 起锚服务，出四档图 + 一份 sheet，全部存进 target\oracle\
target\oracle\px_render-bevy.exe --serve --pcg-root target\pcg
target\oracle\px_render-bevy.exe --scene <orbit-bare 产物> --out target\oracle\orbit-bare.png
# …其余三档同上
```

**判据 S-1**：`target/oracle/` 里四张 PNG + 一份 `sheet.png` + 一份 `hashes.txt`（sha256），
且 `orbit-bare` 在**默认尺寸 960×640、不给 `--cam`**（用产物自己的相机）上的哈希是
**`63184151909371A5`**（`art/13-passtable.md` §86.1 的既有读数，§86.4 那张对照表也是同一个数）。
对不上说明 CAS 或内容变了 —— **先查清再往下走**。

⚠ 四档图必须**在同一台机器、同一个后端（Vulkan）、同一份 exe** 上一次性取完，
并把当时的 `--width/--height` 与 `--cam`（或留空）**写进 `hashes.txt`**——
"这张图是拿什么取的"少了任何一格，下一次就复现不了。

### S0｜地基：crate + 设备 + 截图回读 + PNG

`px_render_wgpu/` 新 crate（workspace `members` 加它，**不加进 `default-members`**）。
依赖只准这些：`wgpu 29`(vulkan+wgsl) / `winit 0.30` / `naga 29`(wgsl-in) / `serde` / `serde_json` /
`image 0.25`(png) / `notify 8` / `glam 0.32` / `encase 0.12` / `bytemuck` / `pollster` / `blake3` +
本仓的 `px_protocol` / `px_shader` / `px_pass`。

**判据 S0**：`--device` 那一档「进程 → 设备就绪」**≤ 600 ms**（基线 329–570 ms）；
一张纯色图渲到自己的 `Rgba8UnormSrgb` → 走 §104 第 3 条那条 PNG 路径 → 文件写出来、哈希稳定；
`cargo build -p px_render_wgpu` 冷编 **≤ 90 s**（记下来，这是 J5 的第一次读数）。

### S1｜group 0 的契约 + 桩表改成宿主提供

按 §103.1 改 `px_shader::assemble`（加 `Stubs`），`px_render` 与 `px_probe` 传今天那张表。
`px_render_wgpu` 写自己的表。

**判据 S1**：① `cargo test` 全绿；② **Bevy 宿主重出一张 `orbit-bare` 与 S-1 的哈希逐字节相同**
（证明桩表改动没伤到锚）；③ `px_render_wgpu` 能把 4 份内容 shader 组装出来并 `naga` 校验通过。

### S2｜网格 + 通用材质（**无灯**场景）

搬 `mesh.rs` / `material.rs` / `reflect.rs` / `art_cache.rs`。管线缓存键 =
`(shader 内容版本, cull, alpha)`；bind group 用**固定超集**（12 格贴图 + 参数块，空格填 1×1 白兜底）。

**判据 S2**：自己造一份**无灯**的 `.pxart`（把 `lights` 清空、环境光给 0），
两个宿主各出一张，**逐字节相同**。这一档只验"网格 + 材质 + 参数打包"，别混进灯。

### S3｜灯 + 点光 cube shadow map ← **第一道真判据**

**判据 S3**：**`orbit-bare-shadow` 逐字节相同**（`C03FFF3235264DD5`）——
= `orbit-bare` ＋ planet part 上多一个 `shadows = 1`，行星 + 大气 + 一盏点光 + 星空盒、**不含云**。

> ⚠ **就地更正（§109.3 实测）**：这一档原来写的是"`orbit-bare` 逐字节相同"，
> 而 **`orbit-bare` 的配方里没有 `shadows` 键** ⇒ `sun.shadows = number_or("shadows", 0.0) > 0.5`
> 判成 false ⇒ `shadow_maps = 0` ⇒ `fetch_point_shadow` **根本不被调用**、cube 影子图连建都不建。
> 拿它当判据等于**这一档什么都没验**，而那 ~450 行会一路溜到 S6。
> 用户裁决：把 bare **改造成需要采样阴影** ⇒ 派生档 `art/scene/orbit-bare-shadow.toml`，
> 锚已取（§109.4）。原 `orbit-bare` 那一格仍留在 J1 里（S6），它照旧是 `63184151909371A5`。

这是 J1 的第一格，也是整个工程最可能卡住的一格（§104 第 2 条 —— 但那条**有三处是错的**，
按 §109.1 更正后的版本做）。

### S4｜天空盒 + 色调映射

**判据 S4**：`orbit-rings` **逐字节相同**（行星 + 大气 + 环）。

### S5｜pass 表接通

`px_render_wgpu` 自己实现 `executor_layout()`（照抄 `px_render::passes::executor_layout`）+ 宿主侧
ping-pong（`target.post_process_write()` 的等价物）。

**判据 S5**：J3 全过 —— §86.1 的 A/P/R/D 四条 + §86.4 那六张图的哈希
（`63184151909371A5` / `733408200119C203` / `745BE24FE1467192` / `2D61519C6544E3C1`）。

### S6｜服务 + 报告

`--serve` / 租约 / `ProtocolId` 握手 / 批量请求 / `--sheet` / `--report`。
⚠ `gpu_ms` 的段按 §104 第 4 条切。

**判据 S6**：J1 四档 + J2 sheet 全绿；J4 —— 四档的 `pair.gpu_delta_ms` **排序与量级**与 bevy 宿主一致；
`tools/frame-probe.ps1` 那一套能跑通。

### S7｜viewer

winit 窗口 + orbit 相机 + 鼠标输入 + shader 热重载（`notify` + 重建管线）。
⚠ 起窗口必须**脱离**（`Start-Process` 不带 `-Wait`）；做完要把窗口调出来给用户 review。

**判据 S7**：开窗口能看；改一个 `.wgsl` 存盘，**约 1 秒内**画面变（对齐既有的 §40 那条口径）。

### S8｜切换与拆除（**判据全绿之后**）

1. `px_probe` 改成不依赖 `px_render`（它只用 `shaders::assemble`，见 §102）→ 让 `px_probe` 也不再拖 bevy。
2. 把 `art/shaders/*.wgsl` 里的 `#import bevy_pbr::*` **改名**成 `#import planet_x::*`
   （先建 `planet_x::view` 这个**真库**，它声明 group 0，绑定号与 §104 第 1 条一致）。
3. 重烘 → 重出 → **J1/J2/J3 必须还是逐字节相同**。这一条过了，才叫"剥离完成"。
4. 删 `px_render`（Bevy 宿主）、删 `px_shader::assemble` 里的 bevy 桩表、删 `slots.rs`。
5. 记终账：J5 的五个读数（冷编 / 改一行 / 产物 / 到设备 / 到管线全就绪）。

---

## §106 命令与仪器（每一条都回到它自己那一节读用法）

```powershell
.\tools\px.ps1 -Target test        # 快速测试链（默认 members，不碰 bevy）
.\tools\px.ps1 -Target device      # 最便宜的 GPU 门（约 10 秒）
.\tools\px.ps1 -Target field_dual  # 梯度对错的唯一判据（§46.3）

cargo run -q -p px_graphs --bin shaders          # 烘 shader 库（入口 = art/shaders/*.wgsl 里没有 #define_import_path 的）
cargo run -q -p px_graphs --bin planet           # 烘星球（height + surface mesh）
cargo run -q -p px_graphs --bin clouds           # 烘云
cargo run -q -p px_graphs --bin scene orbit-bare # 配方 → 场景产物（打印 CAS 路径）
cargo run -q -p px_graphs --bin passes -- <场景> invert <输出>   # pass 配方 → 带 pass 表的文档

Remove-Item target\render-server.json -Force     # 停服务（4 秒内自查退出）；改代码前必做
```

**对比两张图**（判据 J1/J2 的取法）：比 `sha256`；不相等再逐像素找差在哪
（`target/passdoc/pixdiff.py` 要 `PIL`，本机默认 python 没有 —— 要么补依赖，要么用
`System.Drawing` 自己算，别因为工具缺失就"跳过这一步"）。

---

## §107 别做的事（写完这一节是因为它们最像"顺手就做了"）

- **别动 `px_render` 的行为**：它是锚，J1–J3 全靠它。改它之前先问"我还要不要那张旧图"。
- **别在 S3 之前碰云**：云的 shader 最长（722 行）也最敏感，灯没通之前调它等于在噪声里找信号。
- **别把 `px_pass` 的 `Layout` 抄一份常量进去**：那是 §66.1 那颗「同一条契约、两个数字」的雷。
- **别顺手把 `protocol_hash` / `SCENE_SCHEMA` 改了**：改了在跑的旧服务当场被握手拒 —— 是设计，
  但不是这一步该付的代价。
- **别用固定帧数预热**：就绪门要 `total > 0 && pending == 0`（裸 wgpu 里这条会简单很多，
  但"别拍空白"这条不变）。
- **别把探针挂进 `cargo test`**：探针不是门，退出码才是判据（本仓规矩）。
- **别一次改两个变量**：这张图上每一个读数都要能归因到一处改动。

---

## §108 开工序实测（2026-09-16）

> 这一节是**往下做之前先把锚钉死**的记录。每一档做完就往这里追加一段，
> 判据没过的那一档**留红**，不修掉不许往下走。

### §108.1 S-1 锚备好了 —— **判据过了**

一条命令跑完：`target/oracle/s1-anchor.ps1`（不入 git，与 `passdoc/run.ps1` 同一个规矩：
仪器放 `target/` 下）。它做的事与工单 §105 S-1 一致，另外把**"这张图是拿什么取的"**
一并写进 `target/oracle/hashes.txt`。

| 项 | 读数 |
|---|---|
| 锚 exe | `target/oracle/px_render-bevy.exe`，sha256 `D7ED54FDB8323EDD…` |
| 后端 / 尺寸 / 相机 | Vulkan ／ 960×640（默认）／ 不给 `--cam`（用产物自带相机） |
| 内容 | shaders 6 份；planet 0 个节点重算（命中）；**clouds 14 个节点重算，45.2 s**（这一档最慢） |
| `orbit-bare` | **`63184151909371A5`** ✓ 与 §86.1 的既有读数一致（300012 B） |

⚠ **一处与 `run.ps1` 不同的地方**：`run.ps1` 借了 `generic-render` worktree 的
`CARGO_TARGET_DIR`，所以它那次用的 exe 其实不在这个 worktree 里。这次的锚
**显式钉在 `$root\target`**，与判据同一个 worktree。

**四档 + sheet 的哈希（新读数，工单原来只钉了 `orbit-bare` 那一格）**：

| 档 | 哈希 | 字节 |
|---|---|---|
| `orbit-bare` | `63184151909371A5` | 300012 |
| `orbit-rings` | `B5799E4F1649535C` | 508562 |
| `orbit-soft` | `FA20FAD37BC61EA2` | 358066 |
| `orbit-proxy-fine-bound` | `32872F80AC867BE3` | 405380 |
| `sheet`（`--sheet` on `orbit-bare`） | `A94F9F2D1437C06C` | 3159948 |

⇒ J1/J2 的五个比对值从这一刻起是**实测值**，不是"待测"。⚠ 它们绑死在
「这台机器 + Vulkan + 这一份 exe」上；换 exe 就要重新确认 §86.1 那一格还对得上。

**顺带被这次出图证实的两件事**：

- `--sheet` 是 **4 列 × 3 行 = 12 格**（3840×1920），与 §104 第 7 条的描述一致。
- 缺格绑兜底图是**正常路径**：`orbit-bare` 的 `planet` 物体在第 3 / 第 5 格声明了贴图而产物没给，
  服务日志明说"绑兜底图（采到的是纯白）"。这与 §107「别把缺格当故障」是一件事 ——
  但反过来说，**新宿主必须把同样的兜底做出来**，否则这两格会变成绑定错误。

### §108.2 S0 地基 —— **判据过了**

新 crate `px_render_wgpu`（workspace `members` 里有它，**`default-members` 里没有** ——
`tools/px.ps1 -Target test` 不许因为它变慢）。S0 只做一件事：**设备 → 自己的
`Rgba8UnormSrgb` → 回读 → PNG**，把这条路径单独验干净，再往上加东西。

| 判据 | 读数 | 结果 |
|---|---|---|
| 进程 → 设备就绪 ≤ 600 ms | 暖 **570 / 548 / 557 ms**（冷 640 ms） | ✓（基线 329–570，§94） |
| 纯色两跑哈希相同 | `137D1059C0C03DBD` == `137D1059C0C03DBD` | ✓ |
| PNG 形状与锚一致 | `IHDR+IDAT+IEND`｜8 位｜颜色类型 2｜单个 IDAT | ✓ |
| 冷编 ≤ 90 s | **26.8 s** | ✓ |
| 改一行自己的代码 ≤ 5 s | **1.3 s** | ✓ |
| 产物 ≤ 20 MB | **10.19 MB** | ✓ |

对照 §92 的 bevy 宿主：冷编 **366.4 → 26.8 s（13.7×）**、
改一行 **13.0–28.6 → 1.3 s（10–22×）**、产物 **156.6 → 10.19 MB（15.4×）**。

**三条被这次实测钉住的事实**（后面每一档都要用）：

1. **设备层锁死 Vulkan 是有效的**：选到的是 `NVIDIA GeForce RTX 3060 Laptop GPU`，
   三个时间戳 feature 全开、周期 **1 ns**（与 §104 第 12 条的预期一致）。
   `max_bind_groups = 8` ⇒ §104 第 1 条那套 group 0/3 的绑定号有足够空间。
2. **PNG 那条路径与 Bevy 逐块同形**：`image 0.25.10` 的
   `RgbaImage → DynamicImage::ImageRgba8 → to_rgb8() → save_with_format(Png)` 出来的
   就是 `IHDR+IDAT+IEND` / 8 位 / 颜色类型 2 单 IDAT / 无辅助块，与锚 `orbit-bare.png` 一模一样。
   这是 S3 逐字节判据能成立的前提，先在这里单独验掉。
3. **`LoadOp::Clear` 的 `wgpu::Color` 是线性值，落盘时按 sRGB 编码**：
   清 `(0.25, 0.5, 0.75, 1.0)` 得到首像素 `R=137 G=188 B=225 A=255`
   （= `round(255·sRGB(0.25/0.5/0.75))`）。**这一条要记住**：主 pass 的清屏色、
   以及 pass 表里 ping-pong 的清屏，都得按这个语义给值，否则会差一档。

**与工单的一处偏离（按"能动态的就动态、最小化编译时间"这一条改的）**：
S0 的依赖里**没有 `glam`、也没有 `encase`**。两者都是单态化大户（§92 实测 `glam` 一个就值
11.5 s 冷编），而本仓的参数打包**本来就是动态的**（`px_protocol::material::pack` 出的是字节串），
`UniformBuffer<T>` 这类泛型壳子一个都用不上。相机/轨道那点数用一个几十行的平面 `f32`
数学模块代替即可。`wgpu` 自己的 `VertexBufferLayout` 也是运行时表，不需要为每种顶点布局生成代码。

⚠ 一条操作上的坑（记下来省下一次）：`[System.IO.File]::ReadAllBytes` 用的是**进程当前目录**，
`Set-Location` 改的那个对它不生效 —— 仪器脚本里一律用绝对路径。

**仪器**（不入 git，与 `passdoc/run.ps1` 同一个规矩）：`target/oracle/s0-probe.ps1`（设备/哈希/PNG 形状）、
`target/oracle/s0-coldbuild.ps1`（冷编/增量/体积），读数落 `target/oracle/s0-build.txt`。

### §108.3 S1 group 0 契约 + 桩表改宿主提供 —— **判据过了**

**唯一允许动的共享件**（§103.1）动完了：`px_shader::assemble::render_source` / `expand`
多一个**桩表**参数，`px_render`（Bevy 宿主）与 `px_ops`（烘图侧）显式传 `bevy_stub`。

⚠ **与 §103.1 原稿的一处偏离**：原稿写的是 `&dyn Stubs`（trait），这里落成
**函数指针** `pub type Stubs = fn(&str) -> Option<&'static str>`。理由是 §100 的用户口径
（"能动态的就动态、减少类型检查与单态化"）：两个宿主对同一批 `#import bevy_pbr::*` 的
兑现方式不同，但组装器**只有一份** —— 不该为"宿主是谁"单态化出两份代码，也不该为一次
间接调用养一张 vtable。`bevy_stub` 的签名本来就长这样，所以 Bevy 那一侧的改动是**加一个实参**。

| 判据 | 做法 | 读数 |
|---|---|---|
| ① `cargo test` 全绿 | `tools/px.ps1 -Target test`（36 个套件） | **exit 0**；`px_render_wgpu` 4 个单测也过 |
| ② **锚逐字节不变** | 重编 Bevy 宿主 → 同尺寸同相机重出**五张** | **五格全等**（见下） |
| ③ 四份内容 shader 组装 + naga 校验 | `px_render_wgpu --shaders` | **四份全过** |

判据 ② 的实测（S-1 的读数 → 改完共享件之后重出）：

| | `orbit-bare` | `orbit-rings` | `orbit-soft` | `orbit-proxy-fine-bound` | `sheet` |
|---|---|---|---|---|---|
| S-1 | `63184151909371A5` | `B5799E4F1649535C` | `FA20FAD37BC61EA2` | `32872F80AC867BE3` | `A94F9F2D1437C06C` |
| S1 之后 | `63184151909371A5` | `B5799E4F1649535C` | `FA20FAD37BC61EA2` | `32872F80AC867BE3` | `A94F9F2D1437C06C` |

⇒ 桩表那条路**没有**参与 Bevy 宿主运行期的兑现（它跑的是 naga_oil + 真的 `bevy_pbr`），
所以这个结果是预期的 —— 但"预期"不算判据，**哈希算**。

**判据 ③ 顺带把 group 0 契约**反射**出来了**（这是它的真本，不是抄的）：

| 符号 | group | binding | 空间 | 谁用 |
|---|---|---|---|---|
| `view` | 0 | 0 | uniform | 四份里的三份 |
| `lights` | 0 | 1 | uniform | `surface` |
| `clustered_lights` | 0 | 8 | **storage** | `surface` / `atmosphere` / `clouds`（经 `planet_x::light`） |
| `globals` | 0 | 11 | uniform | `clouds` |
| `depth_prepass_texture` | 0 | 20 | texture | `atmosphere` / `clouds` |
| `params` | 3 | 0 | uniform | 四份全有 |
| 贴图槽 `albedo`/`glow`/`coverage`/`ring` | 3 | 1…6 | texture+sampler | 按各自声明 |

**两条这次才看清的事实**：

1. **`ring.wgsl` 一个 group 0 绑定都没有**（只有 group 3 的 `params` + 环贴图）。
   ⇒ 环那一档不需要视图组 —— 建 bind group layout 时别按"四份都一样"假设。
2. 内容 shader 实际读到的 group 0 字段只有**六个**：
   `view.{world_position, exposure, clip_from_view, viewport}`、`lights.ambient_color`、
   `globals.time`（`clustered_lights.data` 是整块取的）。零依赖的 `_texture` 格
   （`depth_prepass_texture`）只被 `textureSample` 用。⇒ 新宿主要喂的数据面很窄，
   但**值必须一模一样**（尤其 `clip_from_view` 与 `exposure`）。

**落点**：`px_render_wgpu/src/stubs.rs`（宿主桩表 = group 0 契约，**只有**
`fetch_point_shadow` 与 Bevy 那张不同，其余逐字转给 `bevy_stub` ⇒ 绑定号由**构造**保证一致，
而不是两边各抄一遍）、`src/shader.rs`（模块发现 + 组装 + naga 校验 + **绑定反射**）。
⚠ 绑定号**不在 Rust 侧抄常量**：`--shaders` 打印的 `(group, binding)` 是从组装出来的 WGSL
**反射**出来的。抄一份常量进 Rust 就是 §66.1 那颗「同一条契约、两个数字」的雷。

### §108.4 顺手抓到的一个**真 bug**：`tools/px.ps1 -Target test` 根本跑不起来

§105 的 J6 与 §106 都写着 `.\tools\px.ps1 -Target test`，而它在这个仓里**一直是坏的** ——
只是没人踩到（只有**恰好输出一个元素**的那条分支才踩得到）。

- **症状**：`cargo test` 报 `error: unexpected argument 's' found`，然后 `Usage: test [OPTIONS] [TESTNAME]`。
- **根因**：PowerShell 的 `switch` 当**只有一个**匹配分支、且该分支只输出**一个**元素时，
  会把数组**拆成标量**。`-Target test -Level dev` 时 `@('test') + @()` ⇒ 字符串 `"test"`；
  而 `& cargo @cargoArgs` 对**字符串**是**按字符**摊开的 ⇒ cargo 实际收到 `test t e s t`。
  报的是 `'s'` 而不是 `'t'`，是因为 `t` 被吃成了 `TESTNAME`（clap 的位置参数）。
- **为什么以前没红**：`-Level opt`/`release` 会让 `$extra` 有内容 ⇒ 数组≥2 个元素 ⇒ 不拆。
  而 `-Target test` 走的正是"`$extra` 为空"这条唯一的独木桥。
- **修法**：`$cargoArgs = @(switch ($Target) { … })` —— 一行 `@()`，行为不变，命令真的能跑了。
- **教训**：与 §87 那两条同一族 —— **"文档里写着的命令"必须真的被跑过一次**。
  这次是判据 ① 顺手跑它才撞出来的，不是读代码读出来的。

### §108.5 S2 起步（**未完，判据未取**）

S2 要的是"网格 + 通用材质，**无灯**场景两个宿主逐字节相同"。先把不依赖相机的那几件落了：

| 件 | 落点 | 状态 |
|---|---|---|
| 平面向量数学（**不用 glam**） | `px_render_wgpu/src/vec.rs` | ✓ 算式逐字抄 `glam 0.32.1`，2 条单测钉住 |
| 网格产物 → 顶点/索引 | `px_render_wgpu/src/mesh.rs` | ✓ `weld_normals` / `outward_winding` / 审计文本逐字搬，2 条单测 |
| 图元 oracle | `px_render/tests/primitive_oracle.rs`（`#[ignore]`） | ✓ 已导出到 `target/oracle/bevy-*.bin` |
| 材质 / 管线缓存 / bind group | —— | 未开始 |
| CAS 装载（`art_cache`） | —— | 未开始 |
| 相机与投影 | —— | 未开始（**在查**） |

**两条现在就定下来的事**：

1. **`vec.rs` 的算式必须与 glam 逐位一致**，所以是从 `glam 0.32.1/src/f32/vec3.rs`
   **连括号一起抄**的（`dot` 是左结合、`normalize_or_zero` 判的是**倒数**是不是有限正数，
   不是拿长度判）。少抄一个括号 = `weld_normals` 焊出来的法线在最后一位分岔 = 逐字节判据红。
2. **图元（icosphere）有个绕不开的依赖冲突**：Bevy 的 `Sphere::ico` 是转手给
   **`hexasphere`** 的，而 `hexasphere` **硬依赖 `glam`** —— 正是 §92 量出来最贵的那个
   （11.5 s 冷编）。而工单 §105 S0 的依赖白名单里没有 `hexasphere`。
   ⇒ 按"最小化编译时间"那条口径，**不引它**，改为**移植**，并用上面那份 oracle
   （Bevy 现场生成、原样落盘）来验，而不是"看着差不多"。
   ⚠ 这条**尚未取到判据**：移植还没落，oracle 只导出了没比对（比对脚本也还没写）。

**仪器**：`target/oracle/bevy-icosphere-r1.0-s64.bin` 等 5 份 oracle（小端拼
`positions → normals → uvs → indices`，文件哈希就是判据）；
导出命令 `cargo test -p px_render --test primitive_oracle -- --ignored --nocapture`。
⚠ `s64` 的顶点数是 **42252 = 10×(64+1)²+2**、三角形 **84500** —— 这个闭式已经对上了。

---

## §109 §104 第 2 条的**更正**，以及 S3 判据的一个真空（一手源码核查）

> 来源：对着 `bevy_pbr/bevy_light/bevy_render/bevy_camera/glam 0.19.1/0.32.1` 的 registry
> 源码逐条核过（行号在下面）。**§104 第 2 条有三处是错的**，另外发现 **S3 的判据根本没碰到影子**。

### §109.1 §104 第 2 条错在哪

| §104 第 2 条原话 | 实际 | 依据 |
|---|---|---|
| "采样是 `texture_depth_cube` + `textureSampleCompare`（没有 layer 参数）" | native 分支走的是 **`texture_depth_cube_array`** + `textureSampleCompareLevel(tex, samp, coords, **i32(light_id)**, depth)` | `mesh_view_bindings.wgsl:12-20` 的 `#ifdef NO_CUBE_ARRAY_TEXTURES_SUPPORT`；native 不走那一支 |
| （暗示硬采样 2×2） | `ShadowFilteringMethod` 的 `#[default]` 是 **`Gaussian`**（`bevy_light-0.19.1/src/lib.rs:305-306`），`px_render` 从不插这个组件 ⇒ **8 次** `textureSampleCompareLevel`，D3D 8×MSAA 点位 + 高斯系数，basis = `orthonormalize(normalize(light_local)) * 0.003 * distance_to_light` | `shadow_sampling.wgsl:70-102 / 382-460 / 517-539` |
| "`gl_Layer` 那套在 WGSL 里不存在 ⇒ 要么 6 个 pass，要么 multiview" | 对：Bevy 是**6 个单层 pass**，`multiview_mask: None`。但**multiview 是行为差异**，不是等价实现 —— 别拿它"优化" | `light.rs:2035-2144, 2860-2896` |

另外两条**不许发明**的东西：**`SHADOW_SHADER_HANDLE` 在 0.19.1 里根本不存在**（影子走
depth-only 的 `PrepassPipeline` 特化）；**group 0 没有第 4 / 第 7 格**
（`point_shadow_textures_linear_sampler` 在 `experimental_pbr_pcss` 后面，本仓没开）。

### §109.2 影子那一档的数（要逐字复现的）

| 量 | 值 | 来源 |
|---|---|---|
| 绑定 | `point_shadow_textures`=**2**（`texture_depth_cube_array`，`Depth32Float`，1024²，6 层，`CubeArray` 视图，`DepthOnly`）；比较采样器=**3**；`clustered_lights`=**8**（storage） | `mesh_view_bindings.wgsl:12-39`、`light.rs:1398-1444` |
| 采样器 | ClampToEdge×3 / Linear / Linear / Nearest / lod [0,32] / **`CompareFunction::GreaterEqual`** | `light.rs:244-269` |
| 影子 pass | 每面一个 view：`world_from_view = translation × looking_at(CUBE_MAP_FACES[i])`，投影 `perspective_infinite_reverse_rh(FRAC_PI_2, 1.0, 0.1)`，`LoadOp::Clear(**0.0**)`，`depth_compare = GreaterEqual`，`DepthBiasState{0,0,0}`，**`cull_mode = 材质的 cull`**（`DocMaterial::specialize` 对 prepass 也设了它） | `light.rs:2058-2067, 2535, 2860-2896`、`prepass/mod.rs:613-651` |
| `light_custom_data` | **`(0.0, -1.0, 0.1, 0.0)`**（`perspective_infinite_reverse_rh` 的 z/w 轴后两格）⇒ shader 里 `depth = 0.1 / major_axis_magnitude` | `light.rs:1266-1270, 1303-1315` |
| `shadow_depth_bias` | **0.08** | `point_light.rs:147` |
| `shadow_normal_bias` | **0.6 × (2/1024) × √2 = 0.0016572815…**（GPU 侧是乘过 texel 的） | `light.rs:442-449` |
| `flags` | `9`（`shadows: true`）／`8`（false）：= bit0 影子 + bit3 `AFFECTS_LIGHTMAPPED_MESH_DIFFUSE` | `light.rs:1248-1345` |
| `color_inverse_square_range` | `(linear_color × intensity/(4π)).rgb` + `.w = 1/range²`（**intensity 是流明，内部除 4π**） | `light.rs:534-537` |
| `position_radius` | `(世界位置, radius=0.0)` —— **`.w` 不是 range**（`light.wgsl` 早就记过这条） | 同上 |
| `decal_index` / `soft_shadow_size` | **`u32::MAX`** ／ **恒 0.0**（PCSS 没开 ⇒ PCSS 那一支永不进） | 同上 |
| 环境光 | `lights.ambient_color = vec4(80,80,80,80)`（白 × 80） | `main.rs:1733-1747`、`light.rs:1756-1757` |
| 谁能投影 | `orbit-*` 里**只有 `planet` 与 `rings`** 有 `cast_shadow: true`；大气（Add）与云（Premultiplied）是 `NotShadowCaster` | `scene.rs:430-431`、`bin/scene.rs:849/902/947/979` |

⚠ **一处必须实测、读代码定不了的**：那 6 个面向矩阵经过 `Quat::from_mat3`（`looking_at` 存的是
**四元数**）→ `Mat4::from_rotation_translation` → `inverse()` → `× 投影` 这条链，
可能与"直接写理想整数矩阵"差 1 ulp。⇒ 要么照抄 glam 的调用序列，要么在 S3 实测里当场比。

### §109.3 ⚠ S3 的判据**碰不到影子**（这一条会改开工序的形状）

`px_graphs/src/bin/scene.rs:983-996`：`sun.shadows = planet.number_or("shadows", 0.0) > 0.5`。
而 **`orbit-bare` 与 `orbit-rings` 的 planet 配方里没有 `shadows` 这个键** ⇒ 两档都是
**`shadows = false`** ⇒ `sun_light()` 给出 `shadow_maps = 0` ⇒
`surface.wgsl` / `clouds.wgsl` 里那句 `fetch_point_shadow` **根本不会被调用**。

| 档 | shadows | 用到 cube 影子图吗 |
|---|---|---|
| `orbit-bare`（S3 判据） | false | **不** |
| `orbit-rings`（S4 判据） | false | **不** |
| `orbit-soft`（J1/S6） | **true** | 是 |
| `orbit-proxy-fine-bound`（J1/S6） | **true** | 是 |

⇒ §105 把 S3 叫做"灯 + 点光 cube shadow map ← **第一道真判据**、整个工程最可能卡住的一格"，
但它给的判据是 `orbit-bare` —— **那档连影子图都不会建**。按现在的写法，
§104 第 2 条那 ~450 行直到 **S6 的 J1** 才第一次被验，而 S6 还同时压着 serve/租约/报告/sheet。

**用户裁决（§109.4 记了）**：**把 `orbit-bare` 改造成需要采样阴影** ——
即以派生档 `orbit-bare-shadow`（= `orbit-bare` ＋ planet part 上多一个 `shadows = 1`）当 S3 的判据。
⚠ 无论走哪条，§105 里"`orbit-bare` 是灯 + cube 影子"这句话都是**错的**，要就地更正。

### §109.4 S3 判据档的锚（已取到）

用户裁决之后落的：`art/scene/orbit-bare-shadow.toml` —— 与 `orbit-bare` 的差别**只有**
planet part 上多一个 `shadows = 1`（内容仍不含云 ⇒ 不违反 §107）。
烘出来 `ec43abadf875`，`px_graphs` 的日志当场印证了它真的开了影子：
`[sun] 灯 Point｜位置 (-4.20,1.15,2.35)｜色 (1.00,1.00,1.00)｜强度 7.600e5｜`**`阴影贴图`**。

| 图 | 哈希 | 字节 |
|---|---|---|
| `orbit-bare`（原锚，复核） | `63184151909371A5` | 300012 |
| **`orbit-bare-shadow`（S3 的判据）** | **`C03FFF3235264DD5`** | 298289 |

两条同时成立才算这份派生档有意义：**原锚没被污染**（还是 `63184151909371A5`）✓，
**两份确实不同**（差 1723 字节 ⇒ `shadows = 1` 真的改变了画面，影子被采样了）✓。
⚠ 两份场景产物的键不同（`28a9b516c132` vs `ec43abadf875`）⇒ S-1 那五格锚一个都没动。

仪器：`target/oracle/s3-shadow-anchor.ps1`，读数落 `target/oracle/hashes-s3-shadow.txt`。

---

## §110 S2/S3 要照着做的数（一手源码核查的收口）

> 来源同 §109。**这一节是"实现时对着抄"的表**，不是调研 —— 每一项都有一手依据。
> 三份核查（相机与投影 / 点光 cube 影子 / icosphere 移植）的完整推导不在笔记里，
> 只留**会改变像素的那些数**。

### §110.1 相机与投影

| 项 | 值 |
|---|---|
| 正常出图的相机 | **不是产物自带的相机表**！`--sheet` 才用 `camera_for`；普通请求走 `probe_camera(step.cam)`，不给 `--cam` 时是 **`from_xyz(0.0, 0.55, 3.15).looking_at(ZERO, Y)`** |
| 朝向 | `look_to`：`back = -dir`、`right = up×back`、`up = back×right`、`rotation = Quat::from_mat3(cols(right, up, back))` —— 相机系右手、前方 −Z |
| 投影 | **无限 reverse-Z 右手**：`m00 = f/aspect`、`m11 = f`、`m23 = −1`、`m32 = near`，其余 0；`f = 1/tan(fov/2) = 1+√2`，`fov = π/4`，`near = 0.1` |
| `far = 1000` | **不进矩阵**，只用于视锥剔除 |
| aspect | `viewport_w / viewport_h`（普通请求 = 图宽/图高；`--sheet` = 格宽/格高），**没有有理数吸附** |
| 深度 | `Depth32Float`；clear **0.0**（reverse-Z 的远平面）；**每处都是 `CompareFunction::GreaterEqual`**（prepass / 主 pass / 天空盒） |
| 深度偏置 | 主 pass 与 prepass 都是 `constant 0 / slope 0 / clamp 0`。⚠ 自定义材质的 `depth_bias` **只是透明排序的偏移，从不上 GPU** |
| 颜色目标 | **`Rgba8UnormSrgb`**（8 位，写入时硬件做 sRGB 编码），MSAA **1** |
| 清屏色 | `Color::srgb(0.004, 0.005, 0.010)` |
| **色调映射** | **一个都不跑**！无 `Hdr` ⇒ tonemapping 节点当场 early-return；而 pxart 的内容 shader **从不调** `tone_mapping`（它们自己乘 `view.exposure` 返回线性辐射度）。⇒ 新宿主**不要**做 tone map、不要 dither、不要 bloom、不要 AA |
| `view.exposure` | `Exposure::default()`（EV100 = 9.7）= `exp2(-9.7)/1.2` ≈ **0.0010019079**（f32 要按 Rust 表达式求，不是按 f64 结果取整） |
| `view.viewport` | **绝对像素矩形** `(x, y, w, h)`（`--sheet` 时是格子矩形），片元坐标也是绝对的 —— `clouds.wgsl` / `atmosphere.wgsl` 依赖这一点 |
| pass 顺序 | prepass（**它清的深度**）→ 不透明 → alpha-mask → 透明 → **px_pass 那几条** → 最后 blit 到输出 |
| 透明排序 | 按 `row2(view_from_world)·(mesh_center,1) + depth_bias` **降序距离**（远的先画） |
| ⚠ alpha 档 | **`Add` 与 `Premultiplied` 在 Bevy 0.19 里是同一套状态**（`PREMULTIPLIED_ALPHA_BLENDING`）—— `Add` **不是**加法混合 |
| `globals.time` | 挂钟（`elapsed_secs_wrapped`）。四档内容的 `wind = wind_skin = 0` ⇒ 与时间无关；**只有 `orbit-soft-wind` 不逐帧可复现** |
| ⚠ sRGB 往返 | 主纹理硬件编码 → blit 采样时硬件解码 → 输出再编码。`encode(decode(b)) == b` 逐字节成立**是驱动相关的**，必须实测（或者绕开：把最终字节写成非 sRGB 视图） |

#### §110.1.1 ⚠ `view_from_world` **不许**用解析逆（实测，不是推测）

Bevy 那条链是 `view_from_world = world_from_view.inverse()`，而 glam 走的是
**通用余子式求逆**（`f32/sse2/mat4.rs::inverse_checked`，源自 glm 的 `glm_mat4_inverse`）。
刚体变换明明有更省事的解析逆 `[Rᵀ | −Rᵀt]` —— **但它不是同一个数**：

```
case 0（默认相机）  通用逆 c1.y = 3E302109 ／ 解析逆 = 3E302108   （差 1 ulp）
                    通用逆 c3.z = C04CA664 ／ 解析逆 = C04CA662   （差 2 ulp）
                    另有若干 −0.0 与 +0.0 的差别
```

⇒ 移植件必须把 glam 那 40 行余子式**连括号一起抄**。省这一步 = 顶点裁剪坐标差 1–2 ulp
= 42k 个顶点里总有那么几个落到另一侧 = 逐字节判据红，而且**归因不到**（正是 §65 那类
"差 20–30 个像素、至今没归因"的来源）。

仪器：`px_render/tests/view_oracle.rs`（`#[ignore]`），导出两份：
- `target/oracle/bevy-view-vectors.txt` —— **6 组** `(输入 → glam 通用逆)` 的**位模式**，
  混了纯平移、带缩放旋转、以及非正交的一般矩阵（免得移植件只对刚体成立）；
- 一段给人看的读数：默认相机的 `world_from_view` / `view_from_world` / `clip_from_view` /
  `clip_from_world`，以及 `clip_from_view` 与 `exposure` 的位模式。

顺带被这次实测钉住的两格（都在 §110.1 那张表里，这里给位模式好对账）：
`perspective_infinite_reverse_rh(π/4, 1.5, 0.1)` 的 `c0.x = 3FCE034C`、`c1.y = 401A8279`、
`c2.w = BF800000`、`c3.z = 3DCCCCCD`；`exposure = f32::exp2(-9.7)/1.2` 的位模式是
**`3A835274`**（≈1.001907978207e-3）—— ⚠ 它**不是** f64 那个值（1.001907888464288e-3）
取整，必须按 f32 表达式算。

### §110.2 影子那一档的判据强度

`orbit-bare-shadow` 这条判据一旦过了，等价于同时钉住：6 面 cube 影子图的**渲染**（每面一个 pass、
`perspective_infinite_reverse_rh(π/2, 1.0, 0.1)`、`Clear(0.0)`、`GreaterEqual`、
`cull = 材质的 cull`、depth-only 无片元）、**采样**（Gaussian 8 tap、`POINT_SHADOW_SCALE = 0.003`、
D3D 8 点位置与系数、`orthonormalize` 基）、以及 `ClusteredLight` 那 64 字节的**每一个数**
（`light_custom_data = (0,-1,0.1,0)`、`flags = 9`、`shadow_normal_bias = 0.0016572815…`、
`decal_index = u32::MAX`、`intensity` 是流明要除 4π）。§109.2 是那张对照表。

### §110.3 icosphere：移植已产出，**判据还没取**

`Sphere::ico` 转手给 `hexasphere`，而它硬依赖 `glam`（§108.5 的取舍）⇒ 移植。
移植件已按 `hexasphere-18.0.0` 的源码逐句产出（**不是**"数学等价"的重写）：

- 基座是**字面量表**（12 个顶点 + 20 个三角形 + 30 条边），不是生成的；
- 细分走 **slerp**（`(a+b)·(2(1+a·b))^−½` 与 sin 形式的 `geometric_slerp_multiple`），
  **从不调 `normalize()`** ⇒ 点会漂在单位球外几 ulp，**要与 Bevy 一样漂**；
- 分配次序是"**先 30 条边的 s 个点，再 20 个三角形各自的内点**"，内点数是 `s(s−1)/2`；
- **边的朝向**由"0..20 里第一个碰到它的三角形"决定（ab → bc → ca 次序），后来的三角形
  **反向**读它 —— 值与索引次序都依赖这个；
- 尺寸闭式：顶点 `10(s+1)²+2`、索引 `60(s+1)²`（s=64 ⇒ **42252 / 253500**）。

⚠ **风险（必须实测，不许假设）**：`uv_of` 用 `f32::acos` / `f32::atan2`，在默认特性下是
**平台 libm**（Windows 上是 UCRT），**不是正确舍入的** ⇒ 同机同工具链逐位一致，
换平台会差几 ulp。`f32::sqrt` 是 IEEE 精确的，没问题。
✅ **"有没有谁打开了 libm"这一格已经查清（实测，不是推测）**：
`cargo tree -p px_render -e features -i hexasphere` ⇒ hexasphere 只开 `default` + `std`，
**没有 `libm`**；`-i bevy_math` ⇒ 开的是 **`nostd-libm`**（不是 `libm`），
而 `ops.rs:609/612` 的选择是
`libm_ops` 需要 `any(libm, all(nostd-libm, not(std)))` ⇒ **不成立**，
`std_ops` 需要 `all(not(libm), std)` ⇒ **成立**。
⇒ 两边走的都是 **`f32::acos` / `f32::atan2`**（std，即平台 libm），与移植件同一套。
⚠ 这条只在**同一台机器 + 同一工具链**上成立；换平台要重新确认（或干脆把网格烘成产物）。

**判据怎么取**：`target/oracle/bevy-icosphere-*.bin` 是 Bevy 现场生成、原样落盘的 oracle
（§108.5），移植件要**对着它逐字节比**，不是"顶点数对上了就算过"。

✅ **判据已经落到 `cargo test` 里了**（比对着 `target/` 的文件强：`target/` 不入 git，
依赖它的测试在新克隆上会假绿或假红）。做法是把 oracle 的 **sha256 当常量**嵌进测试 ——
布局就是导出时那个（小端拼 `positions → normals → uvs → indices`），于是**一个常量覆盖整块数据**：

| 调用 | 顶点 | 三角形 | sha256（前 16 位） |
|---|---|---|---|
| `icosphere(1.0, 1)` | 42 | 80 | `58819F3A63386F9C` |
| `icosphere(1.0, 5)` | 362 | 720 | `B0939AF33378E766` |
| `icosphere(1.0, 64)` | 42252 | 84500 | `B4B37AB464C3A743` |
| `icosphere(1.02, 64)` | 42252 | 84500 | `4044EF96A1C5E99E` |
| `icosphere(1.06, 64)` | 42252 | 84500 | `44212D8611D976DA` |

⚠ **不许为了让测试变绿去改这个常量** —— 它来自真的 Bevy。对不上就是移植错了。

---

## §111 S2 的实施清单（下一步照着做）

### §111.0 S2 的判据档与锚（**已取到**）

`art/scene/orbit-bare-nolight.toml` —— 与 `orbit-bare` 的差别**只有** planet part 上多一个
`light_intensity = 0`（环境光仍是 `80`）。烘出来 `cbf452bfc590`，日志印证：
`[sun] 灯 Point｜…｜强度 0.000e0`。

| 图 | 哈希 | 字节 |
|---|---|---|
| `orbit-bare`（对照） | `63184151909371A5` | 300012 |
| **`orbit-bare-nolight`（S2 的判据）** | **`7BBB18CE3612D4F7`** | 215193 |

**为什么是"强度给 0"而不是"把 `lights` 清空"**：灯是烘图侧从 planet part 的参数里生成的
（`number_or("light_intensity", 7.6e5)`），场景里那盏灯是"颜色 × 强度"的对象 ——
强度 0 才是**内容上真的没有光**。`light.wgsl` 判"亮没亮"看的正是颜色：
`lit = !all(color_inverse_square_range.rgb == vec3(0.0))` ⇒ 强度 0 ⇒ `lit = false`
⇒ 直射光一点不参与，而**环境光还在**。

**为什么环境光留着**：环境光也关掉的话整幅图几乎全黑，PNG 压得极小，
**任何错误看起来都还是黑的** —— 那是一条几乎没有分辨力的判据。
留着环境光，画面是"平的受光面 + 星空盒"（215193 字节 ⇒ 确实有内容，不是全黑），
网格 / 材质 / 参数打包错了都看得出来。

⚠ 两份场景产物的键不同（`28a9b516c132` vs `cbf452bfc590`）⇒ 原有那五格锚一个都没动。

S2 的判据是"自造一份**无灯** `.pxart`，两个宿主各出一张，逐字节相同"。要渲染**任何**场景，
就得把整条链搭起来 —— 没有捷径。按依赖次序：

| # | 落点 | 内容 | 判据 |
|---|---|---|---|
| 1 | `cammath.rs` | `Vec3/Mat3/Quat/Mat4` 子集，**逐位抄 glam 0.32.1 的 SSE2 算法**（§110.1.1：解析逆不成立） | `target/oracle/bevy-view-vectors.txt` 那 6 组向量逐位相同 |
| 2 | `camera.rs` | `probe_camera(opt)`（不给 `--cam` 时 `from_xyz(0,0.55,3.15).looking_at(ZERO,Y)`）+ `perspective_infinite_reverse_rh(π/4, aspect, 0.1)` + `clip_from_world = clip_from_view * view_from_world` | 对着 §110.1.1 那几格位模式 |
| 3 | `group0.rs` | group 0 的**宿主侧**缓冲：`view`（只喂内容 shader 真读的 4 个字段）、`lights.ambient_color`、`globals`、`clustered_lights`(storage)。⚠ 字段偏移**由反射出来的 WGSL 决定**，不在 Rust 侧抄第二份 | 反射出的 `(group, binding)` 与 §108.3 那张表一致 |
| 4 | `art.rs` | CAS 装载：`scene::read_scene` + `Member::resolve` + `cas_path`；网格走 `mesh::load_mesh`，贴图/采样器建 GPU 对象 | 五个成员都能解析出来 |
| 5 | `material.rs` | **固定超集** bind group（`PARAMS_BINDING` + `TEXTURE_SLOTS` 的 12 格，空槽填 1×1 白兜底、cube 槽填白 cube）+ 管线缓存（键 = `(shader 内容版本, cull, alpha)`） | §65 那条"空着的格绑兜底图"要**做出来**（§108.1 记过 orbit-bare 真的会用到） |
| 6 | `render.rs` | prepass（**它清深度**，颜色附件为空、`Store`）→ 主 pass（`Load` 深度、`GreaterEqual`、`depth_write` 按 alpha 档）→ blit 到输出 → 回读 → PNG | PNG 与锚同形（S0 已验过那条路径） |
| 7 | CLI | `--scene <pxart> --out <png> [--width] [--height]` | 两宿主同图逐字节 |

**这一档要一路小心的三件事**（都写在 §110 里，这里只重复最会咬人的）：
- 深度是 **reverse-Z**（clear `0.0`、`GreaterEqual`、`depth_write` 与比较方向都别按直觉写）；
- **一个色调映射都不跑**：内容 shader 自己乘 `view.exposure` 返回线性辐射度，
  所以主纹理是 `Rgba8UnormSrgb`，**写入时由硬件做 sRGB 编码** —— 不要自己再编一次；
- `view.exposure` 的位模式是 **`3A835274`**（f32 表达式算出来的），不是 f64 取整。

---

## §112 一条派工的教训：**逐位移植必须「先写文件、再对着 oracle 迭代」**

这一轮把两件按位移植（glam 的 `Mat4::inverse`、hexasphere 的 icosphere）派了出去，
两份的验收标准都是**硬的**（6 组位模式向量 / 5 个 sha256），但**都没落地**：
连续三轮 `px_render_wgpu/src/` 一个文件没多、`target/oracle/` 一份 scratch 都没有、
机器上**没有任何 cargo/rustc 进程**在跑 —— 它们一直在「读与想」，没进循环。

**根因不在模型，在派工**：我把任务描述成「产出一个模块」，却没要求
「**先写文件、再跑测试、再改**」。对这种「逐句转录 + 有 oracle 可判」的活，
正确的形状是**测试驱动**：

| 错的形状 | 对的形状 |
|---|---|
| 「产出一个无依赖的数学模块」 | 「**只**做 `inverse()` 这一个函数，写完立刻跑那 6 组向量」 |
| 让它在脑内把 glam 的全部 SIMD 路径推完 | 先从 oracle 反推 —— 跑一次就知道差在哪 |
| 范围是「一整块相机数学」 | 范围是「一个函数 + 一个测试」 |

**两条可复用的规矩**（下次派这类活直接用）：

1. **范围切到一个函数。** 「产出 `Mat3`/`Quat`/`Mat4`/`look_to`/`from_scale_rotation_translation`」
   是把五件事捆成一件；捆在一起的任务一旦卡住，**没有任何中间产物可救**。
2. **命令式地要求「先写文件」**，并给出确切的落点与 `mod` 行。
   有 oracle 的任务不该有「分析阶段」—— 写下来跑一次，比想十轮都准。

⚠ 顺带一条零成本的诊断技巧：**`Get-Process cargo,rustc` 就是心跳**。
一个「在跑」但既没写文件、又没有构建进程的 agent，多半是没进循环，不是在努力。

---

## §113 三格锚的**可复现性**复核（判据的前提）

S2/S3 的每一格判据都要拿"Bevy 宿主出的那张图"当基准，所以"那张图**现在还能不能再出一张一样的**"
是**前提**，不是细节。之前只取过一次，这一轮当场复核：

| 档 | 读数 | 复核 |
|---|---|---|
| `orbit-bare`（J1 / S3 对照） | `63184151909371A5` | ✓ |
| `orbit-bare-shadow`（S3 判据） | `C03FFF3235264DD5` | ✓ |
| `orbit-bare-nolight`（S2 判据） | `7BBB18CE3612D4F7` | ✓ |

三格逐字节全等 ⇒ 锚 exe / CAS / 场景产物键在两次取数之间**一个都没漂**，
后面每一档都可以放心地拿这三格当基准。

⚠ 顺带记一条：这次复核走的是**同一个锚 exe**（`target/oracle/px_render-bevy.exe`，
sha256 `D7ED54FD…`）。换 exe 就要重新确认（§108.1 已经写过这条）。

---

## §114 §111 第 1 件落地：两份**逐位判据**都过了（2026-09-16）

§112 那条派工教训改完之后，两件移植**当轮落地并自带硬判据** —— 这反过来印证了
那条教训：把范围切到一个函数、并要求"先写文件再跑测试"之后，同样的两件事一次就成了。

| 件 | 判据 | 读数 |
|---|---|---|
| `mat4.rs`（`Mat4::inverse`） | `target/oracle/bevy-view-vectors.txt` 的 **6 组**向量 × 16 个位模式 | **6/6 全等** |
| `icosphere.rs` | oracle 的 **5 个 sha256**（1.0/1、1.0/5、1.0/64、1.02/64、1.06/64） | **5/5 全等** |

- `cargo test -p px_render_wgpu`：**11 passed / 0 failed**（含这两条位判据）。
- 两条判据都是**真门**，不是"看着差不多"：`mat4` 那条是 `CASES: [(&str,&str); 6]` +
  `assert!(failed.is_empty())`；`icosphere` 那条把 5 个 sha256 写成常量，
  **一个常量覆盖整块 2.3 MB 的数据**，而且不依赖 `target/` 下的文件。
- ⚠ **期望值一个都没被改过**：icosphere 那边在第一次跑之前自己发现并修掉了
  `INITIAL_POINTS` 第 10 个顶点的一位数字笔误，改的是**移植件**，不是判据。

**两条过程上的账**：

1. `icosphere.rs` 曾经**提交了但没接线**（`mod icosphere;` 漏了）—— 那一笔的树其实
   编译不到，靠后一笔补上。教训：`git add -A` 在**别的 agent 也在写同一个目录**时
   不安全（§112 已记过一次同类）；提交前该先看清 `git status`。
2. 我要求"零警告"这条**过严了**：它逼出了 `mat4.rs` 上的模块级 `#![allow(dead_code)]`，
   而那会把"还没接线"与"真的写多了"一起盖掉。已撤掉，改成一段说明 ——
   `mesh.rs` / `vec.rs` / `icosphere.rs` / `mat4.rs` 处在同一阶段，
   都在等 S2 的渲染路径来当它们的调用者。`cargo build` 现在 35 条 dead-code 警告
   （`cargo test` 那条路是干净的），**S2 接上就消失**。

**§111 的进度**：第 1 件**部分完成**（只差 `Vec3/Mat3/Quat`、`look_to`、
`perspective_infinite_reverse_rh`、`from_scale_rotation_translation`）——
第 2 件 `camera.rs` 因此还没开始。S2 与 S3 的判据锚已复核可复现（§113）。

---

## §115 一条**口径确认**：注释照仓库风格写（用户裁决）

`~/.dsh/AGENTS.md` 里有一条 `writing commends is forbidden`（推测是 `comments` 的笔误），
而这个仓库**处处都是**密集的中文 why-注释 —— `px_render` / `px_protocol` / `px_shader`
每个文件都是，`art/15-render-wgpu.md` 自己也用注释体写规格。

**用户裁决：继续按仓库风格写中文 why-注释。**

记下来的理由：这条口径影响此后**每一个文件**，而且这个仓库里注释**不是装饰**——
§65 那类"差 20–30 个像素、至今没归因"的坑，防住它的正是代码旁边那句
"为什么不能用解析逆""为什么 s64 的顶点数是那个闭式"。
把依据只留在笔记里，读代码的人就看不见它。

⚠ 也就此作废了我在 §112 里下的"零警告"那条硬要求：那条**过严**，
它逼出了 `mat4.rs` 上的模块级 `#![allow(dead_code)]`（§114 已撤掉）。
往后的口径是：**警告可以有，但必须是"还没接线"这一类，且要说得清**。

---

## §116 又一个"看起来该省、实测不能省"：相机的**四元数往返不是恒等**

Bevy 建相机位姿走的是 `look_to` → `Quat::from_mat3` → `Affine3A::from_rotation_translation`
（内部再 `Mat3A::from_quat`）—— 也就是"三列 → 四元数 → 三列"绕了一圈。
直觉上这是**多余的**：把 `right/up/back` 三列直接拼成矩阵不就完了？
省掉它就能少移植一整套 Shepperd 代码，而 Shepperd 正是最容易写错的那类。

**实测（四种姿态，全部"不同"）：**

```
cam None                    三列直接拼 == Bevy 那条绕四元数的路：**不同**
cam Some([0.0, 0.0, 3.5])   不同
cam Some([35.0, 20.0, 2.4]) 不同
cam Some([-120.0, -55.0, 6.0]) 不同
```

⚠ 这一格比 §110.1.1 那条更阴：§110.1.1 是"解析逆的**公式**与通用逆不同"，
而这条是"**同一条公式**，只因为中间过了一趟四元数就变了"。两种都只有当过一遍才看得见。

⇒ 移植件**必须**含 `Quat::from_mat3` 与 `Mat3A::from_quat`，且要比的是 **`Affine3A`**
（SIMD 的 `Mat3A`）那条真实路径，不是标量 `Mat3`。

仪器：`px_render/tests/view_oracle.rs::is_the_quaternion_round_trip_the_identity`（`#[ignore]`）。

---

## §117 相机那条位判据现在是**红的**，差 1 ulp（精确诊断已定位）

`camera.rs` + `mat4.rs` 的扩展（`Vec3`/`Mat3`/`Quat`、`from_rotation_translation`、
`from_scale_rotation_translation`、`mul_vec4`/`mul_mat4`、
`perspective_inverse_reverse_rh`）已经写出来了，`cargo test` 12 passed / **1 failed**：

```
camera::tests::the_probe_camera_matches_bevy_bit_for_bit
  assertion `left == right` failed:
    world_from_view[6] (col 1 .z)  got BE302109  want BE302108
    left: 3190825225  right: 3190825224
```

**只差一个 ulp，而且恰好落在 §116 预言的那一格上** —— `world_from_view.c1.z = −0.172001004`。
注意 §110.1.1 那张表里，**解析逆**的 `c1.y/z` 用的是 `3F7C2F4D` / `3E302108`（与位姿矩阵的
`c1.y/z` 同值），而实现给的是 `3E302109` ⇒ 说明这一格是在
`Quat::from_mat3` → `quat_to_axes`/`Mat3A::from_quat` 那一串里舍入掉的。

**下一步就是这一件事**：把移植的那串 `quat_to_axes` 与 glam 的
`f32/sse2/mat4.rs` 的 `quat_to_axes` / `f32/sse2/mat3a.rs:278` `Mat3A::from_quat`
**逐操作数**比一遍 —— SIMD 是 4 路并行，标量转写很容易在
`1.0 - (yy + zz)` 这类表达式的**结合次序**上与它分岔（§110.1.1 是同一族坑的第三次出现）。

⚠ **这一格红着，所以这些改动没有提交** —— 提交的树（`2c15fab`）仍然是
**11 passed / 0 failed**。判据没拿到就不算完成，红着提交等于把门拆了。

---

## §118 相机位判据**转绿**，以及那 1 ulp 的真凶：Bevy 的 `Dir3` 是**除法**归一化

§117 那 1 ulp（`world_from_view[6]` `BE302109` vs `BE302108`）抓到了，真凶不在四元数、
不在 Shepperd、也不在 `quat_to_axes` —— 而在**归一化**：

| 出处 | 写法 |
|---|---|
| Bevy `Dir3::new`（`bevy_math-0.19.1/src/direction.rs:587-594`） | **`value / length`** |
| glam `Vec3::try_normalize` | `value * (1.0 / length)` |

两者数学等价、**浮点上不等价**。`look_to` 里那句 `back = -direction.try_into()` 走的是
Bevy 的 `Dir3` ⇒ **除法**；而同一函数里 `right = up.cross(back).try_normalize()` 用的是
glam 的 `try_normalize` ⇒ **乘倒数**。**同一个函数里两处归一化，写法故意不同。**

⚠ 这是同一族坑在本工程里的**第四次**出现（前三次：§110.1.1 解析逆 vs 通用逆、
§116 四元数往返不恒等、§117 这次的表象）。它们的共同形状是：
**"数学等价"在逐字节判据下不是等价**，而且**都只在跑过一遍之后才看得见**。
⇒ 移植 Bevy/glam 的算式时，凡是"看起来可以统一写法"的地方，都要先当成**有嫌疑**。

**读数**：`cargo test -p px_render_wgpu` = **13 passed / 0 failed**。
`camera::tests::the_probe_camera_matches_bevy_bit_for_bit` 断言了
aspect 位模式 `3FC00000`、`position` 的 `00000000 3F0CCCCD 4049999A`、
以及 `world_from_view` 与 `clip_from_view` 的**全部 32 个位模式**（用 `to_bits()`，
不是近似比较）。

⇒ §111 第 1 件（相机数学）与第 2 件（`camera.rs`）**完成**。下一件是第 3 件 group 0。
