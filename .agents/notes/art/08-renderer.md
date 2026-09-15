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

- `kind = Shader` 的产物：WGSL 作为 **U8 blob**，**键 = WGSL 的字节**（改一个字换一个键），由 `px_graphs --bin shaders` 从 `art/shaders/*.wgsl` 烘出，进 `target/pcg/shaders/manifest.json`。
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
