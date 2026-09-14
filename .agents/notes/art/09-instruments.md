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
