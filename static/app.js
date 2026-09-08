// 行星X WebUI frontend — vanilla JS + SVG. Talks to the JSON API in src/web.rs.
//
// The control panel is a tab-driven hierarchy following the control-scope chain:
//   全局 -> 势力 -> category(舰/资源/天体) -> 天体 -> 城市 -> 建筑
// Every container node shows a tab strip and expands only the selected child.
// Category groups list all their items; global/faction/body/city use tabs.

let world = null;      // current StateView
let meta = null;       // MetaView
let selFaction = 0;    // selected faction id (readout / focus)
let selShip = null;    // selected ship id (readout)
let edControl = [];    // editable copy of ALL factions' control (FactionControlView[])
let edScope = null;    // editable copy of the scope
let prevState = null;  // for a simple diff footer
let selTab = new Map(); // parentKey -> active child key (one tab open per level)

const $ = (sel, root) => (root || document).querySelector(sel);
const el = (tag, attrs, html) => {
  const e = document.createElement(tag);
  if (attrs) for (const k in attrs) e.setAttribute(k, attrs[k]);
  if (html != null) e.innerHTML = html;
  return e;
};
// SVG elements MUST be created in the SVG namespace, otherwise they never
// render. Use this for anything drawn inside <svg>.
const SVGNS = 'http://www.w3.org/2000/svg';
const svgEl = (tag, attrs) => {
  const e = document.createElementNS(SVGNS, tag);
  if (attrs) for (const k in attrs) e.setAttribute(k, attrs[k]);
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

// Faction display color comes from the backend (/api/state -> faction.color),
// which sends a CSS hex string. Fallback grey for safety if unknown.
function facColor(fid) {
  const f = world.factions.find((x) => x.id === fid);
  return (f && f.color) || '#8f9bb3';
}

// --- behavior helpers (serde JSON shape) ----------------------------------
function behaviorType(b) {
  if (typeof b === 'string') return 'idle';
  if (b && b.Move) return 'move';
  if (b && b.TargetShip) return 'ship';
  if (b && b.TargetSettlement) return 'settlement';
  if (b && b.Dock) return 'dock';
  if (b && b.Colonize) return 'colonize';
  return 'idle';
}
function behaviorSummary(b, world) {
  const t = behaviorType(b);
  const shipName = (id) => { const s = world.ships.find((x) => x.id === id); return s ? s.name : '船#' + id; };
  const cityName = (id) => { const c = world.cities.find((x) => x.id === id); return c ? c.name : '城#' + id; };
  switch (t) {
    case 'move': return '移动(' + (b.Move.position[0] | 0) + ',' + (b.Move.position[1] | 0) + ')';
    case 'ship': return '→' + shipName(b.TargetShip.ship) + (b.TargetShip.attack ? '·攻' : '');
    case 'settlement': return '→' + cityName(b.TargetSettlement.city) + (b.TargetSettlement.bombard ? '·轰' : '');
    case 'dock': {
      const bd = world.bodies.find((x) => x.id === b.Dock.body);
      return '→' + (bd ? bd.name : '天体#' + b.Dock.body) + '·停泊';
    }
    case 'colonize': {
      const bd = world.bodies.find((x) => x.id === b.Colonize.body);
      return '→' + (bd ? bd.name : '天体#' + b.Colonize.body) + '·殖民';
    }
    default: return '待命';
  }
}
function behaviorToInput(b) {
  switch (behaviorType(b)) {
    case 'move': return { x: b.Move.position[0], y: b.Move.position[1] };
    case 'ship': return { ship: b.TargetShip.ship, attack: b.TargetShip.attack };
    case 'settlement': return { city: b.TargetSettlement.city, bombard: b.TargetSettlement.bombard };
    case 'colonize': return { body: b.Colonize.body };
    default: return {};
  }
}
function behaviorFromInput(type, d) {
  switch (type) {
    case 'move': return { Move: { position: [+d.x || 0, +d.y || 0] } };
    case 'ship': return { TargetShip: { ship: +d.ship || 0, attack: !!d.attack } };
    case 'settlement': return { TargetSettlement: { city: +d.city || 0, bombard: !!d.bombard } };
    case 'colonize': return { Colonize: { body: +d.body || 0 } };
    default: return 'Idle';
  }
}

// --- node-kind registry ------------------------------------------------------
// Centralises every per-kind behaviour so renderNode/modeToggleFor no longer
// switch on a bare string. To support a new structural level, add one row here
// (plus the matching entry in buildTree). Fields:
//   childMode    'tabs'  -> container: tab strip, one active child expanded
//                'list'  -> category group: all children stacked
//                'leaf'  -> terminal node (no children)
//   scope        source of the AI/玩家 toggle:
//                null        no toggle (pure grouping container)
//                'global'    edScope.global
//                'factions'/'bodies'/'cities'  scopeVal(edScope[k], node.id)
//                'leaf'      node.leaf.mode
//   editor       'ship'  -> ship behavior editor (Player only)
//                'value' -> numeric leaf editor, label = editorLabel
//   editorLabel  label for the 'value' editor
//   selectFaction   clicking the label switches selFaction
//   decorateLabel   fn(node, world) -> suffix appended to the label text
const KIND = {
  global:   { childMode: 'tabs', scope: 'global' },
  faction:  { childMode: 'tabs', scope: 'factions', selectFaction: true },
  group:    { childMode: 'list', scope: null },
  body:     { childMode: 'tabs', scope: 'bodies' },
  city:     { childMode: 'list', scope: 'cities' },
  ship:     { childMode: 'leaf', scope: 'leaf', editor: 'ship', decorateLabel: (n, w) => ' · ' + behaviorSummary(n.leaf.behavior, w) },
  resource: { childMode: 'leaf', scope: 'leaf', editor: 'value', editorLabel: '投资预算/回合' },
  conbudget:{ childMode: 'leaf', scope: 'leaf', editor: 'value', editorLabel: '建造预算/回合' },
  building: { childMode: 'leaf', scope: 'leaf', editor: 'building' },
};

// Resolve the read/write accessors for a node's AI/玩家 toggle, or null if it
// has none (scope: null). Returns {get, set}.
function scopeAccess(node) {
  const spec = KIND[node.kind] || {};
  const s = spec.scope;
  if (s === null || s === undefined) return null;
  if (s === 'leaf') return node.leaf ? { get: () => node.leaf.mode, set: (v) => { node.leaf.mode = v; } } : null;
  if (s === 'global') return { get: () => edScope.global, set: (v) => { edScope.global = v; } };
  // s is a scope list key ('factions' | 'bodies' | 'cities')
  return {
    get: () => scopeVal(edScope[s], node.id),
    set: (v) => setScopeVal(edScope[s], node.id, v),
  };
}

// --- state load / selection -------------------------------------------------
async function init() {
  meta = await fetchJSON('/api/meta');
  prevState = await fetchJSON('/api/state');
  world = await fetchJSON('/api/state');
  selFaction = world.factions.length ? world.factions[0].id : 0;
  buildEdits();
  renderAll();
  updateTop();
}

function buildEdits() {
  edControl = structuredClone(world.control || []);
  edScope = structuredClone(world.scope);
  // Each faction holds a transient list of structural building commands
  // (add / remove / modify). Cleared after every successful apply.
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
    c = {
      faction_id: fid,
      ship_orders: [],
      investment_budget: [],
      construction_budget: [],
      invest_weights: [],
      build_weights: [],
      buildings: [],
    };
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

// Look up a building's controllables within a faction's control view.
function investLeaf(fc, city, bid) {
  return (fc.invest_weights || []).find((e) => e.city === city && e.building === bid);
}
function buildLeaf(fc, city, bid) {
  return (fc.build_weights || []).find((e) => e.city === city && e.building === bid);
}
// The naming for a building node, in the body->city->building story.
function buildingLabel(b) {
  let s = kindName(b.kind);
  if (b.resource) s += '·' + resName(b.resource);
  if (b.ship_type) s += '·' + shipClassName(b.ship_type);
  s += '·' + structName(b.structure);
  s += '×' + (b.deployed || 0).toFixed(1);
  return s;
}

// --- main render ------------------------------------------------------------
function renderAll() {
  renderMap();
  renderTree();
  renderReadout();
  renderDiff();
}

function updateTop() {
  $('#metaRound').textContent = '回合 ' + world.round + ' · ' + world.time_month + ' 个月';
}

// --- map (SVG) --------------------------------------------------------------
function tx(v) { return Math.sign(v) * Math.pow(Math.abs(v), 0.4); }

function renderMap() {
  const svg = $('#map');
  svg.setAttribute('viewBox', '0 0 1000 680');
  svg.innerHTML = '';

  const pts = [];
  world.bodies.forEach((b) => pts.push({ x: tx(b.position[0]), y: tx(b.position[1]), id: b.id, kind: 'body' }));
  world.ships.forEach((s) => pts.push({ x: tx(s.position[0]), y: tx(s.position[1]), id: s.id, kind: 'ship' }));
  pts.push({ x: 0, y: 0, kind: 'sun' });

  let minx = 1e9, maxx = -1e9, miny = 1e9, maxy = -1e9;
  pts.forEach((p) => {
    minx = Math.min(minx, p.x); maxx = Math.max(maxx, p.x);
    miny = Math.min(miny, p.y); maxy = Math.max(maxy, p.y);
  });
  const spanx = Math.max(maxx - minx, 1e-3);
  const spany = Math.max(maxy - miny, 1e-3);
  const pad = 40;
  const X = (v) => pad + ((v - minx) / spanx) * (1000 - 2 * pad);
  const Y = (v) => pad + ((maxy - v) / spany) * (680 - 2 * pad);

  const sun = svgEl('circle', { cx: X(0), cy: Y(0), r: 7, fill: '#fde047', stroke: '#ca8a04' });
  svg.appendChild(sun);
  const sunLbl = svgEl('text', { x: X(0) + 10, y: Y(0) + 4, fill: '#fde047', 'font-size': 11 });
  sunLbl.textContent = '太阳';
  svg.appendChild(sunLbl);

  world.bodies.forEach((b) => {
    const x = X(tx(b.position[0])), y = Y(tx(b.position[1]));
    const c = svgEl('circle', { cx: x, cy: y, r: b.settlement ? 10 : 6, fill: '#334155', stroke: '#22d3ee', 'stroke-width': 1.5 });
    c.setAttribute('data-kind', 'body');
    c.setAttribute('data-ref', b.id);
    c.addEventListener('click', () => { selShip = null; $('#readout').textContent = '天体 ' + b.name; });
    svg.appendChild(c);
    const t = svgEl('text', { x: x + 12, y: y + 4, fill: '#bae6fd', 'font-size': 12, 'font-weight': 600 });
    t.textContent = b.name;
    svg.appendChild(t);
    world.cities.filter((ci) => ci.body_id === b.id).forEach((ci) => {
      const r = svgEl('rect', { x: x - 13, y: y - 13, width: 7, height: 7, fill: facColor(ci.faction_id), stroke: '#0f172a' });
      r.setAttribute('data-kind', 'city');
      r.setAttribute('data-ref', ci.id);
      r.addEventListener('click', () => { selShip = null; $('#readout').textContent = '城市 ' + ci.name; });
      svg.appendChild(r);
    });
  });

  world.ships.forEach((s) => {
    const x = X(tx(s.position[0])), y = Y(tx(s.position[1]));
    const c = svgEl('circle', { cx: x, cy: y, r: 4, fill: facColor(s.faction_id), stroke: '#0f172a', 'stroke-width': 1 });
    c.setAttribute('data-kind', 'ship');
    c.setAttribute('data-ref', s.id);
    c.addEventListener('click', () => {
      selShip = s.id;
      $('#readout').textContent = s.name + ' [' + (world.factions.find((f) => f.id === s.faction_id) || {}).name + '] ' + behaviorSummary(
        (world.control.find((f) => f.faction_id === s.faction_id) || {}).ship_orders.find((o) => o.ship === s.id)?.behavior, world);
    });
    svg.appendChild(c);
  });
}

// --- hierarchy tree (control + scope) --------------------------------------
// Tab drill-down: every container node shows a tab strip of its children and
// only the active child is expanded. 全局 -> 势力 -> category(舰/资源/天体) ->
// 天体 -> 城市 -> 建筑. Legacy openSet/expand arrows are gone.
function buildTree() {
  const root = { key: 'g', kind: 'global', name: '全局', children: [] };
  world.factions.forEach((f) => {
    const fid = f.id;
    const fc = getControl(fid);
    const fn = { key: 'f' + fid, kind: 'faction', id: fid, name: f.name, color: f.color, fid, children: [] };

    // ships owned by this faction (leaves)
    const shipLeaves = (fc.ship_orders || []).map((ord) => {
      const s = world.ships.find((x) => x.id === ord.ship);
      return { key: 'ship' + ord.ship, kind: 'ship', id: ord.ship, name: (s ? s.name : '船#' + ord.ship), leaf: ord, fid };
    });

    // budgets of this faction (投资预算 / 建造预算 leaves)
    const invLeaves = (fc.investment_budget || []).map((e) => ({
      key: 'inv' + fid + ':' + e.resource,
      kind: 'resource',
      name: resName(e.resource),
      leaf: e,
      fid,
    }));
    const conLeaves = (fc.construction_budget || []).map((e) => ({
      key: 'con' + fid + ':' + e.resource,
      kind: 'conbudget',
      name: resName(e.resource),
      leaf: e,
      fid,
    }));
    const budgetKids = [];
    if (invLeaves.length) budgetKids.push({ key: 'gi' + fid, kind: 'group', name: '投资预算', fid, children: invLeaves });
    if (conLeaves.length) budgetKids.push({ key: 'gc' + fid, kind: 'group', name: '建造预算', fid, children: conLeaves });

    // bodies where this faction has cities -> cities -> buildings
    const bodyNodes = [];
    world.cities.filter((c) => c.faction_id === fid).forEach((ci) => {
      const bid = ci.body_id;
      let bn = bodyNodes.find((n) => n.id === bid);
      if (!bn) {
        const b = world.bodies.find((x) => x.id === bid);
        bn = { key: 'b' + fid + '-' + bid, kind: 'body', id: bid, name: (b ? b.name : '天体#' + bid), children: [], fid };
        bodyNodes.push(bn);
      }
      const cn = { key: 'c' + ci.id, kind: 'city', id: ci.id, name: ci.name, children: [], fid, cityId: ci.id };
      (ci.buildings || []).forEach((b) => {
        cn.children.push({
          key: 'bld' + fid + ':' + ci.id + ':' + b.id,
          kind: 'building',
          id: b.id,
          name: buildingLabel(b),
          leaf: investLeaf(fc, ci.id, b.id),
          buildLeaf: buildLeaf(fc, ci.id, b.id),
          b,
          fid,
          cityId: ci.id,
        });
      });
      bn.children.push(cn);
    });

    // category tabs under a faction (only present categories)
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

  // category group: stack ALL its children
  if (spec.childMode === 'list') {
    const kids = el('div', { class: 'tnode-kids' });
    if (hasKids) node.children.forEach((ch) => kids.appendChild(renderNode(ch)));
    if (node.kind === 'city') kids.appendChild(addBuildingButton(node));
    wrap.appendChild(kids);
    return wrap;
  }

  // container node: tab strip of children, only the active one expands
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

  // leaf editors below the header
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
  [['idle', '待命'], ['move', '移动'], ['ship', '攻击舰'], ['settlement', '轰炸城'], ['colonize', '殖民']].forEach(([v, lbl]) => {
    const o = el('option', { value: v }); o.textContent = lbl; o.selected = t === v; typeSel.appendChild(o);
  });
  typeSel.addEventListener('change', () => { leaf.behavior = behaviorFromInput(typeSel.value, d); renderTree(); });
  edit.appendChild(typeSel);

  if (t === 'move') {
    edit.appendChild(inputNum('x', d.x, (v) => { d.x = +v; leaf.behavior = behaviorFromInput(t, d); renderTree(); }));
    edit.appendChild(inputNum('y', d.y, (v) => { d.y = +v; leaf.behavior = behaviorFromInput(t, d); renderTree(); }));
  } else if (t === 'colonize') {
    const bSel = el('select');
    world.bodies.filter((x) => x.settlement_area != null).forEach((bd) => {
      const o = el('option', { value: bd.id }); o.textContent = bd.name; o.selected = d.body === bd.id; bSel.appendChild(o);
    });
    bSel.addEventListener('change', () => { d.body = +bSel.value; leaf.behavior = behaviorFromInput(t, d); renderTree(); });
    edit.appendChild(bSel);
  } else if (t === 'ship') {
    edit.appendChild(inputNum('舰#', d.ship, (v) => { d.ship = +v; leaf.behavior = behaviorFromInput(t, d); renderTree(); }));
    const l = el('label'); l.textContent = '攻';
    const cb = el('input', { type: 'checkbox' }); cb.checked = !!d.attack;
    cb.addEventListener('change', () => { d.attack = cb.checked; leaf.behavior = behaviorFromInput(t, d); renderTree(); });
    l.appendChild(cb); edit.appendChild(l);
  } else if (t === 'settlement') {
    edit.appendChild(inputNum('城#', d.city, (v) => { d.city = +v; leaf.behavior = behaviorFromInput(t, d); renderTree(); }));
    const l = el('label'); l.textContent = '轰';
    const cb = el('input', { type: 'checkbox' }); cb.checked = !!d.bombard;
    cb.addEventListener('change', () => { d.bombard = cb.checked; leaf.behavior = behaviorFromInput(t, d); renderTree(); });
    l.appendChild(cb); edit.appendChild(l);
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

// --- building editor helpers ------------------------------------------------
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
// Queue a structural building modify patch on a faction's control entry.
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

  // structure attribute (混凝土 / 钢结构) — a building's own attribute.
  wrap.appendChild(labelWrap('结构', optSelect(meta.structures, Object.keys(meta.structures || {}), b.structure, (v) => {
    b.structure = v;
    pushModify(fid, cityId, b.id, { structure: v });
    renderTree();
  })));

  // shipyard (建造区): the ship class it produces + its 建造投资权重.
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

// --- readout / diff ---------------------------------------------------------
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

// --- actions ----------------------------------------------------------------
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
  buildEdits();
  renderAll();
  updateTop();
  $('#status').textContent = '已重建世界 (种子 ' + seed + ')';
}

// --- wire up ----------------------------------------------------------------
window.addEventListener('DOMContentLoaded', () => {
  $('#advanceBtn').addEventListener('click', () => advance(+$('#advanceN').value));
  $('#newBtn').addEventListener('click', newGame);
  $('#applyBtn').addEventListener('click', applyControl);
  init();
});
