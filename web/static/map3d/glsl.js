// 行星X WebUI — GLSL 分文件加载。
//
// **为什么 GLSL 不再住在 JS 模板字符串里**：模板字符串会把 GLSL 里任何一个反引号当成
// 结束符，直接截断整个 ES 模块（症状是 SyntaxError + 整块地图不显示）。这个坑前后踩了
// 五次，而它**不是手滑**——是「GLSL 住进 JS 字符串」这个结构决定的。挪进独立文件后，
// 反引号只是普通字符，`check-glsl-backticks.mjs` 这个补丁性门禁也一并删掉了。
//
// 引用关系走 **three.js 自带的 `#include <chunk>`**：`WebGLProgram` 在拼接前缀之前会对
// 顶点/片元各跑一次 `resolveIncludes()`，而且 `includeReplacer` 会**递归**处理
// （见 three r169 `src/renderers/webgl/WebGLProgram.js`），所以任意层嵌套都解得开。
// 名称允许 `[\w\d./]+`，所以 `px/planet/planet.frag` 这种带路径带扩展名的名字合法。
//
// ⚠ **three 的解析器没有 include guard、也不去重**：同一个 chunk 经两条路径进来就会
// 重复定义函数（GLSL 报重定义）。所以每个 .glsl 文件都必须自带 `#ifndef PX_... / #endif`
// 守卫 —— 那也正是本目录里每个文件头尾那两行的由来。
//
// ⚠ 命名空间前缀 `px/` 是必须的：`ShaderChunk` 是 three 的**全局**注册表，直接占用
// `common` / `noise` 这类通用名会和 three 自带的 chunk 撞车。
import * as THREE from 'three';

// 清单 = `web/static/shaders/` 下全部文件（生成时由磁盘枚举，见本文件尾的核对说明）。
// ⚠ 新增 .glsl 必须同时进这个清单，否则 `#include` 会在运行时抛
// 「Can not resolve #include」——`scripts/check-js.sh` 里有一条核对，别手改漏了。
export const CHUNKS = [
  'px/misc/orbit.frag',
  'px/misc/orbit.vert',
  'px/misc/plume.frag',
  'px/misc/plume.vert',
  'px/noise/fbm.glsl',
  'px/noise/hash13.glsl',
  'px/noise/pars.glsl',
  'px/noise/perlin.glsl',
  'px/noise/ridged.glsl',
  'px/noise/rot.glsl',
  'px/noise/vnoise.glsl',
  'px/noise/warp.glsl',
  'px/noise/warpT.glsl',
  'px/planet/atmo.frag',
  'px/planet/atmo.vert',
  'px/planet/cloud.frag',
  'px/planet/cloud.vert',
  'px/planet/common.glsl',
  'px/planet/planet.frag',
  'px/planet/planet.vert',
  'px/planet/ring.frag',
  'px/planet/ring.vert',
  'px/post/common.glsl',
  'px/post/fullscreen.vert',
  'px/post/grade.frag',
  'px/post/sunflare.frag',
  'px/sky/sky.frag',
  'px/sky/sky.vert',
  'px/sky/star.frag',
  'px/sky/star.vert',
  'px/sun/corona.frag',
  'px/sun/corona.vert',
  'px/sun/flow.glsl',
  'px/sun/photo.frag',
  'px/sun/prom.frag',
  'px/sun/prom.vert',
  'px/sun/sun.vert',
];

// 把 chunk 名变成一行 include 指令。各模块的着色器常量就长这样一行，GLSL 全在文件里。
export function INC(chunk) { return '#include <' + chunk + '>'; }

let inflight = null;

/**
 * 把所有 chunk 取回来注册进 `THREE.ShaderChunk`。**幂等**：重复调用共用同一个 Promise。
 * 必须在任何材质编译**之前**完成 —— `index.js::init()` 开头 await 它。
 * 用绝对路径：页面恒在站点根，相对路径会随 index.html 的位置漂。
 */
export function loadShaders() {
  if (inflight) return inflight;
  inflight = Promise.all(CHUNKS.map(async (c) => {
    const res = await fetch('/shaders/' + c);
    if (!res.ok) throw new Error('GLSL 取不到: /shaders/' + c + ' (HTTP ' + res.status + ')');
    THREE.ShaderChunk[c] = await res.text();
  })).then(() => {
    if (window.__PX_DEBUG) console.log('[glsl] 已装载', CHUNKS.length, '个 chunk');
    return CHUNKS.length;
  });
  return inflight;
}
