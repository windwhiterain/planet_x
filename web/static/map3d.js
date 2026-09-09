// 行星X WebUI — 3D 地图渲染器（three.js）。
//
// 独立 ES module，通过 importmap 从 CDN 加载 three.js；用 WebGL 渲染太阳系沙盘：
// 太阳 + 真实轨道（Orbit 参数采样，径向压缩以让内/外行星都可读）+ 天体球 +
// 城市/舰队标记，支持旋转/平移/缩放（OrbitControls）与拾取。与 app.js（经典脚本，
// 控制面板）通过 window.PlanetXMap 这一小接口解耦：
//
//   PlanetXMap.init(container)  创建渲染器/场景/相机/灯光（只调一次）
//   PlanetXMap.setWorld(world)  用 /api/state 的结果重建动态对象（天体/城/舰）
//   PlanetXMap.resetView(world) 重新适配相机（新游戏时调用）
//   PlanetXMap.onSelect = fn    用户点击天体/城市/舰时收到 {kind, name}
//
// 若 CDN 加载失败，本模块整体失败，但 app.js 的控制面板不受影响。

import * as THREE from 'three';
import { OrbitControls } from 'three/addons/controls/OrbitControls.js';

let scene, camera, renderer, controls, raycaster, pointer;
let bodiesG, citiesG, shipsG;
let fitted = false;   // 相机是否已按首帧适配（advance 不再重置视角）
let scale = 110;      // 世界单位：最远天体径向压缩后 ≈ `scale` 单位
let sun = null;

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
function systemCompressedRadius(world) {
  let max = 1e-6;
  world.bodies.forEach((b) => {
    if (b && Array.isArray(b.position)) max = Math.max(max, compressRadius(Math.hypot(b.position[0], b.position[1])));
  });
  return max;
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

function bodyRadius(body) {
  // 用名字哈希给一个稳定、略有差别的半径；带定居点的天体略大。
  const h = [...body.name].reduce((a, ch) => (a * 31 + ch.charCodeAt(0)) >>> 0, 7) % 100;
  const base = 2.6 + (h / 100) * 2.8;
  return body.settlements && body.settlements.length ? base + 1.4 : base;
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

function renderBodies(group, world) {
  world.bodies.forEach((b) => {
    const p = wp(b.position);
    const r = bodyRadius(b);
    const hue = ([...b.name].reduce((a, ch) => (a * 31 + ch.charCodeAt(0)) >>> 0, 7) % 360);
    const col = new THREE.Color().setHSL(hue / 360, 0.5, 0.52);
    const geo = new THREE.SphereGeometry(r, 28, 20);
    const mat = new THREE.MeshStandardMaterial({ color: col, roughness: 0.75, metalness: 0.05 });
    const mesh = new THREE.Mesh(geo, mat);
    mesh.position.copy(p);
    mesh.userData = { kind: 'body', name: b.name };
    group.add(mesh);

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
    const off = bodyRadius(body) + 3.4;
    cities.forEach((c, idx) => {
      const ang = (idx / cities.length) * Math.PI * 2;
      const px = p.x + Math.cos(ang) * off;
      const pz = p.z + Math.sin(ang) * off;
      const geo = new THREE.BoxGeometry(3.4, 3.4, 3.4);
      const mat = new THREE.MeshStandardMaterial({ color: new THREE.Color(facColorFor(world, c.faction_id)), roughness: 0.4, metalness: 0.3 });
      const mesh = new THREE.Mesh(geo, mat);
      mesh.position.set(px, 1.7, pz);
      mesh.userData = { kind: 'city', name: c.name };
      group.add(mesh);
    });
  });
}

function renderShips(group, world) {
  world.ships.forEach((s) => {
    const p = wp(s.position);
    const geo = new THREE.IcosahedronGeometry(1.5, 0);
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

function makeSun() {
  const geo = new THREE.SphereGeometry(3.2, 32, 24);
  const mat = new THREE.MeshBasicMaterial({ color: 0xffe37a });
  const mesh = new THREE.Mesh(geo, mat);
  const haloMat = new THREE.SpriteMaterial({ color: 0xffd766, transparent: true, opacity: 0.5, depthWrite: false });
  const haloSp = new THREE.Sprite(haloMat);
  haloSp.scale.set(34, 34, 1);
  const g = new THREE.Group();
  g.add(mesh);
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

function setWorld(world) {
  if (!scene || !world) return;
  if (!fitted) { fitCamera(world); fitted = true; }
  else { scale = systemScale(world); }
  disposeGroup(bodiesG);
  disposeGroup(citiesG);
  disposeGroup(shipsG);
  renderBodies(bodiesG, world);
  renderCities(citiesG, world);
  renderShips(shipsG, world);
}

function resetView(world) {
  if (!scene || !world) return;
  fitCamera(world);
  fitted = true;
}

window.PlanetXMap = { init, setWorld, resetView };
