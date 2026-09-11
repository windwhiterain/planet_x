// 行星X WebUI — 3D 地图渲染器（three.js）。
//
// 独立 ES module，通过 importmap 从 CDN 加载 three.js；用 WebGL 渲染太阳系沙盘：
// 太阳 + 真实轨道（Orbit 参数采样，径向压缩以让内/外行星都可读）+ 天体球（带自定义
// GLSL 行星/太阳着色器）+ 星环 + 城市/舰队标记，支持旋转/平移/缩放（OrbitControls）
// 与拾取。与 app.js（经典脚本，控制面板）通过 window.PlanetXMap 这一小接口解耦：
//
//   PlanetXMap.init(container)  创建渲染器/场景/相机/灯光（只调一次）
//   PlanetXMap.setWorld(world, bodyKinds) 用 /api/state 重建动态对象；bodyKinds 是
//                                    /api/meta 的 body_kinds（config 的类型视觉表）
//   PlanetXMap.resetView(world) 重新适配相机（新游戏时调用）
//   PlanetXMap.onSelect = fn    用户点击天体/城市/舰时收到 {kind, name}
//
// 视觉是**数据驱动**的：每个 state 天体带一个 `类型` key（config body_kinds 的键），
// 本模块据 bodyKinds[body.类型] 解析出颜色/尺寸/类别/星环/着色器分支，不再内联猜测。
// 若 CDN 加载失败，本模块整体失败，但 app.js 的控制面板不受影响。
//
// 几条贯穿全模块的视觉原则（改之前先读）：
// * **光是太阳给的**：天体 shader 用**世界空间法线**点乘指向原点的太阳方向，暖光只有
//   受光面有；环境光只留一点点，否则整颗球会被「相机头灯」均匀照亮（那是 bug，不是风格）。
// * **阵营色只出现在 UI 层**：城市/舰的 3D 模型一律中性舰船灰，阵营身份由
//   「恒定屏幕尺寸的准星环 + 标签 chip 的色点」这类 UI 元素表达，绝不刷在模型上。
// * **标记的遮挡要看得见**：标记精灵不做深度测试（避免被球面切边），改为每帧用解析式
//   射线-球相交判断**中心点**是否被天体挡住，挡住了就淡出——空间感靠这个，不靠 z-buffer。

import * as THREE from 'three';
import { OrbitControls } from 'three/addons/controls/OrbitControls.js';

let scene, camera, renderer, controls, raycaster, pointer;
let bodiesG, citiesG, shipsG, spinners = [];
let lodItems = [];    // 随相机距离/遮挡更新的标记：{mesh, sprites[], pos, switchDist}
let labelItems = [];  // 天体名标签：恒定屏幕尺寸 + 同样参与遮挡淡出
let occluders = [];   // 遮挡体（天体显示球）：{center:Vector3, radius}
let fitted = false;   // 相机是否已按首帧适配（advance 不再重置视角）
let scale = 110;      // 世界单位：最远天体径向压缩后 ≈ `scale` 单位
let currentWorld = null;
let currentVisuals = null;
let layout = null;    // 当前帧的显示布局（computeLayout 的结果）
let sun = null;

// --- 视觉调参（改这里就能整体改观感，配合截图迭代） --------------------------
const TUNING = {
  // 显示半径 = radiusScale × config 的 `radius`（相对类地行星）。
  // 保持 config 的**相对比例**（气巨>冰巨>类地>卫星>矮行星，与真实太阳系次序一致），
  // 只把整体尺度从「木星比太阳还大」压到「太阳明显最大」。
  radiusScale: 2.2,
  radiusMin: 0.26,
  radiusMax: 5.7,
  sunRadius: 6.4,

  // 径向压缩指数 r^orbitExp：越小内太阳系越舒展（外圈/柯伊伯带越挤）。
  orbitExp: 0.42,

  // 星环：内/外缘 = 相对天体显示半径的倍数（土星主环真实跨度约 1.2–2.3 Rs）。
  ringInner: 1.2,
  ringOuter: 2.0,

  // 卫星与母星之间的最小净空（母星带星环时按环外缘算），不足则沿方位角把卫星推出去。
  moonGap: 1.2,

  // 行星 shader：受光面之外的底光。真实太空里背光面几乎全黑，但全黑会让星球在战略图上
  // 「消失」（连城市都找不到），所以留一点点让暗面仍读得出轮廓。
  shaderAmbient: 0.075,
  // 首帧取景：fitR>0 = 固定取景半径；否则按「当前天体最远距离 × fitMargin」自适应。
  fitR: 0,
  fitMargin: 1.22,
  // 遮挡淡出：depth∈[-0.10, 0.06] 内把标记淡出（depth=0 表示中心点正好落在天体轮廓上）。
  occlFadeLo: -0.10,
  occlFadeHi: 0.06,

  // UI 标记尺寸（屏幕像素）。
  reticlePx: 16,        // 阵营准星环（近景会跟着模型大小放大，见 fitWorld）
  farDotPx: 8,          // 远景 billboard（近景换成 3D 模型）
  farDotStationPx: 10,
  labelPx: 13,          // 天体标签 chip 高度
  labelGapPx: 7,        // 标签与天体轮廓之间的间距
};

// 相机远离到超过此距离（世界单位）时，城市/舰的小模型换成恒定尺寸 billboard。
const LOD_SWITCH_DIST = 30;

// 兜底的天体类型（config body_kinds 缺该项/未传时用）：中性岩石外观。
const DEFAULT_KIND = {
  label: '天体', class: 'rock', color: '#8f9bb3', accent: '#5d6678',
  atmosphere: '#1a1a22', radius: 0.6, banded: false, emissive: 0.0,
  roughness: 1.0, metalness: 0.05,
};

// --- 颜色助手 ------------------------------------------------
// config 里的颜色是 CSS hex（sRGB），而 `gl_FragColor` 写出的值被 three.js 当作**线性**
// 颜色再做 sRGB 编码（renderer.outputColorSpace = SRGB）。所以必须在这里先把 sRGB 转成
// 线性，否则每颗行星都会被「免费提亮」一次（0.85 的驼色会变成 0.93 的近白），
// 看起来发灰发糊。MeshStandardMaterial 走 three 自己的 ColorManagement 不受影响。
function srgbToLinear(c) {
  return c <= 0.04045 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
}
function hex2rgb(hex) {
  if (!hex) return [0.6, 0.6, 0.7];
  const h = hex.replace('#', '');
  const full = h.length === 3 ? h.split('').map((c) => c + c).join('') : h;
  const n = parseInt(full, 16);
  if (Number.isNaN(n)) return [0.6, 0.6, 0.7];
  return [
    srgbToLinear(((n >> 16) & 255) / 255),
    srgbToLinear(((n >> 8) & 255) / 255),
    srgbToLinear((n & 255) / 255),
  ];
}

// --- 坐标：径向压缩 + 世界缩放 ---------------------------------------------
// 天体在 AU 里跨度很大（内行星 ~0.3，外行星 ~40），直接按线性会挤成一团。
// 用 r' = r^orbitExp 做径向压缩（保方向、只改半径），再整体缩放到 `scale`。
function compressRadius(r) {
  return Math.pow(r, TUNING.orbitExp);
}
function compress(pos) {
  const x = pos[0], y = pos[1];
  const r = Math.hypot(x, y);
  if (r < 1e-9) return [0, 0];
  const rr = compressRadius(r);
  const k = rr / r;
  return [x * k, y * k];
}
function wp(pos) {
  const [cx, cz] = compress(pos);
  return new THREE.Vector3(cx * scale, 0, cz * scale);
}
// 稳定的世界缩放：用最远天体的「远日点」径向压缩值定标，而非会随回合移动的
// 当前位置，避免 advance 时整幅地图缩放/漂移。
function systemScale(world) {
  let maxAphe = 1e-6;
  world.bodies.forEach((b) => {
    if (b && b.orbit && b.轨道.远日点距离) maxAphe = Math.max(maxAphe, b.轨道.远日点距离);
  });
  return 110 / Math.max(compressRadius(maxAphe), 1e-3);
}

// 由 Orbit 参数在给定 months 计算天体位置（与 Rust Orbit::position 同一套 Kepler 求解）。
function orbitPositionAt(orbit, months) {
  const peri = orbit.近日点距离;
  const aphe = orbit.远日点距离;
  const a = (peri + aphe) / 2;
  const e = aphe > peri ? (aphe - peri) / (aphe + peri) : 0;
  const period = Math.max(orbit.公转周期, 1e-9) || 1;
  const meanMotion = Math.PI * 2 / period;
  let M = (meanMotion * months) % (Math.PI * 2);
  if (M < 0) M += Math.PI * 2;
  let E = M;
  for (let k = 0; k < 24; k++) {
    const f = E - e * Math.sin(E) - M;
    const fp = 1 - e * Math.cos(E);
    const d = f / fp;
    E -= d;
    if (Math.abs(d) < 1e-9) break;
  }
  const half = E / 2;
  const nu = 2 * Math.atan2(Math.sqrt(1 + e) * Math.sin(half), Math.sqrt(1 - e) * Math.cos(half));
  const r = a * (1 - e * Math.cos(E));
  const apheDir = normalize2(orbit.远日点方向 || [1, 0]);
  const periDir = [-apheDir[0], -apheDir[1]];
  const perp = [-periDir[1], periDir[0]];
  const cx = periDir[0] * Math.cos(nu) + perp[0] * Math.sin(nu);
  const cy = periDir[1] * Math.cos(nu) + perp[1] * Math.sin(nu);
  return [cx * r, cy * r];
}
function normalize2(v) {
  const len = Math.hypot(v[0], v[1]);
  if (!len || len < 1e-12) return [1, 0];
  return [v[0] / len, v[1] / len];
}

// --- GLSL：共享的 value-noise（表面变化/色带/太阳粒面共用一个实现） ------------
const NOISE_GLSL = `
  float hash(vec3 p){ p = fract(p * 0.1031); p += dot(p, p.zyx + 31.32); return fract((p.x + p.y) * p.z); }
  float noise(vec3 p){
    vec3 i = floor(p), f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    float n000 = hash(i);
    float n100 = hash(i + vec3(1.0, 0.0, 0.0));
    float n010 = hash(i + vec3(0.0, 1.0, 0.0));
    float n110 = hash(i + vec3(1.0, 1.0, 0.0));
    float n001 = hash(i + vec3(0.0, 0.0, 1.0));
    float n101 = hash(i + vec3(1.0, 0.0, 1.0));
    float n011 = hash(i + vec3(0.0, 1.0, 1.0));
    float n111 = hash(i + vec3(1.0, 1.0, 1.0));
    return mix(
      mix(mix(n000, n100, f.x), mix(n010, n110, f.x), f.y),
      mix(mix(n001, n101, f.x), mix(n011, n111, f.x), f.y), f.z);
  }
  float fbm(vec3 p){
    float a = 0.5, s = 0.0;
    for (int i = 0; i < 4; i++){ s += a * noise(p); p *= 2.02; a *= 0.5; }
    return s;
  }
`;

// 行星顶点着色器：把对象空间位置、**世界空间**法线、世界坐标传给片元。
//
// 注意 `normalMatrix` 是「对象 → **相机/视图**空间」的法线矩阵；而片元里的光照方向是
// 世界空间的（太阳在原点）。两者混用会让 `dot(n, lightDir)` 失去意义——整个球面的漫反射
// 几乎恒为 0，剩下的只有环境光 + 跟着视线走的边缘辉光，看起来就像相机挂了头灯。
// 所以这里必须用 `mat3(modelMatrix)` 把法线变到世界空间（球体是等比缩放，直接乘即可）。
const PLANET_VERT = `
  varying vec3 vNormal;
  varying vec3 vWorldPos;
  varying vec3 vObjPos;
  void main(){
    vNormal = normalize(mat3(modelMatrix) * normal);
    vObjPos = position;
    vec4 wp = modelMatrix * vec4(position, 1.0);
    vWorldPos = wp.xyz;
    gl_Position = projectionMatrix * viewMatrix * wp;
  }
`;

// 行星片元着色器：按 `uClass`（类型分类）绘制各自的表面——气态/冰巨星的色带与风暴，
// 类地的海洋/大陆/云/极冠，火星的荒漠+极冠，金星涡旋云，冰封卫星的光滑冰面，泰坦雾霾。
// 统一做「太阳方向漫反射 + 大气边缘辉光(fresnel) + 自发光 + 微弱高光」。
const PLANET_FRAG = `
  uniform vec3 uBase;
  uniform vec3 uAccent;
  uniform vec3 uAtmo;
  uniform float uBanded;
  uniform float uEmissive;
  uniform float uRough;
  uniform float uAmbient;
  uniform int uClass;
  ${NOISE_GLSL}
  varying vec3 vNormal;
  varying vec3 vWorldPos;
  varying vec3 vObjPos;

  // 由表面方向算一个「类地大陆」式的噪声图（复用 fbm）。
  vec3 surfaceColor(vec3 sph){
    float lat = sph.y;             // -1..1 纬度
    if (uClass == 5) {
      // 气态巨行星：明亮区带 + 锈色带交替，+ 大红斑。
      float b1 = sin(lat * 7.0 + fbm(sph * 2.0) * 1.5);
      float b2 = sin(lat * 15.0 + fbm(sph * 4.5) * 2.6);
      float band = 0.5 + 0.5 * (0.6 * b1 + 0.4 * b2);
      band = smoothstep(0.30, 0.82, band);      // 收紧成清晰的带
      vec3 c = mix(uAccent, uBase, band);
      // 大红斑：一颗被挟带的暗红斑块。
      vec3 spotDir = normalize(vec3(0.55, -0.16, 0.62));
      float spot = exp(-22.0 * length(sph - spotDir));
      c = mix(c, vec3(0.62, 0.30, 0.18), spot * 0.75);
      return c;
    }
    if (uClass == 6) {
      if (uBanded > 0.5) {
        // 冰巨星：更淡、更细的带（天王星/海王星）。
        float band = 0.5 + 0.5 * sin(lat * 11.0 + fbm(sph * 3.0) * 1.2);
        band = smoothstep(0.35, 0.8, band);
        return mix(uAccent, uBase, band);
      }
      // 冰封卫星：光滑冰面 + 细裂纹线。
      float var = fbm(sph * 2.2);
      vec3 c = mix(uBase, uAccent, var * 0.3);
      float crack = abs(fbm(sph * 6.0) - 0.5);
      c = mix(c, vec3(0.9, 0.95, 1.0), smoothstep(0.16, 0.10, crack) * 0.5);
      return c;
    }
    if (uClass == 1) {
      // 类地：海洋 + 大陆 + 云 + 极冠。
      vec3 ocean = uBase;
      vec3 land = vec3(0.30, 0.42, 0.23);
      vec3 c = mix(ocean, land, smoothstep(0.50, 0.57, fbm(sph * 2.4 + 3.0)));
      float cloud = smoothstep(0.56, 0.74, fbm(sph * 3.4 + 11.0));
      c = mix(c, vec3(0.98, 0.98, 0.99), cloud * 0.55);
      float pole = smoothstep(0.82, 0.93, abs(lat));
      c = mix(c, vec3(0.95, 0.97, 0.99), pole * 0.85);
      return c;
    }
    if (uClass == 2) {
      // 金星：浓厚硫磺涡旋云。
      float s = fbm(sph * 2.6 + vec3(0.0, 0.0, lat * 2.5));
      return mix(uAccent, uBase, smoothstep(0.2, 0.8, s));
    }
    if (uClass == 3) {
      // 火星：红荒漠 + 暗区 + 极冠。
      float var = fbm(sph * 2.6);
      vec3 c = mix(uBase, uAccent, var * 0.8);
      float pole = smoothstep(0.82, 0.93, abs(lat));
      c = mix(c, vec3(0.92, 0.93, 0.94), pole * 0.7);
      return c;
    }
    if (uClass == 7) {
      // 泰坦：橙雾霾，纬度渐变（赤道亮、极地暗）。
      float latT = clamp((lat + 1.0) * 0.5, 0.0, 1.0);
      return mix(uAccent, uBase, latT);
    }
    // 岩石/灰岩/矮行星：噪声地表 + 陨石坑暗示。
    float var = fbm(sph * 3.0);
    vec3 c = mix(uBase, uAccent, var * 0.6);
    float crater = smoothstep(0.68, 0.84, fbm(sph * 7.0));
    c *= (1.0 - crater * 0.22);
    return c;
  }

  void main(){
    vec3 n = normalize(vNormal);
    vec3 sph = normalize(vObjPos);            // 球面方向，用于表面采样
    vec3 lightDir = normalize(vec3(0.0, 0.0, 0.0) - vWorldPos); // 太阳在原点
    vec3 viewDir = normalize(cameraPosition - vWorldPos);
    float diff = max(dot(n, lightDir), 0.0);
    // 加一点半影过渡，让晨昏线是一条柔和的带而不是硬切（真实大气散射的廉价近似）。
    float lit = smoothstep(0.0, 0.12, diff);

    vec3 surface = surfaceColor(sph);

    // 颜色 = 表面 ×(环境 + 太阳漫反射) + 微弱自发光 + 大气辉光 + 高光。
    // 环境项故意压得很低：行星的立体感全靠这盏太阳，背光面就该是暗的。
    vec3 col = surface * (uAmbient + (1.0 - uAmbient) * lit);
    col += surface * uEmissive * 0.35;
    // 大气辉光也**跟着太阳**：只有受光侧的边缘才有大气被照亮，夜晚一侧不发光。
    float rim = pow(1.0 - max(dot(n, viewDir), 0.0), 3.5);
    col += uAtmo * rim * (0.06 + 0.94 * lit);
    vec3 hv = normalize(lightDir + viewDir);
    col += vec3(1.0) * pow(max(dot(n, hv), 0.0), 24.0) * (1.0 - uRough) * 0.22 * lit;

    gl_FragColor = vec4(col, 1.0);
  }
`;

// 太阳核心着色器：热色 + 粒面噪声，自发光（不光照）。
const SUN_FRAG = `
  ${NOISE_GLSL}
  varying vec3 vObjPos;
  void main(){
    vec3 sph = normalize(vObjPos);
    float g = fbm(sph * 5.0);
    vec3 hot = vec3(1.0, 0.98, 0.82);
    vec3 mid = vec3(1.0, 0.62, 0.22);
    vec3 cold = vec3(0.78, 0.28, 0.08);
    vec3 col = mix(hot, mid, smoothstep(0.35, 0.75, g));
    col = mix(col, cold, smoothstep(0.75, 0.95, g) * 0.5);
    col += vec3(0.12) * step(0.8, g);   // 更亮的粒面斑
    gl_FragColor = vec4(col, 1.0);
  }
`;

const SUN_VERT = `
  varying vec3 vObjPos;
  void main(){
    vObjPos = position;
    gl_Position = projectionMatrix * viewMatrix * modelMatrix * vec4(position, 1.0);
  }
`;

// class 字符串 → 着色器里 uClass 的整数分支（见 PLANET_FRAG::surfaceColor）。
const CLASS_INDEX = { rock: 0, terran: 1, venus: 2, martian: 3, lunar: 4, gas: 5, ice: 6, titan: 7, dwarf: 8 };
function classIndex(cls) {
  return CLASS_INDEX[cls] != null ? CLASS_INDEX[cls] : 0;
}

function planetMaterial(spec) {
  const [base, accent, atmo] = [hex2rgb(spec.color), hex2rgb(spec.accent), hex2rgb(spec.atmosphere)];
  return new THREE.ShaderMaterial({
    vertexShader: PLANET_VERT,
    fragmentShader: PLANET_FRAG,
    uniforms: {
      uBase: { value: new THREE.Vector3(...base) },
      uAccent: { value: new THREE.Vector3(...accent) },
      uAtmo: { value: new THREE.Vector3(...atmo) },
      uBanded: { value: spec.banded ? 1.0 : 0.0 },
      uEmissive: { value: spec.emissive || 0.0 },
      uRough: { value: spec.roughness || 0.0 },
      uAmbient: { value: TUNING.shaderAmbient },
      uClass: { value: classIndex(spec.class) },
    },
  });
}

// 星环材质：半透明环带（土星/天王星），带径向 alpha 渐变 + 细密同心环缝（卡西尼缝式暗隙）。
// 注意：three.js 的 RingGeometry **不**用极坐标 UV —— 它用平面/矩形映射：
//   uv.x = (x/outer+1)/2，uv.y = (y/outer+1)/2（各自是局部 X/Y 的线性函数）。
// 因此不能把 uv（任一分量）当「径向」。这里改为由顶点的**局部位置**直接算半径：
//   r = length(position.xy)，再按 inner/outer 归一化到 0..1 得到真正的径向 t。
// 这样 `sin(t*…)` 的环缝与卡西尼缝都是严格**同心**的，不会再画出斜向条纹。
function ringMaterial(inner, outer, rgb) {
  const [r, g, b] = rgb;
  return new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    side: THREE.DoubleSide,
    vertexShader: `
      varying vec2 vPos;
      void main(){ vPos = position.xy; gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }
    `,
    fragmentShader: `
      varying vec2 vPos;
      uniform vec3 uCol;
      uniform float uInner;
      uniform float uOuter;
      // 值噪声：让环缝密度/亮度略有随机变化，避免「黑胶唱片」式的绝对均匀。
      float hash(float n){ return fract(sin(n * 91.3458) * 47453.5453); }
      void main(){
        // 真正的径向坐标：由环内某点的局部半径（position.xy 的模）在 [inner, outer] 内归一化。
        float rad = length(vPos);
        float t = (rad - uInner) / max(uOuter - uInner, 1e-4);   // 0=内缘 … 1=外缘
        // 基础 alpha：内缘/外缘淡出，主体较实。
        float alpha = smoothstep(0.0, 0.08, t) * (1.0 - smoothstep(0.86, 1.0, t));
        // C 环（最内区较暗、半透明）。
        alpha *= 0.55 + 0.45 * smoothstep(0.06, 0.30, t);
        // 细密同心环缝：径向高频，但幅度随噪声起伏 → 有疏密变化而非等距条纹。
        float ringlets = 0.5 + 0.5 * sin(t * 120.0 + hash(floor(t * 34.0)) * 6.28);
        alpha *= (0.78 + 0.22 * ringlets);
        // 卡西尼缝（明显）：t 在 ~0.42–0.47 断掉。
        alpha *= 1.0 - 0.85 * smoothstep(0.415, 0.445, t) * (1.0 - smoothstep(0.475, 0.505, t));
        // 恩克缝（外侧细缝）。
        alpha *= 1.0 - 0.5 * smoothstep(0.86, 0.875, t) * (1.0 - smoothstep(0.89, 0.905, t));
        gl_FragColor = vec4(uCol, alpha);
      }
    `,
    uniforms: {
      uCol: { value: new THREE.Vector3(r, g, b) },
      uInner: { value: inner },
      uOuter: { value: outer },
    },
  });
}

// 把 hex 提亮一点（星环用比行星更亮的冰色）。
function lighten(hex, amt) {
  const [r, g, b] = hex2rgb(hex).map((v) => Math.min(v + amt, 1.0));
  return [r, g, b];
}

// --- 标签 ----------------------------------------------------------------
// 标签画成一张 UI「chip」：半透明深色圆角底 + 白字，名字前面可以带阵营色小色块
// （天体是某势力首都时用）。返回 { sprite, px, aspect } —— `px` 是**期望的屏幕高度**，
// 真正的 scale 由 updateMarkers 每帧按相机距离换算，所以标签在任意缩放下都是同一个
// 像素尺寸（旧版是世界尺寸 sprite，镜头一贴近就占满整个屏幕）。
const LABEL_SS = 2;        // canvas 超采样倍率（HiDPI 下文字才不糊）
function makeLabel(text, color = '#dbe6ff', fontPx = 13, dots = []) {
  const padX = 7, padY = 4, dotR = 3.2, dotGap = 6;
  const font = `600 ${fontPx * LABEL_SS}px system-ui, "Segoe UI", "Microsoft YaHei", sans-serif`;
  const c = document.createElement('canvas');
  let ctx = c.getContext('2d');
  ctx.font = font;
  const textW = ctx.measureText(text).width;
  const dotW = dots.length ? dots.length * (dotR * 2 * LABEL_SS + dotGap * LABEL_SS) : 0;
  const w = Math.ceil(textW + dotW) + padX * 2 * LABEL_SS;
  const h = Math.ceil(fontPx * 1.34 * LABEL_SS) + padY * 2 * LABEL_SS;
  c.width = w; c.height = h;
  ctx = c.getContext('2d');            // 改尺寸会重置 context 状态，必须重设
  ctx.font = font;
  ctx.textBaseline = 'middle';
  ctx.textAlign = 'left';
  // 底：圆角深色 chip，保证压在任何行星上都能读。
  const r = h / 2;
  ctx.beginPath();
  ctx.moveTo(r, 0); ctx.lineTo(w - r, 0); ctx.arc(w - r, r, r, -Math.PI / 2, Math.PI / 2);
  ctx.lineTo(r, h); ctx.arc(r, r, r, Math.PI / 2, -Math.PI / 2);
  ctx.closePath();
  ctx.fillStyle = 'rgba(6,10,22,0.62)';
  ctx.fill();
  ctx.strokeStyle = 'rgba(255,255,255,0.10)';
  ctx.lineWidth = 1 * LABEL_SS;
  ctx.stroke();
  let x = padX * LABEL_SS;
  dots.forEach((d) => {
    ctx.beginPath();
    ctx.arc(x + dotR * LABEL_SS, h / 2, dotR * LABEL_SS, 0, Math.PI * 2);
    ctx.fillStyle = d;
    ctx.fill();
    x += dotR * 2 * LABEL_SS + dotGap * LABEL_SS;
  });
  ctx.fillStyle = color;
  ctx.fillText(text, x, h / 2 + LABEL_SS);
  const tex = new THREE.CanvasTexture(c);
  tex.minFilter = THREE.LinearFilter;
  tex.userData.shared = false;
  const mat = new THREE.SpriteMaterial({ map: tex, transparent: true, depthTest: false, depthWrite: false });
  const sp = new THREE.Sprite(mat);
  return { sprite: sp, px: h / LABEL_SS, aspect: w / h };
}

// --- 动态对象重建 ------------------------------------------------------------
// 共享资源（标记贴图、城市/舰的几何与材质）标记了 userData.shared → 不随单次 setWorld 释放。
function disposeGroup(g) {
  if (!g) return;
  g.traverse((o) => {
    if (o.geometry && !o.geometry.userData.shared) o.geometry.dispose();
    if (o.material) {
      const mats = Array.isArray(o.material) ? o.material : [o.material];
      mats.forEach((m) => {
        if (m.userData.shared) return;
        m.dispose();
        if (m.map && !m.map.userData.shared) m.map.dispose();
      });
    }
  });
  while (g.children.length) g.remove(g.children[0]);
}
function clearSpinners() {
  spinners = [];
}

// 天体显示半径：直接用 config 的 `radius`（相对类地行星）乘一个全局尺度。
// 刻意**不做** `2.4*radius + 0.9` 那种放大：那会让木星(7.1)比太阳(3.2)还大、卫星整个
// 陷进母星里。这里只做整体缩放，保住 config 里「气巨 > 冰巨 > 类地 > 卫星 > 矮行星」
// 的真实次序与大致比例。
function bodyRadius(body, spec) {
  const r = TUNING.radiusScale * (spec.radius || 0.6);
  return Math.min(Math.max(r, TUNING.radiusMin), TUNING.radiusMax);
}

// --- 显示布局：算出每个天体的**显示**位置（纯渲染用，不动 state） ---------------
// 真实比例下卫星轨道半径远小于被夸张过的母星显示半径，直接画会重叠。做法：
//   1) 卫星离母星不足「母星半径(带环则按环外缘) + 自身半径 + moonGap」时，
//      沿母星→卫星的方位角把它推到刚好够远，并记下缩放系数 k；
//   2) 画轨道线时对**整条轨道**用同一个 k（相对母星缩放偏移量），卫星仍然精确落在线
//      自己画出的轨道上，不会「飘在轨道外」。
function computeLayout(world, visuals) {
  const nodes = new Map();     // name -> { pos:Vector3, radius:number }
  world.bodies.forEach((b) => {
    nodes.set(b.天体名, { pos: wp(b.位置), radius: bodyRadius(b, specFor(visuals, b)) });
  });
  const orbitScale = new Map(); // 卫星 name -> k（画轨道线用）
  world.bodies.forEach((b) => {
    if (!b.轨道 || !b.轨道.母天体) return;
    const me = nodes.get(b.天体名);
    const par = nodes.get(b.轨道.母天体);
    if (!me || !par) return;
    const parBody = world.bodies.find((x) => x.天体名 === b.轨道.母天体);
    const parReach = par.radius * (parBody && parBody.星环 ? TUNING.ringOuter : 1.0);
    const minSep = parReach + me.radius + TUNING.moonGap;
    const off = me.pos.clone().sub(par.pos);
    const d = off.length();
    if (d < 1e-6 || d >= minSep) return;
    const k = minSep / d;
    orbitScale.set(b.天体名, k);
    me.pos.copy(par.pos).addScaledVector(off, k);
  });
  return { nodes, orbitScale };
}

function specFor(visuals, body) {
  return (visuals && visuals[body.类型]) || DEFAULT_KIND;
}

// --- UI 标记层：恒定屏幕尺寸的精灵（billboard / 阵营准星环 / 标签） ------------
// 所有标记精灵都是「恒定像素尺寸」：每帧按相机距离换算成世界尺寸（见 updateMarkers），
// 因此镜头拉远拉近时标记不会忽大忽小。标记一律 `depthTest:false`（避免被球面切边、
// 或被自己的行星地面吃掉），遮挡改由 occlAlpha() 显式计算——这才是「空间感」的来源。
const shapeTex = {};
function makeShapeTexture(shape) {
  const S = 128;                      // 画大一点，缩到十几像素时边缘才干净
  const c = document.createElement('canvas');
  c.width = c.height = S;
  const ctx = c.getContext('2d');
  ctx.clearRect(0, 0, S, S);
  ctx.fillStyle = '#ffffff';
  ctx.strokeStyle = '#ffffff';
  const m = S / 2;
  if (shape === 'dot') {
    ctx.beginPath(); ctx.arc(m, m, S * 0.20, 0, Math.PI * 2); ctx.fill();
  } else if (shape === 'diamond') {
    ctx.beginPath();
    ctx.moveTo(m, S * 0.20); ctx.lineTo(S * 0.80, m); ctx.lineTo(m, S * 0.80); ctx.lineTo(S * 0.20, m);
    ctx.closePath(); ctx.fill();
  } else if (shape === 'reticle') {
    // 阵营准星环：细圆环 + 四个短刻度，中间留空给模型自己。
    ctx.lineWidth = S * 0.045;
    ctx.beginPath(); ctx.arc(m, m, S * 0.36, 0, Math.PI * 2); ctx.stroke();
    ctx.lineWidth = S * 0.055;
    for (let i = 0; i < 4; i++) {
      const a = i * Math.PI / 2 + Math.PI / 4;
      ctx.beginPath();
      ctx.moveTo(m + Math.cos(a) * S * 0.40, m + Math.sin(a) * S * 0.40);
      ctx.lineTo(m + Math.cos(a) * S * 0.46, m + Math.sin(a) * S * 0.46);
      ctx.stroke();
    }
  }
  const tex = new THREE.CanvasTexture(c);
  tex.userData.shared = true;   // 多个标记共享，不随单个释放销毁
  tex.anisotropy = 4;
  return tex;
}
// 建一个恒定像素尺寸的标记精灵。`px` 期望屏幕像素；`aspect` 用于非正方形贴图（标签）。
function makeMarkerSprite(shape, colorHex, px, opacity = 1, aspect = 1) {
  const tex = shapeTex[shape] || (shapeTex[shape] = makeShapeTexture(shape));
  const mat = new THREE.SpriteMaterial({
    map: tex,
    color: new THREE.Color(colorHex || '#ffffff'),
    transparent: true,
    opacity,
    depthTest: false,
    depthWrite: false,
  });
  const sp = new THREE.Sprite(mat);
  return { sp, px, aspect, baseOpacity: opacity, always: true };
}

// 解析式遮挡：把「相机 → 标记中心」这条线段与每个天体显示球求交。
// depth ∈ (-∞, 1]：≤0 = 中心点在轮廓外（看得见，越负越远）；0 = 正好压在轮廓边缘；
// 越大 = 中心点越深入球体背面。(r - 垂距)/r 对球体是精确的「背面深度」，而且**连续**——
// 所以沿 [-0.10, 0.06] 淡出不会有跳变。返回 1 = 完全可见，0 = 完全被挡住。
function occlAlpha(pos) {
  if (!occluders.length) return 1;
  _occlD.copy(pos).sub(camera.position);
  const tMax = _occlD.length();
  if (tMax < 1e-6) return 1;
  _occlD.divideScalar(tMax);
  let depth = -Infinity;
  for (let i = 0; i < occluders.length; i++) {
    const o = occluders[i];
    _occlV.copy(o.center).sub(camera.position);
    const tc = _occlV.dot(_occlD);
    if (tc <= 0 || tc >= tMax) continue;              // 球体在标记之后 → 挡不住
    const d2 = _occlV.lengthSq() - tc * tc;
    const r2 = o.radius * o.radius;
    if (d2 >= r2) continue;                            // 光线从天体旁边擦过去
    const d = (o.radius - Math.sqrt(Math.max(d2, 0))) / o.radius;
    if (d > depth) depth = d;
  }
  if (depth === -Infinity) return 1;
  return 1 - THREE.MathUtils.smoothstep(depth, TUNING.occlFadeLo, TUNING.occlFadeHi);
}
const _occlD = new THREE.Vector3();
const _occlV = new THREE.Vector3();

// 城市在行星上的确定性方位：`elev` 为距 +Y 极轴的极角（0=顶，π/2=赤道），返回单位方向。
function cityDir(idx, count, elev) {
  const az = (idx / Math.max(count, 1)) * Math.PI * 2 + 0.6;
  const horiz = Math.sin(elev);
  const y = Math.cos(elev);
  return [Math.cos(az) * horiz, y, Math.sin(az) * horiz];
}

// 每帧更新所有标记：按相机距离切换「近景模型 / 远景 UI 精灵」、按固定像素换算世界尺寸、
// 以及按遮挡淡出。世界尺寸 = px × (2·dist·tan(fov/2) / 视口高)，投影后正好约 px 像素。
function updateMarkers() {
  if (!camera || !renderer) return;
  const fov = camera.fov * Math.PI / 180;
  const vh = renderer.domElement.clientHeight || 600;
  const k = 2 * Math.tan(fov / 2) / vh;               // 世界长度 = k · 距离 · 像素
  for (const it of lodItems) {
    const dist = camera.position.distanceTo(it.pos);
    const wpp = k * dist;                             // 一个屏幕像素对应的世界长度
    const near = dist <= (it.switchDist || LOD_SWITCH_DIST);
    const a = it.fade === false ? 1 : occlAlpha(it.pos);
    if (it.mesh) it.mesh.visible = near;
    for (const s of it.sprites) {
      const vis = (s.always || !near) && a > 0.012;
      s.sp.visible = vis;
      if (!vis) continue;
      // 准星环带 fitWorld 时贴住模型大小（否则拉近后模型会捅出环外）。
      const px = s.fitWorld ? Math.min(Math.max(s.fitWorld / wpp + 9, s.px), 56) : s.px;
      s.sp.scale.set(px * wpp * s.aspect, px * wpp, 1);
      s.sp.material.opacity = s.baseOpacity * a;
    }
  }
  // 标签：恒定像素尺寸 + 始终浮在天体上方（间距也按像素算，缩放时不会越离越远）
  // + 与标记同一套遮挡淡出（用标签自身位置判定，否则会被自己的行星永远挡住）。
  for (const lb of labelItems) {
    const wpp = k * camera.position.distanceTo(lb.center);
    lb.sp.position.set(
      lb.center.x,
      lb.center.y + lb.radius + TUNING.labelGapPx * wpp,
      lb.center.z,
    );
    const a = occlAlpha(lb.sp.position);
    lb.sp.visible = a > 0.012;
    if (!lb.sp.visible) continue;
    lb.sp.scale.set(lb.px * wpp * lb.aspect, lb.px * wpp, 1);
    lb.sp.material.opacity = a;
  }
}

// 轨道路径：`orbit` 的局部（焦点中心）轨道，在 AU 里叠加 `anchor`（母天体的世界坐标，
// 日心行星传入 [0,0] 即绕太阳）后做**径向压缩**，得到该天体在压缩平面上的真实路径——卫星
// 的椭圆就包在它的母天体周围，而不是绕太阳。
// `k`：相对母星的偏移缩放系数（computeLayout 算出的「卫星让位」系数）。整条轨道一起缩放，
// 卫星才仍然精确落在自己画出的轨道线上。
function orbitLine(orbit, anchor, k) {
  const pts = [];
  const period = Math.max(orbit.period, 1e-6);
  const n = 160;
  const ax = anchor ? (anchor[0] || 0) : 0;
  const ay = anchor ? (anchor[1] || 0) : 0;
  const [acx, acz] = compress([ax, ay]);
  const kk = k || 1;
  for (let i = 0; i <= n; i++) {
    const t = (i / n) * period;
    const local = orbitPositionAt(orbit, t);
    const [cx, cz] = compress([ax + local[0], ay + local[1]]);
    pts.push(new THREE.Vector3((acx + (cx - acx) * kk) * scale, 0, (acz + (cz - acz) * kk) * scale));
  }
  const g = new THREE.BufferGeometry().setFromPoints(pts);
  // 轨道画得细而淡，避免与行星/标记抢视觉（减少重叠感）。
  const m = new THREE.LineBasicMaterial({ color: 0x2c3a5f, transparent: true, opacity: 0.22 });
  return new THREE.LineLoop(g, m);
}

function facColorFor(world, fid) {
  const f = world.factions.find((x) => x.id === fid);
  return (f && f.颜色) || '#8f9bb3';
}

function addRing(parent, position, r, colorHex) {
  const inner = r * TUNING.ringInner;
  const outer = r * TUNING.ringOuter;
  const geo = new THREE.RingGeometry(inner, outer, 128, 1);
  const [cr, cg, cb] = lighten(colorHex || '#c9b08a', 0.18);
  const mesh = new THREE.Mesh(geo, ringMaterial(inner, outer, [cr, cg, cb]));
  mesh.rotation.x = -Math.PI / 2;          // 铺平到地图平面（XZ）
  mesh.position.copy(position);            // 放到该天体（而非太阳）处
  parent.add(mesh);
}

// --- 城市/舰模型：中性舰船灰 + 一点结构感 ------------------------------------
// 阵营色**不**刷在模型上（否则就是一堆花花绿绿的塑料块）。身份交给 UI 层：准星环 + 标签
// chip。材质统一「灰白喷涂的航天器」——高 roughness、近零 metalness，只有窗户/舱灯用自发光。
// 几何/材质模块级共享并标记 userData.shared，disposeGroup 会跳过它们。
function shared(o) { o.userData.shared = true; return o; }
const HULL = {
  get hull() { return this._h || (this._h = shared(new THREE.MeshStandardMaterial({ color: 0xd9dde5, roughness: 0.62, metalness: 0.06 }))); },
  get dark() { return this._d || (this._d = shared(new THREE.MeshStandardMaterial({ color: 0x9ba3b1, roughness: 0.8, metalness: 0.04 }))); },
  get lamp() { return this._l || (this._l = shared(new THREE.MeshStandardMaterial({ color: 0x2a3242, emissive: 0xffc978, emissiveIntensity: 1.4, roughness: 0.6 }))); },
  get solar() { return this._s || (this._s = shared(new THREE.MeshStandardMaterial({ color: 0x2f4a6e, roughness: 0.35, metalness: 0.2 }))); },
};
const GEO = {
  get dome() { return this._dome || (this._dome = shared(new THREE.SphereGeometry(0.5, 14, 8, 0, Math.PI * 2, 0, Math.PI / 2))); },
  get tower() { return this._tower || (this._tower = shared(new THREE.BoxGeometry(0.17, 1, 0.17))); },
  get block() { return this._block || (this._block = shared(new THREE.BoxGeometry(0.42, 0.3, 0.42))); },
  get torus() { return this._torus || (this._torus = shared(new THREE.TorusGeometry(0.36, 0.05, 8, 22))); },
  get core() { return this._core || (this._core = shared(new THREE.SphereGeometry(0.15, 12, 8))); },
  get panel() { return this._panel || (this._panel = shared(new THREE.BoxGeometry(0.42, 0.02, 0.2))); },
  get hullBox() { return this._hullBox || (this._hullBox = shared(new THREE.BoxGeometry(0.16, 0.14, 0.62))); },
  get fin() { return this._fin || (this._fin = shared(new THREE.BoxGeometry(0.03, 0.22, 0.2))); },
};

// 地面城市：一个穹顶 + 两三栋塔楼，全部组装在以天体中心为原点的小 Group 里。
function groundCityModel(s) {
  const g = new THREE.Group();
  const dome = new THREE.Mesh(GEO.dome, HULL.dark);
  dome.scale.setScalar(s * 0.9);
  dome.position.y = s * 0.02;
  g.add(dome);
  const n = 3;
  for (let i = 0; i < n; i++) {
    const a = (i / n) * Math.PI * 2 + 0.4;
    const h = s * (1.5 - i * 0.28);
    const t = new THREE.Mesh(GEO.tower, i === 0 ? HULL.hull : HULL.dark);
    t.scale.set(s * 0.55, h, s * 0.55);
    t.position.set(Math.cos(a) * s * 0.42, h * 0.5 + s * 0.1, Math.sin(a) * s * 0.42);
    g.add(t);
  }
  // 舱灯：一点点暖光，让「城市」读起来是活的而不是一块灰。
  const lamp = new THREE.Mesh(GEO.block, HULL.lamp);
  lamp.scale.set(s * 0.5, s * 0.22, s * 0.5);
  lamp.position.y = s * 0.12;
  g.add(lamp);
  return g;
}
// 轨道空间站：一个环 + 核心 + 两片太阳能板。
function stationModel(s) {
  const g = new THREE.Group();
  const ring = new THREE.Mesh(GEO.torus, HULL.hull);
  ring.scale.setScalar(s * 2.1);
  g.add(ring);
  const core = new THREE.Mesh(GEO.core, HULL.dark);
  core.scale.setScalar(s * 1.6);
  g.add(core);
  for (const sign of [-1, 1]) {
    const p = new THREE.Mesh(GEO.panel, HULL.solar);
    p.scale.set(s * 2.6, s * 1.6, s * 2.0);
    p.position.z = sign * s * 1.1;
    g.add(p);
  }
  return g;
}
// 舰：细长船体 + 背鳍 + 尾部喷口。按舰级给不同长度（护卫短、战列长）。
// 尺度刻意做得很小：一枚护卫舰不该有行星半径的三分之一。
const SHIP_LEN = { corvette: 0.14, destroyer: 0.19, cruiser: 0.25, carrier: 0.30, battleship: 0.36 };
function shipModel(cls) {
  const s = SHIP_LEN[cls] || 0.4;
  const g = new THREE.Group();
  const body = new THREE.Mesh(GEO.hullBox, HULL.hull);
  body.scale.set(s / 0.62, s / 0.62, s / 0.62);
  g.add(body);
  const fin = new THREE.Mesh(GEO.fin, HULL.dark);
  fin.scale.setScalar(s / 0.62);
  fin.position.set(0, s * 0.28, -s * 0.1);
  g.add(fin);
  const noz = new THREE.Mesh(GEO.core, HULL.lamp);
  noz.scale.set(s * 1.1, s * 1.1, s * 0.8);
  noz.position.z = -s * 0.78;
  g.add(noz);
  return g;
}

// 每个城市/舰标记 = 中性 3D 模型（近景）+ 阵营色准星环（常显）+ 中性远景点（远景观）。
// 三者都挂在以天体中心为原点的 Group 里，随天体一起移动（「关于星球的坐标」）。
function addMarker(group, world, opts) {
  const g = new THREE.Group();
  g.userData = { kind: opts.kind, name: opts.name };
  if (opts.model) {
    opts.model.position.copy(opts.local);
    g.add(opts.model);
  }
  if (opts.orient) {
    opts.model.quaternion.setFromUnitVectors(new THREE.Vector3(0, 1, 0), new THREE.Vector3(...opts.orient));
  }
  const sprites = [];
  // 阵营色准星环：近景时贴着模型大小（模型大小/像素尺度 + 余量），远景时收到固定尺寸。
  const ret = makeMarkerSprite('reticle', opts.color, TUNING.reticlePx, 0.85);
  ret.fitWorld = opts.modelSize || 0;
  ret.sp.position.copy(opts.local);
  g.add(ret.sp);
  sprites.push(ret);
  // 中性远景点：镜头拉远、模型被 LOD 换掉之后代表这个标记。阵营色只在准星环上。
  const dot = makeMarkerSprite(opts.shape, '#eef2f8', opts.px, 1);
  dot.always = false;
  dot.sp.position.copy(opts.local);
  g.add(dot.sp);
  sprites.push(dot);
  g.position.copy(opts.center);
  group.add(g);
  lodItems.push({
    mesh: opts.model || null,
    sprites,
    pos: opts.world.clone(),
    switchDist: opts.switchDist,
    fade: opts.fade !== false,
  });
}

function renderBodies(group, world, visuals, layout) {
  // 天体是某势力首都时，标签 chip 前面带该势力的色点（阵营身份的 UI 表达）。
  const capsByBody = {};
  (world.factions || []).forEach((f) => {
    if (!f.capital_body) return;
    (capsByBody[f.capital_body] = capsByBody[f.capital_body] || []).push(f.颜色);
  });
  world.bodies.forEach((b) => {
    const spec = specFor(visuals, b);
    const node = layout.nodes.get(b.天体名);
    const p = node.pos;
    const r = node.radius;
    const geo = new THREE.SphereGeometry(r, 48, 32);
    const mat = planetMaterial(spec);
    const mesh = new THREE.Mesh(geo, mat);
    mesh.position.copy(p);
    mesh.userData = { kind: 'body', name: b.天体名 };
    group.add(mesh);

    // 缓慢自转（只转表面 shader 采样可见的球体）；气态/类地/冰巨星转得更明显。
    if (spec.class === 'gas' || spec.class === 'ice' || spec.class === 'terran' || spec.class === 'venus') {
      spinners.push({ mesh, speed: (spec.class === 'terran' || spec.class === 'venus') ? 0.004 : 0.0025 });
    }

    // 星环（intrinsic body 属性：土星/天王星）。
    if (b.星环) addRing(group, p, r, spec.accent);

    // 卫星的轨道画在它的母天体周围：anchor = 母天体的世界坐标（AU）；日心行星 anchor=[0,0]。
    const anchor = b.轨道.母天体
      ? ((world.bodies.find((x) => x.天体名 === b.轨道.母天体) || {}).位置 || [0, 0])
      : [0, 0];
    group.add(orbitLine(b.轨道, anchor, layout.orbitScale.get(b.天体名) || 1));

    const lbl = makeLabel(b.天体名, '#dbe6ff', TUNING.labelPx, capsByBody[b.天体名] || []);
    group.add(lbl.sprite);
    labelItems.push({ sp: lbl.sprite, px: lbl.px, aspect: lbl.aspect, center: p.clone(), radius: r });
  });
}

// 城市模型很小；相对其所属天体定位——
//   地面城市：贴在天体表面的确定性点（按城市在整群里的序号给方位）。
//   空间站：悬在这颗天体更高的轨道上。
function renderCities(group, world, visuals, layout) {
  const byBody = {};
  world.cities.forEach((c) => { (byBody[c.所在天体] = byBody[c.所在天体] || []).push(c); });
  Object.entries(byBody).forEach(([bodyName, cities]) => {
    const body = world.bodies.find((b) => b.天体名 === bodyName);
    const node = layout.nodes.get(bodyName);
    if (!body || !node) return;
    const P = node.pos;
    const r = node.radius;
    // 模型尺寸跟着天体显示半径走，小行星上的城市不会跟母星一样大。
    // 刻意压得**很小**（城市高度 ≈ 行星半径的 10% 量级）——它们是地表上的聚落，
    // 不是贴在行星上的巨型水晶。
    const s = Math.min(Math.max(r * 0.085, 0.030), 0.18);
    cities.forEach((c, idx) => {
      const color = facColorFor(world, c.势力);
      let model, local, orient = null, px, modelSize;
      if (c.轨道空间站) {
        // 空间站：轨道半径略大于行星，绕行星一圈分布。
        const az = (idx / Math.max(cities.length, 1)) * Math.PI * 2 + 1.7;
        const orbR = r * 1.45;
        local = new THREE.Vector3(Math.cos(az) * orbR, r * 0.45, Math.sin(az) * orbR);
        model = stationModel(s * 0.75);
        px = TUNING.farDotStationPx;
        modelSize = s * 1.1;
      } else {
        // 地面城市：贴在天体表面，朝表面法线方向直立。billboard/准星抬到半径之外，
        // 让它贴在天体轮廓外缘——这样它才不会被自己的球体前面。
        const dir = cityDir(idx, cities.length, 0.95);
        const rb = r * 1.05;
        local = new THREE.Vector3(dir[0] * rb, dir[1] * rb, dir[2] * rb);
        model = groundCityModel(s);
        orient = dir;
        px = TUNING.farDotPx;
        modelSize = s * 1.2;
      }
      addMarker(group, world, {
        kind: 'city', name: c.城名, color, model, local, orient,
        center: P, world: new THREE.Vector3(P.x + local.x, P.y + local.y, P.z + local.z),
        shape: c.轨道空间站 ? 'reticle' : 'dot',
        px, modelSize,
        switchDist: Math.max(14, r * 22),
      });
    });
  });
}

function renderShips(group, world, layout) {
  world.ships.forEach((s) => {
    // 舰在星际空间里飞，不挂在任何天体下 → 直接用它的显示坐标（未被布局调整过）。
    const p = wp(s.坐标);
    const color = facColorFor(world, s.势力);
    const y = 0.6;
    const model = shipModel(s.舰级);
    const modelSize = (SHIP_LEN[s.舰级] || 0.4) * 0.9;
    addMarker(group, world, {
      kind: 'ship', name: s.舰名, color, model,
      local: new THREE.Vector3(0, y, 0),
      center: new THREE.Vector3(p.x, 0, p.z),
      world: new THREE.Vector3(p.x, y, p.z),
      shape: 'diamond', px: TUNING.farDotPx, modelSize,
      switchDist: LOD_SWITCH_DIST,
    });
  });
}

function makeStars() {
  const N = 900;
  const pos = new Float32Array(N * 3);
  for (let i = 0; i < N; i++) {
    const r = 900 + Math.random() * 900;
    const a = Math.random() * Math.PI * 2;
    const c = Math.acos(Math.random() * 2 - 1);
    pos[i * 3] = r * Math.sin(c) * Math.cos(a);
    pos[i * 3 + 1] = r * Math.sin(c) * Math.sin(a);
    pos[i * 3 + 2] = r * Math.cos(c);
  }
  const g = new THREE.BufferGeometry();
  g.setAttribute('position', new THREE.BufferAttribute(pos, 3));
  const m = new THREE.PointsMaterial({ color: 0x9fb4d8, size: 1.4, sizeAttenuation: false });
  return new THREE.Points(g, m);
}

// 太阳：自发光白热核心 + 径向光晕 sprite（additive），替代纯色球。
function makeSun() {
  const core = new THREE.Mesh(
    new THREE.SphereGeometry(TUNING.sunRadius, 48, 32),
    new THREE.ShaderMaterial({ vertexShader: SUN_VERT, fragmentShader: SUN_FRAG })
  );

  // 光晕：canvas 径向渐变 → additive sprite，模拟日冕辉光。
  const c = document.createElement('canvas');
  c.width = c.height = 256;
  const ctx = c.getContext('2d');
  const grad = ctx.createRadialGradient(128, 128, 8, 128, 128, 128);
  grad.addColorStop(0.0, 'rgba(255,240,200,0.9)');
  grad.addColorStop(0.25, 'rgba(255,200,110,0.45)');
  grad.addColorStop(0.6, 'rgba(255,140,60,0.15)');
  grad.addColorStop(1.0, 'rgba(255,120,50,0)');
  ctx.fillStyle = grad;
  ctx.fillRect(0, 0, 256, 256);
  const tex = new THREE.CanvasTexture(c);
  // 光晕小一点、淡一点：过大的 additive 光晕在相机贴近太阳系时会铺满全屏，
  // 把整幅画面染成金色（行星看起来像黄铜）。只保留贴近太阳的柔和日冕辉光。
  const haloSp = new THREE.Sprite(new THREE.SpriteMaterial({ map: tex, color: 0xffd98a, transparent: true, opacity: 0.38, depthWrite: false, blending: THREE.AdditiveBlending }));
  haloSp.scale.set(22, 22, 1);

  const g = new THREE.Group();
  g.add(core);
  g.add(haloSp);
  return g;
}

// --- 相机适配 ---------------------------------------------------------------
function fitCamera(world) {
  scale = systemScale(world);
  // 取景半径：按**当前**天体位置的最大半径（而不是最远远日点）来定，否则镜头会为了几条
  // 此刻空无一物的远日点轨道白白缩掉三分之一，行星在屏幕上小得看不清。
  // TUNING.fitR > 0 时用固定值（调试/特殊取景用）。
  let maxR = 0;
  world.bodies.forEach((b) => {
    const v = wp(b.位置);
    maxR = Math.max(maxR, Math.hypot(v.x, v.z));
  });
  const R = TUNING.fitR > 0 ? TUNING.fitR : Math.max(48, maxR * TUNING.fitMargin);
  // 让 R 撑满约 78% 的视口宽度，全屏视图里太阳系不挤在中央一小块。
  const fov = camera.fov * Math.PI / 180;
  const aspect = camera.aspect || 1.6;
  const halfW = Math.tan(fov / 2) * aspect;
  const dist = R / (halfW * 0.78);
  const dir = new THREE.Vector3(0.60, 0.70, 0.42).normalize();
  camera.position.copy(dir.multiplyScalar(dist));
  camera.near = 1;
  camera.far = 6000;
  camera.updateProjectionMatrix();
  controls.target.set(0, 0, 0);
  controls.update();
}

// --- 拾取 ------------------------------------------------------------------
let downX = 0, downY = 0;
function onPointerDown(e) { downX = e.clientX; downY = e.clientY; }
function onPointerUp(e) {
  if (Math.hypot(e.clientX - downX, e.clientY - downY) > 6) return; // 拖拽不当点击
  const rect = renderer.domElement.getBoundingClientRect();
  pointer.x = ((e.clientX - rect.left) / rect.width) * 2 - 1;
  pointer.y = -((e.clientY - rect.top) / rect.height) * 2 + 1;
  raycaster.setFromCamera(pointer, camera);
  const hits = raycaster.intersectObjects([bodiesG, citiesG, shipsG], true);
  for (const h of hits) {
    let o = h.object;
    while (o && !o.userData.kind) o = o.parent;
    if (!o) continue;
    if (window.PlanetXMap.onSelect) window.PlanetXMap.onSelect({ kind: o.userData.kind, name: o.userData.name });
    return;
  }
}

// --- render loop ------------------------------------------------------------
function tick() {
  requestAnimationFrame(tick);
  controls.update();
  const t = performance.now() * 0.001;
  for (const s of spinners) s.mesh.rotation.y = (t * s.speed) % (Math.PI * 2);
  updateMarkers();
  renderer.render(scene, camera);
}

// --- public API -------------------------------------------------------------
function init(container) {
  scene = new THREE.Scene();
  scene.background = new THREE.Color(0x060a16);

  camera = new THREE.PerspectiveCamera(50, 1, 1, 6000);
  camera.position.set(160, 190, 120);

  renderer = new THREE.WebGLRenderer({ antialias: true });
  renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
  renderer.outputColorSpace = THREE.SRGBColorSpace;
  container.appendChild(renderer.domElement);

  controls = new OrbitControls(camera, renderer.domElement);
  controls.enableDamping = true;
  controls.dampingFactor = 0.08;
  controls.maxPolarAngle = Math.PI * 0.47; // 保持在地图平面之上

  raycaster = new THREE.Raycaster();
  pointer = new THREE.Vector2();

  // 只有一点点环境光：城市/舰模型的光**也**应该来自太阳，否则整幅地图又会变成
  // 「相机头灯」式的平光（那是行星 shader 修掉的那个毛病，别在这里重新引入）。
  scene.add(new THREE.AmbientLight(0x8890b0, 0.16));
  // 太阳点光源：decay=0 无距离衰减，整幅系统均匀受光（星际尺度下不做物理衰减）。
  // 行星的 shader 自算太阳方向漫反射；此灯负责照亮城市/舰模型（MeshStandardMaterial）。
  const sunLight = new THREE.PointLight(0xfff4e2, 1.5, 0, 0);
  sunLight.position.set(0, 0, 0);
  scene.add(sunLight);

  sun = makeSun();
  scene.add(sun);

  bodiesG = new THREE.Group();
  citiesG = new THREE.Group();
  shipsG = new THREE.Group();
  scene.add(bodiesG, citiesG, shipsG);
  scene.add(makeStars());

  renderer.domElement.addEventListener('pointerdown', onPointerDown);
  renderer.domElement.addEventListener('pointerup', onPointerUp);

  const resize = () => {
    const w = container.clientWidth || 800;
    const h = container.clientHeight || 600;
    camera.aspect = w / h;
    camera.updateProjectionMatrix();
    renderer.setSize(w, h);
  };
  resize();
  new ResizeObserver(resize).observe(container);

  tick();
}

function setWorld(world, visuals) {
  if (!scene || !world) return;
  currentWorld = world;
  currentVisuals = visuals || null;
  if (!fitted) { fitCamera(world); fitted = true; }
  else { scale = systemScale(world); }
  disposeGroup(bodiesG);
  disposeGroup(citiesG);
  disposeGroup(shipsG);
  clearSpinners();
  lodItems = [];
  labelItems = [];
  // 先算显示布局（含卫星让位），三个渲染层共用同一套显示位置；
  // 顺便把天体显示球登记成遮挡体，供标记的遮挡淡出使用。
  layout = computeLayout(world, visuals);
  occluders = [];
  layout.nodes.forEach((n) => occluders.push({ center: n.pos, radius: n.radius }));
  renderBodies(bodiesG, world, visuals, layout);
  renderCities(citiesG, world, visuals, layout);
  renderShips(shipsG, world, layout);
  updateMarkers();
  applyDebugQuery();
}

function resetView(world) {
  if (!scene || !world) return;
  fitCamera(world);
  fitted = true;
}

// 调试/取景：直接设定相机位置与目标点（辅助截图与视觉调优；控制面板不依赖它）。
function setView(pos, target) {
  if (!camera || !controls) return;
  if (Array.isArray(pos)) camera.position.set(pos[0], pos[1], pos[2]);
  if (Array.isArray(target)) controls.target.set(target[0], target[1], target[2]);
  controls.update();
}
function getView() {
  if (!camera || !controls) return null;
  return { pos: camera.position.toArray(), target: controls.target.toArray() };
}
// 天体在渲染世界坐标里的位置（供 setView 取景/调试）——用**显示布局**里的位置，
// 这样调试取景和画面上看到的球体永远一致（含卫星让位后的位置）。
function bodyPoint(name) {
  if (!currentWorld || !layout) return null;
  const node = layout.nodes.get(name);
  if (!node) return null;
  const v = node.pos;
  return { pos: [v.x, v.y, v.z], radius: node.radius };
}

// 调试取景：把相机放到「太阳朝天体的一侧」看它的受光面；dist 按天体半径逼近。
// hideLabels 为真时隐藏标签 sprite（近距离截图不挡画面），重建时自动恢复。
function focusBody(name, opts) {
  opts = opts || {};
  const bp = bodyPoint(name);
  if (!bp) return null;
  const p = bp.pos;
  const len = Math.hypot(p[0], p[1], p[2]) || 1;
  // 从天体指向太阳的方向（太阳在原点）。
  const dir = [-p[0] / len, -p[1] / len, -p[2] / len];
  const radius = bp.radius;
  const dist = opts.dist || (radius * 3.2);
  const cam = [p[0] + dir[0] * dist, p[1] + dir[1] * dist + dist * 0.28, p[2] + dir[2] * dist];
  setView(cam, p);
  // 隐藏/恢复标签（Sprite）与城市/舰标记（Mesh），便于近距离检查行星表面。
  if (opts.hideLabels && bodiesG) bodiesG.traverse((o) => { if (o.isSprite) o.visible = false; });
  if (opts.hideMarkers) {
    if (citiesG) citiesG.visible = false;
    if (shipsG) shipsG.visible = false;
  }
  return { name, pos: p, radius, cam };
}

// 视觉调参的运行时入口（截图调优/事后微调都用它，不用改源码刷新）。
//   PlanetXMap.tune({ radiusScale: 1.2, orbitExp: 0.45 })
// 改完会自动重建世界；`PlanetXMap.tuning` 是当前生效值。
function tune(patch) {
  Object.assign(TUNING, patch || {});
  if (currentWorld) {
    scale = systemScale(currentWorld);     // 轨道指数变了 → 世界尺度也要重算
    setWorld(currentWorld, currentVisuals);
  }
  return { ...TUNING };
}

// --- 截图/调参用的 URL 参数（开发用，不影响正常操作） ------------------------
//   ?focus=地球&dist=18        取景某个天体（dist 为世界单位，缺省按半径自动）
//   ?view=160,190,120@0,0,0    直接给相机位置与目标点
//   ?tune=radiusScale:1.2,orbitExp:0.45   覆盖视觉调参（见 TUNING）
//   ?hide=labels,markers       隐藏标签/城市舰标记（看星球本体时用）
// 只在页面加载后的**第一次** setWorld 生效一次，之后不再干预相机。
function readDebugQuery() {
  const q = new URLSearchParams(location.search);
  const out = { applied: false, focus: q.get('focus'), view: q.get('view') };
  out.dist = q.has('dist') ? Number(q.get('dist')) : null;
  if (q.has('tune')) {
    q.get('tune').split(',').forEach((kv) => {
      const [k, v] = kv.split(':');
      if (k && v !== undefined && Object.prototype.hasOwnProperty.call(TUNING, k)) TUNING[k] = Number(v);
    });
  }
  const hide = (q.get('hide') || '').split(',').filter(Boolean);
  out.hideLabels = hide.indexOf('labels') >= 0;
  out.hideMarkers = hide.indexOf('markers') >= 0;
  return out;
}
const DEBUG_Q = (typeof location !== 'undefined') ? readDebugQuery() : { applied: true };
function applyDebugQuery() {
  if (DEBUG_Q.applied) return;
  DEBUG_Q.applied = true;
  if (DEBUG_Q.view) {
    const [p, t] = DEBUG_Q.view.split('@');
    setView(p.split(',').map(Number), t ? t.split(',').map(Number) : null);
  } else if (DEBUG_Q.focus) {
    focusBody(DEBUG_Q.focus, {
      dist: DEBUG_Q.dist, hideLabels: DEBUG_Q.hideLabels, hideMarkers: DEBUG_Q.hideMarkers,
    });
    return;
  }
  if (DEBUG_Q.hideLabels && bodiesG) bodiesG.traverse((o) => { if (o.isSprite) o.visible = false; });
  if (DEBUG_Q.hideMarkers) {
    if (citiesG) citiesG.visible = false;
    if (shipsG) shipsG.visible = false;
  }
}

window.PlanetXMap = {
  init, setWorld, resetView, setView, getView, bodyPoint, focusBody, tune,
  tuning: TUNING,
  debug: () => ({
    lod: lodItems.length, labels: labelItems.length, occluders: occluders.length,
  }),
};
