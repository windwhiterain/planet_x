// 行星X WebUI — 3D 渲染的公共小工具（颜色空间 / 确定性哈希 / 共享 GLSL 噪声）。
//
// 这里只放「多个模块都要用、且没有状态」的东西。有状态的一律留在 index.js。

import * as THREE from 'three';

// --- 颜色空间 ---------------------------------------------------------------
// config 里的颜色是 CSS hex（sRGB），而 `gl_FragColor` 写出的值被 three.js 当作**线性**
// 颜色再做 sRGB 编码（renderer.outputColorSpace = SRGB）。所以必须先把 sRGB 转成线性，
// 否则每颗行星都会被「免费提亮」一次。MeshStandardMaterial 走 three 自己的 ColorManagement
// 不受影响。
export function srgbToLinear(c) {
  return c <= 0.04045 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
}
export function hex2rgb(hex) {
  if (!hex) return [0.6, 0.6, 0.7];
  const h = String(hex).replace('#', '');
  const full = h.length === 3 ? h.split('').map((c) => c + c).join('') : h;
  const n = parseInt(full, 16);
  if (Number.isNaN(n)) return [0.6, 0.6, 0.7];
  return [
    srgbToLinear(((n >> 16) & 255) / 255),
    srgbToLinear(((n >> 8) & 255) / 255),
    srgbToLinear((n & 255) / 255),
  ];
}
export const v3 = (hex) => new THREE.Vector3(...hex2rgb(hex));

// 把 hex 提亮/压暗一点（星环比行星更亮的冰色、暗面更暗的底色）。
export function lighten(hex, amt) {
  return hex2rgb(hex).map((v) => Math.min(Math.max(v + amt, 0.0), 4.0));
}
export function tint(hex, k) {
  return hex2rgb(hex).map((v) => v * k);
}

// --- 确定性哈希 -------------------------------------------------------------
// 所有「看起来随机」的东西（行星的轴倾角、小行星的分布、舰体上的 greeble）都必须**确定性**：
// 同一个 seed/名字每次加载都要长一样，否则截图对比和「我明明没改这里」的排查全废。
// 用 FNV-1a 把字符串压成 32 位整数，再交给 splitmix32。
export function hashStr(s) {
  let h = 0x811c9dc5;
  const str = String(s);
  for (let i = 0; i < str.length; i++) {
    h ^= str.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}
// 由任意个整数/字符串派生一个 [0,1) 的确定性随机数（无状态，调用顺序无关）。
export function rnd(...keys) {
  let h = 0x9e3779b9;
  for (const k of keys) {
    const x = typeof k === 'number' ? (k | 0) : hashStr(k);
    h = Math.imul(h ^ x, 0x85ebca6b);
    h ^= h >>> 13;
    h = Math.imul(h, 0xc2b2ae35);
    h ^= h >>> 16;
  }
  return (h >>> 0) / 4294967296;
}
// 由 key 派生的确定性三维单位向量（轴倾角/节点方向/碎岩朝向都用它）。
export function rndDir(seed) {
  const z = rnd(seed, 1) * 2 - 1;
  const a = rnd(seed, 2) * Math.PI * 2;
  const r = Math.sqrt(Math.max(0, 1 - z * z));
  return new THREE.Vector3(r * Math.cos(a), z, r * Math.sin(a));
}

// --- 共享 GLSL --------------------------------------------------------------
// value-noise + fbm。旧的 map3d.js 里 fbm 是固定 4 个八度；这里由 `uFbmOct`（uniform）注入。
//
// **八度数必须是 uniform，不能是 `#define` 常量**（2026-09 实测，别再改回去）：
// 常量上界会让编译期把 `for` 完全展开。planet 片元里有 17 处 `fbm` + 4 处 `warp`
// （每处 3 次 `fbm`）+ `ridged`，而每次 `fbm` 迭代内联一个 3D `vnoise`（8 次 `hash13`）
// ⇒ oct=7 时是**两百多个内联 vnoise**。D3D 编译器（fxc）在这种规模上会退化到近乎跑不完：
// 冷缓存实测 oct=2 停 13 s、oct=3 停 31 s、oct=5 直接**永不结束**；期间 GPU 0% / CPU 满载
// （编译在 CPU 上），GPU 进程被占死 ⇒ 整个浏览器、所有标签页一起卡死。
// 动态上界让循环体只出现一次，编译时间塌回可用范围；代价只是每轮一次循环开销，
// 相对现有帧率余量可忽略。
// **注意**：uniform 默认值是 0，漏设会让噪声全变 0（星球变平）——每个用到本块的材质
// 都必须在 `uniforms` 里挂 `fbmOct(...)`。
export const NOISE_GLSL = /* glsl */`
  uniform int uFbmOct;
  float hash13(vec3 p){
    p = fract(p * 0.1031);
    p += dot(p, p.zyx + 31.32);
    return fract((p.x + p.y) * p.z);
  }
  float vnoise(vec3 p){
    vec3 i = floor(p), f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    float n000 = hash13(i);
    float n100 = hash13(i + vec3(1.0, 0.0, 0.0));
    float n010 = hash13(i + vec3(0.0, 1.0, 0.0));
    float n110 = hash13(i + vec3(1.0, 1.0, 0.0));
    float n001 = hash13(i + vec3(0.0, 0.0, 1.0));
    float n101 = hash13(i + vec3(1.0, 0.0, 1.0));
    float n011 = hash13(i + vec3(0.0, 1.0, 1.0));
    float n111 = hash13(i + vec3(1.0, 1.0, 1.0));
    return mix(
      mix(mix(n000, n100, f.x), mix(n010, n110, f.x), f.y),
      mix(mix(n001, n101, f.x), mix(n011, n111, f.x), f.y), f.z);
  }
  // 标准 fbm：uFbmOct 个八度，lacunarity 2.02（避开整数倍造成的格点对齐）。
  float fbm(vec3 p){
    float a = 0.5, s = 0.0;
    for (int i = 0; i < uFbmOct; i++){ s += a * vnoise(p); p *= 2.02; a *= 0.5; }
    return s;
  }
  // 山脊噪声：1-|2n-1|，八度加权更陡，用来做大陆山系 / 日珥丝 / 星云纤维。
  float ridged(vec3 p){
    float a = 0.5, s = 0.0;
    for (int i = 0; i < uFbmOct; i++){
      float n = 1.0 - abs(2.0 * vnoise(p) - 1.0);
      s += a * n * n;
      p *= 2.13; a *= 0.5;
    }
    return s;
  }
  // 单一八度的域扰动（便宜的「流体感」）：q 是扰动量。
  vec3 warp(vec3 p, float amt, float t){
    vec3 q = vec3(fbm(p + vec3(0.0, 0.0, t)), fbm(p + vec3(5.2, 1.3, t)), fbm(p + vec3(9.7, 4.1, t)));
    return p + (q - 0.5) * amt;
  }
`;

// 每个用到 `NOISE_GLSL` 的材质都要挂这个（见上面那段：漏了 = 噪声全 0，星球变平）。
export function fbmOct(n) { return { value: Math.max(1, Math.round(n || 1)) }; }


// 黑体色温 → 线性 RGB（Tanner Helland 拟合，只在 1000K..40000K 有意义）。
// 星空里每颗星的颜色按这个上色——这是「真实感」最便宜也最有效的一招。
export const BLACKBODY_GLSL = /* glsl */`
  vec3 blackbody(float k){
    float t = clamp(k, 1000.0, 40000.0) / 100.0;
    float r, g, b;
    if (t <= 66.0) { r = 255.0; } else { r = 329.698727446 * pow(max(t - 60.0, 1e-3), -0.1332047592); }
    if (t <= 66.0) { g = 99.4708025861 * log(max(t, 1.0)) - 161.1195681661; }
    else { g = 288.1221695283 * pow(max(t - 60.0, 1e-3), -0.0755148492); }
    if (t >= 66.0) { b = 255.0; }
    else if (t <= 19.0) { b = 0.0; }
    else { b = 138.5177312231 * log(max(t - 10.0, 1e-3)) - 305.0447927307; }
    vec3 c = clamp(vec3(r, g, b) / 255.0, 0.0, 1.0);
    // 拟合式给的是 sRGB 显示值；着色器要的是线性光。
    return pow(c, vec3(2.2));
  }
`;

// 屏幕空间效果的公共小工具（后处理各 pass 共用）。
export const POST_COMMON_GLSL = /* glsl */`
  float luma(vec3 c){ return dot(c, vec3(0.2126, 0.7152, 0.0722)); }
  // 带时间种子的白噪声（胶片颗粒 / 抖动）。
  float grainNoise(vec2 uv, float t){
    return fract(sin(dot(uv + t, vec2(12.9898, 78.233))) * 43758.5453);
  }
`;

// three 的 canvas 纹理都要显式设 filter/wrap，否则默认 mipmap + repeat 会在小尺寸下糊掉。
export function canvasTexture(canvas, { srgb = true, aniso = 4 } = {}) {
  const tex = new THREE.CanvasTexture(canvas);
  tex.minFilter = THREE.LinearMipmapLinearFilter;
  tex.magFilter = THREE.LinearFilter;
  tex.generateMipmaps = true;
  if (srgb) tex.colorSpace = THREE.SRGBColorSpace;
  tex.anisotropy = aniso;
  return tex;
}

// 资源生命周期：模块级共享资源打 `userData.shared = true`，disposeGroup 会跳过它们。
export function shared(o) {
  o.userData.shared = true;
  return o;
}

// 递归释放一个 Group 里所有**非共享**的几何/材质/纹理，然后清空子节点。
export function disposeGroup(g) {
  if (!g) return;
  g.traverse((o) => {
    if (o.geometry && !o.geometry.userData.shared) o.geometry.dispose();
    if (o.material) {
      const mats = Array.isArray(o.material) ? o.material : [o.material];
      mats.forEach((m) => {
        if (m.userData.shared) return;
        m.dispose();
        if (m.map && !m.map.userData.shared) m.map.dispose();
        if (m.uniforms) {
          for (const k of Object.keys(m.uniforms)) {
            const v = m.uniforms[k] && m.uniforms[k].value;
            if (v && v.isTexture && !v.userData.shared) v.dispose();
          }
        }
      });
    }
  });
  while (g.children.length) g.remove(g.children[0]);
}
