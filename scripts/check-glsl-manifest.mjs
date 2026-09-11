#!/usr/bin/env node
// GLSL 清单核对：`map3d/glsl.js::CHUNKS` 必须与 `web/static/shaders/` 下的文件**逐项相等**。
//
// 为什么单独做一道门：chunk 是靠运行时 `fetch` 装载的，**少登记一个文件的症状是
// 运行时报 `Can not resolve #include <px/...>`** —— 本地门禁全绿、页面却白屏。
// 而「新增一个 .glsl 时忘了往清单里加一行」是这类工作的常见疏漏。
// 反过来，清单里有磁盘上不存在的项，就是 `fetch` 404。
//
// 用法：node scripts/check-glsl-manifest.mjs
import { readFileSync, readdirSync } from 'node:fs';
import { join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = fileURLToPath(new URL('..', import.meta.url));
const GLSL_JS = join(ROOT, 'web/static/map3d/glsl.js');
const SHADERS = join(ROOT, 'web/static/shaders');

const src = readFileSync(GLSL_JS, 'utf8');
const block = /export const CHUNKS = \[([\s\S]*?)\];/.exec(src);
if (!block) {
  console.log('check-glsl-manifest: glsl.js 里找不到 `export const CHUNKS = [...]`');
  process.exit(1);
}
// **真的把数组求值**，而不是拿正则抠引号里的名字 —— 正则看不见数组空洞：
// 生成器曾经把每一项都写成 `'x',` 又用 `,` 连接，于是 32 行全是 `'x',,`（空洞 => undefined）。
//
// ⚠ 更正：我一度把「它没炸」归因于「axum 静态服务对未知路径回落到 index.html 并返回 200」——
// **那是错的**，实测 `/shaders/<不存在的文件>` 返回的是正经 **404**。真正的原因是
// **`Array.prototype.map` 会跳过数组空洞**：`CHUNKS.map(fetch…)` 压根没为那些空洞发起请求，
// 所以既没有 404 也没有报错 —— 和 `filter` 跳过空洞是同一个 JS 陷阱（下面 `[...listed]`
// 那处也是为它写的）。一个错误但不显眼的成因解释比没有解释更坏，故留此更正。
let listed;
try {
  listed = new Function('return [' + block[1] + '];')();
} catch (e) {
  console.log('check-glsl-manifest: CHUNKS 不是合法的数组字面量 — ' + e.message);
  process.exit(1);
}
// ⚠ 必须先 `[...listed]` **展开**：`Array.prototype.filter` 会**跳过数组空洞**，
// 直接对带空洞的数组 filter 是看不见那些 undefined 的（第一版就这么漏了）。
const flat = [...listed];
const holes = flat.filter((v) => typeof v !== 'string' || v === '');
if (holes.length) {
  console.log(`  清单里有 ${holes.length} 个空洞/非字符串项（多半是多余的逗号）`);
  process.exit(1);
}
listed = [...listed].sort();

const walk = (d, out = []) => {
  for (const e of readdirSync(d, { withFileTypes: true })) {
    const p = join(d, e.name);
    if (e.isDirectory()) walk(p, out);
    else out.push(relative(SHADERS, p).split(sep).join('/'));
  }
  return out;
};
const disk = walk(SHADERS).sort();

// ⚠ 名字必须能被 three 的 include 正则匹配：/^[ \t]*#include +<([\w\d./]+)>/gm
// 那个字符类**不含连字符**，所以 `px/sun/sun-photo.frag` 这种名字会让 #include 整行原样
// 喂给 GLSL 编译器（报 `'include' : invalid directive name`），**网格直接不渲染**。
// 这个坑真发生过：全项目只有那一个文件名带连字符，于是偏偏只有太阳的光球消失，
// 而当时的门禁只拼装 .glsl→.glsl 的 include，**看不见只出现在 JS 里的入口 chunk 名**。
const THREE_INCLUDE_NAME = /^[\w\d./]+$/;
const badName = listed.filter((f) => !THREE_INCLUDE_NAME.test(f));
if (badName.length) {
  console.log('  这些 chunk 名匹配不了 three 的 include 正则（会整行不解析）: ' + badName.join(', '));
  console.log('  合法字符只有 [A-Za-z0-9_./]，**连字符不行**。');
  process.exit(1);
}

const missing = disk.filter((f) => !listed.includes(f));
const extra = listed.filter((f) => !disk.includes(f));

if (missing.length || extra.length) {
  if (missing.length) console.log('  清单漏登记（运行时会 include 解不开）: ' + missing.join(', '));
  if (extra.length) console.log('  清单里有磁盘上不存在的项（fetch 会 404）: ' + extra.join(', '));
  process.exit(1);
}
console.log(`check-glsl-manifest: OK（${disk.length} 个 chunk 与磁盘一致）`);
