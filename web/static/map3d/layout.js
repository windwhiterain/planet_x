// 行星X WebUI — 轨道数学 + **显示布局**（纯渲染用，不动 state）。
//
// 三层坐标，从上到下：
//   1. **AU 平面**（state 里给的）：天体 `位置` [x, y] 是黄道平面上的二维坐标。
//   2. **压缩平面**：真实 AU 跨度 ~0.3..97，直接画会挤成一团。用 r' = r^orbitExp 做径向
//      压缩（保方向、只改半径）。
//   3. **世界坐标**：压缩平面 × `scale`，再按天体的**轨道小倾角**抬出平面（见 liftVec）。
//
// 倾角的意义：第 3 层以前根本不存在（全系统 y≡0），地图是严格平铺的沙盘。给每个天体一个
// ≤`TUNING.orbitIncMax` 的确定性倾角，整个系统立刻读得出是三维的；因为倾角小、又不改任何
// 二维投影量（压缩、让位、间距全在压缩平面里算完），**俯视时几乎不影响读图**。
//
// 关键不变量：`liftVec` 是一个**刚体旋转**（保长度）。所以
//   * 母星→卫星的三维间距 == 压缩平面里算出来的间距（`moonGap` 的判据在三维里仍然成立）；
//   * 轨道线上每个采样点与天体位置用的是同一个变换 ⇒ 卫星永远精确落在自己画出的轨道上。

import * as THREE from 'three';
import { TUNING } from './tuning.js';
import { rnd } from './util.js';
import { specFor, bodyRadius } from './kinds.js';

export { specFor, bodyRadius };

// --- 径向压缩 ---------------------------------------------------------------
export function compressRadius(r) {
  return Math.pow(r, TUNING.orbitExp);
}
export function compress(pos) {
  const x = pos[0], y = pos[1];
  const r = Math.hypot(x, y);
  if (r < 1e-9) return [0, 0];
  const rr = compressRadius(r);
  const k = rr / r;
  return [x * k, y * k];
}

// 稳定的世界缩放：用最远天体的「远日点」径向压缩值定标，而非会随回合移动的当前位置，
// 避免 advance 时整幅地图缩放/漂移。
export function systemScale(world, target) {
  let maxAphe = 1e-6;
  (world.bodies || []).forEach((b) => {
    const o = b && b.轨道;
    if (o && o.远日点距离) maxAphe = Math.max(maxAphe, o.远日点距离);
  });
  return (target || 110) / Math.max(compressRadius(maxAphe), 1e-3);
}

// --- 倾角平面 ---------------------------------------------------------------
// 每个天体一个 `{inc, node}`：绕「交点轴」把压缩平面上的点抬起来。
// node 是交点方向在压缩平面里的方位角；inc 是倾角（弧度）。
export function planeFor(name, isMoon) {
  const maxDeg = isMoon ? TUNING.orbitIncMoon : TUNING.orbitIncMax;
  if (!(maxDeg > 0)) return { inc: 0, node: 0, cosI: 1, sinI: 0 };
  // 平方根分布：多数天体落在中等倾角，少数接近 0 或 max——看起来更自然（不是一圈等距的环）。
  const u = Math.sqrt(rnd(name, 'inc'));
  const inc = (maxDeg * u) * Math.PI / 180;
  const node = rnd(name, 'node') * Math.PI * 2;
  return { inc, node, cosI: Math.cos(inc), sinI: Math.sin(inc) };
}

// 把压缩平面里的**向量** (dx, dz) 按 plane 抬到三维（刚体旋转，保长度）。
// 返回值复用同一个 Vector3？不——调用方常常要保留，这里返回新对象（调用频率是 O(天体数)，不值优化）。
export function liftVec(dx, dz, plane) {
  const c = Math.cos(plane.node), s = Math.sin(plane.node);
  const a = dx * c + dz * s;          // 沿交点轴的分量
  const b = -dx * s + dz * c;         // 垂直交点轴（在平面内）的分量
  const bi = b * plane.cosI;
  return new THREE.Vector3(
    a * c - bi * s,
    b * plane.sinI,
    a * s + bi * c,
  );
}

// 世界坐标 = 压缩平面 × scale，再按 plane 抬起。`scale` 由调用方给（index.js 持有）。
export function toWorld(p2, plane, scale) {
  const v = liftVec(p2[0], p2[1], plane);
  return v.multiplyScalar(scale);
}

// --- Kepler -----------------------------------------------------------------
// 由 Orbit 参数在给定 months 计算天体位置（与 Rust `Orbit::position` 同一套 Kepler 求解，
// 二维、以焦点为原点）。
export function orbitPositionAt(orbit, months) {
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
export function normalize2(v) {
  const len = Math.hypot(v[0], v[1]);
  if (!len || len < 1e-12) return [1, 0];
  return [v[0] / len, v[1] / len];
}

// --- 显示布局 ---------------------------------------------------------------
// 真实比例下卫星轨道半径远小于被夸张过的母星显示半径，直接画会重叠。做法：
//   1) 卫星离母星不足「母星半径(带环则按环外缘) + 自身半径 + moonGap」时，沿母星→卫星的
//      方位角把它推到刚好够远，并记下缩放系数 k；
//   2) 画轨道线时对**整条轨道**用同一个 k（相对母星缩放偏移量）。
// 全在**压缩平面**里算——倾角是之后才加的刚体旋转，不会破坏这里的间距判据。
//
// ⚠ 单位：`scale` **必须**在这里就乘进去。`bodyRadius` 给的是**世界单位**的显示半径，
// 而 `compress()` 给的是无量纲的压缩平面坐标（0.98…5.5），两者不乘 `scale` 就直接比较，
// moonGap 判据会永远不触发（几个世界单位的净空 vs 零点几个单位的间距）。
// 曾经漏掉这一步：轨道的**线**画在正确的尺度上（orbitPoints 自己有 scale），而天体本体
// 却小了 16 倍——现象是「环在、球不见」。
export function computeLayout(world, visuals, scale) {
  // 漏传 scale 会让所有坐标变成 NaN（`x * undefined`），而 NaN 位置在渲染里表现为
  // 「什么都不见」——离现场很远。所以这里直接把缺参数变成一句看得懂的异常。
  if (!Number.isFinite(scale) || scale <= 0) {
    throw new Error('computeLayout: scale 必须是正有限数（收到 ' + scale + '）——调用方要先算 systemScale()');
  }
  const bodies = world.bodies || [];
  const byName = new Map();
  bodies.forEach((b) => byName.set(b.天体名, b));

  const nodes = new Map();       // name -> { pos:Vector3, radius, plane, ... }
  const planes = new Map();
  const comp2 = new Map();       // name -> [x, z] 压缩平面坐标 × scale（含让位）
  const orbitScale = new Map();  // 卫星 name -> k

  // 第一遍：压缩平面坐标（已乘 scale）+ 显示半径 + 平面。
  bodies.forEach((b) => {
    const isMoon = !!(b.轨道 && b.轨道.母天体);
    const plane = planeFor(b.天体名, isMoon);
    planes.set(b.天体名, plane);
    const c = compress(b.位置 || [0, 0]);
    comp2.set(b.天体名, [c[0] * scale, c[1] * scale]);
    nodes.set(b.天体名, { radius: bodyRadius(b, specFor(visuals, b)), plane, isMoon });
  });

  // 第二遍：卫星让位（压缩平面内）。
  bodies.forEach((b) => {
    if (!b.轨道 || !b.轨道.母天体) return;
    const me = nodes.get(b.天体名);
    const p2 = comp2.get(b.天体名);
    const pp2 = comp2.get(b.轨道.母天体);
    const par = nodes.get(b.轨道.母天体);
    if (!me || !p2 || !pp2 || !par) return;
    const parBody = byName.get(b.轨道.母天体);
    const parReach = par.radius * (parBody && parBody.星环 ? TUNING.ringOuter : 1.0);
    const minSep = parReach + me.radius + TUNING.moonGap;
    const dx = p2[0] - pp2[0], dz = p2[1] - pp2[1];
    const d = Math.hypot(dx, dz);
    if (d < 1e-9 || d >= minSep) return;
    const k = minSep / d;
    orbitScale.set(b.天体名, k);
    p2[0] = pp2[0] + dx * k;
    p2[1] = pp2[1] + dz * k;
  });

  // 第三遍：抬到三维（母星链式累加）。母星先算——按 bodies 顺序可能子在前，所以递归。
  const done = new Set();
  const resolve = (name, depth) => {
    const n = nodes.get(name);
    if (!n) return null;
    if (n.pos) return n.pos;
    if (depth > 8) return new THREE.Vector3();
    const p2 = comp2.get(name);
    const b = byName.get(name);
    const parentName = b && b.轨道 && b.轨道.母天体;
    const parent = parentName ? nodes.get(parentName) : null;
    let v;
    if (parent && comp2.has(parentName)) {
      const pp2 = comp2.get(parentName);
      const off = liftVec(p2[0] - pp2[0], p2[1] - pp2[1], n.plane);
      v = resolve(parentName, depth + 1).clone().add(off);
    } else {
      v = liftVec(p2[0], p2[1], n.plane);
    }
    n.pos = v;
    done.add(name);
    return v;
  };
  bodies.forEach((b) => resolve(b.天体名, 0));

  return { nodes, comp2, orbitScale, planes };
}

// --- 轨道线 ----------------------------------------------------------------
// `orbit` 的局部（焦点中心）轨道，在 AU 里叠加 `anchor`（母天体的 AU 坐标，日心行星传 [0,0]）
// 后做径向压缩，得到该天体在压缩平面上的真实路径——卫星的椭圆包在母天体周围，而不是绕太阳。
// `k`：相对母星的偏移缩放系数（computeLayout 算出的「卫星让位」系数）。
// 返回**世界坐标**数组（已含倾角与 scale）。
export function orbitPoints(orbit, anchor, k, plane, parentPos, scale, n = 192) {
  const pts = [];
  const period = Math.max(orbit.公转周期, 1e-6);
  const ax = anchor ? (anchor[0] || 0) : 0;
  const ay = anchor ? (anchor[1] || 0) : 0;
  const [acx, acz] = compress([ax, ay]);
  const kk = (k || 1) * scale;
  for (let i = 0; i <= n; i++) {
    const t = (i / n) * period;
    const local = orbitPositionAt(orbit, t);
    const [cx, cz] = compress([ax + local[0], ay + local[1]]);
    const dx = (cx - acx) * kk;
    const dz = (cz - acz) * kk;
    if (parentPos) {
      // 卫星：偏移量按自己的平面抬起，再挂到母星的三维位置上。
      pts.push(liftVec(dx, dz, plane).add(parentPos));
    } else {
      pts.push(liftVec(cx, cz, plane).multiplyScalar(scale));
    }
  }
  return pts;
}

// 天体的确定性方位（城市贴地表用）：`elev` 为距 +Y 极轴的极角（0=顶，π/2=赤道）。
export function cityDir(idx, count, elev) {
  const az = (idx / Math.max(count, 1)) * Math.PI * 2 + 0.6;
  const horiz = Math.sin(elev);
  return [Math.cos(az) * horiz, Math.cos(elev), Math.sin(az) * horiz];
}
