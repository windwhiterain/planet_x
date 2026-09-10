// 行星X WebUI 前端 — 经典脚本（控制面板/DOM）+ three.js 3D 地图（map3d.js，ES module）。
//
// 控制面板是一个沿控制作用域链下钻的 tab 树：
//   全局 -> 势力 -> 分类(舰/预算/天体) -> 天体 -> 城市 -> 建筑
// 每个容器节点显示一条 tab 带、只展开选中的子节点；分类组列出全部子项。
// 地图由 map3d.js 渲染 WebGL 场景，通过 window.PlanetXMap.setWorld/onSelect 与这里耦合。
//
// 两个读面，分工明确：
//   * 左侧「控制层级」= **可控 state**（可编辑的控制面：舰指令/预算/权重/迁都，写回服务器）。
//   * 右侧「状态」    = **普通 state + 派生 + 配置**（只读全量）。
//     它把后端给的 `world.info`（每个根 = 模型的整份 JSON dump）交给 jsonview.js 那个
//     schema-agnostic widget 渲染：本文件不写任何字段名，只决定「渲染哪个根」，
//     所以 State/GameConfig/Derived 怎么改都不用动前端。

let world = null;      // 当前 StateView（/api/state）= { control, scope, info }
let st = null;         // info 的 `state` 根：规范世界的**原始** State（天体/定居点/城/势力/舰/…）
let cfg = null;        // info 的 `config` 根：game.ron 的全部调参表（资源/结构/建筑/舰/天体类型/剧情…）
let selFaction = '';   // 选中势力（读面/聚焦）
let sel = null;        // 选中对象 { kind, name }——底部读面用通用 widget 渲染它的完整记录
let edControl = [];    // 所有势力的可编辑控制（FactionControlView[]）
let edScope = null;    // 可编辑作用域
let prevState = null;  // 上一帧，用于 diff 页脚
let selTab = new Map(); // parentKey -> 激活子 key（每层只开一个 tab）
let infoTab = 0;       // 右侧面板当前显示的根（world.info 的下标）
let infoFilter = '';   // 右侧面板的过滤串
const infoExpanded = new Set(); // 右侧面板的展开状态（路径集合，跨渲染保留）
const selExpanded = new Set();  // 底部读面的展开状态

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

// 势力显示色来自 state 根的原始 Faction（唯一键 = name）。未知回落灰。
function facColor(fid) {
  const f = (st.factions || []).find((x) => x.name === fid);
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
// 服务器只给两样东西：写面（control/scope 的可编辑模板）与读面（info：模型的整份 dump）。
// 这里把 info 的根绑成 `st`（规范世界）与 `cfg`（配置表）——**读面的一切都从这两个根取**，
// 后端不再有「给前端拍平一份」的字段，所以模型加字段前端自动就能读到。
function infoValueOf(w, name) {
  const r = ((w && w.info) || []).find((x) => x.name === name);
  return (r && r.value) || {};
}
function stateOf(w) { return infoValueOf(w, 'state'); }
function bindWorld(w) {
  world = w;
  st = stateOf(w);
  cfg = infoValueOf(w, 'config');
}

async function init() {
  bindWorld(await fetchJSON('/api/state'));
  prevState = world;
  selFaction = st.factions && st.factions.length ? st.factions[0].name : '';
  sel = selFaction ? { kind: 'faction', name: selFaction } : null;
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
  sel = { kind: 'faction', name: fid };
  openBottomBar();
  if (fid === selFaction) { renderSelection(); return; }
  selFaction = fid;
  selTab.set('g', 'f' + fid);
  renderTree();
  renderSelection();
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

// 显示名一律从 **config 根**取（以前是 /api/meta——那份 View 与 config 完全同构，纯冗余）。
function resName(r) { return cfg.resources && cfg.resources[r] ? cfg.resources[r].name : r; }
function structName(s) { return cfg.structures && cfg.structures[s] ? cfg.structures[s].name : s; }
function kindName(k) { return cfg.buildings && cfg.buildings[k] ? cfg.buildings[k].label : k; }
function shipClassName(c) { return cfg.ships && cfg.ships[c] ? cfg.ships[c].label : c; }

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
  renderSelection();
  renderDiff();
  renderInfo();
}

function updateTop() {
  $('#metaRound').textContent = '回合 ' + st.round + ' · ' + st.time_month + ' 个月';
}

// 地图：把**从 state 根适配出来**的世界交给 three.js 场景（map3d.js），并把 config 的天体
// 类型表（body_kinds）一并给它，供它按 body.kind 解析视觉（颜色/尺寸/着色器分支）。
//
// map3d 的接口是既定的 `{bodies, cities, ships, factions}`，其中势力按 `id` 查颜色；原始
// `Faction` 的唯一键是 `name`（`id` 已废弃）。所以这里只补一个 `id` **别名**，其余字段原样
// 透传（`Object.assign` 保留 relations/resources/ideology…，地图将来要用就有）。
function mapWorld() {
  return Object.assign({}, st, {
    factions: (st.factions || []).map((f) => Object.assign({}, f, { id: f.name })),
  });
}
function renderMap() {
  if (window.PlanetXMap && window.PlanetXMap.setWorld) window.PlanetXMap.setWorld(mapWorld(), cfg.body_kinds);
}

// 地图点击 → 选中该对象（底部读面用通用 widget 渲染它的完整记录）。
function mapSelect(s) {
  selectEntity(s.kind, s.name);
}

// --- 选中读面（generic widget）---------------------------------------------
// 「选中某个对象」= 在 state 根里按名字定位它，然后把**它的整份记录**交给同一个
// schema-agnostic widget 渲染。所以读面里没有一行「读某个字段」的代码：舰的 hull/
// shield/components/doctrine/attack_hist、势力的 resources/relations/ideology…
// 全部自动出现，模型加字段这里不用改。
//
// 定位规则是通用的：在 state 根的顶层**数组**里找 `name` 相等的元素。`KIND_ARRAY` 只是
// 一个「这类对象通常住在哪个数组」的最小提示（地图点击给的是 kind），找不到就扫全部数组。
const KIND_ARRAY = { faction: 'factions', ship: 'ships', body: 'bodies', city: 'cities' };
const KIND_LABEL = { faction: '势力', ship: '舰', body: '天体', city: '城' };

function findEntity(kind, name) {
  const keys = Object.keys(st).filter((k) => Array.isArray(st[k]));
  const prefer = KIND_ARRAY[kind];
  const order = prefer && keys.includes(prefer) ? [prefer].concat(keys.filter((k) => k !== prefer)) : keys;
  for (const k of order) {
    const i = st[k].findIndex((e) => e && typeof e === 'object' && e.name === name);
    if (i >= 0) return { path: 'state.' + k + '[' + i + ']', value: st[k][i] };
  }
  return null;
}

function selectEntity(kind, name) {
  sel = { kind, name };
  openBottomBar(); // 用户点了东西 → 把读面亮出来，否则「点了没反应」
  if (kind === 'faction') { setFaction(name); return; }
  renderSelection();
}

// 展开底部读面（与边缘 bar 的收起/展开共用同一套类名与箭头）。
function openBottomBar() {
  const panel = $('#diffBar');
  const btn = $('#toggleDiff');
  if (!panel || panel.classList.contains('open')) return;
  panel.classList.add('open');
  if (btn) btn.textContent = btn.dataset.openArrow;
}

function renderSelection() {
  const head = $('#selHead');
  const box = $('#readout');
  if (!head || !box) return;
  head.textContent = '';
  if (!sel) {
    box.textContent = '（在地图上点天体/城/舰，或点左侧控制树里的势力，这里会显示它的完整记录）';
    return;
  }
  const found = findEntity(sel.kind, sel.name);
  if (!found) {
    head.textContent = (KIND_LABEL[sel.kind] || sel.kind) + ' ' + sel.name + '（当前世界里已不存在）';
    box.textContent = '';
    return;
  }
  const label = el('span', 'sel-kind', KIND_LABEL[sel.kind] || sel.kind);
  const nameEl = el('span', 'sel-name clickable', sel.name);
  nameEl.title = '点击复制 JSON 路径';
  nameEl.addEventListener('click', () => copyPath(found.path));
  head.append(label, nameEl, el('span', 'sel-path', found.path));
  window.JsonView.render(box, found.value, {
    rootPath: found.path,
    expandDepth: 1,
    state: { expanded: selExpanded },
    onPathClick: copyPath,
  });
}

// --- 层级树（控制 + 作用域）-----------------------------------------------
// 树本身是**写面**（控制作用域链上的可编辑叶子），所以它按语义搭结构；但「世界里有什么
// 实体」一律从 `st`（原始 State）读，不再依赖后端的拍平视图。
function buildTree() {
  const root = { key: 'g', kind: 'global', name: '全局', children: [] };
  st.factions.forEach((f) => {
    const fid = f.name;
    const fc = getControl(fid);
    const fn = { key: 'f' + fid, kind: 'faction', id: fid, name: f.name, color: f.color, fid, children: [] };

    const shipLeaves = (fc.ship_orders || []).map((ord) => {
      const s = st.ships.find((x) => x.name === ord.ship);
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
    st.cities.filter((c) => c.faction_id === fid).forEach((ci) => {
      const bid = ci.body_id;
      let bn = bodyNodes.find((n) => n.id === bid);
      if (!bn) {
        const b = st.bodies.find((x) => x.name === bid);
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
  if (spec.decorateLabel) labelText += spec.decorateLabel(node, st);
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
          // 势力 tab 是「选中这个势力」——走统一的选中入口（会亮出底部读面）。
          if ((KIND[ch.kind] || {}).selectFaction) { selectEntity('faction', ch.id); return; }
          renderTree();
          renderSelection();
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
    st.bodies.filter((x) => x.settlements && x.settlements.length).forEach((bd) => {
      const o = el('option', { value: bd.name }); o.textContent = bd.name; o.selected = d.body === bd.name; s.appendChild(o);
    });
    s.addEventListener('change', () => { d.body = s.value; leaf.behavior = behaviorFromInput(typeSel.value, d); renderTree(); });
    return s;
  };
  const citySel = () => {
    const s = el('select');
    st.cities.forEach((c) => {
      const o = el('option', { value: c.name }); o.textContent = c.name; o.selected = d.city === c.name; s.appendChild(o);
    });
    s.addEventListener('change', () => { d.city = s.value; leaf.behavior = behaviorFromInput(typeSel.value, d); renderTree(); });
    return s;
  };
  const shipSel = () => {
    const s = el('select');
    st.ships.forEach((sh) => {
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
    o.textContent = m ? (m.name || m.label || k) : k; // 显示名优先 name，其次 label（与配置表同构）
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

  wrap.appendChild(labelWrap('结构', optSelect(cfg.structures, Object.keys(cfg.structures || {}), b.structure, (v) => {
    b.structure = v;
    pushModify(fid, cityId, b.id, { structure: v });
    renderTree();
  })));

  const spec = cfg.buildings[b.kind];
  if (spec && spec.role === 'shipyard') {
    wrap.appendChild(labelWrap('舰型', optSelect(cfg.ships, Object.keys(cfg.ships || {}), b.ship_type, (v) => {
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

  const buildKeys = Object.keys(cfg.buildings || {});
  const kindSel = optSelect(cfg.buildings, buildKeys, 'mining', () => {});
  const structSel = optSelect(cfg.structures, Object.keys(cfg.structures || {}), 'concrete', () => {});
  const shipSel = optSelect(cfg.ships, Object.keys(cfg.ships || {}), (Object.keys(cfg.ships || {})[0] || 'corvette'), () => {});
  const resSel = optSelect(cfg.resources, Object.keys(cfg.resources || {}), (Object.keys(cfg.resources || {})[0] || 'iron'), () => {});
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

// --- 右侧「状态」面板（普通 state 全量读数） -------------------------------
// 后端把模型的**整份 dump** 放在 world.info（[{name, value}]）。这里只做三件事：
// 列 root tab、把选中的 root 交给 JsonView 渲染、接上过滤/展开状态。
// 全程**没有任何字段名**——state 加字段/改结构，这里一行都不用动。
function renderInfo() {
  const roots = (world && world.info) || [];
  const tabs = $('#jvTabs');
  const body = $('#jvBody');
  if (!tabs || !body) return;
  tabs.textContent = '';
  if (!roots.length) {
    body.textContent = '（服务器没有返回信息树）';
    return;
  }
  if (infoTab >= roots.length) infoTab = 0;
  roots.forEach((r, i) => {
    const t = el('span', { class: 'tnode-tab' + (i === infoTab ? ' sel' : '') });
    t.textContent = r.name;
    t.addEventListener('click', () => { infoTab = i; renderInfo(); });
    tabs.appendChild(t);
  });
  const root = roots[infoTab];
  window.JsonView.render(body, root.value, {
    rootPath: root.name,
    expandDepth: 1,
    state: { expanded: infoExpanded },
    filter: infoFilter,
    onPathClick: copyPath,
  });
}

// 点叶子复制它的 JSON 路径（如 `state.cities[3].loyalty`）：agent/CLI 与 UI 用同一套定位。
function copyPath(path) {
  if (navigator.clipboard) navigator.clipboard.writeText(path).catch(() => {});
  const s = $('#status');
  if (s) s.textContent = '路径已复制: ' + path;
}

// 展开/收起**当前根**的全部可折叠节点（路径由 widget 自己枚举，结构无关）。
function setAllOpen(open) {
  const root = ((world && world.info) || [])[infoTab];
  if (!root) return;
  window.JsonView.expandablePaths(root.value, root.name)
    .forEach((p) => window.JsonView.setOpen(infoExpanded, p, open));
  renderInfo();
}

let infoFilterTimer = null;
function onInfoFilter(v) {
  infoFilter = v;
  clearTimeout(infoFilterTimer);
  infoFilterTimer = setTimeout(renderInfo, 120); // 大树上防抖
}

// --- 回合 diff --------------------------------------------------------------
// 以前这里也负责「读面」（手写一行「势力：资源… 交战…」）。现在读面是选中对象的通用
// widget 渲染（见 renderSelection），这里只留回合间的**计数对比**——它比的是两帧的
// state 根，仍然是结构无关的（数数组长度）。
function renderDiff() {
  const box = $('#diffText');
  if (!box) return;
  if (!prevState) { box.textContent = ''; return; }
  const a = stateOf(prevState);
  const counts = (s) => Object.keys(s).filter((k) => Array.isArray(s[k])).map((k) => k + ' ' + s[k].length);
  const prev = Object.fromEntries(counts(a).map((x) => x.split(' ')));
  const now = Object.fromEntries(counts(st).map((x) => x.split(' ')));
  const parts = Object.keys(now)
    .filter((k) => prev[k] !== undefined && prev[k] !== now[k])
    .map((k) => k + ' ' + prev[k] + '→' + now[k]);
  box.textContent = '上回合: ' + a.round + ' → ' + st.round + (parts.length ? '  ' + parts.join('  ') : '  （数组计数无变化）');
}

// --- 边缘 bar 展开/收起 ----------------------------------------------------
function toggleBar(btnId, panelId) {
  const btn = $(btnId);
  const panel = $(panelId);
  if (!btn || !panel) return;
  btn.addEventListener('click', () => {
    panel.classList.toggle('open');
    // 小箭头方向随开/收翻转。
    btn.textContent = panel.classList.contains('open') ? btn.dataset.openArrow : btn.dataset.closedArrow;
  });
}

// --- 动作 ------------------------------------------------------------------
async function advance(n) {
  prevState = world;
  bindWorld(await postJSON('/api/advance', { n }));
  buildEdits();
  renderAll();
  updateTop();
  $('#status').textContent = '已推进 ' + n + ' 回合';
}

async function applyControl() {
  bindWorld(await postJSON('/api/command', { control: edControl, scope: edScope }));
  buildEdits();
  renderAll();
  $('#status').textContent = '已应用到服务器';
}

async function newGame() {
  const seed = $('#seedInput').value || 'random';
  prevState = null;
  bindWorld(await postJSON('/api/new', { seed }));
  selFaction = st.factions && st.factions.length ? st.factions[0].name : '';
  sel = selFaction ? { kind: 'faction', name: selFaction } : null;
  selTab = new Map();
  infoTab = 0;
  infoExpanded.clear(); // 新世界 → 展开状态重来
  selExpanded.clear();
  if (window.PlanetXMap && window.PlanetXMap.resetView) window.PlanetXMap.resetView(mapWorld());
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
  // 边缘 bar 手动展开/收起（默认收起，地图全屏）。
  toggleBar('#toggleTop', '#topbar');
  toggleBar('#toggleSide', '#side');
  toggleBar('#toggleDiff', '#diffBar');
  toggleBar('#toggleInfo', '#info');
  // 右侧状态面板：过滤 + 全展开/全收起（结构无关，全部走 widget 的通用 API）。
  $('#jvFilter').addEventListener('input', (e) => onInfoFilter(e.target.value));
  $('#jvExpand').addEventListener('click', () => setAllOpen(true));
  $('#jvCollapse').addEventListener('click', () => setAllOpen(false));
  init();
});
