// 像素判据：把「这画面看起来对不对」尽量压成**几个可断言的数**。
//
// 这一层的存在理由见 `.agents/notes/glsl-files.md`：本仓为「太阳透明」宣布过一次「无回归」，
// 依据是 bodies/drawCalls/triangles/programs **四项计数逐项相同** —— 而着色器根本没编译过、
// 什么都没画。计数只能当辅助，**验收必须落在像素上**。
//
// 亮度用 Rec.709 加权、在 sRGB 的 0..255 上直接算（判据只做相对比较，不用转线性）。

export const lum = (r, g, b) => 0.2126 * r + 0.7152 * g + 0.0722 * b;
export const pxLum = (img, x, y) => {
  const d = (y * img.width + x) * 4;
  return lum(img.data[d], img.data[d + 1], img.data[d + 2]);
};

const pct = (sorted, p) => sorted.length ? sorted[Math.min(sorted.length - 1, Math.floor(p * sorted.length))] : 0;

function reduce(acc) {
  const n = acc.n || 1;
  const out = {
    n: acc.n,
    meanRGB: [acc.r / n, acc.g / n, acc.b / n].map((v) => +v.toFixed(2)),
    meanLum: +(acc.l / n).toFixed(2),
  };
  const sorted = acc.lums.sort((a, b) => a - b);
  out.p10 = +pct(sorted, 0.10).toFixed(2);
  out.p50 = +pct(sorted, 0.50).toFixed(2);
  out.p90 = +pct(sorted, 0.90).toFixed(2);
  out.p99 = +pct(sorted, 0.99).toFixed(2);
  out.max = +pct(sorted, 0.9999).toFixed(2);
  out.whiteFrac = +(acc.white / n).toFixed(4);      // 三通道都 ≥235：ACES 压成白饼的比例
  out.warmth = +((acc.r - acc.b) / n).toFixed(2);   // R−B 均值：>0 偏暖，越负越蓝
  return out;
}

// 矩形区域统计（矩形可越界，自动裁剪）。
export function stats(img, rect) {
  const x0 = Math.max(0, Math.round(rect.x)), y0 = Math.max(0, Math.round(rect.y));
  const x1 = Math.min(img.width, Math.round(rect.x + rect.w)), y1 = Math.min(img.height, Math.round(rect.y + rect.h));
  const acc = { n: 0, r: 0, g: 0, b: 0, l: 0, white: 0, lums: [] };
  for (let y = y0; y < y1; y++) {
    for (let x = x0; x < x1; x++) {
      const d = (y * img.width + x) * 4;
      const r = img.data[d], g = img.data[d + 1], b = img.data[d + 2];
      const l = lum(r, g, b);
      acc.n++; acc.r += r; acc.g += g; acc.b += b; acc.l += l; acc.lums.push(l);
      if (r >= 235 && g >= 235 && b >= 235) acc.white++;
    }
  }
  return reduce(acc);
}

// 环带统计（以屏幕像素为单位的圆心/半径）—— 日冕/日珥就看这一串环。
export function annulus(img, cx, cy, r0, r1) {
  const acc = { n: 0, r: 0, g: 0, b: 0, l: 0, white: 0, lums: [] };
  const x0 = Math.max(0, Math.floor(cx - r1)), x1 = Math.min(img.width - 1, Math.ceil(cx + r1));
  const y0 = Math.max(0, Math.floor(cy - r1)), y1 = Math.min(img.height - 1, Math.ceil(cy + r1));
  for (let y = y0; y <= y1; y++) {
    for (let x = x0; x <= x1; x++) {
      const d = Math.hypot(x - cx, y - cy);
      if (d < r0 || d > r1) continue;
      const i = (y * img.width + x) * 4;
      const r = img.data[i], g = img.data[i + 1], b = img.data[i + 2];
      const l = lum(r, g, b);
      acc.n++; acc.r += r; acc.g += g; acc.b += b; acc.l += l; acc.lums.push(l);
      if (r >= 235 && g >= 235 && b >= 235) acc.white++;
    }
  }
  return reduce(acc);
}

// 径向剖面：把 annulus 按半径切片，用来画「日面 → 日缘 → 日冕 → 虚空」的衰减曲线。
export function radialProfile(img, cx, cy, r0, r1, bins = 12) {
  const out = [];
  const step = (r1 - r0) / bins;
  for (let i = 0; i < bins; i++) {
    out.push({ r: +(r0 + step * (i + 0.5)).toFixed(1), ...annulus(img, cx, cy, r0 + step * i, r0 + step * (i + 1)) });
  }
  return out;
}

// **日缘参差度**：日珥的核心判据。以日面圆心为中心、按角度扫射线（从外向内找到第一根不是
// 黑的像素），得到「轮廓半径 vs 角度」的分布。分布越宽 ⇒ 边缘越参差；一圈等半径硬环的
// `std ≈ 0`、`spread ≈ 0`。
// `thr` 用低阈值抓针状体的暗梢、高阈值抓亮核 —— 两个都给，避免「阈值一改结论就翻」。
export function silhouette(img, { cx, cy, maxR, thr = 6, a0 = -Math.PI, a1 = Math.PI, n = 180 }) {
  const radii = [];
  for (let i = 0; i < n; i++) {
    const a = a0 + ((a1 - a0) * i) / n;
    const dx = Math.cos(a), dy = Math.sin(a);
    let found = null;
    for (let r = maxR; r > maxR * 0.25; r -= 1) {
      const x = Math.round(cx + dx * r), y = Math.round(cy + dy * r);
      if (x < 0 || y < 0 || x >= img.width || y >= img.height) continue;
      if (pxLum(img, x, y) >= thr) { found = r; break; }
    }
    if (found !== null) radii.push(found);
  }
  if (!radii.length) return { n: 0, mean: 0, std: 0, spread: 0, min: 0, max: 0, p10: 0, p50: 0, p90: 0 };
  const sorted = radii.slice().sort((a, b) => a - b);
  const mean = radii.reduce((a, b) => a + b, 0) / radii.length;
  const std = Math.sqrt(radii.reduce((a, b) => a + (b - mean) ** 2, 0) / radii.length);
  const p10 = pct(sorted, 0.1), p90 = pct(sorted, 0.9);
  return {
    n: radii.length, mean: +mean.toFixed(2), std: +std.toFixed(2), spread: +(p90 - p10).toFixed(2),
    min: +sorted[0].toFixed(2), max: +sorted[sorted.length - 1].toFixed(2),
    p10: +p10.toFixed(2), p50: +pct(sorted, 0.5).toFixed(2), p90: +p90.toFixed(2),
  };
}

// 低分辨率签名：基线不进版本库的整张 PNG（一张 1 MB），存网格即可 —— 又小又能 diff。
export function signature(img, gx = 24, gy = 15) {
  const cells = [];
  for (let j = 0; j < gy; j++) {
    for (let i = 0; i < gx; i++) {
      const x0 = Math.floor((i * img.width) / gx), x1 = Math.max(x0 + 1, Math.floor(((i + 1) * img.width) / gx));
      const y0 = Math.floor((j * img.height) / gy), y1 = Math.max(y0 + 1, Math.floor(((j + 1) * img.height) / gy));
      let r = 0, g = 0, b = 0, n = 0;
      for (let y = y0; y < y1; y += 2) {
        for (let x = x0; x < x1; x += 2) {
          const d = (y * img.width + x) * 4;
          r += img.data[d]; g += img.data[d + 1]; b += img.data[d + 2]; n++;
        }
      }
      cells.push([Math.round(r / n), Math.round(g / n), Math.round(b / n)]);
    }
  }
  return { gx, gy, cells };
}

export function signatureDiff(a, b) {
  let sum = 0, max = 0, n = 0;
  for (let i = 0; i < Math.min(a.cells.length, b.cells.length); i++) {
    for (let c = 0; c < 3; c++) {
      const d = Math.abs(a.cells[i][c] - b.cells[i][c]);
      sum += d; max = Math.max(max, d); n++;
    }
  }
  return { meanAbs: +(sum / Math.max(1, n)).toFixed(2), maxAbs: max };
}

// 一行/一列的亮度剖面（人眼看「日缘那道缝」「临边是否台阶」最直接）。
export function rowProfile(img, y, x0 = 0, x1 = img.width - 1, step = 1) {
  const out = [];
  for (let x = x0; x <= x1; x += step) out.push(+pxLum(img, x, y).toFixed(1));
  return out;
}
