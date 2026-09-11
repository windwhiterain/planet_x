// 色调映射的**正/反算**：把「参考图上那一块颜色」翻译成「着色器该输出多少 HDR」。
//
// 为什么要它：日面配色不是在 HDR 里调的，是在**屏幕像素**上验的。中间隔着
// `renderer.toneMappingExposure` + ACES（三通道互相混） + sRGB 编码三跳 ——
// 「看着太亮就减一点」这种试法每一轮都要跑一次浏览器。这里把这条链**闭式反解**：
// 给目标 sRGB，直接算出需要的线性 HDR 值，palette 一次就位，剩下的只有审美。
//
import { TUNING } from '../../web/static/map3d/tuning.js';
// 镜像的是 three r169 的 `ACESFilmicToneMapping`（含 `* exposure / 0.6` 与两组矩阵）。
//
// ⚠ **矩阵存法**：GLSL 的 `mat3(a,b,c)` 把三个 vec3 当**列**填进去，而这里按行存。
// 第一版直接照抄源码的顺序当行用 ⇒ 整体转置了。转置的矩阵**正反算仍然自洽**
// （`--solve` 与 `--hdr` 互相回代精确复原），所以这个错**不会**在自检里露头 ——
// 只有在把预测色和实机像素并排比的时候才会显形：预测 (186,73,15)、实机 (197,62,47)。
// 教训：标定工具必须**对着实机验一次**，不能只看它自洽。
const IN = [
  0.59719, 0.35458, 0.04823,
  0.07600, 0.90834, 0.01566,
  0.02840, 0.13383, 0.83777,
];
const OUT = [
  1.60475, -0.53108, -0.07367,
  -0.10208, 1.10813, -0.00605,
  -0.00327, -0.07276, 1.07602,
];
// 按行存的 3×3 直接用（列向量的左乘）。
const mul = (m, v) => [
  m[0] * v[0] + m[1] * v[1] + m[2] * v[2],
  m[3] * v[0] + m[4] * v[1] + m[5] * v[2],
  m[6] * v[0] + m[7] * v[1] + m[8] * v[2],
];
const inv3 = (m) => {
  const [a, b, c, d, e, f, g, h, i] = m;
  const A = e * i - f * h, B = -(d * i - f * g), C = d * h - e * g;
  const det = a * A + b * B + c * C;
  return [
    A / det, (c * h - b * i) / det, (b * f - c * e) / det,
    B / det, (a * i - c * g) / det, (c * d - a * f) / det,
    C / det, (b * g - a * h) / det, (a * e - b * d) / det,
  ];
};
const IN_INV = inv3(IN), OUT_INV = inv3(OUT);
const rrt = (v) => (v * (v + 0.0245786) - 0.000090537) / (v * (0.983729 * v + 0.4329510) + 0.238081);
const clamp01 = (v) => Math.min(1, Math.max(0, v));

const srgbToLin = (c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
const linToSrgb = (c) => (c <= 0.0031308 ? c * 12.92 : 1.055 * c ** (1 / 2.4) - 0.055);

// 线性 HDR → 屏幕 0..255
export function hdrToScreen(rgb, exposure = TUNING.exposure) {
  const scaled = rgb.map((c) => (c * exposure) / 0.6);
  const out = mul(OUT, mul(IN, scaled).map(rrt));
  return out.map((c) => Math.round(linToSrgb(clamp01(c)) * 255));
}

// 屏幕 0..255 → 线性 HDR（ACES 的逆）。逐通道解 RRTAndODTFit 的二次方程，取单调那支。
export function screenToHdr(rgb255, exposure = TUNING.exposure) {
  const lin = rgb255.map((c) => srgbToLin(c / 255));
  const y = mul(OUT_INV, lin);
  const a = y.map((Y) => {
    const A = 0.983729 * Y - 1;
    const B = 0.432951 * Y - 0.0245786;
    const C = 0.238081 * Y + 0.000090537;
    if (Math.abs(A) < 1e-9) return -C / B;
    const disc = B * B - 4 * A * C;
    if (disc < 0) return (B > 0 ? -B : B) / (2 * A);   // 落在色域外：给个最近的
    const s = Math.sqrt(disc);
    const r1 = (-B + s) / (2 * A), r2 = (-B - s) / (2 * A);
    // RRTAndODTFit 在 v ≥ 0 上单调：取非负的那个根
    return r1 >= 0 && (r2 < 0 || r1 < r2) ? r1 : r2;
  });
  const x = mul(IN_INV, a);
  return x.map((c) => (c * 0.6) / exposure);
}

export const _internal = { rrt, IN, OUT, IN_INV, OUT_INV };
