// 追加机制（铁律 R 的读面落点）的**真跑**探针 —— 由 `play/tests/g4_spec.py` §8/§10 调用。
//
// 为什么要有它：判据**不许自己抄一份追加口径**（抄的那份一定会和前端的漂开——本仓
// 「判据与实际解析路径不一致」已经吃过一次亏，见 g4 §5d 的注释）。所以这里把
// `web/static/specview.js` **原样加载**，用引擎发的真记录调它自己的出口：
//
//   * `expand(source)`      —— 前端自己把 source 展开成记录（不重写一遍解析器）
//   * `claimedKeys(spec)`   —— 前端自己算「这一行认领了哪些字段」
//   * `omittedKeys(spec)`   —— 前端自己算「声明不看的是哪些」
//   * `residualCols(rows, spec)` —— **要追加的列**（第 8 步的机制）
//   * `residualKeys(rec, spec)`  —— **要追加的行**（第 11 步的落点：详情层末尾）
//   * `autoCol(k, v)` + `format(v, col, rec)` —— 追加列里那一格**到底印出什么**
//   * `evalPath(col.path, …)` —— **声明的读列**在这一帧里取不取得到值（§8e：接手第 9 步
//     随「未组织」页删掉的那个运行时自检；口径见下面 `columnReport`）
//
// ⚠ 2026-10 第 11 步：读面改成**分层骨架**（外层总览 → 内层详情，`layout: layers`）。
// 于是这里多跑一件事：**用一个够用的假 DOM 把真渲染路径跑起来**（`layerReport`），
// 让判据看得见「外层到底列出了哪些行」「点进去详情层真的渲染了哪些行」——
// 而不是只看声明算出来的集合。假 DOM 是**测试的脚手架**，不是第二份渲染器：
// 它只实现 `createElement` / `textContent` / `appendChild` / `addEventListener` 这些
// 宿主 API，排版与逻辑一个字节都不重写（真文件里加一个 DOM API 就会在这里炸出来）。
//
// 用法：`node _append_probe.js <specview.js> <views.json>`，输入从 stdin 读
// `{"roots": {...}, "records": {viewId: [rec, ...]}}`（records 省略 = 从 roots 自己展开），
// 输出一行 JSON。
'use strict';

const fs = require('fs');
const path = require('path');

const [, , specviewPath, viewsPath] = process.argv;
const input = JSON.parse(fs.readFileSync(0, 'utf8'));

// --- 一个**够用的假 DOM** ----------------------------------------------------
// 只实现 `specview.js` 真的用到的那几个宿主 API。`textContent` 的读法是**拼出全部后代的文本**
// ——判据要的就是"这一行在屏幕上长什么样"。
class FakeNode {
  constructor(tag, text) {
    this.tag = String(tag == null ? '' : tag).toLowerCase();
    this.isText = text !== undefined;
    // `textContent = 'x'` 在真 DOM 里是**换成一个文本子节点**（不是存在节点自己身上）——
    // 这一条必须照做：`insertBefore(色点, name.firstChild)` 之后名字还得在（第 11 步
    // 总览行的「色点 + 名字」就是这么摆的）。
    this._text = this.isText ? String(text) : '';
    this.children = this.isText || text === undefined ? [] : [new FakeNode('#text', text)];
    this.parentNode = null;
    this.style = {};
    this.attrs = {};
    this._listeners = {};
    this._classes = new Set();
    this.tabIndex = 0;
    this.title = '';
  }
  get className() { return Array.from(this._classes).join(' '); }
  set className(v) { this._classes = new Set(String(v).split(/\s+/).filter(Boolean)); }
  get classList() {
    const self = this;
    return {
      add: (c) => { self._classes.add(String(c)); },
      remove: (c) => { self._classes.delete(String(c)); },
      contains: (c) => self._classes.has(String(c)),
    };
  }
  get firstChild() { return this.children[0] || null; }
  get nextSibling() {
    if (!this.parentNode) return null;
    const i = this.parentNode.children.indexOf(this);
    return i < 0 ? null : (this.parentNode.children[i + 1] || null);
  }
  get childElementCount() { return this.children.length; }
  get textContent() {
    if (this.isText) return this._text;
    return this.children.map((c) => c.textContent).join('');
  }
  set textContent(v) {
    if (this.isText) { this._text = String(v); return; }
    this._text = '';
    this.children = v === '' || v === null || v === undefined ? [] : [new FakeNode('#text', v)];
  }
  appendChild(c) { c.parentNode = this; this.children.push(c); return c; }
  append(...cs) { cs.forEach((c) => this.appendChild(c)); }
  insertBefore(c, ref) {
    c.parentNode = this;
    const i = ref ? this.children.indexOf(ref) : -1;
    if (i < 0) this.children.push(c);
    else this.children.splice(i, 0, c);
    return c;
  }
  setAttribute(k, v) { this.attrs[k] = String(v); }
  getAttribute(k) { return this.attrs[k] === undefined ? null : this.attrs[k]; }
  addEventListener(t, fn) { (this._listeners[t] = this._listeners[t] || []).push(fn); }
  dispatch(t, ev) {
    const e = ev || { type: t, target: this, key: undefined, preventDefault() {} };
    (this._listeners[t] || []).forEach((fn) => fn(e));
  }
  click() { this.dispatch('click', { type: 'click', target: this, preventDefault() {} }); }
  key(k) { this.dispatch('keydown', { type: 'keydown', target: this, key: k, preventDefault() {} }); }
}

global.document = {
  createElement: (t) => new FakeNode(t),
  createTextNode: (t) => new FakeNode('#text', t),
};

global.window = {};
require(path.resolve(specviewPath));
const sv = global.window.SpecView;
const doc = JSON.parse(fs.readFileSync(viewsPath, 'utf8'));
const roots = input.roots || {};

// 判据自己的**合成根**（不进真世界的账）：只用来验「空来源」那一类边界。
// 第 11 步实测过的真 bug：`@post.haul_steps` 开局是 `{}`，而 `wrap()` 把空对象当成
// 「一条空记录」⇒ 外层总览凭空长出一行名字是 `·` 的**幽灵项目**（还点得进去）。
const PROBE_ROOTS = { empty_map: {}, empty_list: [], not_empty_map: { 甲: { 名字: '甲' } } };

sv.bind({ getRoot: (name) => (name === '__probe' ? PROBE_ROOTS : roots[name]) });
sv.setSpecs(doc);

const isObj = (v) => v !== null && typeof v === 'object' && !Array.isArray(v);

// --- 假 DOM 上的一点点查询工具（判据自己走的路径，不碰渲染逻辑）----------------
function walk(node, fn) {
  if (node.isText) return;
  fn(node);
  node.children.forEach((c) => walk(c, fn));
}
function findAll(root, cls) {
  const out = [];
  walk(root, (n) => { if (n._classes.has(cls)) out.push(n); });
  return out;
}
function findFirst(root, cls) { return findAll(root, cls)[0] || null; }
function textOf(node) { return node ? node.textContent : null; }

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

/// 追加**列**那一格**真的印出什么**：取第一行有这个键的记录，走前端自己的
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

/// **声明过的读列在这一帧里到底取不取得到值**（第 10 步，2026-10）。
///
/// 口径**照抄第 9 步随「未组织」页删掉的运行时自检 `renderSpecCheck`**（不另立一套）：
///   * 只看**读列**（`leaf` / `owner` / `action` 三类写行不算——它们的 `null` 是「这一层
///     还没有叶」，报成"取不到值"是反方向的谎话）；
///   * 「取到值」= 这一列在整表里至少有一格**非空**：`null`/`undefined` 不算，空数组 /
///     空串 / 空对象也不算（那正是"有列没数据"的样子）；
///   * 求值走 `specview.js` 自己的 `evalPath`（判据不抄第二份求值器）。
function columnReport(spec, rows) {
  const readCols = (spec.columns || []).filter(
    (c) => c && c.leaf == null && c.owner == null && c.action == null && typeof c.path === 'string');
  return readCols.map((c) => {
    let nonNil = 0;
    let nonEmpty = 0;
    let sample = null;
    rows.forEach((r) => {
      const rkey = spec.key ? sv.evalPath(spec.key, r.value, r.key) : r.key;
      let v;
      try {
        v = sv.evalPath(c.path, r.value, rkey);
      } catch (e) {
        v = undefined;
      }
      if (v === undefined || v === null) return;
      nonNil += 1;
      const empty = (Array.isArray(v) && !v.length)
        || (typeof v === 'string' && !v)
        || (isObj(v) && !Object.keys(v).length);
      if (empty) return;
      nonEmpty += 1;
      if (sample === null) {
        const text = sv.format(v, c, r.value);
        sample = text == null ? null : String(text);
      }
    });
    return { path: c.path, label: c.label || null, rows: rows.length, nonNil, nonEmpty, sample };
  });
}

// --- 分层：外层总览 → 内层详情（第 11 步）------------------------------------
//
// 把 `layout: layers` 的视图**真渲染一遍**（假 DOM），逐行走「点进去 → 读详情 → 返回」，
// 报告屏幕上真有的东西：外层每行的文本 / 关键指标、点进去有没有详情、面包屑与返回按钮在不在、
// 详情层渲染出来的行（声明读行 / 控制行 / **追加行**）、返回之后外层行数是否恢复。
//
// `autoKeys`（追加行的字段名 = 真 DOM 里 `.sv-sheet-row-auto` 的键）与
// `expectedAuto`（前端自己的 `residualKeys`）两边一起报 ⇒ 判据能直接验「残差真的摆在了详情层」。
function layerReport(spec) {
  const box = document.createElement('div');
  sv.layerState.clear();     // 每条视图从**未打开**的状态开始（判据之间不互相污染）
  sv.renderView(box, spec, { instance: 'probe:' + spec.id, crumb: '（总览）' });
  const outerRows = findAll(box, 'sv-lrow');
  const recs = (() => {
    try {
      return sv.expand(spec.source).filter((r) => isObj(r.value));
    } catch (e) {
      return [];
    }
  })();
  // 按**路径**配对（唯一）：同名记录（`offers` 里同一个卖家的好几种货）只有路径分得开。
  const byPath = new Map(recs.map((r) => [String(r.path), r]));
  const notes = findAll(box, 'sv-more-wrap').map((n) => textOf(n)).filter(Boolean);
  const rows = outerRows.map((row) => {
    const before = findAll(box, 'sv-lrow').length;
    const recKey = row.getAttribute('data-rec');
    const recPath = row.getAttribute('data-path');
    const label = textOf(findFirst(row, 'sv-lkey'));
    const briefs = findAll(row, 'sv-brief').map((b) => textOf(b));
    row.click();
    const det = findFirst(box, 'sv-detail');
    const shown = !!(det && det.style.display !== 'none' && det.children.length);
    const cur = shown ? textOf(findFirst(det, 'sv-crumb-cur')) : null;
    const backBtn = shown ? findFirst(det, 'sv-back') : null;
    const sheetRows = shown ? findAll(det, 'sv-sheet-row') : [];
    const keyOfRow = (r) => textOf(findFirst(r, 'sv-sheet-k'));
    const autoKeys = shown
      ? findAll(det, 'sv-sheet-row-auto').map(keyOfRow)
      : [];
    const declaredKeys = shown
      ? sheetRows.filter((r) => !r._classes.has('sv-sheet-row-auto') && !r._classes.has('sv-sheet-row-ctl')).map(keyOfRow)
      : [];
    const ctlRows = shown
      ? sheetRows.filter((r) => r._classes.has('sv-sheet-row-ctl')).length
      : 0;
    const crumbs = shown ? textOf(findFirst(det, 'sv-crumbs')) : null;
    let backRestored = null;
    let rowCountAfterBack = null;
    if (backBtn) {
      backBtn.click();
      rowCountAfterBack = findAll(box, 'sv-lrow').length;
      backRestored = rowCountAfterBack === before && det.style.display === 'none';
    }
    const rec = byPath.get(String(recPath));
    return {
      key: recKey, path: recPath, label, briefs, detail: shown, crumb: cur, crumbs, back: !!backBtn,
      backRestored, rowCountAfterBack,
      declaredKeys, autoKeys, ctlRows,
      ctlDeclared: (spec.columns || []).filter((c) => sv.isControlRow(c)).length,
      expectedAuto: rec ? sv.residualKeys(rec.value, spec) : null,
      fields: rec ? Object.keys(rec.value) : null,
      claimed: Array.from(sv.claimedKeys(spec)),
      omitted: Array.from(sv.omittedKeys(spec)),
    };
  });
  return {
    id: spec.id,
    layout: spec.layout,
    source: spec.source === undefined ? null : spec.source,
    outer: outerRows.length,
    brief: spec.brief || null,
    single: spec.single || null,
    records: recs.length,
    limit: spec.limit || null,
    notes,
    rows,
  };
}

const out = [];
const nullSource = [];
const views = [];
(doc.pages || []).forEach((p) => (p.views || []).forEach((v) => views.push(v)));
(doc.select || []).forEach((v) => views.push(v));

views.forEach((spec) => {
  const layout = spec.layout || 'table';
  if (!['table', 'sheet', 'cards', 'layers'].includes(layout)) return;
  // `source: null` = **不取任何记录**（只有写行的 sheet，如「全局」页那条作用域账）。
  // 它**不进** `out`：§8a 的「声明 ∪ 追加 == 全部字段」是拿真记录算的，塞一条空记录进去
  // 会让它凭空虚报（声明了记录上没有的字段）。但 §8e 仍要查它的读列**解析得到东西**——
  // 否则 `@scope.factions` 写错一个词，那一列会静静地永远印「·」，没有任何读者会报。
  // 所以它单独走一条出口（口径更松：**解析得到**即可，`[]`/`{}` 也算，见 §8e 的说明）。
  if (spec.source === null || spec.source === undefined) {
    nullSource.push({
      id: spec.id,
      layout,
      cols: columnReport(spec, [{ key: null, value: {} }]),
    });
    return;
  }
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
    // **声明读列的非空盘点**（§8e 用它接手第 9 步删掉的那个运行时自检）。
    cols: columnReport(spec, rows),
  });
});

// 分层视图的**真渲染**报告（第 11 步）：只对 `layout: layers` 跑。
const layered = [];
views.forEach((spec) => {
  if ((spec.layout || '') !== 'layers') return;
  try {
    layered.push(layerReport(spec));
  } catch (e) {
    layered.push({ id: spec.id, error: String((e && e.stack) || e) });
  }
});

// **空来源**的边界（合成根，见 `PROBE_ROOTS`）：`{}` / `[]` ⇒ **一行都不许长出来**
// （要显示声明的 `empty` 文案）。这一条正是实测幽灵行的反向守卫。
const emptySources = ['empty_map', 'empty_list', 'not_empty_map'].map((k) => {
  const spec = {
    id: 'probe-' + k, title: '空来源 · ' + k, mount: 'panel', layout: 'layers',
    source: '@__probe.' + k, key: '名字', brief: ['名字'],
    columns: [{ path: '名字' }], empty: '（这一帧没有数据）',
  };
  const box = document.createElement('div');
  sv.layerState.clear();
  sv.renderView(box, spec, { instance: 'probe:' + k, crumb: '（总览）' });
  return {
    kind: k,
    outer: findAll(box, 'sv-lrow').length,
    text: textOf(box).trim().slice(0, 80),
  };
});

process.stdout.write(JSON.stringify({ views: out, nullSource, layered, emptySources }));
