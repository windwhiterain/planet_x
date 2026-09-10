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
//     所以 State/GameConfig/RoundView 怎么改都不用动前端。

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
//
// ⚠ `null` 是**第四种**情况，不是「待命」：指令读面给的是**有效值**，而 `null` 的意思是
// **链上没有任何一层说话**（叶不存在；⚠ 指令链上 2026-10 起**没有**更高的一层了——
// 舰队默认指令与图上 order 两片叶都已删）。这时引擎
// 才按 `Idle` 兜底——把它显示成「待命」是拿兜底值冒充"有人说了待命"，所以单独一个 `unset`。
function behaviorType(b) {
  if (b === null || b === undefined) return 'unset';
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
    case 'unset': return '无人表态（按待命兜底）';
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
// 第三条风格轴**角色**不是 [-1,1] 的连续轴，也不是一个开关，而是**三选一的枚举**
// （引擎的 `ShipRole`，serde 的 JSON 形态就是这三个字符串）。它只决定自动控制派哪种活：
//   战舰   `War`     —— 找仗打（接战 / 轰炸 / 殖民）
//   运输舰 `Freight` —— 按积压定编去跑集货路线（`autocontrol::freight`）
//   观测舰 `Observe` —— 驻到太阳系外缘的引力异常区（MOND）蹲着，喂「掌握度」那条知识渠道
//                       （`autocontrol::knowledge`）——它是 MOND 掌握度**唯一**的知识来源
// 三态都不解除武装：运输舰 / 观测舰在射程内照样自动开火、照样按 kiting 姿态软移动。
// ⚠ 取值是**字符串**（`'War' | 'Freight' | 'Observe'`），不再是 `true`/`false`。
const ROLE_LABEL = { War: '战舰', Freight: '运输舰', Observe: '观测舰' };
/// 角色 → 中文标签。未知取值**原样显示**（引擎加了第四态时不会静默显示成"战舰"骗人）。
function roleSummary(l) {
  const r = l && l.role;
  return ROLE_LABEL[r] || r || '战舰';
}

// **「自动」这一档到底有没有执行者**——措辞必须与引擎一致（note：control-live-layers §3.2/§13）。
// 三种情况，三句不同的话：
//   ① 逐舰**风格两叶**（`ship_doctrine`/`ship_kiting`）：本轮**有执行者**了
//      （`autocontrol::style` 每回合按战况概率重估、写回叶片）⇒ 照实说「会改写」。
//   ② 逐舰**角色叶**（`ship_role`）：自动控制按积压定编 ⇒ 也照实说「会改写」。
//   ③ **势力级默认叶**（`default_doctrine`/`default_kiting`/`default_role`）：AI **不写**
//      这片叶，而且引擎的取值规则是"默认叶只在**它自己是玩家**时供值" ⇒ 它 `Auto` 时的存储值
//      是**没人读的**。诚实的说法是「本层不供值」，绝不能写成"值由系统写"。
//      （一句话解释这个组合：`Auto` 的默认叶 = AI 的答案是**"不设全舰队默认、逐舰自己说"**，
//       所以它确实不必写任何值——但界面必须把"那个数没人用"说出来。）
const AUTO_RETUNED = '自动（自动控制每回合按战况重估，会改写这片叶）';      // 逐舰风格两叶
const AUTO_WRITTEN = '自动（自动控制每回合按积压/观测需求定编，会改写这片叶）'; // 逐舰角色叶
const AUTO_UNWRITTEN = '自动（本层不供值：引擎只在它是玩家时才取默认值）'; // 势力级默认叶
/// 三组**字段名**：哪一片叶属于哪种措辞（`modeToggleFor` 与各行的 decorateLabel 都读它）。
/// 用字段名而不是 kind：`kind` 只是控制树内部的行键，字段名才是与引擎对齐的那一个
/// （manifest / 读面 / 补丁都用它）。
const AUTO_RETUNED_FIELDS = ['ship_doctrine', 'ship_kiting'];
const AUTO_WRITTEN_FIELDS = ['ship_role'];
const AUTO_UNWRITTEN_FIELDS = ['default_doctrine', 'default_kiting', 'default_role'];

/// 节点的字段名：新控制行（`controls.js`）直接给 `field`，旧控制树的节点在 `buildTree` 里补。
function leafFieldOf(node) { return (node && (node.field || node.kind)) || ''; }

// 势力级**默认风格**行的摘要：只有它自己是「玩家」时那个值才真的被采用（引擎的取值规则：
// 默认叶为 `Inherit`/`Auto` 时不供值），所以这两种情况都不显示那几个数——显示了会骗人。
function fleetStyleLabel(leaf, summary) {
  const m = normMode(leaf[modeField()]);
  if (m === 'Player') return ' · ' + summary(leaf);
  return m === 'Auto' ? ' · ' + AUTO_UNWRITTEN : ' · 未表态';
}

/// 逐舰**风格叶**的 `Auto` 补注：这片叶现在**真的有执行者**（自动控制按战况重估）。
function autoRetuned(leaf) { return normMode(leaf[modeField()]) === 'Auto' ? ' · ' + AUTO_RETUNED : ''; }
/// 逐舰**角色叶**的 `Auto` 补注：这片叶也有执行者（自动控制按积压定编），措辞另说一句。
function autoWritten(leaf) { return normMode(leaf[modeField()]) === 'Auto' ? ' · ' + AUTO_WRITTEN : ''; }

/// 「这个数现在是从哪来的」：引擎的取值链是
///   叶 `Inherit` + 舰队默认是 `Player` ⇒ 舰队默认的值；否则叶自己的值；没有叶 ⇒ 舰上记录值
/// （`State::ship_doctrine` / `ship_kiting`）。只在叶片**自己没表态**时显示——那时"你以为的
/// 归属"与"实际的来源"最容易错位，而这正是「改舰队默认对某艘舰没用」的成因。
function styleFollowHint(node) {
  const leaf = node.leaf;
  if (normMode(leaf[modeField()]) !== 'Inherit') return null;
  const doctrine = node.kind === 'shipdoctrine';
  const summary = doctrine ? doctrineSummary : kitingSummary;
  const fc = getControl(node.fid);
  const d = fc[DEFAULT_LEAF[node.kind]];
  if (d && normMode(d[modeField()]) === 'Player') {
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
/// ⚠ 角色是**三值枚举**，所以提示里的值一律过 [`roleSummary`]，并在「观测舰」那一档说明
/// 它到底去干什么（去异常区蹲着喂 MOND 掌握度）——只说"观测舰"三个字等于没说。
function roleFollowHint(node) {
  const leaf = node.leaf;
  if (normMode(leaf[modeField()]) !== 'Inherit') return null;
  const fc = getControl(node.fid);
  const d = fc.default_role;
  if (d && normMode(d[modeField()]) === 'Player') {
    return hintLine('当前跟随：舰队默认（' + roleSummary(d) + '·' + roleHint(d.role)
      + '）——它是玩家钉的 ⇒ 自动控制的逐舰定编不碰这艘舰');
  }
  const raw = ((st.control || {})[node.fid] || {}).ship_role || {};
  const s = st.ships.find((x) => x.name === node.id);
  if (raw[node.id]) {
    return hintLine('当前跟随：本舰叶片里的角色（' + roleSummary(leaf) + '·' + roleHint(leaf.role)
      + '；没表态 ⇒ 这是**自动控制写下的定编结论**，它每回合会重估；要钉死就把归属改成「玩家」）');
  }
  const rec = s ? roleSummary(s) : null;
  return hintLine('当前跟随：出厂快照' + (rec ? '（' + rec + '·' + roleHint(s.role) + '）' : '')
    + '——这一层还没有叶，自动控制随时可以给这艘舰定编');
}

/// 一个角色取值**到底在干什么**（`roleFollowHint` 与编辑器共用一句话，避免两处各说一套）。
function roleHint(r) {
  switch (r) {
    case 'Freight': return '按积压去跑集货路线';
    case 'Observe': return '驻在引力异常区（MOND），喂「掌握度」那条知识渠道';
    default: return '找仗打（接战 / 轰炸 / 殖民）';
  }
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
//             'kiting' 风筝↔贴脸姿态 | 'role' 角色三选一（战舰/运输舰/观测舰）|
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
  // `Auto` 时补一句实话：这两片叶本轮**真的有执行者**（自动控制每回合按战况重估，见 AUTO_RETUNED）。
  shipdoctrine: { childMode: 'leaf', scope: 'leaf', editor: 'doctrine', decorateLabel: (n) => ' · ' + doctrineSummary(n.leaf) + autoRetuned(n.leaf) },
  shipkiting:   { childMode: 'leaf', scope: 'leaf', editor: 'kiting', decorateLabel: (n) => ' · ' + kitingSummary(n.leaf) + autoRetuned(n.leaf) },
  // 势力级**三条默认**：指令 / 风格 / 风筝姿态。它们是「舰」这一组的前提（先定默认，例外才少写）。
  // 后两片与「舰队默认指令」同形，只是「风格」有两个轴：doctrine = 理智↔热血 + 护航↔独狼，
  // kiting = 风筝↔贴脸。摘要见 fleetStyleLabel（没表态就不显示数——那两个数还不算数）。
  fleetdoctrine: { childMode: 'leaf', scope: 'leaf', editor: 'doctrine', decorateLabel: (n) => fleetStyleLabel(n.leaf, doctrineSummary) },
  fleetkiting:   { childMode: 'leaf', scope: 'leaf', editor: 'kiting', decorateLabel: (n) => fleetStyleLabel(n.leaf, kitingSummary) },
  // 第三条风格轴**角色**（战舰 / 运输舰 / 观测舰）。两行与上面同形，但有一条轴间差别：逐舰那片叶
  // **自动控制每回合也会写**（按积压定编集货 + 派观测舰去异常区）⇒ 它的「自动」是真的（用 autoWritten）。
  shiprole: { childMode: 'leaf', scope: 'leaf', editor: 'role', decorateLabel: (n) => ' · ' + roleSummary(n.leaf) + autoWritten(n.leaf) },
  // 势力级默认角色：AI **不写**这片叶（它只写逐舰叶）⇒ 与另两条风格轴的默认叶同一条措辞。
  fleetrole: { childMode: 'leaf', scope: 'leaf', editor: 'role', decorateLabel: (n) => fleetStyleLabel(n.leaf, roleSummary) },
  resource:  { childMode: 'leaf', scope: 'leaf', editor: 'value', editorLabel: '投资预算/回合' },
  conbudget: { childMode: 'leaf', scope: 'leaf', editor: 'value', editorLabel: '建造预算/回合' },
  building:  { childMode: 'leaf', scope: 'leaf', editor: 'building' },
  // **设计图库**（势力级）：本势力「还不存在的舰」的出厂规格。一组 = 一张图一行 +
  // 末尾一条「＋ 新建设计图」。建造区那一行只写**指针**（指到库里的一张图），
  // 图的内容（舰级/选装/**倾向三轴**/归属）都在这里改。
  bpgroup:   { childMode: 'list' },
  blueprint: { childMode: 'leaf', scope: 'leaf', editor: 'blueprint' },
};

function scopeAccess(node) {
  const spec = KIND[node.kind] || {};
  // 旧控制树的 kind 在注册表里明说了 scope（容器是 null）；新控制行不注册 kind，但**它带的
  // 就是一片叶** ⇒ 默认 'leaf'（归属写在叶片自己的 `mode` 上）。
  const s = spec.scope === undefined ? (node.leaf ? 'leaf' : null) : spec.scope;
  if (s === null || s === undefined) return null;
  if (s === 'leaf') {
    const mf = modeField();
    return node.leaf ? { get: () => node.leaf[mf], set: (v) => { node.leaf[mf] = v; } } : null;
  }
  if (s === 'global') return { get: () => edScope.global, set: (v) => { edScope.global = v; } };
  return {
    get: () => scopeVal(edScope[s], node.id),
    set: (v) => setScopeVal(edScope[s], node.id, v),
  };
}

// --- 读面（组织点）---------------------------------------------------------
// 左侧面板有两个模式：**控制**（写面，手写的控制树）与**读面**（组织点声明出来的视图）。
// 读面的一切由 `web/static/views.json`（数据）驱动，渲染交给 `specview.js`（通用求值器）——
// 本文件只做三件事：把根喂给它、把「未组织」审计算出来、把点击接到选中读面上。
// 依据与铁律见 `.agents/notes/web-human-views.md`。
let viewsDoc = { pages: [], select: [] };   // views.json 的内容
let readPage = 0;                            // 读面当前页（views.json 的 pages 下标）
let readLoaded = false;                      // 视图数据是否加载成功（失败要说话，不是空白）
let readError = '';
const readExpanded = new Set();              // 读面里的展开状态（残差 / 上限）

// 组织点求值器要的「根」：六个 info 根 + 写面的读模板（control / scope）。
// control 是**读面**（有效值 + 叶的表态），跨根 join「这艘舰的指令是谁说的」就靠它。
function specRoot(name) {
  if (name === 'control') return world ? world.control : undefined;
  if (name === 'scope') return world ? world.scope : undefined;
  return infoValueOf(world, name);
}

function bindSpecView() {
  if (!window.SpecView) return;
  window.SpecView.bind({
    getRoot: specRoot,
    maps: {},
    onPathClick: copyPath,
    selection: sel,
    expanded: readExpanded,
    onSelect: (kind, name) => selectEntity(kind, name),
    // 写面那一半（`leaf` / `owner` / `action` 三种行）：求值器只把节点要过去，**不解释它**
    // ——与它不认识 bodies/cities 是同一条纪律。实现在 `web/static/controls.js`。
    controlNode: (col, rec, recKey, opts) => (window.Controls ? Controls.controlNode(col, rec, recKey, opts) : null),
  });
}

async function loadViews() {
  try {
    // 1) 引擎的**结构事实**先拉（控制行要知道「每片叶靠哪几个字段定位、值写在哪」）；
    // 2) 声明装饰（补标签 / 算 field）必须在 `setSpecs` **之前**；
    // 3) `buildEdits` 还要用这份 manifest 建差异回传的地基（`rebuildLeafSpec`）。
    if (window.Controls) await Controls.load();
    viewsDoc = await fetchJSON('views.json');
    if (window.Controls) Controls.decorateDoc(viewsDoc);
    if (window.SpecView) window.SpecView.setSpecs(viewsDoc);
    readLoaded = true;
  } catch (e) {
    readLoaded = false;
    readError = String(e && e.message ? e.message : e);
  }
}

// 左栏：**只有一栏**（页 tab + 页体）。以前是「读面 / 控制」两个模式，而写面住在另一个模式里
// ⇒ 「这个势力有多少库存」（读）与「我给它多少预算」（控制）要切一次 tab 才看得全。
// 现在四种行——`path`（读）/ `leaf`（叶）/ `owner`（归属）/ `action`（命令）——住在**同一份
// 声明、同一个数组**里，顺序就是穿插的顺序；旧控制树降级成一张普通页（第二步才删）。
function renderSide() {
  const side = $('#side');
  if (side) side.classList.add('read');
  const foot = $('.side-foot');
  // 「应用到服务器」每一页都在：控制行就住在读面那些页里。
  if (foot) foot.style.display = '';
  renderReadPanel();
}

function renderReadPanel() {
  const tabsBox = $('#readTabs');
  const body = $('#readBody');
  const treePanel = $('#treePanel');
  if (!tabsBox || !body) return;
  tabsBox.textContent = '';
  body.textContent = '';
  if (!readLoaded) {
    if (treePanel) treePanel.style.display = 'none';
    body.style.display = '';
    body.appendChild(el('div', { class: 'sv-empty' }, '视图声明（views.json）没加载上：' + readError));
    return;
  }
  const pages = sidePages();
  if (readPage >= pages.length) readPage = 0;
  pages.forEach((p, i) => {
    const t = el('span', { class: 'tnode-tab' + (i === readPage ? ' sel' : '') });
    t.textContent = p.title;
    t.addEventListener('click', () => { readPage = i; renderReadPanel(); });
    tabsBox.appendChild(t);
  });
  const page = pages[readPage];
  const isTree = page.id === 'tree-old';
  // 旧控制树那一页用**原来的 DOM**（`#treePanel` / `#tree`）：里面的设计图库、建筑面板都还在
  // 用它，第二步才搬进卡片（见 `views.json` 的 `write_omit` 与 note §5）。
  if (treePanel) treePanel.style.display = isTree ? '' : 'none';
  body.style.display = isTree ? 'none' : '';
  if (isTree) {
    const h = treePanel && treePanel.querySelector('.tree-old-hint');
    if (h && page.hint) h.textContent = page.hint;
    renderTree();
    return;
  }
  if (page.id === 'leftover') {
    renderSpecCheck(body);
    renderWriteCheck(body);
    renderLeftover(body);
    return;
  }
  if (page.hint) body.appendChild(el('div', { class: 'sv-pagehint' }, page.hint));
  (page.views || []).forEach((spec) => window.SpecView.renderView(body, spec));
}

// --- 「未组织」索引（铁律 R 的兜底一侧）--------------------------------------
// 引擎加了新根 / 新顶层集合 / 新字段 ⇒ 它们**自动**出现在这里（通用 widget 渲染），
// 不需要谁来更新前端。反过来，这里也把「某条被整理过的集合里，还有哪些字段没被认领」列出来。
function renderLeftover(body) {
  // 把每条组织点引用的路径归一成**段**（去掉 [*] / [?..] / ${..} 与 @根），供"谁整理了什么"用。
  const claimSegs = () => {
    const out = [];
    window.SpecView.claimedPaths(viewsDoc).forEach((c) => {
      const segs = String(c.expr)
        .split('.')
        .map((s) => s.replace(/\[.*$/, '').replace(/\$\{[^}]*\}/g, '·'))
        .filter((s) => s && s !== '·');
      if (!segs.length) return;
      const root = segs[0].replace(/^@/, '');
      out.push({ view: c.view, root, rest: segs.slice(1) });
    });
    return out;
  };
  const claimed = claimSegs();
  const views = (viewsDoc.pages || []).flatMap((p) => p.views || []).concat(viewsDoc.select || []);
  const specById = (id) => views.find((v) => v.id === id);

  ['state', 'pre', 'post', 'config', 'session', 'control', 'scope'].forEach((rootName) => {
    const root = specRoot(rootName);
    if (root === undefined || root === null) return;
    const wrap = el('div', { class: 'sv-view' });
    wrap.appendChild(el('h3', { class: 'sv-title' }, '@' + rootName));
    if (Array.isArray(root)) {
      const objs = root.filter((x) => x && typeof x === 'object' && !Array.isArray(x));
      if (!objs.length) {
        wrap.appendChild(el('div', { class: 'sv-hint' }, '数组：' + root.length + ' 项（未组织：整份由通用 widget 渲染）'));
        wrap.appendChild(jsonToggle('展开原始数据', () => root, rootName));
        body.appendChild(wrap);
        return;
      }
      // 「一个数组装着一批对象」（`@control` = 每势力一条）⇒ 把**键的并集**逐项标出来：
      // 已经被控制行认领的叶不能又被报成「未组织」（那会把「谁管着它」说反）。
      const keys = [];
      objs.forEach((o) => Object.keys(o).forEach((k) => { if (keys.indexOf(k) < 0) keys.push(k); }));
      wrap.appendChild(el('div', { class: 'sv-hint' },
        '数组：' + root.length + ' 项 × 对象；按**键的并集**逐项标（共 ' + keys.length + ' 个键）'));
      const list = el('div', { class: 'sv-list' });
      keys.forEach((k) => {
        const hits = claimed.filter((c) => c.root === rootName && c.rest[0] === k);
        const row = el('div', { class: 'sv-lrow' });
        row.appendChild(el('span', { class: hits.length ? 'sv-badge ok' : 'sv-badge' }, hits.length ? '已整理' : '未组织'));
        row.appendChild(el('span', { class: 'sv-lkey' }, k));
        const any = objs.find((o) => o[k] !== undefined);
        row.appendChild(el('span', { class: 'sv-lshape' }, shapeOf(any ? any[k] : null)));
        if (hits.length) row.appendChild(el('span', { class: 'sv-lby' }, '← ' + [...new Set(hits.map((c) => c.view))].join('、')));
        row.appendChild(jsonToggle('看原始数据', () => objs.map((o) => o[k]).filter((v) => v !== undefined), rootName + '[*].' + k));
        list.appendChild(row);
      });
      wrap.appendChild(list);
      body.appendChild(wrap);
      return;
    }
    const list = el('div', { class: 'sv-list' });
    Object.keys(root).forEach((k) => {
      const v = root[k];
      const hits = claimed.filter((c) => c.root === rootName && c.rest[0] === k);
      const direct = hits.filter((c) => c.rest.length === 1);
      const nested = hits.filter((c) => c.rest.length > 1);
      const row = el('div', { class: 'sv-lrow' });
      row.appendChild(el('span', (direct.length || nested.length) ? 'sv-badge ok' : 'sv-badge',
        (direct.length || nested.length) ? '已整理' : '未组织'));
      row.appendChild(el('span', { class: 'sv-lkey' }, k));
      row.appendChild(el('span', { class: 'sv-lshape' }, shapeOf(v)));
      if (direct.length) row.appendChild(el('span', { class: 'sv-lby' }, '← ' + [...new Set(direct.map((c) => c.view))].join('、')));
      // 嵌套集合（如 post.decisions 下的 ships/blueprints）：整理了哪些、还剩哪些没人认领。
      const nestedKeys = [...new Set(nested.map((c) => c.rest[1]))];
      if (nestedKeys.length && v && typeof v === 'object' && !Array.isArray(v)) {
        const all = Object.keys(v);
        const left = all.filter((x) => !nestedKeys.includes(x));
        row.appendChild(el('span', { class: 'sv-lby' }, '← ' + [...new Set(nested.map((c) => c.view))].join('、') + ' 整理了 ' + nestedKeys.join('、')));
        if (left.length) row.appendChild(el('span', { class: 'sv-lshape' }, '· 仍未组织：' + left.join('、')));
      }
      // 被整理过的集合：把「这条集合里还有哪些字段没人认领」摆出来（残差的集合级视图）。
      const spec = direct.length ? specById(direct[0].view) : null;
      if (spec && Array.isArray(v) && v.length) {
        const res = window.SpecView.residualOf(v[0], spec);
        if (res) row.appendChild(el('span', { class: 'sv-lshape' }, '· 按「' + spec.id + '」每条记录里未被认领：' + Object.keys(res).join('、')));
      }
      row.appendChild(jsonToggle('看原始数据', () => v, rootName + '.' + k));
      list.appendChild(row);
    });
    wrap.appendChild(list);
    body.appendChild(wrap);
  });
}

function shapeOf(v) {
  if (Array.isArray(v)) return '数组 ' + v.length + (v.length && typeof v[0] === 'object' ? ' × 对象' : '');
  if (v && typeof v === 'object') return '对象 ' + Object.keys(v).length + ' 键';
  return typeof v + ' ' + JSON.stringify(v);
}

// --- 运行时自检（用**真的求值器**跑一遍每条视图）------------------------------
// 静态纪律（`play/tests/g4_spec.py`：路径合文法、引用完整、每个控制叶都被某条行认领）只能保证
// **声明自己**没写错；「这一帧里这条列到底取不取得到值」只有拿求值器在真数据上跑一遍才知道。
// 所以放在这儿，**跟着当前这一帧**，永远不漂移：某列全帧取不到值 ⇒ 明说，而不是安静地显示一串「·」。
// ⚠ 控制行要跟读列**分开判**：`leaf` 行的值是 `null` / 空数组 = 「这一层还没表态」，那是**正常**的，
// 报成"取不到值"就是反方向的谎话（见 `controls.js` 的 `audit()`——写面自检在那边）。
function renderSpecCheck(body) {
  const wrap = el('div', { class: 'sv-view' });
  wrap.appendChild(el('h3', { class: 'sv-title' }, '视图自检（这一帧）'));
  wrap.appendChild(el('div', { class: 'sv-hint' },
    '用真的求值器把每条视图跑一遍：哪些列这帧取不到值、哪条来源整段落空。引擎改了字段名 ⇒ 这里立刻明说，而不是在表格里留一排「·」。'));
  const list = el('div', { class: 'sv-list' });
  const specs = (viewsDoc.pages || []).flatMap((p) => p.views || []).concat(viewsDoc.select || []);
  specs.forEach((spec) => {
    if (spec.mount === 'inline') return;
    let rows = [];
    try {
      rows = window.SpecView.expand(spec.source);
    } catch (e) {
      list.appendChild(checkRow(spec.id, 'warn', 'source 求值抛错：' + e));
      return;
    }
    if (!rows.length) {
      list.appendChild(checkRow(spec.id, 'warn', '这一帧没有记录（' + spec.source + '）'));
      return;
    }
    // 读行与控制行**分开说**：
    //   * 读行的值是"这一帧取不到" ⇒ 那是真问题（引擎改了字段名 / 这局没有），要报；
    //   * `leaf` 行的值是 `null` / 空数组 ⇒ 那是「**这一层还没有叶**」= 正常状态，
    //     界面给的是"写一个值就新建这片叶"的入口。把它报成"列全帧取不到值"是**反过来的
    //     谎话**（把能用的东西说成坏的）——本仓拉黑"失败看起来像成功"，这条是它的镜像。
    const readCols = (spec.columns || []).filter((c) => c.leaf == null && c.owner == null && c.action == null);
    const leafCols = (spec.columns || []).filter((c) => c.leaf != null);
    const dead = [];
    readCols.forEach((c) => {
      const any = rows.some((r) => !isNilLike(window.SpecView.evalPath(c.path, r.value, r.key)));
      if (!any) dead.push(c.path);
    });
    const noLeaf = leafCols.filter((c) => rows.every((r) => {
      const v = window.SpecView.evalPath(c.leaf, r.value, r.key);
      return isNilLike(v) || (Array.isArray(v) && !v.length);
    }));
    const nCtl = (spec.columns || []).length - readCols.length;
    const tag = nCtl
      ? ('（含 ' + nCtl + ' 条控制行' + (noLeaf.length ? '，其中 ' + noLeaf.length + ' 条的叶这一帧还不存在 = 这一层没表态，界面给新建入口' : '') + '）')
      : '';
    if (dead.length) {
      list.appendChild(checkRow(spec.id, 'warn',
        rows.length + ' 条记录 / ' + readCols.length + ' 个读列' + tag + '，其中 ' + dead.length + ' 列全帧取不到值：' + dead.join('、')));
    } else {
      list.appendChild(checkRow(spec.id, 'ok',
        rows.length + ' 条记录 / ' + readCols.length + ' 个读列' + tag + '全部取到了值'));
    }
  });
  wrap.appendChild(list);
  body.appendChild(wrap);
}

function isNilLike(v) {
  return v === null || v === undefined;
}

// --- 写面自检（与读面自检**对偶**）------------------------------------------
// 读面那条铁律管「没被认领的数据要看得见」；写面这一侧的对应物是：**引擎 `leaves` 里每一片叶
// 要么被某条 `leaf` 行认领、要么在 `write_omit` 里写了理由**——否则新加的叶在这个界面上
// **凭空消失**（改不了它），而那正是"静默藏"在写面的版本。
function renderWriteCheck(body) {
  if (!window.Controls) return;
  const rows = Controls.audit();
  const wrap = el('div', { class: 'sv-view' });
  wrap.appendChild(el('h3', { class: 'sv-title' }, '写面自检（引擎的 leaves ↔ 控制行）'));
  wrap.appendChild(el('div', { class: 'sv-hint' },
    '引擎发的结构事实（GET /api/control-schema）里每一片叶：被某条 leaf 行认领，或在 views.json 的 write_omit 里写明理由。两边对不上就在这儿说。'));
  const list = el('div', { class: 'sv-list' });
  rows.forEach((r) => list.appendChild(checkRow(r.field, r.ok ? 'ok' : 'warn', r.text)));
  if (!rows.length) list.appendChild(el('div', { class: 'sv-lrow' }, '（还没有清单：引擎没发 leaves / 前端没拉到）'));
  wrap.appendChild(list);
  body.appendChild(wrap);
}

function checkRow(id, kind, text) {
  const row = el('div', { class: 'sv-lrow' });
  row.appendChild(el('span', { class: kind === 'ok' ? 'sv-badge ok' : 'sv-badge warn' }, kind === 'ok' ? '✓' : '⚠'));
  row.appendChild(el('span', { class: 'sv-lkey' }, id));
  row.appendChild(el('span', { class: 'sv-lshape' }, text));
  return row;
}

// 一个「点开就地看原始 JSON」的小开关（未组织索引里每个键都有）。
function jsonToggle(label, get, rootPath) {
  const btn = el('span', { class: 'sv-more clickable' }, '▸ ' + label);
  const box = el('div', { class: 'sv-residual-box' });
  box.style.display = 'none';
  let built = false;
  btn.addEventListener('click', () => {
    const open = box.style.display === 'none';
    box.style.display = open ? '' : 'none';
    btn.textContent = (open ? '▾ ' : '▸ ') + label;
    if (open && !built) {
      built = true;
      if (window.JsonView) {
        window.JsonView.render(box, get(), { rootPath, expandDepth: 1, onPathClick: copyPath, rerender: renderReadPanel });
      } else box.textContent = JSON.stringify(get());
    }
  });
  return btn;
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
  bindSpecView();
}

async function init() {
  registerTab(); // 先报到：刷新时新页面的登记要尽早落地（服务端有个极短的刷新窗口）
  await loadViews(); // 视图声明先于第一帧（读面要用它）
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
  // 叶的**结构事实**先就位（`pairOrigins` / `buildCommandDiff` 都按它找叶）。
  rebuildLeafSpec();
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
//
// ⚠ 它**不再手抄**：内容来自引擎发的 `GET /api/control-schema`（`src/control/leaves.rs`）。
// 以前这张表、Python kit 的 `LEAF_KINDS`、引擎的结构体是**同一份事实的三份副本**，而漏改的
// 那一端不会报错——它只会静静地不认识那片叶（`role-axis-parity`、`blueprint-stance` 两次
// 踩的都是这个缺口）。现在：一份事实、三端共用。
const LEAF_SPEC = {};
/// 势力级那几片叶子（`Option<...>` 字段，不是数组）＝ manifest 里 `keys` 为空的那几条。
let LEAF_OPTIONS = [];

/// 按引擎的 manifest 重建上面两张表。`buildEdits` 里调（必须在 `pairOrigins` 之前）。
function rebuildLeafSpec() {
  Object.keys(LEAF_SPEC).forEach((k) => { delete LEAF_SPEC[k]; });
  const m = window.Controls && Controls.manifest ? Controls.manifest() : null;
  if (!m) { LEAF_OPTIONS = []; return false; }
  Object.keys(m.leaves).forEach((f) => {
    const l = m.leaves[f];
    LEAF_SPEC[f] = {
      keys: l.keys || [],
      values: l.values || [],
      carries: l.carries || [],
      read_only: l.read_only || [],
    };
  });
  LEAF_OPTIONS = Object.keys(LEAF_SPEC).filter((f) => !LEAF_SPEC[f].keys.length);
  return true;
}

/// 归属三态写在哪个字段里 / 删叶写在哪个字段里——由引擎发（不再散落一堆字面量）。
function modeField() { return (window.Controls && Controls.ownerField()) || 'mode'; }
function removeFieldName() { return (window.Controls && Controls.removeField()) || 'remove'; }

/// 这片叶跟着哪片「舰队默认」叶（`leaf_ui.<field>.follows`）——**轴间关系是前端的呈现**，
/// 引擎不知道「逐舰风格叶和舰队默认风格叶是同一条轴」这件事。
function followsOf(node) {
  const ui = (window.Controls && Controls.uiFor(node && node.field)) || {};
  return ui.follows || null;
}

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
///
/// ⚠ 原值必须**深拷贝**：`components` 是数组，浅拷贝会让"原值"与"编辑面那一片叶"共享同一个
/// 数组——叶里 `push/splice` 一改，原值也跟着变，于是"我明明改了选装，应用却没反应"。
function shellLeaf(leaf, spec) { return rememberOrigin(leaf, structuredClone(leaf), spec, true); }

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
    if (normMode(leaf[modeField()]) === 'Inherit') leaf[modeField()] = 'Player';
    return;
  }
  if (leafValueChanged(leaf)) {
    if (normMode(leaf[modeField()]) === 'Inherit') { leaf[modeField()] = 'Player'; autoPinned.add(leaf); }
  } else if (autoPinned.has(leaf)) {
    leaf[modeField()] = normMode(o.fields[modeField()]);
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
  const mode = normMode(leaf[modeField()]);
  if (mode !== normMode(o.fields[modeField()])) { out[modeField()] = mode; changed = true; }
  else if (o.shell && changed) out[modeField()] = mode;
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
  controlRerender();
  renderSelection();
}

function getControl(fid) {
  let c = edControl.find((x) => x.faction_id === fid);
  if (!c) {
    // 桶按 manifest 建：**键叶 = 数组**（每片叶一个条目）、**单叶 = null**（还没有叶，
    // 由写面按需造壳——「没有叶」与「有叶但没表态」不是一回事，见 `controls.js`）。
    c = { faction_id: fid, buildings: [] };
    Object.keys(LEAF_SPEC).forEach((f) => { c[f] = LEAF_SPEC[f].keys.length ? [] : null; });
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
  bindSpecView();
  renderMap();
  renderSide();      // 左栏：读面（组织点声明出来的页；旧控制树是其中一页）
  renderSelection();
  renderDiff();
  renderInfo();
  renderApplyGate();
}

function updateTop() {
  $('#metaRound').textContent = '回合 ' + st.round + ' · ' + st.time_month + ' 个月';
  const line = $('#nowLine');
  if (line) line.textContent = nowSummary();
}

// 「本回合」一句话：**只报事实，不做判断**（没有阈值、没有高亮——那属于注意力路由，本轮不做）。
// 事件类型的英文 key → 中文标签取自 views.json 里那条声明（一处来源，不在这里再抄一份）。
function nowSummary() {
  const events = st.events || [];
  const labels = (() => {
    const v = (viewsDoc.pages || []).flatMap((p) => p.views || []).find((x) => x.id === 'events');
    const col = v && (v.columns || []).find((c) => c.path === 'type');
    return (col && col.map) || {};
  })();
  const counts = new Map();
  events.forEach((e) => {
    const k = labels[e.type] || (e.type + '（无中文标签）');
    counts.set(k, (counts.get(k) || 0) + 1);
  });
  const top = [...counts.entries()].sort((a, b) => b[1] - a[1]).slice(0, 4)
    .map(([k, n]) => k + (n > 1 ? ' ×' + n : '')).join(' · ');
  const chron = st.chronicle || [];
  const last = chron.length ? chron[chron.length - 1] : null;
  const fresh = last && last.round === st.round ? '｜新故事：' + last.title : '';
  if (!events.length && !fresh) return '本回合没有事件';
  return '本回合 ' + events.length + ' 件事' + (top ? '（' + top + '）' : '') + fresh;
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
  if (kind === 'faction') setFaction(name); // 顺带把控制树切到这个势力（写面）
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

// 选中读面：**先问组织点**（`select` 挂载）——命中就用整理过的卡片 + 「其余字段」；
// 没命中就退回通用 widget 渲染整份记录（老行为，一条也不少）。
function renderSelection() {
  const head = $('#selHead');
  const box = $('#readout');
  if (!head || !box) return;
  head.textContent = '';
  box.textContent = '';
  if (!sel) {
    box.textContent = '（在地图上点天体/城/舰，或点左侧控制树/读面里的名字，这里会显示它的记录）';
    return;
  }
  const found = findEntity(sel.kind, sel.name);
  if (!found) {
    head.textContent = (KIND_LABEL[sel.kind] || sel.kind) + ' ' + sel.name + '（当前世界里已不存在）';
    return;
  }
  const label = el('span', { class: 'sel-kind' }, KIND_LABEL[sel.kind] || sel.kind);
  const nameEl = el('span', { class: 'sel-name clickable' }, sel.name);
  nameEl.title = '点击复制 JSON 路径';
  nameEl.addEventListener('click', () => copyPath(found.path));
  head.append(label, nameEl, el('span', { class: 'sel-path' }, found.path));

  const spec = window.SpecView && window.SpecView.selectSpecFor(sel.kind);
  if (spec) {
    head.appendChild(el('span', { class: 'sel-path' }, '组织点：' + spec.id));
    window.SpecView.renderSelect(box, spec, sel.name);
    return;
  }
  head.appendChild(el('span', { class: 'sel-path' }, '（这个类型没有组织点，按通用 widget 显示）'));
  window.JsonView.render(box, found.value, {
    rootPath: found.path,
    expandDepth: 1,
    state: { expanded: selExpanded },
    onPathClick: copyPath,
    inline: inlineSpec,
    rerender: renderSelection,
  });
}

// 通用树里的**原位重组点**：给 jsonview 的钩子。命中就返回节点，没命中返回 null（走通用渲染）。
function inlineSpec(path, value) {
  if (!window.SpecView || !value || typeof value !== 'object') return null;
  const spec = window.SpecView.inlineFor(path);
  if (!spec) return null;
  const box = el('div', { class: 'sv-inline' });
  const bar = el('div', { class: 'sv-inline-bar' });
  bar.textContent = '此处由组织点「' + spec.id + '」整理（原始字段在每行的「其余字段」里）';
  box.appendChild(bar);
  window.SpecView.renderView(box, spec);
  return box;
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
    // 势力级**三条长期倾向**（风格 / 风筝姿态 / 角色）是可编辑叶片：新舰出生就继承它们。
    // 它们排在「舰」分组**之前**——因为它们是这一组的前提（先定倾向，例外才少写）。
    // 读面里没有这条叶 = 没有人表态，这里补一片 `Inherit` 的**壳**让它出现在树上（与作用域里
    // 「没列出的层 ≡ 继承」同义）。壳只用于显示：没被动过就不会进回传 diff，动过就按
    // 「新建这片叶」整片发出去（见 shellLeaf）。
    //
    // ⚠ **指令**没有势力级那一片了（2026-10 用户裁决）：指令是**即时操作**，
    // 只写逐舰叶；舰队级只留长期倾向这三片。
    const fleetDoctrine = shellLeaf(fc.default_doctrine = fc.default_doctrine
      || { temper: 0, lone_wolf: 0, mode: 'Inherit' }, LEAF_SPEC.default_doctrine);
    const fleetKiting = shellLeaf(fc.default_kiting = fc.default_kiting
      || { kiting: 0, mode: 'Inherit' }, LEAF_SPEC.default_kiting);
    const fleetRole = shellLeaf(fc.default_role = fc.default_role
      || { role: 'War', mode: 'Inherit' }, LEAF_SPEC.default_role);

    const invLeaves = (fc.investment_budget || []).map((e) => ({
      key: 'inv' + fid + ':' + e.resource, kind: 'resource', field: 'investment_budget',
      name: resName(e.resource), leaf: e, fid,
    }));
    const conLeaves = (fc.construction_budget || []).map((e) => ({
      key: 'con' + fid + ':' + e.resource, kind: 'conbudget', field: 'construction_budget',
      name: resName(e.resource), leaf: e, fid,
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
          b, fid, cityId: ci.name, city: ci,
        });
      });
      bn.children.push(cn);
    });

    const cats = [];
    // **设计图库**（本势力「还不存在的舰」的出厂规格）：排在最前——建造区那一行的下拉
    // 只能指向**库里已有的图**，先有库、才有指针。空库也显示（不然你没法建第一张图）。
    const bpNodes = (fc.blueprints || []).map((bp) => blueprintNode(fc, bp));
    cats.push({
      key: 'gbp' + fid, kind: 'bpgroup', id: fid, fid,
      name: '设计图库（' + bpNodes.length + ' 张）', children: bpNodes,
    });
    if (shipNodes.length || fleetDoctrine) {
      const kids = [
        { key: 'fleetdoc' + fid, kind: 'fleetdoctrine', field: 'default_doctrine', id: fid, name: '舰队默认风格', leaf: fleetDoctrine, fid },
        { key: 'fleetkit' + fid, kind: 'fleetkiting', field: 'default_kiting', id: fid, name: '舰队默认风筝姿态', leaf: fleetKiting, fid },
        { key: 'fleetfrt' + fid, kind: 'fleetrole', field: 'default_role', id: fid, name: '舰队默认角色', leaf: fleetRole, fid },
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
// **风筝姿态**（风筝↔贴脸）+ **角色**（战舰 / 运输舰 / 观测舰）。四片叶的归属链各自独立，所以
// 「归谁」的下拉跟着子叶走。
//
// **四片叶的读面都「每舰一行」**（哪怕状态里根本没有那片叶），口径也一致：
// 值 = **有效值**（指令：只有叶；风格/角色：叶 → 出厂图 → 舰队默认 → 舰上记录值）、
// mode = 这片叶自己的表态（没有叶 = `Inherit`）。所以这里显示的就是「这艘舰现在实际用的」。
// ⚠ 这条对**指令**（`ship_orders`）以前不成立：那片叶只列**有叶的舰**，于是「恢复出厂值」
// 一按，这艘舰**整行**（连风格 / 角色）就从控制树里消失——现在也不会了。
// ⚠ 指令的 `behavior === null` 是「链上没人说话」（引擎按 `Idle` 兜底），**不是**「待命」；
// 风格三轴没有这个问题（它们兜底到出厂记录值，永远有一个数）。
// ⚠ `Ship.doctrine` / `Ship.kiting` / `Ship.role` 只是出厂快照（**记录值**），改它们没有
// 任何控制效果——那正是「我明明改了风格却没反应」的坑；要改就走这几片叶。
function shipNode(fc, ord) {
  const name = ord.ship;
  const s = st.ships.find((x) => x.name === name);
  const kids = [{ key: 'ord' + name, kind: 'shiporder', field: 'ship_orders', id: name, name: '指令', leaf: ord, fid: fc.faction_id }];
  // 指令行是**每舰一行**来的，所以正常路径下这艘舰一定在 `st.ships` 里；`s` 缺席只剩
  // 「`/api/state` 的 info 树与 control 段来自不同时刻」这种边缘情况（那时风格叶写进去也只会被丢弃）。
  if (s) {
    kids.push({
      key: 'doc' + name, kind: 'shipdoctrine', field: 'ship_doctrine', id: name, name: '风格', fid: fc.faction_id,
      leaf: styleLeaf(fc, 'ship_doctrine', name, {
        temper: +((s.doctrine || {}).temper) || 0,
        lone_wolf: +((s.doctrine || {}).lone_wolf) || 0,
      }),
    });
    kids.push({
      key: 'kit' + name, kind: 'shipkiting', field: 'ship_kiting', id: name, name: '风筝姿态', fid: fc.faction_id,
      leaf: styleLeaf(fc, 'ship_kiting', name, { kiting: +s.kiting || 0 }),
    });
    // 角色：这片叶**自动控制每回合会写**（按积压定编集货 + 按观测需求派舰去异常区），所以它
    // 常常带着一个玩家没写过的值 + `Inherit` 表态——`autoWritten` 会在那一行把这件事说清楚。
    // 读面这一行的值 = **有效角色**（`ships[].role`，链上算完的结论），不是舰上的出厂快照。
    kids.push({
      key: 'frt' + name, kind: 'shiprole', field: 'ship_role', id: name, name: '角色', fid: fc.faction_id,
      leaf: styleLeaf(fc, 'ship_role', name, { role: s.role || 'War' }),
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
  if (!tree) return;
  tree.innerHTML = '';
  tree.appendChild(renderNode(buildTree()));
}

/// 左栏的页：**声明里的页** + 「未组织」+ 「控制树（旧）」。
/// 旧的写面（手写控制树）降级成一张普通页——第二步才删它（设计图库 / 建筑那些手写面板还在
/// 里面）；新的控制行住在「势力 / 舰队 / 城市」那几页里，**与读行挨着**，那才是这一步的重点。
function sidePages() {
  return (viewsDoc.pages || []).concat([
    { id: 'leftover', title: '未组织', hint: '没有被任何组织点认领的数据 —— 它们照旧由通用 widget 渲染' },
    {
      id: 'tree-old',
      title: '控制树（旧）',
      hint: '手写的控制层级（全局 · 势力 · 天体 · 城 · 设计图 · 叶片）。这一页是**过渡**：'
        + '新的控制行住在「势力 / 舰队 / 城市」几页里，与读行同一条行序；第二步把它删掉。',
    },
  ]);
}

function treePageIndex() { return sidePages().length - 1; }

/// 控制面重画：控制行既可能在读面那些页里，也可能在旧控制树那一页——**两者共用
/// `renderLeafNode`**，所以改完值只要重画「看得见的那一面」。以前这里无脑 `renderTree()`：
/// 在新控制行上改一个数，重画的是那棵没在看的树（值看起来"没有反应"）。
function controlRerender() {
  if (readPage === treePageIndex()) renderTree();
  else renderReadPanel();
}

/// 「应用到服务器」的可用性：引擎的控制面清单没拉回来 ⇒ 回传的 diff 会**空着发出去**
/// ——那是"看起来成功、其实什么都没写"。所以直接禁掉按钮，并把原因写在按钮上。
function renderApplyGate() {
  const btn = $('#applyBtn');
  if (!btn) return;
  const err = window.Controls && Controls.manifestError ? Controls.manifestError() : null;
  btn.disabled = !!err;
  btn.textContent = err ? '⚠ 控制面清单拉不到，不能应用' : '✓ 应用到服务器';
  btn.title = err
    ? ('GET /api/control-schema 失败：' + err + '（没有它，前端不知道每片叶靠哪几个字段定位）')
    : '把改动作为**差异**回传（只发变过的字段；没改过就是空 diff）';
}

/// 层级树的一个节点。**容器**（全局 / 势力 / 天体 / 城 / 舰 / 设计图库 / 分组）在这里搭结构；
/// **叶片那一行**交给 [`renderLeafNode`]——新的声明式控制行（`controls.js`）用的是同一个函数。
function renderNode(node) {
  const spec = KIND[node.kind] || {};
  const hasKids = node.children && node.children.length;

  if (spec.childMode === 'list' || spec.childMode === 'tabs') {
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

    // 一条舰 = 一个四叶容器：容器这一行只留一个**便利**下拉（「这四片叶一起归谁」）。
    if (spec.bulkOwnership) {
      const bulk = bulkOwnershipSelect(node);
      if (bulk) head.appendChild(bulk);
    }
    wrap.appendChild(head);

    if (spec.childMode === 'list') {
      const kids = el('div', { class: 'tnode-kids' });
      if (hasKids) node.children.forEach((ch) => kids.appendChild(renderNode(ch)));
      if (node.kind === 'city') kids.appendChild(addBuildingButton(node));
      if (node.kind === 'bpgroup') {
        if (!hasKids) {
          kids.appendChild(hintLine('库里还没有图：在下面建一张（图名 / 舰级 / 选装 / 角色 / 风格 / 风筝姿态），再到底下「天体 → 城 → 建造区」那一行的「设计图」下拉把它指过去——先有库，才有指针。'));
        }
        kids.appendChild(addBlueprintButton(node));
      }
      wrap.appendChild(kids);
      return wrap;
    }

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
          controlRerender();
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

  return renderLeafNode(node, {});
}

/// 一片叶的**一整行**：`<div class="tnode" data-key>` + head（标签 / 归属 / 恢复继承 /
/// 恢复出厂值）+ 编辑器 + 「这个数现在从哪来」的跟进提示。
///
/// **旧控制树与新的声明式控制行共用这一段**（`renderNode` 与 `controls.js` 的 `controlNode`
/// 都调它）⇒ 归属三态、写值即接管、删叶、编辑器种类这些规则**只有一处**，不会两边各说一套。
///
/// `opts`（只有新控制行会给）：
///   * `noLabel`  不画标签（那一行的标签由卡片/表格的列头给）
///   * `alwaysEditable` 不看「有效归属是玩家」那道闸门：新控制行的值旁边就是归属 chip，
///     闸门由 chip 表达；而「还没有叶」的行更必须能写值——那是新建这片叶的唯一入口
///   * `hint`     行首的一句实话（例如「没有叶（这一层没表态）」）
///   * `carry`    身份是数字 id 的叶用它说清「这是哪座楼」（`invest_weights`）
function renderLeafNode(node, opts) {
  const o = opts || {};
  const spec = KIND[node.kind] || {};
  const mf = modeField();
  const wrap = el('div', { class: 'tnode' + (node.ctl ? ' ctl-leaf' : ''), 'data-key': node.key });
  const head = el('div', { class: 'tnode-head' });

  if (!o.noLabel) {
    const lbl = el('span', { class: 'tnode-label' });
    let labelText = String(node.name == null ? '' : node.name);
    if (spec.decorateLabel) labelText += spec.decorateLabel(node, st);
    if (o.carry) labelText += '（' + o.carry + '）';
    lbl.textContent = labelText;
    if (node.color) lbl.style.color = node.color;
    head.appendChild(lbl);
  }

  const mt = modeToggleFor(node);
  if (mt) head.appendChild(mt);
  // 「恢复继承」：撤销这片叶的**表态**（mode → 继承），值不动。只在它自己有表态时出现——
  // 那时"我想反悔"才有意义（把下拉调回「继承」等价，但这个按钮把撤销写在脸上）。
  if (node.leaf && normMode(node.leaf[mf]) !== 'Inherit') head.appendChild(restoreInheritButton(node.leaf));
  // 「恢复出厂值」/「删掉这张图」：**删掉这片叶**（取值真的回到上层/出厂快照；设计图那一片
  // 叶就是整张图）。只在状态里真的有这片叶时出现。
  if (node.leaf && rawLeafOf(node)) {
    head.appendChild(leafFieldOf(node) === 'blueprints'
      ? removeLeafButton(node, '删掉这张图', '删掉整张设计图（`{"name":…,"remove":true}`）：挂它的建造区随后是悬空指针 ⇒ 本区停产（进度不再涨，Q10(a)），已下水的舰不受影响（快照）。点「应用到服务器」才生效。')
      : removeLeafButton(node));
  }
  // 新控制行的行首一句实话（「没有叶（这一层没表态）」这类）。
  if (o.hint) head.appendChild(el('span', { class: 'ctl-none' }, o.hint));
  wrap.appendChild(head);

  // 被标记删除的叶：不再给编辑器（点「应用」它就没了），只说明会发生什么。
  if (node.leaf && removedLeaves.has(node.leaf)) {
    wrap.appendChild(hintLine(leafFieldOf(node) === 'blueprints'
      ? '已标记删除：点「应用到服务器」之后这张图从库里消失，'
        + yardCountText(node.fid, node.id)
        + '（再点一次按钮可撤销）'
      : '已标记删除：点「应用到服务器」之后这片叶消失，取值回到上层 / 出厂快照（再点一次按钮可撤销）'));
    return wrap;
  }

  const editor = node.editor || spec.editor;
  // 编辑器对「**有效归属**是玩家」的叶子开放，而不是只看叶子自己的 mode：舰队默认设成玩家
  // 之后，继承它的舰也归你管——以前那些舰在 UI 上连编辑器都没有。
  // 改值时会把叶子显式钉成 Player（写值即接管，与 `--apply` 同一条规则）。
  // 新控制行（`alwaysEditable`）不看这道闸门：它的归属 chip 就在值旁边，而「还没有叶」的行
  // 必须能写值——否则「给一个还没有叶的资源设预算」在界面上无路可走。
  const open = !!o.alwaysEditable || effectiveMode(node) === 'Player';
  // 数值叶的标签：旧控制树把它画在编辑器里（`editorLabel`）；新控制行的标签由行/列给。
  const numLabel = node.ctl ? '' : spec.editorLabel;
  // 编辑器种类是**声明**（`views.json` 的 `leaf_ui.<field>.editor`），一张封闭的注册表
  // ——与读面的 `fmt` 一个道理。`behavior` 与旧控制树的 `ship` 是同一件事（`shipEditor`）。
  if (editor === 'ship' || editor === 'behavior') {
    if (open) wrap.appendChild(shipEditor(node.leaf, node));
    else wrap.appendChild(hintLine('由系统自动决定（要自己指挥就把左边的归属改成「玩家」）'));
  } else if (editor === 'doctrine') {
    if (open) wrap.appendChild(doctrineEditor(node));
    else wrap.appendChild(hintLine('由系统自动决定（要自己定风格就把左边的归属改成「玩家」）'));
  } else if (editor === 'kiting') {
    if (open) wrap.appendChild(kitingEditor(node));
    else wrap.appendChild(hintLine('由系统自动决定（要自己定风筝姿态就把左边的归属改成「玩家」）'));
  } else if (editor === 'role') {
    // 角色（战舰 / 运输舰 / 观测舰，单片叶）。这里「由系统自动决定」**不是空话**——自动控制
    // 每回合按积压与观测需求定编，所以提示要说清它真的会替你决定。
    if (open) wrap.appendChild(roleEditor(node));
    else if (leafFieldOf(node) === 'default_role') {
      // ⚠ 势力级这片默认叶**没有执行者**：自动控制只写逐舰角色叶，从不写它。
      // 所以这里不能照抄逐舰那句「由自动控制定编」——那是假话（note §3.2 的措辞纪律）。
      wrap.appendChild(hintLine('未表态：这片默认叶只在它自己是「玩家」时才供值（自动控制的定编只写逐舰角色叶，不写它）——要「全舰队听我的」就把它改成「玩家」'));
    } else {
      wrap.appendChild(hintLine('由自动控制定编（它每回合按积压派集货、按观测需求派舰去异常区蹲着喂 MOND 掌握度）；要自己钉死这艘舰，就把左边的归属改成「玩家」'));
    }
  } else if (editor === 'value' || editor === 'number') {
    wrap.appendChild(leafValueEditor(node.leaf, numLabel, node, { force: !!o.alwaysEditable }));
  } else if (editor === 'body') {
    // 迁都（`capital` 这片叶）：值是一个天体名。旧控制树从来没给它入口，新控制行把它补上。
    wrap.appendChild(bodyEditor(node.leaf, node, { force: !!o.alwaysEditable }));
  } else if (editor === 'building') {
    wrap.appendChild(buildingEditor(node));
  } else if (editor === 'blueprint') {
    // 设计图：**总是**给编辑器（图是玩家自己建的，没有"系统替你决定"这一档）。
    wrap.appendChild(blueprintEditor(node));
    const oh = blueprintOwnershipHint(node);
    if (oh) wrap.appendChild(oh);
  } else if (editor) {
    wrap.appendChild(hintLine('（没有「' + editor + '」这个编辑器：views.json 的 leaf_ui 写了一种前端不认识的编辑器）'));
  }

  // 「这个数现在从哪来」：新控制行按**有效归属**说一句实话；旧控制树照旧用那两句逐轴提示
  // （它们说的是"跟随舰队默认 / 出厂快照"，只在逐舰叶上有意义）。
  const f = leafFieldOf(node);
  if (o.alwaysEditable) {
    const fh = ctlFollowHint(node);
    if (fh) wrap.appendChild(fh);
  } else if (f === 'ship_doctrine' || f === 'ship_kiting') {
    const fh = styleFollowHint(node);
    if (fh) wrap.appendChild(fh);
  } else if (f === 'ship_role') {
    const fh = roleFollowHint(node);
    if (fh) wrap.appendChild(fh);
  }
  return wrap;
}

/// 新控制行的一句实话：这片叶现在**谁在供值**，以及「改一个值」会发生什么。
/// （逐舰那两句 styleFollowHint / roleFollowHint 说的是"跟随舰队默认 / 出厂快照"，
/// 它们靠舰上的记录值，只对逐舰叶成立；这里说的是**通用**的那条链。）
function ctlFollowHint(node) {
  const own = normMode(node.leaf[modeField()]);
  if (own === 'Player') return null;
  if (own === 'Auto') return hintLine('归属「自动」：系统每回合可以改写这片叶（改一个值就归你）');
  return hintLine('这片叶没表态（继承）⇒ 现在按上层：' + effectiveMode(node) + '；改一个值就归你（写值即接管）');
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
    leaf[modeField()] = 'Inherit';
    autoPinned.delete(leaf);
    controlRerender();
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
  const modes = leaves.map((l) => normMode(l[modeField()]));
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
    leaves.forEach((l) => { l[modeField()] = sel.value; autoPinned.delete(l); });
    controlRerender();
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
///
/// **身份键来自引擎的 manifest**：叶条目自己就带着那几个键（`ship` / `resource` /
/// `city`+`building` / `name`），这里只按它挑一次——以前这张「kind → 名单 + 取键闭包」的表
/// 是手抄的第四份副本（已删）。
function rawLeafOf(node) {
  if (!node.field) return null;   // 没有字段名的节点（如建筑行）本来就没有自己的叶
  const raw = (st.control || {})[node.fid];
  if (!raw) return null;
  const bucket = raw[node.field];
  if (bucket == null) return null;
  const spec = LEAF_SPEC[node.field];
  if (!spec || !spec.keys.length) return bucket;
  const leaf = node.leaf || {};
  // ⚠ 原始 state 里的**键叶是映射**（键 = 第一个身份键），而读面给的是**数组**
  // （`control_view` 把映射摊成"一行一片"）——两种形状都要认：
  //   `investment_budget: {"碳": {value, mode}}`（原始） vs `[{resource:"碳", …}]`（读面）。
  // 多键叶（城 + 建筑）在原始状态里的键是元组键的线格式：`"亚特兰大|10"`。
  if (Array.isArray(bucket)) {
    return bucket.find((e) => spec.keys.every((k) => String(e[k]) === String(leaf[k]))) || null;
  }
  if (typeof bucket === 'object') {
    const k0 = String(leaf[spec.keys[0]]);
    if (bucket[k0]) return bucket[k0];
    const kk = spec.keys.map((k) => String(leaf[k])).join('|');
    if (bucket[kk]) return bucket[kk];
  }
  return null;
}

/// 「恢复出厂值」= **删掉这片叶**。它与「恢复继承」不是一回事（见上面的说明），所以两个
/// 动作并存、各自写在按钮上：「恢复继承」= 交还归属；「恢复出厂值」= 把这个数也还回去。
function removeLeafButton(node, label, title) {
  const marked = removedLeaves.has(node.leaf);
  const b = el('button', {
    class: 'restore rm-leaf' + (marked ? ' on' : ''), 'data-role': 'remove-leaf',
    title: title || '删掉这片叶（不是清空）：这一层不再说话，取值回到上层 / 出厂快照。点「应用到服务器」才生效。',
  }, marked ? '取消删除' : (label || '恢复出厂值'));
  b.addEventListener('click', () => {
    if (marked) removedLeaves.delete(node.leaf);
    else removedLeaves.add(node.leaf);
    controlRerender();
  });
  return b;
}

/// 一片叶子的**有效归属**：叶子自己 → （舰：**该轴对应的**舰队默认叶）→ 势力 → 全局。
/// 这是 `State::ship_control` / `ship_doctrine_control` / `ship_kiting_control` /
/// `ship_role_control` 在前端的对应读法，UI 用它决定「这片叶子现在归谁、能不能编辑」。
///
/// ⚠ **指令没有舰队默认叶**（2026-10 裁决）：它是即时操作，链上是 叶 → 势力 → 全局；
/// 风格/姿态/角色三条长期倾向各有一片势力级默认（见 [`DEFAULT_LEAF`]）。
/// 出厂图那一层 UI 不重算（读面给的 `mode` 是叶自己的表态，图层的归属由引擎解析）——
/// 这里少一层只会让 UI **更保守**（少显示一次「已归玩家」），不会让编辑误伤引擎。
// `DEFAULT_LEAF`（kind → 那片「舰队默认」叶）已删：这条轴间关系现在由 `views.json` 的
// `leaf_ui.<field>.follows` 声明，节点按 `field` 查（见 `followsOf`）。

function effectiveMode(node) {
  const own = normMode(node.leaf && node.leaf[modeField()]);
  if (own !== 'Inherit') return own;
  const dkey = followsOf(node);
  if (dkey) {
    const fc = getControl(node.fid);
    const d = normMode(fc[dkey] && fc[dkey][modeField()]);
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
  const isBp = leafFieldOf(node) === 'blueprints';
  // 这片叶上的「自动」有没有执行者（措辞必须与引擎一致，见上面三组 kind 的说明）。
  // ⚠ 风格三轴的逐舰叶**本轮起真的有执行者**（`autocontrol::style`），所以它不再算「冻住」；
  // 势力级默认叶仍然没人写（`AUTO_UNWRITTEN_FIELDS`），措辞必须分开说。
  const f = leafFieldOf(node);
  const retuned = AUTO_RETUNED_FIELDS.indexOf(f) >= 0;
  const written = AUTO_WRITTEN_FIELDS.indexOf(f) >= 0;
  const unwritten = AUTO_UNWRITTEN_FIELDS.indexOf(f) >= 0;

  const sel = el('select', { class: 'mode', 'data-role': 'mode' });
  [['Inherit', '继承'], ['Auto', '自动'], ['Player', '玩家']].forEach(([v, l]) => {
    const o = el('option', { value: v });
    o.textContent = l;
    // 「自动」的措辞必须诚实：有执行者的轴照实说"会来写"，势力级默认叶则**不供值**。
    if (v === 'Auto') {
      if (isBp) {
        o.title = '自动：自动控制会把这张图的舰级改成它算出来的目标级（`retool_shipyards`），'
          + '并且会自己**建图 / 重估 / 回收**（`autocontrol::blueprints`，按 (舰级, 选装) 去重）。'
          + '归属是「玩家」的图它绝不碰。';
      } else if (retuned) {
        o.title = '由自动控制每回合按战况重估（风格三轴的逐舰叶真的有执行者）';
      } else if (written) {
        o.title = '由 AI 每回合按积压定编（指令 / 预算 / 角色轴真的有执行者）';
      } else if (unwritten) {
        o.title = '这一层不供值：引擎只在它是「玩家」时才取默认值——选「自动」等于说"不设全舰队默认，逐舰自己说"';
      } else {
        o.title = '由 AI 每回合按局势重估（指令 / 预算轴真的有执行者）';
      }
    } else if (v === 'Inherit') {
      o.title = isBp
        ? '撤销这一层的表态：设计图的链是 图叶 → 势力作用域 → 全局（没有「舰队默认」这一档），链上都没表态就是「自动」'
        : '撤销这一层的表态：向上层要答案（舰队默认 / 势力 / 全局），风格轴还会落到出厂快照';
    } else {
      o.title = isBp
        ? '归你管：自动控制不再改写这张图的舰级与选装（已下水的舰不受影响——选装是出厂快照）'
        : '归你管：系统不再改写它';
    }
    o.selected = normMode(mode) === v;
    sel.appendChild(o);
  });
  sel.title = isBp
    ? '谁负责这张设计图（图叶 → 势力 → 全局）'
    : (retuned ? '谁负责这片风格叶（自动 = 自动控制每回合按战况重估）' : '谁负责这片叶');
  sel.addEventListener('change', () => {
    set(sel.value);
    // 显式选过模式 = 一次明确的表态：撤回「写值即接管」的书签——值再变回原数也不该翻掉它。
    if (node.leaf) autoPinned.delete(node.leaf);
    controlRerender();
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
  const mode = normMode(node.leaf[modeField()]);
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

/// **角色**编辑器（战舰 / 运输舰 / 观测舰）：**三选一**，不是 [-1,1] 的轴，也不是开关。
/// 写它 = 手动给这艘舰定活，自动控制的逐舰定编从此不碰它（`Player` 是那道闸门）；
/// 它只决定**派哪种活**，不解除武装——运输舰 / 观测舰照样自动开火、照样按 kiting 姿态软移动。
///
/// ⚠ 值的取值就是 serde 的 `ShipRole`：`'War' | 'Freight' | 'Observe'`（**字符串**，
/// 旧版这里是 `true`/`false`）。三态互斥：一艘舰同一时刻只有一种活。
/// 归属（三态 mode）不在这里选——它在行首那个归属下拉里（与其他几条轴一致，见 `modeToggleFor`）。
function roleEditor(node) {
  const leaf = node.leaf;
  const box = el('div', { class: 'ship-editor' });
  const sel = el('select', { class: 'role', 'data-axis': 'role' });
  [['War', '战舰'], ['Freight', '运输舰'], ['Observe', '观测舰']].forEach(([v, lbl]) => {
    const o = el('option', { value: v });
    o.textContent = lbl + '（' + roleHint(v) + '）';
    o.selected = (leaf.role || 'War') === v;
    sel.appendChild(o);
  });
  sel.title = '角色只决定自动控制派哪种活：战舰找仗打、运输舰按积压跑集货路线、'
    + '观测舰驻到太阳系外缘的引力异常区（MOND）蹲着喂「掌握度」那条知识渠道。'
    + '它不是 [-1,1] 的连续轴，也不解除武装——运输舰 / 观测舰在射程内照样自动开火、'
    + '照样按 kiting 姿态软移动。';
  sel.addEventListener('change', () => {
    leaf.role = sel.value;
    // 与另两条风格轴同一条规则：值变了 ⇒ 写值即接管（把这片叶钉成玩家，AI 定编从此不碰它）。
    wroteValue(leaf);
    controlRerender();
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
    controlRerender();
  };

  const typeSel = el('select');
  // `unset`（`behavior === null`）= **链上没有任何一层说话**，引擎按 `Idle` 兜底。它是个
  // **只读的显示状态**（disabled），不冒充「有人说了待命」：想真的下达待命就选「待命」，
  // 那才是一次写值（= 接管）。
  if (t === 'unset') {
    const o = el('option', { value: 'unset' });
    o.textContent = '（无人表态 · 按待命兜底）';
    o.selected = true;
    o.disabled = true;
    typeSel.appendChild(o);
  }
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

function leafValueEditor(leaf, label, node, opts) {
  const force = !!(opts && opts.force);
  const wrap = el('div', { class: 'leaf-val' });
  const t = el('span', { class: 'lv-label' });
  t.textContent = (label == null ? '' : label) + ' ';
  const inp = el('input', { type: 'number', class: 'num', value: leaf.value, step: '0.1' });
  // 只有「有效归属是玩家」的叶子才可编辑（可能继承自势力/天体/城市层的作用域）。
  // `force`（新控制行）跳过这道闸门：它的归属 chip 就在值旁边，而「还没有叶」的行更必须能
  // 写值——那是新建这片叶的唯一入口（写值即接管，与 `--apply` 同一条规则）。
  inp.disabled = !force && (!node || effectiveMode(node) !== 'Player');
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

/// **迁都**（`capital` 这片叶，值 = 一个天体名）。旧控制树从来没给它入口（它只在读面里显示），
/// 新控制行把它补上：一个天体下拉 + 「写值即接管」。
/// ⚠ 这片叶**没有 `remove`**（引擎的 manifest 里 `read_only` 与值字段都不含它）⇒ 这里不提供
/// 「不表态」那一档：发一个 `value: null` 进 presence-aware 的补丁等于**什么都没说**，
/// 给了那个选项才是骗人。要改就换一个天体，要撤就把归属改回「继承」。
function bodyEditor(leaf, node, opts) {
  const wrap = el('div', { class: 'leaf-val' });
  const sel = el('select', { class: 'body-pick', 'data-axis': 'body' });
  if (leaf.value == null) {
    const o = el('option', { value: '' });
    o.textContent = '（这一层还没有说首都是谁）';
    o.selected = true;
    sel.appendChild(o);
  }
  (st.bodies || []).forEach((bd) => {
    const o = el('option', { value: bd.name });
    o.textContent = bd.name + (bd.settlements && bd.settlements.length ? '' : '（无定居点）');
    o.selected = bd.name === leaf.value;
    sel.appendChild(o);
  });
  sel.title = '这一档说的首都是哪个天体（`capital` 是一片叶：写值即接管；建世界时播种，之后由迁都步骤维护）';
  sel.addEventListener('change', () => {
    if (!sel.value) return;
    leaf.value = sel.value;
    wroteValue(leaf);
    controlRerender();
  });
  wrap.appendChild(sel);
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

// --- 设计图库（势力级） ------------------------------------------------------
// 一张设计图 = 势力库里的一片叶（`blueprints`），**建造区只拿一个指针指向它**。
// 读面那一行给的是
// `{name, class, components, doctrine, kiting, role, mode, ship_count, launch_waiting}`：
//   * `class` / `components` / `doctrine` / `kiting` / `role` = 图的值
//     （写面 presence-aware：只报变过的字段）；
//   * `mode`  = **图叶自己的表态**（三态；有效归属还要往势力/全局作用域上溯）；
//   * `ship_count` / `launch_waiting` = **引擎现算的只读派生列**（本图造了多少艘 / 此刻是不是
//     「买不起 ⇒ 没下水」）。它们只用于显示，回传时不发（发了引擎也不看）。
//
// ⚠ **图能表态的是长期倾向（风格 / 风筝姿态 / 角色），不是指令**（2026-10 用户裁决）：
// 指令是即时操作（去那里 / 跟随那艘船），没有"出厂默认"可言；玩家的"这型舰干什么"
// 写在**角色**上（运输舰图 = 它一造出来就被派去跑集货路线）。原来那片 `order` 已删。
//
// 一条口径 A 的硬约束（`blueprint_class_mismatch`）：图的 `class` 必须与**挂它的每个建造区**
// 的 `ship_type` 相等。它是**正确的守卫**，所以这里不是"避免触发"而是**把它显示出来**
// （见 blueprintYardMismatch / 建造区那一行的告警 / 引擎报错回执三类呈现）。
//
// ⚠ 模板必须走**工厂**（每次给新对象、新数组）：模块级常量一旦被 `push` 过就再也洗不干净——
// 「新建表单里上次勾的组件还在」和「壳的原值被同一只数组改掉 ⇒ 补丁发不出去」都是它造成的。
function bpDraft(name) {
  return {
    name: name || '', class: '', components: [],
    doctrine: null, kiting: null, role: null, mode: 'Inherit',
  };
}

function compName(id) { return (cfg.components && cfg.components[id] && cfg.components[id].label) || id; }
function compSlots(cls) {
  const s = cfg.ships && cfg.ships[cls];
  return s ? (+s.slots || 0) : 0;
}
function blueprintOf(fc, name) {
  return name ? (fc.blueprints || []).find((b) => b.name === name) || null : null;
}
/// 本势力所有**指着这张图**的建造区（城名 / 下标 / 该区当前的舰级）。
function blueprintYards(fid, name) {
  const out = [];
  (st.cities || []).filter((c) => c.faction_id === fid).forEach((c) => {
    (c.buildings || []).forEach((b) => {
      if (b.blueprint === name) out.push({ city: c.name, id: b.id, ship_type: b.ship_type || '' });
    });
  });
  return out;
}
function yardCountText(fid, name) {
  const yards = blueprintYards(fid, name);
  if (!yards.length) return '没有任何建造区指着它（删了不影响生产）';
  const names = yards.map((y) => y.city + '/' + y.id).join('、');
  return yards.length + ' 个建造区会变成悬空指针 ⇒ 停产（' + names + '）——进度不再涨，直到你把指针拆掉或重建一张同名的图';
}
/// 这张图的舰级与挂了它的建造区**对不上**的那几个（引擎会报 `blueprint_class_mismatch`）。
function blueprintYardMismatch(fid, name, cls) {
  return blueprintYards(fid, name).filter((y) => y.ship_type !== cls);
}
/// 倾向摘要：图上这条轴**没有说话**（`null`）时明说，而不是显示成"默认值"。
/// 它就是引擎里 `role: null` / `doctrine: null` 的意思：这一层沉默，链往下降到舰队默认。
function stanceSummary(bp) {
  const bits = [];
  if (bp.role != null) bits.push('角色 ' + roleSummary(bp));
  if (bp.doctrine != null) bits.push('风格 ' + doctrineSummary(bp.doctrine));
  if (bp.kiting != null) bits.push('姿态 ' + kitingSummary(bp.kiting));
  return bits.length ? bits.join(' · ') : '倾向不表态（交给舰队默认）';
}
function blueprintSummary(fid, bp) {
  const comps = (bp.components && bp.components.length)
    ? bp.components.map(compName).join('＋')
    : '空装（交给生成器）';
  const yards = blueprintYards(fid, bp.name).length;
  return ' · ' + shipClassName(bp.class) + ' · ' + comps + ' · ' + stanceSummary(bp)
    + ' · ' + (bp.ship_count || 0) + ' 艘（本图造过）'
    + (yards ? ' · 挂在 ' + yards + ' 个建造区' : ' · 还没挂到任何建造区');
}
function blueprintNode(fc, bp) {
  return {
    key: 'bp' + fc.faction_id + ':' + bp.name,
    kind: 'blueprint', field: 'blueprints', id: bp.name, name: bp.name + blueprintSummary(fc.faction_id, bp),
    leaf: bp, fid: fc.faction_id, bp,
  };
}

/// **买不起 ⇒ 未下水**（用户裁决 Q4(b)）落在**这一行**上的判据。
///
/// 读面的 `launch_waiting` 是**整张图**的派生列（引擎算："挂着它的某个城里，这个舰级的进度
/// 已经攒够 `build_points` 却没下水"）。这里再用**本城**的进度把它定位到具体一个建造区——
/// 同一张图可能挂在几座城里，只有进度攒够的那一座才该显示这个标记。两个条件都要：
///   * 图的派生列（引擎算的、唯一真值）为真；
///   * 本城 `ship_progress[本区舰级] ≥ config.ships[舰级].build_points`。
function yardWaiting(fc, city, b) {
  if (!city || !b.blueprint) return false;
  const bp = blueprintOf(fc, b.blueprint);
  if (!bp || !bp.launch_waiting) return false;
  const spec = (cfg.ships || {})[b.ship_type];
  if (!spec) return false;
  const prog = ((city.ship_progress) || {})[b.ship_type] || 0;
  return prog >= (+spec.build_points || 0) - 1e-9;
}

/// 设计图那一行给出的**归属**（图叶自己的表态 + 有效归属的说明）。/// 与其它叶同一条链，只是设计图库没有「舰队默认」这一档：图叶 → 势力 scope → 全局。
///
/// ⚠ 措辞必须与**执行者**的实际行为对齐（`autocontrol/shipbuilding.rs::retool_shipyards`）：
/// `Auto` 图上系统改的是**舰级**（`class`），选装它不写——空选装 = 出厂那一刻由
/// `choose_loadout` 按当时库存现算。说"重算选装"就是给自己发一张空头承诺。
function blueprintOwnershipHint(node) {
  const m = normMode(node.leaf[modeField()]);
  if (m !== 'Inherit') return null;
  const fac = normMode(scopeVal(edScope.factions, node.fid));
  const g = normMode(edScope.global);
  const up = fac !== 'Inherit' ? ('势力作用域：' + fac) : ('全局作用域：' + g);
  return hintLine('这张图自己没有表态（Inherit）⇒ 往上看 ' + up
    + '；链上都没表态就是「自动」= 自动控制会把它的舰级重估成自己算的级（`retool_shipyards`，只在有建造区挂着它时发生；选装它不写）。'
    + '要钉死成你写的配方，把归属改成「玩家」。');
}

/// 设计图的编辑器：舰级下拉 + 组件多选 + **倾向三轴**（角色 / 风格 / 风筝姿态）。
/// ⚠ 图上**不能**写指令（2026-10 裁决：指令是即时操作，只写逐舰叶）。
///
/// 组件多选的两条纪律都在下拉里就守住（引擎那两条守卫仍然在，且报错会被显示出来）：
/// 一件组件一个槽位（勾选框天然不重复）、到槽位上限就把没勾的禁用。
function blueprintEditor(node) {
  const leaf = node.leaf;
  const fid = node.fid;
  const box = el('div', { class: 'ship-editor bp-editor' });

  // ① 舰级（口径 A：它必须与挂它的建造区的 `ship_type` 相等）。
  const classSel = el('select', { 'data-role': 'bp-class' });
  Object.keys(cfg.ships || {}).forEach((k) => {
    const o = el('option', { value: k });
    o.textContent = shipClassName(k) + '（' + k + '）';
    o.selected = k === leaf.class;
    classSel.appendChild(o);
  });
  if (!(cfg.ships || {})[leaf.class]) classSel.value = leaf.class || '';
  classSel.title = '这张图的舰级（`config.ships` 的 key）。口径 A：它必须与每个挂了这张图的建造区的「舰型」相等，否则引擎会拒（blueprint_class_mismatch）。';
  classSel.addEventListener('change', () => {
    leaf.class = classSel.value;
    wroteValue(leaf); // 写值即接管：改配方 = 表态（与引擎的「写值即接管」同一条规则）
    controlRerender();
  });
  box.appendChild(labelWrap('舰级', classSel));

  const bad = blueprintYardMismatch(fid, leaf.name, leaf.class);
  if (bad.length) {
    box.appendChild(hintLine('⚠ 有 ' + bad.length + ' 个建造区挂着这张图，但产的是别的舰级（'
      + bad.map((y) => y.city + '/' + y.id + '=' + shipClassName(y.ship_type)).join('、')
      + '）⇒ 引擎会拒这张补丁（`blueprint_class_mismatch`）。三条出路：把图的舰级改成同一级、同一份改动里把那个建造区的舰型也改过去、或者先拆掉指针。'));
  }

  // ② 组件多选（`components`：顺序 = 槽位顺序；`[]` = 交给生成器）。
  box.appendChild(componentPicker(leaf));

  // ③ **倾向**（角色 / 风格 / 风筝姿态）：图上能表态的就是这三条**长期**轴。
  //    ⚠ 指令**不在**这里（2026-10 裁决）：它是即时操作，要去「舰」分组里逐舰下命令，
  //    或者用**角色**影响这型舰将来干什么。
  box.appendChild(stanceEditor(leaf));

  if (node.bp && node.bp.launch_waiting) {
    box.appendChild(hintLine('⚠ 买不起 ⇒ 未下水：挂着这张图的那座城进度已经攒够，却因为这张图的选装买不起而没有放舰下水（进度不会丢，攒够钱就下水）。'));
  }
  return box;
}

/// 组件多选：勾一个 = 加一件（勾选顺序就是槽位顺序）。到槽位上限就把没勾的禁用，
/// 于是 `duplicate_component` / `too_many_components` 在界面上根本发不出去。
///
/// `opts.inPlace` = 勾选时**只就地更新这一格**（计数 / 禁用态 / 告警），不重画整棵树。
/// 新建表单必须这样：那片叶还没进编辑面，表单里别的东西（图名、草稿的选装）**只活在 DOM 里**，
/// 重画整棵树会把它们悄悄清空——这正是"我填了图名，点两下组件，图名没了"的成因。
function componentPicker(leaf, opts) {
  const inPlace = !!(opts && opts.inPlace);
  const slots = compSlots(leaf.class);
  const chosen = (leaf.components = leaf.components || []);
  const wrap = el('div', { class: 'bp-comps' });
  const head = el('div', { class: 'bp-comps-head' });
  const warn = el('div', { class: 'tnode-hint' });
  const boxes = [];
  const sync = () => {
    head.textContent = '组件 ' + chosen.length + '/' + slots + ' 槽';
    boxes.forEach((b) => {
      const on = chosen.indexOf(b.id) >= 0;
      const full = chosen.length >= slots && !on;
      b.cb.checked = on;
      b.cb.disabled = full;
      b.lbl.className = 'bp-comp' + (on ? ' on' : '') + (full ? ' full' : '');
      b.lbl.title = compName(b.id) + '（' + b.id + '）' + (full ? '：槽位已满（先取消一件）' : '');
    });
    const over = chosen.length - slots;
    warn.textContent = over > 0
      ? ('⚠ 这张图装了 ' + chosen.length + ' 件，而 ' + shipClassName(leaf.class) + ' 只有 ' + slots
        + ' 个槽位 ⇒ 引擎会拒（`too_many_components`）。取消 ' + over + ' 件，或换一个大一点的舰级。')
      : '';
    warn.hidden = over <= 0; // 空告警不要占一行（它只是一个位置）
  };
  wrap.appendChild(head);
  const list = el('div', { class: 'bp-comp-list' });
  Object.keys(cfg.components || {}).forEach((id) => {
    const lbl = el('label', { class: 'bp-comp', 'data-comp': id });
    const cb = el('input', { type: 'checkbox', 'data-role': 'bp-comp' });
    cb.addEventListener('change', () => {
      const i = chosen.indexOf(id);
      if (cb.checked && i < 0) chosen.push(id);
      else if (!cb.checked && i >= 0) chosen.splice(i, 1);
      if (inPlace) { sync(); return; } // 表单别的东西只在 DOM 里 ⇒ 绝不重画整棵树（见函数文档）
      wroteValue(leaf); // 写值即接管（改配方 = 表态，与引擎同一条规则）
      controlRerender();
    });
    boxes.push({ id, cb, lbl });
    lbl.appendChild(cb);
    lbl.appendChild(el('span', { class: 'bp-comp-name' }, compName(id)));
    list.appendChild(lbl);
  });
  wrap.appendChild(list);
  sync();
  wrap.appendChild(warn);
  return wrap;
}

/// **倾向编辑器**：图上能表态的三条轴——**角色 / 风格 / 风筝姿态**。
///
/// ⚠ 图**不再**能指定"指令"（2026-10 用户裁决）：指令是**即时操作**（去那里 / 跟随那艘船），
/// 不是"这型舰是什么"。玩家要"这型舰一造出来就跑运输"，写的是**角色**（运输舰）；
/// 要它更凶/更谨慎，写的是**风格**；要它贴着打/放风筝，写的是**风筝姿态**。
///
/// 每条轴都有「**不表态**」这一档，它就是引擎里的 `null`：**本图对这条轴没有说话** ⇒
/// 链继续往下降到舰队默认。它和「删掉这张图」（`remove: true`）是两回事——删图会让
/// 建造区悬空停产。三条轴**逐轴独立**：表态一条不影响另两条。
///
/// ⚠ 图能供值还有一个前提（引擎侧）：这张图的**归属解析为 `Player`**。图是 `Auto` 时，
/// 上面写的值只是"当前值"，链不会拿它当玩家的表态（与舰队默认叶同一条规则）。
function stanceEditor(leaf) {
  const box = el('div', { class: 'bp-stance' });
  const commit = () => { wroteValue(leaf); controlRerender(); };

  // ① 角色（三值枚举）：**"新舰出厂就干什么"最有用的一片**。
  const roleSel = el('select', { 'data-role': 'bp-role' });
  [['', '不表态（交给舰队默认）'], ['War', '战舰'], ['Freight', '运输舰'], ['Observe', '观测舰']]
    .forEach(([v, lbl]) => {
      const o = el('option', { value: v });
      o.textContent = v ? lbl + '（' + roleHint(v) + '）' : lbl;
      o.selected = (leaf.role || '') === v;
      roleSel.appendChild(o);
    });
  roleSel.title = '这型舰的**角色**：战舰找仗打、运输舰按积压跑集货路线、观测舰驻到太阳系外缘的引力异常区。'
    + '选了它，之后按这张图造出来的新舰一出厂就是这个角色（角色是活层：改这张图，角色叶沉默的老舰也一起跟）。'
    + '「不表态」= 引擎里的 `role: null`：这一层没有说话，链往下降到舰队默认角色。';
  roleSel.addEventListener('change', () => {
    leaf.role = roleSel.value || null; // 空串 = 明确写 null（这一轴回到沉默；缺席才是"不动这一格"）
    commit();
  });
  box.appendChild(labelWrap('角色', roleSel));

  // ② 风格（两轴一片叶：理智↔热血 + 护航↔独狼）。
  const docOn = el('input', { type: 'checkbox', 'data-role': 'bp-doctrine-on' });
  docOn.checked = leaf.doctrine != null;
  const docFields = el('span', { class: 'style-field' });
  const docVals = leaf.doctrine || { temper: 0, lone_wolf: 0 };
  const temperInp = el('input', { type: 'number', class: 'num', step: '0.1', min: '-1', max: '1', value: (+docVals.temper || 0).toFixed(2), 'data-axis': 'temper' });
  temperInp.title = '负 = 欺软怕硬（挑威慑比自己低的）；正 = 飞蛾扑火（挑威慑比自己高的）；0 = 基线';
  const wolfInp = el('input', { type: 'number', class: 'num', step: '0.1', min: '-1', max: '1', value: (+docVals.lone_wolf || 0).toFixed(2), 'data-axis': 'lone_wolf' });
  wolfInp.title = '负 = 空闲时贴本势力旗舰护航；正 = 独狼（空闲时自行就近接战）；0 = 基线';
  const pushDoctrine = () => {
    if (!docOn.checked) { leaf.doctrine = null; return; }
    leaf.doctrine = {
      temper: Math.max(-1, Math.min(1, +temperInp.value || 0)),
      lone_wolf: Math.max(-1, Math.min(1, +wolfInp.value || 0)),
    };
  };
  docOn.addEventListener('change', () => {
    pushDoctrine();
    docFields.hidden = !docOn.checked;
    commit();
  });
  temperInp.addEventListener('input', () => { pushDoctrine(); });
  wolfInp.addEventListener('input', () => { pushDoctrine(); });
  temperInp.addEventListener('change', commit);
  wolfInp.addEventListener('change', commit);
  docFields.hidden = !docOn.checked;
  docFields.append(el('span', { class: 'lv-label' }, '理智↔热血 '), temperInp,
    el('span', { class: 'lv-label' }, ' 护航↔独狼 '), wolfInp);
  const docWrap = el('span', { class: 'bp-stance-row' });
  docWrap.append(docOn, docFields);
  docWrap.title = '这片叶是**两轴一片**：勾上就是给两条轴都表态（引擎不接受只给一条——另一条会被静默当成 0.0 = 基线）。';
  box.appendChild(labelWrap('风格', docWrap));

  // ③ 风筝姿态（单值轴）。
  const kitOn = el('input', { type: 'checkbox', 'data-role': 'bp-kiting-on' });
  kitOn.checked = leaf.kiting != null;
  const kitInp = el('input', { type: 'number', class: 'num', step: '0.1', min: '-1', max: '1', value: (+leaf.kiting || 0).toFixed(2), 'data-axis': 'kiting' });
  kitInp.title = '负 = 风筝（保持最远武器射程、敌近则拉开、更早撤）；正 = 贴脸（压近敌舰、打得更久）；0 = 基线';
  const pushKiting = () => {
    leaf.kiting = kitOn.checked ? Math.max(-1, Math.min(1, +kitInp.value || 0)) : null;
  };
  kitOn.addEventListener('change', () => { pushKiting(); kitInp.hidden = !kitOn.checked; commit(); });
  kitInp.addEventListener('input', pushKiting);
  kitInp.addEventListener('change', commit);
  kitInp.hidden = !kitOn.checked;
  const kitWrap = el('span', { class: 'bp-stance-row' });
  kitWrap.append(kitOn, kitInp);
  box.appendChild(labelWrap('风筝姿态', kitWrap));

  box.appendChild(hintLine('图上写了某条轴 ⇒ 之后按这张图造出来的新舰出厂就带这条倾向'
    + '（倾向是活层：改这张图，**该轴叶沉默**的老舰也一起跟）。'
    + '⚠ 前提是这张图归**玩家**：图是「自动」时，上面的值只是当前值，引擎不把它当玩家的表态。'));
  return box;
}
function firstBodyWithSettlement() {
  const b = (st.bodies || []).find((x) => x.settlements && x.settlements.length);
  return b ? b.name : '';
}
function firstCityName() { return ((st.cities || [])[0] || {}).name || ''; }
function firstShipName() { return ((st.ships || [])[0] || {}).name || ''; }

/// 「＋ 新建设计图」：一行表单（图名 + 舰级 + 组件 + 角色 + 风格 + 风筝姿态 + 归属）→ 点「新建」把它加进
/// **编辑面**（还不是服务器上的图：点「应用到服务器」才真的落地）。
///
/// 它进 diff 的方式与其它叶一样是 presence-aware 的：新建的图在编辑面里是一片**壳**
/// （读面里没有它），壳一旦被碰过就**整片**发出去（名字 + 舰级 + 选装 + 倾向三轴 + 归属），
/// 于是「建图 + 挂指针」可以放同一份 diff 一次成功（引擎按这个顺序应用，spec §4.4 例 1）。
function addBlueprintButton(node) {
  const fid = node.fid;
  const wrap = el('div', { class: 'add-bld bp-add' });
  // 标题独占一行：`.tnode-label` 是 `flex: 1`，窄面板里会被挤成一列一个字。
  wrap.appendChild(el('span', { class: 'bp-add-title' }, '＋ 新建设计图'));

  const draft = bpDraft(''); // 草稿：点「新建」之前它不属于任何一片叶
  const nameInp = el('input', { type: 'text', class: 'bp-name', placeholder: '图名（本势力内唯一）', 'data-role': 'bp-new-name' });
  const classKeys = Object.keys(cfg.ships || {});
  const classSel = optSelect(cfg.ships, classKeys, classKeys[0], (v) => { draft.class = v; });
  classSel.dataset.role = 'bp-new-class'; // 与 data-key 同一条思路：人和脚本都能一步点到那一格
  classSel.title = '这张图的舰级（`config.ships` 的 key）。建之后必须与挂了它的建造区的「舰型」相等，否则引擎会拒（blueprint_class_mismatch）。';
  draft.class = classKeys[0] || '';

  const modeSel = el('select', { class: 'mode', 'data-role': 'bp-new-mode' });
  [['Player', '玩家'], ['Auto', '自动'], ['Inherit', '继承']].forEach(([v, l]) => {
    const o = el('option', { value: v });
    o.textContent = l;
    o.selected = v === 'Player';
    o.title = v === 'Player'
      ? '归你管：系统不再改写这张图的舰级与选装'
      : (v === 'Auto'
        ? '归系统：自动控制会把这张图的舰级改成它算出来的级（`retool_shipyards`）——只有建造区真的挂着它时才会发生；选装它不写（空选装 = 出厂时按库存现算）'
        : '图叶自己没有表态：往上看势力 / 全局作用域，链上都没表态就是「自动」（= 系统会重估舰级）。新图慎用：它和你刚写下的配方不是一回事。');
    modeSel.appendChild(o);
  });
  const modeHint = el('div', { class: 'tnode-hint bp-mode-hint' });
  const syncModeHint = () => {
    modeHint.textContent = modeSel.value === 'Player'
      ? '归属：玩家 ⇒ 这张图钉死成你写的配方，自动控制不碰它。'
      : (modeSel.value === 'Auto'
        ? '归属：自动 ⇒ 你写的配方只是当前值：自动控制会把这张图的舰级改成它算的级（选装它不写）。要钉死就用「玩家」。'
        : '归属：继承 ⇒ 图叶不说话，交给势力 / 全局作用域（默认是「自动」）。你写的值仍然在叶里，但归属不在你手上。');
  };
  syncModeHint();
  modeSel.addEventListener('change', syncModeHint);

  // 新建表单也要能写**倾向**（用户 2026-10 的诉求：图能指定的是风格/角色，不是指令）。
  // 三格：角色（枚举，含「不表态」）/ 风格（两轴一片）/ 风筝姿态。
  const roleSel = el('select', { 'data-role': 'bp-new-role' });
  [['', '不表态'], ['War', '战舰'], ['Freight', '运输舰'], ['Observe', '观测舰']].forEach(([v, l]) => {
    const o = el('option', { value: v });
    o.textContent = '角色：' + l;
    o.selected = v === '';
    roleSel.appendChild(o);
  });
  roleSel.title = '这型舰的**角色**：战舰找仗打、运输舰按积压跑集货路线、观测舰驻到引力异常区。'
    + '不表态 = 引擎里的 `role: null`：这一层没有说话，链往下降到舰队默认角色。';

  const docOn = el('input', { type: 'checkbox', 'data-role': 'bp-new-doctrine-on' });
  const docTemper = el('input', { type: 'number', class: 'num', step: '0.1', min: '-1', max: '1', value: '0.00', 'data-axis': 'temper' });
  const docWolf = el('input', { type: 'number', class: 'num', step: '0.1', min: '-1', max: '1', value: '0.00', 'data-axis': 'lone_wolf' });
  docTemper.title = '理智↔热血：负 = 欺软怕硬；正 = 飞蛾扑火；0 = 基线';
  docWolf.title = '护航↔独狼：负 = 空闲时贴旗舰护航；正 = 独狼；0 = 基线';
  const docWrap = el('span', { class: 'bp-stance-row' });
  docWrap.append(docOn, el('span', { class: 'lv-label' }, '理智↔热血 '), docTemper,
    el('span', { class: 'lv-label' }, ' 护航↔独狼 '), docWolf);
  docWrap.title = '勾上才表态（两轴必须一起给：只给一条时另一条会被引擎静默当成 0.0）。';

  const kitOn = el('input', { type: 'checkbox', 'data-role': 'bp-new-kiting-on' });
  const kitInp = el('input', { type: 'number', class: 'num', step: '0.1', min: '-1', max: '1', value: '0.00', 'data-axis': 'kiting' });
  kitInp.title = '风筝↔贴脸：负 = 风筝（保持最远射程、更早撤）；正 = 贴脸；0 = 基线';
  const kitWrap = el('span', { class: 'bp-stance-row' });
  kitWrap.append(kitOn, kitInp);

  wrap.appendChild(labelWrap('名字', nameInp));
  wrap.appendChild(labelWrap('舰级', classSel));
  wrap.appendChild(labelWrap('归属', modeSel));
  wrap.appendChild(labelWrap('角色', roleSel));
  wrap.appendChild(labelWrap('风格', docWrap));
  wrap.appendChild(labelWrap('风筝姿态', kitWrap));

  // 组件多选用一片**独立草稿**（草稿不在编辑面里 ⇒ 不能进 diff，所以它不共用 componentPicker
  // 的「写值即接管」路径；点「新建」时把那几件一次性写进新叶）。`inPlace` = 勾选不重画整棵树。
  const draftLeaf = Object.assign({}, draft);
  let compBox = componentPicker(draftLeaf, { inPlace: true });
  wrap.appendChild(compBox);
  // 舰级一改，槽位上限跟着变 ⇒ 只换这一格（草稿保留已勾的组件），别动表单别的东西。
  classSel.addEventListener('change', () => {
    draft.class = classSel.value;
    draftLeaf.class = classSel.value;
    const fresh = componentPicker(draftLeaf, { inPlace: true });
    wrap.replaceChild(fresh, compBox);
    compBox = fresh;
  });

  const add = el('button', { 'data-role': 'bp-new' }, '新建');
  add.title = '把这张图加进编辑面（还没发给服务器）：点「应用到服务器」才真的落地。';
  add.addEventListener('click', () => {
    const name = (nameInp.value || '').trim();
    if (!name) { $('#status').textContent = '图名不能为空（图名是库里的唯一 key）'; return; }
    const fc = getControl(fid);
    fc.blueprints = fc.blueprints || [];
    if (fc.blueprints.some((b) => b.name === name)) {
      $('#status').textContent = '库里已经有「' + name + '」这张图 ⇒ 改名（图名是唯一 key，改名 = 删旧建新）';
      return;
    }
    // ① 先按**壳的模板**登记原值（否则它看起来"没改过"、压根不会进 diff）；
    // ② 再把用户选的东西写上去——于是它与模板的差就是「新建这张图」这份补丁。
    const entry = shellLeaf(bpDraft(name), LEAF_SPEC.blueprints);
    fc.blueprints.push(entry);
    entry.class = classSel.value;
    entry.components = (draftLeaf.components || []).slice();
    // 倾向三轴：空 = **明确写 `null`**（这一层没有说话；缺席才是"不动这一格"）。
    entry.role = roleSel.value || null;
    entry.doctrine = docOn.checked ? {
      temper: Math.max(-1, Math.min(1, +docTemper.value || 0)),
      lone_wolf: Math.max(-1, Math.min(1, +docWolf.value || 0)),
    } : null;
    entry.kiting = kitOn.checked ? Math.max(-1, Math.min(1, +kitInp.value || 0)) : null;
    // 归属**显式**写出来（默认「玩家」）。不靠引擎的「写值即接管」兜底：那条规则会让界面
    // 显示「继承」而叶其实归了玩家——正是这个仓库反复反对的那种"界面骗人"。
    entry.mode = modeSel.value;
    controlRerender();
    $('#status').textContent = '已在编辑面里新建「' + name + '」：点「应用到服务器」才真的落地';
  });
  wrap.appendChild(add);
  wrap.appendChild(modeHint);
  return wrap;
}

/// 建造区那一行的**指针状态**：悬空 ⇒ 停产 / 舰级对不上 ⇒ 引擎会拒 / 买不起 ⇒ 没下水。
///
/// 单独一块（而不是直接往行上塞几行 hint）是因为它要**就地重算**：刚在「设计图」下拉里换了图
/// 时，下拉里的选择**领先于读面**（`b.blueprint` 还是载入时的值），而这三条判断都得按"现在
/// 选中的那张图"算——不然会出现"我选了图，行上还是旧状态"（引擎那边的守卫可不会跟着偷懒）。
/// `chosen` = 界面上当前选中的图名（`null` = 没挂图，`undefined` = 按读面 `b.blueprint`）。
function yardStatusBox(fc, node, chosen) {
  const box = el('div', { class: 'bp-yard-status' });
  const b = node.b;
  const name = chosen === undefined ? b.blueprint : chosen;
  const bp = blueprintOf(fc, name);
  if (name && !bp) {
    box.appendChild(hintLine('⚠ 这个建造区指着一张库里没有的图「' + name + '」⇒ 本区停产（进度不涨）。两条出路：把指针拆回「（无：自动选装）」，或者在设计图库里新建一张同名的图。'));
    return box;
  }
  if (!bp) return box;
  if (bp.class !== b.ship_type) {
    box.appendChild(hintLine('⚠ 这个建造区产的是 ' + shipClassName(b.ship_type) + '，而指针上的图「' + bp.name + '」是 ' + shipClassName(bp.class)
      + ' 级 ⇒ 引擎会拒这份补丁（`blueprint_class_mismatch`）：把「舰型」改成同一级，或在图上改（同一份改动里两处一起写也合法）。'));
  }
  // **买不起 ⇒ 未下水**（用户裁决 Q4(b) 的可见标记）。以前它只活在投影
  // （`idx/blueprints.jsonl.launch_waiting`），界面上看不见——于是「进度攒满了却不出舰」
  // 看起来像 bug。判据见 yardWaiting（引擎的派生列 + 本城的进度）。
  if (yardWaiting(fc, node.city, Object.assign({}, b, { blueprint: name }))) {
    const tag = el('span', { class: 'bp-waiting', 'data-role': 'bp-waiting' }, '买不起 ⇒ 未下水（进度在攒）');
    tag.title = '这个建造区本舰级的进度已经攒够 `build_points`，却没放舰下水：这张图（' + bp.name
      + '）的选装此刻买不起。进度不会丢，攒够钱就下水（用户裁决 Q4(b)，只对「玩家」归属的图生效）。';
    box.appendChild(labelWrap('状态', tag));
  }
  return box;
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
    controlRerender();
  })));

  const spec = cfg.buildings[b.kind];
  if (spec && spec.role === 'shipyard') {
    wrap.appendChild(labelWrap('舰型', optSelect(cfg.ships, Object.keys(cfg.ships || {}), b.ship_type, (v) => {
      b.ship_type = v;
      pushModify(fid, cityId, b.id, { ship_type: v });
      controlRerender();
    })));
    // **设计图**：这个建造区把「还不存在的舰」造成什么样。
    //
    // 读面（`world.control[势力].blueprints`）给的是图库；这一行只写**指针**
    // （`buildings[].blueprint`）——「（无：自动选装）」= 拆掉指针（写 `null`，不是
    // 缺席：缺席 = 不动这一格，两者后果不同）。图的内容（选装/倾向/归属）在图上改，
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
      o.textContent = bp.name + '（' + shipClassName(bp.class) + '·' + normMode(bp[modeField()]) + '）';
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
    // 指针的状态**写在行上**，不藏在展开的下拉里（悬空 ⇒ 停产 / 舰级对不上 ⇒ 会被拒 /
    // 买不起 ⇒ 没下水）。它必须能**就地重算**：刚在下拉里换了图时，下拉里的选择领先于读面
    // （`b.blueprint` 还是载入时的值），所以这一格按"当前选中的名字"算，并整块换掉。
    let status = yardStatusBox(fc, node, b.blueprint);
    bpSel.addEventListener('change', () => {
      // `''` ⇒ `null`（**拆掉指针**，回到自动选装），给名字 ⇒ 指过去。
      const chosen = bpSel.value || null;
      pushModify(fid, cityId, b.id, { blueprint: chosen });
      const fresh = yardStatusBox(fc, node, chosen);
      wrap.replaceChild(fresh, status);
      status = fresh;
      renderDiff();
    });
    wrap.appendChild(labelWrap('设计图', bpSel));
    wrap.appendChild(status);
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
    inline: inlineSpec,        // 原位组织点（本文件决定；widget 只认这个钩子）
    rerender: () => renderInfo(),
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
//
// ⚠ 回执**必须**显示出来（`/api/command` 现在把它随响应一起给：`view.report`）：
// `--apply` 的语义是"只触碰 diff 里出现的叶片"⇒ **静默丢掉与成功落地在响应上完全一样**。
// 界面能新建/改/删设计图之后，被守卫拒掉（`blueprint_class_mismatch` / `duplicate_component`
// / `too_many_components` / `no_such_component` / `no_such_blueprint`）是会正常发生的事，
// 而那正是玩家最需要看到的一句话（「失败看起来像成功」是这个仓库拉黑过的坑）。
let lastReport = null; // 最近一次 /api/command 的回执（ApplyReport），只用于显示

async function applyControl() {
  const req = buildCommandDiff();
  const n = countDiffLeaves(req);
  const view = await postJSON('/api/command', req);
  lastReport = view.report || null;
  bindWorld(view);
  buildEdits();
  renderAll();
  renderReport();
  const bad = lastReport ? lastReport.skipped.length : 0;
  $('#status').textContent = bad
    ? ('已应用 ' + (lastReport.applied) + ' 处，但有 ' + bad + ' 处被引擎拒了（见左侧红框）')
    : (n ? ('已应用 ' + n + ' 片叶的改动') : '没有改动要应用（只回传差异）');
}

/// 回执面板：把「引擎实际做了什么」写在按钮旁边。
/// 被拒的每一条都带着**稳定原因码 + 人读理由**（引擎给的原文，不翻译、不吞）。
function renderReport() {
  const box = $('#report');
  if (!box) return;
  box.textContent = '';
  const r = lastReport;
  if (!r) { box.className = 'api-report'; return; }
  const skipped = r.skipped || [];
  box.className = 'api-report' + (skipped.length ? ' bad' : ' ok');
  const head = el('div', { class: 'api-report-head' });
  head.textContent = skipped.length
    ? ('⚠ 引擎拒了 ' + skipped.length + ' 条（其余 ' + r.applied + ' 条已落地）')
    : ('✓ 全部落地：' + r.applied + ' 条');
  box.appendChild(head);
  skipped.forEach((s) => {
    const row = el('div', { class: 'api-skip', 'data-code': s.code });
    row.appendChild(el('span', { class: 'api-code' }, s.code));
    row.appendChild(el('span', { class: 'api-path' }, s.path));
    row.appendChild(el('div', { class: 'api-reason' }, s.reason));
    box.appendChild(row);
  });
  if ((r.took_over || []).length) {
    box.appendChild(hintLine('隐含接管 ' + r.took_over.length + ' 片叶（只写了值没写归属 ⇒ 引擎按「写值即接管」把它们钉成玩家）：' + r.took_over.join('、')));
  }
  if ((r.removed || []).length) {
    box.appendChild(hintLine('删掉了 ' + r.removed.length + ' 片叶：' + r.removed.join('、')));
  }
  if (!skipped.length && !(r.took_over || []).length && !(r.removed || []).length) {
    box.appendChild(hintLine('（没有需要你知道的边角：没有丢弃、没有隐含接管、没有删叶。）'));
  }
}

/// 推进/重建之后回执就过期了——留着它只会让人以为上一条错误还在。
function clearReport() { lastReport = null; renderReport(); }

async function advance(n) {
  prevState = world;
  bindWorld(await postJSON('/api/advance', { n }));
  buildEdits();
  clearReport();
  renderAll();
  updateTop();
  $('#status').textContent = '已推进 ' + n + ' 回合';
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
  clearReport();
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
