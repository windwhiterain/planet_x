// 量日珥流场**像不像"流动"**（不是量画面）。
//
// 为什么需要它：用户裁决 *「现在没有场的感觉啊……那种光滑的曲率，漩涡，
// 现在的场显然太坑坑洼洼了」* —— 而这件事**在画面上很难判断"差多少"**，
// 在数值上一眼就能看出来。真正的判据是下面这个：
//
//   场的**特征转角尺度** = |F| / |∇×F|（世界单位）
//   —— 小于这个距离，场的方向就已经转过一大截了。它**必须显著大于带子的宽度**
//      （0.03~0.4 R），否则"一条带子内部"的场就已经乱转 ⇒ 位移出来是坑洼，不是流动。
//
// 实测演进（都是这个脚本量出来的）：
//   原来（3 个八度、gain 0.5）：特征尺度 **0.225**（0.035R）—— 比带子还窄 ⇒ 当然坑洼
//   现在（2 个八度、gain 0.18、lac 1.6、K 0.20）：**~1.0**（0.16R），方向在 1.2 单位内只转 ~36°
//
// ⚠ 反直觉的坑，改之前先读这段：**微分会放大高频**。第 i 个八度对旋度 `∇×Ψ` 的贡献是
// `幅度ᵢ × 频率ᵢ`，所以 (0.5, 0.25, 0.125) × (1, 2.02, 4.08) 在旋度里是
// **0.50 / 0.50 / 0.51 —— 三等分**：细碎那两层和大漩涡那层一样强。
// 要"大漩涡"就必须让 gain < 1/lac（本仓取 0.18 < 0.625），而不是照抄值噪声的 0.5。
//
// 用法：node scripts/shots/field.mjs
import { PromField } from '../../web/static/map3d/sunfield.js';
const R = 6.4;
const rnd = (s => () => ((s = (s * 1664525 + 1013904223) >>> 0) / 4294967296))(2024);
const norm = (v) => { const l = Math.hypot(...v) || 1e-9; return v.map(x => x / l); };
const f = new PromField();
const F = [0, 0, 0];
// 只在日面附近（band 真的长在那里的地方）采样
const surf = () => { const z = rnd() * 2 - 1, a = rnd() * Math.PI * 2, r = Math.sqrt(Math.max(0, 1 - z * z));
  const k = R * (0.98 + 0.25 * rnd()); return [r * Math.cos(a) * k, z * k, r * Math.sin(a) * k]; };
const dirAt = (p) => { f.curl(p[0], p[1], p[2], F); return norm([F[0], F[1], F[2]]); };
const ang = (a, b) => Math.acos(Math.max(-1, Math.min(1, a[0] * b[0] + a[1] * b[1] + a[2] * b[2]))) * 180 / Math.PI;
console.log('方向随距离的变化（度）—— 越小越"大漩涡"：');
for (const d of [0.15, 0.3, 0.6, 1.2]) {
  let s = 0; const N = 3000;
  for (let i = 0; i < N; i++) {
    const p = surf(); const q = surf();
    const a = dirAt(p), b = dirAt(q);
    // 让 q 落在 p 的 d 距离上：沿随机切向走
    const t = norm([q[1] * p[2] - q[2] * p[1], q[2] * p[0] - q[0] * p[2], q[0] * p[1] - q[1] * p[0]]);
    s += ang(a, dirAt([p[0] + t[0] * d, p[1] + t[1] * d, p[2] + t[2] * d]));
  }
  console.log(`  d=${d.toFixed(2)} 世界单位（${(d / R).toFixed(3)}R）: 平均转 ${(s / N).toFixed(1)}°`);
}
// |F| / |ω| 的比值 = 场"打结"的程度
const mags = [], ws = [];
const W = [0, 0, 0];
for (let i = 0; i < 4000; i++) {
  const p = surf();
  f.curl(p[0], p[1], p[2], F); f.curlOfCurl(p[0], p[1], p[2], W);
  mags.push(Math.hypot(...F)); ws.push(Math.hypot(...W));
}
const q = (a, p) => { const s = a.slice().sort((x, y) => x - y); return s[Math.floor(p * s.length)]; };
console.log(`|F| p50=${q(mags, .5).toFixed(2)}   |ω| p50=${q(ws, .5).toFixed(2)}`);
console.log(`  ⇒ 场的"特征转角尺度" ≈ |F|/|ω| = ${(q(mags, .5) / q(ws, .5)).toFixed(3)} 世界单位（小于它方向就乱了）`);
