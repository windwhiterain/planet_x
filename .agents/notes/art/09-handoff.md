# 09 交接：现在在哪儿，下一步做什么

> 这篇是**唯一的现状出口**。别处只写「是什么 / 为什么」，只有这里写「做到哪了、还差什么」。
> 每次开工前先读这一篇，收工前改这一篇。

---

## 9.1 现在在哪儿

- worktree `.worktrees/field-dual`，分支 `wip/field-dual-arbiter`。
- 已提交：`0071f5d art: 探针搬出 cargo test；产物带相机表与指纹；render 按内容缓存资源`。
- ⚠️ **`game` 编译不过，而且是本轮之前就有的**：`game/src/project.rs:17` 读 `snapshot.executions`，
  而 `Snapshot` 已把这个读数换成 `intake`（`game/src/lib.rs:37` 的注释写着"该读数已随 distribution
  归一化一起删除"）。默认 members **含 `game`** ⇒ `cargo test` 是红的。
  这是 sim 的语义裁决（`DepartmentView.execution` 该喂什么），**没有替它决定**。
  在它修好之前用：
  `cargo test -p px_protocol -p px_ops -p px_graphs -p px_verify`。

## 9.2 已经能跑什么

```
cargo run -p px_graphs --bin planet     # 烘星球（height + surface mesh）
cargo run -p px_graphs --bin clouds     # 烘云（mixed + coverage + 三个 slope）
target\debug\px_render.exe --serve --width 512 --height 352
target\debug\px_render.exe --planet <H.pxart> --mesh <M.pxart> --palette rocky `
   --clouds <C.pxart> --cloud-slope <SX,SY,SZ> --width 512 --height 352 --sheet target/sheet.png
```

`--sheet` 用产物自带的相机表（`.pxart` 的 `AssetManifest.cameras`）出**一张多视角对照图**，
一次请求一个场景 N 个视口。给了 `--sheet` 就不要再给 `--cam`。

## 9.3 已验证 / 未验证（**这条最要紧**）

**已验证**：

- 编译门：`cargo check -p px_protocol -p px_ops -p px_graphs -p px_verify -p px_render -p px_probe --all-targets` 全过。
- 测试（CPU，秒级）：`cargo test -p px_protocol` 28 个、`cargo test -p px_render` 11 个
  （含 6 条 `art_cache` 单测）、`cargo test -p px_protocol -p px_ops -p px_graphs -p px_verify` 全绿。
- GPU 冒烟（DX12 / RTX 3060 Laptop / 44 条管线 0 失败）：
  - `--sheet` 12 格都画了、构图正确（第 3 行是棱/角/面心特写）、无 shader 报错；
  - 同一场景连发 4 次：**冷 1867 ms → 热 1525 ms**，热/热抖动 ±6 ms，
    **四张图 SHA256 全同**（`42ff067e…`）；
  - 程序化球面那条路（不带 `--mesh`）：`sphere`/`texture` 冷→热全命中；换 `--palette ice`
    时**场仍命中、球面与贴图重造**；冷落淘汰真的触发。

**未验证**：

1. **三个探针 bin 从没跑过**（`field_dual` / `gradient` / `device`）。它们守着梯度对错的**唯一**判据，
   而这条判据自 §46 起就没再被执行过。
2. **viewer 那条路**（`--view` / `--show`）本轮没跑：`poll_field` 的「mtime 动过但指纹没变 ⇒ 不重建」
   分支只有 `cargo check`。
3. `--sheet` 的对照图**不可与旧的 `target/probe-*.png` 逐像素比**：相机语义从「世界 yaw/pitch + 倾斜」
   换成「局部方向」，整体转了 19.5°（`SYSTEM_TILT`）——这更正确，但旧图作废。
4. §40.3 那条**窗口比 `--serve` 暗**仍未解释。

## 9.4 第一次冒烟的顺序

```powershell
# 0) 最便宜的 GPU 门（约 10 秒）：探针能不能起、后端是不是 DX12
cargo run -p px_probe --bin device

# 1) 重烘（相机表进了缓存键 ⇒ 会得到新的 CAS 路径）
cargo run -p px_graphs --bin planet     # 记下打印的 <HEIGHT.pxart> 与 <MESH.pxart>
cargo run -p px_graphs --bin clouds     # 记下 <mixed.pxart> 与三个 <slope.pxart>

# 2) 常驻服务；等日志出现「渲染管线全部就绪：共 … 条，失败 0 条」
target\debug\px_render.exe --serve

# 3) 一张对照图
target\debug\px_render.exe --planet <HEIGHT> --mesh <MESH> --palette rocky --sheet target\sheet.png
```

⚠️ 改代码前先停服务（`target\debug\px_render.exe` 被占用会让 `cargo build` 报「拒绝访问」）。
删 `target/render-server.json` 即可（4 秒内自查退出）。

## 9.5 下一步（按价值排）

**P4｜`tools/frame-probe.ps1` / `probe-clouds.ps1` 的 fail-fast**

实证：`target/fp-*.log` 四个文件 mtime 精确相隔 ~60 s ⇒ 那一次 `NoiseSample` 编译失败
让每个 case 白等满 60 s，而真错误被 `continue` 吞了。改法：`Wait-For` 加 `FailPattern`
（命中立刻返回并打印 `.err` 里的「管线编译失败」行）；`try/finally` 收尸；
按租约 pid 停服务，**别** `Get-Process px_render | Stop-Process`（全局杀）。

**P5｜`--cloud-ablate` 进 per-request `View`** ⇒ `frame-probe.ps1` 一个服务跑完 4 个 case。

**P6｜预热与管线**

只 warm 真正会用到的管线（现在 44 条里混着 `StandardMaterial` / `Skybox` 变体）；
评估 wgpu 的**磁盘管线缓存** —— 它是把「每次重启服务 15–25 s」降到 <1 s 的唯一现成手段。

**P7｜删死代码**：`atmosphere.rs::sync_cameras` + `AtmosphereParams.camera_x/y/z`
（shader 早就改读 `view.world_position` 了，它却把最后一台相机的世界位置写进**全局** material uniform）。
注意 uniform 布局要和 WGSL 同步改。

**P8｜`tools/probe.ps1` 退休** ⇒ 直接 `px_render --sheet`。删之前先跑一次 9.4 的第 3 步、和旧图并排比一眼。

**P9｜缓存只覆盖了资源的一半**：两个 `ico(64)` 壳（云、大气）、环、`ring_image(1024,4)`
仍是每请求现造 —— 相对那几百万 texel 是零头，但不是零。

### 一条改变排序的实测

资源缓存（§50）省下的是 **1867 → 1525 ms ≈ 340 ms（18%）**，剩下 82% 是**渲染本身**
（12 个视口 × 体积云 56 步）。早先记下的「探针巨慢的真正大头 = 服务端每请求全量重建场景」
**只对了 18%** —— 真正的大头在 shader 里，该用 §41 那台仪器去量。
所以 P4（白等 60 s）仍然值钱，但「服务端重建场景」不该再排第一。

## 9.6 常用命令

```powershell
.\tools\px.ps1 -Target test                    # 快速测试链（默认 members，不碰 bevy）
.\tools\px.ps1 -Target test-all                # 全量（含 px_render / px_probe，慢）
.\tools\px.ps1 -Target check                   # 只 check 不产 GPU 的那三个 crate
.\tools\px.ps1 -Target device                  # 最便宜的 GPU 门（约 10 秒）
.\tools\px.ps1 -Target field_dual              # §46.3 的 arbiter（唯一梯度判据）
.\tools\px.ps1 -Target gradient                # 探针冒烟 + 逐通道归因
.\tools\px.ps1 -Target planet   -Level opt     # 烘星球图；-Level opt 只提升本地 crate，不重编 bevy
```

## 9.7 文件地图

| 想知道什么 | 看哪个文件 |
|---|---|
| 路线裁决与「为什么不那样做」 | `art/01-decisions.md` |
| PCG 图程序怎么写、缓存键与版本号 | `art/02-pcg.md` |
| 产物里有什么（域 / 指纹 / 相机表 / diff） | `art/03-assets.md` |
| 球面网格、位移、法线、mip、接缝与极点 | `art/04-geometry.md` |
| 天空与大气（含 Bevy 散射大气的挂起状态） | `art/05-sky.md` |
| 体积云与覆盖度 | `art/06-clouds.md` |
| 云密度场的梯度：组装形式与判据 | `art/07-gradient.md` |
| 渲染器怎么跑、改完怎么验 | `art/08-instruments.md` |
| 不变式（每条都付过代价） | `art-framework.md` 顶部 |

代码入口：`px_protocol/src/art.rs`（产物与协议）、`px_ops/src/lib.rs`（算子与缓存键）、
`px_render/src/{main.rs,planet.rs,art_cache.rs,clouds.rs,atmosphere.rs}`、
`px_probe/src/{common.rs,probe.rs,field_dual.rs}`。
