#!/usr/bin/env node
// 截图 / 场景测试的入口。
//
//   node scripts/shots/run.mjs --all                 # 全部场景 → scratch/shots/<场景>.png + .json
//   node scripts/shots/run.mjs --scene sun-limb      # 只拍一个
//   node scripts/shots/run.mjs --scene sun-limb --strict   # 判据不过 ⇒ exit 1（合流门用）
//   node scripts/shots/run.mjs --update              # 把当前画面写成基线
//   node scripts/shots/run.mjs --stats <图.png>      # 量**任意**一张图（参考图标定/对比用）
//   node scripts/shots/run.mjs --compare a.png b.png # 两张图的签名差
//   node scripts/shots/run.mjs --list
//
// 两条本仓反复踩的坑，这一层直接堵死：
//   ① **服务必须是本 worktree 的那个**。/api/ping 里有 exe 路径，对不上就拒绝开拍 ——
//      「我改了代码画面不动」有相当一部分是连到了别的 worktree（见 notes/glsl-files.md）。
//   ② **浏览器必须是我自己的**。用 127.0.0.1:9333 上由本脚本拉起的 headless Edge；
//      DSH 的 browser_* 是跨会话共享的，会被别的会话导航走。
//
// 页面标签**不关**（默认）：服务有「最后一个页面关掉就自退」的规则，关了截图器就把服务器
// 也带走了。要收尾用 --close。
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { setTimeout as sleep } from 'node:timers/promises';

import { ensureBrowser, Session, findBrowser } from './cdp.mjs';
import { decode, encode, crop, scale } from './png.mjs';
import * as M from './metrics.mjs';
import { SCENES, sceneByName, sceneUrl, makeCamera, diskRadiusPx, SUN_R } from './scenes.mjs';
import { hdrToScreen, screenToHdr } from './grade.mjs';
import { TUNING } from '../../web/static/map3d/tuning.js';

const HERE = path.dirname(fileURLToPath(import.meta.url));
export const ROOT = path.resolve(HERE, '..', '..');

// --- 参数 -------------------------------------------------------------------
function parseArgs(argv) {
  const out = { scenes: [], out: path.join(ROOT, 'scratch', 'shots'), port: 9333, strict: false, update: false, list: false, json: false, close: false, stats: null, compare: null, crop: null, scaleK: 1, width: null, height: null };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--all') out.all = true;
    else if (a === '--scene') out.scenes.push(argv[++i]);
    else if (a === '--out') out.out = path.resolve(argv[++i]);
    else if (a === '--url') out.url = argv[++i];
    else if (a === '--port') out.port = Number(argv[++i]);
    else if (a === '--strict') out.strict = true;
    else if (a === '--update') out.update = true;
    else if (a === '--list') out.list = true;
    else if (a === '--json') out.json = true;
    else if (a === '--close') out.close = true;
    else if (a === '--stats') out.stats = argv[++i];
    else if (a === '--solve') out.solve = argv[++i].split(',').map(Number);
    else if (a === '--hdr') out.hdr = argv[++i].split(',').map(Number);
    else if (a === '--compare') { out.compare = [argv[++i], argv[++i]]; }
    else if (a === '--crop') out.crop = argv[++i].split(',').map(Number);
    // `--query` 给一次性的 A/B 探针用（「先证明机制通不通，再调参数」）：
    // 例如 --query 'tune=postfx:0' 看「关掉整条后期会怎样」。
    // ⚠ 分隔符用 `;` 不用 `,` —— `tune` 自己的值就是逗号分隔的（`tune=a:1,b:2`）。
    else if (a === '--query') out.query = Object.fromEntries(argv[++i].split(';').filter(Boolean).map((kv) => { const j = kv.indexOf('='); return [kv.slice(0, j), kv.slice(j + 1)]; }));
    else if (a === '--scale') out.scaleK = Number(argv[++i]);
    else if (a === '--width') out.width = Number(argv[++i]);
    else if (a === '--height') out.height = Number(argv[++i]);
    else if (a === '-h' || a === '--help') out.help = true;
    else throw new Error(`不认识的参数：${a}`);
  }
  return out;
}

const HELP = `用法：node scripts/shots/run.mjs [选项]
  --all                    跑全部场景
  --scene <名字>           跑一个场景（可重复）
  --out <目录>             输出目录（默认 scratch/shots）
  --url <http://…>         指定服务地址（默认自动找本 worktree 的实例）
  --port <n>               headless Edge 的 CDP 端口（默认 9333）
  --strict                 判据不过 / 基线漂移 ⇒ exit 1
  --update                 把这次的结果写成基线（scripts/shots/baselines/）
  --stats <png>            量任意一张图（参考图标定）
  --compare <a.png> <b.png>  两张图的签名差
  --crop x,y,w,h --scale k 配合 --stats：裁一块放大再看/再量
  --list                   列出场景
  --close                  拍完关掉标签页（⚠ 服务若只有这一个页面会自退）`;

// --- 找服务 -----------------------------------------------------------------
const normPath = (p) => String(p || '').replace(/\\/g, '/').toLowerCase();

async function ping(port) {
  try {
    const r = await fetch(`http://127.0.0.1:${port}/api/ping`, { signal: AbortSignal.timeout(500) });
    return r.ok ? await r.json() : null;
  } catch { return null; }
}

async function resolveServer(explicit) {
  if (explicit) return { url: explicit.replace(/\/$/, ''), ping: await ping(new URL(explicit).port) };
  const targetRoot = normPath(path.join(ROOT, 'target'));
  const found = [];
  for (let p = 3000; p <= 3010; p++) {
    const j = await ping(p);
    if (!j) continue;
    found.push({ port: p, j });
    if (normPath(j.exe).startsWith(targetRoot)) return { url: `http://127.0.0.1:${p}`, ping: j, all: found };
  }
  const lines = found.map((f) => `  :${f.port}  ${f.j.exe}`).join('\n');
  throw new Error(
    `找不到本 worktree（${ROOT}）的 planet_x_web 实例。\n`
    + `在跑的实例：\n${lines || '  （一个都没有）'}\n`
    + `起服务：pwsh -File ${path.join(ROOT, 'scripts', 'web.ps1')}`,
  );
}

// --- 页内等待 ---------------------------------------------------------------
// GLSL 是**分文件 fetch** 的，装完之前页面不会渲染任何东西。判据要等在「世界建好 + 至少
// 渲染过若干帧」之后 —— 只等 load 事件会拍到一张空场。
const WAIT_READY = `(async () => {
  const t0 = Date.now();
  while (Date.now() - t0 < 20000) {
    const api = window.PlanetXMap;
    const d = api && api.debug ? api.debug() : null;
    if (d && d.bodies > 0 && d.frameMs > 0 && d.programs > 0) return d;
    await new Promise((r) => setTimeout(r, 120));
  }
  return null;
})()`;

const WAIT_FRAMES = (n) => `new Promise((res) => { let k = 0; const f = () => (++k >= ${n} ? res(k) : requestAnimationFrame(f)); requestAnimationFrame(f); })`;

const HIDE_UI = `(() => {
  const s = document.createElement('style');
  s.textContent = '#topbar,#side,.edge-toggle,footer,.tip,#tip,.toast{display:none!important}';
  document.head.appendChild(s);
  return document.querySelector('#map canvas') ? (() => { const r = document.querySelector('#map canvas').getBoundingClientRect(); return { x: r.x, y: r.y, w: r.width, h: r.height }; })() : null;
})()`;

// --- 量一景 -----------------------------------------------------------------
const mean = (xs) => xs.reduce((a, b) => a + b, 0) / Math.max(1, xs.length);

function cornerBoxes(W, H, bw = 160, bh = 120) {
  return [
    { x: 0, y: 0, w: bw, h: bh },
    { x: W - bw, y: 0, w: bw, h: bh },
    { x: 0, y: H - bh, w: bw, h: bh },
    { x: W - bw, y: H - bh, w: bw, h: bh },
  ];
}

function measure(img, scene) {
  const W = img.width, H = img.height, vp = [W, H];
  const g = scene.geom || {};
  const detail = {};
  const flat = {};

  // 参考几何：日面圆心投在哪儿、半径多少像素。
  let cx = W / 2, cy = H / 2, R = null;
  if (scene.view && g.d !== undefined) {
    const cam = makeCamera(scene.view.pos, scene.view.target, vp);
    const c = cam.project([0, 0, 0]);
    if (c) { cx = c[0]; cy = c[1]; }
    R = diskRadiusPx(g.d, vp);
  }

  const corners = cornerBoxes(W, H);
  const sky = corners.map((b) => M.stats(img, b));
  flat.skyMeanLum = +mean(sky.map((s) => s.meanLum)).toFixed(2);
  flat.cornerMeanLum = flat.skyMeanLum;
  flat.skyMax = +Math.max(...sky.map((s) => s.max)).toFixed(2);
  detail.sky = sky;
  flat.global = M.stats(img, { x: 0, y: 0, w: W, h: H });
  flat.frameMeanLum = flat.global.meanLum;
  detail.global = flat.global;
  flat.centerMeanLum = M.stats(img, { x: W * 0.4, y: H * 0.4, w: W * 0.2, h: H * 0.2 }).meanLum;

  if (R) {
    const r = R * 0.6;
    const disk = M.stats(img, { x: cx - r, y: cy - r, w: 2 * r, h: 2 * r });
    detail.disk = disk;
    flat.diskMeanLum = disk.meanLum;
    flat.diskWarmth = disk.warmth;
    flat.diskWhiteFrac = disk.whiteFrac;
    flat.diskP90minusP50 = +(disk.p90 - disk.p50).toFixed(2);
    detail.ringNear = M.annulus(img, cx, cy, R * 1.0, R * 1.12);
    detail.ringFar = M.annulus(img, cx, cy, R * 1.6, R * 2.2);
    flat.ringNearLum = detail.ringNear.meanLum;
    flat.ringFarLum = detail.ringFar.meanLum;
    flat.promContrast = +(detail.ringNear.meanLum / Math.max(0.5, detail.ringFar.meanLum)).toFixed(3);
    // 日面圆心是否真的在画面里（相机被 OrbitControls 夹走时这条会立刻炸）
    flat.diskCenterOnScreen = (cx > 0 && cx < W && cy > 0 && cy < H) ? 1 : 0;
    detail.radial = M.radialProfile(img, cx, cy, R * 0.9, R * 2.4, 8);
    // ⚠ 阈值必须**抬到背景之上**：天上有星云（~26）与恒星，thr=6 会把它们也算成「日珥」。
    // 低阈值量"日珥能伸多高"、高阈值量"亮核有多高"，两个都给，避免阈值一改结论就翻。
    const silThr = g.silThr ?? 6;
    const sil = M.silhouette(img, { cx, cy, maxR: R * (g.silMaxR ?? 1.5), thr: silThr, n: 240 });
    const silB = M.silhouette(img, { cx, cy, maxR: R * (g.silMaxR ?? 1.5), thr: silThr * 3.5, n: 240 });
    detail.silhouette = sil;
    detail.silhouetteBright = silB;
    flat.edgeStd = sil.std;
    flat.edgeSpread = sil.spread;
    flat.edgeMeanOverR = +(sil.mean / R).toFixed(4);
    flat.edgeStdBright = silB.std;
  }
  detail.profileRow = M.rowProfile(img, Math.round(cy), 0, W - 1, Math.max(1, Math.round(W / 64)));
  return { flat, detail };
}

function checkLimits(flat, limits = {}) {
  const bad = [];
  for (const [k, [lo, hi]] of Object.entries(limits)) {
    if (!(k in flat)) { bad.push(`${k}: 量不到（判据名写错了？）`); continue; }
    if (flat[k] < lo || flat[k] > hi) bad.push(`${k} = ${flat[k]}，要求 [${lo}, ${hi}]`);
  }
  return bad;
}

const baselineFile = (name) => path.join(HERE, 'baselines', `${name}.json`);

// --- main -------------------------------------------------------------------
async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.help || (!args.all && !args.scenes.length && !args.stats && !args.compare && !args.list && !args.solve && !args.hdr)) {
    console.log(HELP);
    return 0;
  }
  if (args.list) {
    for (const s of SCENES) console.log(`${s.name.padEnd(12)} ${s.desc}`);
    return 0;
  }

  // ⓪ 配色换算：参考图上的 sRGB ⇄ 着色器要输出的线性 HDR（ACES 正/反算）
  if (args.solve) {
    const hdr = screenToHdr(args.solve);
    console.log(`screen rgb(${args.solve.join(', ')})  <-  HDR [${hdr.map((v) => v.toFixed(4)).join(', ')}]`
      + `  (exposure ${TUNING.exposure})  回代=${hdrToScreen(hdr).join(',')}`);
    return 0;
  }
  if (args.hdr) {
    console.log(`HDR [${args.hdr.join(', ')}]  ->  screen rgb(${hdrToScreen(args.hdr).join(', ')})`);
    return 0;
  }

  // ① 量任意图（参考图标定 / A-B 对比）—— 不需要服务，也不需要浏览器
  if (args.stats) {
    let img = decode(fs.readFileSync(args.stats));
    if (args.crop) img = crop(img, ...args.crop);
    if (args.scaleK !== 1) img = scale(img, args.scaleK);
    const W = img.width, H = img.height;
    const global = M.stats(img, { x: 0, y: 0, w: W, h: H });
    const center = M.stats(img, { x: W * 0.3, y: H * 0.3, w: W * 0.4, h: H * 0.4 });
    const corners = cornerBoxes(W, H, Math.round(W * 0.14), Math.round(H * 0.14)).map((b) => M.stats(img, b));
    const sil = M.silhouette(img, { cx: W / 2, cy: H / 2, maxR: Math.min(W, H) * 0.7, thr: 6, n: 180 });
    const out = {
      file: args.stats, crop: args.crop || null, scale: args.scaleK, size: [W, H],
      global, center, skyMeanLum: +mean(corners.map((c) => c.meanLum)).toFixed(2),
      centerWarmth: center.warmth, centerWhiteFrac: center.whiteFrac,
      centerP90minusP50: +(center.p90 - center.p50).toFixed(2),
      silhouetteFromImageCenter: sil,
      signature: M.signature(img, 24, 15),
    };
    const jsonPath = args.stats.replace(/\.png$/i, '') + '.stats.json';
    fs.writeFileSync(jsonPath, JSON.stringify(out, null, 1));
    console.log(JSON.stringify({ ...out, signature: undefined }, null, 1));
    if (args.crop && args.scaleK !== 1) {
      const p = args.stats.replace(/\.png$/i, '') + `.crop${args.scaleK}x.png`;
      fs.writeFileSync(p, encode(img));
      console.log('WROTE ' + p);
    }
    return 0;
  }
  if (args.compare) {
    const [a, b] = args.compare;
    const ia = decode(fs.readFileSync(a)), ib = decode(fs.readFileSync(b));
    const d = M.signatureDiff(M.signature(ia), M.signature(ib));
    console.log(`${path.basename(a)} vs ${path.basename(b)}: meanAbs=${d.meanAbs} maxAbs=${d.maxAbs}`);
    return 0;
  }

  // ② 场景测试
  const scenes = args.all ? SCENES : args.scenes.map((n) => {
    const s = sceneByName(n);
    if (!s) throw new Error(`没有这个场景：${n}（--list 看全部）`);
    return s;
  });

  const { url, ping: pj } = await resolveServer(args.url);
  console.log(`SERVE ${url}  pid=${pj?.pid}  exe=${pj?.exe}`);
  if (normPath(pj?.exe).indexOf(normPath(ROOT)) < 0) {
    console.log(`⚠ 服务不在本 worktree 下：${pj?.exe}（本 worktree = ${ROOT}）`);
  }

  const profileDir = path.join(ROOT, 'scratch', '.edge-profile');
  const br = await ensureBrowser({ port: args.port, profileDir, width: 1280, height: 800 });
  console.log(`BROWSER :${args.port}${br.spawned ? `（新拉起：${br.exe || findBrowser()}）` : '（复用已在跑的）'}`);
  const sess = await Session.connect(args.port);

  fs.mkdirSync(args.out, { recursive: true });
  const manifest = { at: new Date().toISOString(), server: url, scenes: {} };
  let failures = 0;

  for (const scene of scenes) {
    // `--query` 给一次性的 A/B 探针用（「先证明机制通不通，再调参数」）：
    // 例如 --query tune=postfx:0 看「关掉整条后期会怎样」。
    const sc = args.query ? { ...scene, query: { ...scene.query, ...args.query } } : scene;
    const vp = args.width && args.height ? [args.width, args.height] : sc.viewport;
    await sess.viewport(vp[0], vp[1], 1);
    const url2 = sceneUrl(url, sc);
    await sess.goto(url2, { waitMs: 250 });
    const ready = await sess.eval(WAIT_READY);
    if (!ready) throw new Error(`${scene.name}: 页面 20 s 内没进入可拍状态（看 console）`);
    const rect = await sess.eval(HIDE_UI);
    if (scene.view) await sess.eval(`window.PlanetXMap.setView(${JSON.stringify(scene.view.pos)}, ${JSON.stringify(scene.view.target)}); 1`);
    await sess.eval(WAIT_FRAMES(45));
    await sleep(scene.hold ?? 500);
    // ⚠ `ready` 是**取景之前**读的（它在 setView 之前），所以那里的 promLod/frameMs 是
    //   "fitCamera 的远景"那一下的数 —— 用它判断"这一景最终用了哪档 LOD"会**正好反着**：
    //   贴近日面的 limb 景会被记成最粗的 lod=2。取景之后要再读一次。
    const settled = await sess.eval('(() => { const d = window.PlanetXMap.debug(); return { promLod: d.promLod, frameMs: d.frameMs }; })()');

    const png = await sess.screenshot({ clip: rect ? { x: rect.x, y: rect.y, width: rect.w, height: rect.h } : undefined });
    const tag = args.query ? `${scene.name}.probe` : scene.name;
    const pngPath = path.join(args.out, `${tag}.png`);
    fs.writeFileSync(pngPath, png);
    const img = decode(png);
    const { flat, detail } = measure(img, scene);
    const sig = M.signature(img, 24, 15);
    const bad = checkLimits(flat, scene.limits);
    const errs = sess.errors.slice(-6);

    // 基线：签名变小图存 JSON（整张 PNG 一张 1 MB，不该进版本库）
    let drift = null;
    const bf = baselineFile(scene.name);
    if (args.update) {
      fs.mkdirSync(path.dirname(bf), { recursive: true });
      fs.writeFileSync(bf, JSON.stringify({ scene: scene.name, at: manifest.at, limits: scene.limits, flat, signature: sig }, null, 1));
    } else if (fs.existsSync(bf)) {
      const b = JSON.parse(fs.readFileSync(bf, 'utf8'));
      drift = M.signatureDiff(b.signature, sig);
      if (drift.meanAbs > (args.strict ? 6 : 1e9)) bad.push(`基线漂移 meanAbs=${drift.meanAbs} > 6`);
    }

    const rec = { desc: scene.desc, url: url2, viewport: vp, ready: { tier: ready.tier, gpu: ready.gpu, bodies: ready.bodies, programs: ready.programs, promLod: settled && settled.promLod, frameMs: settled && settled.frameMs }, flat, detail, limits: scene.limits, failures: bad, drift, consoleErrors: errs };
    fs.writeFileSync(path.join(args.out, `${tag}.json`), JSON.stringify(rec, null, 1));
    manifest.scenes[scene.name] = { flat, failures: bad, drift, gpu: ready.gpu, tier: ready.tier };
    if (bad.length) failures++;
    console.log(`${bad.length ? '✗' : '✓'} ${scene.name.padEnd(11)} ${pngPath}  tier=${ready.tier} promLod=${settled && settled.promLod} gpu=${String(ready.gpu).slice(0, 34)}`);
    for (const [k, v] of Object.entries(flat)) if (typeof v === 'number') process.stdout.write(`    ${k}=${v}`.padEnd(30).replace(/^ {4}/, '    '));
    console.log('');
    if (drift) console.log(`    基线漂移 meanAbs=${drift.meanAbs} maxAbs=${drift.maxAbs}`);
    for (const b of bad) console.log(`    判据不过：${b}`);
    for (const e of errs) console.log(`    console error: ${e}`);
  }
  fs.writeFileSync(path.join(args.out, 'manifest.json'), JSON.stringify(manifest, null, 1));
  if (args.close) await sess.closeTarget();
  console.log(`\n共 ${scenes.length} 景，${failures} 景有未过判据。输出：${args.out}`);
  return args.strict && failures ? 1 : 0;
}

main()
  .then((c) => process.exit(c))
  .catch((e) => { console.error('ERROR ' + (e && e.stack ? e.stack : e)); process.exit(2); });
