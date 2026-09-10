// 通用 JSON 组件 —— **schema-agnostic widget**。
//
// 设计铁律：本文件**不认识任何领域字段名**（不出现 bodies/cities/factions/ships/
// control/…，也不假设任何字段的类型）。它只认 JSON 的形状，并且「形状 → 布局」的
// 决策只有一处（[`layoutOf`]），渲染与「全部展开」都从它派生，不会各自漂移：
//
//   object   值全是标量      → 键值行（key | value）
//            值全是对齐对象   → 自动表格（首列 = 对象 key，其余列 = 内层键并集）
//            其余             → 递归子节点（每个 key 一个可折叠节点）
//   array    全标量          → chips
//            全对象           → 自动表格（列 = 各元素键的并集，按首次出现顺序）
//            其余             → 按下标的递归节点
//   标量                      → 数字/布尔/字符串渲染
//
// 推论：**state 结构怎么改都行**——加字段、加深嵌套、换类型、换命名、换 key 名，
// 组件都不需要改一个字符：新字段自动出现在树里。领域知识只存在于「决定渲染哪个根」
// 的调用代码里（app.js 几行），不在渲染逻辑里。
//
// 用法：
//   JsonView.render(container, anyJsonValue, {
//     rootPath: 'state',            // 路径前缀（面包屑 / copy 路径 / 展开状态的身份）
//     expandDepth: 1,               // 默认展开到第几层
//     state: { expanded: Set },     // 跨渲染保留的展开状态（调用方持有）
//     filter: '治理',                // 子串过滤（大小写不敏感；命中祖先自动展开）
//     onPathClick: (path, v) => {}, // 点击叶子：拿到它的 JSON 路径与值
//   });
'use strict';

(function () {
  const PAGE = 50;        // 表格分页：每次多渲染多少行
  const MAX_COLS = 16;    // 自动表格最多几列
  const PREVIEW = 72;     // 折叠态预览串最大长度
  const MAX_DEPTH = 14;   // 路径遍历的深度上限（防御性）

  // --- 形状判定（唯一「类型知识」，与领域无关） ------------------------------
  const isArr = (v) => Array.isArray(v);
  const isObj = (v) => v !== null && typeof v === 'object' && !isArr(v);
  const isScalar = (v) => v === null || typeof v !== 'object';

  // 「形状 → 布局」的唯一决策点。
  //
  // 「值全是对象」的容器要 ≥3 项才当**集合**（映射表：一行一项）；只有 2 项的
  // 容器更像**记录**（如派生态的 `{flow, metrics}`），铺成 2 行 × N 列的宽表反而
  // 难读，所以按普通节点展开。这是纯粹的项数阈值，与字段名无关。
  function layoutOf(v) {
    if (isScalar(v)) return 'leaf';
    if (isArr(v)) {
      if (!v.length) return 'empty';
      if (v.every(isScalar)) return 'chips';
      if (v.every(isObj)) return 'table';
      return 'nodes';
    }
    const vals = Object.keys(v).map((k) => v[k]);
    if (!vals.length) return 'empty';
    if (vals.every(isScalar)) return 'kv';
    if (vals.every(isObj) && vals.length >= 3) return 'table';
    return 'nodes';
  }

  function fmtNum(n) {
    if (!Number.isFinite(n)) return String(n);
    if (Number.isInteger(n)) return String(n);
    if (Math.abs(n) < 1e-9) return '0';
    return String(Math.round(n * 1e4) / 1e4);
  }
  function fmtScalar(v) {
    if (v === null) return '∅';
    if (typeof v === 'number') return fmtNum(v);
    if (typeof v === 'boolean') return v ? 'true' : 'false';
    if (typeof v === 'string') return v === '' ? '""' : v;
    return String(v);
  }

  // 路径：`state.cities[3].loyalty`（不是合法标识符的 key 用 ["..."] 包裹）
  function joinPath(parent, key, isIndex) {
    if (isIndex) return parent + '[' + key + ']';
    const k = String(key);
    if (/^[A-Za-z_$][\w$]*$/.test(k)) return parent ? parent + '.' + k : k;
    return parent + '["' + k.replace(/"/g, '\\"') + '"]';
  }

  // 折叠态的一行预览（generic：只取前几个标量子项）
  function preview(v) {
    if (isArr(v)) return v.filter(isScalar).slice(0, 5).map(fmtScalar).join(', ').slice(0, PREVIEW);
    const ks = Object.keys(v);
    const s = ks.filter((k) => isScalar(v[k])).slice(0, 4).map((k) => k + '=' + fmtScalar(v[k])).join('  ');
    return (s || ks.slice(0, 4).join(', ')).slice(0, PREVIEW);
  }

  // --- 过滤（generic：键名 + 标量值 的子串匹配） ----------------------------
  function hit(v, key, q) {
    if (!q) return true;
    if (v === undefined) return false;
    if (String(key).toLowerCase().includes(q)) return true;
    if (isScalar(v)) return fmtScalar(v).toLowerCase().includes(q);
    if (isArr(v)) return v.some((e, i) => hit(e, i, q));
    return Object.keys(v).some((k) => hit(v[k], k, q));
  }

  // --- 展开状态（编码封装在本文件；调用方只持有那个 Set） -------------------
  const OPEN = '+', SHUT = '-';
  function setOpen(set, path, open) {
    set.delete(OPEN + path);
    set.delete(SHUT + path);
    set.add((open ? OPEN : SHUT) + path);
  }
  function isOpen(ctx, path, depth) {
    if (ctx.expanded.has(SHUT + path)) return false;
    if (ctx.expanded.has(OPEN + path)) return true;
    if (ctx.q) return true; // 过滤时默认展开，让命中可见
    return depth <= ctx.expandDepth;
  }

  // --- DOM 小工具 -----------------------------------------------------------
  function el(tag, cls, text) {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text != null) e.textContent = text;
    return e;
  }
  // 标量值 span（点击复制路径）
  function valSpan(v, path, ctx) {
    const s = el('span', 'jv-val jv-' + (v === null ? 'null' : typeof v), fmtScalar(v));
    s.title = path;
    if (ctx.onPathClick) {
      s.classList.add('clickable');
      s.addEventListener('click', () => ctx.onPathClick(path, v));
    }
    return s;
  }
  // 一行 key | value
  function leafEl(v, key, path, ctx) {
    const row = el('div', 'jv-leaf');
    if (key !== undefined) row.appendChild(el('span', 'jv-key', String(key)));
    row.appendChild(valSpan(v, path, ctx));
    return row;
  }

  // 组合节点（对象/数组）：头部（caret + key + 计数 + 折叠预览）+ 惰性 body。
  //
  // `childCtx` = 铺 body 时用的上下文：与 `ctx` 只差一个「是否继续过滤」——当**节点
  // 自己的名字**命中了过滤词时，它的内部就整体展示（不再逐层过滤，否则会出现
  // 「节点在、内容却全是（无匹配）」这种自相矛盾的视图）。
  function compositeEl(v, key, path, depth, ctx, childCtx) {
    const inner = childCtx || ctx;
    const wrap = el('div', 'jv-node');
    const head = el('div', 'jv-head');
    const caret = el('span', 'jv-caret');
    head.appendChild(caret);
    head.appendChild(el('span', 'jv-key', String(key)));
    head.appendChild(el('span', 'jv-kind', isArr(v) ? '[' + v.length + ']' : '{' + Object.keys(v).length + '}'));
    const pv = el('span', 'jv-preview', preview(v));
    head.appendChild(pv);
    const body = el('div', 'jv-body');
    wrap.append(head, body);

    let open = isOpen(ctx, path, depth);
    let built = false;
    const paint = () => {
      caret.textContent = open ? '▾' : '▸';
      pv.style.display = open ? 'none' : '';
      body.style.display = open ? '' : 'none';
      if (open && !built) {
        built = true;
        fill(body, v, path, depth + 1, inner);
      }
    };
    paint();
    head.addEventListener('click', () => {
      open = !open;
      setOpen(ctx.expanded, path, open);
      paint();
    });
    return wrap;
  }

  // --- 自动表格（generic：列 = 键的并集，按首次出现顺序） -------------------
  // entries = [{ label, value, path }]；value 是对象时才有列。
  function tableEl(entries, ctx, depth) {
    const allCols = colsOf(entries);
    // 列级过滤：查询命中某些**列名**时只留这些列（在宽表里找某个字段立刻聚焦）；
    // 没命中任何列名就保留全部列，只按行过滤。规则纯结构，不认字段名。
    const named = ctx.q ? allCols.filter((c) => String(c).toLowerCase().includes(ctx.q)) : allCols;
    const cols = ctx.q && named.length ? named : allCols;
    // 行级过滤：行标签命中，或**所留列**的深层值命中（留下的列正是用户要找的）。
    const kept = entries.filter(
      (e) => !ctx.q || hit(e.label, e.label, ctx.q) || cols.some((c) => hit(cellOf(e.value, c), c, ctx.q))
    );
    if (!kept.length || !cols.length) return el('div', 'jv-empty', '（无匹配）');

    const box = el('div', 'jv-table-box');
    const table = el('table', 'jv-table');
    const thead = el('thead');
    const htr = el('tr');
    htr.appendChild(el('th', 'jv-th jv-th-key', ''));
    cols.forEach((c) => htr.appendChild(el('th', 'jv-th', c)));
    thead.appendChild(htr);
    table.appendChild(thead);
    const tbody = el('tbody');
    table.appendChild(tbody);
    box.appendChild(table);

    const more = el('button', 'jv-more');
    let shown = 0;
    const page = () => {
      const next = kept.slice(shown, shown + PAGE);
      next.forEach((e) => tbody.appendChild(rowEl(e, cols, ctx, depth)));
      shown += next.length;
      const left = kept.length - shown;
      if (left > 0) {
        more.textContent = '显示更多（还有 ' + left + ' 项）';
        if (!more.isConnected) box.appendChild(more);
      } else if (more.isConnected) {
        more.remove();
      }
    };
    more.addEventListener('click', page);
    page();
    return box;
  }

  // 取出「行」的某一列值；行不是对象时无值。
  function cellOf(row, col) {
    return isObj(row) ? row[col] : undefined;
  }

  // 列 = 各行键的并集，按首次出现顺序（上限 MAX_COLS）。
  function colsOf(entries) {
    const cols = [];
    outer: for (const e of entries) {
      if (!isObj(e.value)) continue;
      for (const k of Object.keys(e.value)) {
        if (!cols.includes(k)) cols.push(k);
        if (cols.length >= MAX_COLS) break outer;
      }
    }
    return cols;
  }

  // 一行：标量单元格直写；组合单元格显示摘要，点击在该行下就地递归展开。
  function rowEl(e, cols, ctx, depth) {
    const tr = el('tr', 'jv-tr');
    const td0 = el('td', 'jv-td jv-td-key');
    const lbl = el('span', 'jv-key' + (ctx.onPathClick ? ' clickable' : ''), String(e.label));
    if (ctx.onPathClick) lbl.addEventListener('click', () => ctx.onPathClick(e.path, e.value));
    td0.appendChild(lbl);
    tr.appendChild(td0);

    for (const c of cols) {
      const td = el('td', 'jv-td');
      const v = isObj(e.value) ? e.value[c] : undefined;
      const p = joinPath(e.path, c, false);
      if (v === undefined) {
        td.appendChild(el('span', 'jv-absent', '·'));
      } else if (isScalar(v)) {
        td.appendChild(valSpan(v, p, ctx));
      } else {
        const sum = el('span', 'jv-sum clickable', (isArr(v) ? '[' + v.length + ']' : '{' + Object.keys(v).length + '}') + ' ' + preview(v));
        sum.title = p;
        sum.addEventListener('click', () => {
          const holder = td.querySelector('.jv-holder');
          if (holder.childElementCount) holder.textContent = '';
          else fill(holder, v, p, depth + 2, ctx);
        });
        td.append(sum, el('div', 'jv-holder'));
      }
      tr.appendChild(td);
    }
    return tr;
  }

  // --- fill：把一个组合值铺进容器（按 layoutOf 分派） ----------------------
  function fill(body, v, path, depth, ctx) {
    const q = ctx.q;
    const layout = layoutOf(v);
    if (layout === 'empty') return void body.appendChild(el('div', 'jv-empty', isArr(v) ? '[] 空' : '{} 空'));

    if (layout === 'chips') {
      const chips = el('div', 'jv-chips');
      v.forEach((e, i) => {
        if (!hit(e, i, q)) return;
        const c = el('span', 'jv-chip', fmtScalar(e));
        const p = joinPath(path, i, true);
        c.title = p;
        if (ctx.onPathClick) {
          c.classList.add('clickable');
          c.addEventListener('click', () => ctx.onPathClick(p, e));
        }
        chips.appendChild(c);
      });
      if (!chips.childElementCount) body.appendChild(el('div', 'jv-empty', '（无匹配）'));
      else body.appendChild(chips);
      return;
    }

    if (layout === 'table') {
      const entries = isArr(v)
        ? v.map((e, i) => ({ label: '[' + i + ']', value: e, path: joinPath(path, i, true) }))
        : Object.keys(v).map((k) => ({ label: k, value: v[k], path: joinPath(path, k, false) }));
      return void body.appendChild(tableEl(entries, ctx, depth));
    }

    if (layout === 'kv') {
      const grid = el('div', 'jv-kv');
      Object.keys(v).forEach((k) => {
        if (!hit(v[k], k, q)) return;
        grid.appendChild(leafEl(v[k], k, joinPath(path, k, false), ctx));
      });
      if (!grid.childElementCount) body.appendChild(el('div', 'jv-empty', '（无匹配）'));
      else body.appendChild(grid);
      return;
    }

    // layout === 'nodes'
    const kids = isArr(v)
      ? v.map((e, i) => ({ key: '[' + i + ']', value: e, path: joinPath(path, i, true) }))
      : Object.keys(v).map((k) => ({ key: k, value: v[k], path: joinPath(path, k, false) }));
    let any = false;
    kids.forEach((k) => {
      // 自己的名字命中过滤词 → 整棵子树照常展示（不再层层过滤）。
      const nameHit = !!q && String(k.key).toLowerCase().includes(q);
      if (!nameHit && !hit(k.value, k.key, q)) return;
      any = true;
      const sub = nameHit ? stopFiltering(ctx) : ctx;
      body.appendChild(
        isScalar(k.value) ? leafEl(k.value, k.key, k.path, ctx) : compositeEl(k.value, k.key, k.path, depth, ctx, sub)
      );
    });
    if (!any) body.appendChild(el('div', 'jv-empty', '（无匹配）'));
  }

  /// 关掉子树过滤：只在父级**按名字**命中时使用（见 [`fill`] 的 nodes 分支）。
  function stopFiltering(ctx) {
    return ctx.q ? { ...ctx, q: '' } : ctx;
  }

  // --- 公开 API -------------------------------------------------------------
  // 渲染任意 JSON 值进 container（会清空 container）。
  function render(container, value, opts) {
    const o = opts || {};
    const ctx = {
      rootPath: o.rootPath || '',
      q: String(o.filter == null ? '' : o.filter).trim().toLowerCase(),
      expanded: (o.state && o.state.expanded) || new Set(),
      expandDepth: o.expandDepth == null ? 1 : o.expandDepth,
      onPathClick: o.onPathClick || null,
    };
    container.textContent = '';
    const root = el('div', 'jv-root');
    ctx.rootPath = ctx.rootPath || 'root';
    if (isScalar(value)) root.appendChild(leafEl(value, undefined, ctx.rootPath, ctx));
    else fill(root, value, ctx.rootPath, 0, ctx);
    container.appendChild(root);
    return container;
  }

  // 所有**可折叠节点**的路径——供「全部展开/收起」用。
  //
  // 与 `fill` 同源：节点 = 「父级布局是 nodes」的非标量子项（父级若是 kv/chips/table，
  // 其子项不是节点，而是行/单元格/chip）。注意「节点」与「它的 body 是什么布局」是两件事：
  // 一个节点的 body 是表格，它自己依然可折叠。root 不产节点（root 只提供容器）。
  function expandablePaths(value, rootPath) {
    const out = [];
    const walk = (v, path, depth, isRoot) => {
      if (isScalar(v) || depth > MAX_DEPTH) return;
      if (!isRoot) out.push(path);
      if (layoutOf(v) !== 'nodes') return;
      if (isArr(v)) v.forEach((e, i) => walk(e, joinPath(path, i, true), depth + 1, false));
      else Object.keys(v).forEach((k) => walk(v[k], joinPath(path, k, false), depth + 1, false));
    };
    walk(value, rootPath, 0, true);
    return out;
  }

  window.JsonView = { render, expandablePaths, setOpen, fmtScalar, preview, joinPath };
})();
