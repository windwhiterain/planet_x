// 追加机制（铁律 R 的读面落点）的**真跑**探针 —— 由 `play/tests/g4_spec.py` §8 调用。
//
// 为什么要有它：判据**不许自己抄一份追加口径**（抄的那份一定会和前端的漂开——本仓
// 「判据与实际解析路径不一致」已经吃过一次亏，见 g4 §5d 的注释）。所以这里把
// `web/static/specview.js` **原样加载**，用引擎发的真记录调它自己的三个出口：
//
//   * `expand(source)`      —— 前端自己把 source 展开成记录（不重写一遍解析器）
//   * `claimedKeys(spec)`   —— 前端自己算「这一行认领了哪些字段」
//   * `omittedKeys(spec)`   —— 前端自己算「声明不看的是哪些」
//   * `residualCols(rows, spec)` —— **要追加的列**（正是这一步改出来的新机制）
//   * `autoCol(k, v)` + `format(v, col, rec)` —— 追加列里那一格**到底印出什么**
//
// 用法：`node _append_probe.js <specview.js> <views.json>`，输入从 stdin 读
// `{"roots": {...}, "records": {viewId: [rec, ...]}}`（records 省略 = 从 roots 自己展开），
// 输出一行 JSON。
'use strict';

const fs = require('fs');
const path = require('path');

const [, , specviewPath, viewsPath] = process.argv;
const input = JSON.parse(fs.readFileSync(0, 'utf8'));

global.window = {};
require(path.resolve(specviewPath));
const sv = global.window.SpecView;
const doc = JSON.parse(fs.readFileSync(viewsPath, 'utf8'));
const roots = input.roots || {};

sv.bind({ getRoot: (name) => roots[name] });
sv.setSpecs(doc);

const isObj = (v) => v !== null && typeof v === 'object' && !Array.isArray(v);

function recordsOf(rows) {
  return rows.filter((r) => isObj(r.value));
}

function fieldsOf(recs) {
  const out = [];
  const seen = new Set();
  recs.forEach((r) => Object.keys(r.value).forEach((k) => {
    if (!seen.has(k)) { seen.add(k); out.push(k); }
  }));
  return out;
}

/// 追加列那一格**真的印出什么**：取第一行有这个键的记录，走前端自己的
/// `autoCol` + `format`（也就是实机那一条渲染路径）。
function sampleOf(recs, key) {
  for (const r of recs) {
    const v = r.value[key];
    if (v === undefined || v === null) continue;
    const col = sv.autoCol(key, v);
    const text = sv.format(v, col, r.value);
    let kind = 'other';
    let inner = [];
    if (Array.isArray(v)) {
      kind = 'array';
      inner = v.slice(0, 3).map((x) => (isObj(x) ? Object.keys(x).slice(0, 2).join(' ') : String(x)));
    } else if (isObj(v)) {
      kind = 'object';
      inner = Object.keys(v).slice(0, 3);
    } else if (typeof v === 'number') kind = 'number';
    else if (typeof v === 'boolean') kind = 'boolean';
    else kind = 'string';
    return { key, kind, fmt: col.fmt || 'text', text: text == null ? null : String(text), inner };
  }
  return null; // 这一列在所有行上都是空值 ⇒ 没有可印的东西
}

const out = [];
const views = [];
(doc.pages || []).forEach((p) => (p.views || []).forEach((v) => views.push(v)));
(doc.select || []).forEach((v) => views.push(v));

views.forEach((spec) => {
  const layout = spec.layout || 'table';
  if (!['table', 'sheet', 'cards'].includes(layout)) return;
  if (spec.source === null || spec.source === undefined) return;
  let rows;
  if (input.records && Object.prototype.hasOwnProperty.call(input.records, spec.id)) {
    rows = input.records[spec.id].map((value, i) => ({ key: String(i), value, path: spec.source + '[' + i + ']' }));
  } else {
    try {
      rows = sv.expand(spec.source);
    } catch (e) {
      out.push({ id: spec.id, error: String(e && e.message ? e.message : e) });
      return;
    }
  }
  const recs = recordsOf(rows);
  const claimed = Array.from(sv.claimedKeys(spec));
  const omitted = Array.from(sv.omittedKeys(spec));
  const auto = sv.residualCols(rows, spec);
  out.push({
    id: spec.id,
    layout,
    source: spec.source,
    rows: rows.length,
    records: recs.length,
    claimed,
    omitted,
    auto,
    fields: fieldsOf(recs),
    // 追加列的四种东西**都交给前端自己算**（顺序 = 引擎字段序；值 = 前端那条渲染路径）。
    autoViews: auto.map((k) => sampleOf(recs, k)),
  });
});

process.stdout.write(JSON.stringify({ views: out }));
