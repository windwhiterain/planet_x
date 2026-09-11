# WebUI 3D 地图渲染质量：光照/尺度/材质/遮挡（用户验收 4 项）

> 状态 `[x]`（`web/static/map3d.js`） ｜ 索引：[notes.md](../notes.md) ｜ 前身：`ideas.md` §22

> 用户报的 4 个问题：①相机像挂了头灯（不真实）②星球太拥挤/太大、互相重叠 ③城市/舰模型太塑料、
> 太花花绿绿，阵营色不该刷在模型上 ④billboard 改成 `depthTest:false` 后被遮挡完全看不出来，
> 空间感没了。全部只改前端（静态文件，改完刷新即生效，无需重编译）。

- `[x]` **① 头灯 = 真 bug：法线空间混用**。顶点着色器 `vNormal = normalize(normalMatrix * normal)`
  得到的是**视图空间**法线（`normalMatrix` = 逆置 transpose(modelViewMatrix)），片元里却与世界空间的
  `lightDir = normalize(-vWorldPos)` 做点积 → 漫反射项无意义。实测（相机垂直于太阳方向看海王星）：
  盘面水平剖面 103/105/105/115 几乎全平，只有屏幕左缘 157；而「纯环境光 0.28」的理论值 luma=109 与
  实测 105 吻合 ⇒ **整个球面 diff≈0，看到的亮度 100% 来自环境光**，加上 `rim=pow(1-dot(n,viewDir),3.5)`
  这个纯视线相关的辉光，就成了「相机头灯」。修法：`vNormal = normalize(mat3(modelMatrix) * normal)`
  （球体等比缩放，直接乘即可）；同时把环境底 0.28 → `TUNING.shaderAmbient`（0.075）、辉光改成
  **太阳驱动**（`uAtmo*rim*(0.06+0.94*lit)`）、漫反射加 `smoothstep(0,0.12,diff)` 柔化晨昏线。
- `[x]` **② 尺度/布局**。旧 `bodyRadius = 2.4*radius + 0.9`（+0.9 定居点加成）导致木星/土星 7.14、
  太阳才 3.2，实测重叠：地球/月球 −1.9、木星/欧罗巴 −6.7、土星/泰坦 −7.2、冥王星/卡戎 −1.1。
  现在：显示半径 = `TUNING.radiusScale(2.2) × config.radius`（**保持 config 的相对比例**，即真实
  太阳系次序与大致比例），太阳 3.2 → 6.4（终于明显最大）；径向压缩 `r^0.5` → `r^0.42`（内太阳系
  半径大约翻倍，外圈略挤）；新增 `computeLayout()`：卫星离母星不足「母星半径(带环按环外缘)+自身半径+
  `moonGap(1.2)`」时沿方位角推出去，并记下缩放系数 `k`，画轨道线时对**整条轨道相对母星缩放同一 k**
  （卫星仍精确落在自己画出的轨道上）；`fitCamera` 改成按**当前天体最远距离 ×1.22** 取景（旧版按最远
  远日点固定 R=110，白白浪费三分之一画面）。结果：seed 42 只剩木星/欧罗巴、冥王星/卡戎 两对
  「刚好等于 moonGap」的紧邻，无重叠。
- `[x]` **③ 材质 + 阵营色上移到 UI**。城市/舰模型一律中性舰船灰 PBR（`roughness 0.62`、`metalness 0.06`），
  几何也换成有结构感的小模型（地面城=穹顶+2~3 塔楼+暖色舱灯；空间站=环+核心+太阳能板；舰=细长船体+
  背鳍+尾喷口，长度按舰级 0.14~0.36 世界单位）。阵营身份改由 UI 表达：**恒定屏幕尺寸的准星环**
  （`reticle` 贴图，`fitWorld` 让近景时它环住模型）+ **标签 chip 名字前的阵营色圆点**（天体是某势力
  首都时，多首都就有多点）。远景 billboard 也改成中性白点，颜色只在环上。
  顺带修掉一个隐藏的颜色空间 bug：`hex2rgb` 直接把 sRGB hex 当**线性**值喂给 `gl_FragColor`，
  而 `outputColorSpace=SRGB` 会再做一次 sRGB 编码 ⇒ 所有行星被免费提亮（0.85 驼色→0.93 近白），
  正是「发灰发糊像塑料」的元凶之一。现在 `hex2rgb` 做 sRGB→linear。
- `[x]` **④ 标记遮挡淡出**。保留 `depthTest:false`（不被球面切边、边缘锐利），但每帧对每个标记做
  **解析式「相机→标记中心」射线 × 天体显示球**求交（≤18 球 × 43 标记，可忽略）：半径方向的
  背面深度 `depth=(r-垂距)/r`，`≤0` 在轮廓外、`0` 正好压在轮廓上、越大越深入球体背面。
  `alpha = 1 - smoothstep(depth, -0.10, 0.06)` —— 连续、无跳变，中心点被挡就淡出。
  顺带把天体标签也从**世界尺寸** sprite 改成恒定屏幕尺寸 + 同样的遮挡淡出（旧版贴近看会变成占满
  全屏的巨型文字），间距按像素算（`labelGapPx`），永远浮在行星上方。
- `[x]` 新增 URL 调试参数（开发用，只在首帧 setWorld 生效一次）：`?focus=<天体>&dist=<世界单位>`、
  `?view=px,py,pz@tx,ty,tz`、`?tune=k:v,...`（覆盖任意 TUNING）、`?hide=labels,markers`；
  配合 `PlanetXMap.tune({...})` / `PlanetXMap.tuning` 可以不改源码刷参数。
- `[x]` **dev 静态服务不发 Cache-Control → 已修**（`web/src/lib.rs::router` + `web/Cargo.toml`）：
  `ServeDir` 只给 `Last-Modified`，浏览器按启发式缓存（10% × (Date−Last-Modified)）把旧的
  `map3d.js`/`app.js` 缓存住，**改了前端刷新却看不到**（两轮排查都被这个坑骗过：一次是
  `fetch('map3d.js')` 拿到旧内容、`PlanetXMap.tune` 不存在；一次是「验证通过」其实是浏览器喂了旧
  `app.js`，害我基于错误的运行时状态写了个假兜底常量）。现在整个 router 压一层
  `SetResponseHeaderLayer::overriding(CACHE_CONTROL, "no-cache")`（`tower-http` 加 `set-header`
  feature）：仍是「可缓存」，但**用前必须回服务器问一句**，命中就是 304（静态文件照旧带
  `Last-Modified`），代价可忽略。实测：`/`、`/app.js`、`/map3d.js`、`/jsonview.js`、`/style.css`、
  `/api/state` 全部 `Cache-Control: no-cache`；在全新 origin 上加载页面 → 改一行 `app.js` → **普通刷新
  （不带任何 cache-bust）即拿到新内容**。注意：**已经**被旧规则缓存住的条目仍会撑到自己过期，
  那之后就一直对了（实在遇到就硬刷新一次）。
- `[ ]` **同屏 9 艘舰停在同一城时标记会叠成一团**（seed 42 的灶神星）：可做屏幕空间去重/聚合成
  「×9」徽标，或按势力分扇区摆开。本轮没做（VFX 重构那轮也没做）。
- `[x]` **太阳表面还是有点「奶酪」感** → **已解决**（`feature/web-vfx-render`）。根因有两条：
  ① `ridged(p*26)` 的特征尺度是 `1/26 rad`，在 350px 的日面上 ≈ **31 px** —— 那不是米粒，是海绵孔，
  现在改成 `p*95`（照 `scratch/ref/sun-granulation.jpg` 定标）；
  ② `sunIntensity` 原来是 **7.0**，而 ACES 在 ~5.8 之上就把一切压成纯白，米粒的 ±20% 动态范围
  全被压平 ⇒ 现在压到 **1.9**，让颗粒落在 ACES 的肩部以内，辉光交给 bloom。
  边缘变暗早就有（`1-0.62(1-μ)-0.18(1-μ)²`），只是之前被过曝盖住了。
- ➡ **整个 3D 渲染层已在 `feature/web-vfx-render` 重写为高端 VFX 管线**（程序化星空/银河、
  太阳光球+日冕、行星地形/云/大气壳/环影、小行星带、HDR 后处理、质量档 + 自适应分辨率）。
  实现细节、**以及那一轮把时间烧掉的三个坑**（GLSL 模板里的反引号 / 着色器编译失败=物体不渲染 /
  浏览器默认跑在核显上）见 **[`web-vfx-pipeline.md`](web-vfx-pipeline.md)**。
  旧文件 `web/static/map3d.js` 已删，换成 `web/static/map3d/` 下 8 个 ES module；
  `window.PlanetXMap` 的公开契约**未变**。
