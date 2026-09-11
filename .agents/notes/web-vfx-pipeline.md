# WebUI 3D 的 VFX 重构：程序化星空 / 太阳 / 行星 / 后处理

> 状态 `[x]`（`web/static/map3d/`，分支 `feature/web-vfx-render`）｜ 索引：[notes.md](../notes.md)
> ｜ 前身：[`webui-3d-rendering.md`](webui-3d-rendering.md)（那一轮的 4 项验收仍在生效）

目标（用户原话）：**「优化提升 three.js 的渲染，要求使用高端 VFX，追求真实/科幻/震撼细节，
可以重构，可以加依赖，但模型一定要程序生成，不要下载网上的。在单独的 worktree 做」**。
四个设计分叉由用户授权「按你的审美来」，最终裁决：小倾角 ≤8°、电影感克制（bloom + god rays +
极轻颗粒 + 极轻色差，**不做桶形畸变**）、three.js 仍走 CDN importmap、**加**小行星带 / 柯伊伯带。

**硬约束：一切资产程序生成**（无贴图、无模型、无网络资源）。库走 CDN 是被允许的
（「可以加依赖」）。改动**只在 `web/static/`**，引擎一行没动。

---

## 0. 先看这一节：三个把时间烧掉的坑

这一轮 90% 的调试时间花在三件事上，下次直接照做能省掉：

### 0.1 GLSL 写在 JS 模板字符串里 ⇒ 注释里的一个反引号会毁掉整个模块

`const X = /* glsl */\`...\`;` 这种写法里，**GLSL 注释中出现反引号会提前终止模板字符串**。
现象不是「着色器编译失败」，而是 **整个 ES module 变成语法错误**：`window.PlanetXMap` 根本不存在，
控制台只给一句离现场很远的 `Unexpected identifier 'FBM_OCT'`。本轮在 6 个文件上踩了 6 次。

⇒ 已固化成门：**`scripts/check-glsl-backticks.mjs`**（`scripts/check-js.sh` 会先跑它）。
它对 `/* glsl */\`` 块做专门扫描，一次报全（`node --check` 只报第一个）。
**约定：GLSL 注释里不要用反引号**，要引术语就直接写 `normalMatrix`。

### 0.2 着色器「编译失败」= 那个物体**完全不渲染**，而且只有控制台一行字

`vec3 + vec2`、`abs(vec3)` 赋给 `vec2`、片元里引用 `modelMatrix`（顶点着色器才有的内置量）
——这些都会让**整个 program 编译失败**，于是整颗行星/整片星空什么都不画。
本轮开头「天空全黑 + 行星不见」就是这个，查了很久才去读 console。

⇒ 已固化成门：**`scripts/check-shaders.mjs`**（+ `scripts/check-js.sh` 调用）。
它复刻 three.js 给 `ShaderMaterial` 注入的前缀（`#version 300 es` + 内置 uniform 声明，
注意注入是**不对称**的：`modelMatrix/modelViewMatrix/projectionMatrix/normalMatrix` 只在**顶点**阶段，
`viewMatrix/cameraPosition/isOrthographic` 两阶段都有），再用 glslang 编一遍。
当前 **24/24 个 stage 全过**。glslang 是可选外部工具（`scratch/glsl-tools/`，不在版本库里），
缺了脚本自己跳过。

**验证 GLSL 的姿势**：`#version 300 es`、`glslang -S frag|vert`、**不要**加
`-G/-V/--target-env/--client opengl100`（那些要生成 SPIR-V，而 ES 3.00 不被接受）。
退出码可靠（0 干净 / 2 编译错），但诊断走 **stdout**。

### 0.3 测量帧率时，浏览器跑在哪块 GPU 上比什么都重要

**这台机器有两块 GPU**：`NVIDIA GeForce RTX 3060 Laptop GPU` + `Intel Iris Xe`。
浏览器默认被 Windows 分到**核显**上，于是所有性能结论都是错的：

| 1280×800 全档 | Iris Xe | RTX 3060 |
|---|---|---|
| `ultra`（58 万三角形 + 全后处理） | ~35 ms（29 fps） | **~6.2 ms（162 fps）** |
| `minimal` | ~16.5 ms | ~6 ms |

RTX 上「分辨率扫到 2560×1440 还是 6.5 ms」⇒ 巨大余量；而且默认档位探测现在会正确选中 `ultra`。

**怎么切过去**（已经做了，可复现）：给 Edge 写一条 per-app 偏好（等同 Windows 设置里的
「图形性能首选项 → 高性能」）：

```powershell
$k='HKCU:\Software\Microsoft\DirectX\UserGpuPreferences'
Set-ItemProperty -Path $k -Name 'C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe' -Value 'GpuPreference=2;'
# 2 = 高性能，1 = 省电，0 = 交给 Windows
```
改完**必须重启浏览器进程**（已在跑的不会换）。验证：
```js
canvas.getContext('webgl2').getExtension('WEBGL_debug_renderer_info')
  → g.getParameter(d.UNMASKED_RENDERER_WEBGL)
```
再看 `PlanetXMap.debug().gpu`。

> ⚠ 悬而未决：这只是**开发机**的偏好。真实用户如果只有核显，走的是 `high` + 自适应降分辨率
> 那条路（见 §3），这条路径必须继续可跑——**不要在核显上跑不动**。

---

## 1. 模块切分（`web/static/map3d/`）

旧的单文件 `web/static/map3d.js` 已删，换成 8 个 ES module（`index.html` 用
`<script type="module" src="map3d/index.js">` 引入）：

| 文件 | 职责 |
|---|---|
| `tuning.js` | 所有可调数值 + **质量档表** + `detectTier()` / `parseTier()` |
| `util.js` | 颜色空间、确定性哈希（FNV-1a + splitmix32）、**共享 GLSL 片段**、canvas 贴图 |
| `kinds.js` | 天体类型 → 程序化外观规格（颜色/大气/环/云量/环带…） |
| `layout.js` | 半径压缩、倾角（≤8°）、卫星让位、开普勒位置、轨道采样 |
| `sky.js` | 银河+星云（烘 cubemap）／恒星（`Points` 几何）——**见 §2.1** |
| `sun.js` | 光球（颗粒/黑子/临边变暗）、色球、日冕+日珥、弥散光晕 |
| `planet.js` | 地形/分析式法线/海洋/云/夜面城市灯/大气壳/星环投影 |
| `models.js` | 程序化舰船 / 地面城市 / 空间站 / 小行星带（`InstancedMesh`） |
| `markers.js` | 标记与标签（恒定屏幕尺寸 + 解析式遮挡淡出） |
| `postfx.js` | EffectComposer 管线（bloom / god rays / flare / grade） |
| `index.js` | 公开 API `window.PlanetXMap` + 世界构建 + 过渡 + 拾取 + 自适应分辨率 |

`window.PlanetXMap` 的**契约没变**：`init / setWorld / resetView / setView / getView /
bodyPoint / focusBody / tune / tuning / setQuality / quality / select / debug`
（`app.js` 只依赖这些；`scratch/interact.mjs` 会逐条验）。

---

## 2. 关键实现决定

### 2.1 星空**拆成两半**——这是本轮最重要的一个决定

第一版把**所有**东西（恒星 + 银河 + 星云）都烘进一张 HDR cubemap，结果是**一整片白色方块雪花**。
根因是**分辨率语义对不上**：恒星必须落在「约 1 个屏幕像素」上，而 cubemap 是**固定角度分辨率**的
贴图——768/面 覆盖 90°，在 50° 视场 800px 高的屏幕上等价于每面 1440px，也就是被**放大 1.9 倍**，
烘进去的星全变成一坨，还暴露出立方体的纹素。

现在：
- **银河 / 星云 → 烘 cubemap**（`bakeSky`，512/面就够，它们是低频的）。用 `CubeCamera` 渲 6 面，
  烘制期间**必须关掉 `renderer.toneMapping`**（否则 HDR 星空被 ACES 压一次、后处理再压一次）。
  贴图 `colorSpace = LinearSRGBColorSpace`（shader 写出的已是线性 HDR）。
- **恒星 → 真的几何**（`createStarfield`，一个 `THREE.Points`，一次 draw call）。
  `gl_PointSize` 直接以**帧缓冲像素**为单位，缩放到哪都是 1–4 px 的锐利星点，没有立方体放大问题。
  颜色走**黑体色温**，亮度走幂律 `pow(rnd, 5)`（绝大多数暗、极少数亮），最亮那批画十字衍射。
  44% 的星集中在银道面附近（三次均匀相加 ≈ 高斯），于是星自己就描出了银河的形状。
  `uPixelRatio` 抵消 DPR，HiDPI 下星点不会变粗。

### 2.2 后处理管线（`postfx.js`）

```
RenderPass(HDR HalfFloat RT, samples:4) → UnrealBloomPass → SunFlarePass → OutputPass → GradePass → [FXAA?]
```
- `renderer.toneMapping = ACESFilmic`，但**只有 `OutputPass` 那一步真的做色调映射**
  （three 只在 `currentRenderTarget === null` 时应用）⇒ 场景渲进 HDR target 时是**未色调映射**的线性值，
  这正是「太阳可以 >1 从而喂 bloom」的前提。
- MSAA 靠 render target 的 `samples` 传递：`EffectComposer` 内部 `renderTarget.clone()`，
  而 `WebGLRenderTarget.copy` 会带上 `samples`，所以 MSAA 4× 能活到 HDR pass。
  `msaa === 0` 的档位才挂 `FXAAShader`。
- `SunFlarePass`：① 沿「本像素→太阳」的径向散射（god rays）② 各向异性拉丝（变形宽银幕那道蓝线）
  ③ 程序化 `aperture()` 光圈核做的 6 个镜头鬼影。
- `GradePass`：色差 + cos⁴ 暗角 + luma 加权的极轻颗粒（在 sRGB 里做）。

**这一块踩过的坑（都会重犯）**：
1. **不要用 `s * luma(s)` 当散射权重**——那是把亮度**平方**，HDR 下日面 ~7 ⇒ 每次采样 49，
   累积后整幅画面洗成一片白。正确做法是**阈值掩码** `smoothstep(uThreshold, uThreshold*3, luma(s))`。
2. **拉丝必须乘一条横向包络**。采样是锚在**太阳的屏幕位置**上做的，于是整行的 `st` 是同一个值
   ⇒ 不乘 `exp(-|Δx|·k)` 会画出一条**贯穿全屏的亮带**。
3. **径向散射本身给不出径向梯度**：每个像素的采样点分布是一样的（`delta` 是按「追到太阳为止」的
   整段路算的）。屏幕上的径向渐变来自 `exp(-d2s * 3.1)` 这条**包络**，不是来自采样几何。
4. **uniform 是在 `createPostFX` 时从 `TUNING` 拷进去的**。`PlanetXMap.tune({...})` 改 `TUNING`
   对已建好的 pass **毫无影响** ⇒ 曾经做过一次「关掉 god rays，画面没变」的**无效 A/B 对照**。
   现在 `tune()` 会调 `post.applyTuning()` **和** `post.setTier(tier)`（前者灌数值、后者管
   「这个 pass 开不开」，两个都要）。
5. 关闭某条射线时要在 shader 里**真的跳过循环**（`if (uGodStrength > 0.0) {...}`），
   而不是把结果乘 0 —— 否则「关掉体积光」一点都不变快。

### 2.3 日冕是**幂律**，而且要早退

`fall = pow(R/r, 3.6) * smoothstep(1.0, 0.72, r)`。不能用 `pow(1-r, k)` 那种「到边缘才归零」的
浅包络：日冕 billboard 的半径是 4 个太阳半径，相机贴近行星时它**铺满全屏**，浅包络会让整个画面
蒙上一层灰。另外幂律段要在 main() 开头先算、`fall * uIntensity < 0.0025` 就 `discard`，
再跑昂贵的 fbm/ridged —— **集显上这一条就吃掉一半帧时间**。远处的弥散光晕用
`#define CORONA_SIMPLE` 走无噪声分支。

### 2.4 大气壳：弦长 × 切点密度

第一版写的是「壳面顶点到球心的距离 h，`thickness = pow(1-h, 0.6)`」——壳上**每个**顶点到球心
的距离都**正好等于** Rs，于是 `h≡1`、`thickness≡0`，**整层大气一点都没渲染**（现象是行星边缘
一圈死黑，且怎么调 `uDensity` 都没反应）。现在做的是最经典的解析近似：

```
① 视线到球心的垂距 b（碰撞参数）
② 视线在壳内的弦长 = sqrt(Rs²-b²)；b < Rp 时后半被行星挡住，只剩 sqrt(Rs²-b²)-sqrt(Rp²-b²)
③ 密度取**切点高度**上的指数分布 rho = (b<Rp) ? 1 : exp(-(b-Rp)/H)
④ od = 弦长 × rho，照明用**切点**法线（不是壳面那点的法线）
```
弦最长 / 切点最低正好都发生在 `b≈Rp`（临边），两者相乘自动给出「贴着实心的一圈亮环 + 盘面上
很淡的雾」。材质用 `side: FrontSide`（双面会在 b>Rp 处让前后壳投到同一批像素、亮度翻倍）。

### 2.5 地形法线的幅度

`nS = normalize(ds - bump * uRelief * 0.055 * flat_)`。原来是 `0.34`，把坡度推到 ~55°——
行星看起来像**揉皱的纸**。有限差分 `(h1-h)/e` 里 `e` 很小，梯度量级是 ~5，系数量级要对得上。

### 2.6 夜面城市灯的角半径

`core = smoothstep(0.99980, 0.99995, ang)`、`glow = smoothstep(0.9980, 0.99980, ang) * 0.45`。
**角半径由 `ang > cosθ` 反推**：`0.99980 ⇒ θ≈1.1°`。第一版用了 `0.99965/0.99995`，
核太小、加上增益不够，**肉眼完全看不见**（一度怀疑 uniform 没绑上，还为此做了一次
「把半径放大到 0.98」的诊断——那种诊断手法值得留用：**先证明机制通不通，再调参数**）。
`uCityDirs[CITY_MAX]` 由 `buildCities` 填（`layout` 节点的 `cityUniform` 引用）。

### 2.7 轨道线在近景要淡出

相机钻进某个行星系时，**卫星的轨道环会横穿母星的脸**，读起来像星球上一道白色划痕
（additive 细线压在亮盘面上格外显眼）。判据：`相机到该轨道中心的距离 / 轨道半径`，
`smoothstep(ratio, 1.25, 3.6)` 淡到 6%。

### 2.8 小行星带的尺寸是个折中

岩石半径原来 0.17–0.5 世界单位，而行星显示半径才 2.2 ⇒ 「小行星」有**地球半径的 23%**，
凑近看全是巨石阵。真实比例下小行星当然看不见，所以折中到 **0.050**（主带）/ **0.070**（柯伊伯）：
系统视角下约 **1.7 px**（读起来是尘埃带），凑近才是几块小石头。数量补到 2400 / 1300。

---

## 3. 质量档与自适应分辨率

`minimal | low | medium | high | ultra`，`detectTier()` 读 `WEBGL_debug_renderer_info` 的
渲染器串：**独显**（`NVIDIA|AMD|Radeon|RTX|GTX`）在高分辨率下给 `ultra`，核显（`Intel|UHD|Iris|Apple`）
给 `high`，`SwiftShader|Software` 给 `minimal`。`?q=<档>` 强制。

自适应分辨率：`PERF = { slowMs: 26, fastMs: 12.5, minScale: 0.62, cooldownMs: 1400 }`，
跳过前 90 帧（着色器编译/首帧抖动），帧时间超了降 DPR、回到余量里慢慢加回来。
**帧时间统计必须始终在跑**（它同时是 `debug().fps` 的来源）——曾经把统计写在 `adaptive()` 里、
被自适应开关一起关掉，于是「关掉自适应就再也测不到帧率，`frameMs` 恒为 0」。
`TUNING.adaptive` / `adaptiveMinScale` / `postfx` 都是逃生门（`?tune=adaptive:0` 等）。

---

## 4. 工具（`scripts/` 与本轮新增）

| 工具 | 作用 |
|---|---|
| `scripts/check-js.sh` | **合流门**：GLSL 反引号哨兵 → 全文件语法检查 → glslang 编译全部 24 个 stage |
| `scripts/check-glsl-backticks.mjs` | 见 §0.1 |
| `scripts/check-shaders.mjs` | 见 §0.2（glslang 缺失时自跳过） |
| `scratch/shot.mjs` | CDP headless 截图 + 任意 JS eval |
| `scratch/perf.mjs` | 帧时间探针（跑固定帧数取中位数/分位数，可指定视口尺寸） |
| `scratch/interact.mjs` | 交互回归：公开 API / 拾取 / select / advance+过渡 / 新局 / 切档 / 零报错 |
| `scratch/ref/` | 18 张**参考图**（NASA/ESA/Wikimedia，仅作视觉标定，**不进构建、不是资产**）+ `INDEX.md` |

**截图/测量一律用 `scratch/*.mjs` 走 `127.0.0.1:9333` 的 headless Edge**，
不要用 DSH 的共享 `browser_*`——那个浏览器会被**别的会话**导航走（本轮就被导到
`http://127.0.0.1:3002/` 打断过两次）。启动：

```bash
"/c/Program Files (x86)/Microsoft/Edge/Application/msedge.exe" --headless=new \
  --remote-debugging-port=9333 --user-data-dir=/c/resource/planet_x-vfx/scratch/.edge-profile2 \
  --window-size=1280,800 --no-first-run about:blank &
```
`--headless=new` 下**用的是真 GPU**（不是 SwiftShader），所以截图与帧率都有代表性。

---

## 5. 参考图怎么用（以及**不能**怎么用）

`scratch/ref/` 的 18 张图只用于**标定**（对比度、色相、结构尺度），例如：
- `earth-blue-marble.jpg` 直接推翻了第一版的配色——真实地球是**接近黑的深蓝洋面** +
  **橄榄/土黄**陆地 + 极亮的白云，而第一版是「糖绿 + 浅蓝」。据此改了 `terranColor`、
  收窄大陆架过渡、把云的不透明度提到 0.97。
- 米粒组织的尺度也是照 `sun-granulation.jpg` 定的：`p*26` ⇒ 特征尺度 1/26 rad ≈ 31 px，
  那是海绵孔不是米粒；改成 `p*95`。

**一条都不能进构建产物**（用户约束是「模型一定要程序生成」，参考图只是给人看的）。
它们放在 `scratch/` 下（已被 `.gitignore` 覆盖）。

---

## 6. 未做 / 待裁决

- `[ ]` **WebGPU 迁移**：查证结论是 `WebGPURenderer` **不支持裸 GLSL `ShaderMaterial`**，
  必须改用 TSL，且 `EffectComposer`/`UnrealBloomPass` 在 WebGPU 下**不存在**（后处理要整条重写）。
  本轮 24 个 stage 的 GLSL + 整条后处理管线都押在 WebGL 上，迁移等于重做。
  而我们的瓶颈从来不是 draw call / CPU 提交（174 calls / 58 万三角形），是**填充率**——
  WebGPU 的招牌收益（低 CPU 开销、实例化、compute）对不上。
  **若要做**：先在新分支起一个 spike，只迁星空+后处理，量一遍再说；不要在主线上试。
- `[ ]` **把 `scratch/perf.mjs` 与 `scratch/interact.mjs` 提升进 `scripts/`** 并接进
  `check-js.sh`（需要先固定一个 headless 浏览器端口的启动方式）。
- `[ ]` **星云/银河还可以更「亮」**：现在偏灰，参考 `milkyway-core.jpg` 应该有更亮的恒星云团
  与更黑更锐的暗尘带。
- `[ ]` 同屏多舰标记叠成一团（前一篇的遗留项，仍未做）。
- `[ ]` 云层是**静态**的（`uTime` 只进了 shader，但天体自转没有让云层独立漂移）；
  真实地球的云比地面转得快。值得做。

---

## 太阳的「耿直」——色球从**壳体**改成体积（用户：「弄得像日珥一点」）

### 病灶不是参数，是**几何**

色球原先是 `SphereGeometry(R * 1.012)` 的一层壳 + `chromo.frag` 的 `pow(1-μ, 3.4)` 临边项。
**壳在几何上没有径向厚度**：它的外缘永远等于轮廓线，所以无论怎么调噪声，读出来都只能是
**一圈等半径的均匀硬红环**。而日珥的本质恰恰是径向**长短不一**的针与弧 —— 那需要体积。

顺带一个真的 bug：旧 `chromo.frag` 的针状体是 `ridged(p * vec3(38,38,9))`，那是**在对象空间
的某一根固定轴上压扁** ⇒ 所有"针"都朝同一个方向、与日面法线无关，转到侧面就露馅。
正确的做法是在**局部正交基**（u=径向, t1/t2=切向）里各向异性采样：径向频率低、切向高。

### 改法

1. **删掉色球壳**（`chromo.frag` 连同清单项一起删），把它的贡献并进 `corona.frag` 的密度场：
   + `base`（贴面薄层，σ=0.06R，高频各向异性 `vnoise` 折出针）
   + `arcade`（贴面弧道，σ=0.13R）
   + `filament`（原有）+ `fp`（足点低频，把弧带切成一束束）→ 加上**切向剪切**随时间扭动。
2. 颜色沿半径走三段：`rr≈1` **深红（色球）** → 暖白（K 日冕）→ 蓝白（外冕）。
3. `sun.js`：`parts = [photosphere, corona]`，删 `chromoMat` / `chromosphere` / 档位开关 /
   `uTime` 同步。**少一个 draw call、少一次着色器编译。**

### ⚠ 两条量出来的教训

- **每一层都必须"薄"**。我第一版给了 `base` σ=0.17R、`arcade` σ=0.42R —— 那都是"厚壳"，
  正穿日面时路径依然很长（中心射线在日冕里要走 ~3R），整张日面被红洗成暗红盘。
  真色球在日面上几乎透明、只在临边亮，这一点是**几何自带**的：薄壳正穿路径短、掠射路径长。
  σ=0.06R / 0.13R 时正穿:掠射 ≈ 1:8，才读得出"临边一圈红"。
- **别信缩略图上的"暗"**。缩略图里日面看着是暗褐盘，实测像素却是 **RGB(236,222,210)**（近白）。
  那是被周围红光衬出来的对比错觉。**开/关日冕做 A/B 实测**：日冕开 236 / 关 224 ——
  日冕不但没压暗日面，还提亮了 12 级。剖面：日面内 224→152、出边缘 70（日冕柔光）、暗侧 27→70。
