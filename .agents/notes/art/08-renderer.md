# 渲染器怎么跑

px_render 长什么样：离屏还是常驻、实体分几类、那张图是怎么出来的、空白画面怎么定性，
以及它怎么记住自己造过的东西。**改完怎么验**在 `09-instruments.md`。

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

## §52 渲染内容由产物决定（场景产物）

### §52.1 病根：接口是一串旗标，不是一个产物

§18 写着「产物格式 = `ArtBundle`，与渲染器之间唯一的接口」，但实现里接口是一串命令行旗标：
`--planet --mesh --clouds --cloud-slope --palette --displace --sea --radius --spin --rings --atmo --cloud`。
后果两条，第二条更毒：

1. **哈希路径静默过期**。内容寻址的路径一改就变；`tools/frame-probe.ps1` 把六个默认路径写死在参数里，CAS 一清就全死，而它又吞掉客户端退出码 ⇒ 服务继续画预热场景、照样每 120 帧打一行帧时间（§51.1）。
2. **成员配错时形状校验照样通过**。拿 v1 的 `mixed` 配 v2 的 `slope_x`：宽高都是 `256²×6`，`clouds.rs` 只校验 `projection == CubeMap` 与尺寸一致 ⇒ **校验通过、法线是错的、不报错**。这正是 §43 花大力气建 arbiter 防的那类故障，却被 CLI 接口重新放开了一个口子。

### §52.2 形状：一般场景描述，格式本身不认识"行星/云/大气"

一个 part 只有五样东西：`id`、`kind`（装配器名）、`shader`（绑定槽）、`members`（按角色引用的产物）、`params`（按名字取的参数）。

```json
{ "frame": "scene", "schema": 1, "name": "orbit", "ambient": 80.0, "cameras": [ … ],
  "parts": [
    { "id": "planet", "kind": "planet", "shader": "surface",
      "members": { "height": {"graph":"planet","node":"height","key":"69ee…"},
                   "mesh":   {"graph":"planet","node":"surface","key":"f5d6…"} },
      "params":  { "palette": "rocky", "displace": 0.06, "sea_level": 0.52, … } },
    { "id": "clouds", "kind": "clouds", "shader": "clouds",
      "members": { "shader":  {"graph":"shaders","node":"clouds","key":"0f83…"},
                   "field":   {…}, "slope_x": {…}, "slope_y": {…}, "slope_z": {…} },
      "params":  { "inner": 1.01, "outer": 1.06, "extinction": 900.0, "coverage": 0.35, … } },
    { "id": "atmosphere", "kind": "atmosphere", "shader": "atmosphere",
      "members": { "shader": {…} },
      "params":  { "inner": 1.0, "outer": 1.14, "density": 0.3, "tint": [0.44,0.64,0.98] } } ] }
```

- 成员 = **图名 + 节点名 + 内容键**（§17.1「键 = 内容」）。名字给人看与报错，键给渲染器取产物。
- 加物体（环、卫星、空间站、舰）**不动格式**，只加一个装配器 + 一个 shader。
- 缺成员、缺参数、不认识的 kind、不认识的槽 —— 一律 `Err`，并且报出"这份有哪些可选项"。
- 代价：参数变成字符串键、类型检查挪到运行期。这与 `AssetManifest.params` 和 `art/<图>/<节点>.toml` 是同一套既有做法，不是新发明的负担。

### §52.3 shader 也是资产，走同一个缓存模式

- `kind = Shader` 的产物：WGSL 作为 **U8 blob**，**键 = WGSL 的字节 ‖ include 闭包指纹**（改一个字、或者改它 `#import` 到的任一模块，都换一个键），由 `px_graphs --bin shaders` 从 `art/shaders/*.wgsl` 烘出，进 `target/pcg/shaders/manifest.json`。
- **include 闭包为什么进键**（2026-09 修，`02-pcg.md` §17.1）：入口里的 `#import planet_x::*` 由 **naga_oil 在运行期**组装，模块真本住 `px_render/assets/shaders/*.wgsl` ⇒ 只哈希入口文本时，改一个 include **什么都不动**（键、清单、场景键、槽版本全不动），而画出来的东西变了。规则只一份实现：叶子 crate **`px_shader`**（`closure` / `modules_fingerprint` / `#define_import_path` 解析），烘图侧、`px_render::shaders`、`reflect`、离线门都用它；只收**可达**模块，外部符号（`bevy_pbr::…`）只记名字（它们的实现归 `SHADER_VERSION` 手动那一档，§19.1）。
- **装载时对账**：产物清单参数里记着闭包指纹（`closure_hi` / `closure_lo`）与规模（`closure_modules` / `closure_externals`）；`preload_shaders` 拿它跟**盘上现在**的闭包比 —— 不一致、或者老产物压根没记过 ⇒ **当场拒**（`scene::closure_check`，三条单测钉住三种情形），并给出重烘配方 `cargo run -p px_graphs --bin shaders` → 逐个 `--bin scene <名>`。为什么必须有这条：改了库、没重烘时场景指的还是老产物，而组装用的是新库 —— 那幅图**既不是老那一版、也不是新那一版**，而键 / 场景键 / 槽版本全没动（§52.3 那个"云静默消失"的同族故障：所有门都绿）。
  实测（2026-09-16，`.worktrees/shader-include`）：`orbit-bare` 基线出图 960×640 / 300012 字节；
  给 `light.wgsl` 尾巴加一行注释（不重烘）⇒ 客户端退出码 1、不出图，拒词是
  「产物记的 `abedb20f868bf99c`｜盘上现在的 `99665bc6f4471b30`」；重烘 `shaders`＋`scene` 后
  三个 shader 键与场景键全换、出图恢复且**与基线逐字节相同**（改的是注释）；把那一行撤回再重烘，
  键**逐字节回到基线**（键是纯函数）。
- ⚠ **残留（有意留着，没处理）**：服务在跑的时候改库文件，Bevy 的 `file_watcher` 会把新模块组装进去、并重建管线，而**槽版本没变** ⇒ 在下一次装载场景之前的那一小段里，画的是"新库 + 旧键"。下一次请求会被上面那条闸门拒掉，但那一小段窗口没有机制兜 —— 要彻底就得把库也变成 CAS 成员（是另一个档次的改动：场景文档要声明库成员、`slots://` 内存目录要扩到库、探针与门全跟着改）。
- 场景里用 `members["shader"]` 引用它 —— 和场、网格**同一个引用方式**。
- 渲染器侧：注册一个 `slots://` 资产源（`bevy_asset::io::memory::Dir` 内存目录 + `MemoryAssetReader`），材质**静态**返回 `ShaderRef::Path(slots://<槽>.wgsl)`；装载场景时把产物里的 WGSL 写进内存目录，再 `asset_server.reload`。
- ⚠ **资产源必须在 `AssetPlugin` 之前注册**（Bevy 自己的文档：`bevy_asset/src/lib.rs:561`，违反时 `:614` 会打 `must be registered before AssetPlugin`）。把 `SlotsPlugin` 排在 `DefaultPlugins` 之后 ⇒ 槽不存在 ⇒ shader 加载失败 ⇒ 管线永远编译不出来、服务永远到不了「渲染管线全部就绪」，而**编译与离线测试全绿**。这条只有真起一次服务才暴露 —— 别把「编译过 + 单测过」当成「能跑」。
- **为什么这样是对的**：查实了 Bevy 的热重载就是这条调用 —— 文件监视器发 `ModifiedAsset` → `reload_path` → `reload_internal(path, true)`（`bevy_asset/src/server/mod.rs:2163`、`:2209`），而公开的 `AssetServer::reload` → `reload_internal(path, false)`（`mod.rs:952`）：**同一个函数，只差一个 log 标记**；下游 `pipeline_cache.rs:745` 听 `AssetEvent::Added | Modified` 重建管线。所以"运行时换 shader"不是绕路，**就是热重载本体**，区别只在触发者（场景装载 vs 文件 mtime）与字节来源（CAS vs 资产文件）。
- 换来的第三条：**可复现**。文件热重载不记录当时用的是哪一版 shader，改一次文件、下一次测量量的就悄悄变成别的东西；场景产物把 shader 钉在内容键上。
- ⚠⚠ **每请求都 `install + reload` 是个静默的大坑**（实测踩过）：reload 会把用这个 shader 的管线全部打回重编，而出图只等 `FRAMES_AFTER_JOB = 6` 帧 ⇒ 那一张图上**云壳没有管线可画，云直接消失**。更坏的是它**一切指标全绿**：退出码 0、不是洋红、图片大小稳定、编译与单测全过 —— 于是 `orbit` 与 `orbit-bare` 渲出**逐字节相同**的图，仪器测到的"云成本"恒为 0。判别靠哈希：`p2-single ≡ p2-seq-orbit ≡ p2-ablated ≡ scene-smoke`（全是 91071 / `5D7947A7…`），而修好后 `orbit` = 106070 / `4BE5AE3F…`，**与旧旗标路的 `legacy-smoke` 逐字节相同** —— 这一条同时钉死了因果。
  修法两处：① 只在槽内容**真变了**时才装（`ShaderSlots::peek()` 比对，日志写「槽里已是这一份，不重装」），顺带不再每请求把管线打回重编；② `drive` 里加 `awaiting_pipelines`：装过 shader 的那一步先等"管线真的入了队"再开始数出图前的帧（上限 40 帧，等不到就带警告照常出图，有界不挂）。
  **教训**：内容搬进产物之后，"换内容"的代价从"改文件"变成了"重编管线"，而重编是异步的 —— 出图窗口必须等它，否则画面缺件而所有门都是绿的。
- 代价：改 shader 的循环变成「改 `art/shaders` → 重烘 `shaders` → 重烘 `scene` → 用新的场景产物路径请求」，热重载那条 1 秒近路不再直接适用（引擎那半截还在，缺的只是工具层的 watcher 去串这一步）。
- ⚠ Bevy 的 `Material` 所有 shader 钩子都是**静态**的（`fragment_shader()` 没有 `self`），所以「槽名 → 绑定布局」永远由编译期注册表决定；产物能决定的是**槽里的源码**，不是绑定布局。另：占位 WGSL 是 Rust 内联常量（没有占位文件），并单独进了 shader 门（`slot_placeholders_parse_and_validate`），否则它会从「每个 shader 都要解析+校验」下面溜走。

### §52.4 判据

- 场景不成形 ⇒ `Err`（不是画一半再静默兜底）；
- **场景键 = 场景 JSON + 全部成员键** ⇒ 成员内容一变场景键就变（§17.1）；
- 逐字节判据现在是"两份只差一个 part 的场景产物之间的 PNG 对比"（实测：`orbit` 对 `orbit-bare` 差 10.05% 的通道，同一对场景在旧接口下量的）；
- `cargo test` 里 shader 门同时覆盖 `art/shaders/*.wgsl`（真本）与 Rust 内联占位，按文件名解析时**两处同名要报错**（否则 `px_probe` 的梯度仲裁者会静默拿到占位 = 判据被换成空壳）。

### §52.5 状态（P10 落地后）

- ✅ `--scene <场景产物>` + `--pcg-root <目录>`（默认 `target/pcg`）已通。
- ✅ **内容旗标全删**：`--planet/--mesh/--clouds/--cloud-slope/--cloud/--cloud-ablate/--scatter/--atmo/--ambient/--palette/--displace/--sea/--radius/--spin/--rings` 连同 `render::Scene::Planet` 变体、`Options::planet_spec()`、`ServerAblate` 资源一起没了。`--scene` 可以给多次（每步的 `--out/--cam` 配在它前面那个 `--scene` 上）。`SCHEMA_VERSION 8 → 9`，快照同步更新。
- ✅ **消融归内容**：clouds part 的 `ablate = "surface"`（文本参数）走 `clouds::Ablate::parse`（认不出就报错并列出可用档名）；`params.steps` 与新增的 `params.surface_level` **现在都由 WGSL 硬表面路径读**（`SURFACE_STEPS` / `SURFACE_LEVEL` 两个常量删掉 —— 在这之前场景里写的 `steps` 对硬表面路是个谎）。三处 `struct CloudParams` 必须同序同布局：`art/shaders/clouds.wgsl`、`clouds.rs`、以及 `slots.rs` 里的内联占位。
- ✅ **产物相机表接上**：`--sheet` 是裸开关，用 `.pxart` 里那 12 台评审相机，复用同一套 `SheetCell`/viewport 逻辑（实测 1920×900，洋红 0）。
- ✅ **批量请求**：`Scene::Sequence { shots: Vec<Shot> }`（`Shot { scene, out, cam? }`，**没有 ablate**）；`ActiveJob` 排队，每存完一张 despawn `ScenePart` 再按下一步重建、重置 `warm/requested/warned`，**一步一行「出图：」**（harness 靠它逐步同步），队空才回 `Response`（`shots` 列全部、`out` = 最后一张）。实测两步：`orbit-bare` 91071 / `orbit` 106070，两张不同 ✓。
- ✅ 旧路数值不变有回归测试钉住（`CloudShape::default()` = `CloudParams::new` 原先那 14 个数）。
- ⚠ `--view` / `--show` 只到**编译级**：`ViewRequest` 换成「场景路径 + 内容键」、窗口只在键变了才重建，但没有开窗口实跑。
- ⏳ 若一份 WGSL **真的换了内容**，那一张仍要赌重编能在 40 帧内入队；要彻底就得在装过之后等 READY 再放行（怕管线永远编不出来时把任务挂住，所以现在是有界等待 + 警告）。
- ⏳ `--scatter` 删掉后 `planet::spawn_scattering`（Bevy `AtmosphereSettings` 那条）与 `planet::check_scene` 成了死码，留着没删。

## §62 坏管线要**当场拒**，不许"一直 pending"

**病**（2026-09-15，用户报："server 请求遇到坏管线要提前退出而不是一直 pending"）。出图前
只有一道路障，它有两个洞：

1. **预算记错了对象**：`drive` 比的是 `PIPELINE_DEADLINE = 1800` 对 `Ticks` —— 那是
   `watch_lease` 每帧 +1 的**服务启动以来帧数**，从不按请求重置。于是服务起来满 1800 帧
   （60 Hz 下 30 s；`--fps` 下几秒）之后，"等管线"这道闸**整个失效**，`drive` 直接开拍 ⇒
   又回到 P32 那个"静默出缺材质图"（退出码 / 颜色 / 字节数 / 编译 / 单测全绿，只有哈希看得出）。
2. **等不到就永远等**：`drive_stable` 的 `StablePhase::Pipelines` 是
   `if state != PIPELINES_READY { return false; }`，不设期限。而 `ShaderNotLoaded` /
   `ShaderImportNotYetAvailable` 在 Bevy 里是**无限重试**的（`bevy_render` 的
   `render_resource/pipeline_cache.rs` 685~690 行把它们打回 `Queued`）：只要那份 shader 进不了
   `ShaderCache`，`pending` 就永远 > 0 ⇒ 永远不就绪 ⇒ 性能那一路的请求**永远挂着**
   （客户端 180 s、服务端监听线程 300 s 才报超时）。

**判据（两条，缺一不可）**：

- **能证明坏了 ⇒ 当场拒**，不等超时。两条证明路：
  · 渲染世界：`ProcessShaderError` / `CreateShaderModule` 是**终态**（同文件 692~707 行不再重试）
    ⇒ `watch_pipelines` 把它连同**管线名 + naga 原文**写进 `RenderReady.failure` 并置 FAILED；
  · 主世界：`AssetServer` 那侧 `LoadState::Failed`（含依赖 / 递归依赖）⇒ `pipeline_gate` 与
    `accept_jobs` 当场拒并把失败资产点出来。
- **只是还没编完 ⇒ 有界地等**：预算按**这一步**（`rebuilt_instant`）起算（`PIPELINE_WAIT_BUDGET`）。
  超预算的出路是**不出图**（回 `Frame::Refused`），不是"警告后照出"——后者正是 §34 那条不变式
  禁止的"少了那个材质的成功图"。超预算**不动管线、不置失败** ⇒ 下一次请求照样能拿到。

**失败明细不是锁**：`RenderReady.failure` 只记**当前这一批**失败的第一条原因，管线重新编过
（`watch_pipelines` 里失败清空时 `clear_failure`）或重建（`invalidate`）就清。理由：一次坏管线
不能把服务钉死到重启 —— 否则"在这修好之前拒绝一切出图任务"会变成"永远拒绝"。

**落点**（`px_render/src/main.rs`）：

| 在哪 | 做什么 |
|---|---|
| `RenderReady::{failure, fail, clear_failure, invalidate}` | 失败明细：第一条原因 + FAILED；恢复/重建清 |
| `watch_pipelines` | 有 `failures` ⇒ `ready.fail(明细)`；没有 ⇒ `clear_failure()` |
| `accept_jobs` | 收件箱拿到新请求先查明细与 shader 库装载态 ⇒ **搭场景之前**就拒（这才是"提前退出"） |
| `pipeline_gate` | 出图前唯一的闸：`Ready` / `Waiting`（有界）/ `Refuse(原因)` |
| `shader_load_failure` | `AssetServer` 侧**可证明**的坏（含依赖与递归依赖） |
| `drive` | 闸不放行就 `return`；`Refuse` ⇒ 回 `Refused`、放下这一步 |

**实测**（Vulkan / RTX 3060 Laptop / 坏法是在运行时那份 `px_render/assets/shaders/light.wgsl`
末尾加一行 `this is not wgsl`；服务用
`--pcg-root .worktrees/cloud-surface-perf/target/pcg`，场景 `orbit-soft`）：

| 请求 | 结果 |
|---|---|
| 坏 shader 库 | **1~2 s** 回 `渲染服务拒绝：渲染管线失败，拒绝出图：premultiplied_alpha_mesh_pipeline｜Composer error: … found "this"`，退出码 1，`target/failfast-shot1.png` **不存在** |
| 修好 `light.wgsl` 后**同一个服务**再请求 | 成功，`357843` 字节 / `de36e672a30b502f…`，服务没重启 |
| `PIPELINE_WAIT_BUDGET` 临时改 300 ms 后第一次请求（真 shader 要现编） | 回 `⚠ 等渲染管线就绪超过 300ms：这一步不出图…`，退出码 1，不出图 |
| 紧接着同服务第二次请求 | 成功，`357843` 字节（管线没被终止） |

`cargo test -p px_render` 全绿（新增 `a_proved_failure_keeps_its_first_reason_until_the_pipelines_recover`），
好管线那条路照旧出图（最终二进制 + 真 30 s 预算：`357843` 字节 / 退出码 0）。

**没验到**：Bevy 那条"无限重试"支路在本机**造不出来**。试了两种造法（文件尾部 `#import` 被
naga_oil 忽略；顶部 `#import "definitely-missing.wgsl"` 居然照样编过，只是慢 1.2 s）⇒ "等超预算"
只有上表那次人为把预算改成 300 ms 的实测。

**没做**：`drive_stable` 的 `Assets` 相位仍只看 `is_loaded_with_dependencies`（资产卡在
`Loading` 永不落地时会无限等）；它现在被 `pipeline_gate` 的资产检查盖住同一批句柄，但那条路
本身没改。viewer（`--view` / `--show`）没接这道闸。


---

## §65 通用渲染：渲染器只认产物，不认行星（2026-09-15）

**病**：`pcg → render` 那条路一直叫"内容由产物决定"，但产物说的其实是**参数**，渲染器仍然
认识"行星 / 云 / 大气"三件套 —— `KINDS = ["planet","clouds","atmosphere"]` + `assembler(kind)`
+ `PlanetSpec` + `spawn_planet`。代价是每加一种东西就要在渲染器里加一个分支、一份材质类型、
一个槽名：**渲染器的形状被内容拽着走**。这一轮把它倒过来。

**新形状**：一份 `.pxart` 就是一份**渲染文档**（`px_protocol::scene` v2，`SCENE_SCHEMA = 2`）：

| 文档里有什么 | 渲染器拿它做什么 |
|---|---|
| `objects[]`：几何（`mesh` 产物 或 内建图元）+ 材质 + 世界系变换 + 投不投影 | 建实体，一样一个 |
| `material.shader`：一份 WGSL 产物 | **反射**出它的绑定契约，装进唯一的那个 shader 槽 |
| `material.params`：按**名字**给的数 | 按那份 WGSL **自己声明的结构体**打包（偏移从 naga 读） |
| `material.textures`：按**绑定下标**给的贴图产物 + 采样器 | 绑到那一格（采样器跟图一起走） |
| `material.alpha / cull / depth_bias` | 三条渲染状态 |
| `lights[]` | 点 / 聚 / 平行各一盏实体（位置、色、强度、射程、开不开影全是数） |
| `environment`：环境光 + 天空盒（cube 贴图产物）+ 亮度 | 相机上的 `AmbientLight` / `Skybox` |
| `cameras[]`：**世界系**方向 + 距离 | `--sheet` 那 12 格视口 |
| `expects[]`：内容声明的期望标签（如 `clouds`） | 只当字符串转给报告（判据用），渲染器不认识它 |

**绑定约定**（`px_render::reflect` 的表，也是全部契约）：

```
第 0 格  uniform  参数块（结构体由这份 WGSL 自己声明）
1 / 2    texture_2d<f32> / sampler      ← 第 1 格那张 2D
3 / 4    texture_2d<f32> / sampler      ← 第 3 格那张 2D
5 / 6    texture_cube<f32> / sampler    ← 第 5 格那张 cube
7 / 8    texture_cube<f32> / sampler    ← 第 7 格那张 cube
```

**为什么布局必须是固定超集**：Bevy 每种材质类型只建**一份**绑定布局（
`Material::fragment_shader()` 是静态函数），而产物能决定的只有槽里的源码。于是布局写成
"参数块 + 4 格贴图（各带采样器）"的**超集**，空着的格一律绑兜底贴图；参数块那一格
`min_binding_size: None`，每个材质各自建缓冲。**代价**：多绑 6 个空格（实测无代价，
42→51 条管线、出图时间同量级）；**收益**：换 shader 不动布局，一版 WGSL = 一条管线。

**参数为什么是反射的**：产物给的是"名字 → 数"，而"名字在缓冲的第几个字节、是什么类型"
只有那份 WGSL 知道。写第二张 Rust 表就是第二个会漂开的默认值 ⇒ 用 naga 读它自己声明的
结构体（`reflect.rs`）。三档当场报错、不静默：**缺参 / 多参 / 类型不符**。反射结果按
（内容版本, 库指纹）缓存 —— 键 = 内容，同一版永远反射出同一份契约。

**搬走了什么**（这是这一轮的主体）：

| 原来在渲染器里 | 现在在哪 |
|---|---|
| `planet.rs` / `clouds.rs` / `surface.rs` / `atmosphere.rs`（四个材质 + 生成器 + spawn） | **删掉** |
| 色板 → 颜色/发光贴图（含整条 mip 链、极点滤波） | `px_ops::generate::surface_color`（逐字节相同） |
| 覆盖度 RGBA16F 立方图 | `px_ops::generate::coverage_cube`（逐字节相同） |
| 星空立方图 / 环带贴图 / 环网格 | `px_ops::generate::{stars, ring_band, ring_mesh}`（逐字节相同） |
| `KINDS` / `assembler` / `SceneBuild` / `PlanetSpec` / `spawn_planet` | **删掉**；语义搬到 `px_graphs --bin scene`（配方 → 文档） |
| `SYSTEM_TILT`（相机与物体的倾斜） | 烘图侧：文档里的方向与朝向**都是世界系**的 |
| `SUN_RANGE_FACTOR` / 天空盒亮度 / 云影 gain | 烘图侧（写进文档的数） |
| 三个槽（clouds / atmosphere / surface） | 一个槽（`material.wgsl`） |
| 渲染器里的 `CloudParams` / `SurfaceParams` / `AtmosphereParams` | 烘图侧一份（`px_ops::generate` + 场景编译器）；探针那一侧另有一份镜像（`px_probe::params`） |

**留下的**：`mesh.rs`（网格产物 → `Mesh`，含缠绕翻转与法线焊）、`art_cache.rs`（三张表：
mesh / texture / shader，键 = 路径 + 载荷指纹）、`material.rs`（`DocMaterial`：手动
`AsBindGroup` + 固定超集布局 + 每版 shader 一条管线）、`reflect.rs`、`scene.rs`（通用装配）、
`slots.rs`（一份 WGSL 一个内容版本，最多养 4 版）。

**判据（同一 worktree、同一套工具链，只差代码；`--cam 0,5,3.2`、960×640）**：

| 场景 | 不同像素 | 最大通道差 |
|---|---|---|
| `orbit-bare`（行星 + 大气，无云） | **0 / 614400** | 0（逐字节同像素） |
| `orbit-allmiss`（云的片元全 discard） | **0 / 614400** | 0（逐字节同像素） |
| `orbit-soft`（云 + 云影） | 22 / 614400（0.0036%） | 2 |
| `orbit-soft-nocloudshadow`（云、无关云影） | 33 / 614400（0.0054%） | 2 |
| `orbit-soft` 的 12 视角对照图（2240×1050） | 944 / 2352000（0.04%） | 17 |

**已经逐项验过相同的**：网格与贴图产物（内容键相同 ⇒ 字节相同）、云的组装后 shader
（`diff` 只有绑定下标与注释）、云的 25 个参数（与配方/旧结构体逐值核对）、灯
（位置/色/强度/射程/影）、相机、物体变换（`quat_mul`/`rotate`/`length` 都改成 glam 的
**逐项次序**：等价的另一种写法在 f32 下差最后一位，会被 `looking_at` 放大成亚像素抖动）。

**没归因**：带云的两档那 20~30 个像素。输入、数学、状态都已逐项验过相同，剩下的唯一差别是
**管线/绑定布局**（云那张 cube 从第 1 格挪到第 5 格，布局多了 6 个没用的格）。要钉死它需要
一个专门实验：把约定改成"cube 在前"，让云的 cube 回到第 1 格、地表的 2D 挪到后面，看差异是
跟着 cube 走还是跟着地表走。**没做**（时间预算），也**不该**在没钉死之前随便改约定。

**硬化**：`for _ in 0..8 { 重出 }` 无（没做稳定性扫描）；`cargo test -p px_protocol -p px_ops
-p px_graphs -p px_verify -p px_render` 全绿；`px_probe` 编译过（三个探针 bin 这一轮**没跑**）。

### §65.1 环：唯一一条没实测过的路径，现在有图了

`rings > 0` 这条批场景里**从来是 0** ⇒ 迁移前那套 `spawn_rings`（Bevy 内建
`StandardMaterial { base_color_texture, unlit, blend, cull: none }`）**没有一张实测图**，
迁移后它换成自写 `art/shaders/ring.wgsl` + 烘图侧生成的环网格（`generate::ring_mesh`）
与环带贴图（`generate::ring_band`）—— 两条都只在代码里活着。

**判据**：新增配方 `art/scene/orbit-rings.toml`（`rings = 1.6`，行星 + 大气 + 环）。

```
cargo run -p px_graphs --bin scene orbit-rings
px_render --scene <产物> --cam 0,22,4.2 --out target/rings-shot.png --width 1200 --height 800
```

实测：`placeholder_px = 0`（不是占位）、3 个物体（planet / atmosphere / rings）、
管线 0 失败、`rings/shader@5b1613353a06` 进了槽。图里：环面与行星**同一个倾斜**
（`SYSTEM_TILT` 在世界系里，环与行星拿的是同一个四元数）、近侧环压在行星上、远侧被行星挡住、
环带有条纹（`ring_band` 的 alpha 环）、没有剔除错面（`cull = none`）。

**没验**：环的**逐像素**对照（迁移前那条路没有基线图可对）；环的曝光处理与内建
`StandardMaterial{unlit}` 不同（后者乘 `view.exposure`，自写材质不乘 —— 与云/大气同一处口径，
见 `10-handoff.md` §9.1.7 第 3 条"云自己的曝光没动"）。

### §65.2 仪器跟着改：harness 的 shader 一致性闸门

`tools/harness.ps1` 的 `Get-SceneShaderMembers` 原来按 **v1** 的 `scene.parts[].members[role]`
读场景帧。v2 文档没有 `parts` ⇒ 它返回一张**空表**，而空表在 `Assert-ShaderMembersAgree`
里等于"没有不一致"⇒ **闸门静默失效**（比报错坏得多：那正是"混版量出来的数"要拦的东西）。

改成读通用渲染文档的每个 `objects[].material.shader`；并且**读到 0 条就抛错**
（"仪器拿不到数据时必须响"）。两条判据：

```
. .\tools\harness.ps1
Get-SceneShaderMembers -Path <v2 产物>   # → planet/shader=shaders/surface@… / atmosphere/… / clouds/…
Get-SceneShaderMembers -Path <v1 产物>   # → 当场报错「场景帧不像通用渲染文档（缺 objects）」
```

后者是本轮从**我的 worktree 的 CAS** 里翻出来的一份旧产物（`target/pcg/ab/d3/d3bcdfcc…pxart`）
当反例。

## §66 材质参数 schema：谁定义它、能不能真正动态（2026-09-16 调研，**代码未动**）

**起因**：一个断言 —— 「render 对 schema 的要求是 compile time fixed，材质 schema 就应当定义在 `px_protocol`」。
**结论**：参数块的**大小与内容今天已经是完全动态的**（Bevy 那条路没堵），被钉死的只有**绑定契约**那一层；
而「动态 schema」只能落在两条线上 —— **文本层（naga 反射）**或**数据层（schema 当产物/当输入）**，
没有第三条：WebGPU 把驱动反射那条路删掉了。

### §66.1 硬边界（都在依赖的源码里查实，不是推理）

| 层 | 能不能动态 | 证据 |
|---|---|---|
| 绑定格集合（组号、格号、维度） | **不能**：同一类型的材质只有一份布局（只能"固定超集"） | `bevy_render-0.19.1/src/render_resource/bind_group.rs:609`：`bind_group_layout_entries(render_device, force_no_bindless)` 是**静态**方法（没有 `&self`） |
| 材质绑定组在第几组 | 不能，Bevy 写死 | `bevy_pbr-0.19.1/src/material.rs:466-486`：`descriptor.layout.insert(3, …)` + shader def `MATERIAL_BIND_GROUP = MATERIAL_BIND_GROUP_INDEX`（= 3）—— 槽 shader 里那句 `#{MATERIAL_BIND_GROUP}` 运行期就是这样变成 3 的 |
| 参数块的**大小与内容** | **已经能**，逐材质任意 | `px_render/src/material.rs:102-113`（`min_binding_size: None`，注释原文就是"每个 shader 的结构体大小不同，而布局只有一份"）+ `:143-153`（逐材质 `create_buffer_with_data`） |
| 运行期问驱动"这程序有哪些 uniform" | **没有这条路** | GL 有 [`getActiveUniform`](https://developer.mozilla.org/en-US/docs/Web/API/WebGLRenderingContext/getActiveUniform)；WebGPU 明确放弃（[gpuweb#2470](https://github.com/gpuweb/gpuweb/issues/2470)），代价是 shader / layout / bind group 三处必须互相兼容、信息重复（[Toji: WebGPU bind group best practices](https://toji.dev/webgpu-best-practices/bind-groups.html)） |
| 反射从哪来 | naga（今天）+ naga_oil（还能注生成物） | `px_render/src/reflect.rs:295-340` 读 `member.name` / `member.offset`；`naga_oil-0.22` 的 `Composer::make_naga_module` 直接回一份 `naga::Module`；`add_composable_module(ComposableModuleDescriptor { source, as_name, shader_defs, … })` 能把**生成的模块**注进组装 —— Bevy 自己就是这么把 `MATERIAL_BIND_GROUP` 注进去的 |

⚠ **顺手查到的地雷（今天自洽，谁都别乱"修"）**：`#{MATERIAL_BIND_GROUP}` 在**运行期是 3**（Bevy 的 def），
而我们自己组装文本时替成 **`"2"`**（`px_render/src/shaders.rs:265`，`reflect::MATERIAL_BIND_GROUP` 也是 2），
`px_probe` 的管线布局同样放在第 2 组（`px_probe/src/probe.rs:406` 的 `&[None, None, Some(&material), …]`）。
两边各自自洽（反射只需要文本**内部**一致；probe 用自己的管线），所以今天没错 ——
但这是「同一条契约、两个数字」的活证据：谁哪天把那处字面量"修"成引用常量，就会静默错位。

### §66.2 同一条契约今天有五个真相源

| 位置 | 它说了什么 |
|---|---|
| `px_render/src/reflect.rs:22-30` | 真源：组号、第 0 格 uniform、贴图只占 1/3/5/7 及各格维度 |
| `px_render/src/slots.rs:167-187` | 占位 WGSL 把那张表**手抄**了一遍 |
| `px_render/src/shaders.rs:265` | 离线组装把 `#{MATERIAL_BIND_GROUP}` 替成**字面量 `"2"`** |
| `px_protocol/src/scene.rs:262-284` | 半份：`TextureRef` 的注释 + `check()` 只查「贴图占奇数格、`>=1`」+ `sampler_binding()` |
| `px_protocol::scene::Value`（4 种写法） vs `reflect::ParamKind`（5 种类型） | **同一份类型词表的两半**，对应关系只活在 `reflect.rs:192-248` 的 `write_value` match 里（无类型级约束、无门） |

### §66.3 五条落地形态

| | schema 的载体 | 谁解析、什么时候 | 增改一个参数 | 一致性靠什么 | 代价 |
|---|---|---|---|---|---|
| **1 今天** | WGSL 文本 | render 运行期 naga 反射 | 只改 WGSL + 配方，**0 重编** | 反射即唯一来源 | schema 只在 render 可见 ⇒ 烘图侧**盲写**名字；每次装载跑 naga |
| **2 descriptor 进产物** | 反射结果落盘（`AssetManifest.params` 或单开一帧） | 烘图时反射一次；装载时读 | **0 重编** | 「产物记的 vs 盘上反射的」当场对账（`closure_check` 同款） | 烘图侧要能跑 naga（reflect 搬进共享叶子 crate，`px_graphs` 引 naga）；protocol 加 descriptor 类型 |
| **3 protocol 里的 Rust 类型** | `px_protocol` | 两侧**编译期** | 改 protocol + **两侧重编** + 快照 + 旧服务被握手拒 | **必须补一道跨层门**（反射 `art/shaders/*.wgsl` ↔ protocol schema 逐字段比） | 破 §65 / §10.4 的「加材质 = 0 编译」；换来类型安全（烘图侧编译期抓错，`px_probe/src/params.rs` 那份镜像可删） |
| **4 schema 生成 WGSL** | 同一份 schema（数据或 Rust），WGSL 的 `struct Params` 由它生成 | 烘图/装载时生成 | **0 重编** | 生成器保证一致（schema 是输入、WGSL 是输出） | WGSL 变成「生成的结构体 + 手写 body」；body 引用的名字要由生成器校验或改成生成常量 |
| **5 容器化** | `struct Params { data: array<vec4<f32>, N> }` + 生成的下标常量 | 谁都行 | **0 重编，且布局恒定** | 下标由 schema 唯一决定 | 类型信息丢、可读性差、浪费带宽；body 要写成 `params.data[K].x` |

### §66.4 选之前要认的三条判据

1. **被钉死的只是绑定契约那一层** —— 它是 Bevy / WebGPU 的硬约束（§66.1 前两行），逐材质变不了；
   但它的**声明**可以收敛到协议侧（`Material.params` 那一袋值已经在 protocol 里了）。
2. **真正和「schema 定义在 protocol」冲突的不是动态性，是"唯一来源"**：`px_render/src/reflect.rs:289-293`
   已经把这条裁决写死 ——「参数怎么排只能有**一个**来源，就是那份源码；写第二张表就是第二个会漂开的默认值」。
   ⇒ 要在 protocol 里定义 schema 而**不同时生成** WGSL，就必须补那道跨层门（§10.2 的思路：语言管不住就用门看住）；
   纯形态 3（protocol 类型 + 手写 WGSL + 无门）= 两份真相，与本仓既有裁决正面冲突。
3. **凡是参与"shader 到底是什么"的东西都要进键**（§17.1、§52.3 的 include 闭包就是刚补的一课）：
   schema 一旦生成 WGSL（形态 4/5），schema + 生成器版本就得跟闭包一样进 shader 键；
   「产物记一份、装载时对账」的落点已经搭好（`scene::closure_check`）。

### §66.5 状态与候选批次（都还没做）

调研完成（2026-09-16），**代码未动**。一并裁决待做的一条：**`Value::Text` 从 wire 的 `Value` 里删掉** ——
它今天在参数块里**永远非法**（`reflect.rs:192-248` 的 `write_value` 对任何 kind 都拒文本），全仓只有
配方 / 烘图侧在用文本（`palette = "rocky"`、`ablate = "surface"`）。

- **P1**：契约骨架（绑定表 + `ParamKind` + `Value↔ParamKind` 合法映射）收进 `px_protocol`、删 `Value::Text`、
  §66.2 那五处手抄改成单一源（含 §66.1 那个 2/3 的坑）、schema descriptor 落进产物 + 装载时对账。
  **不动 §65 的分工。**
- **P2**（叠在 P1 上）：protocol 里加材质 schema 类型（烘图侧编译期抓错）+ 那道跨层门 +
  探针改用 protocol 类型（删掉 `px_probe/src/params.rs` 的 `CloudParams` 镜像）。
- **P3**：schema 生成 WGSL 的结构体声明（或容器化 + 生成下标），schema 与生成器版本一并进 shader 键；
  `art/shaders` 从「手写资产」变成「模板」—— 这一刀要用户点头。

⚠ 三批都会动 `SCENE_SCHEMA`（形态 3 还要动 protocol 快照）⇒ 在跑的旧服务会被握手拒，这是设计不是故障。

> **续（2026-09-16）**：美术那边提出「schema 要能像 Shader Graph 那样动态改变 ＋ render graph 要能动态组装重载」，
> 于是又调研了一轮，落成 **`art/11-graph.md` §67–§73**。三条与本节直接相关的更新：
> ① **§66 的缺口清单要补**：除了本节那五处手抄，**烘图侧**还有两张手写词表
> （`px_graphs/src/bin/scene.rs:442-487` 的白名单与 `:517-586` 的映射、`shaders.rs:8` 的 `SLOTS` 常量）
> —— 它们才是「加一个参数要重编 Rust」的真凶；
> ② **Unity 的对照**：Shader Graph 的 schema 是**编译期**烘进材质的（材质是快照、运行期不能改），
> 所以「像 Shader Graph 那样动态」在运行期这一侧本仓**已经超过它**；缺的是作者侧（§68）；
> ③ **render graph**：Bevy 0.19 的 `RenderGraph` 已经是 **schedule**（不是节点图），
> 动态组装的正确落点是「文档里的 pass 表 + 一个执行器系统」，不是运行期改 schedule（§70）。

---

## §80 契约收口与加宽超集（§76 开工序的第 1 步，2026-09-16）

**一句话**：材质绑定契约从**八处手抄**收成**一份表**（`px_protocol::material`），
并在这份表上一次性把两个硬顶加宽（贴图 **4 → 12 格**、参数块 **1024 → 4096 字节**）。
提交 **`c44e136`**（分支 `feature/graph-research`）。

判据：**产物键逐字节不变**（只搬代码 + 加宽都换不动键）｜**出图哈希四张全同**
（`2b1a76f4…` = §75 的基线）｜配对量到的代价**在噪声内**（§80.3）｜84 个用例通过。

### §80.1 收口：表住哪儿、谁抄过它

| 原来（§67.4 的八处） | 现在 |
|---|---|
| `px_render/src/reflect.rs:22-30` 的真源（组号 / 格号 / 维度 / 上限） | **`px_protocol::material`**（新模块）：`MATERIAL_BIND_GROUP` / `PARAMS_BINDING` / `TEXTURE_SLOTS` / `MAX_PARAMS_BYTES` / `PARAMS_ALIGN` |
| `px_render/src/slots.rs:167-187` 的占位 WGSL 手抄同一张表 | 占位 WGSL **由表生成**（`slots::placeholder_material`）；单测把生成物组装 + 反射回来与表逐格对账 |
| `px_render/src/shaders.rs:254` 把 `#{MATERIAL_BIND_GROUP}` 替成字面量 `"2"` | 组装器（现在住 `px_shader::assemble`）替 **`MATERIAL_BIND_GROUP` = 3** —— 这就是 Bevy 在运行期用的那个数（`ShaderDefVal::UInt("MATERIAL_BIND_GROUP", 3)`，`bevy_pbr-0.19.1/src/material.rs:74` + `:466-473`）⇒ **离线组装 == 运行期组装** |
| `px_protocol/src/scene.rs` `TextureRef` 的半份 `check()`（「奇数格、≥1」） | `check()` 问 `material::texture_slot_of` —— 半份抄本在表加宽之后会**开始拒合法的格** |
| `Value`（4 种写法）vs `ParamKind`（5 种类型）：对应关系只活在 `write_value` 的 match 里 | `MaterialLayout::pack` 搬进 `px_protocol::material`，`Value ↔ ParamKind` 的合法映射就那一处 |
| naga 反射住 `px_render`（拖 bevy ⇒ 烘图侧用不了） | 反射搬进叶子 crate **`px_shader::reflect`**（该 crate 加 `naga 29` + `px_protocol`）；`px_render::reflect` 只剩「版本 + 库指纹 → 缓存」 |
| 组装（`bevy_pbr` 桩 + `#import` 展开）住 `px_render::shaders` | 搬进 **`px_shader::assemble`**：烘图侧也要组装 —— 烘 shader 产物时要反射出 descriptor |
| `reflect.rs:499-527` 把 `tint@16` / `inner@32` / `params_bytes == 128` **钉死**（§75 的 W4） | 只钉**名字与类型**（偏移是 shader 的事、改布局是作者的权利）；「25 格 / 128 字节」那条留给 `tests/cloud_field.rs`，因为它是**探针镜像**的契约，不是布局的 |

**依赖方向**：`px_protocol`（只有类型；运行时依赖被 `tests/crate_graph.rs` 钉死 `serde` / `serde_json`）
← `px_shader`（naga + 组装 + 反射）← `px_render` / `px_ops` / `px_graphs`。

**探针**：材质组从写死的 2 挪到 **3**、job/out 从 3 挪到 **4**（`px_probe/src/common.rs` 的
`MATERIAL_BIND_GROUP` / `JOB_BIND_GROUP`，WGSL 里写 `#{JOB_BIND_GROUP}` 由探针自己替）。
⚠ **探针三个 bin 本轮没跑**（待跑）。

### §80.2 descriptor 进产物（第二个 U8 blob）＋ 装载时对账

- 烘：`px_ops::write_shader` 反射一次 → 规范 JSON（字段顺序由结构体决定，没有 map 迭代顺序可以漂）
  → 写成**第二个 U8 blob**（`[0]` WGSL、`[1]` descriptor）。
  ⚠ **它不参与键**（键 = `px_shader/v2` ‖ `SHADER_VERSION` ‖ 闭包指纹 ‖ WGSL 字节）⇒ 加这一条**不换任何产物键**。
  但**改反射规则要同时升 `SHADER_VERSION`**（`px_ops::shader_key` 的注释里写了为什么）：
  否则盘上会出现「同一个键、两份契约」。
- 装：`art_cache::shader` 用 `art::read_shader_parts` 把两半一起读出来；`scene::schema_check`
  拿**产物记的那份**与**现在反射出来的那份**逐字节比：
  - 对不上 ⇒ 拒（「这份产物是拿另一版**反射规则**烘的：里面的参数值按老契约打包」+ 重烘配方）；
  - 没记过（契约收口之前的产物）⇒ 拒（与 §52.3 的闭包闸门同款）。
- 为什么必须对账：反射规则会变（表加宽、`ParamKind` 多一档、偏移规则修正），而**键 / 清单 / 场景键 /
  槽版本全都不动** ⇒ 那是「同一个键、不同内容」的另一种形态。

### §80.3 加宽超集：数字与代价（配对测量）

| 档 | 贴图格 | 参数上限 | 出图 sha256 | gpu p50（两轮） |
|---|---|---|---|---|
| **A**（加宽前） | 4（2×2D + 2×cube） | 1024 字节 | `2b1a76f4…` | 4.718 / 4.548 ⇒ 均值 **4.633 ms** |
| **B**（加宽后） | **12（8×2D + 4×cube）** | **4096 字节** | `2b1a76f4…`（逐字节同 A） | 4.541 / 4.533 ⇒ 均值 **4.537 ms** |

- 差 **−0.096 ms（−2.1%）**，在噪声内 ⇒ **空格不花钱**（与 §65「多绑 6 个空格无代价」同结论，这次是 16 个）。
- 口径：`--perf --frames 60`、960×640、场景 `orbit`、Vulkan、**配对**（A/B/A/B 逐轮换序）、
  两支 exe 各自起服务（`target/step1/px_render-a4.exe` / `-b12.exe`，报告与图在 `target/step1/`）。
- ⚠ **一次被污染的读数（值得记住）**：单发对比（A 先跑）给出 A **10.43** / B **5.49 ms** ——
  而 A 档自己的前 5 帧是 5.4 ms、之后跳到 10.4 ms：**别的负载插进来了**（同一个 worktree 的另一会话在用同一块 GPU）。
  **结论：这类比较必须配对**，单发数字再漂亮也不算数。
- 老四格 `1 / 3 / 5 / 7` **一个都没动**：加宽只许往后**追加**（挪老格 = 把既有 shader 的贴图换到别的格上），
  `material.rs` 与 `slots.rs` 的单测钉着这一条。

### §80.4 没做 / 待办

1. **配方透传（§79 的 W1）不在这一步**：`CLOUDS_KEYS` / `PLANET_KEYS` / `ATMOSPHERE_KEYS` 与 `SLOTS`
   一个字没动 —— 那是 §76 开工序的**第 2 步**（做完这一步，「加一个参数」才真的不用重编）。
2. 探针三个 bin（`device` / `gradient` / `field_dual`）**没跑**：这一轮只改了它的绑定组号，
   机械改动，但按本仓规矩「没跑过不算验过」。
3. ⚠ `tests/cloud_field.rs` 那条门**红着** —— **不是这一步引入的**：
   `art/shaders/clouds.wgsl` 上有一处**不是本轮**的未提交改动（`@align(16) density`），
   把 `CloudParams` 从 128 撑到 **144**（`wind_skin@128`、span 132 对齐到 16），
   而那条门钉的正是**探针镜像**的 25 格 / 128 字节。已按用户裁决留着不动，等那边收工再撤。
4. `--view` / `--sheet` 没跑；`SCENE_SCHEMA` 的「加法不升版本 + 拒未知字段」也没做（同属第 2 步）。
