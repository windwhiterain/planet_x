// 组织点求值器 —— **只认 spec 的形状，不认领域字段名**。
//
// 与 jsonview.js 守同一条铁律：本文件里不出现 bodies/cities/ships/factions/control 这类
// 领域词汇，也不假设任何字段的类型或含义。它认的是这些**通用**东西：
//
//   路径表达式   `船体` / `@post.factions.${势力}.upkeep` / `指令[?舰=${name}]`
//                / `cargo` / `@key`（映射表的键）/ `${字段}`（模板，取自当前记录）
//   格式化器     num / pct / ratio / enum / map / sum / top / tagged / …（一张通用表）
//   布局         table / sheet / cards / timeline / pairs
//   组织         group（分组）、order（行序）、limit（上限，**必须明说藏了多少**）
//   控制行       `leaf` / `owner` / `action` —— **本文件不解释它们**，只把节点要过来
//                （见 isControlRow / ctx.controlNode）：写面住在 `web/static/controls.js`，
//                与「本文件不认识 bodies/cities」是同一条纪律。四种行住在同一个 `columns`
//                数组里 ⇒ 行序就是「读与控制穿插」的顺序。
//
// 于是「新增一个组织点 = 写一段 JSON」，前端一行代码都不用改。
//
// ⚠ **铁律 R（只能整理，不能隐藏）**：
//   展示 = 认领（整理形态）⊎ 残差（通用形态）
//   残差的字段集合 = 该记录实际有的顶层字段 ∖ 这一行所有列**相对路径**的首段
//                    ∖ 显式 omit
//   而且残差是**渲染时用集合差算出来的**（[`residualOf`]），不是声明的 ⇒ 引擎给记录加了
//   新字段，下一帧它就出现在「其余字段」里，组织点不用改。唯一能藏数据的方式是显式 omit，
//   而界面会把省略**明说**出来（[`omitLine`]）。
'use strict';

(function () {
  // --- 小工具 ---------------------------------------------------------------
  const isArr = (v) => Array.isArray(v);
  const isObj = (v) => v !== null && typeof v === 'object' && !isArr(v);
  const isNil = (v) => v === null || v === undefined;

  function el(tag, cls, text) {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text != null) e.textContent = text;
    return e;
  }

  function num(v, digits) {
    const n = +v;
    if (!Number.isFinite(n)) return String(v);
    const d = digits == null ? 2 : digits;
    const s = n.toFixed(d);
    // 去掉 0 尾巴（1.00 → 1，1.20 → 1.2）——但对齐靠右的数字列，整数仍保留小数点前的位数。
    return d > 0 ? s.replace(/\.?0+$/, '') : s;
  }

  // --- 上下文：根、模板引用、格式化器 ---------------------------------------
  // `roots(name)` 返回一个根（state/pre/post/config/session/control/scope）。
  // `maps` 是 views.json 顶层的**具名表**（如 tagged_behavior），供列用 map_ref 引用。
  const ctx = {
    getRoot: () => undefined,
    maps: {},
    onPathClick: null,   // 点值复制 JSON 路径
    onSelect: null,      // 点一行选中实体（底栏读面）
    selection: null,     // {kind, name}
    expanded: new Set(), // 上限展开 / 残差展开的状态（viewId#row 之类）
    controlNode: null,   // 控制行（`leaf`/`owner`/`action`）交给宿主渲染——见 isControlRow
    tip: null,           // 悬停弹窗：把"这条列声明"交给宿主，由它查名词的解释（见 web/static/tip.js）
  };

  function bind(opts) {
    const o = opts || {};
    if (o.getRoot) ctx.getRoot = o.getRoot;
    // maps 是 views.json 顶层的**具名表**，由 setSpecs 填；这里**合并**而不是覆盖
    // （覆盖过一次的后果：tagged 变体全退回原始 JSON——`Haul {"from":…}` 直接糊在表格里）。
    if (o.maps) Object.assign(ctx.maps, o.maps);
    if (o.onPathClick !== undefined) ctx.onPathClick = o.onPathClick;
    if (o.onSelect !== undefined) ctx.onSelect = o.onSelect;
    if (o.selection !== undefined) ctx.selection = o.selection;
    if (o.expanded) ctx.expanded = o.expanded;
    // 写面钩子（`web/static/controls.js`）：`(col, rec, recKey, {where}) → 一个 DOM 节点`。
    if (o.controlNode !== undefined) ctx.controlNode = o.controlNode;
    // 悬停弹窗（`tip`）：宿主实现「这个名词是什么、查不到时用哪个字段名兜底」。
    if (o.tip !== undefined) ctx.tip = o.tip;
  }

  // --- 路径表达式 ------------------------------------------------------------
  // 文法（全部可空：任何一步取不到 ⇒ 整个表达式 = undefined ⇒ 该格显示 missing，
  // 而**列不会消失**——「这局没有」必须与「没人摆它」长得不一样）：
  //
  //   expr     := seg ('.' seg)*
  //   seg      := name | name '[' pick ']' | name '[' '*' ']' | '@key'
  //   pick     := index | '?' field '=' value        （value 可用 ${…} 模板）
  //   name/value 里可嵌 ${字段} 模板（相对当前记录求值；${@key} = 映射键）
  //
  // 首段以 '@' 开头 = **绝对路径**（@state/@post/@config/@pre/@session/@control/@scope），
  // 否则相对当前记录。绝对路径让「跨根 join」成为同一套语法里的普通写法：
  //   @post.factions.${name}.upkeep          —— 势力行 ⋈ 本回合的过程量
  //   @control[?势力=${势力}].指令[?舰=${name}].行为
  //                                          —— 舰 ⋈ 它的**有效**指令
  function splitSegments(expr) {
    const out = [];
    let buf = '';
    let depth = 0;
    for (const ch of String(expr)) {
      if (ch === '[') depth++;
      if (ch === ']') depth--;
      if (ch === '.' && depth === 0) {
        out.push(buf);
        buf = '';
      } else buf += ch;
    }
    out.push(buf);
    return out.filter((s) => s !== '');
  }

  function parseSeg(seg) {
    const m = /^([^\[\]]*)(?:\[(.*)\])?$/.exec(seg);
    const rawName = m[1];
    const inside = m[2];
    const s = { name: rawName };
    if (inside === '*') s.spread = true;
    else if (inside != null) {
      if (inside.startsWith('?')) {
        const eq = inside.indexOf('=');
        s.pickField = inside.slice(1, eq);
        s.pickValue = inside.slice(eq + 1);
      } else s.index = +inside;
    }
    return s;
  }

  // ${字段} 模板：对当前记录求值（也支持 ${@key}）。找不到就原样留着——不猜。
  function tmpl(str, rec, recKey) {
    if (typeof str !== 'string' || str.indexOf('${') < 0) return str;
    return str.replace(/\$\{([^}]*)\}/g, (all, expr) => {
      const v = evalPath(expr, rec, recKey);
      return isNil(v) ? all : String(v);
    });
  }

  // 一段 = **两步**：① 取名字（空名 = 当前值，`@key` = 这一条在映射表/数组里的键）
  //              ② 施加方括号（下标 / `[*]` 展开 / `[?f=v]` 挑一条）
  // 顺序不能反：`chronicle[*]` 必须先取到 `chronicle` 再展开，`指令[?舰=x]` 必须先
  // 取到那个数组再挑——忘了第 ① 步就会在**父对象**上展开/挑选（本轮真踩过：`chron 0..17`
  // 的键就是这么来的），症状是整页"值全是 ·"。
  function step(cur, parts, i, rec, recKey) {
    if (i >= parts.length) return cur;
    const p = parts[i];
    if (isNil(cur)) return undefined;

    let base;
    if (p.name === '@key') base = recKey;
    else if (!p.name) base = cur; // 恒等段：`@根` 与纯 `[*]` / `[?…]`
    else {
      const nm = tmpl(p.name, rec, recKey);
      if (isArr(cur)) base = /^\d+$/.test(String(nm)) ? cur[+nm] : undefined;
      else if (isObj(cur)) base = cur[nm];
      else return undefined;
    }

    if (p.spread) {
      const items = isArr(base) ? base : isObj(base) ? Object.values(base) : [];
      const got = items.map((v) => step(v, parts, i + 1, rec, recKey)).filter((v) => v !== undefined);
      return got.length ? got : undefined;
    }
    if (p.index != null) {
      return step(isArr(base) ? base[p.index] : undefined, parts, i + 1, rec, recKey);
    }
    if (p.pickField !== undefined) {
      const f = tmpl(p.pickField, rec, recKey);
      const want = tmpl(p.pickValue, rec, recKey);
      let hit;
      if (isArr(base)) hit = base.find((it) => isObj(it) && String(it[f]) === String(want));
      else if (isObj(base)) {
        hit = f === '@key' ? base[want] : Object.values(base).find((it) => isObj(it) && String(it[f]) === String(want));
      }
      return step(hit, parts, i + 1, rec, recKey);
    }
    return step(base, parts, i + 1, rec, recKey);
  }

  // 求值一条路径表达式。`rec` = 当前记录，`recKey` = 它在映射表/数组里的键（@key）。
  /// 一条记录**对外报出的身份**：视图声明了 `key`（`城名` / `舰名` / `势力` / `天体名`…）就用它
  /// 求值，否则回落原始键（数组下标 / 映射键）。写面拿它当作用域 id（`edScope` 里的城/天体键），
  /// 所以「这条记录叫什么」只有**一份事实：视图的声明**——前端不再自己猜 `rec.name`。
  function recordKeyOf(spec, r) {
    return spec && spec.key ? evalPath(spec.key, r.value, r.key) : r.key;
  }

  function evalPath(expr, rec, recKey) {
    const parts = splitSegments(expr).map(parseSeg);
    if (!parts.length) return rec;
    if (parts[0].name && parts[0].name.charAt(0) === '@' && parts[0].name !== '@key') {
      const root = ctx.getRoot(parts[0].name.slice(1));
      const head = Object.assign({}, parts[0], { name: '' });
      return step(root, [head].concat(parts.slice(1)), 0, rec, recKey);
    }
    return step(rec, parts, 0, rec, recKey);
  }

  // --- 源：一个视图的数据从哪来 ---------------------------------------------
  // 展开成 [{key, value, path}]：
  //   数组        → 逐项（key = 下标）
  //   全对象的值   → 一张**映射表**（key = 映射键，记录里用 @key 取）
  //   其余对象     → 一条记录（sheet / timeline 用）
  function expand(source) {
    const parts = splitSegments(source).map(parseSeg);
    if (!parts.length) return [];
    let cur;
    if (parts[0].name && parts[0].name.charAt(0) === '@' && parts[0].name !== '@key') {
      cur = ctx.getRoot(parts[0].name.slice(1));
      const head = Object.assign({}, parts[0], { name: '' });
      cur = step(cur, [head].concat(parts.slice(1)), 0, null, null);
    } else {
      // 相对源（少见）：以 state 根为准，保证 `ships[*]` 这类写法也说得通。
      cur = step(ctx.getRoot('state'), parts, 0, null, null);
    }
    return wrap(cur, source);
  }

  function wrap(v, source) {
    if (isNil(v)) return [];
    if (isArr(v)) return v.map((value, i) => ({ key: String(i), value, path: source + '[' + i + ']' }));
    if (isObj(v)) {
      const vals = Object.values(v);
      // 「值全是对象」= 映射表（一条记录映射）。混合的（如 RoundView）当**一条记录**。
      if (vals.length && vals.every(isObj)) {
        return Object.keys(v).map((k) => ({ key: k, value: v[k], path: source + '.' + k }));
      }
      return [{ key: null, value: v, path: source }];
    }
    return [{ key: null, value: v, path: source }];
  }

  // 标量映射（资源 → 数）：pairs 布局要的是「一项一行」，而不是"一条记录"。
  function expandScalarEntries(source) {
    const parts = splitSegments(source).map(parseSeg);
    if (!parts.length) return [];
    let cur;
    if (parts[0].name && parts[0].name.charAt(0) === '@' && parts[0].name !== '@key') {
      cur = ctx.getRoot(parts[0].name.slice(1));
      const head = Object.assign({}, parts[0], { name: '' });
      cur = step(cur, [head].concat(parts.slice(1)), 0, null, null);
    } else cur = step(ctx.getRoot('state'), parts, 0, null, null);
    if (isNil(cur)) return [];
    if (isArr(cur)) return cur.map((value, i) => ({ k: String(i), v: value }));
    if (isObj(cur)) return Object.keys(cur).map((k) => ({ k, v: cur[k] }));
    return [{ k: source, v: cur }];
  }

  // --- 残差（铁律 R 的落点） ------------------------------------------------
  // 认领 = 该视图所有列里**相对路径**的首段；残差 = 记录实际字段 ∖ 认领 ∖ omit。
  // 绝对路径（@…）认领的不是这条记录的字段，所以不参与。
  function claimedKeys(spec) {
    const out = new Set();
    (spec.columns || []).forEach((c) => {
      // 控制行里 `action` 认领的**就是**记录上的那个字段（`buildings` 是城记录的一个字段，
      // 它由「建筑与建造区」那一行渲染 ⇒ 不该再落进「其余字段」里当没被认领）。
      // `leaf`/`owner` 是绝对路径（@control/@scope），不认领本条记录的字段。
      if (c.action) out.add(String(c.action).replace(/\[.*$/, ''));
      const p = String(c.path || '');
      if (!p || p.charAt(0) === '@') return;
      const first = splitSegments(p)[0] || '';
      out.add(first.replace(/\[.*$/, ''));
    });
    // 分组键、行标签、timeline 的三处路径也算认领（它们在界面上出现过）。
    [spec.group && spec.group.by, spec.title_path, spec.body]
      .filter(Boolean)
      .forEach((p) => {
        if (String(p).charAt(0) === '@') return;
        out.add(String(p).replace(/\[.*$/, ''));
      });
    (spec.meta || []).forEach((p) => {
      if (String(p).charAt(0) === '@') return;
      out.add(String(p).replace(/\[.*$/, ''));
    });
    return out;
  }

  function omittedKeys(spec) {
    return new Set((spec.omit || []).map((o) => String(o.path).replace(/\[.*$/, '')));
  }

  function residualOf(rec, spec) {
    if (!isObj(rec)) return null;
    const claimed = claimedKeys(spec);
    const omit = omittedKeys(spec);
    const out = {};
    Object.keys(rec).forEach((k) => {
      if (claimed.has(k) || omit.has(k)) return;
      out[k] = rec[k];
    });
    return Object.keys(out).length ? out : null;
  }

  // --- 格式化器（通用表；一个领域词都没有） ---------------------------------
  function enumLabel(v, col) {
    if (isNil(v)) return null;
    if (col.label_from) {
      const tbl = ctx.getRoot(col.label_from.split('.')[0]);
      const rest = splitSegments(col.label_from).slice(1);
      let node = tbl;
      rest.forEach((k) => { node = node && node[k]; });
      const hit = node && node[String(v)];
      if (hit && typeof hit === 'object') return hit.label || hit.name || String(v);
    }
    if (col.map && Object.prototype.hasOwnProperty.call(col.map, String(v))) return col.map[String(v)];
    // 没声明的取值**如实显示**（并在标题里点出「没有中文标签」）——不假装它是别的什么。
    return col.unmapped ? col.unmapped + ' ' + String(v) : String(v);
  }

  function mapString(v, col) {
    if (isNil(v)) return null;
    if (isArr(v)) return v.map((x) => stringify(x, col)).join(' · ');
    if (!isObj(v)) return stringify(v, col);
    const keys = Object.keys(v);
    if (!keys.length) return null;
    return keys.map((k) => k + ' ' + stringify(v[k], col)).join(' · ');
  }

  function sumOf(v, field, col) {
    if (isNil(v)) return null;
    if (!isObj(v)) return null;
    let s = 0;
    let any = false;
    Object.values(v).forEach((x) => {
      const t = field && isObj(x) ? x[field] : x;
      if (typeof t === 'number') { s += t; any = true; }
    });
    return any ? num(s, col.digits == null ? 2 : col.digits) : null;
  }

  function topString(v, col) {
    if (!isObj(v)) return null;
    const rows = Object.keys(v).map((k) => [k, +v[k]]).filter((r) => Number.isFinite(r[1]));
    rows.sort((a, b) => b[1] - a[1]);
    const n = col.n == null ? 4 : col.n;
    const head = rows.slice(0, n).map((r) => r[0] + ' ' + num(r[1] * 100, col.digits == null ? 0 : col.digits) + '%');
    const restN = rows.length - head.length;
    return { text: head.join(' · '), more: restN > 0 ? '还有 ' + restN + ' 家未显示' : null };
  }

  function pairsString(v, col) {
    if (!isArr(v) || !v.length) return null; // 空数组 = 「没有」，不是空字符串（否则那格看起来像没渲染）
    const sep = col.sep || '↔';
    return v.map((p) => (isArr(p) ? p.map((x) => stringify(x, col)).join(sep) : stringify(p, col))).join(' · ');
  }

  function progressString(v, col) {
    if (isNil(v)) return null;
    if (!isObj(v)) return stringify(v, col);
    const keys = Object.keys(v);
    if (!keys.length) return null;
    return keys
      .map((k) => {
        const it = v[k];
        if (!isObj(it)) return k + ' ' + stringify(it, col);
        const done = it.increment;
        const rate = it.rate;
        return k + ' ' + num(done, 1) + (rate != null ? '/' + num(rate, 1) : '');
      })
      .join(' · ');
  }

  function taggedString(v, col) {
    // tagged enum（serde 的单键对象）：{Haul:{from,to}} ⇒ 用声明的模板渲染。
    const table = col.map_ref ? ctx.maps[col.map_ref] : null;
    const map = (table && table.map) || col.map || {};
    const empty = (table && table.empty) || (col.map && col.map['']) || null;
    if (isNil(v)) return null;
    if (typeof v === 'string') return map[v] != null ? map[v] : v;
    if (!isObj(v)) return stringify(v, col);
    const keys = Object.keys(v);
    if (!keys.length) return empty;
    const tag = keys[0];
    const tpl = map[tag];
    if (tpl == null) {
      // 没声明的变体：**如实显示**（标签 + 载荷），不退回"待命"这种假话。
      return tag + ' ' + JSON.stringify(v[tag]);
    }
    return tpl.replace(/\{([^}]*)\}/g, (all, f) => {
      const got = v[tag] && v[tag][f];
      return isNil(got) ? all : stringify(got, col);
    });
  }

  function fieldsString(v, col) {
    if (!isObj(v)) return null;
    return (col.fields || []).map(([p, label]) => label + ' ' + stringify(v[p], col)).join(' · ');
  }

  function rowsString(v, col) {
    if (!isArr(v)) return null;
    return v
      .map((it) => {
        if (!isObj(it)) return stringify(it, col);
        const head = it.name || it.id || '';
        const body = (col.fields || []).map((f) => f + ' ' + stringify(it[f], col)).join(' ');
        return (head ? head + '：' : '') + body;
      })
      .join(' ｜ ');
  }

  // 三态归属的人话版。⚠ 措辞**按轴**看才对，所以这里给的是"通用且不骗人"的那一句：
  // 指令链上（2026-10 起）没有更高的一层——舰队默认指令与图上 `order` 两片叶都已删——
  // 所以指令叶写 `Inherit` 时**值就是叶里那个值**，没有"上层"可继承。
  // 倾向三轴仍然有上层（叶 → 出厂图 → 舰队默认 → 记录值），所以那一列自己覆盖措辞
  // （见 `views.json` 里对应列的 `map`）。
  const OWNER = { Player: '玩家（系统只读）', Auto: '自动（系统每回合可改写）', Inherit: '叶没表态（用叶里的值）' };

  function stringify(v, col) {
    if (isNil(v)) return '·';
    if (typeof v === 'number') return num(v, col && col.digits != null ? col.digits : 2);
    if (typeof v === 'boolean') return v ? '是' : '否';
    if (isArr(v)) return v.map((x) => stringify(x, col)).join(', ');
    if (isObj(v)) return Object.keys(v).map((k) => k + ' ' + stringify(v[k], col)).join(' ');
    return String(v);
  }

  function format(v, col, rec) {
    const f = col.fmt || 'text';
    switch (f) {
      case 'int': return isNil(v) ? null : num(Math.round(+v), 0);
      case 'num': return isNil(v) ? null : num(v, col.digits == null ? 2 : col.digits);
      case 'pct': return isNil(v) ? null : num(+v * 100, col.digits == null ? 0 : col.digits) + '%';
      case 'vec': return isArr(v) ? v.map((x) => num(x, col.digits == null ? 1 : col.digits)).join(', ') : null;
      case 'bool': {
        if (isNil(v)) return null;
        const k = String(v);
        return col.map && Object.prototype.hasOwnProperty.call(col.map, k) ? col.map[k] : v ? '是' : '否';
      }
      case 'enum': return enumLabel(v, col);
      case 'count': return isNil(v) ? null : isArr(v) ? String(v.length) : isObj(v) ? String(Object.keys(v).length) : '1';
      case 'list': return isNil(v) ? null : isArr(v) ? v.map((x) => stringify(x, col)).join(', ') : stringify(v, col);
      case 'map': return mapString(v, col);
      case 'sum': return sumOf(v, col.of, col);
      case 'top': return topString(v, col);
      case 'pairs': return pairsString(v, col);
      case 'progress': return progressString(v, col);
      case 'tagged': return taggedString(v, col);
      case 'fields': return fieldsString(v, col);
      case 'rows': return rowsString(v, col);
      case 'owner': return isNil(v) ? null : OWNER[String(v)] || String(v);
      case 'ratio': {
        if (isNil(v)) return null;
        const max = col.with ? evalPath(col.with, rec) : null;
        registerRatio(v, max);
        return num(v, col.digits == null ? 0 : col.digits) + (isNil(max) ? '' : ' / ' + num(max, col.digits == null ? 0 : col.digits));
      }
      default: return stringify(v, col);
    }
  }

  // --- 单元格 / 行 ----------------------------------------------------------
  // ratio 需要「值 + 上限」两处一起算，用一个小槽把它们传给渲染器画比例条。
  let ratioSlot = null;
  function registerRatio(v, max) { ratioSlot = { v: +v, max: max == null ? null : +max }; }

  function barNode(slot) {
    const box = el('span', 'sv-bar');
    const fill = el('span', 'sv-bar-fill');
    const pct = isNil(slot.max) || !slot.max ? (slot.v > 0 ? 100 : 0) : Math.max(0, Math.min(100, (slot.v / slot.max) * 100));
    fill.style.width = pct + '%';
    if (pct < 34) fill.classList.add('low');
    box.appendChild(fill);
    return box;
  }

  // 一格的显示节点。三条规则（**「没有值」和「格式化器说没什么可显示」都要说人话**）：
  //   text === ''  → 空格子（作者显式映射成空，例如 bool 的 false）
  //   text == null → 声明的 missing 文案（如 `空` / `—`），**不是** String(null) 印出个 "null"
  //   否则         → 值
  function valueNode(col, v, text) {
    if (isNil(text) && !isNil(v) && typeof text === 'object' && text && text.text != null) return null;
    if (text === '') return el('span', 'sv-val', '');
    if (isNil(text)) return el('span', 'sv-missing', col.missing != null ? col.missing : '·');
    if (typeof text === 'object' && text && text.text != null) {
      const holder = el('span', 'sv-val');
      holder.appendChild(document.createTextNode(text.text));
      if (text.more) holder.appendChild(el('span', 'sv-more-inline', '（' + text.more + '）'));
      return holder;
    }
    return el('span', 'sv-val', String(text));
  }

  // --- 控制行：本文件**不解释**它们 ------------------------------------------
  // 一行除了 `path`（读）之外，还可以是 `leaf`（一片控制叶）/ `owner`（作用域归属）/
  // `action`（命令列表）——那是**写面**的三种行。它们怎么渲染、写回什么、归属怎么写，
  // 全在宿主的钩子里（`web/static/controls.js`）：本文件只认「这一行带没带这三个键」，
  // 然后把节点要过来 append 进去。**这与它不认识 bodies/cities 是同一条纪律**。
  const CONTROL_KEYS = ['leaf', 'owner', 'action'];

  function isControlRow(col) {
    return !!col && CONTROL_KEYS.some((k) => col[k] != null);
  }

  /// 控制行的宿主节点。**钩子给不出节点就明说**（不静默留一个空格子——那是"失败看起来像成功"）。
  function controlNodeFor(col, rec, recKey, where) {
    if (!ctx.controlNode) return el('span', 'sv-missing', '（没人接写面钩子）');
    let node = null;
    try {
      node = ctx.controlNode(col, rec, recKey, { where });
    } catch (e) {
      return el('span', 'sv-missing', '写面渲染抛错：' + (e && e.message ? e.message : e));
    }
    return node || el('span', 'sv-missing', '·');
  }

  function cellNode(rec, recKey, col, identity) {
    const td = el('td', 'sv-td');
    // 控制格：一个 `<td>` 里放宿主给的节点（`compact` 的格子窄一些，完整编辑器在卡片里）。
    if (isControlRow(col)) {
      td.classList.add('sv-td-ctl');
      if (col.compact) td.classList.add('sv-compact');
      td.appendChild(controlNodeFor(col, rec, recKey, 'table'));
      return td;
    }
    const v = evalPath(col.path, rec, recKey);
    ratioSlot = null;
    const text = format(v, col, rec);
    if (col.dot) {
      const c = evalPath(col.dot, rec, recKey);
      if (typeof c === 'string' && c) {
        const d = el('span', 'sv-dot');
        d.style.background = c;
        td.appendChild(d);
      }
    }
    const holder = valueNode(col, v, text);
    if (col.click && ctx.onSelect) {
      // 身份用**这一行是谁**（key 字段/首列），不是数组下标——否则会去选中"0 号"。
      const who = identity != null ? identity : v;
      holder.classList.add('clickable');
      holder.addEventListener('click', () => ctx.onSelect(col.click, who));
    }
    if (ctx.onPathClick && holder.title == null) holder.title = col.path;
    td.appendChild(holder);
    // 比例条只在真有数的时候画（值缺失时不画一条 0% 的假条）。
    if (!isNil(v) && (col.bar || (col.fmt === 'ratio' && ratioSlot))) holder.appendChild(barNode(ratioSlot || { v: +v, max: null }));
    return td;
  }

  // 「其余字段」：这一行的残差，交给通用的 jsonview 渲染（铁律 R）。默认收起，一条也不藏。
  // 列里只留一个短记号（`▸9`），键名放 title——宽表在 430px 的面板里很宝贵。
  function residualCell(rec, spec, rowId) {
    const td = el('td', 'sv-td sv-residual');
    const res = residualOf(rec, spec);
    if (!res) {
      td.appendChild(el('span', 'sv-missing', '无'));
      return td;
    }
    const keys = Object.keys(res);
    const open = ctx.expanded.has(rowId);
    const btn = el('span', 'sv-more clickable', (open ? '▾' : '▸') + keys.length);
    btn.title = '其余字段（' + keys.length + '）：' + keys.join('、') + '\n（引擎给这条记录加字段，它就会出现在这里——不需要改前端）';
    const box = el('div', 'sv-residual-box');
    box.style.display = open ? '' : 'none';
    if (open) {
      if (window.JsonView) {
        window.JsonView.render(box, res, { rootPath: '', expandDepth: 1, onPathClick: ctx.onPathClick, tip: ctx.tip });
      } else box.textContent = JSON.stringify(res);
    }
    btn.addEventListener('click', () => {
      const now = ctx.expanded.has(rowId);
      if (now) ctx.expanded.delete(rowId);
      else ctx.expanded.add(rowId);
      renderResidualInto(box, btn, res, keys, !now);
    });
    td.addEventListener('click', (ev) => {
      if (ev.target !== btn && !box.contains(ev.target)) btn.click();  // 整格可点（同上）
    });
    td.append(btn, box);
    return td;
  }

  function renderResidualInto(box, btn, res, keys, open) {
    btn.textContent = (open ? '▾' : '▸') + keys.length;
    btn.title = '其余字段（' + keys.length + '）：' + keys.join('、');
    box.style.display = open ? '' : 'none';
    if (open && !box.childElementCount) {
      if (window.JsonView) window.JsonView.render(box, res, { rootPath: '', expandDepth: 1, onPathClick: ctx.onPathClick, tip: ctx.tip });
      else box.textContent = JSON.stringify(res);
    }
  }

  // --- 上限的如实披露 -------------------------------------------------------
  function limitNote(shown, total, what) {
    if (total <= shown) return null;
    return el('div', 'sv-note', '已显示前 ' + shown + ' / 共 ' + total + ' ' + (what || '条') + '（其余 ' + (total - shown) + ' 条未显示）');
  }

  function orderRows(rows, order) {
    if (!order || !order.length) return rows;
    const out = rows.slice();
    out.sort((a, b) => {
      for (const o of order) {
        const av = evalPath(o.by, a.value, a.key);
        const bv = evalPath(o.by, b.value, b.key);
        let c = 0;
        if (typeof av === 'number' && typeof bv === 'number') c = av - bv;
        else c = String(av == null ? '' : av).localeCompare(String(bv == null ? '' : bv), 'zh');
        if (c) return (o.dir === 'desc' ? -1 : 1) * c;
      }
      return 0;
    });
    return out;
  }

  // --- 布局 -----------------------------------------------------------------
  function renderTable(container, spec) {
    // `source: null` 对**表**没有意义（表是"多条记录"的布局）⇒ 明说这是声明写错了，
    // 而不是安静地显示「这一帧没有数据」（那是另一件事：来源在，只是这帧空）。
    // 静态纪律在 `play/tests/g4_spec.py` 里也会拦（`layout: table` 不许 `source: null`）。
    if (spec.source === null) {
      container.appendChild(el('div', 'sv-empty',
        '（这条表的 `source` 是 `null`：表布局是"多条记录"的布局，`source: null` 只对 sheet / cards 有意义）'));
      return;
    }
    let rows = expand(spec.source);
    if (!rows.length) {
      container.appendChild(el('div', 'sv-empty', spec.empty || '（这一帧没有数据）'));
      return;
    }
    if (spec.group && spec.group.by) {
      const groups = new Map();
      rows.forEach((r) => {
        const g = evalPath(spec.group.by, r.value, r.key);
        const k = g == null ? '（未指定）' : String(g);
        if (!groups.has(k)) groups.set(k, []);
        groups.get(k).push(r);
      });
      const total = rows.length;
      const order = spec.order;
      groups.forEach((list, k) => {
        const head = el('div', 'sv-group');
        if (spec.group.dot) {
          const first = list[0];
          const c = evalPath(spec.group.dot, first.value, first.key);
          if (typeof c === 'string' && c) {
            const d = el('span', 'sv-dot');
            d.style.background = c;
            head.appendChild(d);
          }
        }
        head.appendChild(el('span', 'sv-group-name', k));
        head.appendChild(el('span', 'sv-group-n', list.length + ' 条'));
        container.appendChild(head);
        container.appendChild(tableFor(spec, orderRows(list, order), total, k));
      });
      if (spec.limit && spec.limit.n && total > spec.limit.n) {
        const n = limitNote(spec.limit.n, total, '条');
        if (n) container.appendChild(n);
      }
      return;
    }
    const ordered = orderRows(rows, spec.order);
    container.appendChild(tableFor(spec, ordered, rows.length, null));
  }

  function tableFor(spec, rows, total, groupKey) {
    const limited = spec.limit && spec.limit.n ? rows.slice(0, spec.limit.n) : rows;
    // **列序即优先级**：数组的顺序就是信息重要性的顺序。
    // 首列若就是 `key`（身份字段），就把它交给那一列身份格，别再单开一列（不然名字出现两次）。
    const all = spec.columns || [];
    const keyCol = all.length && spec.key && all[0].path === spec.key ? all[0] : null;
    const cols = keyCol ? all.slice(1) : all;

    const box = el('div', 'sv-tablebox');
    const table = el('table', 'sv-table');
    const thead = el('thead');
    const htr = el('tr');
    // `card` = 这一行展开时内联渲染哪张 `select` 卡片（读行 + 控制行都在那张卡里）。
    const wantCard = !!spec.card;
    if (wantCard) {
      const th = el('th', 'sv-th sv-th-card', '');
      th.title = '展开这一行的卡片（组织点「' + spec.card + '」）——读行与控制行住在同一条行序里';
      htr.appendChild(th);
    }
    // 身份列的表头也是**名词**（`势力`/`舰`/`城`…）⇒ 同样挂弹窗（它不是 `cols` 里的一条，
    // 所以上面那个循环盖不到它）。`spec.key` 就是那一列取值的字段名。
    const kth = el('th', 'sv-th sv-th-key', keyCol ? keyCol.label || spec.key : spec.key_label || '');
    if (ctx.tip) ctx.tip(kth, keyCol || { path: spec.key });
    htr.appendChild(kth);
    cols.forEach((c) => {
      const th = el('th', 'sv-th', c.label || c.path);
      // 列头是**名词** ⇒ 悬停弹它的解释。求值器仍然不认识领域词：把整条列声明交给宿主
      // （`ctx.tip`），由它决定"这条行的名词是什么、该查哪个字段"。
      // ⚠ 只挂给"名词"（列头 / 控制行标签 / 卡片里的字段名），**不给每个单元格挂**（太吵）。
      if (ctx.tip) ctx.tip(th, c);
      htr.appendChild(th);
    });
    htr.appendChild(el('th', 'sv-th sv-th-res', '其余'));
    thead.appendChild(htr);
    table.appendChild(thead);
    const tbody = el('tbody');
    table.appendChild(tbody);

    // 一条记录的**行内卡片**：把**手里这一条记录**喂给那张 `select` 卡片。
    // ⚠ 不是"按名字回 source 里再找一次"：表是分组的、记录本来就在手上，重新找既慢又可能
    // 找到另一条同名记录（舰与城同名、天体与势力同名都会踩）。
    const cardRowFor = (tr, r) => {
      const trCard = el('tr', 'sv-card-tr');
      const td = el('td', 'sv-card-td');
      td.colSpan = tr.childElementCount || 1;
      const box = el('div', 'sv-card-box');
      renderCard(box, spec.card, r.value, r.key);
      td.appendChild(box);
      trCard.appendChild(td);
      tr.parentNode.insertBefore(trCard, tr.nextSibling);
      return trCard;
    };

    const paint = (list) => {
      tbody.textContent = '';
      list.forEach((r) => {
        const tr = el('tr', 'sv-tr');
        const rowId = spec.id + '#' + (groupKey || '') + (r.key == null ? '' : r.key);
        const cardKey = rowId + ':card';
        let cardRow = null;
        if (wantCard) {
          const tdE = el('td', 'sv-td sv-td-expand');
          const open = ctx.expanded.has(cardKey);
          const btn = el('span', 'sv-more clickable', open ? '▾' : '▸');
          btn.title = '展开这一行的卡片（组织点「' + spec.card + '」）——读行与控制行在同一条行序里';
          // ⚠ 监听挂在**整个格**上（不是只挂那个 20px 的 span）：点在内边距上没反应是真实摩擦
          // ——2026-10 修；同一个毛病「其余 ▸N」那格也有，见 [`residualCell`]。
          btn.addEventListener('click', () => {
            const now = !ctx.expanded.has(cardKey);
            if (now) ctx.expanded.add(cardKey);
            else ctx.expanded.delete(cardKey);
            btn.textContent = now ? '▾' : '▸';
            if (now && !cardRow) cardRow = cardRowFor(tr, r);
            if (cardRow) cardRow.style.display = now ? '' : 'none';
          });
          tdE.appendChild(btn);
          tdE.addEventListener('click', (ev) => {
            if (ev.target !== btn) btn.click();   // 点在格子里（无论哪一处）都等于点那个箭头
          });
          tr.appendChild(tdE);
        }
        const td0 = el('td', 'sv-td sv-td-key');
        const label = keyCol ? evalPath(keyCol.path, r.value, r.key) : spec.key ? evalPath(spec.key, r.value, r.key) : r.key;
        const who = label == null ? r.key : label;
        const lab = el('span', keyCol || spec.key ? 'sv-val' : '', label == null ? String(r.key) : String(label));
        if (keyCol && keyCol.dot) {
          const c = evalPath(keyCol.dot, r.value, r.key);
          if (typeof c === 'string' && c) {
            const d = el('span', 'sv-dot');
            d.style.background = c;
            td0.appendChild(d);
          }
        }
        if (keyCol && keyCol.click && ctx.onSelect) {
          lab.classList.add('clickable');
          lab.addEventListener('click', () => ctx.onSelect(keyCol.click, who));
        }
        td0.appendChild(lab);
        tr.appendChild(td0);
        cols.forEach((c) => tr.appendChild(cellNode(r.value, recordKeyOf(spec, r), c, who)));
        tr.appendChild(residualCell(r.value, spec, rowId));
        tbody.appendChild(tr);
        // 展开状态住在 ctx.expanded 里 ⇒ 重画（推进回合 / 应用之后）时把开着的卡片一起重建。
        if (wantCard && ctx.expanded.has(cardKey)) cardRow = cardRowFor(tr, r);
      });
    };
    let shown = limited.length;
    paint(limited);
    const moreWrap = el('div', '');
    const apply = () => {
      moreWrap.textContent = '';
      const n = limitNote(shown, rows.length, '条');
      if (n) {
        n.classList.add('clickable');
        n.addEventListener('click', () => {
          shown = rows.length;
          paint(rows);
          apply();
        });
        moreWrap.appendChild(n);
      }
      if (spec.limit && spec.limit.n && rows.length > limited.length && shown > limited.length) {
        // 用户点了「显示全部」：此时把上限写成已解除
        moreWrap.appendChild(el('div', 'sv-note', '已展开全部 ' + rows.length + ' 条'));
      }
    };
    apply();
    const aside = box;
    aside.appendChild(table);
    aside.appendChild(moreWrap);
    return aside;
  }

  function renderSheet(container, spec) {
    // `source: null` = **不取任何记录**：那是「不挂在任何一条 state 记录上的东西」的家
    // （例如全局作用域的归属行——它属于控制面，不属于某一条势力/城/舰的记录）。
    // 与「这一帧没有数据」**不是一回事**：后者要明说（`sv-empty`），前者本来就该渲染。
    // ⚠ 键**必须在**（写 `null`，别省掉）：声明要自己说清「这张卡故意不依赖记录」。
    if (spec.source === null) {
      sheetOfRecord(container, spec, { value: {}, key: null });
      return;
    }
    const rows = expand(spec.source);
    if (!rows.length) {
      container.appendChild(el('div', 'sv-empty', spec.empty || '（这一帧没有数据）'));
      return;
    }
    rows.forEach((r) => sheetOfRecord(container, spec, r));
  }

  // 一条记录的「键值表 + 其余字段」——sheet 布局，也是 cards 布局的一块。
  function sheetOfRecord(container, spec, r) {
    const grid = el('div', 'sv-sheet');
    (spec.columns || []).forEach((c) => {
      // 控制行：中文标签 + 宿主给的节点。读行与控制行住在**同一个 columns 数组**里，
      // 所以「首都库存（读）紧挨投资预算（控制）」是数组顺序的直接结果。
      if (isControlRow(c)) {
        const crow = el('div', 'sv-sheet-row sv-sheet-row-ctl');
        const ck = el('div', 'sv-sheet-k', c.label || c.leaf || c.owner || c.action);
        if (ctx.tip) ctx.tip(ck, c);   // 卡片里的字段名同样是名词 ⇒ 悬停弹解释
        const cv = el('div', 'sv-sheet-v');
        cv.appendChild(controlNodeFor(c, r.value, recordKeyOf(spec, r), 'sheet'));
        crow.append(ck, cv);
        grid.appendChild(crow);
        return;
      }
      const v = evalPath(c.path, r.value, r.key);
      ratioSlot = null;
      const text = format(v, c, r.value);
      const row = el('div', 'sv-sheet-row');
      const k = el('div', 'sv-sheet-k', c.label || c.path);
      if (ctx.tip) ctx.tip(k, c);
      const val = el('div', 'sv-sheet-v');
      if (c.dot) {
        const col = evalPath(c.dot, r.value, r.key);
        if (typeof col === 'string' && col) {
          const d = el('span', 'sv-dot');
          d.style.background = col;
          val.appendChild(d);
        }
      }
      val.appendChild(valueNode(c, v, text));
      if (!isNil(v) && (c.bar || (c.fmt === 'ratio' && ratioSlot))) val.appendChild(barNode(ratioSlot || { v: +v, max: null }));
      row.append(k, val);
      grid.appendChild(row);
    });
    const res = residualOf(r.value, spec);
    const rowId = spec.id + '#sheet' + (r.key == null ? '' : r.key);
    if (res) {
      const keys = Object.keys(res);
      const open = ctx.expanded.has(rowId);
      const btn = el('div', 'sv-more clickable', (open ? '▾ ' : '▸ ') + '其余字段 ' + keys.length + ' 项');
      btn.title = '没有被这条视图认领的字段（引擎加字段会自动出现在这里）：' + keys.join('、');
      const box = el('div', 'sv-residual-box');
      box.style.display = open ? '' : 'none';
      if (open && window.JsonView) window.JsonView.render(box, res, { rootPath: '', expandDepth: 1, onPathClick: ctx.onPathClick, tip: ctx.tip });
      btn.addEventListener('click', () => {
        const now = !ctx.expanded.has(rowId);
        if (now) ctx.expanded.add(rowId);
        else ctx.expanded.delete(rowId);
        renderResidualInto(box, btn, res, keys, now);
      });
      grid.appendChild(btn);
      grid.appendChild(box);
    }
    container.appendChild(grid);
  }

  function renderTimeline(container, spec) {
    let rows = expand(spec.source);
    if (!rows.length) {
      container.appendChild(el('div', 'sv-empty', spec.empty || '（这一帧没有数据）'));
      return;
    }
    const n = spec.limit && spec.limit.n ? spec.limit.n : rows.length;
    const fromEnd = !spec.limit || spec.limit.from !== 'start';
    const shownRows = fromEnd ? rows.slice(Math.max(0, rows.length - n)) : rows.slice(0, n);
    const list = el('div', 'sv-timeline');
    shownRows.forEach((r) => {
      const item = el('div', 'sv-tl-item');
      const head = el('div', 'sv-tl-head');
      head.appendChild(el('span', 'sv-tl-title', String(evalPath(spec.title_path, r.value, r.key) || r.key || '')));
      (spec.meta || []).forEach((p) => {
        const v = evalPath(p, r.value, r.key);
        if (isNil(v)) return;
        head.appendChild(el('span', 'sv-tl-meta', isArr(v) ? v.join('、') : '回合 ' + v));
      });
      item.appendChild(head);
      if (spec.body) {
        const b = evalPath(spec.body, r.value, r.key);
        if (!isNil(b)) item.appendChild(el('div', 'sv-tl-body', String(b)));
      }
      list.appendChild(item);
    });
    container.appendChild(list);
    if (rows.length > shownRows.length) {
      const note = limitNote(shownRows.length, rows.length, '条故事节拍');
      if (note) {
        note.classList.add('clickable');
        note.title = '点一下显示更早的节拍';
        note.addEventListener('click', () => {
          spec = Object.assign({}, spec, { limit: { n: rows.length, from: 'end' } });
          container.textContent = '';
          renderTimeline(container, spec);
        });
        container.appendChild(note);
      }
    }
  }

  function renderPairs(container, spec) {
    const rows = expandScalarEntries(spec.source);
    if (!rows.length) {
      container.appendChild(el('div', 'sv-empty', spec.empty || '（这一帧没有数据）'));
      return;
    }
    let pairs = rows.map((r) => ({ k: r.k, v: r.v }));
    if (spec.sort) {
      pairs.sort((a, b) => {
        const va = typeof a.v === 'number' ? a.v : -Infinity;
        const vb = typeof b.v === 'number' ? b.v : -Infinity;
        return spec.sort === 'desc' ? vb - va : va - vb;
      });
    }
    const n = spec.limit && spec.limit.n ? spec.limit.n : pairs.length;
    const shown = pairs.slice(0, n);
    const box = el('div', 'sv-pairs');
    shown.forEach((p) => box.appendChild(el('span', 'sv-pair', p.k + ' ' + format(p.v, spec, null))));
    container.appendChild(box);
    if (pairs.length > shown.length) {
      const note = limitNote(shown.length, pairs.length, '项');
      if (note) container.appendChild(note);
    }
  }

  // cards：一条记录一张卡片（同 sheet，只是外面有框、可横排）
  function renderCards(container, spec) {
    const rows = expand(spec.source);
    if (!rows.length) {
      container.appendChild(el('div', 'sv-empty', spec.empty || '（这一帧没有数据）'));
      return;
    }
    const n = spec.limit && spec.limit.n ? spec.limit.n : rows.length;
    rows.slice(0, n).forEach((r) => {
      const card = el('div', 'sv-card');
      if (spec.title_path) {
        const t = evalPath(spec.title_path, r.value, r.key);
        if (!isNil(t)) card.appendChild(el('div', 'sv-card-title', String(t)));
      }
      sheetOfRecord(card, spec, r);
      container.appendChild(card);
    });
    if (rows.length > n) {
      const note = limitNote(n, rows.length, '张卡片');
      if (note) container.appendChild(note);
    }
  }

  const LAYOUT = { table: renderTable, sheet: renderSheet, timeline: renderTimeline, pairs: renderPairs, cards: renderCards };

  // --- 视图（一个视图 = 标题 + 一行说明 + 布局；omit 的省略必须写在脸上） -----
  function renderView(container, spec) {
    const wrap = el('div', 'sv-view');
    if (spec.title) wrap.appendChild(el('h3', 'sv-title', spec.title));
    if (spec.hint) wrap.appendChild(el('div', 'sv-hint', spec.hint));
    const body = el('div', 'sv-body');
    (LAYOUT[spec.layout] || renderTable)(body, spec);
    wrap.appendChild(body);
    const om = omitLine(spec);
    if (om) wrap.appendChild(om);
    container.appendChild(wrap);
  }

  function omitLine(spec) {
    if (!spec.omit || !spec.omit.length) return null;
    const d = el('div', 'sv-omit');
    d.textContent = '（本视图声明不看 ' + spec.omit.length + ' 项：' + spec.omit.map((o) => o.path + '——' + o.why).join('；') + '；它们仍在「未组织」页与原始 state 里）';
    return d;
  }

  // **选中读面**（`mount: select`）：从来源集合里挑出**这一条**记录，摆成键值表 + 其余字段。
  // 没命中就明说「当前世界里没有这条记录」——不是空白（那就是"失败看起来像成功"）。
  function renderSelect(container, spec, name) {
    const wrap = el('div', 'sv-view');
    const rows = expand(spec.source);
    const keyOf = (r) => (spec.key ? evalPath(spec.key, r.value, r.key) : r.key);
    const hit = rows.find((r) => String(keyOf(r)) === String(name));
    if (!hit) {
      wrap.appendChild(el('div', 'sv-empty', '（' + (spec.title || spec.id) + '「' + name + '」不在本帧的 @' + String(spec.source).replace(/^@/, '') + ' 里）'));
      container.appendChild(wrap);
      return;
    }
    sheetOfRecord(wrap, spec, hit);
    const om = omitLine(spec);
    if (om) wrap.appendChild(om);
    container.appendChild(wrap);
  }

  /// 把**一条记录**喂给一张 `select` 卡片（表格的行内展开用它，见 `tableFor` 的 `card`）。
  /// 卡片就是 `mount: select` 的普通视图：同一份列声明，读行与控制行按同一条行序。
  function renderCard(container, cardId, record, recKey) {
    const spec = findById(cardId);
    if (!spec) {
      container.appendChild(el('div', 'sv-empty', '（没有这张卡片：' + cardId + '）'));
      return null;
    }
    container.appendChild(el('div', 'sv-card-head',
      '卡片「' + (spec.title || spec.id) + '」（组织点 ' + spec.id + '）——读行与控制行按同一条行序'));
    sheetOfRecord(container, spec, { key: recKey, value: record, path: '' });
    const om = omitLine(spec);
    if (om) container.appendChild(om);
    return spec;
  }

  // --- 挂载点查找（给 app.js / jsonview.js 用） ------------------------------
  let DOC = { pages: [], select: [] };

  function setSpecs(doc) {
    DOC = doc || { pages: [], select: [] };
    ctx.maps = {};
    Object.keys(DOC).forEach((k) => {
      if (k === 'pages' || k === 'select' || k === 'version' || k === 'note') return;
      ctx.maps[k] = DOC[k];
    });
    INLINE.length = 0;
    DOC.pages.forEach((p) => (p.views || []).forEach((v) => { if (v.mount === 'inline' && v.inline_at) INLINE.push(v); }));
    (DOC.inline || []).forEach((v) => INLINE.push(v));
  }

  const INLINE = [];

  // 通用树里的**原位重组**：路径命中某个组织点 ⇒ 那一处交给 spec 渲染（卡片/表格 + 残差）。
  function inlineFor(path) {
    for (const spec of INLINE) {
      if (pathMatch(spec.inline_at, path)) return resolve(spec);
      for (const [pat, id] of Object.entries(spec.use_at || {})) if (pathMatch(pat, path)) return findById(id);
    }
    return null;
  }

  function findById(id) {
    if (!id) return null;
    const all = [];
    DOC.pages.forEach((p) => all.push(...(p.views || [])));
    all.push(...(DOC.select || []), ...INLINE);
    const hit = all.find((v) => v.id === id);
    return hit ? resolve(hit) : null;
  }

  function resolve(spec) {
    if (spec.use) {
      const base = findById(spec.use);
      if (base) return Object.assign({}, base, spec, { columns: base.columns });
    }
    return spec;
  }

  // 路径模式匹配（只支持 `[*]` 通配，够表达「这个集合的任意一项」）。
  function pathMatch(pattern, path) {
    if (!pattern) return false;
    const rx = String(pattern)
      .split('.')
      .map((s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&').replace(/\\\[\\\*\\\]/g, '\\[\\d+\\]'))
      .join('\\.');
    return new RegExp('^' + rx + '$').test(String(path));
  }

  function selectSpecFor(kind) {
    const hit = (DOC.select || []).find((v) => v.select_kind === kind);
    return hit ? resolve(hit) : null;
  }

  function pages() { return DOC.pages || []; }

  /// 整份声明（宿主要读 `leaf_ui`/`action_ui`/`write_omit` 这类顶层段——它们不是视图，
  /// 但也住在同一份声明里；`setSpecs` 已把它们塞进 `ctx.maps`，这里给宿主一个直读口）。
  function doc() { return DOC; }

  // 一个视图「认领了哪些字段」——给「未组织」审计用（铁律 R 的另一半：没被认领的要看得见）。
  // 控制行也算认领：`leaf` 行认领的是 `@control` 上的那条路径（它由写面的控制行渲染，
  // 不能同时又报成「未组织」），`owner` 行认领 `@scope` 的层，`action` 行认领记录上的那个字段。
  function claimedPaths(doc) {
    const out = [];
    const walk = (spec) => {
      (spec.columns || []).forEach((c) => {
        if (c.leaf != null) out.push({ view: spec.id, expr: c.leaf, control: true });
        else if (c.owner != null) out.push({ view: spec.id, expr: '@scope.' + c.owner, control: true });
        else if (c.action != null) out.push({ view: spec.id, expr: c.action, control: true });
        else out.push({ view: spec.id, expr: c.path });
      });
      if (spec.source) out.push({ view: spec.id, expr: spec.source });
    };
    (doc.pages || []).forEach((p) => (p.views || []).forEach(walk));
    (doc.select || []).forEach(walk);
    return out;
  }

  window.SpecView = {
    bind,
    setSpecs,
    pages,
    doc,
    renderView,
    renderSheet,
    renderSelect,
    renderCard,
    inlineFor,
    selectSpecFor,
    isControlRow,
    evalPath,
    expand,
    claimedKeys,
    residualOf,
    claimedPaths,
    format,
    num,
    el,
  };
})();
