// 行星X WebUI — 程序化模型：舰 / 地面城 / 轨道站 / 小行星带。
//
// **没有一个顶点来自下载的资源文件**：全部由 `BufferGeometry` 图元按游戏数据拼出来。
//
// 两条设计原则（继承自旧版，别推翻）：
//   1. **阵营色不上模型**。舰/城一律「中性航天器灰」——身份由 UI 层表达（恒定像素准星环 +
//      标签 chip 上的色点）。刷在模型上只会得到一堆花花绿绿的塑料块。
//   2. **形状由数据决定**。舰体的长宽比/引擎数/炮塔数/甲板层数不是写死的常量，而是从 config
//      里该类舰的 `hull / armor_mult / speed_mult / accel_mult / attack_mult / slots / cargo`
//      映射过来——所以「护卫舰是细长快船、战列舰是宽厚的炮台」这件事是**算出来的**，
//      以后调 config 的舰级数值，3D 剪影会跟着变。
//
// 性能：几何按 (舰级, 变体) 缓存并打 `userData.shared`，disposeGroup 不会碰它们；同一级
// 的舰共享同一份 geometry，只有 Group 是新的。

import * as THREE from 'three';
import { INC } from './glsl.js';
import { mergeGeometries } from 'three/addons/utils/BufferGeometryUtils.js';
import { rnd, canvasTexture, shared } from './util.js';

// --- 共享材质 ---------------------------------------------------------------
// 高 roughness、低 metalness 的「喷涂航天器」；只有窗/舱灯/引擎自发光。
export const MAT = {
  get hull() {
    return this._h || (this._h = shared(new THREE.MeshStandardMaterial({ color: 0xc9ced8, roughness: 0.55, metalness: 0.22 })));
  },
  get plate() {
    return this._p || (this._p = shared(new THREE.MeshStandardMaterial({ color: 0x9aa2b0, roughness: 0.68, metalness: 0.28 })));
  },
  get dark() {
    return this._d || (this._d = shared(new THREE.MeshStandardMaterial({ color: 0x545b68, roughness: 0.75, metalness: 0.30 })));
  },
  get gold() {
    return this._g || (this._g = shared(new THREE.MeshStandardMaterial({ color: 0xd8b46a, roughness: 0.34, metalness: 0.85 })));
  },
  get glass() {
    return this._gl || (this._gl = shared(new THREE.MeshStandardMaterial({
      color: 0x0a1622, roughness: 0.12, metalness: 0.4,
      emissive: 0x7fd8ff, emissiveIntensity: 1.6,
    })));
  },
  get lamp() {
    return this._l || (this._l = shared(new THREE.MeshStandardMaterial({
      color: 0x2a3242, emissive: 0xffc978, emissiveIntensity: 3.0, roughness: 0.6,
    })));
  },
  get panel() {
    return this._s || (this._s = shared(new THREE.MeshStandardMaterial({ color: 0x22304a, roughness: 0.28, metalness: 0.55 })));
  },
};

// 引擎羽流：additive 锥，亮度随时间抖动 + 沿轴向的噪声。所有舰共享一份材质（同一个 `uTime`）。
const PLUME_VERT = INC('px/misc/plume.vert');
const PLUME_FRAG = INC('px/misc/plume.frag');
let plumeMat = null;
let plumeTime = { value: 0 };
export function plumeMaterial() {
  if (!plumeMat) {
    plumeTime = { value: 0 };
    plumeMat = shared(new THREE.ShaderMaterial({
      uniforms: { uTime: plumeTime, uCore: { value: new THREE.Vector3(0.72, 0.88, 1.0) }, uEdge: { value: new THREE.Vector3(0.20, 0.42, 1.0) }, uIntensity: { value: 1.5 } },
      vertexShader: PLUME_VERT,
      fragmentShader: PLUME_FRAG,
      transparent: true,
      blending: THREE.AdditiveBlending,
      depthWrite: false,
      side: THREE.DoubleSide,
    }));
  }
  return plumeMat;
}
export function tickPlumes(t) { plumeTime.value = t; }

// --- 几何小工具 -------------------------------------------------------------
function box(w, h, d, x = 0, y = 0, z = 0, rx = 0, ry = 0, rz = 0) {
  const g = new THREE.BoxGeometry(w, h, d);
  if (rx) g.rotateX(rx);
  if (ry) g.rotateY(ry);
  if (rz) g.rotateZ(rz);
  g.translate(x, y, z);
  return g;
}
function cyl(rt, rb, h, seg, x = 0, y = 0, z = 0, axis = 'y') {
  const g = new THREE.CylinderGeometry(rt, rb, h, seg, 1, false);
  if (axis === 'z') g.rotateX(Math.PI / 2);
  if (axis === 'x') g.rotateZ(Math.PI / 2);
  g.translate(x, y, z);
  return g;
}
function cone(r, h, seg, x = 0, y = 0, z = 0, axis = 'z') {
  const g = new THREE.ConeGeometry(r, h, seg, 1, false);
  if (axis === 'z') g.rotateX(Math.PI / 2);
  g.translate(x, y, z);
  return g;
}
function merge(list) {
  const clean = list.filter(Boolean);
  if (!clean.length) return null;
  const m = mergeGeometries(clean, false);
  clean.forEach((g) => g.dispose());
  // 合并结果会被缓存/复用，必须标 shared —— 否则 disposeGroup 第一次重建就把它销毁了。
  m.userData.shared = true;
  return m;
}
function meshOf(geo, mat) {
  if (!geo) return null;
  const m = new THREE.Mesh(geo, mat);
  m.userData.shared = true;    // 几何是缓存复用的，别被 disposeGroup 收走
  return m;
}

// 共享的单元锥/圆盘：所有舰的羽流共用一份几何，靠 `mesh.scale` 变形（每艘舰一份几何会泄漏）。
const UNIT_PLUME = geomOnce(() => {
  const g = new THREE.ConeGeometry(1, 1, 8, 1, true);
  g.rotateX(-Math.PI / 2);
  g.translate(0, 0, -0.5);
  g.userData.shared = true;
  return g;
});
const UNIT_DISC = geomOnce(() => {
  const g = new THREE.CircleGeometry(1, 20);
  g.userData.shared = true;
  return g;
});
function geomOnce(f) {
  let g = null;
  return () => (g || (g = f()));
}
const CITY_HALO_MAT = shared(new THREE.MeshBasicMaterial({
  color: 0xffb055, transparent: true, opacity: 0.17,
  blending: THREE.AdditiveBlending, depthWrite: false, side: THREE.DoubleSide,
}));

// --- 舰 ---------------------------------------------------------------------
// 舰级 → 剪影参数。**注意**：这里的每一项都是 config 数值的函数，不是「corvette 就长这样」。
// 之所以还要按舰级索引，是因为 config 里每一级还有一组「招牌修正」（见 game.ron 的注释）：
// 护卫的高加速、驱逐的再生、巡洋的重甲、航母的超远程、战列的一锤定音。这里把那组招牌
// 翻译成几何特征，让剪影本身就能读出舰级。
function shipParams(spec) {
  const hull = spec.hull != null ? spec.hull : 20;
  const len = 0.10 + 0.0030 * Math.min(hull, 90);                 // 0.14 .. 0.37
  const beam = len * (0.18 + 0.16 * Math.min(spec.armor_mult || 1, 2));  // 重甲 → 更宽厚
  const draught = beam * (0.7 + 0.5 * Math.min(spec.cargo || 2, 20) / 20);
  // 引擎数：加速度 + 速度，1..4 台。
  const eng = Math.max(1, Math.min(4, Math.round((spec.accel_mult || 1) * 1.2 + (spec.speed_mult || 1) * 0.8)));
  // 炮塔数：槽位 + 攻击修正。
  const turrets = Math.max(0, Math.min(6, Math.round((spec.slots || 2) * (spec.attack_mult || 1) * 1.6)));
  // 甲板/舱段：舱容越大越像「运输舰」。
  const decks = Math.max(1, Math.min(4, Math.round(1 + (spec.cargo || 2) / 6)));
  // 雷达/天线：攻击距离修正高的（航母）背一个大盘子。
  const dish = (spec.range_mult || 1) >= 1.25;
  // 点防御：舷侧小炮座数量。
  const pd = Math.max(0, Math.min(4, Math.round((spec.pd_mult || 1) * 1.4) - 1));
  return { len, beam, draught, eng, turrets, decks, dish, pd };
}

function buildShipGeo(cls, spec, variant) {
  const p = shipParams(spec);
  const L = p.len, B = p.beam, D = p.draught;
  const hull = [], plate = [], dark = [], glow = [];
  const seed = `${cls}#${variant}`;

  // 主船体：六棱柱（比盒子「有结构」），前段收成锥，后段接引擎舱。
  const mid = new THREE.CylinderGeometry(B, B * 0.92, L * 0.62, 6, 1, false);
  mid.rotateX(Math.PI / 2);
  mid.rotateZ(Math.PI / 6);
  mid.scale(1, D / B, 1);
  hull.push(mid);
  hull.push(cone(B * 0.92, L * 0.30, 6, 0, 0, L * 0.46));
  // 舰桥/上层建筑：靠后一点，有层次。
  hull.push(box(B * 1.15, D * 0.55, L * 0.22, 0, D * 0.62, -L * 0.02));
  plate.push(box(B * 1.45, D * 0.22, L * 0.16, 0, D * 0.90, L * 0.02));

  // 甲板分段：沿 Z 的一串横向加强肋（让侧面不是一片平板）。
  for (let i = 0; i < p.decks + 2; i++) {
    const z = -L * 0.28 + (i / (p.decks + 1)) * L * 0.55;
    plate.push(box(B * 1.06, D * 1.06, L * 0.022, 0, 0, z));
  }

  // 舰首：撞角/传感器。
  dark.push(cyl(B * 0.10, B * 0.16, L * 0.10, 8, 0, 0, L * 0.63, 'z'));
  glow.push(cyl(B * 0.055, B * 0.055, L * 0.012, 8, 0, 0, L * 0.681, 'z'));

  // 引擎：尾部一圈喷口 + 喷管。
  const engR = B * 0.30;
  for (let i = 0; i < p.eng; i++) {
    const a = (i / p.eng) * Math.PI * 2 + Math.PI / 4;
    const ex = Math.cos(a) * B * 0.68;
    const ey = Math.sin(a) * D * 0.68;
    dark.push(cyl(engR, engR * 1.25, L * 0.14, 8, ex, ey, -L * 0.36, 'z'));
    glow.push(cyl(engR * 0.72, engR * 0.72, L * 0.02, 8, ex, ey, -L * 0.428, 'z'));
  }

  // 炮塔：甲板上两列（战列/巡洋多、护卫少）。
  for (let i = 0; i < p.turrets; i++) {
    const side = (i % 2 === 0) ? 1 : -1;
    const row = Math.floor(i / 2);
    const z = L * (0.18 - row * 0.22);
    const x = side * B * 0.72;
    const y = D * 0.95;
    plate.push(cyl(B * 0.20, B * 0.24, D * 0.28, 10, x, y, z));
    dark.push(box(B * 0.05, B * 0.05, B * 0.62, x, y + D * 0.12, z + B * 0.3));
    dark.push(box(B * 0.05, B * 0.05, B * 0.62, x, y + D * 0.12, z - B * 0.3));
  }

  // 点防御：舷侧小炮座。
  for (let i = 0; i < p.pd; i++) {
    const side = (i % 2 === 0) ? 1 : -1;
    const z = -L * (0.10 + Math.floor(i / 2) * 0.20);
    plate.push(cyl(B * 0.09, B * 0.11, D * 0.16, 8, side * B * 0.86, D * 0.10, z));
  }

  // 天线阵 / 大雷达（远洋部署的招牌）。
  if (p.dish) {
    const dg = new THREE.SphereGeometry(B * 0.34, 14, 8, 0, Math.PI * 2, 0, Math.PI * 0.45);
    dg.rotateX(-Math.PI * 0.62);
    dg.translate(0, D * 1.25, -L * 0.06);
    plate.push(dg);
    dark.push(cyl(B * 0.05, B * 0.05, D * 0.24, 6, 0, D * 1.02, -L * 0.06));
  }

  // Greeble：沿舰体表面确定性撒一圈小方块，打破平板感（数量跟着舰体大小走）。
  const gn = Math.round(14 + 60 * (L / 0.37));
  for (let i = 0; i < gn; i++) {
    const u = rnd(seed, 'g', i);
    const v = rnd(seed, 'g2', i);
    const w = rnd(seed, 'g3', i);
    const z = (u - 0.5) * L * 0.72;
    const ang = v * Math.PI * 2;
    const rr = 0.92 + w * 0.25;
    const x = Math.cos(ang) * B * rr;
    const y = Math.sin(ang) * D * rr;
    const s = B * (0.10 + 0.26 * w);
    (w > 0.6 ? plate : dark).push(box(s, s * 0.6, s * (1.0 + 2 * w), x, y, z));
  }

  // 舷灯：几点暖色（不是阵营色）。
  for (let i = 0; i < 3; i++) {
    const t = (i / 2 - 0.5) * L * 0.5;
    glow.push(cyl(B * 0.035, B * 0.035, B * 0.02, 6, 0, D * 1.08, t));
  }

  return {
    hull: merge(hull), plate: merge(plate), dark: merge(dark), glow: merge(glow), params: p,
  };
}

const shipGeoCache = new Map();
function shipGeo(cls, spec, variant) {
  const key = `${cls}#${variant}`;
  let g = shipGeoCache.get(key);
  if (!g) { g = buildShipGeo(cls, spec, variant); shipGeoCache.set(key, g); }
  return g;
}

// 返回一个 Group：舰体（中性灰）+ 引擎羽流。`spec` 是 config 里该类舰的记录。
export function buildShip(name, cls, spec) {
  const variant = Math.floor(rnd(name, 'variant') * 3);
  const g = shipGeo(cls, spec || {}, variant);
  const grp = new THREE.Group();
  grp.add(meshOf(g.hull, MAT.hull));
  grp.add(meshOf(g.plate, MAT.plate));
  grp.add(meshOf(g.dark, MAT.dark));
  const gl = meshOf(g.glow, MAT.lamp);
  if (gl) grp.add(gl);

  // 引擎羽流：每个喷口一小段锥（共享单元锥，靠 scale 变形）。
  const p = g.params;
  const pg = new THREE.Group();
  const unit = UNIT_PLUME();
  for (let i = 0; i < p.eng; i++) {
    const a = (i / p.eng) * Math.PI * 2 + Math.PI / 4;
    const m = new THREE.Mesh(unit, plumeMaterial());
    m.scale.set(p.beam * 0.26, p.beam * 0.26, p.len * 0.85);
    m.position.set(Math.cos(a) * p.beam * 0.68, Math.sin(a) * p.draught * 0.68, -p.len * 0.42);
    pg.add(m);
  }
  grp.add(pg);
  grp.userData.plume = pg;
  return grp;
}

// --- 地面城市 ---------------------------------------------------------------
// 建模依据 = 该城**真实的建筑列表**（kind/area）。住宅区给塔楼、开采区给井架、建造区给厂房。
// 所以城市的天际线是游戏状态的一个读数，不是随机装饰。
function buildingKind(b) {
  return String((b && (b.kind || b.建筑类型 || b.类型)) || 'residential');
}
function buildingArea(b) {
  const a = b && (b.area != null ? b.area : (b.面积 != null ? b.面积 : 0));
  return Number(a) || 0;
}

export function buildGroundCity(city, opts) {
  const s = opts.size;
  const blds = Array.isArray(city.建筑) ? city.建筑 : (Array.isArray(city.buildings) ? city.buildings : []);
  const grp = new THREE.Group();
  const plate = [], hull = [], dark = [], glow = [];
  const seed = city.城名 || city.name || 'city';

  // 基座：一个浅浅的六边形平台 + 一圈护墙。
  const base = new THREE.CylinderGeometry(s * 0.95, s * 1.05, s * 0.16, 6);
  base.translate(0, s * 0.08, 0);
  dark.push(base);

  const n = Math.max(1, Math.min(blds.length || 3, 9));
  for (let i = 0; i < n; i++) {
    const b = blds[i] || {};
    const kind = buildingKind(b);
    const area = buildingArea(b);
    const u = rnd(seed, 'b', i);
    const ang = (i / n) * Math.PI * 2 + u * 0.7;
    const rad = (0.18 + 0.62 * rnd(seed, 'r', i)) * s;
    const x = Math.cos(ang) * rad;
    const z = Math.sin(ang) * rad;
    // 面积 → 体量；住宅高瘦、开采矮胖、建造中长条（厂房）。
    const scale = 0.55 + Math.min(area, 80) / 80;
    if (kind === 'mining' || kind === '开采区') {
      const h = s * (0.30 + 0.35 * scale);
      hull.push(box(s * 0.55 * scale, h, s * 0.55 * scale, x, h / 2 + s * 0.12, z));
      dark.push(cyl(s * 0.05, s * 0.05, h * 1.7, 6, x, h * 0.85, z));   // 井架
      glow.push(cyl(s * 0.07, s * 0.07, s * 0.04, 6, x, h * 1.7 + s * 0.12, z));
    } else if (kind === 'construction' || kind === '建造区') {
      const h = s * (0.20 + 0.22 * scale);
      hull.push(box(s * 1.05 * scale, h, s * 0.42 * scale, x, h / 2 + s * 0.12, z, 0, ang, 0));
      // 未完工的桁架。
      dark.push(box(s * 0.08, h * 1.6, s * 0.08, x, h * 0.9 + s * 0.12, z));
      glow.push(box(s * 0.10, s * 0.03, s * 0.10, x, h * 1.75 + s * 0.12, z));
    } else {
      const h = s * (0.55 + 1.25 * scale);
      const w = s * (0.28 + 0.16 * scale);
      hull.push(box(w, h, w, x, h / 2 + s * 0.12, z, 0, u * 1.2, 0));
      // 楼顶设备。
      plate.push(box(w * 0.5, s * 0.08, w * 0.5, x, h + s * 0.16, z));
      dark.push(cyl(w * 0.12, w * 0.12, s * 0.10, 6, x + w * 0.3, h + s * 0.20, z - w * 0.3));
      // 航空警示灯。
      glow.push(cyl(s * 0.035, s * 0.035, s * 0.04, 6, x, h + s * 0.21, z));
    }
  }

  const hullM = meshOf(merge(hull), MAT.hull);
  const plateM = meshOf(merge(plate), MAT.plate);
  const darkM = meshOf(merge(dark), MAT.dark);
  const glowM = meshOf(merge(glow), MAT.lamp);
  [hullM, plateM, darkM, glowM].forEach((m) => { if (m) grp.add(m); });

  // 城区辉光：贴着地面的一小片 additive，让城市在夜面/远景也是「一摊光」而不是几个灰点。
  const halo = new THREE.Mesh(UNIT_DISC(), CITY_HALO_MAT);
  halo.rotation.x = -Math.PI / 2;
  halo.position.y = s * 0.02;
  halo.scale.setScalar(s * 0.95);
  grp.add(halo);
  return grp;
}

// --- 轨道空间站 -------------------------------------------------------------
export function buildStation(city, opts) {
  const s = opts.size;
  const seed = city.城名 || city.name || 'station';
  const grp = new THREE.Group();
  const hull = [], plate = [], dark = [], glow = [];
  const rings = [];

  // 居住环：主环 + 两片辐条（会自转 —— 真实空间站的离心重力）。
  const R = s * 1.0;
  const ringGeo = new THREE.TorusGeometry(R, s * 0.10, 10, 44);
  ringGeo.rotateX(Math.PI / 2);
  hull.push(ringGeo);
  for (let i = 0; i < 3; i++) {
    const a = (i / 3) * Math.PI * 2 + rnd(seed, 'spoke') * 0.6;
    const sp = new THREE.BoxGeometry(R * 1.95, s * 0.055, s * 0.075);
    sp.rotateY(a);
    hull.push(sp);
  }
  // 轮毂 + 对接锥。
  hull.push(cyl(s * 0.22, s * 0.30, s * 0.42, 12));
  dark.push(cone(s * 0.16, s * 0.34, 8, 0, s * 0.34, 0, 'y'));

  // 桁架 + 太阳能板 + 散热片（真实站最抢眼的两组面）。
  for (const sign of [-1, 1]) {
    const tz = sign * s * 0.9;
    dark.push(box(s * 0.9, s * 0.05, s * 0.05, 0, 0, tz));
    const panel = box(s * 1.5, s * 0.02, s * 0.62, sign * s * 0.30, 0, tz + sign * s * 0.55, 0, sign * 0.22, 0);
    plate.push(panel);
    const rad = box(s * 1.0, s * 0.015, s * 0.42, sign * s * 0.95, 0, tz, 0, -sign * 0.15, 0);
    plate.push(rad);
    glow.push(cyl(s * 0.05, s * 0.05, s * 0.03, 6, sign * s * 0.95, s * 0.06, tz));
  }

  const hullM = meshOf(merge(hull), MAT.hull);
  const plateM = meshOf(merge(plate), MAT.plate);
  const darkM = meshOf(merge(dark), MAT.dark);
  const glowM = meshOf(merge(glow), MAT.lamp);
  [hullM, plateM, darkM, glowM].forEach((m) => { if (m) grp.add(m); });

  // 把「会自转的部分」单独提出来（主环 + 辐条）——转的是它，不是整个站。
  const spinner = new THREE.Group();
  if (hullM) { grp.remove(hullM); spinner.add(hullM); }
  grp.add(spinner);
  grp.userData.spinner = spinner;
  void rings;
  return grp;
}

// --- 小行星带 / 柯伊伯带 -----------------------------------------------------
// 实例化碎岩：一颗低多边形不规则石块，乘上几百个实例（每个实例有自己的尺度/朝向/自转轴）。
// 位置在环带里按确定性伪随机分布，**不是**从任何资源里读的。
export function buildBelt(opts) {
  const {
    count, inner, outer, thickness, yScale = 1.0, seed = 'belt',
    color = 0x8d8579, size = 1.0, sizeSpread = 2.4,
  } = opts;
  if (count <= 0) return null;

  // 碎岩原型：把二十面体的顶点按噪声推一推，得到一块不规则石头（比球体可信得多）。
  const proto = new THREE.IcosahedronGeometry(1, 1);
  const pos = proto.attributes.position;
  const v = new THREE.Vector3();
  for (let i = 0; i < pos.count; i++) {
    v.fromBufferAttribute(pos, i);
    const k = 0.62 + 0.55 * rnd(seed, 'rock', i) + 0.22 * Math.sin(v.x * 5.1) * Math.sin(v.y * 4.3) * Math.sin(v.z * 3.7);
    v.multiplyScalar(k);
    pos.setXYZ(i, v.x, v.y, v.z);
  }
  proto.computeVertexNormals();
  proto.userData.shared = true;

  const mat = shared(new THREE.MeshStandardMaterial({ color, roughness: 0.94, metalness: 0.06, flatShading: true }));
  const inst = new THREE.InstancedMesh(proto, mat, count);
  inst.userData.shared = true;
  inst.frustumCulled = false;

  const m = new THREE.Matrix4();
  const q = new THREE.Quaternion();
  const e = new THREE.Euler();
  const sc = new THREE.Vector3();
  const tr = new THREE.Vector3();
  for (let i = 0; i < count; i++) {
    const a = rnd(seed, 'a', i) * Math.PI * 2;
    // 半径用「面积均匀」的分布（sqrt），否则会在内缘堆一堆。
    const rr = inner + (outer - inner) * Math.sqrt(rnd(seed, 'r', i));
    const yy = (rnd(seed, 'y', i) - 0.5) * thickness * (0.4 + 0.6 * rnd(seed, 'y2', i));
    tr.set(Math.cos(a) * rr, yy * yScale, Math.sin(a) * rr);
    e.set(rnd(seed, 'ex', i) * Math.PI * 2, rnd(seed, 'ey', i) * Math.PI * 2, rnd(seed, 'ez', i) * Math.PI * 2);
    q.setFromEuler(e);
    const sz = size * (0.35 + sizeSpread * Math.pow(rnd(seed, 's', i), 3.0));
    sc.set(sz, sz * (0.7 + 0.5 * rnd(seed, 's2', i)), sz);
    m.compose(tr, q, sc);
    inst.setMatrixAt(i, m);
  }
  inst.instanceMatrix.needsUpdate = true;
  return inst;
}
