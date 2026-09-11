// GLSL 模板字符串里的反引号哨兵。
//
// 背景：本目录的 GLSL 全部写在 JS 模板字符串里（`const X = /* glsl */`...`;`）。而模板字符串
// 里出现任何一个反引号都会**提前终止**它——现象是整个 ES module 变成语法错误，页面里
// `window.PlanetXMap` 根本不存在，报错还是「Unexpected identifier 'xxx'」这种离现场很远的话。
// 这个坑在 sun.js / planet.js / util.js / postfx.js / sky.js 上一共踩了 6 次。
//
// `node --check` 只能报出**第一个**错误，所以这里用一个专门的扫描器一次报全：
// 从 `/* glsl */\`` 开始，把「后面（允许空白）跟着 ; , ) ] 或换行后的 }」的那个反引号当作
// 真正的结束符；两者之间出现的任何反引号都是 bug。
//
// 用法：node scripts/check-glsl-backticks.mjs <目录>
import fs from 'node:fs';
import path from 'node:path';

const root = process.argv[2] || 'web/static';
const problems = [];

function walk(dir) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) { if (e.name !== 'node_modules') walk(p); }
    else if (e.name.endsWith('.js')) scan(p);
  }
}

function scan(file) {
  const src = fs.readFileSync(file, 'utf8');
  const marker = /\/\*\s*glsl\s*\*\/\s*`/g;
  let m;
  while ((m = marker.exec(src))) {
    const bodyStart = m.index + m[0].length;
    // 找「后面跟着 ; , ) ] } 或行尾」的反引号作为结束（跳过那些只是普通字符的）。
    let end = -1;
    for (let i = bodyStart; i < src.length; i++) {
      if (src[i] !== '`') continue;
      const rest = src.slice(i + 1).match(/^\s*([;,)\]}]|$)/);
      if (rest) { end = i; break; }
    }
    if (end < 0) { problems.push(`${file}: GLSL 块没有结束反引号（从第 ${lineOf(src, bodyStart)} 行起）`); continue; }
    const body = src.slice(bodyStart, end);
    let idx = body.indexOf('`');
    while (idx >= 0) {
      const abs = bodyStart + idx;
      const ln = lineOf(src, abs);
      const lineText = src.split('\n')[ln - 1].trim();
      problems.push(`${file}:${ln}: GLSL 里出现反引号 → ${lineText.slice(0, 100)}`);
      idx = body.indexOf('`', idx + 1);
    }
    marker.lastIndex = end;
  }
}

function lineOf(src, index) {
  return src.slice(0, index).split('\n').length;
}

walk(root);
if (problems.length) {
  console.error('check-glsl-backticks: 发现 ' + problems.length + ' 处');
  for (const p of problems) console.error('  ' + p);
  process.exit(1);
}
console.log('check-glsl-backticks: OK');
