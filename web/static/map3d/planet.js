// 行星X WebUI — 行星：表面着色器 / 云层 / 大气散射壳 / 星环。
//
// 旧版是一颗「平涂噪声球」：fbm 直出色、没有法线细节、没有云层、没有海洋高光、没有夜面、
// 没有大气。远看还行，一旦 `?focus=地球&dist=7` 拉近就露馅——大陆是色块，球面是塑料。
//
// 这一版的做法：
//   * **表面**：域扰动 fbm 造大陆 + ridged 造山系，再用**有限差分**求高度场梯度做**解析法线**
//     （真地形起伏，不是贴一张法线图）。岩石类再加一层程序化陨石坑场（环脊 + 碗）。
//   * **自转**：不改 `mesh.rotation`，而是把 `uSpin` 交给着色器去旋转**采样方向**。
//     这样「地表的纹理在转」与「城市钉在固定的球面方向上」可以同时成立——夜面城市灯才有意义。
//   * **夜面**：`uCityDirs` 是这颗星上**真实城市**的对象空间方向（由 state 里的城算出），
//     只有背光面亮起来，暖色、带一圈更宽的晕。
//   * **大气**：`DoubleSide` 的自发光壳，正面（贴地那半）给薄雾、背面（绕到球后那半）给
//     临边辉光；颜色沿晨昏线从白蓝过渡到落日橙红（Rayleigh 的廉价近似）。
//   * **星环**：径向密度用噪声而非等距条纹，带卡西尼缝；**环在行星上的投影**（表面着色器里
//     对每个像素反解「该点朝太阳的射线是否穿过环平面」）与**行星在环上的投影**（环着色器里
//     解析求「该点到太阳的射线是否穿过行星球」）双向都有——这两个影子是「土星感」的一半。

import * as THREE from 'three';
import { INC } from './glsl.js';
import { NOISE_GLSL, fbmOct, hex2rgb, lighten } from './util.js';
import { classIndexFor, bandedFor, surfaceOf } from './kinds.js';
import { TUNING } from './tuning.js';

const PLANET_VERT = INC('px/planet/planet.vert');

// 行星/卫星/矮行星都能用的公共小块：绕轴旋转、切空间、海洋/岩石交界。
const COMMON_FRAG = /* glsl */`
  vec3 rotAxis(vec3 v, vec3 a, float ang){
    float c = cos(ang), s = sin(ang);
    return v * c + cross(a, v) * s + a * dot(a, v) * (1.0 - c);
  }
  vec3 hash3t(vec3 p){
    p = vec3(dot(p, vec3(127.1, 311.7, 74.7)),
             dot(p, vec3(269.5, 183.3, 246.1)),
             dot(p, vec3(113.5, 271.9, 124.6)));
    return fract(sin(p) * 43758.5453123);
  }
  // 法线差分专用的低八度 fbm。**上界也是 uniform**：原来手写成
  // 0.5*vnoise(p) + 0.25*vnoise(p*2.02) + 0.125*vnoise(p*4.08)，就是「手工展开 3 份
  // vnoise」，而它有 5 个调用点 ⇒ 15 份内联 vnoise。改成动态循环后只剩一份。
  uniform int uFbmFastOct;
  float fbmFast(vec3 p){
    float a = 0.5, s = 0.0;
    for (int i = 0; i < uFbmFastOct; i++){ s += a * vnoise(p); p *= 2.02; a *= 0.5; }
    return s;
  }
  // 陨石坑场：每格最多一个坑，坑心被限制在格子内部（±0.16，影响半径 ≲0.34）⇒ 不需要查
  // 邻格，也不会有格边界的硬切。返回 = 碗(负) + 环脊(正)。
  float craterField(vec3 p, float S, float density){
    vec3 sp = p * S;
    vec3 cell = floor(sp);
    vec3 r = hash3t(cell);
    if (r.z > density) return 0.0;
    vec3 c = cell + 0.5 + (r - 0.5) * 0.32;   // 必须整体用 vec3：vec3 + vec2 不合法
    float d = length(sp - c);
    float rad = 0.14 + 0.10 * hash13(cell + 3.7);
    float t = d / rad;
    if (t > 1.7) return 0.0;
    float bowl = -smoothstep(0.0, 0.92, t);
    float rim = exp(-pow((t - 0.98) / 0.17, 2.0));
    return bowl * 0.55 + rim * 0.45;
  }
`;

const PLANET_FRAG = INC('px/planet/planet.frag');

// --- 大气壳 ------------------------------------------------------------------
// 用 `DoubleSide` + `gl_FrontFacing` 把「贴着球的那半」和「绕到球后的那半」分开处理：
// 背面 = 临边辉光（视线穿过最厚的大气），正面 = 覆盖在星球上的薄雾。
const ATMO_VERT = INC('px/planet/atmo.vert');

const ATMO_FRAG = INC('px/planet/atmo.frag');

// --- 云层 -------------------------------------------------------------------
const CLOUD_VERT = INC('px/planet/cloud.vert');

const CLOUD_FRAG = INC('px/planet/cloud.frag');

// --- 星环 -------------------------------------------------------------------
const RING_VERT = INC('px/planet/ring.vert');

const RING_FRAG = INC('px/planet/ring.frag');

export const CITY_MAX = 12;

// ---------------------------------------------------------------------------
// 程序化参数 → uniform：`band_freq` ⇒ `uBandFreq`。**刻意不写映射表** —— 那张表是同一事实
// 的第二份表示，加了字段忘了登记不会报错，只会静默变成「config 里填了但没生效」，
// 而那种故障看起来就跟「某个参数没作用」一样，最难查。
// GLSL 侧按同样的名字声明（见 PLANET_FRAG 顶部那串 uniform）；每个类只用到自己那几个，
// 其余的照发不误 —— three.js 只上传程序里真的存在的 uniform，多余的会被忽略。
function paramUniforms(spec) {
  const out = {};
  for (const [k, v] of Object.entries(surfaceOf(spec))) {
    out['u' + k.replace(/(^|_)(\w)/g, (_m, _s, c) => c.toUpperCase())] = { value: v };
  }
  return out;
}

export function planetMaterial(spec, tier, opts = {}) {
  const [base, accent, atmo] = [hex2rgb(spec.color), hex2rgb(spec.accent), hex2rgb(spec.atmosphere)];
  const dirs = [];
  for (let i = 0; i < CITY_MAX; i++) dirs.push(new THREE.Vector3(0, 0, 0));
  const cls = classIndexFor(spec);
  const isGas = cls === 5;
  const hasAtmo = (spec.atmosphere && spec.atmosphere !== '#000000') || cls === 1 || cls === 2 || cls === 5 || cls === 6 || cls === 7;

  const mat = new THREE.ShaderMaterial({
    vertexShader: PLANET_VERT,
    fragmentShader: PLANET_FRAG,
    defines: { CITY_MAX: CITY_MAX },
    uniforms: {
      uFbmOct: fbmOct(tier.oct),
      uFbmFastOct: fbmOct(3),      // 法线差分用的低八度版（见 COMMON_FRAG）
      uCityCount: { value: 0 },    // 由 index.js 在灌城市方向时同步
      uBase: { value: new THREE.Vector3(...base) },
      uAccent: { value: new THREE.Vector3(...accent) },
      uAtmo: { value: new THREE.Vector3(...atmo) },
      uAtmoAmt: { value: hasAtmo ? (isGas ? 0.85 : 1.30) : 0.0 },
      uBanded: { value: bandedFor(spec) ? 1.0 : 0.0 },
      ...paramUniforms(spec),
      uEmissive: { value: spec.emissive || 0.0 },
      uRough: { value: spec.roughness != null ? spec.roughness : 0.8 },
      uMetal: { value: spec.metalness || 0.0 },
      uAmbient: { value: TUNING.shaderAmbient },
      uSpin: { value: 0 },
      uRelief: { value: tier.relief ? TUNING.relief : 0.0 },
      uCloudAmt: { value: TUNING.cloudAmount },
      uCityLights: { value: tier.nightLights ? TUNING.cityLights : 0.0 },
      uClass: { value: cls },
      uHasRing: { value: opts.ring ? 1.0 : 0.0 },
      uRingN: { value: opts.ring ? opts.ring.normal.clone() : new THREE.Vector3(0, 1, 0) },
      uRingIn: { value: opts.ring ? opts.ring.inner : 1 },
      uRingOut: { value: opts.ring ? opts.ring.outer : 1 },
      uCityDirs: { value: dirs },
    },
  });
  mat.userData.hasAtmo = hasAtmo;
  return mat;
}

export function createAtmosphere(radius, spec, tier) {
  const [atmo, sunset] = [hex2rgb(spec.atmosphere), hex2rgb(spec.accent)];
  const shellR = radius * 1.035;
  const mat = new THREE.ShaderMaterial({
    vertexShader: ATMO_VERT,
    fragmentShader: ATMO_FRAG,
    uniforms: {
      uAtmo: { value: new THREE.Vector3(...atmo).multiplyScalar(0.4).add(new THREE.Vector3(0.22, 0.34, 0.62)) },
      uSunset: { value: new THREE.Vector3(...sunset).lerp(new THREE.Vector3(0.95, 0.38, 0.16), 0.55) },
      uDensity: { value: 1.0 },
      uPlanetR: { value: radius },
      uShellR: { value: shellR },
      // 标高：真实大气 ~8 km vs 地球 6371 km ⇒ 约 1/800。这里为了看得见放宽到壳厚的 45%。
      uScaleH: { value: (shellR - radius) * 0.26 },
    },
    transparent: true,
    blending: THREE.AdditiveBlending,
    depthWrite: false,
    // 只要相机侧那半个壳：b>Rp 时前后两个壳面投到同一批像素，双面会让亮度翻倍。
    side: THREE.FrontSide,
  });
  const mesh = new THREE.Mesh(new THREE.SphereGeometry(shellR, tier.seg[0], tier.seg[1]), mat);
  mesh.visible = !!tier.atmo;
  return mesh;
}

export function createClouds(radius, spec, tier) {
  const cls = classIndexFor(spec);
  const mat = new THREE.ShaderMaterial({
    vertexShader: CLOUD_VERT,
    fragmentShader: CLOUD_FRAG,
    uniforms: {
      uFbmOct: fbmOct(Math.max(3, tier.oct - 1)),
      uTime: { value: 0 },
      uSpin: { value: 0 },
      uAmount: { value: cls === 2 ? 0.92 : 0.40 },
      uTint: { value: new THREE.Vector3(0.96, 0.97, 1.0) },
      uOpacity: { value: 0.97 },
    },
    transparent: true,
    depthWrite: false,
    side: THREE.FrontSide,
  });
  const mesh = new THREE.Mesh(new THREE.SphereGeometry(radius * 1.008, tier.seg[0], tier.seg[1]), mat);
  mesh.visible = !!tier.clouds;
  mesh.renderOrder = 2;
  return mesh;
}

export function createRing(radius, spec, tier, planeQuat) {
  const inner = radius * TUNING.ringInner;
  const outer = radius * TUNING.ringOuter;
  const [r, g, b] = lighten(spec.accent || '#c9b08a', 0.22);
  const [r2, g2, b2] = lighten(spec.color || '#e6d9c0', 0.30);
  const mat = new THREE.ShaderMaterial({
    vertexShader: RING_VERT,
    fragmentShader: RING_FRAG,
    uniforms: {
      uFbmOct: fbmOct(Math.max(2, tier.oct - 2)),
      uCol: { value: new THREE.Vector3(r, g, b) },
      uCol2: { value: new THREE.Vector3(r2, g2, b2) },
      uInner: { value: inner },
      uOuter: { value: outer },
      uCenter: { value: new THREE.Vector3() },
      uPlanetR: { value: radius },
    },
    transparent: true,
    depthWrite: false,
    side: THREE.DoubleSide,
  });
  const geo = new THREE.RingGeometry(inner, outer, 256, 1);
  const mesh = new THREE.Mesh(geo, mat);
  // RingGeometry 躺在局部 XY 平面；转到 XZ（地图平面），再乘上该天体的轨道平面朝向。
  mesh.rotation.x = -Math.PI / 2;
  if (planeQuat) mesh.quaternion.premultiply(planeQuat);
  mesh.renderOrder = 3;
  return mesh;
}

// 由 `{inc, node}` 造出该天体轨道平面在世界里的朝向（星环/自转轴都挂它）。
export function planeQuaternion(plane) {
  const q = new THREE.Quaternion();
  if (!plane || !plane.inc) return q;
  // 平面法线：把 +Y 绕交点轴转 inc。
  const axis = new THREE.Vector3(Math.cos(plane.node), 0, Math.sin(plane.node));
  // 交点轴在平面内 → 绕它转 inc 得到倾斜的平面。
  const perp = new THREE.Vector3(-Math.sin(plane.node), 0, Math.cos(plane.node));
  const n = perp.clone().multiplyScalar(Math.sin(plane.inc)).add(new THREE.Vector3(0, Math.cos(plane.inc), 0));
  q.setFromUnitVectors(new THREE.Vector3(0, 1, 0), n.normalize());
  void axis;
  return q;
}
