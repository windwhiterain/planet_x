// 行星X WebUI — 3D 地图渲染器（three.js）。**公共入口**。
//
// 与 app.js（经典脚本，控制面板）通过 `window.PlanetXMap` 这一小接口解耦，契约逐字不变：
//
//   PlanetXMap.init(container)              创建渲染器/场景/相机/灯光（只调一次）
//   PlanetXMap.setWorld(world, bodyKinds, cfg)  用 /api/state 重建动态对象
//       —— 第三个参数是**新增的可选**参数（config 根）：舰的剪影要按 config.ships 的
//          hull/armor_mult/speed_mult/... 算出来，所以把它一起传进来。不传就退回默认值，
//          旧调用照样能跑。
//   PlanetXMap.resetView(world)             重新适配相机（新游戏时调用）
//   PlanetXMap.onSelect = fn                用户点击天体/城市/舰时收到 {kind, name}
//   另有 setView / getView / bodyPoint / focusBody / tune / tuning / debug（调试与截图用）
//   新增（不改旧契约）：setQuality(name) / quality() / select(kind, name)
//
// 模块划分（本目录）：
//   tuning.js   视觉调参 + 质量分档表
//   layout.js   轨道数学（Kepler / 径向压缩 / ≤8° 倾角）+ 显示布局
//   kinds.js    config 天体类型 → 视觉参数
//   sky.js      程序化星空/银河/星云 → 烘成 HDR cubemap
//   sun.js      光球/色球/日冕/日珥
//   planet.js   行星表面/云层/大气壳/星环
//   models.js   程序化舰/城/站/小行星带
//   markers.js  恒定像素标记 + 解析式遮挡 + 标签
//   postfx.js   HDR 后处理管线
//
// 几条贯穿全模块的视觉原则（改之前先读）：
//   * **光是太阳给的**：所有天体/模型的光都来自原点那盏太阳。环境光只留一点点，否则整幅
//     地图会退化成「相机头灯」式的平光（那是 bug，不是风格）。
//   * **阵营色只出现在 UI 层**：城市/舰模型一律中性航天器灰，身份由恒定屏幕尺寸的准星环 +
//     标签色点表达。
//   * **标记的遮挡要看得见**：标记精灵不做深度测试（避免被球面切边），改为每帧解析式
//     射线-球相交决定淡出——空间感靠这个，不靠 z-buffer。
//   * **亮的东西真的比白更亮**：太阳/大气边缘/城市灯/引擎羽流在 HDR 里都 >1，辉光由
//     UnrealBloom 产生，而不是在物体外画一圈半透明橙色。

import * as THREE from 'three';
import { INC, loadShaders } from './glsl.js';
import { OrbitControls } from 'three/addons/controls/OrbitControls.js';

import { TUNING, TIERS, detectTier, parseTier } from './tuning.js';
import { disposeGroup, hashStr } from './util.js';
import {
  computeLayout, systemScale, orbitPoints, compress, compressRadius,
  cityDir, specFor, planeFor,
} from './layout.js';
import { axialTilt, spinSpeed, variantOf } from './kinds.js';
import { planetMaterial, createAtmosphere, createClouds, createRing, planeQuaternion, CITY_MAX } from './planet.js';
import { createSun } from './sun.js';
import { bakeSky, createStarfield } from './sky.js';
import { createPostFX } from './postfx.js';
import { createMarkers, makeLabel } from './markers.js';
import { buildShip, buildGroundCity, buildStation, buildBelt, tickPlumes } from './models.js';

// --- 模块状态 ---------------------------------------------------------------
let scene, camera, renderer, controls, raycaster, pointer;
let bodiesG, citiesG, shipsG, beltsG;
let sun = null, sky = null, post = null, markers = null, stars = null;
let tier = TIERS.high, tierName = 'high';
let scale = 110;
let layout = null;
let currentWorld = null;
let currentVisuals = null;
let currentCfg = null;
let fitted = false;
let occluders = [];
let spinners = [];            // 自转：改的是着色器的 uSpin，不是 mesh.rotation
let cloudSpinners = [];
let orbitHeads = [];          // { mat, name } —— 每帧把「天体当前位置」喂给轨道线
let tweenItems = [];          // 回合推进时的位置过渡
let tweenFollowers = [];      // 轨道线（要跟着天体的过渡位移整体平移）
let tweenT = 1;
const TWEEN_DUR = 0.55;
let prevPos = new Map();
let started = false;
let containerEl = null;
let timeS = 0;
let lastT = 0;

// 自适应分辨率
const perf = { samples: [], lastAdjust: 0, dprScale: 1, warmup: 0 };
let baseDpr = 1;
const PERF = { slowMs: 26, fastMs: 12.5, minScale: 0.6, cooldownMs: 1400 };

// --- 调试 URL ---------------------------------------------------------------
//   ?focus=地球&dist=18           取景某个天体（dist 为世界单位，缺省按半径自动）
//   ?view=160,190,120@0,0,0       直接给相机位置与目标点
//   ?tune=radiusScale:1.2         覆盖视觉调参（见 TUNING）
//   ?hide=labels,markers          隐藏标签/城市舰标记
//   ?q=ultra|high|medium|low      强制质量档（缺省按 GPU 自动探测）
// 只在页面加载后的**第一次** setWorld 生效一次（`?hide`/`?q` 除外，它们是常驻开关）。
function readDebugQuery() {
  const q = new URLSearchParams(location.search);
  const out = { applied: false, focus: q.get('focus'), view: q.get('view'), q: q.get('q') };
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
const DEBUG_Q = (typeof location !== 'undefined')
  ? readDebugQuery()
  : { applied: true, q: null, hideLabels: false, hideMarkers: false };

// 隐藏标记/标签必须是**常驻**的：`markers.update()` 每帧都会按遮挡重写 `visible`，只在
// 隐藏那一刻置 false 会被下一帧覆盖（旧版 `?hide=labels` 就是这么失效的）。
let labelsHidden = false;
let markersHidden = false;

// --- 质量 -------------------------------------------------------------------
function applyTier(name, opts = {}) {
  const t = TIERS[name];
  if (!t) return;
  tierName = name;
  tier = t;
  if (scene) scene.backgroundIntensity = TUNING.skyIntensity;
  if (renderer) {
    perf.dprScale = 1;
    baseDpr = Math.min(window.devicePixelRatio || 1, t.dprCap);
    renderer.setPixelRatio(baseDpr);
    renderer.toneMappingExposure = TUNING.exposure;
  }
  if (post) { post.setTier(t); post.applyTuning(); }
  if (sun) sun.setTier(t);
  if (scene && opts.sky !== false) rebuildSky();
  if (scene) {
    // 几何分段 / 噪声八度都编在材质里 ⇒ 必须重建整个动态世界。
    resizeRenderer();
    if (currentWorld) buildWorld(currentWorld, currentVisuals, currentCfg, { snap: true });
  }
}

// GLSL 现在是**分文件 fetch** 装进来的（见 glsl.js）。装完之前任何烘焙都会当场编译着色器
// 而编译失败，所以本函数在 ready 之前是**空操作**（init 末尾那条链会在 ready 之后烘一次；
// 质量档切换那条路径也走同一个守卫）。
let shadersReady = false;

function rebuildSky() {
  if (!renderer || !scene || !shadersReady) return;
  if (sky) { try { sky.dispose(); } catch (e) { /* 忽略 */ } }
  sky = bakeSky(renderer, tier.skyRes);
  scene.background = sky.texture;
  scene.backgroundIntensity = TUNING.skyIntensity;
  // 恒星是几何（不是烘进 cubemap）——见 sky.js 顶部的推导。
  if (stars) { stars.dispose(); scene.remove(stars.object); stars = null; }
  stars = createStarfield(tier.stars || 0, 2200);
  stars.setPixelRatio(renderer.getPixelRatio());
  scene.add(stars.object);
}

function resizeRenderer() {
  if (!renderer || !containerEl) return;
  const w = containerEl.clientWidth || 800;
  const h = containerEl.clientHeight || 600;
  camera.aspect = w / h;
  camera.updateProjectionMatrix();
  renderer.setSize(w, h);
  if (post) post.setSize(w, h);
}

// --- 轨道线 -----------------------------------------------------------------
// 画成 additive 的发光细丝：天体**当前所在处**有一个亮「彗头」，越靠近越亮；被行星/太阳
// 挡住时由深度测试自然消失（这一层是几何，不是 UI 标记，所以它**要**参与深度测试）。
const ORBIT_VERT = INC('px/misc/orbit.vert');
const ORBIT_FRAG = INC('px/misc/orbit.frag');
function orbitLineMesh(pts, rgb, alpha = 0.30) {
  const geo = new THREE.BufferGeometry().setFromPoints(pts);
  let rmax = 1;
  const c = pts.length ? pts[0] : new THREE.Vector3();
  for (const p of pts) rmax = Math.max(rmax, p.distanceTo(c));
  const mat = new THREE.ShaderMaterial({
    uniforms: {
      uColor: { value: new THREE.Vector3(rgb[0], rgb[1], rgb[2]) },
      uHead: { value: new THREE.Vector3() },
      uHeadSharp: { value: 1 / Math.pow(Math.max(1.2, rmax * 0.085), 2) },
      uOpacity: { value: alpha },
      uHeadGain: { value: alpha * 3.4 },
    },
    vertexShader: ORBIT_VERT,
    fragmentShader: ORBIT_FRAG,
    transparent: true,
    blending: THREE.AdditiveBlending,
    depthWrite: false,
    depthTest: true,
  });
  const line = new THREE.Line(geo, mat);
  line.userData = { kind: 'orbit' };
  return { line, radius: rmax };
}
function hexLin(hex) {
  const h = String(hex || '#3f5688').replace('#', '');
  const n = parseInt(h.length === 3 ? h.split('').map((x) => x + x).join('') : h, 16);
  const s = (x) => (x <= 0.04045 ? x / 12.92 : Math.pow((x + 0.055) / 1.055, 2.4));
  return [s(((n >> 16) & 255) / 255), s(((n >> 8) & 255) / 255), s((n & 255) / 255)];
}
function factionColorFor(world, fid) {
  const f = (world.factions || []).find((x) => (x.id || x.势力) === fid);
  return (f && f.颜色) || '#8f9bb3';
}
function hashFloat(a, b) {
  return (hashStr(String(a) + '|' + String(b)) >>> 8) / 16777216;
}

// --- 场景构建 ---------------------------------------------------------------
const shipHolders = new Map();

function buildWorld(world, visuals, cfg, opts = {}) {
  if (!scene || !world) return;
  const snap = !!opts.snap;
  currentWorld = world;
  currentVisuals = visuals || null;
  currentCfg = cfg || currentCfg;

  // ⚠ 顺序要紧：`scale` 必须先算出来，再交给 `computeLayout` —— 卫星让位判据里
  // `bodyRadius` 是**世界单位**，而压缩平面坐标是无量纲的，不乘 scale 就永远比不出来。
  scale = systemScale(world);
  layout = computeLayout(world, visuals, scale);
  if (!fitted) { fitCamera(world); fitted = true; }

  disposeGroup(bodiesG);
  disposeGroup(citiesG);
  disposeGroup(shipsG);
  disposeGroup(beltsG);
  spinners = [];
  cloudSpinners = [];
  orbitHeads = [];
  tweenFollowers = [];
  shipHolders.clear();
  if (markers) markers.clear();

  layout = computeLayout(world, visuals, scale);
  occluders = [];
  layout.nodes.forEach((n) => occluders.push({ center: n.pos, radius: n.radius }));
  markers.setOccluders(occluders);

  const newPrev = new Map();
  buildBodies(world, visuals, newPrev);
  buildCities(world, visuals);
  buildShips(world, newPrev);
  buildBelts();

  if (snap) { tweenItems = []; tweenT = 1; }
  else setupTween(world);

  prevPos = newPrev;
  applyDebugQuery();
}

function setupTween(world) {
  tweenItems = [];
  const delta = new THREE.Vector3();
  layout.nodes.forEach((n, name) => {
    const prev = prevPos.get(name);
    if (!n.holder || !prev) return;
    delta.copy(prev).sub(n.pos);
    if (delta.lengthSq() < 1e-6) return;
    const d = delta.clone();
    tweenItems.push({ obj: n.holder, base: n.pos.clone(), delta: d, node: n });
    (n.cityHolders || []).forEach((ch) => {
      tweenItems.push({ obj: ch, base: ch.userData.base.clone(), delta: d.clone(), lod: ch.userData.lod });
    });
  });
  (world.ships || []).forEach((s) => {
    const holder = shipHolders.get(s.舰名);
    const prev = prevPos.get('ship:' + s.舰名);
    if (!holder || !prev) return;
    const base = holder.userData.base;
    const d = prev.clone().sub(base);
    if (d.lengthSq() < 1e-6) return;
    tweenItems.push({ obj: holder, base: base.clone(), delta: d, lod: holder.userData.lod });
  });
  tweenT = tweenItems.length ? 0 : 1;
}

function buildBodies(world, visuals, newPrev) {
  const capsByBody = {};
  (world.factions || []).forEach((f) => {
    if (!f.capital_body) return;
    (capsByBody[f.capital_body] = capsByBody[f.capital_body] || []).push(f.颜色);
  });

  (world.bodies || []).forEach((b) => {
    const spec = specFor(visuals, b);
    const node = layout.nodes.get(b.天体名);
    if (!node) return;
    const p = node.pos;
    const r = node.radius;
    const plane = layout.planes.get(b.天体名);
    const variant = variantOf(spec);

    // 每个天体一个 holder：属于它的东西全挂在 holder 上，位置过渡时只动 holder。
    const holder = new THREE.Group();
    holder.position.copy(p);
    holder.userData.base = p.clone();
    node.holder = holder;
    newPrev.set(b.天体名, p.clone());
    bodiesG.add(holder);

    // 朝向：轨道平面 + 轴向倾角。自转在着色器里（uSpin），所以这里只放倾角。
    const pq = planeQuaternion(plane);
    const tilt = new THREE.Quaternion().setFromAxisAngle(
      new THREE.Vector3(1, 0, 0), axialTilt(b.天体名, hashFloat),
    );
    const orient = pq.clone().multiply(tilt);
    node.orient = orient;

    const ringGeom = b.星环 ? {
      inner: r * TUNING.ringInner,
      outer: r * TUNING.ringOuter,
      normal: new THREE.Vector3(0, 1, 0).applyQuaternion(orient),
    } : null;

    const mat = planetMaterial(spec, tier, { ring: ringGeom });
    const mesh = new THREE.Mesh(new THREE.SphereGeometry(r, tier.seg[0], tier.seg[1]), mat);
    mesh.quaternion.copy(orient);
    mesh.userData = { kind: 'body', name: b.天体名 };
    holder.add(mesh);

    spinners.push({ mat, speed: spinSpeed(spec), phase: hashFloat(b.天体名, 'phase') * 6.283 });

    if (variant === 'Terran' || variant === 'Venus') {
      const clouds = createClouds(r, spec, tier);
      clouds.quaternion.copy(orient);
      holder.add(clouds);
      cloudSpinners.push({
        mat: clouds.material, speed: spinSpeed(spec) * 1.35,
        phase: hashFloat(b.天体名, 'cph') * 6.283,
      });
    }

    if (mat.userData.hasAtmo) holder.add(createAtmosphere(r, spec, tier));

    if (b.星环) {
      const ring = createRing(r, spec, tier, orient);
      holder.add(ring);
      node.ring = ring;
      // 行星在环上的投影要知道环的世界中心 —— holder 会在过渡里移动，所以每帧刷新。
      ring.onBeforeRender = () => {
        ring.material.uniforms.uCenter.value.copy(holder.position);
      };
    }

    // 夜面城市灯：这颗星上**真实城市**的对象空间方向。槽位由 buildCities 填。
    node.cityDirs = [];
    node.cityUniform = mat.uniforms.uCityDirs.value;
    node.cityCountUniform = mat.uniforms.uCityCount;

    // 轨道线。
    const anchor = b.轨道.母天体
      ? (((world.bodies || []).find((x) => x.天体名 === b.轨道.母天体) || {}).位置 || [0, 0])
      : [0, 0];
    const parentNode = b.轨道.母天体 ? layout.nodes.get(b.轨道.母天体) : null;
    const pts = orbitPoints(
      b.轨道, anchor, layout.orbitScale.get(b.天体名) || 1,
      plane, parentNode ? parentNode.pos : null, scale,
      tier.seg[0] > 56 ? 256 : 160,
    );
    const oc = hexLin(spec.atmosphere && spec.atmosphere !== '#141210' ? spec.atmosphere : '#4a5f96');
    const ol = orbitLineMesh(pts, [oc[0] * 0.7, oc[1] * 0.85, oc[2] * 1.2], 0.30);
    bodiesG.add(ol.line);
    tweenFollowers.push({ line: ol.line, holder });
    // 轨道线的**淡出**：相机钻到某个行星系里时，卫星的轨道环会横穿母星的脸，
    // 读起来像星球上的一道白色划痕（additive 细线压在亮盘面上格外显眼）。
    // 判据用「相机到该轨道中心的距离 / 轨道半径」——离得近就淡出。
    orbitHeads.push({
      mat: ol.line.material, name: b.天体名, radius: ol.radius,
      center: parentNode ? parentNode.pos : new THREE.Vector3(),
      base: 0.30,
    });

    // 标签。
    const lbl = makeLabel(b.天体名, '#dbe6ff', TUNING.labelPx, capsByBody[b.天体名] || []);
    bodiesG.add(lbl.sprite);
    node.label = markers.addLabel(lbl, p, r);
  });
}

function buildCities(world, visuals) {
  const byBody = {};
  (world.cities || []).forEach((c) => { (byBody[c.所在天体] = byBody[c.所在天体] || []).push(c); });
  Object.entries(byBody).forEach(([bodyName, cities]) => {
    const node = layout.nodes.get(bodyName);
    if (!node) return;
    const P = node.pos;
    const r = node.radius;
    // 城市高度刻意压得很小（≈行星半径的 8.5%）——它们是地表聚落，不是贴在行星上的巨型水晶。
    const s = Math.min(Math.max(r * 0.085, 0.030), 0.18);
    node.cityHolders = [];
    node.cityDirs = [];

    cities.forEach((c, idx) => {
      const color = factionColorFor(world, c.势力);
      // `local`/`orient` 都在**未倾斜的对象空间**里算；天体整体乘了 `node.orient`，所以
      // 两者都要跟着转一次——否则倾斜的天体上城市会浮在球面外、并且朝向错误。
      const q = node.orient;
      let model, local, orient = null, px, modelSize, spin = null, orientAxis = 'y';
      if (c.轨道空间站) {
        const az = (idx / Math.max(cities.length, 1)) * Math.PI * 2 + 1.7;
        const orbR = r * 1.55;
        local = new THREE.Vector3(Math.cos(az) * orbR, r * 0.5, Math.sin(az) * orbR);
        local.applyQuaternion(q);
        model = buildStation(c, { size: s * 0.75 });
        spin = model.userData.spinner || null;
        px = TUNING.farDotStationPx;
        modelSize = s * 1.1;
      } else {
        const dir = cityDir(idx, cities.length, 0.95);
        // 贴着地表（1.006 而不是 1.045）：城市模型是**平底**的，抬到 4.5% 半径高就会
        // 明显浮在球面上方，看起来像悬空的圆盘。
        const rb = r * 1.006;
        local = new THREE.Vector3(dir[0] * rb, dir[1] * rb, dir[2] * rb).applyQuaternion(q);
        orient = new THREE.Vector3(dir[0], dir[1], dir[2]).applyQuaternion(q).normalize().toArray();
        model = buildGroundCity(c, { size: s });
        px = TUNING.farDotPx;
        modelSize = s * 1.2;
        // 城市灯用的是**对象空间**方向（着色器里的 `d` 就是对象空间），不乘 q。
        if (node.cityDirs.length < CITY_MAX) {
          node.cityDirs.push(new THREE.Vector3(dir[0], dir[1], dir[2]).normalize());
        }
      }
      const world3 = new THREE.Vector3(P.x + local.x, P.y + local.y, P.z + local.z);
      const item = markers.add(citiesG, {
        kind: 'city', name: c.城名, color, model, local, orient, orientAxis,
        center: P, world: world3,
        shape: c.轨道空间站 ? 'reticle' : 'dot',
        px, modelSize, switchDist: Math.max(14, r * 22), spin,
      });
      if (item) {
        item.group.userData.base = world3.clone();
        item.group.userData.lod = item;
        node.cityHolders.push(item.group);
      }
    });

    // 把城市方向灌进行星着色器（空槽写 0 —— 着色器用 dot(cd,cd)<0.5 判空）。
    const u = node.cityUniform;
    if (u) {
      for (let i = 0; i < CITY_MAX; i++) {
        if (i < node.cityDirs.length) u[i].copy(node.cityDirs[i]);
        else u[i].set(0, 0, 0);
      }
      // 循环上界交给 uniform（见 PLANET_FRAG）：常量上界会把 12 次迭代全展开。
      if (node.cityCountUniform) node.cityCountUniform.value = node.cityDirs.length;
    }
  });
}

function buildShips(world, newPrev) {
  const shipKinds = (currentCfg && currentCfg.ships) || {};
  (world.ships || []).forEach((s) => {
    const p2 = compress(s.坐标 || [0, 0]);
    // 舰在星际空间里飞，不挂在任何天体下 —— 抬到轨道面之上一点点，避免和轨道线糊在一起。
    const lift = 0.55 + hashFloat(s.舰名, 'y') * 0.9;
    const p = new THREE.Vector3(p2[0] * scale, lift, p2[1] * scale);
    const color = factionColorFor(world, s.势力);
    const spec = shipKinds[s.舰级] || null;
    const model = buildShip(s.舰名, s.舰级, spec);

    // 朝向：有速度向量就沿速度；没有（引擎只给标量速率）就朝外（背离太阳）。
    let orientZ = null;
    const vel = s.速度;
    if (Array.isArray(vel) && (Math.abs(vel[0]) > 1e-9 || Math.abs(vel[1]) > 1e-9)) {
      const b2 = compress([(s.坐标[0] || 0) + vel[0] * 3.0, (s.坐标[1] || 0) + vel[1] * 3.0]);
      const d = new THREE.Vector3(b2[0] * scale - p.x, 0, b2[1] * scale - p.z);
      if (d.lengthSq() > 1e-12) orientZ = d.normalize().toArray();
    }
    if (!orientZ && p.lengthSq() > 1e-9) orientZ = p.clone().normalize().toArray();

    const item = markers.add(shipsG, {
      kind: 'ship', name: s.舰名, color, model,
      local: new THREE.Vector3(0, 0, 0),
      orient: orientZ, orientAxis: 'z',
      center: p, world: p.clone(),
      shape: 'diamond', px: TUNING.farDotPx, modelSize: shipModelSize(spec),
      switchDist: 30,
    });
    if (item) {
      shipHolders.set(s.舰名, item.group);
      item.group.userData.base = p.clone();
      item.group.userData.lod = item;
    }
    newPrev.set('ship:' + s.舰名, p.clone());
  });
}

// 舰的显示尺寸：与 models.js 的 shipParams 同一口径（config.hull → 世界长度）。
function shipModelSize(spec) {
  const hull = spec && spec.hull != null ? spec.hull : 20;
  return (0.10 + 0.0030 * Math.min(hull, 90)) * 0.95;
}

// --- 小行星带 / 柯伊伯带 -----------------------------------------------------
function buildBelts() {
  if (!tier.belt || TUNING.beltDensity <= 0) return;
  const k = tier.belt * TUNING.beltDensity;
  const mk = (a, b, cnt, thick, seed, size, color) => {
    const inner = compressRadius(a) * scale;
    const outer = compressRadius(b) * scale;
    if (!(outer > inner)) return;
    const n = Math.max(0, Math.round(cnt * k));
    const mesh = buildBelt({
      count: n, inner, outer, thickness: thick, seed,
      size: size * (scale / 16), color, sizeSpread: 2.6,
    });
    if (mesh) beltsG.add(mesh);
  };
  // 主带 2.1–3.3 AU；柯伊伯带 29.5–49 AU（更稀、更厚、更冷色）。
  //
  // ⚠ 尺寸是这里唯一的坑：岩石半径原来是 0.17–0.5 世界单位，而行星显示半径才 2.2
  // ——等于把「小行星」做成了地球半径的 23%，凑近看全是巨石阵。真实比例下小行星当然
  // 看不见，所以取一个**折中**：0.05 半径 ⇒ 系统视角下约 1.7 像素，读起来是尘埃带；
  // 凑近才是几块小石头。数量补上去，否则带会太稀。
  mk(2.10, 3.30, 2400, 3.0, 'mainbelt', 0.050, 0x4c4740);
  mk(29.5, 49.0, 1300, 9.0, 'kuiper', 0.070, 0x4a515e);
}

// --- 相机适配 ---------------------------------------------------------------
function fitCamera(world) {
  scale = systemScale(world);
  let maxR = 0;
  (world.bodies || []).forEach((b) => {
    const node = layout && layout.nodes.get(b.天体名);
    if (node) maxR = Math.max(maxR, node.pos.length());
  });
  const R = TUNING.fitR > 0 ? TUNING.fitR : Math.max(48, maxR * TUNING.fitMargin);
  const fov = camera.fov * Math.PI / 180;
  const aspect = camera.aspect || 1.6;
  const halfW = Math.tan(fov / 2) * aspect;
  const dist = R / (halfW * 0.78);
  const dir = new THREE.Vector3(0.60, 0.70, 0.42).normalize();
  camera.position.copy(dir.multiplyScalar(dist));
  camera.near = Math.max(0.05, dist * 0.0012);
  camera.far = Math.max(8000, dist * 12);   // 必须 > 星天球半径（2200）
  camera.updateProjectionMatrix();
  controls.target.set(0, 0, 0);
  controls.update();
}

// --- 太阳的屏幕位置与可见度 --------------------------------------------------
// 体积光/拉丝/鬼影都要「太阳在屏幕哪、有没有被挡住」。遮挡用**解析求交**（相机→太阳的线段
// × 天体显示球），不接 composer 的深度纹理——少一层脆弱的管线，O(天体数) 成本可忽略。
const _sunW = new THREE.Vector3();
const _camDir = new THREE.Vector3();
const _seg = new THREE.Vector3();
const _toC = new THREE.Vector3();
function computeSunScreen() {
  _sunW.set(0, 0, 0).project(camera);
  const sx = _sunW.x * 0.5 + 0.5;
  const sy = _sunW.y * 0.5 + 0.5;
  camera.getWorldDirection(_camDir);
  // 太阳在相机背后 → 关掉全部镜头效果（否则会在屏幕上翻到反方向去）。
  if (_camDir.dot(new THREE.Vector3(0, 0, 0).sub(camera.position)) <= 0) {
    return { x: sx, y: sy, vis: 0 };
  }
  let vis = 1;
  _seg.set(0, 0, 0).sub(camera.position);
  const tMax = _seg.length();
  if (tMax > 1e-6) {
    _seg.divideScalar(tMax);
    for (const o of occluders) {
      _toC.copy(o.center).sub(camera.position);
      const tc = _toC.dot(_seg);
      if (tc <= 0 || tc >= tMax) continue;
      const d2 = _toC.lengthSq() - tc * tc;
      const r2 = o.radius * o.radius;
      if (d2 >= r2) continue;
      const cov = (o.radius - Math.sqrt(Math.max(d2, 0))) / o.radius;
      vis = Math.min(vis, 1 - THREE.MathUtils.smoothstep(cov, -0.25, 0.6));
    }
  }
  return { x: sx, y: sy, vis: Math.max(0, vis) };
}

// --- 拾取 / 交互 -------------------------------------------------------------
let downX = 0, downY = 0, hoverRaf = 0;
function pickAt(clientX, clientY) {
  const rect = renderer.domElement.getBoundingClientRect();
  pointer.x = ((clientX - rect.left) / rect.width) * 2 - 1;
  pointer.y = -((clientY - rect.top) / rect.height) * 2 + 1;
  raycaster.setFromCamera(pointer, camera);
  const hits = raycaster.intersectObjects([bodiesG, citiesG, shipsG], true);
  for (const h of hits) {
    let o = h.object;
    while (o && !(o.userData && o.userData.kind)) o = o.parent;
    if (!o) continue;
    return { kind: o.userData.kind, name: o.userData.name };
  }
  return null;
}
function onPointerDown(e) { downX = e.clientX; downY = e.clientY; }
function onPointerUp(e) {
  if (Math.hypot(e.clientX - downX, e.clientY - downY) > 6) return;   // 拖拽不当点击
  const hit = pickAt(e.clientX, e.clientY);
  if (hit && window.PlanetXMap.onSelect) window.PlanetXMap.onSelect(hit);
}
function onPointerMove(e) {
  if (hoverRaf) return;
  const cx = e.clientX, cy = e.clientY;
  hoverRaf = requestAnimationFrame(() => {
    hoverRaf = 0;
    if (!markers || !renderer) return;
    const hit = pickAt(cx, cy);
    markers.hover(hit ? hit.name : null, hit ? hit.kind : null);
    renderer.domElement.style.cursor = hit ? 'pointer' : '';
  });
}

// --- 调参入口 ---------------------------------------------------------------
function tune(patch) {
  Object.assign(TUNING, patch || {});
  if (currentWorld && renderer) {
    scale = systemScale(currentWorld);
    renderer.toneMappingExposure = TUNING.exposure;
    // `applyTuning` 只灌 uniform；**开不开某个 pass** 在 setTier 里，两个都要调
    // （否则 `?tune=postfx:0` 这种「关掉整条后处理」的开关完全没反应——实测踩过）。
    if (post) { post.setExposure(TUNING.exposure); post.applyTuning(); post.setTier(tier); }
    // 倾角/尺度变了 → 世界布局要重算；但相机保持不动（不然调参会跳视角）。
    const keep = getView();
    buildWorld(currentWorld, currentVisuals, currentCfg, { snap: true });
    if (keep) setView(keep.pos, keep.target);
  }
  return { ...TUNING };
}

// --- render loop ------------------------------------------------------------
function tick() {
  requestAnimationFrame(tick);
  const now = performance.now() * 0.001;
  const dt = Math.min(0.05, (now - lastT) || 0.016);
  lastT = now;
  timeS = now;

  controls.update();

  // 位置过渡（回合推进时天体沿自己的轨道平滑滑过去，而不是瞬移）。
  if (tweenT < 1) {
    tweenT = Math.min(1, tweenT + dt / TWEEN_DUR);
    const e = tweenT < 0.5 ? 4 * tweenT ** 3 : 1 - Math.pow(-2 * tweenT + 2, 3) / 2;
    const k = 1 - e;
    for (const it of tweenItems) {
      it.obj.position.copy(it.base).addScaledVector(it.delta, k);
      if (it.lod && it.lod.pos) it.lod.pos.copy(it.obj.position);
      if (it.node && it.node.label) it.node.label.center.copy(it.obj.position);
    }
    // 轨道线整体跟着天体的位移平移，否则推进的 0.55 s 里天体会「离开」自己的轨道线。
    for (const f of tweenFollowers) {
      if (!f.holder || !f.holder.userData.base) continue;
      f.line.position.copy(f.holder.position).sub(f.holder.userData.base);
    }
  }

  // 自转 / 云 / 日冕 / 羽流的时间。
  // **自转先关掉**（用户裁决）：未来的自转应当由 **state 提供**（每回合的相位），
  // 不该由前端按墙钟自己推 —— 墙钟推的话，暂停、切档、不同机器上都不一致，
  // 而且它一旦和法线出错叠在一起（见 planet.js 气巨那条分支修掉的 bug），
  // 现象会伪装成「光源在自转」，极难查。
  // 现在把 uSpin 冻结在各天体自己的 phase 上：保留「每颗星朝向不同」的多样性，
  // 但不随时间转。`speed` 字段先留着，等 state 接上再决定去留。
  for (const s of spinners) s.mat.uniforms.uSpin.value = s.phase;
  for (const c of cloudSpinners) {
    c.mat.uniforms.uSpin.value = c.phase;
    c.mat.uniforms.uTime.value = timeS;      // 云自己的漂移照旧（那是 uTime，不是自转）
  }
  if (sun) sun.update(timeS, camera, post ? post.depthTarget : null);   // 深度：日冕积分靠它夹断
  tickPlumes(timeS);

  // 轨道线的彗头跟住天体当前（可能还在过渡中）的位置。
  for (const oh of orbitHeads) {
    const node = layout && layout.nodes.get(oh.name);
    const p = node && node.holder ? node.holder.position : null;
    if (!p) continue;
    oh.mat.uniforms.uHead.value.copy(p);
    // 淡出（见 buildBodies 里的说明）。
    const ratio = camera.position.distanceTo(oh.center) / Math.max(oh.radius, 0.01);
    const fade = THREE.MathUtils.smoothstep(ratio, 1.25, 3.6);
    oh.mat.uniforms.uOpacity.value = oh.base * (0.06 + 0.94 * fade);
    oh.mat.uniforms.uHeadGain.value = oh.base * 3.4 * (0.06 + 0.94 * fade);
  }

  markers.update(camera, renderer, timeS);
  applyHideFlags();

  const si = computeSunScreen();
  if (post) {
    post.setSun(si.x, si.y, si.vis);
    post.setTime(timeS);
  }

  adaptive(dt);

  renderer.info.reset();
  if (post) post.render(dt);
  else renderer.render(scene, camera);
}

function applyHideFlags() {
  if (labelsHidden) for (const lb of markers.labels) lb.sp.visible = false;
  if (markersHidden) { citiesG.visible = false; shipsG.visible = false; }
  else { citiesG.visible = true; shipsG.visible = true; }
}

function adaptive(dt) {
  if (perf.warmup < 90) { perf.warmup++; return; }   // 跳过着色器编译/首帧抖动
  perf.samples.push(dt);
  if (perf.samples.length > 90) perf.samples.shift();
  if (!tier.adaptive || !TUNING.adaptive) return;
  const now = performance.now();
  if (perf.samples.length < 60 || now - perf.lastAdjust < PERF.cooldownMs) return;
  const avg = perf.samples.reduce((a, b) => a + b, 0) / perf.samples.length;
  const ms = avg * 1000;
  const minScale = Math.max(0.35, Math.min(1, TUNING.adaptiveMinScale));
  if (ms > PERF.slowMs && perf.dprScale > minScale) {
    perf.dprScale = Math.max(minScale, perf.dprScale - 0.12);
    applyResolution();
    perf.lastAdjust = now;
    perf.samples.length = 0;
  } else if (ms < PERF.fastMs && perf.dprScale < 1) {
    perf.dprScale = Math.min(1, perf.dprScale + 0.08);
    applyResolution();
    perf.lastAdjust = now;
    perf.samples.length = 0;
  }
}
function applyResolution() {
  if (!renderer || !containerEl) return;
  renderer.setPixelRatio(baseDpr * perf.dprScale);
  if (stars) stars.setPixelRatio(renderer.getPixelRatio());
  resizeRenderer();
}

// --- init -------------------------------------------------------------------
// ⚠ **必须保持同步**。app.js 是 `initMap()`（同步）→ … → `renderMap()`（拉完 state 之后）
// → `setWorld()` 这样调下来的。一旦这里变成 async，`setWorld` 就会在 scene/renderer 还没
// 建起来时跑 —— 实测症状是 `bodies=0`、**行星一个都不显示**（只剩星云和日冕）。
// GLSL 分文件带来的异步，改用「同步建场 + 把依赖 GLSL 的两步挂到 ready 上」消化，
// 见本函数末尾那段。
function init(container) {
  if (started) return;
  started = true;
  containerEl = container;

  scene = new THREE.Scene();

  camera = new THREE.PerspectiveCamera(50, 1, 0.1, 6000);
  camera.position.set(160, 190, 120);

  renderer = new THREE.WebGLRenderer({
    antialias: false,          // AA 交给 composer 的 MSAA render target（见 postfx.js）
    powerPreference: 'high-performance',
    stencil: false,
  });
  renderer.outputColorSpace = THREE.SRGBColorSpace;
  // 一帧里 composer 会 render 好几次（场景 + 每个全屏 pass）；不自清才能累加出**整帧**的
  // draw call / 三角形数（否则 debug() 永远读到最后一个全屏 quad 的 1）。
  renderer.info.autoReset = false;
  // 色调映射在后期由 OutputPass 执行，但 `renderer.toneMapping` 是它的**开关**，必须在这里设。
  renderer.toneMapping = THREE.ACESFilmicToneMapping;
  renderer.toneMappingExposure = TUNING.exposure;
  container.appendChild(renderer.domElement);

  // 质量档：URL 强制 > 按 GPU 自动探测。
  const det = detectTier(renderer, window.devicePixelRatio || 1);
  tierName = parseTier(DEBUG_Q.q) || det.tier;
  tier = TIERS[tierName] || TIERS.high;
  baseDpr = Math.min(window.devicePixelRatio || 1, tier.dprCap);
  renderer.setPixelRatio(baseDpr);

  controls = new OrbitControls(camera, renderer.domElement);
  controls.enableDamping = true;
  controls.dampingFactor = 0.08;
  controls.maxPolarAngle = Math.PI * 0.47;   // 保持在地图平面之上
  controls.minDistance = 0.5;
  controls.zoomSpeed = 0.85;
  controls.rotateSpeed = 0.7;

  raycaster = new THREE.Raycaster();
  pointer = new THREE.Vector2();

  // 只有一点点环境光：城市/舰的光**也**应该来自太阳，否则整幅地图又会变成「相机头灯」。
  scene.add(new THREE.AmbientLight(0x8890b0, 0.13));
  // 太阳点光源：decay=0 无距离衰减，整幅系统均匀受光（星际尺度下不做物理衰减）。
  const sunLight = new THREE.PointLight(0xfff4e2, 1.7, 0, 0);
  scene.add(sunLight);

  sun = createSun(tier);
  scene.add(sun.group);

  bodiesG = new THREE.Group();
  citiesG = new THREE.Group();
  shipsG = new THREE.Group();
  beltsG = new THREE.Group();
  scene.add(beltsG, bodiesG, citiesG, shipsG);

  markers = createMarkers();
  // 注意这里**没有** rebuildSky()：它会当场烘焙天空（渲染进立方体贴图 ⇒ 立刻编译着色器），
  // 必须等 GLSL 装完。见本函数末尾那条 ready 链。

  const w = container.clientWidth || 800;
  const h = container.clientHeight || 600;
  camera.aspect = w / h;
  camera.updateProjectionMatrix();
  renderer.setSize(w, h);
  post = createPostFX(renderer, tier, { w, h });
  post.setScene(scene, camera);
  // 深度预趟**不**包含日冕自己 —— 否则它会挡住自己（预趟用的是覆盖材质，会照写深度）。
  post.setDepthExclude([sun.group]);
  post.setExposure(TUNING.exposure);
  post.applyTuning();

  renderer.domElement.addEventListener('pointerdown', onPointerDown);
  renderer.domElement.addEventListener('pointerup', onPointerUp);
  renderer.domElement.addEventListener('pointermove', onPointerMove);
  renderer.domElement.addEventListener('pointerleave', () => { if (markers) markers.hover(null); });

  new ResizeObserver(() => {
    if (stars) stars.setPixelRatio(renderer.getPixelRatio());
    resizeRenderer();
  }).observe(container);
  window.addEventListener('resize', resizeRenderer);

  // --- 依赖 GLSL 的两步，串在同一个 ready 上 ---------------------------------
  //   ① rebuildSky()：bakeSky 立刻渲染进立方体贴图 ⇒ **当场编译**天空着色器
  //   ② tick()：首帧会编译**其余全部**材质
  // 两者都不允许抢在 chunk 装完之前。装载失败就明确报出来并停摆 —— 缺一个 chunk 的症状
  // 是 three 抛「Can not resolve #include <...>」，比这里报得晚得多、也难懂得多。
  loadShaders().then(() => {
    shadersReady = true;
    rebuildSky();
    tick();
  }).catch((e) => {
    // eslint-disable-next-line no-console
    console.error('[map3d] GLSL 装载失败，地图无法渲染：', e);
  });
}

// --- debug query 应用（只在第一次 setWorld 后生效一次）-----------------------
function applyDebugQuery() {
  if (DEBUG_Q.applied) return;
  DEBUG_Q.applied = true;
  if (DEBUG_Q.view) {
    const [p, t] = DEBUG_Q.view.split('@');
    setView(p.split(',').map(Number), t ? t.split(',').map(Number) : null);
  } else if (DEBUG_Q.focus) {
    focusBody(DEBUG_Q.focus, { dist: DEBUG_Q.dist });
  }
  if (DEBUG_Q.hideLabels) labelsHidden = true;
  if (DEBUG_Q.hideMarkers) markersHidden = true;
}

// --- public API -------------------------------------------------------------
function setWorld(world, visuals, cfg) {
  if (!scene || !world) return;
  buildWorld(world, visuals, cfg);
}

function resetView(world) {
  if (!scene || !world) return;
  currentWorld = world;
  // ⚠ **不要**在这里重算 layout。`setWorld` 已经把 holder（天体组）/ `cityUniform`
  // （夜面城市灯的 uniform 数组）/ 标签引用全都挂在 layout 的节点上；换一份新 layout 会
  // 让它们统统失联——现象是 `bodyPoint` 回到未过渡的位置、轨道彗头不再跟随、
  // `debug().cityLights` 变回 0。旧版正是在这里重算，于是 `resetView` 变成了
  // 「必须在 setWorld 之前调，否则地图半残」的顺序陷阱（app.js 恰好是这个顺序，所以
  // 一直没暴露）。现在只做它名字说的事：把相机重新适配。
  if (!layout) {
    scale = systemScale(world);
    layout = computeLayout(world, currentVisuals, scale);
    occluders = [];
    layout.nodes.forEach((n) => occluders.push({ center: n.pos, radius: n.radius }));
    if (markers) markers.setOccluders(occluders);
  }
  fitCamera(world);
  fitted = true;
}

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

function bodyPoint(name) {
  if (!currentWorld || !layout) return null;
  const node = layout.nodes.get(name);
  if (!node) return null;
  const v = node.holder ? node.holder.position : node.pos;
  return { pos: [v.x, v.y, v.z], radius: node.radius };
}

// 调试取景：把相机放到「太阳朝天体的一侧」看它的受光面；dist 按天体半径逼近。
function focusBody(name, opts = {}) {
  const bp = bodyPoint(name);
  if (!bp) return null;
  const p = bp.pos;
  const len = Math.hypot(p[0], p[1], p[2]) || 1;
  const dir = [-p[0] / len, -p[1] / len, -p[2] / len];
  const radius = bp.radius;
  const dist = opts.dist || (radius * 3.2);
  const cam = [p[0] + dir[0] * dist, p[1] + dir[1] * dist + dist * 0.28, p[2] + dir[2] * dist];
  setView(cam, p);
  if (opts.hideLabels) labelsHidden = true;
  if (opts.hideMarkers) markersHidden = true;
  return { name, pos: p, radius, cam };
}

function setQuality(name) {
  if (!TIERS[name]) return { tier: tierName, available: Object.keys(TIERS) };
  applyTier(name);
  return { tier: tierName, def: tier };
}

function debugInfo() {
  const avg = perf.samples.length
    ? perf.samples.reduce((a, b) => a + b, 0) / perf.samples.length : 0;
  const gpu = (() => {
    try {
      const gl = renderer.getContext();
      const d = gl.getExtension('WEBGL_debug_renderer_info');
      return d ? gl.getParameter(d.UNMASKED_RENDERER_WEBGL) : 'n/a';
    } catch (e) { return 'n/a'; }
  })();
  const base = markers ? markers.count() : { lod: 0, labels: 0, occluders: 0 };
  return {
    ...base,
    tier: tierName,
    gpu,
    fps: avg > 0 ? Math.round(1 / avg) : 0,
    frameMs: +(avg * 1000).toFixed(2),
    dprScale: +perf.dprScale.toFixed(2),
    pixelRatio: renderer ? renderer.getPixelRatio() : 0,
    drawCalls: renderer ? renderer.info.render.calls : 0,
    triangles: renderer ? renderer.info.render.triangles : 0,
    programs: renderer && renderer.info.programs ? renderer.info.programs.length : 0,
    bodies: layout ? layout.nodes.size : 0,
    // 夜面城市灯的槽位总数（>0 才说明 uCityDirs 真的灌进了着色器）。
    cityLights: layout
      ? [...layout.nodes.values()].reduce((a, n) => a + ((n.cityDirs && n.cityDirs.length) || 0), 0)
      : 0,
  };
}

window.PlanetXMap = {
  init, setWorld, resetView, setView, getView, bodyPoint, focusBody, tune,
  tuning: TUNING,
  setQuality,
  select: (kind, name) => { if (markers) markers.select(name, kind); },
  quality: () => ({ tier: tierName, def: tier, available: Object.keys(TIERS) }),
  debug: debugInfo,
};
