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
