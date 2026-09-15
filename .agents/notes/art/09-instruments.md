# 改完怎么验

改完一个功能之后，怎么证明它真的对了：热重载要不要重启、窗口 review 回路、
单帧时间怎么量、探针在哪跑。**渲染器本身怎么跑**在 `08-renderer.md`。

---

## §37 / §42 shader 热重载

**能**。把 `clouds.wgsl` 故意写坏一行、不碰任何 Rust、不重编：

```
INFO  bevy_asset::server:                Reloaded shaders\clouds.wgsl
ERROR bevy_render::…::pipeline_cache:    failed to process shader error: shaders/clouds.wgsl:332:25
```

**1 秒内重载并重编**，文件恢复后自己又好。serve 与 viewer 两个进程都验过。

### §42.1 什么时候**必须**重启

| 改了什么 | 要不要重启 |
|---|---|
| **只有 `.wgsl`** | **不要** —— 存盘即可，约 1 秒生效 |
| `CloudParams` 之类的 **uniform 结构** | **要** —— 布局变了 ⇒ 必须 `cargo build` ⇒ exe 被占用 ⇒ 先停进程 |
| CLI / 系统 / 插件 | **要** —— 同上 |

⚠️ 每次重启的代价是 15–25 秒预热，**而且会把用户的 viewer 窗口杀掉**。

⚠️ **证据要看全，不能只看尾部**：用 `Select-Object -Last 12` 看日志时，窗口的 resize 报错
刚好会把 shader 报错挤出视野 ⇒ 会得出与事实相反的结论。**取样方式决定了你能得出什么结论**
（与 §41.1 那次「在错的分辨率上取证」是同一类错）。

---

## §40 review 回路：做完一个功能**必须**把窗口调出来

这是一条**工作流义务，不是可选项**。

```
px_render --view --scene <SCENE.pxart> --pcg-root target\pcg          # 场景产物决定一切（P10 起）
px_render --show --scene <SCENE.pxart> --shot                            # 推新场景给常驻窗口
```

路径取法：按 **图名 + 节点名** 从 `target/pcg/<图名>/manifest.json` 取 `key`，再拼
`target/pcg/ab/<key 前两位>/<key>.pxart`（`tools/harness.ps1` 的 `Resolve-Artifact` 就是干这个的）。
窗口侧的**契约**（起始 `ablate` 由产物决定、`v/m/n` 只是临时覆盖、推同一份路径是空操作）、
**历史坑**与**判档判据**见 **§56**；本节只管"起窗口 + 验收"的动作。

⚠️ **本段曾经是过期形状，照它找旗标等于被指错路**：`--planet / --mesh / --clouds / --cloud-slope /
--cloud / --cloud-ablate / --scatter / --atmo / --ambient / --palette / --displace / --sea / --radius /
--spin / --rings` 这一串**内容旗标在本分支全删了**（§52 的决定，删单见 `08-renderer.md:195`），内容只走
`--scene`。另外 `--width / --height / --cam` 只对 `--serve` / 出图那条路生效：`--view` 的窗口尺寸是写死的
（1280×800 逻辑，本机 DPI 1.75 ⇒ 物理 2240×1400），相机是窗口自己的轨道相机。

- 窗口是**常驻**的：只开一次，之后每次只用 `--show` **推**，不要每次重开（§27.1）。
- `--show` 写 `target/viewer-scene.json`，窗口每帧轮询；窗口还会盯**场文件的 mtime**
  ⇒ **重烘之后不用推，窗口自己更新**。
- `--show` 会报「窗口在线 / 没在跑」（读 `target/viewer.json` 心跳）。
- **`--shot` 让窗口自己存一张 `target/viewer-shot.png`** —— 这是不看屏幕就能验收的判据；
  截图要等管线就绪。

⚠️ **起窗口必须"脱离"**：用一个**永不退出**的 GUI 进程当后台作业去等，会把自己挂住。
正确起法（PowerShell，**不要**加 `-WindowStyle Hidden`）：

```powershell
$p = Start-Process -FilePath target\debug\px_render.exe `
     -ArgumentList @("--view","--scene",$scene,"--pcg-root","target\pcg") `
     -RedirectStandardOutput target/viewer.log -RedirectStandardError target/viewer.err -PassThru
```

`Start-Process` 不带 `-Wait` 立刻返回，窗口留在用户桌面上，agent 继续干活。
判据（`场景 <名>｜环境光 …｜相机 N 个`、`渲染管线全部就绪…失败 0 条`）都在 `target/viewer.log` 里。

⚠️ **`--scene` 只决定"起手"那一份**：窗口每帧还轮询 `target/viewer-scene.json`，里面残留的请求
（`at` 与启动时不同）照样会被采纳，并打一行 `窗口切到：…` ⇒ 上一个 session 推过的场景会在启动后
不久把起手场景切走。起窗口前先看一眼这个文件：要的就是它就别管，不要它就挪开。
⚠️ 两个都没给（既无 `--scene`、请求文件也没有）时窗口直接报错退出。

⚠️ 未解释的观察：窗口里的**整体曝光比 `--serve` 出图暗**（实测均色 `{41,47,53}` → `{32,33,35}`，
蓝通道掉得最多），两者 `AmbientLight` 与大气壳参数一致。**没查出来**。

⚠️ **交接文档的"能跑什么"清单不等于用法说明**：清单里出现的每条命令，都要回到它自己的那一节读用法。

---

## §41 怎么量单帧时间（`--fps`）

先去掉两道帧率上限，否则量到的是上限不是成本：`--serve` 的 `run_loop(1/60)` 会**睡觉**凑 60 Hz，
窗口默认 **vsync**。

```
px_render --serve --fps --width W --height H      # 服务端不再限速
px_render --view  --fps  --novsync  --scene <SCENE.pxart>   # 窗口 AutoNoVsync
```

`--fps` 每 **120 帧**打一行平均帧时间（`FRAME_PROBE_WINDOW`）。
⚠️ 窗口侧 `--fps` **只**打开帧时间探针，**去掉 vsync 要另外给 `--novsync`**（`--serve` 那边 `--fps`
会把主循环设成 `Duration::ZERO`，窗口这边不会）⇒ 只给 `--fps` 量到的仍是 60 Hz 上限，不是成本。
⚠️ `--fps` 下 `idle_between_jobs` **不关相机**，否则后台渲染只持续 6 帧、凑不满 120 帧。

### §41.1 会骗人的那一半：低分辨率量到的是「地板」，不是 shader

实测（RTX 3060 Laptop / DX12，行星 + 大气 + 云）：

| 分辨率 | 像素 | 无云 | 有云 | 云的成本 |
|---|---|---|---|---|
| 320×200 | 6.4 万 | 13.87 ms | 13.65 ms | **量不到** |
| 960×640 | 61 万 | 13.5 ms | 13.6 ms | **量不到** |
| 2240×1400 | 314 万 | 15.29 ms | 24.34 ms | **9.05 ms** |

有一条**与分辨率无关的 ~13.5 ms 地板**（307k 三角形网格 + 两个壳 + 预通道的顶点/绘制成本）。
**只要 shader 的成本低于地板，帧时间就一动不动** ⇒ 「帧时间没变」**不等于**「shader 免费」。

⇒ **量 shader 一定要在 GPU 真的成为瓶颈的分辨率上量**，或者扫一遍分辨率找到地板在哪里。

### §41.2 优化前后（同一个仪器，2240×1400）

| | 整帧 | 云单独 |
|---|---|---|
| 无云 | 15.29 ms（65 fps） | — |
| 优化前 shader | **35.17 ms（28 fps）** | **19.9 ms** |
| 优化后 shader | **24.34 ms（41 fps）** | **9.05 ms** |

真实窗口（2240×1400 物理像素）：无云 16.8–17.1 ms，有云 31–37 ms。

⚠️ **算子计数只能用来找嫌疑人，不能当成绩**：按算子计数说「干了 7 倍的活」，实测只快 2.2 倍
（19.9 → 9.05 ms）—— 算子砍掉后 shader 从**吞吐受限**变成**延迟受限**，而且提前退出/自适应步数
本来就让不少像素提前走了。成绩必须用 §41.1 的仪器量。

---

## §47 探针搬出 `cargo test`

探针不是测试：它们要 GPU、要几分钟、**退出码才是判据**。现在住在独立 crate `px_probe`：

```
cargo run -p px_probe --bin field_dual   # §46.3 的 arbiter（梯度对错的唯一判据）
cargo run -p px_probe --bin gradient     # 探针冒烟 + 门约定 + 逐通道归因
cargo run -p px_probe --bin device       # 只要「无窗口设备能起来」
cargo run -p px_probe --bin dual         # 对偶数微分本身：分支、夹取、smoothstep、乘积（纯 CPU）
cargo run -p px_probe --bin dual_field   # 云场解析梯度 vs 对偶数（纯 CPU）
cargo run -p px_probe --bin dual_noise   # 噪声解析梯度 vs 对偶数（纯 CPU，--diagnose 打原始数据）
```

后三个原来住在 `px_verify/tests/` 当 `#[test]`，见 §53：它们是判据不是门。它们**纯 CPU**，
所以**不调** `require_gpu()` —— 拿不到设备就 exit 2 那条规矩只对要 GPU 的探针成立。

- **一个进程一个设备**（`OnceLock`），后端固定 **DX12**（`WGPU_BACKEND=vulkan|gl` 仍可覆盖）：
  `Instance::default()` 走 Vulkan 本机要 2.8 s（还在挨个找不存在的 layer JSON），DX12 只用 0.27 s。
- `px_probe` 是 workspace 成员，但**不进 `default-members`** ⇒ `cargo test`（默认那五个）不再碰 bevy。
- 优化程度从命令行选（`opt` 只提升本地 crate，**不重编 bevy**）：

```
cargo run -p px_probe --bin field_dual `
  --config 'profile.dev.package.px_ops.opt-level=2' `
  --config 'profile.dev.package.px_verify.opt-level=2' `
  --config 'profile.dev.package.px_probe.opt-level=2'
```

⚠️ **不加** `[profile.probe]` 自定义 profile：cargo 会为新 profile 全量重编 552 个依赖（一次性 ~3 min）。

### §47.3 顺手堵掉的洞（比快更要紧）

`connect()` 失败时原来是 `eprintln!("跳过：没有可用的 wgpu 适配器"); return;` ⇒ **测试静默变绿**。
而 arbiter 是梯度对错的**唯一**判据，唯一判据可以被静默跳过是比慢严重得多的问题。现在：

- `require_gpu()` 拿不到设备直接 `exit(2)`（并报出请求的后端）；
- 11 处「跳过」全部换成硬失败 `assert!(!rows.is_empty(), "探针没拿到数据…不要把它读成通过")`；
- bin 用 `catch_unwind` 逐个 check、打印 `✓ / ✗`、有失败 `exit(1)`。**断言本身一字未改。**

⚠️ 探针 bin 名里的 `✓ / ✗` 与 `exit(1)` 是脚本判据的来源 —— `.ps1` 要用**退出码**判，
不要靠 `Select-String`。

## §53 门与探针的边界（附实测每条耗时）

**判据**：能失败、且失败就意味着东西错了 ⇒ 可以当**门**（`cargo test`，要便宜）；
需要真计算 / 要 GPU / 要几分钟 ⇒ 当**探针**（显式跑，退出码才是判据，见 §47）。

实测（扣掉测试进程启动基线，Windows / 本机，2026-09）：

| 目标 | 条数 | 净耗时 |
|---|---|---|
| `dual_field::the_dual_gradient_of_the_field_matches_the_fields_own_values` | 1 | **297.7 ms**（占全套 85%） |
| `shaders::the_backend_shader_stays_small_enough_for_a_driver` | 1 | 21.8 ms |
| `shaders::every_shader_parses_and_validates` | 1 | 16.2 ms |
| `cloud_field::…carries_the_field_and_its_gradient` | 1 | 7.0 ms |
| 其余 | 63 | 全部 < 5 ms |
| **合计** | **67** | **349 ms** |

⇒ `px_verify/tests/*` 那三个文件（`dual` / `dual_field` / `dual_noise`）是**唯一有真实计算量**的部分，
而它们本来就是判据不是门 ⇒ 搬进 `px_probe` 成 `--bin dual / dual_field / dual_noise`（纯 CPU，不调
`require_gpu()`）。搬完默认测试链只剩 ~52 ms。`dual_noise` 里那条 `diagnose_the_noise_discrepancy`
只有 `println!`、没有断言 ⇒ **不算 check**（一个永远不会 ✗ 的 ✓ 是假绿），改成 `--diagnose` 才跑。

**当下这些门能抓到什么**（免得被当成摆设）：

| 门 | 抓到什么 |
|---|---|
| `crate_graph` | §10.1 的依赖方向：`px_render` 不许依赖 `px_sim`/`px_ops`（一句 `use` 就能把分层悄悄拆掉，编译照过） |
| `px_ops/tests/keys` | **键 = 内容**：注释/空格不改键、改值改键、版本/画布/输入进键、未知参数拒绝（键撞了 = 缓存静默给旧内容） |
| `px_protocol/tests/{cube,cubemap,octahedral,domain}` | 投影/UV 编码可逆、面序层序正确（这里错了整条烘焙链全歪，而画面只是"看起来怪"） |
| `px_ops/src/ops/gradient` + `cube_map_sampling` | 梯度取切向投影、平坦场梯度为零、**面边界不跳变**（§43/§46 打过的那类接缝 bug） |
| `px_render/src/art_cache` | 缓存失效规则：指纹 0 永不入缓存、冷计数复位、派生键要全部输入可缓存、同内容两路径算两条 |
| `px_render/tests/shaders` | 每个 shader 解析+校验、HLSL < 4000 行、**以及校验器必须能拒坏 shader / 未知 import 必须报错**（后两条是前两条可信的前提） |
| `snapshot` + `roundtrip` | 跨进程契约：协议形状一变 `protocol_hash` 就变，旧对端必须被握手拒掉 |

**近乎同义反复的**（便宜但只证明 serde 能用）：`snapshot_drives_the_protocol_hash`、
`f32_payload_round_trips_exactly`、`stream_round_trips_byte_identically`、
`identical_bundles_are_identical`、`the_review_set_is_twelve_tagged_normalized_cameras`。

⚠ **这一整套里没有一条能说"画面是对的"** —— 那是 §41 的仪器与 §46.3 的仲裁者的活。
GPU 浮点跨厂商不是逐位一样的，把它塞进 `cargo test` 只会得到时绿时红，然后大家学会忽略它。

---

## §55 「从不 present 的渲染循环」把 wgpu 的间接绘制校验池顶成线性泄漏（上游 issue 草稿）

§51.12 留下的那一环（**为什么偏偏有云才有**）在这里闭环，并且结论与当时的猜测**相反**：
间接绘制与云无关，有云只是**让 GPU 成为瓶颈**。下面 55.1–55.3 是本机实测，55.4 是可直接提交的 issue 文字。

### §55.1 哪一条绘制是间接的（实测，不是猜）

设备：RTX 3060 Laptop / DX12 / wgpu 29.0.4 / bevy 0.19.1，`WGPU_BACKEND=dx12`。
运行时打印 `GpuPreprocessingSupport.max_supported_mode = **Culling**`（判定见
`bevy_render-0.19.1/src/batching/gpu_preprocessing.rs:1360-1379`：`INDIRECT_FIRST_INSTANCE | IMMEDIATES`
两个 feature + storage 限制 + `COMPUTE_SHADERS` 全满足）⇒ `BinnedRenderPhaseBatchSets::MultidrawIndirect`
（`render_phase/mod.rs:1426-1434`）⇒ 网格走 `DrawMesh` 的间接分支。

临时仪器（数各 binned phase 的 `multidrawable_meshes`，以及排序 phase 里 `extra_index` 真的是
`IndirectParametersIndex` 的项数），480×300、单相机、每档一帧的稳定值：

| 场景 | Opaque3d | Opaque3dPrepass | Transparent3d（排序项 / 真间接） | 合计间接绘制 |
|---|---|---|---|---|
| `orbit-bare`（无云） | 1 | 1 | 1 / 1（大气壳） | **3** |
| `orbit-surface`（有云） | 1 | 1 | 2 / 2（大气壳 + 云壳） | **4** |
| `orbit-bare` + 原版 `StandardMaterial` + `AlphaMode::Blend` 球（对照） | 1 | 1 | 2 / 2 | **4** |

三条结论：

1. **有云只多一笔**（云壳是排序 phase 的一项，`Transparent3d.extra_index` 是 pub 字段，
   `bevy_core_pipeline-0.19.1/src/core_3d/mod.rs:377-388`）。"无云档几乎不产生"是**错的**：无云档也产生。
2. 对照实验（原版 Bevy 材质的透明球）与云**行为完全一样** ⇒ 走间接是**phase 决定的**，不是
   `CloudsMaterial` 特有。【task 要求的对照：只在临时调试里做，没动 `art/scene/orbit-*.toml`】
3. 真正的调用点是 `bevy_pbr-0.19.1/src/render/mesh.rs` 的 `DrawMesh`：`extra_index` 是
   `IndirectParametersIndex` 就走 `multi_draw_indexed_indirect[_count]`（indexed：4532/4541；
   non-indexed：4600/4609），否则走直接 `draw_indexed`（4481）。
   ⚠ 另一处每帧必发的间接绘制是 GPU clustering 的两个光栅 pass（count + populate，
   `bevy_pbr-0.19.1/src/cluster/gpu.rs:1071`，每视图每帧各一次）；把 `GlobalClusterSettings.gpu_clustering`
   置 `None` 后池子从 8 笔掉到 2 笔 ⇒ 它也是同一池子的来源之一，但同样**与云无关**（无云档也有）。

### §55.2 为什么"有云才有"：不是画了什么，是**谁跑得更快**

泄漏的池子就是 §51.12 那个固定 1 MiB 池。分配点在
`wgpu-core-29.0.4/src/indirect_validation/draw.rs:34`（`BUFFER_SIZE = 1_048_560`），
label 在 `draw.rs:197`（destination）/ `draw.rs:222`（metadata），**只在**
`impl Drop for DrawResources`（`draw.rs:771-779`）归还；而这份 `DrawResources` 挂在**一次提交**上
（`wgpu-core-29.0.4/src/device/queue.rs:287`、`queue.rs:414`）⇒ 只要提交没退役，这条提交里用到的
1 MiB 条目就不还。**每个含 ≥1 次间接绘制的 command buffer 至少钉住 1 笔 metadata + 1 笔 destination。**

同一场景、同一分辨率（2240×1400）、相机常开（`--fps`）下换"有没有背压"：

| 配置 | 校验池（分配笔数） |
|---|---|
| 无背压（= 修前）+ `run_loop(Duration::ZERO)` | 线性涨：T300 84 笔 → T720 860 笔（36 s） |
| 无背压 + `ScheduleRunnerPlugin::run_loop(1/60)` | **也线性涨**：T240 56 笔 → T720 880 笔（16 s） |
| 每 4 帧 `poll(wait_indefinitely())`（现状兜底） | **平**：T220 8 笔 → T300 16 笔 → 之后 27 s 一动不动（metadata 16 MiB + destination 16 MiB = 32 MiB）；另一次跑到 8 笔也平 |
| `orbit-bare`（无云）无背压 + ZERO | **平在 8 笔**（同一个 2240×1400、同一台机器） |

⇒ 无云与有云产生的间接绘制**是同一量级（3 vs 4 笔/帧）**，唯一的差别是**无云档 CPU 受限**（§41.1 那条 13.5 ms 地板、
利用率 25%）：CPU 提交速率 ≈ GPU 退役速率 ⇒ 队列不积；有云档 GPU 成为瓶颈（71.6 ms vs CPU ~30 ms）
⇒ CPU 无限跑在 GPU 前面 ⇒ 每个在飞提交钉住的 1 MiB × 2 逐帧累积。

这也解释了当初两条观察：**480×300 有云也是平的**（GPU 跟得上，队列不积）、
**不带 `--fps` 的普通出图从不泄漏**（`idle_between_jobs` 在 6 帧后关相机 ⇒ 根本没有后续帧）。
⚠ 教训：拿"不关相机的 1/60 循环"当对照会得到**假绿**（表里第 2 行才是把循环设成 1/60 而相机照旧的正确测法）。

### §55.3 修法三选（含对帧时间测量的污染）

| 修法 | 有没有用（实测/查证） | 对帧时间测量的污染 |
|---|---|---|
| **现状**：每 4 帧 `poll(wait_indefinitely())`（`px_render/src/main.rs` 的 `gpu_backpressure`，`GPU_IN_FLIGHT_FRAMES = 4`） | **有用**：上表第 3 行，池子平在 16 笔（= 在飞 ≤4 帧 × 含间接绘制的 command buffer 数）；§51.12 已验 2240×1400 + 云 64 s 平在 481 MiB | 每帧都等会把 CPU 编码与 GPU 渲染串起来（实测 81 → 71.6 ms，约 +13%），所以取 4 帧；**4 帧这一档实测帧时间没变差** |
| `ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(1.0/60.0))` | **没用**：它只把**主循环**压到 60 Hz，**不给在飞帧数上界**（表里第 2 行照样线性涨）。它之所以有时看着"有效"，是因为**不带 `--fps` 时相机 6 帧后就关了**，渲染根本没继续 | 会把 CPU 受限档的帧时间**钉在 16.67 ms**（§41.1 那类"量到地板"的假象），`--fps` 这个仪器本来就是为了去掉它 |
| Bevy / wgpu 有没有现成开关 | **没有**。`bevy_render-0.19.1/src/settings.rs:39-67` 的 `WgpuSettings` 只有 backends/features/limits/priority…，**没有** frame pacing / 在飞帧数上限；`desired_maximum_frame_latency` 只存在于**窗口**（`bevy_render/src/view/window/mod.rs:57`）⇒ 只对 surface 生效，离屏 `RenderTarget::Image` 用不上；`synchronous_pipeline_compilation` 只管管线编译。<br>wgpu 侧唯一的机制就是 `Device::poll`（即现状）。<br>⚠ 存在一个**escape hatch 但不要用**：`WGPU_VALIDATION_INDIRECT_CALL=0`（`wgpu-core-29.0.4/src/device/resource.rs:502-519`）会让 wgpu 干脆不装间接校验 ⇒ 池子消失；但 bevy 在 `settings.rs:130-139` 明确**在 DX12 上保留**这个 flag（"additional necessary logic during validation passes for the DX12 backend"），关掉等于撤掉一层必要的校验 | — |

**结论**：现状（每 4 帧 poll）是这三条里唯一既真能压住在飞帧数、又不污染帧时间读数的；
根因仍在 Bevy 无窗口循环没有在飞帧数上界，所以按 §55.4 上报。

### §55.4 可直接提交的 issue 文字（**只写在这里，没有提交**）

> **Title**: Headless / never-presenting Bevy app leaks GPU memory linearly: wgpu's indirect-draw
> validation pool is pinned per in-flight submission and nothing bounds in-flight submissions
>
> **Environment**
> - bevy 0.19.1 (`bevy_render` 0.19.1, `bevy_pbr` 0.19.1), wgpu 29.0.4, naga 29
> - Windows 11 22631, NVIDIA RTX 3060 Laptop (driver 32.0.15.9636), backend DX12 (`WGPU_BACKEND=dx12`)
> - Rendering to an offscreen `RenderTarget::Image` with `WindowPlugin { primary_window: None, exit_condition: DontExit }`,
>   `WinitPlugin` disabled, driven by `ScheduleRunnerPlugin::run_loop(Duration::ZERO)` — **the app never presents**.
>
> **Minimal reproduction**
> 1. Build a headless 3D app as above (no window, no surface, render into an image).
> 2. `run_loop(Duration::ZERO)` so the loop is not throttled.
> 3. Render any scene that contains at least one mesh (it does not need to be special — with the default
>    `GpuPreprocessingSupport::Culling` mode every binned-phase mesh is drawn with
>    `multi_draw_indexed_indirect`, and each transparent phase item likewise, see `bevy_pbr/src/render/mesh.rs:4532/4541/4600/4609`).
> 4. Let the CPU-side frame cost stay below the GPU frame cost (e.g. 2240×1400 with a heavy fragment shader):
>    now the CPU submits faster than the GPU retires.
> 5. Watch VRAM: it grows linearly (~150 MiB/s in our case) until `DXGI_ERROR_DEVICE_REMOVED` (`0x887A0005`).
>
> In our measurement the growing allocations are labelled
> `(wgpu internal) Indirect draw validation metadata buffer` and
> `(wgpu internal) Indirect draw validation destination buffer`, 1.00 MiB each, +2 per frame.
>
> **Expected**: in-flight frames are bounded (or at least the temporary resources of a frame are
> reclaimed independently of GPU progress), so VRAM stays flat while idle-rendering.
>
> **Actual**: VRAM grows without bound. Allocation site:
> `wgpu-core-29.0.4/src/indirect_validation/draw.rs:34` (`BUFFER_SIZE = 1_048_560`, a fixed pool),
> buffers are acquired per indirect draw (`draw.rs:810-873`) and are only returned by
> `impl Drop for DrawResources` (`draw.rs:771-779`). That `DrawResources` lives on the *submission*
> (`wgpu-core-29.0.4/src/device/queue.rs:287`, created at `queue.rs:414`), so every not-yet-retired
> submission keeps its own ≥1 metadata + ≥1 destination 1 MiB entry. With an unthrottled never-presenting
> loop the number of in-flight submissions is unbounded.
>
> **Evidence that this is not about *what* is drawn**: in the same scene, removing the heavy fragment
> shader (so the GPU is no longer the bottleneck) keeps the same number of indirect draws per frame
> (3 vs 4) yet VRAM stays flat; only the queue build-up changes. Two scene-independent sources of
> indirect draws exist on DX12: binned mesh phases (`bevy_pbr/src/render/mesh.rs`) and the GPU-clustering
> rasterization passes (`bevy_pbr/src/cluster/gpu.rs:1071`, count + populate, twice per view per frame).
>
> **Impact**: any Bevy app that renders without presenting — offline/headless renderers, frame servers,
> screenshot farms, CI image tests, `ScheduleRunnerPlugin`-driven tools — leaks GPU memory proportional
> to how far the CPU can run ahead of the GPU, and will eventually die with `0x887A0005`. Presenting
> apps are unaffected because present (vsync / `desired_maximum_frame_latency`) throttles the loop.
>
> **Workaround we use**: a render-world system that every 4 frames calls
> `RenderDevice::poll(PollType::wait_indefinitely())`, which bounds in-flight submissions
> (measured: the validation pool grows to 16 entries per kind / 32 MiB and then stays flat for 27 s;
> 2240×1400 + heavy clouds flat for 64 s, and `n=8` protocol runs complete). Polling every frame works
> too but serialises CPU encoding with GPU rendering (~+13% frame time), which we cannot afford in a
> frame-time measurement instrument.
> `ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(1.0 / 60.0))` does **not** fix it (measured
> growth is identical); `desired_maximum_frame_latency` is surface-only and therefore not applicable.



## §56 viewer（`--view` / `--show`）：契约、坑、以及怎么一眼判对错

**契约**（P10 之后）：

- 场景由**产物**决定：`px_render --view --scene <SCENE.pxart> --pcg-root target\pcg`。起始 `ablate`（体积 / 硬表面）也由产物里的 clouds part 决定 —— `orbit-proxy` / `orbit-surface` / `orbit-bound` 是硬表面，`orbit` 是体积云，`orbit-bare` 没有云。
- **窗口是常驻的**：只开一次。之后一律用 `--show --scene <…pxart>` **推**，不要每次重开。
- **推同一份路径是空操作**（`同一份产物…再推一次 ⇒ 不重建`）。要强制重建同一份产物，就让路径字符串不同（绝对 vs 相对），或者推另一份产物。
- `v` / `m` / `n`（体积 / 硬表面 / 法线）是**临时**交互覆盖，**下一次重建会被产物重写**。
- 场景路径取法：`target/pcg/scene/manifest.json` 里按图名（`scene`）+ 节点名取键 ⇒ `target/pcg/ab/<键前两位>/<键>.pxart`。

⚠ **历史坑（§52.3 同族，已修，留证）**：修之前 `main.rs` 把 `CloudView` 硬编码成 `Ablate::None`，而 **Bevy 里构建期插入的资源在系统首次 run 时算 changed**（配合 `auto_insert_apply_deferred = true`，同帧 `rebuild_scene` 刚 spawn 的云对它可见）⇒ **每次 `--view` 启动都被盖成体积云，产物说什么都没用**；而且推完场景后 `CloudView` 与材质不一致，按一下 `v` 就弹回体积云。症状："**产物写硬表面、窗口画体积云**"。

**两条可复用的判据**（这一轮踩出来的，不是检讨）：

1. **这套工具自己的说明书就是本文件**：开窗口 / 推场景 / 起服务这类操作，先读这里再动手 —— 上一轮有人没读 §27.1（"窗口常驻、一律用 `--show` 推，不要每次重开"），又多开了一个窗口，还把 `target/viewer-scene.json`（就是推的场景）挪走"保护"，结果窗口卡在"启动即被盖成体积云"。**正确动作本来只有一句话**。
2. **窗口里现在画的是哪一档，抓图比读源码快**：硬表面与体积云在同分辨率下截图字节数差别很大（2240×1400 上约 **2,681,841 B** vs **1,368,965 B**；480×300 硬表面有云约 **124,433 B** 而"丢云"约 **91,071 B**）。一分钟的实测胜过翻 `is_newer_than` / `auto_insert_apply_deferred` 的源码。
   ⚠ 但那对 2240×1400 的数**不能单独当 `ablate` 的判据**：两次抓图的**相机也不一样**（滚轮拉近过），
   差值里混着取景差 ⇒ 拿字节数当判据必须先**固定相机**；看图判档不受此限（硬表面 = 块状硬边、
   场梯度法线的黏土质感；体积云 = 软、灰、絮状）。

**修后最省事的那条判据**（比抓图还快）：窗口每次重建都打一行 `云视图 ← 产物：<档名>`
（`rebuild_scene`）；按键打 `云视图切到：<档名>` 与 `应用云视图覆盖：…（改了 N 个云壳）`；
另有一条自检 `云视图与材质一致：<档名>（码 N）`，不一致就 `⚠ 云视图与材质不一致…`。
⇒ **判档先看这几行日志**（都在 `target/viewer.log`），抓图当第二判据。
#### §56.1 修复在树里 ≠ 窗口里生效（这一轮为此白折腾两次）

1. **改完源码必须重编，重编必须先把占着 exe 的窗口停掉**。Windows 不允许覆盖正在运行的 exe（`failed to remove file … px_render.exe`），**但允许重命名运行中的 exe** ⇒ 想边跑边构建，就把它挪走再编（本轮就是这么做的：旧构建停在 `target/viewprobe/px_render.exe.old-build-buggy-*.bak`，`target/debug/px_render.exe` 换成新版）。**别把"文件时间戳没变"当成"没改到"** —— 先看是不是被运行中的进程锁着。
2. **判"窗口跑的是哪一版"，看日志里有没有新版才有的行**：`云视图 ← 产物：X` / `云视图与材质一致：X（码 N）`。旧版没有这两行 ⇒ 光看画面分不出版本，只能靠日志。这条又是"别把编译过当跑起来了"的同一个形状。
3. **多个窗口会互相干扰**：`target/viewer-scene.json` 是**共享**的推场景文件 ⇒ 一个脚本只要 cwd 不对，就会把场景推到**别人的窗口**里（本轮发生过）。推场景的脚本务必在自己的 cwd 里跑，或者认准心跳/窗口。
4. **看不到新版行为时，先确认二进制，再确认代码** —— 顺序反了就会去翻源码（这一轮有人为此翻过 `bevy_ecs` 的 `is_newer_than`/`auto_insert_apply_deferred`），而真因其实已经在树里修好了、只是没进 exe。

## §57 三路请求 + 结构化报告（v11；性能主路径改成"等条件 + 直读"）

> **权威版本在这里**：下面「三件必须在报告里读到口径的事」的第 1 条（app 逐帧序列双峰 ⇒ app 中位不是每帧成本），以及「丢窗/热身退休」「"稳定"是事件不是时间」两条 —— 都是本文件为权威版；`06-clouds.md` §51.18 只留一句 + 指针（同一批实测数见 §58.1）。

**为什么改（v10 → v11）**：v10 的测量协议全是**经验常数**，因为它量的是**代理量**：
「场景已稳定」被当成一段时间（丢 1 个 120 帧的窗、热身 4 s、`awaiting_pipelines ≤ 40 帧`），
「这一帧 GPU 花了多久」被当成 app 循环周期（还要靠 `K=4` 在飞帧 + `poll(wait_indefinitely)`
去逼近队列饱和）。v11 把这两个信号从 Bevy/wgpu **直读**，让窗/丢窗/热身/在飞帧那批魔数成批退休。
逐条信号能不能拿到、卡在哪一层，见 **§58**。

**三路**（每一路都回一份结构化 JSON 报告：`--report <路径>` 落盘，与回给调用方的那份**逐字节相同**）

| | 截图 `--shots` | **性能主路径 `--perf`** | 性能回退 `--perf --windows N --drop M` |
|---|---|---|---|
| 干什么 | 一串场景各出一张图就立刻切下一个 | **等到条件成立**（管线就绪 + 资产装完 + 重建后已渲染 K 帧）再**逐帧**采 `--frames` 帧（默认 60） | v10 语义，一字未改：每个场景先出一张图，再收 N 个 120 帧窗口（前面丢 M 个） |
| 怎么选 | 默认 | `--perf` 且**没给** `--windows` | 给了 `--windows` 就自动走它（老脚本不用改） |
| 要什么 | `--scene A --out a.png [--scene B --out b.png …]` | `--perf --frames 60 --scene A [--scene B] --report r.json` | `--perf --windows 4 --drop 1 --scene A --out a.png --report r.json` |
| 服务要求 | `--serve` | `--serve --fps`（不给就**当场拒收**） | `--serve --fps` |
| 出图吗 | 出 | **不出**（出图是一次 2240×1400 的回读，就在要量的东西旁边） | 出（判据图） |
| 报告里有什么 | 每张图：路径、字节、sha256、`placeholder_px`、`diff_vs_ref_grid`、`has_cloud` | 每档：`frames[]`（逐帧原始值）+ `min/p50/p90/p99/max`、`waits{}`（每次等待的实测耗时）、`gpu_ms{}`（时间戳逐帧序列 + 分位数）、`compare{}`（GPU vs app）；两档时 `Report.pair` 给**配对差** | 每档：`windows[]`、`dropped[]`、min/中位/n、`error_bar`、逐窗口 `gpu{sm_mhz,…}` |

**`--perf` 的默认动作是"改哪个场景就测哪个"**：`Scenes` 只给一档时自动配上参照档
（`-Reference`，默认 `orbit-bare`）⇒ **一次配对测量**。轮数默认 1；`-Rounds N` 是给误差棒用的。
5 档 × 3 轮那种大扫降级成显式开关 `-Sweep`。

**JSON schema（字段名是契约，改了要升 `SCHEMA_VERSION`）**

```
Report        { schema_version, protocol_hash, job, width, height, millis, shots[], perf[],
                pair?{measured,reference,app_delta_ms,app_mean_delta_ms,gpu_delta_ms?,rule} }
ShotReport    { scene, label, out, width, height, bytes, sha256,
                placeholder_px, bright_px, diff_vs_ref_grid,
                declared_clouds, has_cloud, verdict }
PerfReport    { scene, label, windows[], dropped[], min, median, n,
                error_bar{rule,value_ms}, gpu[],
                frames[], p50, p90, p99, max, key,
                waits?{pipelines_ms,assets_ms,settle_ms,sample_ms,total_ms},
                gpu_ms?{source,lag_frames,n,min,p50,p90,p99,max,frames[],poll_us},
                compare?{app_p50_ms,app_mean_ms,gpu_p50_ms,ratio,ratio_mean,note} }
GpuSample     { window, sm_mhz[min,max], power_w[…], util_pct[…], vram_mib_max, temp_c_max, samples }
Job            shots | perf{windows,drop} | stable{frames}      ← `Report.job` 是这三者之一
```

老两路（`shots` / `perf{windows,drop}`）**一个字段都没动**；新字段全部 `#[serde(default)]`，
所以旧报告照样能反序列化。`job = "stable"` 时 `windows`/`dropped` 为空、`frames`/分位数/`waits` 有值；
`job = "perf"` 时反过来（但 `key` 两路都给，`--changed` 用得上）。

**`has_cloud` 的口径**（防"丢云壳"）：`declared_clouds && placeholder_px == 0 &&
diff_vs_ref_grid ≥ 1% × 参考图总光通量`。参考图 = **这一批的第一张** ⇒ 调用方把无云档放第一个
（`orbit-bare`）。第一张自己没人可比，判据降一档（"不是占位、亮像素 > 0"），报告里 `verdict` 会写明。
⚠ 网格里存的是**逐格光通量和**，不是"亮像素计数"：云画在行星**上面**，亮像素总数几乎不动
（实测 1 249 627 → 1 257 629），用计数判不出来（差值只有阈值的 1/2）；换成光通量后
`orbit-surface` 对 `orbit-bare` 的差分是阈值的 **41 倍**。

**为什么逐窗口的卡读数不在主循环里采**（只对回退路成立）：`nvidia-smi` 一次要几十毫秒，
主循环里调会把那一帧顶高（正是这段代码要量的东西）。服务起来时开一条后台线程每 2 s 采一次，
主循环只读快照，再按窗口边界（每个窗口结束时的采样条数）把采样切给窗口。

**三件必须在报告里读到口径的事**（不然数会被读错）：

1. **app 逐帧序列是双峰的**。无窗口这条路里 `gpu_backpressure` 每 `GPU_IN_FLIGHT_FRAMES`（= 4）
   帧 `poll(wait_indefinitely)` 一次，那一帧特别长（实测 `orbit-soft`：3 帧 ~13 ms + 1 帧 ~134 ms）。
   所以 **app 的 p50/p99 不是"每帧成本"**，要看 `compare.app_mean_ms`（它才是 v10 那个
   "120 帧一窗的平均帧时间"的同类量）或直接看 `gpu_ms`。`Report.pair` 里 `app_delta_ms`（中位口径）
   和 `app_mean_delta_ms`（均值口径）都给出来，就是为了让人**看见前者错**：
   实测 `orbit-soft − orbit-bare` 是 **app 中位 −0.7 ms（符号都反了）／app 均值 +30.6 ms／GPU +36.5 ms**。
2. **GPU 读回是异步环形的，而且到达率不是 1 帧 1 条**。Bevy 每帧把时间戳 resolve + copy 进
   staging 再 `map_async`，回调在**后面某一帧**的 `begin_frame` 里取走
   （`bevy_render/src/diagnostic/internal.rs`）。主循环**一次都不等**；我们这边只是读
   `DiagnosticsStore`，实测 `gpu_ms.poll_us` ≈ **50 µs/帧**。固定开销是每帧约
   2 KiB 时间戳 + 5 KiB 管线统计的 resolve/copy。
   ⚠ `RenderDiagnosticsMutex` **只有一个槽**，一次 `begin_frame` 里完成多条只留最后一条 ⇒
   GPU 饱和时实测**每 4 帧才拿到 1 条**，所以 `gpu_ms.lag_frames` 是"凑够 N 条多等了几帧"、
   `gpu_ms.n` 是实际条数。
3. **分位数口径**：线性插值（numpy `linear` / R type 7）；`n` 是口径的一部分，报告里带着。
   `error_bar.value_ms = p99 − p50`（长尾）。跨轮误差棒由仪器侧算 = 各轮配对差的半极差，**不是标准差**。

**真实样例**（本机 RTX 3060 Laptop / DX12 / 2240×1400，v11，节选真实字段）：

```json
{
  "schema_version": 11, "protocol_hash": "ef7385f389914e10", "job": "stable",
  "millis": 6100,
  "shots": [],
  "perf": [{
    "label": "场景 orbit-soft｜…", "key": "69eec9d90fcb3a95",
    "frames": [12.6, 14.7, 132.7, 14.3, 12.7, 13.9, 134.8, "…（n=30）"],
    "min": 11.06, "p50": 13.50, "p90": 135.05, "p99": 143.74, "max": 145.02, "n": 30,
    "error_bar": { "rule": "新主路径：逐帧分位数（线性插值…）；value_ms = p99 − p50 = 长尾",
                   "value_ms": 130.25 },
    "waits": { "pipelines_ms": 107.0, "assets_ms": 12.5, "settle_ms": 168.1,
               "sample_ms": 5276.0, "total_ms": 5563.6 },
    "gpu_ms": { "source": "render/main_opaque_pass_3d/elapsed_gpu=0.379 + render/main_transparent_pass_3d/elapsed_gpu=36.299 + render/upscaling/elapsed_gpu=0.329 + …",
                "lag_frames": 87, "n": 30, "min": 36.66, "p50": 37.36, "p90": 37.63,
                "p99": 37.87, "max": 37.96, "poll_us": 50.8 },
    "compare": { "app_p50_ms": 13.50, "app_mean_ms": 45.30, "gpu_p50_ms": 37.36,
                 "ratio": 2.768, "ratio_mean": 0.819 }
  }, {
    "label": "场景 orbit-bare｜…",
    "frames": [14.3, 14.9, 13.9, "…（n=30）"], "p50": 15.04, "p99": 17.00,
    "gpu_ms": { "p50": 0.61, "source": "…main_transparent_pass_3d/elapsed_gpu=0.071…" },
    "compare": { "app_p50_ms": 15.04, "app_mean_ms": 14.75, "gpu_p50_ms": 0.61,
                 "ratio": 0.041, "ratio_mean": 0.042 }
  }],
  "pair": { "measured": "场景 orbit-soft｜…", "reference": "场景 orbit-bare｜…",
            "app_delta_ms": -1.54, "app_mean_delta_ms": 30.55, "gpu_delta_ms": 36.49,
            "rule": "配对差 = perf[0] − perf[1]（被测档 − 参照档）…" }
}
```

**这一版退休掉的魔数**（对照 v10）：丢窗 `--drop`（新路径不存在）、热身 4 s、
`FRAME_PROBE_WINDOW = 120`（折窗）、`PIPELINE_POLL_INTERVAL = 20`（重建后先等一个轮询周期）、
`PIPELINE_SETTLE_FRAMES = 40`（等"看见管线重编入队"）、`FRAMES_AFTER_JOB = 6`（出图前那几帧）。
换成的直读信号：`PipelineCache` 的**连续 2 个渲染帧没有新排队/在编的管线**、
`AssetServer` 的 `is_loaded_with_dependencies`、渲染世界的帧计数、以及 GPU 时间戳。

**「稳定」现在是事件，不是时间**（管线就绪 + 8 渲染帧），不再靠丢窗/热身 —— 这条与上一条同源，是 v11 的口径本体；`06-clouds.md` §51.18 只留指针。
**还必须经验化的**：`GPU_IN_FLIGHT_FRAMES = 4`（它决定 app 帧序列的双峰形状，也决定 GPU 读回
到达率 —— 它不在协议里，但它是"app 侧任何数"的形状来源）；`STABLE_SETTLE_FRAMES = 8`
（重建后前几帧还在付新管线的首次绑定/上传）；`--frames` 的默认 60（分位数要多少样本）。
逐条见 **§58**。

**`px_render` 的 CLI**：`--shots`（默认）/ `--perf` / `--frames N` / `--windows N` / `--drop N`
/ `--report <路径>`。给了 `--windows` 就走 v10 回退路。

**自转没了（P31，仍然成立）**：以前云/行星按**逻辑帧时间**推进自转（`spin_bodies`，
`time.delta_secs()*0.12`）。两个害处：① 逻辑帧率与渲染帧率不一致时每帧步长都不同 ⇒ 一顿一顿；
② 每帧内容都在变 ⇒ 同一场景连续帧不逐字节相同，截图不可复现、帧时间也没法比。**功能已删**；
场景产物里的 `spin` 现在只当**静态相位**用（产物格式没变，`spin` 的含义从"每秒转多少的起点"
收窄成"起始相位"）。判据：连推两次同一档，图**逐字节相同**（`orbit-soft` 两次都是
`871780fe46aaf0fa…` / 1 425 061 B；v11 的截图回归里 `orbit-soft` 仍是 1 425 061 B）。

**`-Changed`（仪器侧）**：拿上一次的产物键（`target/<Tag>-keys.json`，**按 Tag 分开存**）与现在的
清单做 diff，报出"哪几档真的变了"。⚠ 反直觉但必须记住：**改 shader 会让所有钉它的云的场景键全变**
⇒ 那时它会指向全部云档，**这不代表你要全测** —— 该由人指定"我改的是哪一档"，
工具只负责算差值与给历史。
⚠ 两种"键"别混：脚本的 `-Changed` 用的是 `manifest.json` 里那个 **64 位 CAS 键**；
报告里的 `PerfReport.key` 是服务端算的 **`.pxart` 载荷指纹**（`art_cache::fingerprint_of`，
8 字节十六进制）。两者都是"内容变了才变"，但不是同一个数。

---

## §58 直读信号的实测清单（每条：拿法 → 本机可用? → 证据）

口径：Bevy **0.19.1**、wgpu **29.0.4**、本机 DX12、RTX 3060 Laptop、`cargo build`（debug）。
源码行号指的是 `C:\Users\17124\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\<crate>\…`。

| 信号 | 拿法（API + 文件行号） | 本机可用? | 实测证据 |
|---|---|---|---|
| 管线全部就绪 | `PipelineCache::pipelines()` 扫 `CachedPipelineState::{Queued,Creating,Ok,Err}`（`bevy_render/src/render_resource/pipeline_cache.rs` **221 / 46-55**）；`waiting_pipelines()`（**226**） | ✅ | 服务启动打 `首个渲染管线入队：当前 44 条，待编译 44 条` → 随后 `渲染管线全部就绪：共 44 条，失败 0 条` |
| 「**有新管线被排队**」本身 | ❌ **没有公开信号**：`new_pipelines` 是私有字段（**210**），`queue_render_pipeline` 只往里推（**393-402**），要等 `process_queue`（**640-654**）搬进 `pipelines` 才对外可见；`get_render_pipeline_state` 对未处理的**报 Queued**（**276-281**） | ⚠️ 只能**间接**、且晚 ≤1 渲染帧：看 `pipelines().count()` 增长 + 出现 Queued/Creating | 把轮询从每 20 帧改成**每帧**（并排到 `RenderSystems::Cleanup`，即 `process_pipeline_queue_system` 之后）后，**立刻**抓到一次真的瞬态：`CachedPipelineState::Err(ShaderNotLoaded)`（老 20 帧轮询看不见）⇒ 见下一行 |
| 「可重试错误」vs「编译失败」 | `ShaderCacheError::{ShaderNotLoaded,ShaderImportNotYetAvailable}` 会被 `process_pipeline` 下一帧打回 Queued（**685-690**） | ✅（必须区分） | 未区分前：`管线编译失败｜environment_pipeline｜…shader could not be loaded` 且 `pending` 永远 > 0 ⇒ 永远不就绪。区分后恢复正常 |
| 调度位置（为什么"连续 2 帧干净"够） | Render 调度链 `Specialize → … → Render → Cleanup`（`bevy_render/src/lib.rs` **301-316**）；`process_pipeline_queue_system` 在 `RenderSystems::Render`（**430**） | ✅ | 把 `watch_pipelines` 放进 `Cleanup` 后，重建后的流水线在**同一帧**被看见 |
| 资产装完 | `AssetServer::is_loaded_with_dependencies`（`bevy_asset/src/server/mod.rs` **1331**）/ `get_load_states`（**1221**）/ `load_state`（**1291**）；`LoadState` **2248**、`DependencyLoadState` **2283**、`RecursiveDependencyLoadState` **2318**；`AssetEvent::LoadedWithDependencies` **2236** | ✅ 但**对本仓库近乎空转** | 服务日志：`等资产：4 个句柄全部 Loaded（其中「非 Loaded」0 个），等了 12.5 ms` + 逐句柄 `Loaded / 依赖 Loaded / 递归 Loaded`。原因是**场景内容全走同步 `Assets::add`**（`planet.rs` `images.add`/`meshes.add`），根本不在 `AssetServer` 上挂号；只有槽 shader（`slots::activate` 的 `AssetServer::add`）与 shader 库（`shaders.rs` 的 `assets.load`）是它管得到的。⚠ 所以**"整个场景装完了"没有确切信号**，这一条只证明"这 4 个句柄装完了" |
| GPU 时间戳：设备能力 | `RenderDevice::features()`（wgpu `Features::TIMESTAMP_QUERY` / `TIMESTAMP_QUERY_INSIDE_ENCODERS` / `TIMESTAMP_QUERY_INSIDE_PASSES` / `PIPELINE_STATISTICS_QUERY`，`wgpu-types-29.0.4/src/features.rs` **61 / 718 / 740 / 1620**） | ✅ 时间戳全开；❌ 管线统计**不支持** | 服务启动打：`GPU 时间戳能力：TIMESTAMP_QUERY=true INSIDE_ENCODERS=true INSIDE_PASSES=true PIPELINE_STATISTICS=false｜时间戳周期 1 ns`（设备 `NVIDIA GeForce RTX 3060 Laptop GPU`，backend `Dx12`） |
| GPU 时间戳：Bevy 有没有请求/暴露 | Bevy **请求**：默认 `WgpuSettingsPriority::Functionality` ⇒ `features = adapter.features()`（`bevy_render/src/renderer/mod.rs` **300-301**，`settings.rs` **151** 只额外加 `TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES`）。Bevy **暴露**的现成实现是 `RenderDiagnosticsPlugin`（`bevy_render/src/diagnostic/mod.rs` **64**），但它**默认不加** —— `RenderPlugin` 只在 `#[cfg(feature = "tracing-tracy")]` 下加（`bevy_render/src/lib.rs` **381-382**） | ✅（显式 `add_plugins(RenderDiagnosticsPlugin)` 即可） | 加进 `serve()` 后，`DiagnosticsStore` 里出现 `render/**/elapsed_gpu`，实打实给出每帧 GPU 毫秒 |
| 每帧 GPU 毫秒（直读） | `DiagnosticsStore` 里 `render/<span>/elapsed_gpu`（由 `RecordDiagnostics::pass_span`/`time_span` 写：`main_opaque_pass_3d`、`main_transparent_pass_3d`、`prepass`、mip、`tonemapping`、`upscaling`）。`internal.rs` **244**（建 query set）、**480-509**（resolve+copy）、**558-565**（`map_async`）、**569-576**（`begin_frame` 里轮询回调）、**723-743**（`sync_diagnostics`） | ✅ | `orbit-soft`：`main_transparent_pass_3d=36.299 ms`（云就在这条 pass）＋ `main_opaque_pass_3d=0.379` ＋ `upscaling=0.329` ⇒ GPU p50 **37.36 ms**（n=30，min 36.66 / max 37.96）。`orbit-bare`：同一条 pass **0.071 ms** ⇒ GPU p50 **0.61 ms** |
| 读回的代价（不许同步等） | 同一份实现就是**异步环形**：`submitted_frames` 队列 + `map_async` 回调（**30-38 / 84-98 / 113-144 / 569-576**），主循环只在 `PreUpdate` 取走 | ✅ | 我们读 `DiagnosticsStore` 的实测代价 `poll_us ≈ 50 µs/帧`；固定开销 = 每帧 ~2 KiB 时间戳 + ~5 KiB 统计的 resolve/copy。⚠ 到达率不足：见 §57 第 2 条（GPU 饱和时 1 条 / 4 帧，多了 87 帧才凑够 30 条） |
| 重建后"已渲染 K 帧" | 渲染世界自己的帧计数：`Render` 调度加一条自增系统（本仓库 `RenderFrames` + `count_render_frames`）。Bevy 的 `FrameCount` 是**主世界**的帧号，靠 `extract_frame_count`（`bevy_render/src/globals.rs` **32-34**）抽进渲染世界 —— 它不能区分"抽了几帧"与"渲了几帧" | ✅ | 每一步报告 `waits.settle_ms`：`orbit-soft` **168 ms**、`orbit-bare` **58 ms**（8 个渲染帧，含重建那一帧的排队） |
| 其它"稳定"信号 | 提取/渲染世界帧计数 = 上一条；场景实体 spawn 完成 = **没有**这样的标志（`commands.spawn` 是延迟命令，只能靠"渲染帧数"间接保证）；`AssetEvent::LoadedWithDependencies` 见上（对本仓库空转） | ⚠️ | 只有"渲染帧计数"这一条能真的用 |

**拿不到 / 只能 CPU 侧的，明说**：

* **「有新管线被排队」这个事件**：Bevy 没有公开它。能观测的只有"缓存里出现了新的待编译条目"，
  比真排队晚 ≤1 渲染帧（`process_queue` 那一跳）。想要严格意义的事件，得给 `PipelineCache` 打补丁。
* **管线统计（顶点/片元调用数）**：Bevy 会请求，但本机 DX12 适配器**不支持**
  （`PIPELINE_STATISTICS=false`）⇒ `render/**/vertex_shader_invocations` 之类一条都不会有。
* **"整个场景的资产都装完了"**：场景内容是同步 `Assets::add` 的，`AssetServer` 的装载态看不见它们。
  只有槽 shader / shader 库在它的管辖内。
* **GPU 每帧一条时间戳**：拿得到，但**不是每帧一条**（Bevy 的 `RenderDiagnosticsMutex` 只有一个槽，
  一次 poll 放出多条时只留最后一条）。要多条只能多采帧。


### §58.1 验收实测（本机 RTX 3060 Laptop / DX12 / 2240×1400 / debug exe）

> 下面的「配对差」表是权威副本：`06-clouds.md` §51.18 保留同一批数并指向本节（口径铁律的正文在 §57）。

**5 档 × 3 轮**（`-Phase stable -Sweep -Rounds 3 -Frames 60`）：**整轮墙钟 157.2 s**，
对照 v10 的同一批（5 档 × 3 轮 × 5 窗 × 120 帧 + 丢窗）**386 s** ⇒ 2.46×。

逐帧分位数（被测档 3 轮合起来，n=180）：

| 档 | p50 | p90 | p99 | max | 均值 | app 长尾 p99−p50 | GPU p50 | GPU p99 | GPU 长尾 |
|---|---|---|---|---|---|---|---|---|---|
| orbit-bare（地板） | 14.76 | 15.67 | 16.58 | 16.69 | 14.81 | 1.82 | 0.64 | 0.84 | 0.19 |
| orbit | 13.66 | 140.48 | 158.28 | 161.38 | 45.41 | 144.61 | 39.57 | 42.82 | 3.25 |
| orbit-proxy | 13.92 | 115.38 | 130.29 | 171.00 | 38.83 | 116.38 | 32.37 | 38.76 | 6.38 |
| orbit-soft | 13.26 | 152.38 | 162.59 | 171.64 | 46.97 | 149.33 | 41.13 | 44.26 | 3.13 |
| orbit-surface | 12.74 | 202.02 | 224.88 | 277.15 | 59.68 | 212.13 | 53.39 | 73.59 | 20.20 |

每次等待的实测耗时（被测档那一份，三次的范围）：等管线 **34–1021 ms**（只有每次会话的第一个
请求贵，其余 52–64 ms）、等资产 **12–108 ms**、等稳定 **58–233 ms**、采样 **896–15049 ms**、
整个请求 **1.1–16.5 s**。

**配对差（被测 − orbit-bare，3 轮中位 ± 半极差）** —— 这一栏就是"代理量错在哪"：

| 档 | app **中位**差 | app **均值**差 | GPU p50 差 |
|---|---|---|---|
| orbit | **−1.33 ± 1.27** | 30.25 ± 1.27 | 38.68 ± 2.29 |
| orbit-proxy | **−0.70 ± 1.47** | 23.77 ± 1.47 | 32.15 ± 1.34 |
| orbit-soft | **−1.53 ± 1.76** | 32.82 ± 1.76 | 41.10 ± 1.86 |
| orbit-surface | **−1.83 ± 3.05** | 45.37 ± 3.05 | 52.86 ± 3.04 |

⇒ 拿 app 循环的**中位**当"每帧成本"，配对差的**符号都是反的**（云档看起来比无云档还快）；
拿 **GPU 时间戳**，配对差是 +32 ~ +53 ms 且跨轮 ±1.3~3.0 ms。这是本次最硬的一条结论。

**默认路径（单档 + 参照档，轮数 1）**：`orbit-proxy` 一次 **9.2–10.7 s**（含参照档与两条请求），
对照 v10 的 ~29 s/档。报告里的 `pair` 字段给 `app_delta_ms`（中位，−1.2）、
`app_mean_delta_ms`（+24.4）、`gpu_delta_ms`（+32.4）。

**魔数退休清单**：`--drop`、热身 4 s、`FRAME_PROBE_WINDOW=120`、`PIPELINE_POLL_INTERVAL=20`、
`PIPELINE_SETTLE_FRAMES=40`、`FRAMES_AFTER_JOB=6` 在新主路径上**全部不再参与**。
还留着的经验量：`GPU_IN_FLIGHT_FRAMES=4`（决定 app 序列的双峰形状与 GPU 读回到达率）、
`STABLE_SETTLE_FRAMES=8`（重建后前几帧还在付首次绑定/上传）、`--frames` 默认 60（分位数样本数）。
回退路（`--windows/--drop`）原样保留，`-Phase shot` 的 `has_cloud` 回归本次全绿
（`orbit`/`orbit-surface`/`orbit-proxy`/`orbit-soft` 四档都有云，`orbit-soft` 仍是 1 425 061 B）。