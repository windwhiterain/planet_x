# GLSL 拆成独立文件 + three 的 `#include` 引用关系

## 为什么

GLSL 原先住在 JS 的模板字符串里（`const X = /* glsl */\`...\`;`），于是 **GLSL 注释里任何一个
反引号都会提前终止字符串、把整个 ES module 变成语法错误**（现象：`window.PlanetXMap` 根本
不存在、地图白屏）。这个坑前后踩了**五次**，而且**不是手滑**——是「GLSL 住进 JS 字符串」这个
结构决定的。所以挪进独立文件，让反引号变回普通字符，并**删掉**那个补丁性门禁
`check-glsl-backticks.mjs`。**结构上消灭，而不是继续靠一道门看住。**

## 形状

```
web/static/shaders/px/
  noise/{pars,hash13,vnoise,fbm,ridged,warp,warpT}.glsl   ← 一函数一文件（学 LYGIA 的粒度）
  planet/common.glsl  post/common.glsl
  planet/{planet,atmo,cloud,ring}.{vert,frag}
  sky/{sky,star}.{vert,frag}   sun/{sun,corona}.{vert,frag} + {sun-photo,chromo}.frag
  post/{fullscreen.vert,sunflare.frag,grade.frag}   misc/{orbit,plume}.{vert,frag}
```

JS 侧每个着色器常量只剩一行：`const PLANET_FRAG = INC('px/planet/planet.frag');`
（`INC()` 返回一行 `#include <...>`，GLSL 一个字都不在 JS 里。）

引用关系是**真的一张图**，`check-shaders.mjs` 会把它打出来：

```
px/noise/vnoise.glsl  → hash13
px/noise/fbm.glsl     → pars + vnoise
px/noise/warpT.glsl   → warp → vnoise
px/planet/planet.frag → planet/common + noise/{fbm,ridged,warpT}
```

## 为什么用 three 自带的 `#include <...>`

`WebGLProgram` 在拼前缀之前对顶点/片元各跑一次 `resolveIncludes()`，而 `includeReplacer`
**递归**调用 `resolveIncludes`（three r169 `src/renderers/webgl/WebGLProgram.js`）⇒ 任意层嵌套
都解得开。名称允许 `[\w\d./]+` ⇒ 带路径带扩展名合法。**零新依赖**。

搜索过的其它方案：LYGIA（`#include "path"` + vanilla JS 的 `resolveLygia`，成熟但引它太重，
我们只借了它的**组织粒度**）；glslify / vite-plugin-glsl / esbuild-glsl / webpack-glsl **都要
构建步骤**（本项目刻意没有）；`import x from './a.glsl' with { type: 'text' }` **实测在
Edge 152 不支持**（`"text" is not a valid module type`）。

## 三个坑（都踩了）

1. **three 的解析器没有 include guard、也不去重。** 同一个 chunk 经两条路径进来 ⇒ 重复定义
   函数 ⇒ GLSL 报重定义。所以**每个 .glsl 必须自带 `#ifndef PX_.../#endif`**（全目录如此）。
2. **chunk 名必须带扩展名。** 我第一版把扩展名去掉了，于是 `planet.vert` 和 `planet.frag`
   撞进**同一个 `ShaderChunk` 槽位**（先写后被覆盖）。带扩展名后 `px/planet/planet.frag`
   才是合法且唯一的键。
3. **`init()` 绝对不能改成 async。** 我第一版把 `await loadShaders()` 放在 init 最顶，
   于是连 `scene`/`renderer` 都被推后；app.js 是
   `initMap()`（同步）→ … → `renderMap()`（**拉完 state 之后**）→ `setWorld()` 这样调下来的，
   两条网络请求赛跑、`setWorld` 输了 ⇒ **`bodies=0`、行星一个都不显示**（只剩星云和日冕）。
   ⚠ 我当时的推理只看了 `window.PlanetXMap` 那个竞态，**漏了 init→setWorld 这个**。

## 正确的时序处理（现方案）

`init()` 保持**同步**建场；**只有两步依赖 GLSL**，串在同一个 `loadShaders()` 之后：

- `rebuildSky()` —— `bakeSky` 会**立刻渲染进立方体贴图** ⇒ 当场编译天空着色器
- `tick()` —— 首帧会编译**其余全部**材质

`rebuildSky()` 另加 `shadersReady` 守卫（质量档切换那条路径也走它），所以 ready 之前调用是空操作。

## 门禁（`scripts/check-js.sh`，三道）

1. `check-glsl-manifest.mjs` —— `glsl.js::CHUNKS` 必须与磁盘**逐项相等**。防「新增 .glsl 忘了
   登记」：那种错本地门全绿、页面却报 `Can not resolve #include`。
2. `check-shaders.mjs` —— 镜像 three 的解析语义（含「找不到就抛」和成环检测）后喂 glslang。
   无参数时校验全部 `.vert`/`.frag` 入口。
3. `node --check` —— 每个 ES module / 经典脚本的语法。

---

## ⚠ 两个「验证方法本身是错的」的教训（都是这次踩出来的）

### 1. **计数指标不能验证渲染改动**

太阳「透明」这次，我上一轮宣布「重构无回归」，依据是实机
`bodies=18 / drawCalls=177 / triangles=570940 / programs=23` **与重构前逐项相同**。
但**这些数字完全反映不出着色器编译失败**：网格照样提交、draw call 照样计、三角形照样数，
**只是画不出来**。（当时光球早已因下面的连字符问题编译失败、消失不见，而四个指标一个都没
抖。）**渲染改动的验收必须看图或看像素**，计数只能当辅助。

### 2. **chunk 名里的连字符会让 `#include` 整行不解析**

three 的 include 正则 `/^[ \t]*#include +<([\w\d./]+)>/gm` 的字符类**不含连字符**。
入口 chunk 命名成 `px/sun/sun-photo.frag` 时，那一行**根本没被 `resolveIncludes` 匹配**，
原样喂给 GLSL 编译器：

```
ERROR: 0:69: 'include' : invalid directive name
> 69: #include <px/sun/sun-photo.frag>
```

⇒ 片元编译失败 ⇒ **网格不渲染**。全项目只有那一个文件名带连字符，所以**偏偏只有太阳的光球
消失**，其余 23 个入口全正常 —— 这种「只有一个物体坏掉」的现象极容易被误诊成"那个物体的
着色器逻辑/参数有问题"，而实际上它**连编译都没通过**。

**合法字符只有 `[A-Za-z0-9_./]`。** `check-glsl-manifest.mjs` 现在逐个校验这条
（并做了负向测试：改回带连字符 ⇒ 报错 + exit 1）。

### 顺带的门禁盲区

`check-shaders.mjs` 只拼装 `.glsl` → `.glsl` 的 include，而**入口 chunk 名只出现在 JS 里**，
它从不读 JS ⇒ 上面那个名字它看不见。门禁覆盖的是「文件之间」，漏的是「JS → 文件」这一跳。
