# 09 交接：现在在哪儿，下一步做什么

> 这篇是**唯一的现状出口**。别处只写「是什么 / 为什么」，只有这里写「做到哪了、还差什么」。
> 每次开工前先读这一篇，收工前改这一篇。

---

## 9.1 现在在哪儿

### 9.1.1 本轮（2026-09-15，`feature/cloud-surface-perf` worktree）：软档收影 ＋ 地表云影

**在哪条线上**：`.worktrees/cloud-surface-perf`（分支 `feature/cloud-surface-perf`）。软档与
场景产物那条路（§52）只活在这个 worktree 里，**v2 上没有** ⇒ 这一轮的所有改动都在这里。

**这一轮做了什么**（用户的两条要求，口径与实测全在 `06-clouds.md` §59）：

- **云收"别人"的影**：软分支每步采样 Bevy 的 directional shadow map（山尖 / 环挡住的光），
  云自己的自阴影**仍旧**是每步法线的 N·L。`DirectionalLight.shadow_maps_enabled` 由 planet part
  的 `shadows` 参数说了算（缺省 0 = 老行为）；级联按这颗行星定（2 级、0.1–2.5–6.0）。
- **地表云影**：`surface` 槽不再是"走 Bevy 内建材质"的槽 ⇒ 新增 `art/shaders/surface.wgsl`
  ＋ `px_render/src/surface.rs` 的 `SurfaceMaterial`；云影 = 按**指定高度** `shadow_height`
  查云覆盖度立方图的解析近似（切向偏置 ＋ 三方向半影），**只压直接光**。
- **顺手修的仪器**（§59.5）：`frame-probe.ps1` 把"取产物路径"排在重烘之前 ⇒ 新场景报"清单里
  没有这个节点"、改过内容的场景**安静地量上一份产物**。已改成一律先重烘再取。

**已验证**：

- `cargo check -p px_render --all-targets` ✅；`cargo test -p px_render` 全绿（含 shader 门：
  三个 shader 解析＋校验＋体量，`clouds.wgsl` 1378 行 HLSL / `surface.wgsl` 552 行，上限 4000）。
- 出图（Vulkan / 2240×1400 / 5 档一批）：`orbit-soft`、`orbit-soft-plain`、`orbit-soft-noshadow`、
  `orbit-soft-nocloudshadow`、`orbit-bare`；**管线 0 失败**（44 → 51 条）。
- 差异带（`target/pixdiff.ps1`，逐像素最大通道差）：只换材质 max 47、云影 max 84（13.4% 像素）、
  shadow map max 229 但只有 0.68% 像素 —— 数字与图都在 `06-clouds.md` §59.3。
- 回归：`orbit` / `orbit-surface` / `orbit-proxy` / `orbit-bare` 重烘重出，都正常（无洋红、无丢云）。
- 代价（Vulkan / GPU p50 / 780×520 / 配对）：软档全开 − 全关 = **+0.31 ms**（轮间抖动同量级，
  只能说"小于 1 ms 量级"）。

**没验证 / 没做**（详见 §59.6）：

1. `--sheet`（12 视角对照图）**没出**；`soft-e300/e6000/e24000` 只重烘了场景、没重出图。
2. `--view` / `--show` 那条路这一轮**没跑**（只用了 `--serve` 出图与 `-Phase stable` 收帧）。
3. 云自己的**曝光**没动：本仓两个自写材质（云、大气）都不乘 `view.exposure`，所以本来就偏亮
   （云的"均匀白"有一部分来自这里）。这次只给新的 surface 材质乘了曝光。
4. 环影（`rings > 0`）只有代码路径，**没有实测图**：这批场景 `rings = 0.0`。
5. 三个探针 bin（`field_dual` / `gradient` / `device`）这一轮仍没跑。

**review 用的图**（都在 `target/`，2240×1400）：`shot4-shot-r1-orbit-soft.png`（全开）、
`shot4-shot-r1-orbit-soft-nocloudshadow.png`（关云影）、`shot4-shot-r1-orbit-soft-noshadow.png`
（关 shadow map）、`shot4-shot-r1-orbit-soft-plain.png`（两个都关）、`shot4-shot-r1-orbit-bare.png`
（无云），以及三张差异图 `pixdiff-material/cloudshadow/shadowmap.png`、晨昏线并排 `crop-limb.png`。

- **这条线已经合进 `v2`**：`a666c13 Merge branch 'wip/field-dual-arbiter' into v2`（117 个文件，
  +22892/−14，零冲突）。`wip/field-dual-arbiter` 已完全被 v2 包含 ⇒ **后续开工在 v2 上**，
  或从 v2 拉新分支；原 worktree `.worktrees/field-dual` 只是个旧址。
- 合并后 CPU 链在 v2 上复验过：`cargo check -p px_protocol -p px_ops -p px_graphs -p px_verify
  --all-targets` ✅ 4.34 s；同四个 crate 的 `cargo test` 全绿。
- ⚠️ **`game` 编译不过 —— 已知，且用户裁决「不管 game」**（2026-09-14）：
  `game/src/project.rs:17` 读 `snapshot.executions`，而 `game` 自己的 `Snapshot` 已把这个读数
  换成 `intake`（`game/src/lib.rs:47` 的注释：「原来这里是 `execution`（执行率），该读数已随
  `distribution` 归一化一起删除；物理活跃度由 `intake` 承担」）。默认 members **含 `game`**
  ⇒ `cargo test` 是红的。要绿用：
  `cargo test -p px_protocol -p px_ops -p px_graphs -p px_verify`。

### 9.1.2 本轮续（同一天，接着 9.1.1 的两条之后）：光源去常量（点光源）＋ 细节风

**用户的两条**：①"不要硬编码 `SUN_DIRECTION`，用通用的光源来处理"（追问定为**先支持点光源、
太阳换点光源**，影一起接，光源做成场景参数）；②"让细节随着 noise 场时间变化而变化，
用 shader 内时间，不要逻辑帧注入"（追问定为**分两层、不同风速**，缺省关）。
口径、实测与踩到的坑全在 `06-clouds.md` **§60** 与 **§61**。

- **光源**：删 `SUN_DIRECTION`；新增 shader 库 `px_render/assets/shaders/light.wgsl`
  （`sun_light()`：先点光源（走 `bevy_pbr::clustered_forward` 的三跳去查 `clustered_lights`），
  没有才退回第 0 盏平行光）；`spawn_lights` 换成 `PointLight`，位置/色/强度进 planet part
  （`light_position` / `light_color` / `light_intensity`，缺省 = 旧平行光的坐标与照度）；
  影子跟着灯的种类走（cube / 级联）；**地表云影那条解析解一个字没改**。
- **细节风**：`billows` 两层各加一份随 `globals.time` 的**有界正弦**偏置（时间尺度写在 WGSL 里、
  两层不同），场景参数 `wind` / `wind_skin`（幅度，缺省 0 = 不动）。偏置存在 `var<private>` 里，
  片段入点开头 `arm_wind()` 设一次 —— 探针的 compute 入点不碰它，所以空 bind group 0/1 照样跑。

**已验证**：

- `cargo test -p px_render` 全绿（含 shader 门：三个 shader，`clouds.wgsl` 1606 行 HLSL /
  `surface.wgsl` 672 行，上限 4000）；`cargo check -p px_render --all-targets` ✅。
- 出图（Vulkan / 2240×1400）：`orbit-soft` / `-wind` / `-nocloudshadow` / `-noshadow` 两轮一批，
  管线 0 失败、没有"等管线超时"。
- 判据（`uv run target/pixdiff.py`，numpy，0.2~0.4 s 一对）：
  - **换光源** 39.08% 像素、p99 45、max 227；**亮度标定**：近天底比值 0.97~0.99（远处 0.70~0.85 = 1/d² 与受光帽收缩）；
  - **云影**（点光源时代）10.89%、p99 35、max 79；**cube 阴影** 1.71%、p99 5、max 231；
  - **细节风**：`orbit-soft-wind` 两轮 13.27%、p99 85、max 222；**对照 `orbit-soft` 两轮 0 像素**。
- `tools/px.ps1 -Target field_dual` **全绿**（新 import 没把探针的空 bind group 0/1 打挂）。

**没验证 / 待办**：

1. `tools/px.ps1 -Target gradient` **有 1 个 check 红着**（`the_residual_is_attributed_to_one_channel`，
   `[简化]` 夹具里解析路径 vs 值路径的噪声值差 2.26e-1，判据要求 < 1e-6）。推理上**不是这次引入的**
   （那条夹具直接调两个这次没改的噪声函数），但没有 stash 基线实测 ⇒ 谁再动噪声库之前先把这条查清（§61.4）。
2. 点光源的**多灯累加 / spot / rect / 灯间遮挡**都没做（现在取"cluster 里最近的一盏"）。
3. `--sheet` 12 视角、`soft-e*` 三个不透明度档、环影（`rings > 0`）这一轮仍没出图。
4. 光源换了 ⇒ `orbit*` 的旧基线像素全作废（要重出再对账）；`soft-e*` 之间互比仍有效。
5. 云与大气**仍不乘 `view.exposure`**（§59.2 末尾那条），这次没动。

**review 用的图**（`target/`，2240×1400）：`wind3-shot-r1-orbit-soft.png`（全开）、
`-nocloudshadow.png`、`-noshadow.png`、`wind3-shot-r1/r2-orbit-soft-wind.png`（两轮 ⇒ 细节在动）、
`light1-shot-r1-orbit-bare.png`（点光源下的裸行星），差异图
`pixdiff-light.png` / `pixdiff-cloudshadow-point.png` / `pixdiff-shadowmap-point.png` / `pixdiff-wind.png`。

### 9.1.3 本轮（2026-09-15，`fix/pipeline-fail-fast` worktree）：坏管线当场拒，不许一直 pending

**用户的两条**：①把 `feature/cloud-surface-perf` 合进 v2；②"server 请求遇到坏管线要提前退出
而不是一直 pending"。追问定下的口径：**失败当场拒绝，不能靠超时**；超预算时**只让这一步请求
失败退出、不终止管线**（再请求一次可以拿到）。

- **① 已合**：那条线的 tip `ffcef4b` 本来就是 v2 的祖先，真正没合的是 worktree 里
  **63 项 / +12581−1356 未提交改动**（全 git 只此一份）⇒ 先落成一个提交 `52298d9`，再
  `--no-ff` 合进 v2 = `f7da895`（零冲突；v2 那处编不过的 `ready.get()` 残留按用户裁决丢弃）。
  合并结果复验：`cargo check -p px_render --all-targets` ✅、六个 CPU crate `--all-targets` ✅、
  `cargo test -p px_render` 与五个 CPU crate 全绿。
- **② 已做**：口径、落点表与实测全在 `08-renderer.md` **§62**。要点：`pipeline_gate` 成了出图前
  唯一的闸（能证明坏就当场拒；只是没编完就有界地等，超预算**不出图**）；`accept_jobs` 在搭场景
  **之前**就查失败明细与 shader 库装载态；等待预算从"全局 `Ticks`"改成"这一步的 `rebuilt_instant`"。

**已验证**（Vulkan / RTX 3060 Laptop / 场景 `orbit-soft`）：坏 shader 库 ⇒ 请求 **1~2 s** 被拒
（点名管线 + naga 原文、不出图）；修好后**同一个服务**再请求成功（357843 字节 /
`de36e672a30b502f…`，没重启）；把预算临时改成 300 ms ⇒ 第一次请求被拒、同服务第二次成功
（管线没被终止）；`cargo test -p px_render` 全绿。

**没验证**：Bevy 那条"无限重试"支路本机造不出来（两种造法都被 naga_oil 放过）⇒ "等超预算"
只有人为把预算改成 300 ms 那一次实测。`drive_stable` 的 `Assets` 相位、viewer（`--view` /
`--show`）没接这道闸。

## 9.2 已经能跑什么

> ⚠ **本节的命令行是旧接口（原文保留）**：内容旗标已在 §52 / P10 删除，内容只走 `--scene`；用法见 `09-instruments.md` §40 与 `08-renderer.md` §52。下面的 `--planet / --mesh / --clouds / …` 只能当历史形状看。

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

**P4｜`tools/frame-probe.ps1` / `probe-clouds.ps1` 的 fail-fast** —— ✅ 已做

实证：`target/fp-*.log` 四个文件 mtime 精确相隔 ~60 s ⇒ 那一次 `NoiseSample` 编译失败
让每个 case 白等满 60 s，而真错误被 `continue` 吞了。现在两个脚本都 dot-source
`tools/harness.ps1`：按**图名 + 节点名**从 `target/pcg/<图>/manifest.json` 解析产物
（缺图/缺节点/产物不在 ⇒ 抛错并列出可选项）、客户端退出码非 0 与 `Refused` 一律硬失败、
按日志的「渲染管线全部就绪」等就绪、按租约 pid 停服务。顺手发现并记进 §51.1 的更深一层：
**六个默认路径全是内容哈希，早就死了**，而脚本吞退出码 ⇒ §46.1 那张表是空转测出来的。

**P5｜`--cloud-ablate` 进 per-request `View`** —— ✅ 协议侧已加（`View.ablate`），
渲染器侧仍读服务级的 `ServerAblate`，等 P10 一起收尾。

> **P5 状态**：✅ 已随 P10 收尾（`ServerAblate` 全删，消融改成 clouds part 的 `ablate`）；原文保留。

**P6｜预热与管线**

只 warm 真正会用到的管线（现在 44 条里混着 `StandardMaterial` / `Skybox` 变体）；
评估 wgpu 的**磁盘管线缓存** —— 它是把「每次重启服务 15–25 s」降到 <1 s 的唯一现成手段。

> **P6 状态**：⚠ §51.19/§51.20 已查实结论 —— **本机不值得做**（DX12 上 no-op；要用只能 fork `bevy_render`），等上游 bevy#19809；原文保留。

**P7｜删死代码**：`atmosphere.rs::sync_cameras` + `AtmosphereParams.camera_x/y/z`
（shader 早就改读 `view.world_position` 了，它却把最后一台相机的世界位置写进**全局** material uniform）。
注意 uniform 布局要和 WGSL 同步改。

**P8｜`tools/probe.ps1` 退休** ⇒ 直接 `px_render --sheet`。删之前先跑一次 9.4 的第 3 步、和旧图并排比一眼。

**P9｜缓存只覆盖了资源的一半**：两个 `ico(64)` 壳（云、大气）、环、`ring_image(1024,4)`
仍是每请求现造 —— 相对那几百万 texel 是零头，但不是零。

**P10｜删掉内容旗标（§52）** —— ✅ 已做（含相机表与批量）

`render::Scene::Planet` 与那串内容旗标（以及 `Options::planet_spec()`、`ServerAblate`）全删；
`--scene` 可给多次，每步的 `--out/--cam` 配在它前面那个 `--scene` 上；
`Scene::Sequence { shots: Vec<Shot> }`（`Shot { scene, out, cam? }`）批量出图，一步一行「出图：」；
`--sheet` 变裸开关、用 `.pxart` 里那 12 台评审相机；消融改成 **clouds part 的参数**
（`ablate = "surface"`），`params.steps` 与新增的 `params.surface_level` 现在真被 WGSL 硬表面路径读
（之前场景里写的 `steps` 对硬表面路是个谎）。`SCHEMA_VERSION 8 → 9`。

⚠ **这一段最值钱的发现**：每请求 `install + reload` 会把管线打回重编，而出图只等 6 帧 ⇒
那一张图上云直接消失，**且退出码 / 颜色 / 图片大小 / 编译 / 单测全绿**。判别只能靠哈希
（三张图逐字节相同）。修法：只在槽内容真变了时才装 + `drive` 等管线入队（有界 40 帧）。详见 §52.3。

**P11｜viewer 走场景** —— ⚠ 只到编译级

`ViewRequest` 已换成「场景产物路径 + 内容键」，窗口只在键变了才重建，`--show` 只推场景路径；
但**没有开窗口实跑**。

> **P11 状态**：viewer 的契约与坑已由 `09-instruments.md` §56 / §56.1 补齐（含"起窗口必须脱离"、判档日志、修复没进 exe 的两个坑）⇒ 本节状态以 §56 为准；原文保留。

**P14｜装过 shader 之后要等 READY**：现在只等"管线入队被看见"（有界 40 帧，超时带警告照常出图）。
如果一份 WGSL 真换了内容，那一张仍可能赌输。要彻底就得等 READY，但要处理"管线永远编不出来"时别把任务挂住。

> **P14 状态**：⚠ v11 起主路径改成"等条件"（管线全部就绪 + 8 渲染帧，见 `09-instruments.md` §57/§58）⇒ 本节"只等入队（有界 40 帧）"是旧形状；原文保留。

**P15｜删死码**：`--scatter` 删掉后 `planet::spawn_scattering`（Bevy `AtmosphereSettings` 那条）与
`planet::check_scene` 没人调了。

**P12｜把改 shader 的近路做回工具层**：引擎那半截（`slots://` 稳定槽 + `reload`）已经在了
（§52.3 查实它就是热重载本体）。缺的是「监视 `art/shaders/*.wgsl` → 重烘 `shaders` +
`scene` → 用新场景路径再请求」这一步；做完就把「改一个字等 1 秒」还回来，而且比原来更硬
（场景键钉住当时用的是哪版 WGSL）。

**P13｜把 §51.4 那三条量完** —— ✅ 已做完（§51.7 + §51.9）

上界早退（`bound = 1`）逐字节不变、中位 30.00 → 18.56 ms（−38%）；
四档归因（§51.9）：**梯度只值 1–2 ms，大头是全 miss 的步进（7.8–13.7 ms）**
⇒ 优化该往"少走步"做，不该往"优化梯度"做。
剩：分辨率扫描（要避开 2240×1400 的驱动天花板）。

> **P13 状态**：⚠ 它引用的 §51.7/§51.9 **已被 §51.12 作废**（泄漏期数）；修后重测见 §51.12/§51.13，且**步数曲线与分辨率扫描仍未重做**；原文保留。

**P16｜`bound` 的读法要严**：现在用 `optional_number(...)? as u32`，`bound = 0.5` 会被截断成 0
而不报错。缺省 0 是对的（老场景没有这个参数），但写了非整数应当报错。

**P18｜云的代理几何接通了（渲染侧）** —— ⚠ 判据差一点 + 撞到一个既有显存泄漏

`art/scene/orbit-proxy.toml`（与 `orbit-surface` 只差 clouds part 多一个 `proxy` 成员）+
装配器按成员取代理 mesh + shader 硬表面分支改成"壳入射点为绝对栅格、代理落点只定起始下标、
可双向"。详见 §51.11。三件事要接着做：

1. **代理 mesh 的缠绕朝里**（有向体积 −0.4709）。现在渲染侧读的时候按有向体积自动翻面
   （`planet.rs::outward_winding`，星球那张 +4.25 不动）。**要么**就这么留着（以后谁修了烘焙侧
   也不会双重翻面），**要么**在 `px_mc` 里把缠绕翻正 —— 后者换内容键，且渲染侧那段自省逻辑
   仍然安全。
2. **逐字节判据没达到**：差 1924 px @480×300 / 12901 px @2240×1400，其中 98% 只差 1–7 个
   通道值。控制实验（球壳只换镶嵌 `ico(64)→ico(60)`）差 1917 px、直方图相当 ⇒ 残差来自
   fragment 落点变了 ⇒ `ray` 末位变 ⇒ 量化翻一位，**不是**搜索逻辑。真要逐字节得把光线改成
   从像素反推（会动到 orbit-surface 的像素）。
3. **2240×1400 长跑必炸：显存线性泄漏 ~150 MiB/s（云档），无云档是平的、480×300 也是平的**
   ⇒ 与像素数成正比、只跟云有关；`orbit`（体积，本段没碰）漏得一样多 ⇒ 既有缺陷，不是代理带来的。
   修掉它之前 `frame-probe` 在 2240×1400 上跑不满 n=8（n=4 可以）。

> **P18 状态**：第 3 条（显存泄漏）**已闭环** —— 修法与验证在 §51.12，根因与上游 issue 文字在 `09-instruments.md` §55；第 1 条（缠绕）仍按渲染侧自动翻面处理，第 2 条（逐字节判据）仍未达到（§51.11）；原文保留。

**P17｜仪器要先热身一张再取判据**：冷启动后第一个请求仍可能丢云壳（装 shader 打回重编的窗口，
§52.3 的同一个坑）。已有一条实测留证（`p13-surface.png` = 无云那张）。
⚠ **补充（本段实测）**：这个窗口比想象的长 —— 装完 shader 之后**同一个批量请求里背靠背的每一步**
都还在窗口里（`p480-orbit-proxy.png` 三张全无云）。**要按"热身一张 + 停顿几秒 + 再出判据图"
做**，不是往同一批里多塞几步。

> **P17 状态**：⚠ v11 起判据侧由 `has_cloud` 兜（`09-instruments.md` §57/§58.1）、"热身 4 s"已退休（§51.18）；本节的手工步骤属旧协议/回退路（`--windows`/`--drop`）的形状；原文保留。

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
| 渲染器怎么跑（离屏 / 服务 / viewer）＋ 资源缓存 | `art/08-renderer.md` |
| 改完怎么验（热重载 / review 回路 / 单帧时间 / 探针） | `art/09-instruments.md` |
| 不变式（每条都付过代价） | `art-framework.md` 顶部 |

代码入口：`px_protocol/src/art.rs`（产物与协议）、`px_ops/src/lib.rs`（算子与缓存键）、
`px_render/src/{main.rs,planet.rs,art_cache.rs,clouds.rs,atmosphere.rs,surface.rs,slots.rs}`、
shader 库 `px_render/assets/shaders/{common,noise,light}.wgsl`（**库住这里**，
入口 shader 住在 `art/shaders/`）、
`px_probe/src/{common.rs,probe.rs,field_dual.rs}`。

**判据图怎么比**：`uv run target/pixdiff.py -A <png> -B <png> -Bands 4,8,20 -Out <diff.png>`
（numpy，313 万像素一对 0.2~0.4 s）。
⚠ 别再用 `target/pixdiff.ps1` 那版 PowerShell 逐像素 —— 同一件事要几分钟，
而"仪器慢"会直接变成"少测几档"（§59.3 注）。

**P19｜用修好的仪器重测 2240×1400（§51.12）**：显存泄漏修掉之后 n=8 才第一次真正可跑，而**泄漏期间量的**那些数（§51.3 步数曲线、§51.7 的 −38%、§51.9 的四档归因）**全部作废** —— 那时打出来的"帧时间"是 CPU 提交速率（~30 fps 的假象），GPU 真实只有 ~16 fps。修后的真实中位：`bare 14.76｜orbit 52.15｜orbit-surface 71.63｜orbit-proxy 49.43｜orbit-bound 44.09 ms`。**优先重测这几条**：
1. 代理 vs 球壳（`orbit-proxy` vs `orbit-surface`）到底省多少；
2. 上界早退（`orbit-bound` vs `orbit-surface`）的收益与逐字节判据；
3. 步数曲线（现在步数是场景参数，好做）；
4. 四档归因（梯度 vs 全 miss 步进）。
⚠ ~~还有一条"没闭环"：为什么偏偏**有云**才把 wgpu 的间接绘制校验抬到约 2 笔/帧~~ —— **已闭环（§55）**：
无云档**也**产生间接绘制（3 笔/帧 vs 有云 4 笔/帧），云不是原因；有云只是**让 GPU 成为瓶颈**，
CPU 于是无限跑在前面，每个在飞提交钉住 wgpu 那对 1 MiB 校验缓冲。上游 issue 文字（含最小复现、
分配点行号、三选修法实测）在 `09-instruments.md` §55.4，**没有真去提交**。
⚠ 顺带更正：§51.12 原来那次"修前后逐字节相同"用的是**两张无云图**（P17 的重编窗口），
已按 P17 重做并留证（480×300 `orbit-surface` = `DF6D5BC1…` / 124433 字节，修前/修后构建相同）。

> **P19 状态**：**1/2/4 已做**（修后重测 = §51.13，另见 §51.12）；**3（步数曲线）未见修后重扫**。⚠ 且 §51.20 判定产品口径是 Vulkan、并注明"DX12 时代按 GPU 毫秒算的百分比改进在 Vulkan 下余量小得多 ⇒ 优化判断需重做" ⇒ 本节的"重测"在 Vulkan 口径下**部分仍待做**；原文保留。

**P20｜清掉「细代理」这条弯路的残留**：`px_mc/src/lib.rs` 里为"几何外扩消轮廓丢片"加的 `pub offset`（默认 `0.0`，老路径逐位不变）+ 310–354 行的外扩块，是给**已排除的 `field=final` 路径**打的补丁 ⇒ 若确定不再回头，删掉它（与 P15 死码一起清）。`art/clouds/proxy_fine.toml` 里的 `offset` 已退回（不再设值）。`art/scene/orbit-proxy-fine*.toml` 两份**留着**当已排除选项的可复现记录（§51.14.1），别当正式档。

> **P20 状态**：§51.14.1 已把该路排除（"不再继续投入"）；本节仍是"若确定不再回头"的条件句，`orbit-proxy-fine*.toml` 按原文留档，`px_mc` 的 `offset` 待清理；原文保留。

**P21｜硬表面云剩下的两笔真实开销**（都不是 mesh 的事，见 §51.14）：
1. **22% 的 fragment 擦面而过** ⇒ 沿线 32 个采样全 ≤ τ，走到栅格末端才放弃（~7.6 ms）。**要先诊断**这 22% 是"代理覆盖了但云确实不在"（正常剔除损耗）还是"云在那儿但点采样漏了峰"（真问题，与 §51.10.3 的 `reach=1` 同族）—— 两者修法完全不同。
2. **命中像素的解析梯度 + 着色 + 透明混合 ~10 ms**（该档 27%，与几何无关）。梯度本身只值 **2.08 ms**（`surface−nograd`）。
