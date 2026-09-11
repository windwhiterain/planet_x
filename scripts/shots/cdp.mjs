// 最小 CDP 客户端 —— 零依赖（Node ≥ 22 自带全局 WebSocket），负责拉起并驱动一个**专用**的
// headless Edge。
//
// 为什么不用 DSH 的 `browser_*` 工具：那个浏览器**跨 session 共享**，别的会话一导航，我拍到
// 的就是别人的页面、量到的也是别人那份代码（`.agents/notes/glsl-files.md` 里为此白烧了七八轮）。
// 为什么不用 Playwright/Puppeteer：本仓库**刻意没有构建步骤**（three 走 CDN importmap、
// 静态文件直接 fetch），为一台截图器引一个 node_modules 与整棵依赖树不成比例；CDP 协议本身
// 就是个 WebSocket + JSON-RPC，60 行足够。
//
// `--headless=new` 走的是**真 GPU**（不是 SwiftShader），所以截图与帧率都有代表性
// （`.agents/notes/web-vfx-pipeline.md` §0.3）。
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';
import { setTimeout as sleep } from 'node:timers/promises';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');

const EDGE_CANDIDATES = [
  'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',
  'C:/Program Files/Microsoft/Edge/Application/msedge.exe',
  process.env.LOCALAPPDATA && `${process.env.LOCALAPPDATA}/Microsoft/Edge/Application/msedge.exe`,
  '/usr/bin/microsoft-edge',
  '/usr/bin/google-chrome',
  '/usr/bin/chromium',
].filter(Boolean);

export function findBrowser() {
  if (process.env.PX_BROWSER && fs.existsSync(process.env.PX_BROWSER)) return process.env.PX_BROWSER;
  for (const p of EDGE_CANDIDATES) if (fs.existsSync(p)) return p;
  return null;
}

async function httpJson(url, opts = {}, timeoutMs = 1200) {
  const r = await fetch(url, { ...opts, signal: AbortSignal.timeout(timeoutMs) });
  if (!r.ok) throw new Error(`${url} → HTTP ${r.status}`);
  const txt = await r.text();
  try { return JSON.parse(txt); } catch { return txt; }
}

export async function cdpUp(port, timeoutMs = 800) {
  try { return !!(await httpJson(`http://127.0.0.1:${port}/json/version`, {}, timeoutMs)); }
  catch { return false; }
}

// 浏览器**常驻**：已经在 9333 上跑着就直接复用（冷启动 ~1.5 s，热连 ~50 ms；一次迭代要拍
// 七八张图，这个差别很明显）。profile 目录放在调用方给的 scratch 下 —— 它是机器级临时物，
// 不是分支级资产。
export async function ensureBrowser({ port = 9333, profileDir, width = 1280, height = 800, timeoutMs = 25000 } = {}) {
  if (await cdpUp(port)) return { port, spawned: false };
  const exe = findBrowser();
  if (!exe) throw new Error('找不到 Edge/Chrome；用 PX_BROWSER=<exe 路径> 指定');
  if (profileDir) fs.mkdirSync(profileDir, { recursive: true });
  const args = [
    '--headless=new',
    `--remote-debugging-port=${port}`,
    ...(profileDir ? [`--user-data-dir=${profileDir}`] : []),
    `--window-size=${width},${height}`,
    '--no-first-run', '--no-default-browser-check', '--disable-extensions',
    '--hide-scrollbars', '--force-device-scale-factor=1',
    '--disable-background-timer-throttling', '--disable-renderer-backgrounding',
    // 无头下默认可能把 WebGL 关掉；显式要求 GPU 光栅化。
    '--use-gl=angle', '--use-angle=d3d11', '--enable-gpu-rasterization',
    'about:blank',
  ];
  spawn(exe, args, { detached: true, stdio: 'ignore' }).unref();
  const t0 = Date.now();
  while (Date.now() - t0 < timeoutMs) {
    if (await cdpUp(port)) return { port, spawned: true, exe };
    await sleep(250);
  }
  throw new Error(`headless 浏览器 ${timeoutMs} ms 内没起来（${exe} :${port}）`);
}

// 找一个**本工具自己的**页面来复用（指向 127.0.0.1 的游戏页；`about:blank`/devtools 不算）。
//
// 为什么需要它：`Session.connect` 每次 `newPage` 一个，而 `run.mjs` 又**故意不关**
// 那个页面（服务有「最后一个页面关掉就自退」的规则）⇒ **每跑一次截图就漏一个页面**，
// 而每个页面都在跑自己的 rAF 渲染循环。实测跑了一轮性能探针之后浏览器里堆了 **37 个**
// 活着的 WebGL 页面，把同一台机器上**用户正在玩的游戏**拖卡了（用户原话：
// 「卡是因为和你在一起跑」）。复用同一个页面就既不会漏、也不会让服务自退。
export async function reusablePage(port) {
  const list = await httpJson(`http://127.0.0.1:${port}/json/list`);
  return (Array.isArray(list) ? list : [])
    .find((t) => t.type === 'page' && /^http:\/\/127\.0\.0\.1:\d+\//.test(t.url || '')) || null;
}

export async function newPage(port) {
  // 新版 Chrome/Edge 的 /json/new 只接受 PUT，老版只接受 GET —— 两个都试。
  for (const method of ['PUT', 'GET']) {
    try {
      const t = await httpJson(`http://127.0.0.1:${port}/json/new?${encodeURIComponent('about:blank')}`, { method });
      if (t && t.webSocketDebuggerUrl) return t;
    } catch { /* 试下一种 */ }
  }
  // 兜底：复用已有标签页。
  const list = await httpJson(`http://127.0.0.1:${port}/json/list`);
  const page = (Array.isArray(list) ? list : []).find((t) => t.type === 'page');
  if (!page) throw new Error('CDP 上没有可用的标签页');
  return page;
}

const sleepMs = (n) => new Promise((r) => setTimeout(r, n));

// --- CDN 本地缓存 -------------------------------------------------------------
//
// **为什么要它**：`index.html` 的 importmap 把 three.js / addons 指向 jsdelivr，而本工具
// 为了让改完的本地 .js/.frag 立刻生效，**全局关掉了浏览器缓存** ⇒ 每次重载都要重新从
// CDN 拉一遍（three.module.js 就 1.3 MB，还有几十个 addon）。
// 后果有二：① 每跑一景白花 1 s 网络；② **网络一抖整批截图直接失败**
// （实测 `ERR_NETWORK_CHANGED` / `ERR_TIMED_OUT`，而同一时刻宿主 curl 是 200 ——
//   用户那次限速到 1.3 MB / 7~10 s，20 s 的"可拍"窗口直接超时）。
//
// 做法：把 jsdelivr 的响应**落盘到 scratch/**（不是版本库资产，是本地构建缓存），
// 再用 CDP 的 `Fetch` 域直接从内存回填 ⇒ 第一次跑把它填满，之后**离线可跑**。
// 只拦 jsdelivr：本地 127.0.0.1 的请求照旧直连（那才是"改完要立刻生效"的部分）。
const CDN_HOST = 'cdn.jsdelivr.net';
const CDN_CACHE_DIR = path.join(ROOT, 'scratch', '.cdn-cache');

function cdnCachePath(url) {
  const u = new URL(url);
  const rel = (u.host + u.pathname).replace(/[^A-Za-z0-9._/-]/g, '_');
  return path.join(CDN_CACHE_DIR, rel);
}

/// 取一个 CDN 文件：命中缓存直接读盘；否则用 node 拉一次（带重试）并落盘。
/// 失败返回 null（调用方退回让浏览器自己请求）。
async function cdnBytes(url, { tries = 3, timeoutMs = 30000 } = {}) {
  const p = cdnCachePath(url);
  try { return await fs.readFile(p); } catch { /* 未命中 */ }
  for (let i = 0; i < tries; i++) {
    try {
      const r = await fetch(url, { signal: AbortSignal.timeout(timeoutMs) });
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      const buf = Buffer.from(await r.arrayBuffer());
      await fs.mkdir(path.dirname(p), { recursive: true });
      await fs.writeFile(p, buf);
      return buf;
    } catch (e) {
      if (i === tries - 1) return null;
      await sleepMs(500 * (i + 1));
    }
  }
  return null;
}

/// 打开 CDN 拦截（`Session.connect` 里调）。命中的请求**由我们回填**，
/// 未命中的先拉下来（这样第一次跑就把缓存填满），拉不到才交回浏览器。
async function installCdnCache(sess) {
  sess.on('Fetch.requestPaused', async (params) => {
    const { requestId, request } = params;
    try {
      if (!request.url.includes(CDN_HOST)) return void sess.send('Fetch.continueRequest', { requestId });
      const buf = await cdnBytes(request.url);
      if (!buf) return void sess.send('Fetch.continueRequest', { requestId });
      await sess.send('Fetch.fulfillRequest', {
        requestId,
        responseCode: 200,
        responseHeaders: [
          { name: 'Content-Type', value: 'text/javascript; charset=utf-8' },
          { name: 'Access-Control-Allow-Origin', value: '*' },
          // 版本号在 URL 里 ⇒ 不可变，给一个长缓存（浏览器那份虽然被全局关了，但语义要对）
          { name: 'Cache-Control', value: 'public, max-age=31536000, immutable' },
        ],
        body: buf.toString('base64'),
      });
    } catch {
      try { await sess.send('Fetch.continueRequest', { requestId }); } catch { /* 已经走了 */ }
    }
  });
  await sess.send('Fetch.enable', {
    patterns: [{ urlPattern: `https://${CDN_HOST}/*`, requestStage: 'Request' }],
  });
}

export class Session {
  constructor(ws, target) {
    this.ws = ws;
    this.target = target;
    this.id = 0;
    this.pending = new Map();
    this.logs = [];        // console 输出（error/warning）
    this.errors = [];      // 未捕获异常 + console.error + Log.entryAdded(severity=error)
    this.events = new Map();
    // **常驻**处理器（`once` 是一次性的，Fetch 拦截这类要一直挂着）
    this.handlers = new Map();
    this.closed = false;
    ws.addEventListener('message', (ev) => this._onMessage(ev));
    ws.addEventListener('close', () => { this.closed = true; });
  }

  static async connect(port, { reuse = true } = {}) {
    // **优先复用**已有的游戏页面（见 `reusablePage` 上面那段：不复用会每跑一次漏一个）。
    // 只有一个页面时会抢 —— 本工具本来就是**独占**那个私有浏览器（9333）的，
    // 所以不并发跑两个 run.mjs 就不会撞。
    const target = (reuse ? await reusablePage(port) : null) || await newPage(port);
    const ws = new WebSocket(target.webSocketDebuggerUrl);
    await new Promise((res, rej) => {
      ws.addEventListener('open', res, { once: true });
      ws.addEventListener('error', () => rej(new Error('CDP WebSocket 连接失败')), { once: true });
    });
    const s = new Session(ws, target);
    await s.send('Page.enable');
    await s.send('Runtime.enable');
    try { await s.send('Log.enable'); } catch { /* 老协议没有 */ }
    await s.send('Network.enable');
    // **静态文件没有 cache 头** ⇒ 浏览器会按启发式缓存住旧的 map3d.js / *.frag，于是
    // 「改了代码画面不动」。这是本仓反复踩的坑，截图器从一开始就关掉缓存。
    await s.send('Network.setCacheDisabled', { cacheDisabled: true });
    // CDN 走本地缓存回填（见 installCdnCache 上面那段：网络一抖整批截图会失败）
    try { await installCdnCache(s); } catch { /* 老协议/enable 失败就退回直连 */ }
    return s;
  }

  _onMessage(ev) {
    let m;
    try { m = JSON.parse(ev.data); } catch { return; }
    if (m.id && this.pending.has(m.id)) {
      const { res, rej } = this.pending.get(m.id);
      this.pending.delete(m.id);
      if (m.error) rej(new Error(`${m.error.message} (${JSON.stringify(m.error.data ?? '')})`));
      else res(m.result);
      return;
    }
    const method = m.method;
    if (!method) return;
    const p = m.params || {};
    if (method === 'Runtime.exceptionThrown') {
      const d = p.exceptionDetails || {};
      this.errors.push(d.exception?.description || d.text || 'exception');
    } else if (method === 'Runtime.consoleAPICalled') {
      const text = (p.args || []).map((a) => a.value ?? a.description ?? a.type).join(' ');
      this.logs.push(`${p.type}: ${text}`);
      if (p.type === 'error') this.errors.push(text);
    } else if (method === 'Log.entryAdded') {
      const e = p.entry || {};
      if (e.level === 'error') this.errors.push(`${e.source}: ${e.text}`);
    }
    const waiters = this.events.get(method);
    if (waiters && waiters.length) {
      this.events.set(method, []);
      waiters.forEach((w) => w(m.params));
    }
    const hs = this.handlers.get(method);
    if (hs) for (const h of hs) { try { h(p); } catch { /* 处理器自己负责 */ } }
  }

  /// 挂一个**常驻**的事件处理器（`once` 只等一次就没了）。
  on(method, cb) {
    const arr = this.handlers.get(method) || [];
    arr.push(cb);
    this.handlers.set(method, arr);
    return this;
  }

  send(method, params = {}) {
    const id = ++this.id;
    return new Promise((res, rej) => {
      this.pending.set(id, { res, rej });
      this.ws.send(JSON.stringify({ id, method, params }));
      setTimeout(() => {
        if (this.pending.has(id)) { this.pending.delete(id); rej(new Error(`CDP 超时：${method}`)); }
      }, 30000);
    });
  }

  once(method, timeoutMs = 15000) {
    return new Promise((res, rej) => {
      const arr = this.events.get(method) || [];
      arr.push(res);
      this.events.set(method, arr);
      setTimeout(() => rej(new Error(`等不到事件 ${method}`)), timeoutMs);
    });
  }

  async viewport(width, height, dsf = 1) {
    await this.send('Emulation.setDeviceMetricsOverride', {
      width, height, deviceScaleFactor: dsf, mobile: false,
    });
  }

  async goto(url, { waitMs = 0 } = {}) {
    const loaded = this.once('Page.loadEventFired').catch(() => null);
    await this.send('Page.navigate', { url });
    await loaded;
    if (waitMs) await sleepMs(waitMs);
  }

  async eval(expression, { awaitPromise = true } = {}) {
    const r = await this.send('Runtime.evaluate', { expression, awaitPromise, returnByValue: true });
    if (r.exceptionDetails) {
      throw new Error(`页面内求值抛错：${r.exceptionDetails.exception?.description || r.exceptionDetails.text}`);
    }
    return r.result?.value;
  }

  async screenshot({ clip } = {}) {
    const params = { format: 'png', captureBeyondViewport: false };
    if (clip) params.clip = { ...clip, scale: 1 };
    const r = await this.send('Page.captureScreenshot', params);
    return Buffer.from(r.data, 'base64');
  }

  async closeTarget() {
    try { this.ws.close(); } catch { /* 忽略 */ }
    try { await httpJson(`http://127.0.0.1:${this.target.id}`, { method: 'PUT' }, 500); } catch { /* 忽略 */ }
  }
}

export { sleepMs };
