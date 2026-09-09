// 行星X WebUI 前端 — 经典脚本（控制面板/DOM）+ three.js 3D 地图（map3d.js，ES module）。
//
// 控制面板是一个沿控制作用域链下钻的 tab 树：
//   全局 -> 势力 -> 分类(舰/预算/天体) -> 天体 -> 城市 -> 建筑
// 每个容器节点显示一条 tab 带、只展开选中的子节点；分类组列出全部子项。
// 地图由 map3d.js 渲染 WebGL 场景，通过 window.PlanetXMap.setWorld/onSelect 与这里耦合。

let world = null;      // 当前 StateView（/api/state）
let meta = null;       // MetaView（/api/meta）
let selFaction = 0;    // 选中势力 id（读面/聚焦）
let selShip = null;    // 选中舰名（读面）
let edControl = [];    // 所有势力的可编辑控制（FactionControlView[]）
let edScope = null;    // 可编辑作用域
let prevState = null;  // 上一帧，用于 diff 页脚
let selTab = new Map(); // parentKey -> 激活子 key（每层只开一个 tab）

const $ = (sel, root) => (root || document).querySelector(sel);
const el = (tag, attrs, html) => {
  const e = document.createElement(tag);
  if (attrs) for (const k in attrs) e.setAttribute(k, attrs[k]);
  if (html != null) e.innerHTML = html;
  return e;
};

async function fetchJSON(url) {
  const r = await fetch(url);
  return r.json();
}
async function postJSON(url, body) {
  const r = await fetch(url, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
  return r.json();
}

// 势力显示色来自后端（/api/state -> faction.color，CSS hex）。未知回落灰。
function facColor(fid) {
  const f = world.factions.find((x) => x.id === fid);
  return (f && f.color) || '#8f9bb3';
}

// --- 行为辅助（当前 ShipBehavior 的 serde 形状） --------------------------
// Idle / {Move:{position}} / {Follow:{ship}} / {DockCity:{city}} /
// {Dock:{body}} / {Colonize:{body}} —— 实体一律用「唯一名」作为 key。
function behaviorType(b) {
  if (typeof b === 'string') return 'idle';
  if (b && b.Move) return 'move';
  if (b && b.Follow) return 'follow';
  if (b && b.DockCity) return 'dock_city';
  if (b && b.Dock) return 'dock';
  if (b && b.Colonize) return 'colonize';
  return 'idle';
}
function behaviorSummary(b, w) {
  const shipName = (id) => { const s = w.ships.find((x) => x.name === id); return s ? s.name : '船#' + id; };
  const cityName = (id) => { const c = w.cities.find((x) => x.name === id); return c ? c.name : '城#' + id; };
  const bodyName = (id) => { const bd = w.bodies.find((x) => x.name === id); return bd ? bd.name : '天体#' + id; };
  switch (behaviorType(b)) {
    case 'move': return '移动(' + (b.Move.position[0] | 0) + ',' + (b.Move.position[1] | 0) + ')';
    case 'follow': return '→' + shipName(b.Follow.ship) + '·跟随';
    case 'dock_city': return '→' + cityName(b.DockCity.city) + '·停泊城';
    case 'dock': return '→' + bodyName(b.Dock.body) + '·停泊轨道';
    case 'colonize': return '→' + bodyName(b.Colonize.body) + '·殖民';
    default: return '待命';
  }
}
function behaviorToInput(b) {
  switch (behaviorType(b)) {
    case 'move': return { x: b.Move.position[0], y: b.Move.position[1] };
    case 'follow': return { ship: b.Follow.ship };
    case 'dock_city': return { city: b.DockCity.city };
    case 'dock': return { body: b.Dock.body };
    case 'colonize': return { body: b.Colonize.body };
    default: return {};
  }
}
function behaviorFromInput(type, d) {
  switch (type) {
    case 'move': return { Move: { position: [+d.x || 0, +d.y || 0] } };
    case 'follow': return { Follow: { ship: (d.ship || '') } };
    case 'dock_city': return { DockCity: { city: (d.city || '') } };
    case 'dock': return { Dock: { body: (d.body || '') } };
    case 'colonize': return { Colonize: { body: (d.body || '') } };
    default: return 'Idle';
  }
}

// --- 节点类型注册表 ---------------------------------------------------------
// 每种的 per-kind 行为统一放在这里：renderNode/modeToggleFor 不再 switch 裸字符串。
//  childMode  'tabs'  容器：tab 带，仅展开激活子节点
//             'list'  分类组：全部子项堆叠
//             'leaf'  终端节点（无子）
//  scope      选 AI/玩家 toggle 的来源：
//             null 无 toggle（纯分组容器）；'global' edScope.global；
//             'factions'/'bodies'/'cities' scopeVal(edScope[k], id)；'leaf' node.leaf.mode
//  editor     'ship'  舰行为编辑器（仅 Player）| 'value' 数值叶子编辑器 | 'building' 建筑
const KIND = {
  global:    { childMode: 'tabs', scope: 'global' },
  faction:   { childMode: 'tabs', scope: 'factions', selectFaction: true },
  group:     { childMode: 'list', scope: null },
  body:      { childMode: 'tabs', scope: 'bodies' },
  city:      { childMode: 'list', scope: 'cities' },
  ship:      { childMode: 'leaf', scope: 'leaf', editor: 'ship', decorateLabel: (n, w) => ' · ' + behaviorSummary(n.leaf.behavior, w) },
  resource:  { childMode: 'leaf', scope: 'leaf', editor: 'value', editorLabel: '投资预算/回合' },
  conbudget: { childMode: 'leaf', scope: 'leaf', editor: 'value', editorLabel: '建造预算/回合' },
  building:  { childMode: 'leaf', scope: 'leaf', editor: 'building' },
};

function scopeAccess(node) {
  const spec = KIND[node.kind] || {};
  const s = spec.scope;
  if (s === null || s === undefined) return null;
  if (s === 'leaf') return node.leaf ? { get: () => node.leaf.mode, set: (v) => { node.leaf.mode = v; } } : null;
  if (s === 'global') return { get: () => edScope.global, set: (v) => { edScope.global = v; } };
  return {
    get: () => scopeVal(edScope[s], node.id),
    set: (v) => setScopeVal(edScope[s], node.id, v),
  };
}

// --- 状态加载 / 选择 --------------------------------------------------------
async function init() {
  meta = await fetchJSON('/api/meta');
  prevState = await fetchJSON('/api/state');
  world = await fetchJSON('/api/state');
  selFaction = world.factions.length ? world.factions[0].id : 0;
  initMap();
  buildEdits();
  renderAll();
  updateTop();
}

function initMap() {
  const c = $('#map');
  if (!c || !window.PlanetXMap) return;
  try { window.PlanetXMap.init(c); } catch (e) { console.error('map init failed', e); }
  window.PlanetXMap.onSelect = mapSelect;
}

function buildEdits() {
  edControl = structuredClone(world.control || []);
  edScope = structuredClone(world.scope);
  edControl.forEach((c) => { c.buildings = c.buildings || []; });
}

function setFaction(fid) {
  if (fid === selFaction) return;
  selFaction = fid;
  selTab.set('g', 'f' + fid);
  renderTree();
  renderReadout();
}

function getControl(fid) {
  let c = edControl.find((x) => x.faction_id === fid);
  if (!c) {
    c = { faction_id: fid, ship_orders: [], investment_budget: [], construction_budget: [], invest_weights: [], build_weights: [], buildings: [] };
    edControl.push(c);
  }
  return c;
}

function scopeVal(list, id) {
  const e = list.find((x) => x[0] === id);
  return e ? e[1] : null;
}
function setScopeVal(list, id, val) {
  const i = list.findIndex((x) => x[0] === id);
  if (i >= 0) list[i] = [id, val];
  else list.push([id, val]);
}

function resName(r) { return meta.resources[r] ? meta.resources[r].name : r; }
function structName(s) { return meta.structures[s] ? meta.structures[s].name : s; }
function kindName(k) { return meta.buildings[k] ? meta.buildings[k].label : k; }
function shipClassName(c) { return meta.ships[c] ? meta.ships[c].label : c; }

function investLeaf(fc, city, bid) {
  return (fc.invest_weights || []).find((e) => e.city === city && e.building === bid);
}
function buildLeaf(fc, city, bid) {
  return (fc.build_weights || []).find((e) => e.city === city && e.building === bid);
}
function buildingLabel(b) {
  let s = kindName(b.kind);
  if (b.resource) s += '·' + resName(b.resource);
  if (b.ship_type) s += '·' + shipClassName(b.ship_type);
  s += '·' + structName(b.structure);
  s += '×' + (b.deployed || 0).toFixed(1);
  return s;
}

// --- 主渲染 ----------------------------------------------------------------
function renderAll() {
  renderMap();
  renderTree();
  renderReadout();
  renderDiff();
}

function updateTop() {
  $('#metaRound').textContent = '回合 ' + world.round + ' · ' + world.time_month + ' 个月';
}

// 地图：把当前 world 交给 three.js 场景（map3d.js）。
function renderMap() {
  if (window.PlanetXMap && window.PlanetXMap.setWorld) window.PlanetXMap.setWorld(world);
}

// 地图点击回调，更新读面 + 选中舰。
function mapSelect(sel) {
  const readout = $('#readout');
  if (sel.kind === 'body') {
    selShip = null;
    readout.textContent = '天体 ' + sel.name;
  } else if (sel.kind === 'city') {
    selShip = null;
    readout.textContent = '城市 ' + sel.name;
  } else if (sel.kind === 'ship') {
    selShip = sel.name;
    const s = world.ships.find((x) => x.name === sel.name);
    const fc = (world.control || []).find((x) => x.faction_id === (s && s.faction_id));
    const order = fc ? (fc.ship_orders || []).find((o) => o.ship === sel.name) : null;
    const fname = (world.factions.find((x) => x.id === (s && s.faction_id)) || {}).name;
    readout.textContent = s.name + ' [' + (fname || '') + '] ' + behaviorSummary(order ? order.behavior : null, world);
  }
}

// --- 层级树（控制 + 作用域）-----------------------------------------------
function buildTree() {
  const root = { key: 'g', kind: 'global', name: '全局', children: [] };
  world.factions.forEach((f) => {
    const fid = f.id;
    const fc = getControl(fid);
    const fn = { key: 'f' + fid, kind: 'faction', id: fid, name: f.name, color: f.color, fid, children: [] };

    const shipLeaves = (fc.ship_orders || []).map((ord) => {
      const s = world.ships.find((x) => x.name === ord.ship);
      return { key: 'ship' + ord.ship, kind: 'ship', id: ord.ship, name: (s ? s.name : '船#' + ord.ship), leaf: ord, fid };
    });

    const invLeaves = (fc.investment_budget || []).map((e) => ({
      key: 'inv' + fid + ':' + e.resource, kind: 'resource', name: resName(e.resource), leaf: e, fid,
    }));
    const conLeaves = (fc.construction_budget || []).map((e) => ({
      key: 'con' + fid + ':' + e.resource, kind: 'conbudget', name: resName(e.resource), leaf: e, fid,
    }));
    const budgetKids = [];
    if (invLeaves.length) budgetKids.push({ key: 'gi' + fid, kind: 'group', name: '投资预算', fid, children: invLeaves });
    if (conLeaves.length) budgetKids.push({ key: 'gc' + fid, kind: 'group', name: '建造预算', fid, children: conLeaves });

    const bodyNodes = [];
    world.cities.filter((c) => c.faction_id === fid).forEach((ci) => {
      const bid = ci.body_id;
      let bn = bodyNodes.find((n) => n.id === bid);
      if (!bn) {
        const b = world.bodies.find((x) => x.name === bid);
        bn = { key: 'b' + fid + '-' + bid, kind: 'body', id: bid, name: (b ? b.name : '天体#' + bid), children: [], fid };
        bodyNodes.push(bn);
      }
      const cn = { key: 'c' + ci.name, kind: 'city', id: ci.name, name: ci.name, children: [], fid, cityId: ci.name };
      (ci.buildings || []).forEach((b) => {
        cn.children.push({
          key: 'bld' + fid + ':' + ci.name + ':' + b.id,
          kind: 'building', id: b.id, name: buildingLabel(b),
          leaf: investLeaf(fc, ci.name, b.id), buildLeaf: buildLeaf(fc, ci.name, b.id),
          b, fid, cityId: ci.name,
        });
      });
      bn.children.push(cn);
    });

    const cats = [];
    if (shipLeaves.length) cats.push({ key: 'gs' + fid, kind: 'group', name: '舰', fid, children: shipLeaves });
    if (budgetKids.length) cats.push({ key: 'gbd' + fid, kind: 'group', name: '预算', fid, children: budgetKids });
    if (bodyNodes.length) cats.push({ key: 'gb' + fid, kind: 'group', name: '天体', fid, children: bodyNodes });
    fn.children = cats;

    root.children.push(fn);
  });
  return root;
}

function renderTree() {
  const tree = $('#tree');
  tree.innerHTML = '';
  tree.appendChild(renderNode(buildTree()));
}

function renderNode(node) {
  const spec = KIND[node.kind] || {};
  const wrap = el('div', { class: 'tnode' });
  const head = el('div', { class: 'tnode-head' });

  const lbl = el('span', { class: 'tnode-label' });
  let labelText = node.name;
  if (spec.decorateLabel) labelText += spec.decorateLabel(node, world);
  lbl.textContent = labelText;
  if (node.color) lbl.style.color = node.color;
  if (spec.selectFaction) {
    lbl.classList.add('clickable');
    lbl.addEventListener('click', () => setFaction(node.id));
  }
  head.appendChild(lbl);

  const mt = modeToggleFor(node);
  if (mt) head.appendChild(mt);
  wrap.appendChild(head);

  const hasKids = node.children && node.children.length;

  if (spec.childMode === 'list') {
    const kids = el('div', { class: 'tnode-kids' });
    if (hasKids) node.children.forEach((ch) => kids.appendChild(renderNode(ch)));
    if (node.kind === 'city') kids.appendChild(addBuildingButton(node));
    wrap.appendChild(kids);
    return wrap;
  }

  if (spec.childMode === 'tabs') {
    if (hasKids) {
      const active = selTab.get(node.key) || node.children[0].key;
      const tabs = el('div', { class: 'tnode-tabs' });
      node.children.forEach((ch) => {
        const tab = el('span', { class: 'tnode-tab' + (ch.key === active ? ' sel' : '') });
        tab.textContent = ch.name;
        tab.addEventListener('click', () => {
          selTab.set(node.key, ch.key);
          if ((KIND[ch.kind] || {}).selectFaction) selFaction = ch.id;
          renderTree();
          renderReadout();
        });
        tabs.appendChild(tab);
      });
      wrap.appendChild(tabs);

      const activeChild = node.children.find((ch) => ch.key === active);
      if (activeChild) wrap.appendChild(renderNode(activeChild));
    }
    return wrap;
  }

  if (spec.editor === 'ship') {
    if (node.leaf.mode === 'Player') wrap.appendChild(shipEditor(node.leaf));
  } else if (spec.editor === 'value') {
    wrap.appendChild(leafValueEditor(node.leaf, spec.editorLabel));
  } else if (spec.editor === 'building') {
    wrap.appendChild(buildingEditor(node));
  }
  return wrap;
}

function modeToggleFor(node) {
  const acc = scopeAccess(node);
  if (!acc) return null;
  const mode = acc.get();
  const set = acc.set;

  const sel = el('select', { class: 'mode' });
  [['', '默认'], ['Ai', 'AI'], ['Player', '玩家']].forEach(([v, l]) => {
    const o = el('option', { value: v });
    o.textContent = l;
    o.selected = mode === (v === '' ? null : v);
    sel.appendChild(o);
  });
  sel.addEventListener('change', () => {
    set(sel.value === '' ? null : sel.value);
    renderTree();
  });
  return sel;
}

function shipEditor(leaf) {
  const edit = el('div', { class: 'ship-editor' });
  const t = behaviorType(leaf.behavior);
  const d = behaviorToInput(leaf.behavior);

  const typeSel = el('select');
  [['idle', '待命'], ['move', '移动'], ['follow', '跟随舰'], ['dock_city', '停泊城'], ['dock', '停泊轨道'], ['colonize', '殖民']].forEach(([v, lbl]) => {
    const o = el('option', { value: v }); o.textContent = lbl; o.selected = t === v; typeSel.appendChild(o);
  });
  typeSel.addEventListener('change', () => { leaf.behavior = behaviorFromInput(typeSel.value, d); renderTree(); });
  edit.appendChild(typeSel);

  const bodySel = () => {
    const s = el('select');
    world.bodies.filter((x) => x.settlements && x.settlements.length).forEach((bd) => {
      const o = el('option', { value: bd.name }); o.textContent = bd.name; o.selected = d.body === bd.name; s.appendChild(o);
    });
    s.addEventListener('change', () => { d.body = s.value; leaf.behavior = behaviorFromInput(typeSel.value, d); renderTree(); });
    return s;
  };
  const citySel = () => {
    const s = el('select');
    world.cities.forEach((c) => {
      const o = el('option', { value: c.name }); o.textContent = c.name; o.selected = d.city === c.name; s.appendChild(o);
    });
    s.addEventListener('change', () => { d.city = s.value; leaf.behavior = behaviorFromInput(typeSel.value, d); renderTree(); });
    return s;
  };
  const shipSel = () => {
    const s = el('select');
    world.ships.forEach((sh) => {
      const o = el('option', { value: sh.name }); o.textContent = sh.name; o.selected = d.ship === sh.name; s.appendChild(o);
    });
    s.addEventListener('change', () => { d.ship = s.value; leaf.behavior = behaviorFromInput(typeSel.value, d); renderTree(); });
    return s;
  };

  if (t === 'move') {
    edit.appendChild(inputNum('x', d.x, (v) => { d.x = +v; leaf.behavior = behaviorFromInput(t, d); renderTree(); }));
    edit.appendChild(inputNum('y', d.y, (v) => { d.y = +v; leaf.behavior = behaviorFromInput(t, d); renderTree(); }));
  } else if (t === 'colonize' || t === 'dock') {
    edit.appendChild(bodySel());
  } else if (t === 'follow') {
    edit.appendChild(shipSel());
  } else if (t === 'dock_city') {
    edit.appendChild(citySel());
  }
  return edit;
}

function inputNum(label, val, onSet) {
  const w = el('span');
  w.textContent = label + ' ';
  const inp = el('input', { type: 'number', class: 'num', value: val });
  inp.addEventListener('input', () => onSet(inp.value));
  w.appendChild(inp);
  return w;
}

function leafValueEditor(leaf, label) {
  const wrap = el('div', { class: 'leaf-val' });
  const t = el('span', { class: 'lv-label' });
  t.textContent = label + ' ';
  const inp = el('input', { type: 'number', class: 'num', value: leaf.value, step: '0.1' });
  inp.disabled = leaf.mode !== 'Player';
  inp.addEventListener('input', () => { if (!inp.disabled) leaf.value = +inp.value || 0; });
  wrap.appendChild(t);
  wrap.appendChild(inp);
  return wrap;
}

// --- 建筑编辑器辅助 ---------------------------------------------------------
function optSelect(metaMap, keys, value, onChange) {
  const s = el('select');
  keys.forEach((k) => {
    const o = el('option', { value: k });
    const m = metaMap[k];
    o.textContent = m ? (m.name || m.label || k) : k;
    o.selected = k === value;
    s.appendChild(o);
  });
  s.value = value || keys[0];
  s.addEventListener('change', () => onChange(s.value));
  return s;
}
function labelWrap(label, control) {
  const w = el('span', { class: 'leaf-val' });
  w.appendChild(el('span', { class: 'lv-label' }, label + ' '));
  w.appendChild(control);
  return w;
}
function pushModify(fid, cityId, bid, attrs) {
  const fc = getControl(fid);
  const i = fc.buildings.findIndex((p) => p.building === bid && !p.remove);
  if (i >= 0) fc.buildings[i] = Object.assign({}, fc.buildings[i], attrs, { city: cityId, building: bid });
  else fc.buildings.push(Object.assign({ city: cityId, building: bid }, attrs));
}

function buildingEditor(node) {
  const wrap = el('div', { class: 'ship-editor' });
  const b = node.b;
  const fid = node.fid;
  const cityId = node.cityId;

  wrap.appendChild(labelWrap('结构', optSelect(meta.structures, Object.keys(meta.structures || {}), b.structure, (v) => {
    b.structure = v;
    pushModify(fid, cityId, b.id, { structure: v });
    renderTree();
  })));

  const spec = meta.buildings[b.kind];
  if (spec && spec.role === 'shipyard') {
    wrap.appendChild(labelWrap('舰型', optSelect(meta.ships, Object.keys(meta.ships || {}), b.ship_type, (v) => {
      b.ship_type = v;
      pushModify(fid, cityId, b.id, { ship_type: v });
      renderTree();
    })));
    if (node.buildLeaf) wrap.appendChild(leafValueEditor(node.buildLeaf, '建造权重'));
  }

  if (node.leaf) wrap.appendChild(leafValueEditor(node.leaf, '建设权重'));

  const rm = el('button', { class: 'rm' }, '移除');
  rm.addEventListener('click', () => {
    const fc = getControl(fid);
    fc.buildings.push({ city: cityId, building: b.id, remove: true });
    applyControl();
  });
  wrap.appendChild(rm);
  return wrap;
}

function addBuildingButton(node) {
  const wrap = el('div', { class: 'add-bld' });
  wrap.appendChild(el('span', { class: 'tnode-label', style: 'font-weight:600' }, '+ 新建'));

  const buildKeys = Object.keys(meta.buildings || {});
  const kindSel = optSelect(meta.buildings, buildKeys, 'mining', () => {});
  const structSel = optSelect(meta.structures, Object.keys(meta.structures || {}), 'concrete', () => {});
  const shipSel = optSelect(meta.ships, Object.keys(meta.ships || {}), (Object.keys(meta.ships || {})[0] || 'corvette'), () => {});
  const resSel = optSelect(meta.resources, Object.keys(meta.resources || {}), (Object.keys(meta.resources || {})[0] || 'iron'), () => {});
  const areaInp = el('input', { type: 'number', class: 'num', value: '4', step: '1' });

  wrap.appendChild(kindSel);
  wrap.appendChild(structSel);
  wrap.appendChild(shipSel);
  wrap.appendChild(resSel);
  wrap.appendChild(labelWrap('面积', areaInp));

  const add = el('button', {}, '添加');
  add.addEventListener('click', () => {
    const kind = kindSel.value;
    const fc = getControl(node.fid);
    const patch = { city: node.cityId, building: null, kind, structure: structSel.value, area: +areaInp.value || 4 };
    if (kind === 'mining') patch.resource = resSel.value;
    if (kind === 'construction') patch.ship_type = shipSel.value;
    fc.buildings.push(patch);
    applyControl();
  });
  wrap.appendChild(add);
  return wrap;
}

// --- 读面 / diff ------------------------------------------------------------
function renderReadout() {
  const f = world.factions.find((x) => x.id === selFaction);
  if (f) {
    const res = f.resources.filter(([, v]) => v >= 0.05).map(([k, v]) => (meta.resources[k]?.name || k) + ' ' + (+v).toFixed(1)).join('  ');
    $('#readout').textContent = '[ ' + f.name + ' ] ' + (res || '无资源') + '  交战: ' +
      f.relations.filter(([, v]) => v <= -20).map(([id]) => world.factions.find((x) => x.id === id)?.name).filter(Boolean).join('、') || '无';
  }
}

function renderDiff() {
  const box = $('#diffBar');
  if (!prevState) { box.textContent = ''; return; }
  box.textContent = '上回合: ' + prevState.round + ' → ' + world.round +
    '  舰 ' + prevState.ships.length + '→' + world.ships.length +
    '  城 ' + prevState.cities.length + '→' + world.cities.length;
}

// --- 动作 ------------------------------------------------------------------
async function advance(n) {
  prevState = world;
  world = await postJSON('/api/advance', { n });
  buildEdits();
  renderAll();
  updateTop();
  $('#status').textContent = '已推进 ' + n + ' 回合';
}

async function applyControl() {
  world = await postJSON('/api/command', { control: edControl, scope: edScope });
  buildEdits();
  renderAll();
  $('#status').textContent = '已应用到服务器';
}

async function newGame() {
  const seed = $('#seedInput').value || 'random';
  prevState = null;
  world = await postJSON('/api/new', { seed });
  selFaction = world.factions.length ? world.factions[0].id : 0;
  selTab = new Map();
  if (window.PlanetXMap && window.PlanetXMap.resetView) window.PlanetXMap.resetView(world);
  buildEdits();
  renderAll();
  updateTop();
  $('#status').textContent = '已重建世界 (种子 ' + seed + ')';
}

// --- 接线 -------------------------------------------------------------------
window.addEventListener('DOMContentLoaded', () => {
  $('#advanceBtn').addEventListener('click', () => advance(+$('#advanceN').value));
  $('#newBtn').addEventListener('click', newGame);
  $('#applyBtn').addEventListener('click', applyControl);
  init();
});
