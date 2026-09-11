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
const listed = [...block[1].matchAll(/'([^']+)'/g)].map((m) => m[1]).sort();

const walk = (d, out = []) => {
  for (const e of readdirSync(d, { withFileTypes: true })) {
    const p = join(d, e.name);
    if (e.isDirectory()) walk(p, out);
    else out.push(relative(SHADERS, p).split(sep).join('/'));
  }
  return out;
};
const disk = walk(SHADERS).sort();

const missing = disk.filter((f) => !listed.includes(f));
const extra = listed.filter((f) => !disk.includes(f));

if (missing.length || extra.length) {
  if (missing.length) console.log('  清单漏登记（运行时会 include 解不开）: ' + missing.join(', '));
  if (extra.length) console.log('  清单里有磁盘上不存在的项（fetch 会 404）: ' + extra.join(', '));
  process.exit(1);
}
console.log(`check-glsl-manifest: OK（${disk.length} 个 chunk 与磁盘一致）`);
