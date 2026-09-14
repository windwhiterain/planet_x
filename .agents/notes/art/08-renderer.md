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
