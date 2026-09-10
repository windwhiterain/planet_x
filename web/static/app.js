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
let baseControl = [];  // 载入时的读面快照（回传 diff 的基准，见 buildCommandDiff）
let baseScope = null;  // 同上，作用域那一段
let prevState = null;  // 上一帧，用于 diff 页脚
let selTab = new Map(); // parentKey -> 激活子 key（每层只开一个 tab）
let infoTab = 0;       // 右侧面板当前显示的根（world.info 的下标）
let infoFilter = '';   // 右侧面板的过滤串
const infoExpanded = new Set(); // 右侧面板的展开状态（路径集合，跨渲染保留）
const selExpanded = new Set();  // 底部读面的展开状态
const leafOrigin = new Map();   // 编辑面里的叶 -> 原值（回传 diff 的基准，见 buildCommandDiff）
let autoPinned = new WeakSet(); // 「写值即接管」钉成 Player 的叶（值改回原数时撤回，见 wroteValue）

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

// --- 风格辅助（行为风格两轴 + 风筝<->贴脸姿态） -----------------------------
// 三个轴都取 [-1,1]、0 = 基线（引擎 `ShipDoctrine` / `Ship.kiting` 的约定）：
//   temper     理智↔热血：<0 欺软怕硬（打威慑低于自己的），>0 飞蛾扑火（打威慑高于自己的）
//   lone_wolf  护航↔独狼：<0 空闲时贴旗舰护航，>0 空闲时自行就近接战
//   kiting     风筝↔贴脸：<0 风筝（保持最远武器射程、敌近则拉开、更早撤），>0 贴脸（压近、打得更久）
function num2(v) {
  const n = +v || 0;
  return (n >= 0 ? '+' : '') + n.toFixed(2);
}
function doctrineSummary(l) { return '理智↔热血 ' + num2(l.temper) + ' · 护航↔独狼 ' + num2(l.lone_wolf); }
function kitingSummary(l) { return '风筝↔贴脸 ' + num2(l.kiting); }
// 第三条风格轴**角色**（运输舰↔战舰）不是 [-1,1] 的连续轴，而是**一个开关**：它只决定
// 自动控制派哪种活（`autocontrol::freight` 按积压定编谁去跑集货路线），不解除武装
// ——运输舰在射程内照样自动开火、照样按 kiting 姿态软移动。
function freighterSummary(l) { return l.freighter ? '运输舰' : '战舰'; }

// 风格轴上的「自动」大都是**空头承诺**：全仓没有一处生产代码写风格叶（`ship_doctrine`/
// `ship_kiting` 的唯一写入者就是玩家/agent），所以系统**不会**来重估它。对照：指令轴的
// `Auto` 是真的有执行者（AI 每回合往 `ship_orders` 写叶，所以那里照旧说「值由系统写」）。
// 措辞必须分开——照抄「值由系统写」在风格轴上是假话（note：control-live-layers §3.2）。
//
// ⚠ 唯一的例外是**逐舰角色叶**：`ship_freighter` 是自动控制每回合真的会写的叶（按积压定编），
// 所以它的「自动」名副其实——它拿的正是 `ship_orders` 那套措辞，**不在这里**。
// 而**势力级默认角色**（`default_freighter`）依旧是空头承诺：AI 只写逐舰叶，不写这片默认叶。
const AUTO_FROZEN = '自动（本轴暂无重估者：值冻结）';   // 逐舰风格叶：值在用，但没人会改它
const AUTO_NO_VALUE = '自动（本轴暂无重估者：不给值）'; // 势力级默认叶：引擎只在它是玩家时供值
const AUTO_WRITTEN = '自动（自动控制每回合按积压定编，会改写这片叶）'; // 角色轴逐舰叶：真有执行者
/// 「自动」在这片叶上是不是**空头承诺**（没有执行者）。只有它没执行者时才敢说"值冻结"。
const AUTO_FROZEN_KINDS = ['shipdoctrine', 'fleetdoctrine', 'shipkiting', 'fleetkiting', 'fleetfreighter'];

// 势力级**默认风格**行的摘要：只有它自己是「玩家」时那个值才真的被采用（引擎的取值规则：
// 默认叶为 `Inherit`/`Auto` 时不供值），所以这两种情况都不显示那几个数——显示了会骗人。
function fleetStyleLabel(leaf, summary) {
  const m = normMode(leaf.mode);
  if (m === 'Player') return ' · ' + summary(leaf);
  return m === 'Auto' ? ' · ' + AUTO_NO_VALUE : ' · 未表态';
}

/// 逐舰风格叶的 `Auto` 补注：值是**在用**的（叶片自己的值优先），但没人会来重估它。
function autoFrozen(leaf) { return normMode(leaf.mode) === 'Auto' ? ' · ' + AUTO_FROZEN : ''; }
/// 角色轴逐舰叶的 `Auto` 补注：这片叶**真的有执行者**（自动控制每回合定编），措辞相反。
function autoWritten(leaf) { return normMode(leaf.mode) === 'Auto' ? ' · ' + AUTO_WRITTEN : ''; }

/// 「这个数现在是从哪来的」：引擎的取值链是
///   叶 `Inherit` + 舰队默认是 `Player` ⇒ 舰队默认的值；否则叶自己的值；没有叶 ⇒ 舰上记录值
/// （`State::ship_doctrine` / `ship_kiting`）。只在叶片**自己没表态**时显示——那时"你以为的
/// 归属"与"实际的来源"最容易错位，而这正是「改舰队默认对某艘舰没用」的成因。
function styleFollowHint(node) {
  const leaf = node.leaf;
  if (normMode(leaf.mode) !== 'Inherit') return null;
  const doctrine = node.kind === 'shipdoctrine';
  const summary = doctrine ? doctrineSummary : kitingSummary;
  const fc = getControl(node.fid);
  const d = fc[DEFAULT_LEAF[node.kind]];
  if (d && normMode(d.mode) === 'Player') {
    return hintLine('当前跟随：舰队默认（' + summary(d) + '）');
  }
  // 叶片**存在**但没有表态：引擎取的是叶里的值（`leaf.map(...)` 优先于记录值）——也就是说
  // 「没表态」不等于「没值」：这个数已经被钉在叶里，改出厂/舰队默认都不会再影响它。
  // ⚠ 「恢复继承」只撤**表态**、不动值（note §8 第 2 条），所以它**不会**把这个数放回出厂值
  // ——今天没有任何接口能删掉一片叶（补丁只能新建/改写），说法必须写到这一步。
  const raw = ((st.control || {})[node.fid] || {})[doctrine ? 'ship_doctrine' : 'ship_kiting'] || {};
  const s = st.ships.find((x) => x.name === node.id);
  if (raw[node.id]) return hintLine('当前跟随：本舰叶片里的数（没表态 ≠ 没值：引擎优先用叶里的值）——要真正还回出厂快照，用「恢复出厂值」删掉这片叶');
  const rec = s ? (doctrine ? s.doctrine : s.kiting) : null;
  return hintLine('当前跟随：出厂快照' + (rec != null ? '（' + summary(rec) + '）' : ''));
}

/// 「这艘舰的**角色**现在是从哪来的」：与 [`styleFollowHint`] 同一条取值链
/// （叶 → 舰队默认 → 舰上记录值），但有一句只属于这条轴的话——**这片叶是自动控制写的**，
/// 所以在这里「没表态」的含义是「AI 每回合可以重新决定这艘舰干什么活」，而不是「永远冻着」。
function freighterFollowHint(node) {
  const leaf = node.leaf;
  if (normMode(leaf.mode) !== 'Inherit') return null;
  const fc = getControl(node.fid);
  const d = fc.default_freighter;
  if (d && normMode(d.mode) === 'Player') {
    return hintLine('当前跟随：舰队默认（' + freighterSummary(d) + '）——它是玩家钉的 ⇒ 自动控制的逐舰定编不碰这艘舰');
  }
  const raw = ((st.control || {})[node.fid] || {}).ship_freighter || {};
  const s = st.ships.find((x) => x.name === node.id);
  if (raw[node.id]) {
    return hintLine('当前跟随：本舰叶片里的角色（没表态 ⇒ 这是**自动控制写下的定编结论**，它每回合会重估；要钉死就把归属改成「玩家」）');
  }
  const rec = s ? freighterSummary({ freighter: !!s.freighter }) : null;
  return hintLine('当前跟随：出厂快照' + (rec ? '（' + rec + '）' : '') + '——这一层还没有叶，自动控制随时可以给这艘舰定编');
}

// --- 节点类型注册表 ---------------------------------------------------------
// 每种的 per-kind 行为统一放在这里：renderNode/modeToggleFor 不再 switch 裸字符串。
//  childMode  'tabs'  容器：tab 带，仅展开激活子节点
//             'list'  分类组：全部子项堆叠
//             'leaf'  终端节点（无子）
//  scope      选「谁负责」三态 toggle 的来源：
//             null 无 toggle（纯分组容器）；'global' edScope.global；
//             'factions'/'bodies'/'cities' scopeVal(edScope[k], id)；'leaf' node.leaf.mode
//             （三态：Inherit=继承上层 / Auto=系统自动 / Player=玩家；读面永远给全三态之一）
//  editor     'ship'  舰行为编辑器 | 'doctrine' 行为风格两轴（理智↔热血/护航↔独狼）|
//             'kiting' 风筝↔贴脸姿态 | 'freighter' 角色开关（运输舰↔战舰）|
//             'value' 数值叶子编辑器 | 'building' 建筑
//             （前四种都只在**有效归属是玩家**时给编辑器，否则给一行 .tnode-hint）
const KIND = {
  global:    { childMode: 'tabs', scope: 'global' },
  faction:   { childMode: 'tabs', scope: 'factions', selectFaction: true },
  group:     { childMode: 'list', scope: null },
  body:      { childMode: 'tabs', scope: 'bodies' },
  city:      { childMode: 'list', scope: 'cities' },
  // 一条舰 = 一个**四叶容器**（指令 / 风格 / 风筝姿态 / 角色）。四片叶的归属链各自独立
  // （叶 → 舰队默认 → 势力 → 全局），所以「归谁」的下拉在子叶那一行，不在这容器上；
  // 容器这一行只留一个**便利**下拉（`bulkOwnership`）＝「这四片叶一起归谁」。
  ship:      { childMode: 'tabs', bulkOwnership: true },
  shiporder: { childMode: 'leaf', scope: 'leaf', editor: 'ship', decorateLabel: (n, w) => ' · ' + behaviorSummary(n.leaf.behavior, w) },
  // 逐舰风格两叶：值 = **有效风格**（叶 → 舰队默认 → 舰上记录值），mode = 该叶自己的表态。
  // `Auto` 时补一句实话（值在用，但风格轴没有重估者 ⇒ 它冻着；见 AUTO_FROZEN）。
  shipdoctrine: { childMode: 'leaf', scope: 'leaf', editor: 'doctrine', decorateLabel: (n) => ' · ' + doctrineSummary(n.leaf) + autoFrozen(n.leaf) },
  shipkiting:   { childMode: 'leaf', scope: 'leaf', editor: 'kiting', decorateLabel: (n) => ' · ' + kitingSummary(n.leaf) + autoFrozen(n.leaf) },
  // 势力级**三条默认**：指令 / 风格 / 风筝姿态。它们是「舰」这一组的前提（先定默认，例外才少写）。
  // 后两片与「舰队默认指令」同形，只是「风格」有两个轴：doctrine = 理智↔热血 + 护航↔独狼，
  // kiting = 风筝↔贴脸。摘要见 fleetStyleLabel（没表态就不显示数——那两个数还不算数）。
  fleetorder:    { childMode: 'leaf', scope: 'leaf', editor: 'ship', decorateLabel: (n, w) => ' · ' + behaviorSummary(n.leaf.behavior, w) },
  fleetdoctrine: { childMode: 'leaf', scope: 'leaf', editor: 'doctrine', decorateLabel: (n) => fleetStyleLabel(n.leaf, doctrineSummary) },
  fleetkiting:   { childMode: 'leaf', scope: 'leaf', editor: 'kiting', decorateLabel: (n) => fleetStyleLabel(n.leaf, kitingSummary) },
  // 第三条风格轴**角色**（运输舰↔战舰）。两行与上面同形，但有一条轴间差别：逐舰那片叶
  // **自动控制每回合也会写**（按积压定编）⇒ 它的「自动」是真的（用 autoWritten，不是 autoFrozen）。
  shipfreighter: { childMode: 'leaf', scope: 'leaf', editor: 'freighter', decorateLabel: (n) => ' · ' + freighterSummary(n.leaf) + autoWritten(n.leaf) },
  // 势力级默认角色：AI **不写**这片叶（它只写逐舰叶）⇒ 与另两条风格轴的默认叶同一条措辞。
  fleetfreighter: { childMode: 'leaf', scope: 'leaf', editor: 'freighter', decorateLabel: (n) => fleetStyleLabel(n.leaf, freighterSummary) },
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
  registerTab(); // 先报到：刷新时新页面的登记要尽早落地（服务端有个极短的刷新窗口）
  bindWorld(await fetchJSON('/api/state'));
  prevState = world;
  selFaction = st.factions && st.factions.length ? st.factions[0].name : '';
  sel = selFaction ? { kind: 'faction', name: selFaction } : null;
  initMap();
  buildEdits();
  renderAll();
  updateTop();
}

// --- 页面 vs 服务：最后一个页面关掉，服务就退 ---------------------------------
// 服务端**没有空闲计时器**（不会因为「你没在操作」而退）：它只在最后一个页面离开后
// 自退。所以每个页面载入时报到、离开时注销，且每次载入用一个**新的** tab id
// （刷新 = 旧 id 注销先到 + 新 id 登记后到，服务端留一个 500ms 的刷新窗口吸收它）。
const TAB_ID = (window.crypto && crypto.randomUUID)
  ? crypto.randomUUID()
  : String(Date.now()) + Math.random().toString(36).slice(2);

function registerTab() {
  postJSON('/api/tab', { tab: TAB_ID }).catch(() => {});
}

function leaveTab() {
  const body = JSON.stringify({ tab: TAB_ID });
  // sendBeacon 是唯一在 pagehide 里还算可靠的投递方式（fetch 会被页面卸载掐掉）。
  if (navigator.sendBeacon) navigator.sendBeacon('/api/bye', new Blob([body], { type: 'application/json' }));
  else fetch('/api/bye', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body, keepalive: true }).catch(() => {});
}
// bfcache 里页面还可能回来（persisted=true）→ 那时不许注销，回来时再报到一次。
window.addEventListener('pagehide', (e) => { if (!e.persisted) leaveTab(); });
window.addEventListener('pageshow', (e) => { if (e.persisted) registerTab(); });

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
  // 回传 diff 的基准 = **载入时**的读面（不是编辑面：编辑面里有界面自己补出来的壳）。
  baseControl = structuredClone(world.control || []);
  baseScope = structuredClone(world.scope || {});
  pairOrigins();
}

// --- 控制面的「原值」与回传 diff ---------------------------------------------
// 引擎的写面是 presence-aware diff：**只改动出现在补丁里的叶**，缺省的字段/叶一律保留
// 现值（`FactionControlPatch` 的文档）。所以前端只回传差异就够——而"回传整份"是有害的：
// 读面里的风格叶给的是**有效值**（叶 → 舰队默认 → 出厂记录值），整份回传会把
// 「没有叶 ⇒ 兜底到出厂记录值」变成「叶钉住这个数」；将来记录值的语义一变（例如蓝图
// 接管出厂风格），这些舰就**不再跟随**，而现象是「改图/改配置对这艘舰没用」——极难归因。
// 只回传差异还让「应用」变成幂等的：没改过 ⇒ 空 diff ⇒ 什么都没动（见 note §8 第 3 条）。
//
// 下面这张表是 diff 的地基：每片叶的**身份键**（回传必须带着）+ **值字段**（值 vs 表态）。
// 不能靠 `for (k in leaf)` 猜：读面里的条目还带着 `kind`/`structure`/`ship_type` 这类
// 实体属性（它们属于 state，不属于控制面）。
const LEAF_SPEC = {
  capital:             { keys: [],                values: ['value'] },
  default_ship_order:  { keys: [],                values: ['behavior'] },
  default_doctrine:    { keys: [],                values: ['temper', 'lone_wolf'] },
  default_kiting:      { keys: [],                values: ['kiting'] },
  default_freighter:   { keys: [],                values: ['freighter'] },
  ship_orders:         { keys: ['ship'],          values: ['behavior'] },
  ship_doctrine:       { keys: ['ship'],          values: ['temper', 'lone_wolf'] },
  ship_kiting:         { keys: ['ship'],          values: ['kiting'] },
  ship_freighter:      { keys: ['ship'],          values: ['freighter'] },
  investment_budget:   { keys: ['resource'],      values: ['value'] },
  construction_budget: { keys: ['resource'],      values: ['value'] },
  invest_weights:      { keys: ['city', 'building'], values: ['value'] },
  build_weights:       { keys: ['city', 'building'], values: ['value'] },
  loyalty_budget:      { keys: ['city'],          values: ['value'] },
};
/// 势力级那几片叶子（`Option<...>` 字段，不是数组）。
const LEAF_OPTIONS = ['capital', 'default_ship_order', 'default_doctrine', 'default_kiting', 'default_freighter'];

/// 记下一片叶的原值。**不覆盖**已经记过的：读面里的叶在 [`pairOrigins`] 里配过对。
function rememberOrigin(leaf, fields, spec, shell) {
  if (leaf && !leafOrigin.has(leaf)) {
    leafOrigin.set(leaf, { fields: fields, values: spec.values, shell: !!shell });
  }
  return leaf;
}

/// 界面**补出来**的叶（读面里没有它）：值只用于显示，原值就是这片壳的模板。
///
/// 壳一旦被**动过**，回传时按「新建这片叶」发出去，而且是**完整**的（两轴一起给）：
/// 势力级两轴叶单轴新建会被引擎响亮拒绝（note §3.1）。没动过的壳不会进 diff。
function shellLeaf(leaf, spec) { return rememberOrigin(leaf, Object.assign({}, leaf), spec, true); }

/// 把编辑面里的每一片叶与读面里的**同一片叶**配成对：回传时只报这两者之间变了的字段。
function pairOrigins() {
  leafOrigin.clear();
  autoPinned = new WeakSet();
  edControl.forEach((fac) => {
    const base = baseControl.find((b) => b.faction_id === fac.faction_id);
    if (!base) return;
    LEAF_OPTIONS.forEach((name) => {
      if (fac[name] && base[name]) rememberOrigin(fac[name], Object.assign({}, base[name]), LEAF_SPEC[name], false);
    });
    Object.keys(LEAF_SPEC).forEach((name) => {
      const spec = LEAF_SPEC[name];
      if (!spec.keys.length) return;
      const b = base[name] || [];
      (fac[name] || []).forEach((e) => {
        const o = b.find((x) => spec.keys.every((k) => x[k] === e[k]));
        if (o) rememberOrigin(e, Object.assign({}, o), spec, false);
      });
    });
  });
}

/// 一片叶现在与原值相比**值**有没有变（身份键与 `mode` 不算）。
function leafValueChanged(leaf) {
  const o = leafOrigin.get(leaf);
  if (!o) return false;
  return o.values.some((k) => JSON.stringify(leaf[k]) !== JSON.stringify(o.fields[k]));
}

/// 写值时的接管规则（note §8 第 2 条）：
/// * 值**变了** ⇒ 写值即接管（这片叶本来是 `Inherit` 就钉成 `Player`，与 `--apply` 同一条规则）；
/// * 值又变回**载入时那个数** ⇒ 这次编辑不算表态：把界面自己钉的 `Player` 撤回原来的表态。
///   只撤**界面自己钉的**——你在下拉里显式选过的模式不会被值的变化推翻。
function wroteValue(leaf) {
  const o = leafOrigin.get(leaf);
  if (!o) {
    // 没有原值可参照（理论上不会发生）：退回「写值即接管」这条保守规则。
    if (normMode(leaf.mode) === 'Inherit') leaf.mode = 'Player';
    return;
  }
  if (leafValueChanged(leaf)) {
    if (normMode(leaf.mode) === 'Inherit') { leaf.mode = 'Player'; autoPinned.add(leaf); }
  } else if (autoPinned.has(leaf)) {
    leaf.mode = normMode(o.fields.mode);
    autoPinned.delete(leaf);
  }
}

/// 一片叶的回传形态：身份键 + **变过的**字段（原值里没有别的字段能变）。
/// 壳（读面里没有这片叶）被碰过 ⇒ 值字段**全给**：那是"新建这片叶"，缺一条轴会被拒。
function diffLeaf(leaf, spec) {
  const o = leafOrigin.get(leaf);
  if (!o) return null; // 连原值都没有 ⇒ 不敢猜，宁可不回传
  if (removedLeaves.has(leaf)) {
    // 删叶：只发身份键 + `remove`（引擎拒绝「删叶 + 写值」混在一条补丁里）。
    if (o.shell) return null; // 读面里本来就没有这片叶 ⇒ 没什么可删的
    const out = {};
    spec.keys.forEach((k) => { out[k] = leaf[k]; });
    out.remove = true;
    return out;
  }
  const out = {};
  spec.keys.forEach((k) => { out[k] = leaf[k]; });
  let changed = false;
  spec.values.forEach((k) => {
    if (JSON.stringify(leaf[k]) !== JSON.stringify(o.fields[k])) { out[k] = leaf[k]; changed = true; }
    else if (o.shell) out[k] = leaf[k];
  });
  const mode = normMode(leaf.mode);
  if (mode !== normMode(o.fields.mode)) { out.mode = mode; changed = true; }
  else if (o.shell && changed) out.mode = mode;
  return changed ? out : null;
}

/// 作用域的差异：只报**表态变过**的键（没列出过 = `Inherit`，与"没有说话"等价）。
function buildScopeDiff() {
  const cur = edScope || {};
  const base = baseScope || {};
  const out = {};
  if (normMode(cur.global) !== normMode(base.global)) out.global = normMode(cur.global);
  ['factions', 'bodies', 'cities'].forEach((k) => {
    const kept = [];
    (cur[k] || []).forEach((pair) => {
      const hit = (base[k] || []).find((p) => p[0] === pair[0]);
      const v = normMode(pair[1]);
      if (v !== normMode(hit ? hit[1] : 'Inherit')) kept.push([pair[0], v]);
    });
    if (kept.length) out[k] = kept;
  });
  return Object.keys(out).length ? out : null;
}

/// 回传给 `/api/command` 的**差异**：只有真的被改过的叶、每片叶只带真的变过的字段。
/// 没改过 ⇒ `{control: []}`（+ 可能为空的作用域段）⇒ 幂等。
function buildCommandDiff() {
  const control = [];
  edControl.forEach((fac) => {
    const out = { faction_id: fac.faction_id };
    let n = 0;
    LEAF_OPTIONS.forEach((name) => {
      if (!fac[name]) return;
      const d = diffLeaf(fac[name], LEAF_SPEC[name]);
      if (d) { out[name] = d; n++; }
    });
    Object.keys(LEAF_SPEC).forEach((name) => {
      if (!LEAF_SPEC[name].keys.length) return;
      const kept = [];
      (fac[name] || []).forEach((e) => { const d = diffLeaf(e, LEAF_SPEC[name]); if (d) kept.push(d); });
      if (kept.length) { out[name] = kept; n++; }
    });
    // `buildings` 是**命令列表**（按下「新建 / 移除 / 改属性」才存在的一条条意图），不是快照：
    // 里面的每一条本来就只该发一次，原样回传。
    if (fac.buildings && fac.buildings.length) { out.buildings = fac.buildings; n++; }
    if (n) control.push(out);
  });
  const req = { control: control };
  const scope = buildScopeDiff();
  if (scope) req.scope = scope;
  return req;
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
  return e ? e[1] : 'Inherit';
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
// map3d 的接口是既定的 `{bodies, cities, ships, factions}`，其中势力按 `id` 查阵营色、按
// `capital_body` 给天体标签加「首都色点」。这两个名字都不是原始 `Faction` 的字段，但**值都在
// state 里**：
//   * `id`           —— 就是 `Faction.name`（势力唯一键）。
//   * `capital_body` —— 有效首都在**命令控制**里：`state.control[势力].capital.value`
//                       （`State::capital_body` 的唯一事实来源，建世界时就按 `initial_capital`
//                       播种、之后由 sim 的迁都步骤维护；`Faction` 刻意不存首都，无 shadow 双状态）。
// 所以适配层只把这两处**接通**给地图，不新增任何事实：控制叶子缺了就不给别名（map3d 会跳过该
// 色点），绝不瞎兜一个天体。其余字段 `Object.assign` 原样透传（relations/resources/ideology…）。
function mapWorld() {
  const ctrl = st.control || {};
  return Object.assign({}, st, {
    factions: (st.factions || []).map((f) => {
      const cap = ctrl[f.name] && ctrl[f.name].capital;
      return Object.assign({}, f, { id: f.name, capital_body: cap ? cap.value : undefined });
    }),
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

    const shipNodes = (fc.ship_orders || []).map((ord) => shipNode(fc, ord));
    // 势力级**四条默认**（指令 / 风格 / 风筝姿态 / 角色）也是可编辑叶片：新舰出生就继承它们，
    // 一次性指令收尾也回落到它们。它们排在「舰」分组**之前**——因为它们是这一组的前提
    // （先定默认，例外才少写）。读面里没有这条叶 = 没有人表态，这里补一片 `Inherit` 的**壳**
    // 让它出现在树上（与作用域里「没列出的层 ≡ 继承」同义）。壳只用于显示：没被动过就不会
    // 进回传 diff，动过就按「新建这片叶」整片发出去（见 shellLeaf）。
    const fleetOrder = shellLeaf(fc.default_ship_order = fc.default_ship_order
      || { behavior: 'Idle', mode: 'Inherit' }, LEAF_SPEC.default_ship_order);
    const fleetDoctrine = shellLeaf(fc.default_doctrine = fc.default_doctrine
      || { temper: 0, lone_wolf: 0, mode: 'Inherit' }, LEAF_SPEC.default_doctrine);
    const fleetKiting = shellLeaf(fc.default_kiting = fc.default_kiting
      || { kiting: 0, mode: 'Inherit' }, LEAF_SPEC.default_kiting);
    const fleetFreighter = shellLeaf(fc.default_freighter = fc.default_freighter
      || { freighter: false, mode: 'Inherit' }, LEAF_SPEC.default_freighter);

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
    if (shipNodes.length || fleetOrder) {
      const kids = [
        { key: 'fleet' + fid, kind: 'fleetorder', id: fid, name: '舰队默认指令', leaf: fleetOrder, fid },
        { key: 'fleetdoc' + fid, kind: 'fleetdoctrine', id: fid, name: '舰队默认风格', leaf: fleetDoctrine, fid },
        { key: 'fleetkit' + fid, kind: 'fleetkiting', id: fid, name: '舰队默认风筝姿态', leaf: fleetKiting, fid },
        { key: 'fleetfrt' + fid, kind: 'fleetfreighter', id: fid, name: '舰队默认角色', leaf: fleetFreighter, fid },
      ].concat(shipNodes);
      cats.push({ key: 'gs' + fid, kind: 'group', name: '舰', fid, children: kids });
    }
    if (budgetKids.length) cats.push({ key: 'gbd' + fid, kind: 'group', name: '预算', fid, children: budgetKids });
    if (bodyNodes.length) cats.push({ key: 'gb' + fid, kind: 'group', name: '天体', fid, children: bodyNodes });
    fn.children = cats;

    root.children.push(fn);
  });
  return root;
}

// 一条舰 = 一个四叶容器：**指令**（干什么）+ **风格**（理智↔热血 / 护航↔独狼）+
// **风筝姿态**（风筝↔贴脸）+ **角色**（运输舰↔战舰）。四片叶的归属链各自独立，所以
// 「归谁」的下拉跟着子叶走。
//
// 风格两叶与角色叶的读面**总是**给（哪怕状态里根本没有那片叶）：值 = **有效值**（叶 →
// 舰队默认 → 舰上记录值）、mode = 这片叶自己的表态。所以这里显示的就是「这艘舰现在实际
// 用的风格/角色」。
// ⚠ `Ship.doctrine` / `Ship.kiting` / `Ship.freighter` 只是出厂快照（**记录值**），改它们没有
// 任何控制效果——那正是「我明明改了风格却没反应」的坑；要改就走这几片叶。
function shipNode(fc, ord) {
  const name = ord.ship;
  const s = st.ships.find((x) => x.name === name);
  const kids = [{ key: 'ord' + name, kind: 'shiporder', id: name, name: '指令', leaf: ord, fid: fc.faction_id }];
  // 世界里已经没有这艘舰（战沉 / 改名换代）时只留指令行：它的风格叶写进去也只会被丢弃。
  if (s) {
    kids.push({
      key: 'doc' + name, kind: 'shipdoctrine', id: name, name: '风格', fid: fc.faction_id,
      leaf: styleLeaf(fc, 'ship_doctrine', name, {
        temper: +((s.doctrine || {}).temper) || 0,
        lone_wolf: +((s.doctrine || {}).lone_wolf) || 0,
      }),
    });
    kids.push({
      key: 'kit' + name, kind: 'shipkiting', id: name, name: '风筝姿态', fid: fc.faction_id,
      leaf: styleLeaf(fc, 'ship_kiting', name, { kiting: +s.kiting || 0 }),
    });
    // 角色：这片叶**自动控制每回合会写**（按积压定编谁去跑集货路线），所以它常常带着一个
    // 玩家没写过的值 + `Inherit` 表态——`autoWritten` 会在那一行把这件事说清楚。
    kids.push({
      key: 'frt' + name, kind: 'shipfreighter', id: name, name: '角色', fid: fc.faction_id,
      leaf: styleLeaf(fc, 'ship_freighter', name, { freighter: !!s.freighter }),
    });
  }
  return { key: 'ship' + name, kind: 'ship', id: name, name: (s ? s.name : '船#' + name), fid: fc.faction_id, children: kids };
}

// 逐舰风格叶：读面里**每艘舰都有**一行（值 = 有效值、mode = 叶片表态），正常路径就是取它。
// 万一没有那一行（老服务端 / 舰还没进读面）就补一片壳，值取**舰上记录值**——那是引擎在没有
// 叶片时的兜底值，比凭空写个 0 诚实（读面里有这一行时 pairOrigins 已配好原值，壳不生效）。
function styleLeaf(fc, key, ship, blank) {
  const list = (fc[key] = fc[key] || []);
  let e = list.find((x) => x.ship === ship);
  if (!e) {
    e = Object.assign({ ship: ship, mode: 'Inherit' }, blank);
    list.push(e);
  }
  return shellLeaf(e, LEAF_SPEC[key]);
}

function renderTree() {
  const tree = $('#tree');
  tree.innerHTML = '';
  tree.appendChild(renderNode(buildTree()));
}

function renderNode(node) {
  const spec = KIND[node.kind] || {};
  // `data-key` = 这棵树里的稳定节点键（「哪个势力的哪片叶」）。纯属可读性/可自动化：
  // 人和脚本都能 `[data-key="fleetdoc中国"]` 一步点到那一行，不必靠中文文本猜。
  const wrap = el('div', { class: 'tnode', 'data-key': node.key });
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
  // 「恢复继承」：撤销这片叶的**表态**（mode → 继承），值不动。只在它自己有表态时出现——
  // 那时"我想反悔"才有意义（把下拉调回「继承」等价，但这个按钮把撤销写在脸上）。
  if (node.leaf && normMode(node.leaf.mode) !== 'Inherit') head.appendChild(restoreInheritButton(node.leaf));
  // 「恢复出厂值」：**删掉这片叶**（取值真的回到上层/出厂快照）。只在状态里真的有这片叶时出现。
  if (node.leaf && rawLeafOf(node)) head.appendChild(removeLeafButton(node));
  if (spec.bulkOwnership) {
    const bulk = bulkOwnershipSelect(node);
    if (bulk) head.appendChild(bulk);
  }
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

  // 被标记删除的叶：不再给编辑器（点「应用」它就没了），只说明会发生什么。
  if (node.leaf && removedLeaves.has(node.leaf)) {
    wrap.appendChild(hintLine('已标记删除：点「应用到服务器」之后这片叶消失，取值回到上层 / 出厂快照（再点一次按钮可撤销）'));
    return wrap;
  }

  if (spec.editor === 'ship') {
    // 编辑器对「**有效归属**是玩家」的叶子开放，而不是只看叶子自己的 mode：舰队默认指令
    // 设成玩家之后，继承它（`Inherit`）的舰也归你管——以前那些舰在 UI 上连编辑器都没有。
    // 改行为时会把叶子显式钉成 Player（写值即接管，与 `--apply` 同一条规则）。
    if (effectiveMode(node) === 'Player') wrap.appendChild(shipEditor(node.leaf, node));
    else wrap.appendChild(hintLine('由系统自动决定（要自己指挥就把左边的归属改成「玩家」）'));
  } else if (spec.editor === 'doctrine') {
    // 行为风格两轴（理智↔热血 / 护航↔独狼）：与「舰队默认指令」同一套开放规则——按有效归属
    // 判断能不能编辑（继承舰队默认风格也算你的），改值即把这片叶钉成玩家。
    if (effectiveMode(node) === 'Player') wrap.appendChild(doctrineEditor(node));
    else wrap.appendChild(hintLine('由系统自动决定（要自己定风格就把左边的归属改成「玩家」）'));
  } else if (spec.editor === 'kiting') {
    // 风筝↔贴脸姿态（单片叶）：同上。
    if (effectiveMode(node) === 'Player') wrap.appendChild(kitingEditor(node));
    else wrap.appendChild(hintLine('由系统自动决定（要自己定风筝姿态就把左边的归属改成「玩家」）'));
  } else if (spec.editor === 'freighter') {
    // 角色（运输舰↔战舰，单片叶）：同一条开放规则。但这里「由系统自动决定」**不是空话**
    // ——自动控制每回合按积压定编，所以提示要说清它真的会替你决定。
    if (effectiveMode(node) === 'Player') wrap.appendChild(freighterEditor(node));
    else if (node.kind === 'fleetfreighter') {
      // ⚠ 势力级这片默认叶**没有执行者**：自动控制只写逐舰角色叶，从不写它。
      // 所以这里不能照抄逐舰那句「由自动控制定编」——那是假话（note §3.2 的措辞纪律）。
      wrap.appendChild(hintLine('未表态：这片默认叶只在它自己是「玩家」时才供值（自动控制的定编只写逐舰角色叶，不写它）——要「全舰队听我的」就把它改成「玩家」'));
    } else {
      wrap.appendChild(hintLine('由自动控制定编（它每回合按积压决定谁去跑集货路线）；要自己钉死这艘舰，就把左边的归属改成「玩家」'));
    }
  } else if (spec.editor === 'value') {
    wrap.appendChild(leafValueEditor(node.leaf, spec.editorLabel, node));
  } else if (spec.editor === 'building') {
    wrap.appendChild(buildingEditor(node));
  }
  // 风格两叶 / 角色叶：把「这个数现在从哪来」说清楚（只在叶片自己没表态时，见两个 followHint）。
  if (node.kind === 'shipdoctrine' || node.kind === 'shipkiting') {
    const fh = styleFollowHint(node);
    if (fh) wrap.appendChild(fh);
  } else if (node.kind === 'shipfreighter') {
    const fh = freighterFollowHint(node);
    if (fh) wrap.appendChild(fh);
  }
  return wrap;
}

function hintLine(text) {
  const d = el('div', { class: 'tnode-hint' });
  d.textContent = text;
  return d;
}

/// 「恢复继承」：撤销这片叶的**表态**（`mode → Inherit`），**值不动**（note §8 第 2 条的
/// 主手段）。它与「把下拉调回继承」是同一件事，只是把"怎么撤销"写在脸上——「我碰过这一格，
/// 但我不想再对它表态」是常见意图，而以前只能靠理解三态的含义才做得到。
///
/// ⚠ 它**不是**「删掉这片叶」：引擎的取值规则是"叶存在就用叶里的值"（哪怕叶说 Inherit），
/// 而补丁接口只能新建/改写叶、删不掉（见 styleFollowHint 的说明）。所以撤销表态之后，
/// 叶里那个数**仍然在用**——这正是「只回传差异」为什么重要：别让界面顺手造出这些叶。
function restoreInheritButton(leaf) {
  const b = el('button', {
    class: 'restore', 'data-role': 'restore',
    title: '撤销这片叶的表态（重新交给上层：舰队默认 / 势力 / 全局）。叶里那个数不动——它不是"删掉这片叶"',
  }, '恢复继承');
  b.addEventListener('click', () => {
    leaf.mode = 'Inherit';
    autoPinned.delete(leaf);
    renderTree();
  });
  return b;
}

/// 舰行上的**便利**下拉：「这条舰的各片叶（指令 / 风格 / 风筝姿态 / 角色）一起归谁」。
///
/// 它**只写 mode**（每片叶各写一次表态），**永不写值、永不接管**——所以它是"批量效率"，
/// 不是"一个下拉代表一种归属"的假象（各片叶各有自己的归属链，见 note §8 第 1 条）。
/// 各叶表态不一致时多一个「各自不同」占位项，绝不假装它们一样。
function bulkOwnershipSelect(node) {
  const leaves = (node.children || []).map((ch) => ch.leaf).filter(Boolean);
  if (leaves.length < 2) return null; // 只剩指令行（舰已不在世界里）时不值得有这个下拉
  const modes = leaves.map((l) => normMode(l.mode));
  const mixed = modes.some((m) => m !== modes[0]);
  const sel = el('select', { class: 'mode bulk', 'data-role': 'bulk' });
  if (mixed) {
    const o = el('option', { value: '' });
    o.textContent = leaves.length + ' 片叶：各自不同';
    o.selected = true;
    sel.appendChild(o);
  }
  [['Inherit', '一起：继承'], ['Auto', '一起：自动'], ['Player', '一起：玩家']].forEach(([v, l]) => {
    const o = el('option', { value: v });
    o.textContent = l;
    o.selected = !mixed && modes[0] === v;
    sel.appendChild(o);
  });
  sel.title = '把这条舰的 ' + leaves.length + ' 片叶（指令 / 风格 / 风筝姿态 / 角色）的归属一起改掉：只写归属、不写值';
  sel.addEventListener('change', () => {
    if (!sel.value) return;
    leaves.forEach((l) => { l.mode = sel.value; autoPinned.delete(l); });
    renderTree();
  });
  return sel;
}

// --- 删叶（「恢复出厂值」） --------------------------------------------------
// 引擎的取值规则是「叶存在就用叶里的值」（`leaf.map(|l| l.value).unwrap_or(record)`，与叶的
// `mode` 无关），所以「恢复继承」只交还**归属**、交还不了**数值**：碰过一次的风格叶会一直
// 钉着那个数。真正把它放回出厂快照 / 舰队默认的动作是**删掉这片叶**（补丁的 `remove: true`）。
// 这里就是那个动作（note §10.4 选定的方案 A）。
const removedLeaves = new WeakSet(); // 被标记「删掉这片叶」的叶（点「应用」时才真的发出去）

/// 这片叶在**原始 state** 里存在吗？——删叶按钮只在真的有这片叶时出现。
/// （读面里逐舰风格两行**总是**在，哪怕叶不存在；要知道真相得看 `state.control`。）
const RAW_LEAF = {
  shiporder:     { list: 'ship_orders', key: (n) => n.id },
  shipdoctrine:  { list: 'ship_doctrine', key: (n) => n.id },
  shipkiting:    { list: 'ship_kiting', key: (n) => n.id },
  shipfreighter: { list: 'ship_freighter', key: (n) => n.id },
  fleetorder:    { list: 'default_ship_order' },
  fleetdoctrine: { list: 'default_doctrine' },
  fleetkiting:   { list: 'default_kiting' },
  fleetfreighter:{ list: 'default_freighter' },
  resource:      { list: 'investment_budget', key: (n) => n.leaf.resource },
  conbudget:     { list: 'construction_budget', key: (n) => n.leaf.resource },
};

function rawLeafOf(node) {
  const spec = RAW_LEAF[node.kind];
  if (!spec) return null;
  const raw = (st.control || {})[node.fid];
  if (!raw) return null;
  const bucket = raw[spec.list];
  if (bucket == null) return null;
  return spec.key ? (bucket[spec.key(node)] || null) : bucket;
}

/// 「恢复出厂值」= **删掉这片叶**。它与「恢复继承」不是一回事（见上面的说明），所以两个
/// 动作并存、各自写在按钮上：「恢复继承」= 交还归属；「恢复出厂值」= 把这个数也还回去。
function removeLeafButton(node) {
  const marked = removedLeaves.has(node.leaf);
  const b = el('button', {
    class: 'restore rm-leaf' + (marked ? ' on' : ''), 'data-role': 'remove-leaf',
    title: '删掉这片叶（不是清空）：这一层不再说话，取值回到上层 / 出厂快照。点「应用到服务器」才生效。',
  }, marked ? '取消删除' : '恢复出厂值');
  b.addEventListener('click', () => {
    if (marked) removedLeaves.delete(node.leaf);
    else removedLeaves.add(node.leaf);
    renderTree();
  });
  return b;
}

/// 一片叶子的**有效归属**：叶子自己 → （舰：**该轴对应的**舰队默认叶）→ 势力 → 全局。
/// 这是 `State::ship_control` / `ship_doctrine_control` / `ship_kiting_control` 在前端的
/// 对应读法，UI 用它决定「这片叶子现在归谁、能不能编辑」。
///
/// 「该轴对应的默认叶」不是写死的 `default_ship_order`：引擎里三条轴各有一片势力级默认
/// （指令 → `default_ship_order`、风格 → `default_doctrine`、风筝姿态 → `default_kiting`、
/// 角色 → `default_freighter`），见 [`DEFAULT_LEAF`]。
const DEFAULT_LEAF = {
  shiporder: 'default_ship_order',
  shipdoctrine: 'default_doctrine',
  shipkiting: 'default_kiting',
  shipfreighter: 'default_freighter',
};

function effectiveMode(node) {
  const own = normMode(node.leaf && node.leaf.mode);
  if (own !== 'Inherit') return own;
  const dkey = DEFAULT_LEAF[node.kind];
  if (dkey) {
    const fc = getControl(node.fid);
    const d = normMode(fc[dkey] && fc[dkey].mode);
    if (d !== 'Inherit') return d;
  }
  const fac = normMode(scopeVal(edScope.factions, node.fid));
  if (fac !== 'Inherit') return fac;
  return normMode(edScope.global);
}

function modeToggleFor(node) {
  const acc = scopeAccess(node);
  if (!acc) return null;
  const mode = acc.get();
  const set = acc.set;
  const styleAxis = AUTO_FROZEN_KINDS.indexOf(node.kind) >= 0;

  const sel = el('select', { class: 'mode', 'data-role': 'mode' });
  [['Inherit', '继承'], ['Auto', '自动'], ['Player', '玩家']].forEach(([v, l]) => {
    const o = el('option', { value: v });
    o.textContent = l;
    // 「自动」的措辞必须诚实（note §3.2）：风格轴上**没有**执行者，不能暗示"系统会来写"。
    if (v === 'Auto') {
      o.title = styleAxis
        ? '风格轴目前没有 AI 执行者：选「自动」不会有人来重估这个值，它只会冻在现在这个数'
        : '由 AI 每回合按局势重估（指令 / 预算轴真的有执行者）';
    } else if (v === 'Inherit') {
      o.title = '撤销这一层的表态：向上层要答案（舰队默认 / 势力 / 全局），风格轴还会落到出厂快照';
    } else {
      o.title = '归你管：系统不再改写它';
    }
    o.selected = normMode(mode) === v;
    sel.appendChild(o);
  });
  sel.title = styleAxis ? '谁负责这片风格叶（注意：风格轴暂无 AI 执行者）' : '谁负责这片叶';
  sel.addEventListener('change', () => {
    set(sel.value);
    // 显式选过模式 = 一次明确的表态：撤回「写值即接管」的书签——值再变回原数也不该翻掉它。
    if (node.leaf) autoPinned.delete(node.leaf);
    renderTree();
  });
  return sel;
}

// 读面的三态是权威拼写；缺省（null/undefined，例如 scope 里没列出的层）算「继承」。
function normMode(m) { return m || 'Inherit'; }

// --- 风格编辑器（行为风格两轴 / 风筝↔贴脸一条轴）-----------------------------
// 两片都写**叶片**（逐舰是 `ship_doctrine`/`ship_kiting`，势力级是 `default_doctrine`/
// `default_kiting`），**不碰** `Ship.doctrine`/`Ship.kiting`——那对字段在引擎里已降级为
// 「记录值」（出厂快照 + AI 流水），写它不产生任何控制效果。
// 每条轴取 [-1,1]（0 = 基线），钳在两端；值一改就把这片叶钉成 Player（写值即接管，
// 与 `--apply` 同一条规则：只写值不写 mode ⇒ 该叶变成玩家指令）。
/// 值一写就可能接管（`Inherit` → `Player`）：把这一行 head 里的归属下拉与「恢复继承」**就地**
/// 跟上去。不重画整棵子树——那会换掉你正在编辑的那个输入框（与 styleField 的取舍一致），
/// 于是会出现「我刚敲了数，归属却还写着继承」这种骗人的画面。
function syncOwnership(node) {
  if (!node || !node.leaf) return;
  const wrap = document.querySelector('.tnode[data-key="' + node.key + '"]');
  const head = wrap && wrap.querySelector(':scope > .tnode-head');
  if (!head) return;
  const mode = normMode(node.leaf.mode);
  const sel = head.querySelector('select.mode[data-role="mode"]');
  if (sel && sel.value !== mode) sel.value = mode;
  const btn = head.querySelector('button[data-role="restore"]');
  if (mode === 'Inherit') {
    if (btn) btn.remove();
  } else if (!btn) {
    head.insertBefore(restoreInheritButton(node.leaf), sel ? sel.nextSibling : null);
  }
}

function setStyleAxis(leaf, key, raw) {
  leaf[key] = Math.max(-1, Math.min(1, +raw || 0));
  // 值变了 ⇒ 写值即接管；值又改回**载入时那个数** ⇒ 不算表态（note §8 第 2 条）。
  wroteValue(leaf);
}

function styleField(axis, label, title, val, onSet, node) {
  const w = el('span', { class: 'style-field' });
  w.appendChild(el('span', { class: 'lv-label' }, label + ' '));
  const inp = el('input', { type: 'number', class: 'num', step: '0.1', min: '-1', max: '1', value: (+val || 0).toFixed(2), 'data-axis': axis });
  if (title) inp.title = title;
  inp.addEventListener('input', () => { onSet(inp.value); syncOwnership(node); });
  // `change`（离开这一格 / 回车）后重画一次：行摘要显示的就是叶里现在那个数。
  // 输入过程中不重画——那会把正在编辑的输入框换掉（与数值叶编辑器同一条取舍）。
  inp.addEventListener('change', () => renderTree());
  w.appendChild(inp);
  return w;
}

function doctrineEditor(node) {
  const leaf = node.leaf;
  const box = el('div', { class: 'ship-editor' });
  box.appendChild(styleField('temper', '理智↔热血', '负 = 欺软怕硬（挑威慑比自己低的）；正 = 飞蛾扑火（挑威慑比自己高的）；0 = 基线', leaf.temper, (v) => setStyleAxis(leaf, 'temper', v), node));
  box.appendChild(styleField('lone_wolf', '护航↔独狼', '负 = 空闲时贴本势力旗舰护航；正 = 独狼（空闲时自行就近接战）；0 = 基线', leaf.lone_wolf, (v) => setStyleAxis(leaf, 'lone_wolf', v), node));
  return box;
}

function kitingEditor(node) {
  const leaf = node.leaf;
  const box = el('div', { class: 'ship-editor' });
  box.appendChild(styleField('kiting', '风筝↔贴脸', '负 = 风筝（保持最远武器射程、敌近则拉开、更早撤）；正 = 贴脸（压近敌舰、打得更久）；0 = 基线', leaf.kiting, (v) => setStyleAxis(leaf, 'kiting', v), node));
  return box;
}

/// **角色**编辑器（运输舰↔战舰）：一条开关，不是 [-1,1] 的轴。
/// 写它 = 手动给这艘舰定活，自动控制的逐舰定编从此不碰它（`Player` 是那道闸门）；
/// 它只决定**派哪种活**，不解除武装——运输舰照样自动开火、照样按 kiting 姿态软移动。
function freighterEditor(node) {
  const leaf = node.leaf;
  const box = el('div', { class: 'ship-editor' });
  const sel = el('select', { class: 'role', 'data-axis': 'freighter' });
  [['true', '运输舰（按积压派集货路线）'], ['false', '战舰（找仗打）']].forEach(([v, lbl]) => {
    const o = el('option', { value: v });
    o.textContent = lbl;
    o.selected = String(!!leaf.freighter) === v;
    sel.appendChild(o);
  });
  sel.title = '角色只决定自动控制派哪种活：运输舰去跑集货路线（按积压抽签），战舰去找仗打。它**不解除武装**，也不是「军舰/民船」的军备差别。';
  sel.addEventListener('change', () => {
    leaf.freighter = sel.value === 'true';
    // 与另两条风格轴同一条规则：值变了 ⇒ 写值即接管（把这片叶钉成玩家，AI 定编从此不碰它）。
    wroteValue(leaf);
    renderTree();
  });
  box.appendChild(sel);
  return box;
}

function shipEditor(leaf, node) {
  const edit = el('div', { class: 'ship-editor' });
  const t = behaviorType(leaf.behavior);
  const d = behaviorToInput(leaf.behavior);
  // 改行为 = 变成你自己的指令（写值即接管）：否则这次编辑会被系统下一回合按自己的逻辑
  // 覆盖掉，而界面上看起来「我明明改了」。
  const commit = (b) => {
    leaf.behavior = b;
    // 写值即接管（否则这次编辑会被系统下一回合按自己的逻辑覆盖掉，而界面上看起来
    // 「我明明改了」）；把行为改回**载入时那个**行为则不算表态，同风格轴的规则。
    wroteValue(leaf);
    renderTree();
  };

  const typeSel = el('select');
  [['idle', '待命'], ['move', '移动'], ['follow', '跟随舰'], ['dock_city', '停泊城'], ['dock', '停泊轨道'], ['colonize', '殖民']].forEach(([v, lbl]) => {
    const o = el('option', { value: v }); o.textContent = lbl; o.selected = t === v; typeSel.appendChild(o);
  });
  typeSel.addEventListener('change', () => commit(behaviorFromInput(typeSel.value, d)));
  edit.appendChild(typeSel);

  const bodySel = () => {
    const s = el('select');
    st.bodies.filter((x) => x.settlements && x.settlements.length).forEach((bd) => {
      const o = el('option', { value: bd.name }); o.textContent = bd.name; o.selected = d.body === bd.name; s.appendChild(o);
    });
    s.addEventListener('change', () => { d.body = s.value; commit(behaviorFromInput(typeSel.value, d)); });
    return s;
  };
  const citySel = () => {
    const s = el('select');
    st.cities.forEach((c) => {
      const o = el('option', { value: c.name }); o.textContent = c.name; o.selected = d.city === c.name; s.appendChild(o);
    });
    s.addEventListener('change', () => { d.city = s.value; commit(behaviorFromInput(typeSel.value, d)); });
    return s;
  };
  const shipSel = () => {
    const s = el('select');
    st.ships.forEach((sh) => {
      const o = el('option', { value: sh.name }); o.textContent = sh.name; o.selected = d.ship === sh.name; s.appendChild(o);
    });
    s.addEventListener('change', () => { d.ship = s.value; commit(behaviorFromInput(typeSel.value, d)); });
    return s;
  };

  if (t === 'move') {
    edit.appendChild(inputNum('x', d.x, (v) => { d.x = +v; commit(behaviorFromInput(t, d)); }));
    edit.appendChild(inputNum('y', d.y, (v) => { d.y = +v; commit(behaviorFromInput(t, d)); }));
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

function leafValueEditor(leaf, label, node) {
  const wrap = el('div', { class: 'leaf-val' });
  const t = el('span', { class: 'lv-label' });
  t.textContent = label + ' ';
  const inp = el('input', { type: 'number', class: 'num', value: leaf.value, step: '0.1' });
  // 只有「有效归属是玩家」的叶子才可编辑（可能继承自势力/天体/城市层的作用域）。
  inp.disabled = !node || effectiveMode(node) !== 'Player';
  inp.addEventListener('input', () => {
    if (inp.disabled) return;
    leaf.value = +inp.value || 0;
    // 写值即接管；值改回**载入时那个数**不算表态（note §8 第 2 条，与风格轴同一条规则）。
    wroteValue(leaf);
    syncOwnership(node); // 接管了就立刻把那一行的归属下拉跟上（输入框不重画）
  });
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
  const fc = getControl(fid);

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
    // **设计图**：这个建造区把「还不存在的舰」造成什么样。
    //
    // 读面（`world.control[势力].blueprints`）给的是图库；这一行只写**指针**
    // （`buildings[].blueprint`）——「（无：自动选装）」= 拆掉指针（写 `null`，不是
    // 缺席：缺席 = 不动这一格，两者后果不同）。图的内容（选装/意图/归属）在图上改，
    // 引擎会在 `--apply` 时报 `blueprint_class_mismatch`（图的舰级必须与舰型相等）。
    const bps = fc.blueprints || [];
    const bpSel = el('select', { 'data-key': 'blueprint-' + cityId + '-' + b.id });
    const none = el('option', { value: '' });
    none.textContent = '（无：自动选装）';
    none.selected = !b.blueprint;
    bpSel.appendChild(none);
    bps.forEach((bp) => {
      const o = el('option', { value: bp.name });
      // 归属是本势力的 scope 链解析出来的（这里只有叶自己的表态，够用：Player = 系统不许动）。
      o.textContent = bp.name + '（' + shipClassName(bp.class) + '·' + normMode(bp.mode) + '）';
      o.selected = bp.name === b.blueprint;
      bpSel.appendChild(o);
    });
    if (b.blueprint && !bps.some((bp) => bp.name === b.blueprint)) {
      // **悬空指针**（图被改名/删掉了）：读面原样输出它，这里也必须显示出来——它意味着
      // **这个建造区停产**，静默吞掉就等于「失败看起来像成功」。
      const o = el('option', { value: b.blueprint });
      o.textContent = b.blueprint + '（库里没有这张图 ⇒ 本区停产）';
      o.selected = true;
      bpSel.appendChild(o);
    }
    bpSel.disabled = !bps.length && !b.blueprint;
    bpSel.addEventListener('change', () => {
      // `''` ⇒ `null`（**拆掉指针**，回到自动选装），给名字 ⇒ 指过去。
      pushModify(fid, cityId, b.id, { blueprint: bpSel.value || null });
      renderDiff();
    });
    wrap.appendChild(labelWrap('设计图', bpSel));
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

/// 回传 diff 里到底碰了几片叶（状态行诚实一点：空 diff 与"应用成功"必须看得出区别）。
function countDiffLeaves(req) {
  let n = 0;
  (req.control || []).forEach((fac) => {
    Object.keys(fac).forEach((k) => {
      if (k === 'faction_id') return;
      n += Array.isArray(fac[k]) ? fac[k].length : 1;
    });
  });
  if (req.scope) n++;
  return n;
}

// 「应用」只回传**差异**（见 buildCommandDiff）：没改过 ⇒ 空 diff ⇒ 幂等，而且不会把读面里
// 的**有效值**写进叶里（那会让这些舰从此不再跟随舰队默认 / 出厂快照，且事后无从归因）。
async function applyControl() {
  const req = buildCommandDiff();
  const n = countDiffLeaves(req);
  bindWorld(await postJSON('/api/command', req));
  buildEdits();
  renderAll();
  $('#status').textContent = n ? ('已应用 ' + n + ' 片叶的改动') : '没有改动要应用（只回传差异）';
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
