#!/usr/bin/env node
// 参考图量具：把**任何**一张参考图（SDO/AIA 的日面、别人的渲染图、概念图）量成
// 「径向剖面 + 分位数 + 日珥能伸多高」三张表，好把"像不像"变成可比的数。
//
// 为什么单独一个工具（而不是塞进 run.mjs）：`--stats` 量的是**本工程渲染出来的图**
// （它知道相机、知道圆心和半径）；参考图是**别人给的像素**，圆心/半径得先自己找出来，
// 而且要看的是"分布"（分位数、径向）而不是"这一景过不过判据"。
//
//   node scripts/shots/refstat.mjs scratch/ref/sdo-304.png                 # 自动找日面
//   node scripts/shots/refstat.mjs ref.png --disk 236.8,145.4,112          # 手动给圆心+半径
//   node scripts/shots/refstat.mjs ref.png --box 120,240,520,220           # 只量一块矩形（特写图）
//   node scripts/shots/refstat.mjs a.png b.png                             # 多张并排对比
//
// 量什么、为什么量这些（都是这几轮真用到的判据）：
//   · 分位数 p5/p25/p50/p75/p95/p99 —— 参考图之间差得最远的其实是**两端**：
//     "发闷"的那种渲染 p5..p95 挤在一起，"有内发光层次"的拉得很开。
//   · 盘面上的 lum<60 占比 —— 日面的暗纹是**细丝**还是**大陆**，这一条最灵敏。
//   · 径向剖面 —— 304Å 是**临边增亮**（盘心最暗、一路亮到边），做反了会得到一颗"空心球"。
//   · 日珥外沿半径 /R 的 p50/p90 —— "日珥伸出去多远"，拿它定插片的高度，不用靠眼睛猜。
import fs from 'node:fs';
import path from 'node:path';
import { decode } from './png.mjs';

const lum = (r, g, b) => 0.2126 * r + 0.7152 * g + 0.0722 * b;

function parseArgs(argv) {
  const out = { files: [], disk: null, box: null, thr: 45, furMax: 2.0 };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--disk') out.disk = argv[++i].split(',').map(Number);
    else if (a === '--box') out.box = argv[++i].split(',').map(Number);
    else if (a === '--thr') out.thr = Number(argv[++i]);
    else if (a === '--furmax') out.furMax = Number(argv[++i]);
    else if (a === '-h' || a === '--help') out.help = true;
    else out.files.push(a);
  }
  return out;
}

// 自动找日面：亮度 > 阈值的像素的重心 + 包围盒高度的一半当半径。
// ⚠ 包围盒的**宽**会被侧面的日珥撑大（横着伸出去的那条），所以半径取高/宽里**小**的那个。
function findDisk(img, thr = 70) {
  const { width: w, height: h, data } = img;
  const ch = data.length / (w * h);
  let sx = 0, sy = 0, n = 0, minx = 1e9, maxx = -1e9, miny = 1e9, maxy = -1e9;
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
    const i = (y * w + x) * ch;
    if (lum(data[i], data[i + 1], data[i + 2]) > thr) {
      sx += x; sy += y; n++;
      if (x < minx) minx = x; if (x > maxx) maxx = x;
      if (y < miny) miny = y; if (y > maxy) maxy = y;
    }
  }
  if (!n) return null;
  return { cx: sx / n, cy: sy / n, R: Math.min(maxx - minx, maxy - miny) / 2 };
}

function quantiles(list, ps) {
  const v = list.slice().sort((a, b) => a[0] - b[0]);
  const q = (p) => v[Math.min(v.length - 1, Math.floor(p * v.length))];
  return ps.map((p) => [p, q(p)]);
}

function report(file, args) {
  const img = decode(fs.readFileSync(file));
  const { width: w, height: h, data } = img;
  const ch = data.length / (w * h);
  const at = (x, y) => { const i = (y * w + x) * ch; return [data[i], data[i + 1], data[i + 2]]; };

  console.log(`\n=== ${file}  ${w}x${h}`);
  let cx, cy, R;
  if (args.disk) { [cx, cy, R] = args.disk; } else if (!args.box) {
    const d = findDisk(img);
    if (!d) { console.log('  找不到日面（全图都很暗？）—— 用 --disk cx,cy,R 手动给'); return; }
    ({ cx, cy, R } = d);
  }
  if (!args.box) console.log(`  日面：圆心 (${cx.toFixed(1)}, ${cy.toFixed(1)})  半径 ${R.toFixed(1)} px`);

  const inside = [];
  if (args.box) {
    const [x0, y0, bw, bh] = args.box;
    for (let y = y0; y < y0 + bh; y++) for (let x = x0; x < x0 + bw; x++) {
      const [r, g, b] = at(x, y); inside.push([lum(r, g, b), r, g, b]);
    }
    console.log(`  取样矩形 ${args.box.join(',')}（${inside.length} px）`);
  } else {
    for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
      if (Math.hypot(x - cx, y - cy) < R * 0.92) { const [r, g, b] = at(x, y); inside.push([lum(r, g, b), r, g, b]); }
    }
    // 径向剖面（相对半径 0..1.55，每格 0.05）
    const bins = Array.from({ length: 32 }, () => ({ s: 0, n: 0 }));
    for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
      const d = Math.hypot(x - cx, y - cy) / R;
      if (d >= 1.6) continue;
      const [r, g, b] = at(x, y);
      const bi = Math.floor(d * 20); bins[bi].s += lum(r, g, b); bins[bi].n++;
    }
    console.log('  径向剖面 lum（r/R：值）');
    console.log('   ' + bins.map((b, i) => `${(i / 20).toFixed(2)}:${b.n ? (b.s / b.n).toFixed(0) : '-'}`).join(' '));
  }

  const mean = inside.reduce((a, v) => [a[0] + v[0], a[1] + v[1], a[2] + v[2], a[3] + v[3]], [0, 0, 0, 0])
    .map((v) => v / inside.length);
  console.log(`  平均 lum ${mean[0].toFixed(1)}  rgb(${mean.slice(1).map((v) => v.toFixed(0)).join(',')})`);
  for (const [p, v] of quantiles(inside, [0.05, 0.25, 0.5, 0.75, 0.95, 0.99])) {
    console.log(`  p${String(Math.round(p * 100)).padStart(2)}  lum ${v[0].toFixed(0).padStart(3)}  rgb(${v.slice(1).map((x) => x.toFixed(0)).join(',')})`);
  }
  const frac = (f) => (inside.filter(f).length / inside.length * 100).toFixed(1) + '%';
  console.log(`  暗纹 lum<60 ${frac((v) => v[0] < 60)}   亮核 lum>=190 ${frac((v) => v[0] >= 190)}`
    + `   白热 lum>=220 ${frac((v) => v[0] >= 220)}`);
  console.log(`  极差 p95-p5 = ${(quantiles(inside, [0.05, 0.95])[1][1][0] - quantiles(inside, [0.05])[0][1][0]).toFixed(0)}`
    + `   p95-p50 = ${(quantiles(inside, [0.95])[0][1][0] - quantiles(inside, [0.5])[0][1][0]).toFixed(0)}`);

  if (args.box) return;   // 特写图没有"日面圆心"，量不了日珥伸多高

  // 日珥外沿：每个角度往外扫，最远一处仍亮于 thr 的半径（相对 R）
  const outs = [];
  for (let k = 0; k < 180; k++) {
    const a = (k * Math.PI) / 90;
    let last = 0;
    for (let rr = 0.6; rr < args.furMax; rr += 0.004) {
      const x = Math.round(cx + Math.cos(a) * rr * R), y = Math.round(cy + Math.sin(a) * rr * R);
      if (x < 0 || y < 0 || x >= w || y >= h) break;
      if (lum(...at(x, y)) > args.thr) last = rr;
    }
    outs.push(last);
  }
  outs.sort((a, b) => a - b);
  const oq = (p) => outs[Math.floor(p * outs.length)];
  console.log(`  日珥外沿 /R (thr=${args.thr}): p10 ${oq(0.1).toFixed(3)}  p50 ${oq(0.5).toFixed(3)}`
    + `  p90 ${oq(0.9).toFixed(3)}  max ${outs[outs.length - 1].toFixed(3)}`);
}

const args = parseArgs(process.argv.slice(2));
if (args.help || !args.files.length) {
  console.log('用法：node scripts/shots/refstat.mjs <图.png> [更多图…] [--disk cx,cy,R] [--box x,y,w,h] [--thr 45]');
  process.exit(args.help ? 0 : 1);
}
for (const f of args.files) report(path.resolve(f), args);
