# 着色器编译把浏览器卡死：`#define` 常量上界 ⇒ 循环全展开

> 状态 `[x]`（`web/static/map3d/util.js` + `planet.js` / `sky.js` / `sun.js`，分支
> `feature/web-vfx-shader-compile`）｜ 索引：[notes.md](../notes.md)
> ｜ 前身：[`web-vfx-pipeline.md`](web-vfx-pipeline.md)（VFX 重构本身）

用户原话：**「我用我的默认浏览器打开这个网页它好像选择的是集成显卡，然后就卡死了」**、
**「有UI，没有太阳系的渲染」**、**「在这个VFX feature 之前是好的」**、**「不inline才是关键」**。

## 1. 现象与误判

用户报「用了核显 + 卡死」。**这个归因是错的，而且我一开始也跟着错了**：

| 观察 | 真实解释 |
| --- | --- |
| NVIDIA 监控 `GPU 0%` | 编译在 **CPU** 上跑，GPU 本来就没事干 |
| `CPU 40%` | D3D 编译器（`fxc`）在烧 CPU |
| 「有 UI，没有太阳系」 | DOM 早就画完了；canvas 永远等不到第一帧 |
| 「开另一个 Edge 也把所有 Edge 卡死」 | **同一个 GPU 进程被编译占住**，所有标签页一起等 |
| 「选的是集显」 | **不成立**：`nvidia-smi` 显示 Edge 的 GPU 进程就在 RTX 3060 上 |

`nvidia-smi` / 注册表 `GpuPreference=2` 都证明独显路径是通的（详见 §5）。

## 2. 根因

`NOISE_GLSL` 里的 fbm/ridged 用 **`#define FBM_OCT`**（由 `tier.oct` 注入）当循环上界。
常量上界 ⇒ 编译期**完全展开**。而 `planet.js` 的片元里有：

* **17 处 `fbm(`** 调用点
* **4 处 `warp(`**，每处内部 **3 次 `fbm`**（= 12 处）
* **1 处 `ridged(`**
* 每次 `fbm` 迭代内联一个 3D `vnoise` = **8 次 `hash13`**

⇒ `oct=7`（ultra）时是**两百多个内联 `vnoise`**（上千个 `hash13`）。`fxc` 的优化器在这个
规模上退化到近乎跑不完。

**冷着色器缓存下「第 1 帧 → 第 2 帧」的实测停顿**（`scratch/diag.mjs`，每次擦掉
`ShaderCache`/`GPUCache`/`GrShaderCache`）：

| 档 | `oct` | 修复前 | 修复后 |
| --- | --- | --- | --- |
| `minimal` | 2 | 13.2 s | 5.5 s |
| `low` | 3 | 30.9 s | 5.5 s |
| `medium` | 4 | 挂死 | 5.2 s |
| `high` | 5 | **永不结束** | 7.1 s |
| `ultra` | 7 | **永不结束** | 6–14 s（抖动大） |

修复前停顿**随 `oct` 爆炸**；修复后**基本与 `oct` 无关**——这正是「展开被消除」的特征。

## 3. 修法（用户裁决：**不 inline 才是关键**）

把八度数从 `#define` 常量改成 **uniform**：

```glsl
uniform int uFbmOct;                       // NOISE_GLSL 里声明一次
for (int i = 0; i < uFbmOct; i++) { ... }  // fbm 与 ridged
```

上界不再是编译期常量 ⇒ 循环体只出现一次，不能展开。GLSL ES 3.00 允许非常量上界。
每个用到 `NOISE_GLSL` 的材质都必须挂 `fbmOct(n)`（`util.js` 导出），**漏挂 = uniform 默认
0 = 噪声全 0 = 星球变平**（静默失败，见 §6）。当前 7 个模板 ↔ 7 个赋值点，一一对应。

**顺带的好处**：`sun.js` 的 `setTier` 以前改 `defines` + `needsUpdate`（**每次换档重编太阳
shader**），现在只改一个数值，不再触发重编译。

## 4. 仍然残留的冷编译（未解决）

修复后 `ultra` 冷缓存仍要 **6–14 s**（同配置重复测量抖动很大：6.1 / 9.6 / 10.5 / 13.7 /
19.8 s）。**热缓存只要 1.19 s** ⇒ 这 10 倍差距证明残留**仍是编译**，不是无头浏览器的伪影。

残留的来源已经**不是** `oct`，而是**固定的内联规模**：每个 program 仍有 ~17 份 `vnoise`
内联副本 × 24 个 program。下一步可选（**未做，待裁决**）：

1. **砍 `fbm` 调用点**（②）——17 处里一批是 relief 法线的有限差分，线性地砍编译量；
2. **把噪声烘成贴图**——shader 退化成纹理采样，编译量与运行成本一起塌；
3. 兜底：给 `oct` 定实测安全上限。

## 5. 顺带查清的两件事（与本次 bug 无关，但别再误判）

* **独显选择是通的**：默认浏览器是 Edge，注册表
  `HKCU\Software\Microsoft\DirectX\UserGpuPreferences` → `msedge.exe = GpuPreference=2`，
  且 `nvidia-smi` 里 Edge 的 GPU 进程确实挂在 RTX 3060 上。内核/独显的**内屏接在 Intel
  上**（Optimus）⇒ **任务管理器里 Intel GPU 永远有占用**，这不代表页面跑在核显上；要确认
  看 `edge://gpu` 的 `GL_RENDERER`。
* **没有「按网址选显卡」这种东西**：浏览器只在**进程启动时**选一次卡；`powerPreference:
  'high-performance'`（`index.js` 里早就设了）在 Windows 上基本是空转（只在 macOS 双卡上
  真生效）。粒度是 per-exe / per-快捷方式，不是 per-URL。
* 排查中清掉了一个**我自己上一轮留下的无头 Edge**（profile `.edge-profile2`、CDP 9333），
  它一直以 ~160fps 渲染同一页，把 GPU 顶到 **76% / 86 °C / 68 W**（清掉后 19% / 82 °C /
  37.8 W）。**收尾没做干净会伪装成用户的 bug。**

## 6. 方法教训（比 bug 本身更值钱）

**我上一轮报的「163 fps / 24 programs 全部通过」是假的通过。** 探针复用了持久化的
`.edge-profile2` ⇒ **着色器磁盘缓存是热的**，量到的永远是「第二次运行」。**任何新的渲染
改动都必须用冷缓存量一次**，否则「第一次打开」这条路径从来没被验证过——

量具：`scratch/diag.mjs`（新增，值得提升进 `scripts/`）：

* 每次 **新建 profile**（默认），冷缓存天然成立；`PROFILE_DIR` 固定时才复用；
* 注入启动记录器（`DOMContentLoaded` / `load` / `PlanetXMap-ready` / 每帧 + heap），
  日志是**流式**的 ⇒ **主线程一卡住日志就停**，停在第几帧直接区分「活着但慢」和「死了」；
* `Runtime.evaluate` 超时后改用 **`Debugger.pause` 抢栈** —— 抢不到 JS 栈就说明卡在
  **原生驱动调用**里（本次正是如此），这是「是 JS 死循环还是驱动停顿」的判据。
  ⚠ 两个 `send()` 都必须带超时，否则脚本自己会被拖死（踩过）。
