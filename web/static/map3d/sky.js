// 行星X WebUI — 程序化星空 / 银河带 / 星云。
//
// **分成两半**，因为两者的分辨率需求差了三个数量级：
//
//   ① 银河带 + 星云 → 烘成一张 HDR cubemap（`bakeSky`）。
//      它们是**低频**的（云状结构），512/面就够，而且永远不动 —— 初始化时用 `CubeCamera`
//      渲一次 6 面，之后 `scene.background` 直接采样，每帧成本 = 一次背景采样。
//
//   ② 恒星 → **真的几何**（`createStarfield`，一个 `THREE.Points`，一次 draw call）。
//      为什么不做进 cubemap：恒星必须落在「一个屏幕像素」上，而 cubemap 是**固定角度分辨率**
//      的贴图——768/面 覆盖 90°，在 50° 视场 800px 高的屏幕上等于每面 1440px，也就是被放大
//      1.9 倍。烘进去的星会被放大成一坨并暴露出立方体的纹素。走 `gl_PointSize` 就没有这个
//      问题：点的大小直接以**帧缓冲像素**为单位，缩放到哪都是 1–4 px 的锐利星点。
//      （这是踩过的坑：第一版把星烘进 cubemap，结果整片天空是一片白色方块雪花。）
//
// 全部程序生成：没有贴图、没有模型、没有网络资源。星的颜色走**黑体色温**（O/B 蓝白、
// K/M 橙红），亮度走幂律（绝大多数暗、极少数亮），最亮的那批带十字衍射。

import * as THREE from 'three';
import { NOISE_GLSL, fbmOct, rnd } from './util.js';

// ---------------------------------------------------------------------------
// ① 银河带 / 星云（低频，烘 cubemap）
// ---------------------------------------------------------------------------
const SKY_VERT = /* glsl */`
  varying vec3 vDir;
  void main(){
    vDir = position;
    gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
  }
`;

const SKY_FRAG = /* glsl */`
  precision highp float;
  ${NOISE_GLSL}
  varying vec3 vDir;

  // 银道极（随便取一个不与坐标轴平行的方向，让银河斜着穿过天空）。
  const vec3 GAL_N = vec3(0.3180, 0.8000, -0.5080);

  void main(){
    vec3 d = normalize(vDir);
    vec3 col = vec3(0.0);

    vec3 gu = normalize(cross(GAL_N, vec3(0.0, 0.0, 1.0)));
    vec3 gv = cross(GAL_N, gu);
    float lat = abs(dot(d, GAL_N));
    // 银道面：|sin(b)| 的高斯型包络。
    float band = exp(-pow(lat / 0.19, 1.7));
    // 银心方向（银经 0 附近最亮最厚）。
    float coreness = exp(-pow(length(vec2(dot(d, gu), dot(d, gv) - 1.0)) / 0.85, 2.0));
    // 云状结构：域扰动 fbm，再用 ridged 抠出暗尘带。
    vec3 wp = warp(d * 5.5, 1.35, 0.0);
    float cloud = fbm(wp * 1.7);
    float dust = ridged(d * 7.5 + 3.1);
    // 暗尘带要**黑得下去**（对照 scratch/ref/milkyway-core.jpg：银河最抓人的是亮星云
    // 与黑尘带的强对比，而不是一层均匀的灰雾）。
    float bright = band * (0.10 + 0.90 * cloud) * (1.0 - 0.88 * smoothstep(0.28, 0.86, dust));
    // 银河的色：银心偏暖黄（老年星族），外围偏冷蓝（年轻星族 + 尘埃散射）。
    vec3 galCol = mix(vec3(0.42, 0.52, 0.86), vec3(1.05, 0.92, 0.70), coreness * 0.85 + 0.12);
    col += galCol * bright * 0.165;
    // 弥漫的银道面辉光（不带结构的底光）。
    col += vec3(0.28, 0.33, 0.55) * band * 0.020;

    // --- 星云：几团定点 + fbm 调制；发射线配色（Hα 红 / OIII 青 / 反射星云蓝）------
    // 中心是**固定常量**，不是随机——星空每次加载必须一模一样，截图才可比。
    const vec3 NEB[3] = vec3[3](
      vec3(0.86, 0.31, 0.24), vec3(-0.62, 0.44, 0.65), vec3(0.15, -0.78, 0.60)
    );
    const vec3 NEB_COL[3] = vec3[3](
      vec3(1.00, 0.24, 0.30),   // Hα
      vec3(0.22, 0.90, 0.78),   // OIII
      vec3(0.35, 0.48, 1.00)    // 反射星云
    );
    for (int i = 0; i < 3; i++) {
      vec3 nc = normalize(NEB[i]);
      float dd = length(d - nc);
      float fall = exp(-pow(dd / 0.42, 2.0));
      if (fall > 0.002) {
        vec3 nq = warp(d * 9.0 + float(i) * 21.0, 2.2, 0.0);
        float fil = ridged(nq * 1.4);
        float wisp = smoothstep(0.42, 0.95, fil) * (0.35 + 0.65 * fbm(nq * 3.1));
        col += NEB_COL[i] * fall * wisp * 0.075;
      }
    }

    // 极暗的「未分辨恒星」底噪：真实长曝光下天空从不全黑。
    col += vec3(0.014, 0.017, 0.028);

    gl_FragColor = vec4(col, 1.0);
  }
`;

/**
 * 烘一张程序化银河/星云 cubemap（**不含**恒星，恒星走 `createStarfield`）。
 * @param {THREE.WebGLRenderer} renderer
 * @param {number} res 每面分辨率（512 已经够：银河是低频的）
 */
export function bakeSky(renderer, res) {
  const R = Math.max(64, res | 0);
  const rt = new THREE.WebGLCubeRenderTarget(R, {
    type: THREE.HalfFloatType,
    format: THREE.RGBAFormat,
    generateMipmaps: true,
    minFilter: THREE.LinearMipmapLinearFilter,
    magFilter: THREE.LinearFilter,
  });
  // 显式声明线性：星空 shader 写出的是**线性 HDR**，不能再被当 sRGB 解码一次。
  rt.texture.colorSpace = THREE.LinearSRGBColorSpace;

  const mat = new THREE.ShaderMaterial({
    vertexShader: SKY_VERT,
    fragmentShader: SKY_FRAG,
    side: THREE.BackSide,
    depthTest: false,
    depthWrite: false,
    uniforms: { uFbmOct: fbmOct(4) },
  });
  const geo = new THREE.SphereGeometry(1, 48, 32);
  const mesh = new THREE.Mesh(geo, mat);

  const bakeScene = new THREE.Scene();
  bakeScene.add(mesh);

  // 烘制期间必须关掉色调映射——否则 HDR 星空会被 ACES 压一次，再在后处理里压第二次。
  const prevTone = renderer.toneMapping;
  const prevTarget = renderer.getRenderTarget();
  renderer.toneMapping = THREE.NoToneMapping;
  const cam = new THREE.CubeCamera(0.1, 10, rt);
  cam.update(renderer, bakeScene);
  renderer.toneMapping = prevTone;
  renderer.setRenderTarget(prevTarget);

  geo.dispose();
  mat.dispose();

  return {
    texture: rt.texture,
    dispose() { rt.dispose(); },
  };
}

// ---------------------------------------------------------------------------
// ② 恒星（几何，一次 draw call）
// ---------------------------------------------------------------------------
const STAR_VERT = /* glsl */`
  attribute float aSize;
  attribute vec3  aColor;
  uniform float uPixelRatio;
  uniform float uScale;
  varying vec3 vCol;
  varying float vSize;
  void main(){
    vCol = aColor;
    vSize = aSize;
    vec4 mv = modelViewMatrix * vec4(position, 1.0);
    gl_Position = projectionMatrix * mv;
    // 恒定屏幕像素：不做距离衰减。uPixelRatio 抵消 DPR 缩放，星点在 HiDPI 下不变粗。
    gl_PointSize = max(1.0, aSize * uPixelRatio * uScale);
  }
`;

const STAR_FRAG = /* glsl */`
  precision highp float;
  varying vec3 vCol;
  varying float vSize;
  void main(){
    vec2 p = gl_PointCoord * 2.0 - 1.0;
    float r2 = dot(p, p);
    if (r2 > 1.0) discard;
    float core = exp(-r2 * 4.2);
    // 只有大点（=亮星）才画十字衍射，与「人眼/望远镜看亮星」一致。
    float spike = 0.0;
    if (vSize > 2.0) {
      vec2 ap = abs(p);
      spike = (exp(-ap.x * 16.0) * exp(-ap.y * 1.8) + exp(-ap.y * 16.0) * exp(-ap.x * 1.8));
      spike *= smoothstep(2.0, 3.4, vSize) * 0.32;
    }
    gl_FragColor = vec4(vCol * (core + spike), 1.0);
  }
`;

// 黑体色温 → 线性 RGB（与 util.js 里的 GLSL 版本同一套拟合）。
function blackbodyJS(k) {
  const t = Math.min(Math.max(k, 1000), 40000) / 100;
  let r, g, b;
  if (t <= 66) r = 255; else r = 329.698727446 * Math.pow(Math.max(t - 60, 1e-3), -0.1332047592);
  if (t <= 66) g = 99.4708025861 * Math.log(Math.max(t, 1)) - 161.1195681661;
  else g = 288.1221695283 * Math.pow(Math.max(t - 60, 1e-3), -0.0755148492);
  if (t >= 66) b = 255;
  else if (t <= 19) b = 0;
  else b = 138.5177312231 * Math.log(Math.max(t - 10, 1e-3)) - 305.0447927307;
  return [(r / 255), (g / 255), (b / 255)].map((v) => Math.pow(Math.min(Math.max(v, 0), 1), 2.2));
}

const _GAL_N = new THREE.Vector3(0.3180, 0.8000, -0.5080).normalize();

/**
 * 造一片恒星（一个 `THREE.Points`）。
 * @param {number} count 恒星数量
 * @param {number} radius 天球半径（世界单位；只要远大于太阳系即可，深度测试会照常遮挡）
 */
export function createStarfield(count, radius = 2200) {
  const N = Math.max(0, count | 0);
  const pos = new Float32Array(N * 3);
  const col = new Float32Array(N * 3);
  const siz = new Float32Array(N);

  const gu = new THREE.Vector3().crossVectors(_GAL_N, new THREE.Vector3(0, 0, 1)).normalize();
  const gv = new THREE.Vector3().crossVectors(_GAL_N, gu);
  const d = new THREE.Vector3();

  for (let i = 0; i < N; i++) {
    if (rnd('sf', 'band', i) < 0.44) {
      // 44% 集中在银道面附近（三次均匀相加 ≈ 高斯），让恒星也描出银河的形状。
      const l = rnd('sf', 'l', i) * Math.PI * 2;
      const b = (rnd('sf', 'b1', i) + rnd('sf', 'b2', i) + rnd('sf', 'b3', i) - 1.5) * 0.30;
      d.set(0, 0, 0)
        .addScaledVector(gu, Math.cos(l) * Math.cos(b))
        .addScaledVector(gv, Math.sin(l) * Math.cos(b))
        .addScaledVector(_GAL_N, Math.sin(b));
    } else {
      // 其余均匀分布在天球上。
      const z = rnd('sf', 'z', i) * 2 - 1;
      const a = rnd('sf', 'a', i) * Math.PI * 2;
      const rr = Math.sqrt(Math.max(0, 1 - z * z));
      d.set(rr * Math.cos(a), z, rr * Math.sin(a));
    }
    d.normalize().multiplyScalar(radius);
    pos[i * 3] = d.x; pos[i * 3 + 1] = d.y; pos[i * 3 + 2] = d.z;

    // 幂律星等：pow(...,5) 之后绝大多数星很暗、极少数很亮。
    const m = Math.pow(rnd('sf', 'm', i), 5.0);
    const T = 2700 + 13500 * Math.pow(rnd('sf', 't', i), 1.7);
    const c = blackbodyJS(T);
    const bright = 0.12 + 1.35 * m;
    col[i * 3] = c[0] * bright;
    col[i * 3 + 1] = c[1] * bright;
    col[i * 3 + 2] = c[2] * bright;
    siz[i] = 0.85 + 3.1 * Math.pow(m, 1.7);
  }

  const geo = new THREE.BufferGeometry();
  geo.setAttribute('position', new THREE.BufferAttribute(pos, 3));
  geo.setAttribute('aColor', new THREE.BufferAttribute(col, 3));
  geo.setAttribute('aSize', new THREE.BufferAttribute(siz, 1));
  geo.boundingSphere = new THREE.Sphere(new THREE.Vector3(), radius * 1.01);
  geo.userData.shared = true;

  const mat = new THREE.ShaderMaterial({
    vertexShader: STAR_VERT,
    fragmentShader: STAR_FRAG,
    uniforms: {
      uPixelRatio: { value: 1 },
      uScale: { value: 1 },
    },
    transparent: true,
    blending: THREE.AdditiveBlending,
    depthWrite: false,
    depthTest: true,
  });
  mat.userData.shared = true;

  const points = new THREE.Points(geo, mat);
  points.frustumCulled = false;
  points.userData.shared = true;

  return {
    object: points,
    setPixelRatio(pr) { mat.uniforms.uPixelRatio.value = pr; },
    setScale(s) { mat.uniforms.uScale.value = s; },
    dispose() { geo.dispose(); mat.dispose(); },
  };
}
