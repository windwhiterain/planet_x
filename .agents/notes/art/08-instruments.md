# 08 仪器：px_render 怎么跑，改完怎么验

这篇讲渲染器本身的运行形态（离屏 / 常驻服务 / viewer 窗口）以及验证它的那几台仪器：
单帧时间怎么量、shader 热重载要不要重启、探针在哪跑、空白画面怎么定性。

---

## §12 px_render 的形状

`px_render`（Bevy 0.19.1）读 `.pxstream` / `.pxart` → **离屏渲染** → PNG。

```
cargo run -p px_render -- --stream target/world.pxstream --round 200 --out target/shot.png
```

- **离屏 render-to-image**（`Image::new_target_texture` + `RenderTarget::Image` + `Screenshot::image`），
  配 `WindowPlugin { primary_window: None, .. }` + `disable::<WinitPlugin>()` + `ScheduleRunnerPlugin`。
  这条路的性质正是美术迭代要的：**无窗口、无 DPI 缩放、`--width/--height` 就是像素数**。
- 协议指纹不匹配的流会被**拒绝**（显式比对 `ProtocolId.protocol_hash`）。
- 同一轮跑两次**逐字节相同**（960×640，第 60 / 200 / 240 轮都验过）。
- 场景高度按**本流自身的峰值归一化**（相对比例，不是绝对量）。

**管线是异步编译的，所以出图必须等就绪门。** 门读的是渲染世界的 `PipelineCache`：

```rust
app.get_sub_app_mut(RenderApp).unwrap()
   .insert_resource(RenderReady(flag.clone()))   // Arc<AtomicU8>，两个世界各插一份
   .add_systems(Render, watch_pipelines);
```

⚠️ 就绪条件必须是 **`total > 0 && pending == 0`**。只写 `pending == 0` 会在**第 3 帧**就"就绪"
（队列为空天然成立），照样拍到空白。修正后：41 条管线、首帧 20 条待编译 ⇒ **第 167 帧就绪**、
启动到出图 **7.5 s**（固定 600 帧预热是 14.7 s，而且随时可能拍到空白）。

⚠️ **不要用固定帧数预热**：20 帧、120 帧都是空白，400 帧才有内容，而这个数不稳定。

### §12.2 「画面空白」的四种成因与定性手段

四种失败症状都是「只有背景色」，而手段各不相同：

| 手段 | 一眼看出什么 |
|---|---|
| **把清屏色改成洋红** | 区分「什么都没画」与「画了但很暗」—— 一秒钟的事，先做这个 |
| **打印 `ViewVisibility`** | 剔除还是没画（柱子堆到 12 米高跑出画面会被正确剔除，别误判成「全被剔除了」） |
| **打印管线队列**（总数 + 待编译数） | 「还没编译完」还是「编译失败」 |
| **复刻官方最小例子** | 分清「我的场景错」与「环境/后端错」 |

前三条是 `px_render` 的常驻诊断（`diagnose` 打印可见性；管线状态在就绪时打印一次）。

---

## §13 常驻渲染服务

```
px_render --serve [--port N] [--width W] [--height H]   # 常驻服务，启动时预热管线
px_render --planet … --mesh … --sheet target/sheet.png  # 客户端：连上服务要一张图
```

- **wire = `px_protocol` 的 Frame 流**（`Request` / `Response` / `Refused` 三种帧），走 TCP 127.0.0.1。
- **握手**：`ProtocolId`（schema 版本 + 协议指纹 + git rev）双向校验，不匹配直接 `Refused` 并报出双方指纹。
- **租约文件** `target/render-server.json`（pid / 端口 / 指纹 / rev / exe）；**删掉租约即停服务**
  （服务每 120 帧自查，实测 4 秒内自行退出）。
- 任意 Rust 模块可直接调 `px_protocol::client::request(Request { .. })` 要图，不必经过 CLI。

| 路径 | 出图耗时 |
|---|---|
| 一次性进程（§12） | **7.5 s** |
| 常驻服务第一次请求（含服务预热） | 2.9 s |
| **常驻服务后续请求** | **0.33 s** |

服务端出的图与一次性路径**逐字节一致**。

**三条结构规则**（每一条都对应一类「看起来对、其实错」）：

- **实体分三类**：`StageCamera` / `Stage`（灯）/ `WorldPart`。重建世界时**只动 `WorldPart`** ——
  把相机一起 despawn 会让之后没有任何相机在渲染，四张图全是背景色。
- ⚠️ **换尺寸时别弄丢灯**：resize 分支 despawn 全部 `Stage` 却只重建相机会让环境光与方向光消失。
  **「改尺寸再改回来必须回到原样」是一条免费的强断言**，SHA256 一比就知道（325212 vs 326452 就是这么抓到的）。
- ⚠️ **默认不自动拉起服务**：客户端发现没有服务时顺手 spawn 一个 ⇒ 服务成了客户端的**子进程** ⇒
  任何等整棵进程树结束的调用方（PowerShell 作业、`cargo test`、DSH 本身）会一直挂下去，
  而**服务端日志显示一切正常、图也出了**。现在默认「连不上就快速失败并打印怎么起服务」（实测 2.37 s），
  自动拉起降级为显式 `--autostart`。

### §13.2 使用须知（会咬人的两条）

- ⚠️ **服务运行时 `target\debug\px_render.exe` 被占用 ⇒ `cargo build` 报「拒绝访问」。**
  改代码前先停服务：删掉 `target/render-server.json`（4 秒内自查退出）或 `Stop-Process`。
- 请求里的路径是**相对路径，由服务端 cwd 解析**；客户端与服务端 cwd 不同时要用绝对路径。

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
px_render --view --planet <H.pxart> --mesh <M.pxart> [--clouds <C.pxart>] --palette rocky
px_render --show --planet <…> [--clouds <…>] --palette rocky [--shot]      # 推新场景
```

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
     -ArgumentList @("--view","--planet",$P,"--mesh",$M,"--clouds",$C,"--palette","rocky") `
     -RedirectStandardOutput target/viewer.log -RedirectStandardError target/viewer.err -PassThru
```

`Start-Process` 不带 `-Wait` 立刻返回，窗口留在用户桌面上，agent 继续干活。
判据（`云层：…`、`渲染管线全部就绪…失败 0 条`）都在 `target/viewer.log` 里。

⚠️ **命令行给了 `--planet` 就以命令行为准**，没给才回退到 `target/viewer-scene.json`
（否则填了的参数会被上一个 session 留下的请求文件**静默覆盖**）。

⚠️ 未解释的观察：窗口里的**整体曝光比 `--serve` 出图暗**（实测均色 `{41,47,53}` → `{32,33,35}`，
蓝通道掉得最多），两者 `AmbientLight` 与大气壳参数一致。**没查出来**。

⚠️ **交接文档的"能跑什么"清单不等于用法说明**：清单里出现的每条命令，都要回到它自己的那一节读用法。

---

## §41 怎么量单帧时间（`--fps`）

先去掉两道帧率上限，否则量到的是上限不是成本：`--serve` 的 `run_loop(1/60)` 会**睡觉**凑 60 Hz，
窗口默认 **vsync**。

```
px_render --serve --fps --width W --height H      # 服务端不再限速
px_render --view  --fps  --planet … --clouds …    # 窗口 AutoNoVsync
```

`--fps` 每 **120 帧**打一行平均帧时间（`FRAME_PROBE_WINDOW`）。
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
```

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

---

## §50 资源缓存：渲染器不重造它已经造过的东西

服务端原来每请求从零造一遍：解码场、建 46 万条边的 HashMap 审计、逐 texel 上色 + 整条 mip 链、
f16 立方图、八面体壳 + 位移 + 网格法线 + 焊法线。这些活的输入只有两样 —— **产物内容**和
**渲染参数** —— 所以缓存键就是这两样的拼接。实体（几十个）照旧每请求重建。

实现在 `px_render/src/art_cache.rs`，五张表：

| 表 | 键 |
|---|---|
| `field` | `(规范路径, 清单里的载荷指纹)` |
| `mesh`（`--mesh` 那条路：载入 + 焊法线） | 同上 |
| `sphere`（程序化球面网格） | `(场, palette, sea_level, displace, radius)` |
| `texture`（逐 texel 上色 + mip 链） | `(场, palette, sea_level)` |
| `coverage`（云覆盖度 f16 立方图） | `(覆盖度, 三个梯度)` 四个指纹 |

- `palette` 决定 `flat_sea`（海平面压平），`sea_level` / `displace` / `radius` 直接乘进顶点位置
  ⇒ 一个都不能漏；而 `texture` **不许**放 `displace` / `radius`（它们只动顶点、不动贴图）。
  **多放一个参数只是浪费内存，少放一个就是「同一个键、不同内容」**（§17.1、§25.2）。
- ⚠️ **指纹为 0 的旧产物一律不缓存**（每次现造）：键里少了「内容」这一维时，命中就等于认错了东西。
  这条由 `CacheKey::cacheable` 从类型上兜住 —— 派生键只要有一个输入不可缓存，整条就不可缓存。
- **淘汰按冷落次数，不是 LRU**：`MISS_LIMIT = 3`，每次 `sweep()` 给没被用到的条目 `misses += 1`、
  命中清零、超过就丢。服务是长期进程，但相邻请求常在 A/B 之间来回（两个色板、两个消光档），
  LRU 会挤掉还要用的那条。
- ⚠️ **只在场景真搭起来之后才结算**（`sweep()`）：一次被 `Refused` 的请求不该让任何条目变冷。

**只读清单帧**：`px_protocol::art::read_manifest` 读 64 KB 前缀就能拿到清单（载荷指纹 + 相机表），
不再为了问一句「这份产物变了吗」把 8 MB 的场整个读进来。前缀里解不出清单帧就回落整读。

**命中时把造它那一次的审计原样重放**（`replay()`，前缀一行「缓存命中，下面是造它那一次记下的审计」）。
为此 `load_mesh` / `surface_textures` / `octahedral_mesh` 的 `println!` 都变成了返回值。
**缓存可以省重算，不该省仪器。**

**实测**（同一场景连发 4 次 `--sheet`）：冷 **1867 ms** → 热 **1526 / 1530 / 1518 ms**，
热/热抖动 ±6 ms，**四张图 SHA256 全同**。⇒ 缓存省下的是约 **18%**，剩下 82% 是渲染本身
（12 个视口 × 体积云 56 步）—— 真正的大头在 shader 里，用 §41 的仪器去量。

viewer 与 serve 共用同一份 `spawn_planet` 与同一份缓存；`poll_field` 的判据也从 mtime
换成了**载荷指纹**（mtime 只当「该去看一眼」的闹钟）：重新烘一份内容一模一样的场不该让窗口重建。
