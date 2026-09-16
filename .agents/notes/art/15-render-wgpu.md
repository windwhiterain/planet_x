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

**判据 S3**：**`orbit-bare` 逐字节相同**（行星 + 大气 + 一盏点光 + 星空盒）。
这是 J1 的第一格，也是整个工程最可能卡住的一格（§104 第 2 条）。

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
