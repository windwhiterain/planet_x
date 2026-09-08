// 行星X WebUI frontend — vanilla JS + SVG. Talks to the JSON API in src/web.rs.

let world = null;      // current StateView
let meta = null;       // MetaView
let selFaction = 0;    // selected faction id
let selShip = null;    // selected ship id (readout)
let edControl = null;  // editable copy of the selected faction's control
let edScope = null;    // editable copy of the scope
let prevState = null;  // for a simple diff footer

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

// Faction marker colors (the model stores a char, not a color).
const PALETTE = [
  '#3b82f6', '#06b6d4', '#8b5cf6', '#ef4444', '#ec4899',
  '#eab308', '#22c55e', '#f8fafc', '#d946ef', '#fb923c',
];
function fracColor(fid) { return PALETTE[fid % PALETTE.length]; }

// --- behavior helpers (serde JSON shape) ----------------------------------
function behaviorType(b) {
  if (typeof b === 'string') return 'idle';
  if (b && b.Move) return 'move';
  if (b && b.TargetShip) return 'ship';
  if (b && b.TargetSettlement) return 'settlement';
  return 'idle';
}
function behaviorSummary(b, world) {
  const t = behaviorType(b);
  const shipName = (id) => {
    const s = world.ships.find((x) => x.id === id);
    return s ? s.name : '船#' + id;
  };
  const cityName = (id) => {
    const c = world.cities.find((x) => x.id === id);
    return c ? c.name : '城#' + id;
  };
  switch (t) {
    case 'move': return '移动(' + (b.Move.position[0] | 0) + ',' + (b.Move.position[1] | 0) + ')';
    case 'ship': return '→' + shipName(b.TargetShip.ship) + (b.TargetShip.attack ? '·攻' : '');
    case 'settlement': return '→' + cityName(b.TargetSettlement.city) + (b.TargetSettlement.bombard ? '·轰' : '');
    default: return '待命';
  }
}
function behaviorToInput(b) {
  switch (behaviorType(b)) {
    case 'move': return { x: b.Move.position[0], y: b.Move.position[1] };
    case 'ship': return { ship: b.TargetShip.ship, attack: b.TargetShip.attack };
    case 'settlement': return { city: b.TargetSettlement.city, bombard: b.TargetSettlement.bombard };
    default: return {};
  }
}
function behaviorFromInput(type, d) {
  switch (type) {
    case 'move': return { Move: { position: [+d.x || 0, +d.y || 0] } };
    case 'ship': return { TargetShip: { ship: +d.ship || 0, attack: !!d.attack } };
    case 'settlement': return { TargetSettlement: { city: +d.city || 0, bombard: !!d.bombard } };
    default: return 'Idle';
  }
}
function behaviorTypeName(t) {
  return { idle: '待命', move: '移动', ship: '攻击舰', settlement: '轰炸城' }[t] || '待命';
}

function modeName(m) {
  return m === 'Ai' ? 'AI' : m === 'Player' ? '玩家' : '默认';
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
  const c = world.control.find((x) => x.faction_id === selFaction);
  edControl = structuredClone(c || { faction_id: selFaction, ship_orders: [], budget: [], invest_weights: [] });
  edScope = structuredClone(world.scope);
  // Ensure ship_orders of this faction also include ships with no order (they
  // will be whatever the sim wrote). The control list already reflects them.
}

function setFaction(fid) {
  if (fid === selFaction) return;
  selFaction = fid;
  buildEdits();
  renderAll();
}

// --- main render ------------------------------------------------------------
function renderAll() {
  renderMap();
  renderFactionBar();
  renderControl();
  renderScope();
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

  // gather projected points
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

  // ordering: sun behind, then bodies, then ships on top
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
    // cities on this body
    world.cities.filter((ci) => ci.body_id === b.id).forEach((ci) => {
      const fc = fracColor(ci.faction_id);
      const r = svgEl('rect', { x: x - 13, y: y - 13, width: 7, height: 7, fill: fc, stroke: '#0f172a' });
      r.setAttribute('data-kind', 'city');
      r.setAttribute('data-ref', ci.id);
      r.addEventListener('click', () => { selShip = null; $('#readout').textContent = '城市 ' + ci.name; });
      svg.appendChild(r);
    });
  });

  world.ships.forEach((s) => {
    const x = X(tx(s.position[0])), y = Y(tx(s.position[1]));
    const c = svgEl('circle', { cx: x, cy: y, r: 4, fill: fracColor(s.faction_id), stroke: '#0f172a', 'stroke-width': 1 });
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

// --- faction bar ------------------------------------------------------------
function renderFactionBar() {
  const bar = $('#factionBar');
  bar.innerHTML = '';
  world.factions.forEach((f) => {
    const b = el('button', { class: 'fac-btn' + (f.id === selFaction ? ' sel' : '') });
    b.textContent = f.name;
    b.style.color = fracColor(f.id);
    b.addEventListener('click', () => setFaction(f.id));
    bar.appendChild(b);
  });
}

// --- control editor ---------------------------------------------------------
function renderControl() {
  renderShips();
  renderBudget();
  renderInvest();
}

function modeSelect(leaf, onSet, key) {
  const sel = el('select');
  [['', '默认'], ['Ai', 'AI'], ['Player', '玩家']].forEach(([v, lbl]) => {
    const o = el('option', { value: v });
    o.textContent = lbl;
    o.selected = leaf.mode === (v === '' ? null : v);
    sel.appendChild(o);
  });
  sel.addEventListener('change', () => {
    leaf.mode = sel.value === '' ? null : sel.value;
    onSet(key, leaf.mode);
  });
  return sel;
}

function renderShips() {
  const box = $('#shipList');
  box.innerHTML = '';
  const ships = edControl.ship_orders;
  if (!ships.length) { box.textContent = '该势力暂无飞船'; return; }
  ships.forEach((ord, i) => {
    const info = world.ships.find((s) => s.id === ord.ship);

    // Header row: ship name + behavior summary on the left, mode toggle right.
    const row = el('div', { class: 'row' });
    const label = el('div', { class: 'label' });
    label.textContent = (info ? info.name : '船#' + ord.ship) + ' · ' + behaviorSummary(ord.behavior, world);
    row.appendChild(label);
    const modeSel = modeSelect(ord, () => renderShips(), i);
    row.appendChild(modeSel);
    box.appendChild(row);

    // Behavior editor goes on its OWN line (full width) so it never squeezes
    // the mode toggle out of view.
    if (ord.mode === 'Player') {
      const edit = el('div', { class: 'ship-editor' });
      const t = behaviorType(ord.behavior);
      const d = behaviorToInput(ord.behavior);

      const typeSel = el('select');
      [['idle', '待命'], ['move', '移动'], ['ship', '攻击舰'], ['settlement', '轰炸城']].forEach(([v, lbl]) => {
        const o = el('option', { value: v }); o.textContent = lbl; o.selected = t === v; typeSel.appendChild(o);
      });
      typeSel.addEventListener('change', () => {
        ord.behavior = behaviorFromInput(typeSel.value, d);
        renderShips();
      });
      edit.appendChild(typeSel);

      if (t === 'move') {
        edit.appendChild(inputNum('x', d.x, (v) => { d.x = v; ord.behavior = behaviorFromInput(t, d); }));
        edit.appendChild(inputNum('y', d.y, (v) => { d.y = v; ord.behavior = behaviorFromInput(t, d); }));
      } else if (t === 'ship') {
        edit.appendChild(inputNum('舰#', d.ship, (v) => { d.ship = v; ord.behavior = behaviorFromInput(t, d); }));
        const atk = el('label'); atk.textContent = '攻';
        const cb = el('input', { type: 'checkbox' }); cb.checked = !!d.attack;
        cb.addEventListener('change', () => { d.attack = cb.checked; ord.behavior = behaviorFromInput(t, d); });
        atk.appendChild(cb); edit.appendChild(atk);
      } else if (t === 'settlement') {
        edit.appendChild(inputNum('城#', d.city, (v) => { d.city = v; ord.behavior = behaviorFromInput(t, d); }));
        const bomb = el('label'); bomb.textContent = '轰';
        const cb = el('input', { type: 'checkbox' }); cb.checked = !!d.bombard;
        cb.addEventListener('change', () => { d.bombard = cb.checked; ord.behavior = behaviorFromInput(t, d); });
        bomb.appendChild(cb); edit.appendChild(bomb);
      }
      box.appendChild(edit);
    }
  });
}

function inputNum(label, val, onSet) {
  const w = el('span');
  w.textContent = label + ' ';
  const inp = el('input', { type: 'number', class: 'num', value: val });
  inp.addEventListener('input', () => onSet(inp.value));
  w.appendChild(inp);
  return w;
}

function renderBudget() {
  const box = $('#budgetList');
  box.innerHTML = '';
  const list = edControl.budget;
  if (!list.length) { box.textContent = '无预算项'; return; }
  list.forEach((e, i) => {
    const row = el('div', { class: 'row' });
    const lbl = el('div', { class: 'label' });
    lbl.textContent = meta.resources[e.resource] ? meta.resources[e.resource].name : e.resource;
    row.appendChild(lbl);
    const inp = el('input', { type: 'number', class: 'num', value: e.value });
    inp.addEventListener('input', () => { e.value = +inp.value || 0; });
    row.appendChild(inp);
    row.appendChild(modeSelect(e, () => renderBudget(), i));
    box.appendChild(row);
  });
}

function renderInvest() {
  const box = $('#investList');
  box.innerHTML = '';
  const list = edControl.invest_weights;
  if (!list.length) { box.textContent = '无建筑权重'; return; }
  list.forEach((e, i) => {
    const row = el('div', { class: 'row' });
    const lbl = el('div', { class: 'label' });
    const city = world.cities.find((c) => c.id === e.city);
    const name = (city ? city.name : '城#' + e.city) + '·' + (e.kind in meta.buildings ? meta.buildings[e.kind].label : e.kind) + (e.resource ? '·' + (meta.resources[e.resource]?.name || e.resource) : '');
    lbl.textContent = name;
    row.appendChild(lbl);
    const inp = el('input', { type: 'number', class: 'num', value: e.value, step: '0.1' });
    inp.addEventListener('input', () => { e.value = +inp.value || 0; });
    row.appendChild(inp);
    row.appendChild(modeSelect(e, () => renderInvest(), i));
    box.appendChild(row);
  });
}

// --- scope editor -----------------------------------------------------------
function scopeModeSelect(obj, key) {
  const cur = obj[key];
  const sel = el('select');
  [['', '默认'], ['Ai', 'AI'], ['Player', '玩家']].forEach(([v, lbl]) => {
    const o = el('option', { value: v }); o.textContent = lbl; o.selected = cur === (v === '' ? null : v); sel.appendChild(o);
  });
  sel.addEventListener('change', () => { obj[key] = sel.value === '' ? null : sel.value; });
  return sel;
}

function renderScope() {
  const box = $('#scopeList');
  box.innerHTML = '';

  // global
  let row = el('div', { class: 'row' });
  const gl = el('div', { class: 'label' }); gl.textContent = '全局'; row.appendChild(gl);
  row.appendChild(scopeModeSelect(edScope, 'global'));
  box.appendChild(row);

  // faction-level scope modes (only the selected faction for compactness)
  const fac = el('div', { class: 'row' });
  const fl = el('div', { class: 'label' }); fl.textContent = '本势力(' + (world.factions.find((f) => f.id === selFaction) || {}).name + ')'; fac.appendChild(fl);
  fac.appendChild(scopeMapSelect(edScope.factions, selFaction));
  box.appendChild(fac);

  // bodies / cities with a marker
  world.bodies.forEach((b) => {
    const has = edScope.bodies.some(([id]) => id === b.id);
    const r = el('div', { class: 'row' });
    const l = el('div', { class: 'label' }); l.textContent = '天体·' + b.name; r.appendChild(l);
    r.appendChild(scopeMapSelect(edScope.bodies, b.id, has));
    box.appendChild(r);
  });
  world.cities.forEach((c) => {
    const has = edScope.cities.some(([id]) => id === c.id);
    const r = el('div', { class: 'row' });
    const l = el('div', { class: 'label' }); l.textContent = '城市·' + c.name; r.appendChild(l);
    r.appendChild(scopeMapSelect(edScope.cities, c.id, has));
    box.appendChild(r);
  });
}

function scopeMapSelect(list, id, has) {
  const entry = list.find(([k]) => k === id);
  const cur = entry ? entry[1] : null;
  const sel = el('select');
  [['', '默认'], ['Ai', 'AI'], ['Player', '玩家']].forEach(([v, lbl]) => {
    const o = el('option', { value: v }); o.textContent = lbl; o.selected = cur === (v === '' ? null : v); sel.appendChild(o);
  });
  sel.addEventListener('change', () => {
    const i = list.findIndex(([k]) => k === id);
    const val = sel.value === '' ? null : sel.value;
    if (i >= 0) list[i] = [id, val]; else list.push([id, val]);
  });
  return sel;
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
  // Build a small textual diff of round / control counts for feedback.
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
  world = await postJSON('/api/command', { faction_id: selFaction, control: edControl, scope: edScope });
  buildEdits();
  renderAll();
  $('#status').textContent = '已应用到服务器';
}

async function newGame() {
  const seed = $('#seedInput').value || 'random';
  prevState = null;
  world = await postJSON('/api/new', { seed });
  selFaction = world.factions.length ? world.factions[0].id : 0;
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
