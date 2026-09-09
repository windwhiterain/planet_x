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
// 视觉是**数据驱动**的：每个 state 天体带一个 `kind` key（config body_kinds 的键），
// 本模块据 bodyKinds[body.kind] 解析出颜色/尺寸/类别/星环/着色器分支，不再内联猜测。
// 若 CDN 加载失败，本模块整体失败，但 app.js 的控制面板不受影响。

import * as THREE from 'three';
import { OrbitControls } from 'three/addons/controls/OrbitControls.js';

let scene, camera, renderer, controls, raycaster, pointer;
let bodiesG, citiesG, shipsG, spinners = [];
let fitted = false;   // 相机是否已按首帧适配（advance 不再重置视角）
let scale = 110;      // 世界单位：最远天体径向压缩后 ≈ `scale` 单位
let currentWorld = null;
let currentVisuals = null;
let sun = null;

// 兜底的天体类型（config body_kinds 缺该项/未传时用）：中性岩石外观。
const DEFAULT_KIND = {
  label: '天体', class: 'rock', color: '#8f9bb3', accent: '#5d6678',
  atmosphere: '#1a1a22', radius: 0.6, banded: false, emissive: 0.0,
  roughness: 1.0, metalness: 0.05,
};

// --- 颜色助手 ------------------------------------------------
function hex2rgb(hex) {
  if (!hex) return [0.6, 0.6, 0.7];
  const h = hex.replace('#', '');
  const full = h.length === 3 ? h.split('').map((c) => c + c).join('') : h;
  const n = parseInt(full, 16);
  if (Number.isNaN(n)) return [0.6, 0.6, 0.7];
  return [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255];
}

// --- 坐标：径向压缩 + 世界缩放 ---------------------------------------------
// 天体在 AU 里跨度很大（内行星 ~0.3，外行星 ~40），直接按线性会挤成一团。
// 用 r' = r^0.5 做径向压缩（保方向、只改半径），再整体缩放到 `scale`。
function compressRadius(r) {
  return Math.sqrt(r);
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
    if (b && b.orbit && b.orbit.aphelion_distance) maxAphe = Math.max(maxAphe, b.orbit.aphelion_distance);
  });
  return 110 / Math.max(compressRadius(maxAphe), 1e-3);
}

// 由 Orbit 参数在给定 months 计算天体位置（与 Rust Orbit::position 同一套 Kepler 求解）。
function orbitPositionAt(orbit, months) {
  const peri = orbit.perihelion_distance;
  const aphe = orbit.aphelion_distance;
  const a = (peri + aphe) / 2;
  const e = aphe > peri ? (aphe - peri) / (aphe + peri) : 0;
  const period = Math.max(orbit.period, 1e-9) || 1;
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
  const apheDir = normalize2(orbit.aphelion_direction || [1, 0]);
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

// 行星顶点着色器：把对象空间位置、世界法线、世界坐标传给片元。
const PLANET_VERT = `
  varying vec3 vNormal;
  varying vec3 vWorldPos;
  varying vec3 vObjPos;
  void main(){
    vNormal = normalize(normalMatrix * normal);
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

    vec3 surface = surfaceColor(sph);

    // 颜色 = 表面×(环境+漫反射) + 自发光 + 大气辉光 + 高光。
    vec3 col = surface * (0.28 + 0.72 * diff);
    col += surface * uEmissive * 0.6;
    float rim = pow(1.0 - max(dot(n, viewDir), 0.0), 3.5);
    col += uAtmo * rim * (0.35 + 0.65 * diff);
    vec3 hv = normalize(lightDir + viewDir);
    col += vec3(1.0) * pow(max(dot(n, hv), 0.0), 24.0) * (1.0 - uRough) * 0.25 * diff;

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
      uClass: { value: classIndex(spec.class) },
    },
  });
}

// 星环材质：半透明环带（土星/天王星），带径向 alpha 渐变 + 细密环缝（卡西尼缝式暗隙）。
function ringMaterial(rgb) {
  const [r, g, b] = rgb;
  return new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    side: THREE.DoubleSide,
    vertexShader: `
      varying vec2 vUv;
      void main(){ vUv = uv; gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }
    `,
    fragmentShader: `
      varying vec2 vUv;
      uniform vec3 uCol;
      void main(){
        float t = vUv.x;                    // 0=内缘 … 1=外缘
        float alpha = smoothstep(0.0, 0.14, t) * (1.0 - smoothstep(0.82, 1.0, t));
        alpha *= 0.62 + 0.22 * sin(t * 60.0);   // 细密的环缝
        alpha *= 1.0 - 0.65 * smoothstep(0.40, 0.44, t) * (1.0 - smoothstep(0.46, 0.50, t)); // 卡西尼缝
        gl_FragColor = vec4(uCol, alpha);
      }
    `,
    uniforms: { uCol: { value: new THREE.Vector3(r, g, b) } },
  });
}

// 把 hex 提亮一点（星环用比行星更亮的冰色）。
function lighten(hex, amt) {
  const [r, g, b] = hex2rgb(hex).map((v) => Math.min(v + amt, 1.0));
  return [r, g, b];
}

// --- 标签 ----------------------------------------------------------------
function makeLabel(text, color = '#cdd6f4', fontPx = 42) {
  const pad = 12;
  const c = document.createElement('canvas');
  const font = `${fontPx}px system-ui, "Segoe UI", sans-serif`;
  const ctx = c.getContext('2d');
  ctx.font = font;
  const w = Math.max(Math.ceil(ctx.measureText(text).width) + pad * 2, 32);
  const h = fontPx + pad * 2;
  c.width = w;
  c.height = h;
  const ctx2 = c.getContext('2d');
  ctx2.font = font;
  ctx2.fillStyle = color;
  ctx2.textBaseline = 'middle';
  ctx2.textAlign = 'left';
  ctx2.fillText(text, pad, h / 2);
  const tex = new THREE.CanvasTexture(c);
  tex.minFilter = THREE.LinearFilter;
  const mat = new THREE.SpriteMaterial({ map: tex, depthTest: false, transparent: true });
  const sp = new THREE.Sprite(mat);
  sp.scale.set(w / 12, h / 12, 1);
  return sp;
}

// --- 动态对象重建 ------------------------------------------------------------
function disposeGroup(g) {
  if (!g) return;
  g.traverse((o) => {
    if (o.geometry) o.geometry.dispose();
    if (o.material) {
      if (Array.isArray(o.material)) o.material.forEach((m) => { m.dispose(); if (m.map) m.map.dispose(); });
      else { o.material.dispose(); if (o.material.map) o.material.map.dispose(); }
    }
  });
  while (g.children.length) g.remove(g.children[0]);
}
function clearSpinners() {
  spinners = [];
}

// 天体显示半径：由类型表 radius（相对类地行星）乘一个基准，短边钳到可读。
// 带定居点（宜居/有城）的天体略大，好容纳城市标记。
function bodyRadius(body, spec) {
  const base = 2.4 * (spec.radius || 0.6);
  const hab = body.settlements && body.settlements.length ? 0.9 : 0.0;
  return Math.max(base + hab, 0.9);
}

function specFor(visuals, body) {
  return (visuals && visuals[body.kind]) || DEFAULT_KIND;
}

function orbitLine(orbit) {
  const pts = [];
  const period = Math.max(orbit.period, 1e-6);
  const n = 160;
  for (let i = 0; i <= n; i++) {
    const t = (i / n) * period;
    const p = orbitPositionAt(orbit, t);
    const [cx, cz] = compress(p);
    pts.push(new THREE.Vector3(cx * scale, 0, cz * scale));
  }
  const g = new THREE.BufferGeometry().setFromPoints(pts);
  const m = new THREE.LineBasicMaterial({ color: 0x3b4a75, transparent: true, opacity: 0.5 });
  return new THREE.LineLoop(g, m);
}

function facColorFor(world, fid) {
  const f = world.factions.find((x) => x.id === fid);
  return (f && f.color) || '#8f9bb3';
}

function addRing(parent, position, r, colorHex) {
  const inner = r * 1.45;
  const outer = r * 2.75;
  const geo = new THREE.RingGeometry(inner, outer, 128, 1);
  const [cr, cg, cb] = lighten(colorHex || '#c9b08a', 0.18);
  const mesh = new THREE.Mesh(geo, ringMaterial([cr, cg, cb]));
  mesh.rotation.x = -Math.PI / 2;          // 铺平到地图平面（XZ）
  mesh.position.copy(position);            // 放到该天体（而非太阳）处
  parent.add(mesh);
}

function renderBodies(group, world, visuals) {
  world.bodies.forEach((b) => {
    const spec = specFor(visuals, b);
    const p = wp(b.position);
    const r = bodyRadius(b, spec);
    const geo = new THREE.SphereGeometry(r, 48, 32);
    const mat = planetMaterial(spec);
    const mesh = new THREE.Mesh(geo, mat);
    mesh.position.copy(p);
    mesh.userData = { kind: 'body', name: b.name };
    group.add(mesh);

    // 缓慢自转（只转表面 shader 采样可见的球体）；气态/类地/冰巨星转得更明显。
    if (spec.class === 'gas' || spec.class === 'ice' || spec.class === 'terran' || spec.class === 'venus') {
      spinners.push({ mesh, speed: (spec.class === 'terran' || spec.class === 'venus') ? 0.004 : 0.0025 });
    }

    // 星环（intrinsic body 属性：土星/天王星）。
    if (b.ring) addRing(group, p, r, spec.accent);

    group.add(orbitLine(b.orbit));

    const lbl = makeLabel(b.name, b.settlements && b.settlements.length ? '#e2f3ff' : '#9fb4d8', 40);
    lbl.position.set(p.x, p.y + r + 5, p.z);
    group.add(lbl);
  });
}

function renderCities(group, world) {
  const byBody = {};
  world.cities.forEach((c) => { (byBody[c.body_id] = byBody[c.body_id] || []).push(c); });
  Object.entries(byBody).forEach(([bodyName, cities]) => {
    const body = world.bodies.find((b) => b.name === bodyName);
    if (!body) return;
    const p = wp(body.position);
    const off = bodyRadius(body, specFor(null, body)) + 3.6;
    cities.forEach((c, idx) => {
      const ang = (idx / cities.length) * Math.PI * 2;
      const px = p.x + Math.cos(ang) * off;
      const pz = p.z + Math.sin(ang) * off;
      const geo = new THREE.BoxGeometry(3.2, 3.2, 3.2);
      const mat = new THREE.MeshStandardMaterial({ color: new THREE.Color(facColorFor(world, c.faction_id)), roughness: 0.4, metalness: 0.3 });
      const mesh = new THREE.Mesh(geo, mat);
      mesh.position.set(px, 1.6, pz);
      mesh.userData = { kind: 'city', name: c.name };
      group.add(mesh);
    });
  });
}

function renderShips(group, world) {
  world.ships.forEach((s) => {
    const p = wp(s.position);
    const geo = new THREE.IcosahedronGeometry(1.4, 0);
    const mat = new THREE.MeshStandardMaterial({ color: new THREE.Color(facColorFor(world, s.faction_id)), roughness: 0.35, metalness: 0.5 });
    const mesh = new THREE.Mesh(geo, mat);
    mesh.position.set(p.x, 2.8, p.z);
    mesh.userData = { kind: 'ship', name: s.name };
    group.add(mesh);
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
    new THREE.SphereGeometry(3.2, 48, 32),
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
  const haloSp = new THREE.Sprite(new THREE.SpriteMaterial({ map: tex, color: 0xffd98a, transparent: true, opacity: 0.85, depthWrite: false, blending: THREE.AdditiveBlending }));
  haloSp.scale.set(66, 66, 1);

  const g = new THREE.Group();
  g.add(core);
  g.add(haloSp);
  return g;
}

// --- 相机适配 ---------------------------------------------------------------
function fitCamera(world) {
  scale = systemScale(world);
  const s = 110;
  camera.position.set(s * 1.5, s * 1.75, s * 1.05);
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

  scene.add(new THREE.AmbientLight(0x8890b0, 0.5));
  // 太阳点光源：decay=0 无距离衰减，整幅系统均匀受光（星际尺度下不做物理衰减）。
  // 行星的 shader 自算太阳方向漫反射；此灯主要照亮城市/舰标记。
  const sunLight = new THREE.PointLight(0xfff2d0, 1.5, 0, 0);
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
  renderBodies(bodiesG, world, visuals);
  renderCities(citiesG, world);
  renderShips(shipsG, world);
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
// 天体在渲染世界坐标里的位置（供 setView 取景/调试）。
function bodyPoint(name) {
  if (!currentWorld) return null;
  const b = currentWorld.bodies.find((x) => x.name === name);
  if (!b) return null;
  const v = wp(b.position);
  return { pos: [v.x, v.y, v.z], radius: bodyRadius(b, specFor(currentVisuals, b)) };
}

// 调试取景：把相机放到「太阳朝天体的一侧」看它的受光面；dist 按天体半径逼近。
// hideLabels 为真时隐藏标签 sprite（近距离截图不挡画面），重建时自动恢复。
function focusBody(name, opts) {
  opts = opts || {};
  if (!currentWorld) return null;
  const b = currentWorld.bodies.find((x) => x.name === name);
  if (!b) return null;
  const v = wp(b.position);
  const p = [v.x, v.y, v.z];
  const len = Math.hypot(p[0], p[1], p[2]) || 1;
  // 从天体指向太阳的方向（太阳在原点）。
  const dir = [-p[0] / len, -p[1] / len, -p[2] / len];
  const spec = specFor(currentVisuals, b);
  const radius = bodyRadius(b, spec);
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

window.PlanetXMap = { init, setWorld, resetView, setView, getView, bodyPoint, focusBody };
