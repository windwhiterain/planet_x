// 行星X WebUI 前端 — 经典脚本（控制面板/DOM）+ three.js 3D 地图（map3d.js，ES module）。
//
// 左栏**只有一栏**：`web/static/views.json` 声明出来的页（读与控制的四种行住在同一份声明、
// 同一个数组里 ⇒ 顺序就是穿插的顺序）。求值器在 `specview.js`（通用，不认识领域词），
// 写面那一半在 `controls.js`（叶的结构事实来自引擎的 `GET /api/control-schema`）。
//
// ⚠ 2026-10：**手写的控制层级树已整条删除**（`buildTree`/`shipNode`/`renderNode`/`KIND`/
// 「＋ 新建设计图」表单那一套）。它以前是第二栏「控制」，与新面并存了一段过渡期；
// 它提供的每一件事现在都由声明的行提供（设计图 → 「设计图」页的 leaf 行 + `new: true`、
// 建筑与建造区 → 城市卡片的 `action: buildings` 行、全局作用域 → 「全局」页的 `owner: global` 行、
// 迁都 → 势力卡片的 `capital` 行）。**编辑器函数本身留着**——它们是 widget 注册表
// （`leaf_ui.<field>.editor` 选一个），新行在用。见 `.agents/notes/web-control-spec.md`。
//
// 右侧「状态」 = **普通 state + 派生 + 配置**（原始读数；被组织点认领的集合会**就地**换成
// 整理后的表，那张表里也有控制行）。它把后端给的 `world.info`（每个根 = 模型的整份 JSON dump）
// 交给 jsonview.js 那个 schema-agnostic widget 渲染：本文件不写任何字段名，只决定「渲染哪个根」，
// 所以 State/GameConfig/RoundView 怎么改都不用动前端。

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
function kitingSummary(l) { return '风筝↔贴脸 ' + num2(l.姿态); }
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
  const r = l && l.角色;
  return ROLE_LABEL[r] || r || '战舰';
}

// **「自动」这一档到底有没有执行者**——措辞必须与引擎一致（note：control-live-layers §3.2/§13）。
// 三种情况，三句不同的话：
//   ① 逐舰**风格两叶**（`风格`/`姿态`）：本轮**有执行者**了
//      （`autocontrol::style` 每回合按战况概率重估、写回叶片）⇒ 照实说「会改写」。
//   ② 逐舰**角色叶**（`角色`）：自动控制按积压定编 ⇒ 也照实说「会改写」。
//   ③ **势力级默认叶**（`舰队默认风格`/`舰队默认姿态`/`舰队默认角色`）：AI **不写**
//      这片叶，而且引擎的取值规则是"默认叶只在**它自己是玩家**时供值" ⇒ 它 `Auto` 时的存储值
//      是**没人读的**。诚实的说法是「本层不供值」，绝不能写成"值由系统写"。
//      （一句话解释这个组合：`Auto` 的默认叶 = AI 的答案是**"不设全舰队默认、逐舰自己说"**，
//       所以它确实不必写任何值——但界面必须把"那个数没人用"说出来。）
/// 三组**字段名**：哪一片叶属于哪种措辞（归属下拉里「自动」那一档的说明文字读它）。
/// 用字段名而不是 kind：`kind` 只是控制树内部的行键，字段名才是与引擎对齐的那一个
/// （manifest / 读面 / 补丁都用它）。
const AUTO_RETUNED_FIELDS = ['风格', '姿态'];
const AUTO_WRITTEN_FIELDS = ['角色'];
const AUTO_UNWRITTEN_FIELDS = ['舰队默认风格', '舰队默认姿态', '舰队默认角色'];

/// 节点的字段名：控制行由 `controls.js` 直接给 `field`（引擎 manifest 里的那个拼写）。
function leafFieldOf(node) { return (node && (node.field || node.kind)) || ''; }

/// 一个角色取值**到底在干什么**（角色下拉的每个选项后面跟着它，避免两处各说一套）。
function roleHint(r) {
  switch (r) {
    case 'Freight': return '按积压去跑集货路线';
    case 'Observe': return '驻在引力异常区（MOND），喂「掌握度」那条知识渠道';
    default: return '找仗打（接战 / 轰炸 / 殖民）';
  }
}

// --- 读面（组织点）---------------------------------------------------------
// 左侧面板有两个模式：**控制**（写面，手写的控制树）与**读面**（组织点声明出来的视图）。
// 读面的一切由 `web/static/views.json`（数据）驱动，渲染交给 `specview.js`（通用求值器）——
// 本文件只做两件事：把根喂给它、把点击接到选中读面上。
// ⚠ 2026-10 第 9 步：以前还有第三件事——把「未组织」审计（页级兜底桶）算出来，那是**页级的
// 「其余」**（用户裁决 *「我不希望有"其余"这样的栏目」*），整条删除；墓碑见 `sidePages`。
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

// 悬停弹窗（**名词 → 解释**）的**唯一一处宿主决策**：读面（specview.js 的 `ctx.tip`）与
// 原始 JSON 视图（jsonview.js 的 `ctx.tip`）都走这里——各家只把「界面上显示的那个名字 /
// 那条列声明」递过来，**不许自己查表**（查表只发生在 tip.js 里）。这样「显示的名词是什么、
// 查不到时用哪个字段名兜底」只有一份实现，两边不会漂移。
//
//   * `col` 是列声明（读行 / 控制行）或是 `{path: 字段名}`（JSON 视图递来的裸键名）；
//   * `label` = **界面上印出来的那个词**（优先用它查语料）；
//   * `field` = **字段名兜底**（控制行用叶的字段名；读行用裸字段路径；
//     表达式列用自己声明的 `noun`）——因为 `label` 允许覆盖引擎的名词（`舰名`→`舰`），
//     覆盖之后按 label 就查不到解释了。表达式路径（`@post…`）不兜底：**别去表达式里猜**，
//     `@state.ships[?舰名=…].势力` 的裸段是 `ships`，猜出来必错。
function nounTip(node, col) {
  if (!window.Tip || !node || col == null) return;
  const c = typeof col === 'object' ? col : { path: String(col) };
  const label = c.label || c.path || c.leaf || c.owner || c.action;
  const bare = typeof c.path === 'string' && /^[A-Za-z_\u4e00-\u9fff][\w\u4e00-\u9fff]*$/.test(c.path)
    ? c.path
    : null;
  const field = c.noun || (c.leaf && window.Controls ? Controls.fieldOf(c.leaf) : bare);
  window.Tip.attach(node, label, field);
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
    // 悬停弹窗：求值器把「这条列声明」递过来，下面是那份唯一实现。
    tip: nounTip,
  });
}

async function loadViews() {
  try {
    // 1) 引擎的**结构事实**先拉（控制行要知道「每片叶靠哪几个字段定位、值写在哪」）；
    // 2) 声明装饰（补标签 / 算 field）必须在 `setSpecs` **之前**；
    // 3) `buildEdits` 还要用这份 manifest 建差异回传的地基（`rebuildLeafSpec`）。
    if (window.Controls) await Controls.load();
    // 4) **名词 → 解释**（`GET /api/schema`）：悬停弹窗的那段文字。同样拉一次就够，
    //    它只随引擎的声明变，不随回合变。失败不致命（弹不出解释而已，界面照常）。
    if (window.Tip) {
      try {
        window.Tip.useSchema(await fetchJSON('api/schema'));
      } catch (e) {
        console.warn('GET /api/schema 失败：名词弹不出解释', e);
      }
    }
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
// 声明、同一个数组**里，顺序就是穿插的顺序。
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
  if (!tabsBox || !body) return;
  // ⚠ 滚动的**是 `#side`**（`#readBody` 自己不滚，见 `style.css`）：重画前先记下位置、画完还回去
  // ——推进回合 / 应用改动之后，用户正看着的那一段不该跳回顶部（第 11 步的「位置保持」）。
  const side = $('#side');
  const keepScroll = side ? side.scrollTop : 0;
  tabsBox.textContent = '';
  body.textContent = '';
  if (!readLoaded) {
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
  // 第 11 步：视图是**分层**的（外层总览 → 点进内层详情，见 `specview.js::renderLayers`）。
  // 宿主只报三件事：这是哪一处实例（层状态按实例分开）、面包屑根写什么（页名只有宿主知道）、
  // 以及进/出内层时要保住滚动的那个容器。
  (page.views || []).forEach((spec) => window.SpecView.renderView(body, spec, {
    instance: 'side',
    crumb: page.title,
    scrollHost: side,
  }));
  if (side) side.scrollTop = keepScroll;
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
  selFaction = st.factions && st.factions.length ? st.factions[0][entityIdKey('faction')] : '';
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
  edControl.forEach((c) => { c.建筑 = c.建筑 || []; });
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

/// 归属三态写在哪个字段里——由引擎发；设计图的删除字段是蓝图专用动作，不走控制叶 manifest。
function modeField() { return (window.Controls && Controls.ownerField()) || '归属'; }
const BLUEPRINT_DELETE_FIELD = '删除';

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
    const base = baseControl.find((b) => b.势力 === fac.势力);
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
  if (removedBlueprints.has(leaf)) {
    // 设计图删除：只发身份键 + `删除`（引擎拒绝「删除 + 写值」混在一条补丁里）。
    if (o.shell) return null; // 读面里本来就没有这张图 ⇒ 没什么可删的
    const out = {};
    spec.keys.forEach((k) => { out[k] = leaf[k]; });
    out[BLUEPRINT_DELETE_FIELD] = true;
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
    const out = { 势力: fac.势力 };
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
    if (fac.建筑 && fac.建筑.length) { out.建筑 = fac.建筑; n++; }
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
  controlRerender();
  renderSelection();
}

function getControl(fid) {
  let c = edControl.find((x) => x.势力 === fid);
  if (!c) {
    // 桶按 manifest 建：**键叶 = 数组**（每片叶一个条目）、**单叶 = null**（还没有叶，
    // 由写面按需造壳——「没有叶」与「有叶但没表态」不是一回事，见 `controls.js`）。
    c = { 势力: fid, 建筑: [] };
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
  return (fc.建设权重 || []).find((e) => e.城 === city && e.建筑 === bid);
}
function buildLeaf(fc, city, bid) {
  return (fc.建造权重 || []).find((e) => e.城 === city && e.建筑 === bid);
}
function buildingLabel(b) {
  let s = kindName(b.类型);
  if (b.开采资源) s += '·' + resName(b.开采资源);
  if (b.建造舰级) s += '·' + shipClassName(b.建造舰级);
  s += '·' + structName(b.结构);
  s += '×' + (b.已建成面积 || 0).toFixed(1);
  return s;
}

// --- 主渲染 ----------------------------------------------------------------
function renderAll() {
  bindSpecView();
  renderMap();
  renderSide();      // 左栏：组织点声明出来的页（读与控制四种行同一条行序）
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
      const cap = ctrl[f.势力] && ctrl[f.势力].首都;
      return Object.assign({}, f, { id: f.势力, capital_body: cap ? cap.值 : undefined });
    }),
  });
}
function renderMap() {
  // 第三个参数是**整份 config 根**：3D 层要按 `cfg.ships` 里每一级舰的 hull/armor_mult/
  // speed_mult/… 把剪影算出来（舰的形状是数值的函数，不是在渲染层写死的常量表）。
  // `cfg` 为 null 时不传，渲染层退回默认值。
  if (window.PlanetXMap && window.PlanetXMap.setWorld) {
    window.PlanetXMap.setWorld(mapWorld(), cfg.body_kinds, cfg);
  }
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

/// 一个实体在 raw state 里的**身份键**（`城名` / `舰名` / `势力` / `天体名`…）。
///
/// ⚠ 不在这里手抄：**从读面的声明取**（`select` 视图的 `key`，与 `views.json` 同源）。
/// 引擎把字段名改成什么，这里跟着走——前端不认识"哪个结构体的人叫 name"。
function entityIdKey(kind) {
  const spec = ((viewsDoc && viewsDoc.select) || []).find((v) => v.select_kind === kind);
  return (spec && spec.key) || 'name';
}

function findEntity(kind, name) {
  const keys = Object.keys(st).filter((k) => Array.isArray(st[k]));
  const prefer = KIND_ARRAY[kind];
  const order = prefer && keys.includes(prefer) ? [prefer].concat(keys.filter((k) => k !== prefer)) : keys;
  for (const k of order) {
    const i = st[k].findIndex((e) => e && typeof e === 'object' && e[entityIdKey(kind)] === name);
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

// 选中读面：**先问组织点**（`select` 挂载）——命中就用整理过的卡片（没被声明的字段
// 由求值器**追加成普通行**，见 `specview.js` 的铁律 R：没有折叠桶）；
// 没命中就退回通用 widget 渲染整份记录（老行为，一条也不少）。
function renderSelection() {
  const head = $('#selHead');
  const box = $('#readout');
  if (!head || !box) return;
  head.textContent = '';
  box.textContent = '';
  if (!sel) {
    box.textContent = '（在地图上点天体/城/舰，或点左栏任何一页里的名字，这里会显示它的记录——控制行也能在这一格里直接改）';
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
    tip: nounTip,              // 通用树里的字段名同样是名词（宿主那一份唯一实现）
  });
}

// 通用树里的**原位重组点**：给 jsonview 的钩子。命中就返回节点，没命中返回 null（走通用渲染）。
// ⚠ 第 11 步：重组出来的视图现在是**分层**的（外层总览 → 点进详情）。这里给它一个**独立实例名**
// （`inline:<路径>`）⇒ 右栏点进某一项，不会把左栏那一页也一起切到详情层（层状态按实例分开）。
function inlineSpec(path, value) {
  if (!window.SpecView || !value || typeof value !== 'object') return null;
  const spec = window.SpecView.inlineFor(path);
  if (!spec) return null;
  const box = el('div', { class: 'sv-inline' });
  const bar = el('div', { class: 'sv-inline-bar' });
  bar.textContent = '此处由组织点「' + spec.id + '」整理（外层一行一个项目，点进去看全部字段：没被声明的字段追加在详情末尾）';
  box.appendChild(bar);
  window.SpecView.renderView(box, spec, { instance: 'inline:' + path, crumb: spec.title || spec.id });
  return box;
}

/// 左栏的页：**就是声明里的那些页**（`views.json` 的 `pages`，顺序即页签顺序）。
/// ⚠ 2026-10：这里以前还有一张「控制树（旧）」页（手写的控制层级）。它已经**整条删除**——
/// 旧树能做的每一件事都改由 `views.json` 声明的行提供：设计图库 → 「设计图」页的 `leaf` 行
/// （带 `new: true` 的「＋ 新建」）、建筑与建造区 → 城市卡片的 `action: buildings` 行、
/// 全局作用域 → 「全局」页的 `owner: global` 行、迁都 → 「势力」卡片的 `capital` 行、
/// 恢复继承 → 每条控制行自带；设计图删除是蓝图专用动作。见 `.agents/notes/web-control-spec.md`。
/// ⚠ 2026-10 第 9 步：这里还挂过一张**自动生成的「未组织」页**（`id: 'leftover'`）——它是
/// **页级的兜底桶**，与第 8 步删掉的那个「其余」列是同一个概念（用户裁决 *「我不希望有"其余"
/// 这样的栏目」*）。它承载的两件事都有了去处，所以**整条删除**：
///   ① 「哪些字段还没被整理」的审计 ⇒ 挪到数据级守卫：`play/tests/g4_spec.py` §8a 拿真世界 +
///      **原样跑 `specview.js`** 对账「声明列 ∪ 追加列 ∪ 不看列 == 全部引擎字段」（每个字段
///      要么进表、要么被声明不看，没有第三种去处 —— 页级的汇总表因此不再承载任何独有信息）；
///   ② 「看原始数据」的开关 ⇒ 删掉（理由：它**实测是 no-op**，而且右边的「状态」面板本来就是
///      全部根的原始 JSON，带过滤/全展开/全收起）。见 `.agents/notes/web-read-append.md` §7。
/// 同一批删掉的还有它的渲染代码（`renderLeftover` / `renderSpecCheck` / `renderWriteCheck` /
/// `shapeOf` / `isNilLike` / `checkRow` / `jsonToggle`）与只为它存在的 CSS 类
/// （`.sv-badge` / `.sv-lshape` / `.sv-lrow` / `.sv-lkey` / `.sv-lby` / `.sv-list` / `.sv-raw-box`）。
/// 判据：`g4_spec.py` §9（页级兜底桶不许再出现；且不允许顺手把第 8 步的追加接线一起删掉）。
function sidePages() {
  return viewsDoc.pages || [];
}

/// 控制面重画：控制行住在声明出来的那些页里，也可能同时出现在底部的选中卡上
/// （同一片叶两边都能改）⇒ 两边都要重画，否则会出现「改了左边、右下角还写着旧值」。
function controlRerender() {
  renderReadPanel();
  renderSelection();
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

/// **一片叶那一行**。（2026-10 之前它叫「层级树的一个节点」：容器由已删除的 `renderNode` 搭，
/// 叶片走这里。现在只有这一半留下——`controls.js` 把声明里的控制行做成同形的节点喂进来。）
/// 节点形状：`{key, field, name, leaf, fid, editor, ctl?}`；`opts` 是呈现上的开关
/// （`noLabel` / `alwaysEditable` / `carry`）。
///
/// ⚠ 2026-10（用户裁决：*「自动/玩家选项单独一行很占地方，弄到同一行」* +
/// *「UI 上的各种描述文字删掉」*）：一片叶现在**只有一行**——标签 / 值编辑器 / 归属下拉 /
/// 旁路按钮全住同一个 `.tnode-line`，它 `flex-wrap: wrap` ⇒ **装不下就换行、横向永不溢出**
/// （面板不会被撑宽，元素不会被挤出屏幕）。以前是「归属一行 head + 值一行 + 一句说明」，
/// 一片叶三行；逐句解释改挂在 `title=`（悬停才占地方，见 `modeToggleFor`）。
function renderLeafNode(node, opts) {
  const o = opts || {};
  const mf = modeField();
  const wrap = el('div', { class: 'tnode' + (node.ctl ? ' ctl-leaf' : ''), 'data-key': node.key });
  const line = el('div', { class: 'tnode-line' });

  if (!o.noLabel) {
    const lbl = el('span', { class: 'tnode-label' });
    let labelText = String(node.name == null ? '' : node.name);
    if (o.carry) labelText += '（' + o.carry + '）';
    lbl.textContent = labelText;
    if (node.color) lbl.style.color = node.color;
    line.appendChild(lbl);
  }
  wrap.appendChild(line);

  /// 归属下拉 + 旁路按钮：**跟在值后面**（同一行）。它必须在编辑器之后追加，所以是一个闭包。
  /// 「恢复继承」：撤销这片叶的**表态**（mode → 继承），值不动。只在它自己有表态时出现——
  /// 那时"我想反悔"才有意义（把下拉调回「继承」等价，但这个按钮把撤销写在脸上）。
  const ownBits = () => {
    const mt = modeToggleFor(node);
    if (mt) line.appendChild(mt);
    if (node.leaf && normMode(node.leaf[mf]) !== 'Inherit') line.appendChild(restoreInheritButton(node.leaf));
    // 设计图是库里可以删掉的对象（删图会让建造区悬空 ⇒ 停产）；这是蓝图专用动作，
    // 与控制叶无关。控制叶不再有「恢复出厂值」机制。
    if (node.leaf && leafFieldOf(node) === '设计图库' && rawLeafOf(node)) {
      line.appendChild(removeBlueprintButton(node));
    }
  };

  // 被标记删除的设计图：不再给编辑器（点「应用」它就没了），只说明会发生什么。
  if (node.leaf && removedBlueprints.has(node.leaf)) {
    ownBits();
    wrap.appendChild(hintLine('已标记删除：点「应用到服务器」之后这张图从库里消失，'
      + yardCountText(node.fid, node.id)
      + '（再点一次按钮可撤销）'));
    return wrap;
  }

  // 编辑器种类是**声明**（`views.json` 的 `leaf_ui.<field>.editor`）：`controls.js` 写在节点上。
  // 下面那条 if 链就是**封闭的编辑器注册表**——与读面的 `fmt` 一个道理。
  const editor = node.editor;
  // 编辑器对「**有效归属**是玩家」的叶子开放，而不是只看叶子自己的 mode：舰队默认设成玩家
  // 之后，继承它的舰也归你管——以前那些舰在 UI 上连编辑器都没有。
  // 改值时会把叶子显式钉成 Player（写值即接管，与 `--apply` 同一条规则）。
  // 新控制行（`alwaysEditable`）不看这道闸门：它的归属 chip 就在值旁边，而「还没有叶」的行
  // 必须能写值——否则「给一个还没有叶的资源设预算」在界面上无路可走。
  const open = !!o.alwaysEditable || effectiveMode(node) === 'Player';
  // 数值叶的标签由**行/列**给（那一行自己的名字），编辑器里不再重复一次。
  const numLabel = '';
  // 编辑器种类是**声明**（`views.json` 的 `leaf_ui.<field>.editor`），一张封闭的注册表
  // ——与读面的 `fmt` 一个道理。`behavior` 与旧控制树的 `ship` 是同一件事（`shipEditor`）。
  // ⚠ 编辑器与归属**都进那一行**（`line`）：只有 ⚠ 告警才另起一行（它们不是描述，是状态）。
  if (editor === 'ship' || editor === 'behavior') {
    if (open) line.appendChild(shipEditor(node.leaf, node));
  } else if (editor === 'doctrine') {
    if (open) line.appendChild(doctrineEditor(node));
  } else if (editor === 'kiting') {
    if (open) line.appendChild(kitingEditor(node));
  } else if (editor === 'role') {
    if (open) line.appendChild(roleEditor(node));
  } else if (editor === 'value' || editor === 'number') {
    line.appendChild(leafValueEditor(node.leaf, numLabel, node, { force: !!o.alwaysEditable }));
  } else if (editor === 'body') {
    // 迁都（`capital` 这片叶）：值是一个天体名。旧控制树从来没给它入口，新控制行把它补上。
    line.appendChild(bodyEditor(node.leaf, node, { force: !!o.alwaysEditable }));
  } else if (editor === 'building') {
    line.appendChild(buildingEditor(node));
  } else if (editor === 'blueprint') {
    // 设计图：**总是**给编辑器（图是玩家自己建的，没有"系统替你决定"这一档）。
    line.appendChild(blueprintEditor(node));
  } else if (editor) {
    line.appendChild(hintLine('（没有「' + editor + '」这个编辑器：views.json 的 leaf_ui 写了一种前端不认识的编辑器）'));
  }
  ownBits();
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
/// 而补丁接口只能新建/改写叶、删不掉。所以撤销表态之后，
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
// --- 设计图删除（蓝图专用动作，与控制叶无关） ------------------------------
// 控制叶不再支持「恢复出厂值 / 删叶」——出厂默认只是初始值，不存在一个可恢复的
// 保存目标。设计图不一样：它是用户建的对象，可以从库里删掉；删掉后挂它的建造区
// 变成悬空指针 ⇒ 停产（Q10(a)），但已下水的舰不受影响。
const removedBlueprints = new WeakSet(); // 被标记「删掉这张图」的设计图叶

/// 这张图在**原始 state** 里存在吗？——删图按钮只在真的有这张图时出现。
/// 读面里设计图行**总是**在；要知道真相得看 `state.control`。
function rawLeafOf(node) {
  if (!node.field) return null;
  const raw = (st.control || {})[node.fid];
  if (!raw) return null;
  const bucket = raw[node.field];
  if (bucket == null) return null;
  const spec = LEAF_SPEC[node.field];
  if (!spec || !spec.keys.length) return bucket;
  const leaf = node.leaf || {};
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

/// 删掉整张设计图（不是"改属性"）：只发身份键 + `删除`；挂它的建造区随后
/// 变成悬空指针 ⇒ 本区停产（Q10(a)）。点「应用到服务器」才生效。
function removeBlueprintButton(node) {
  const marked = removedBlueprints.has(node.leaf);
  const b = el('button', {
    class: 'restore rm-leaf' + (marked ? ' on' : ''), 'data-role': 'remove-blueprint',
    title: `删掉整张设计图（\`{"图名":…,"删除":true}\`）：挂它的建造区随后是悬空指针 ⇒ 本区停产（进度不再涨，Q10(a)），已下水的舰不受影响（快照）。点「应用到服务器」才生效。`,
  }, marked ? '取消删除' : '删掉这张图');
  b.addEventListener('click', () => {
    if (marked) removedBlueprints.delete(node.leaf);
    else removedBlueprints.add(node.leaf);
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
  // 归属三态写在**叶片自己**身上（引擎的 `owner_field`）。以前这里还兼管全局/势力/天体/城的
  // 作用域节点——那些节点随旧树一起删了；作用域现在是声明里的 `owner` 行（`controls.js`）。
  if (!node.leaf) return null;
  const mf0 = modeField();
  const mode = node.leaf[mf0];
  const set = (v) => { node.leaf[mf0] = v; };
  const isBp = leafFieldOf(node) === '设计图库';
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
/// 值一写就可能接管（`Inherit` → `Player`）：把这一行里的归属下拉与「恢复继承」**就地**
/// 跟上去。不重画整棵子树——那会换掉你正在编辑的那个输入框（与 styleField 的取舍一致），
/// 于是会出现「我刚敲了数，归属却还写着继承」这种骗人的画面。
function syncOwnership(node) {
  if (!node || !node.leaf) return;
  const wrap = document.querySelector('.tnode[data-key="' + node.key + '"]');
  const line = wrap && wrap.querySelector(':scope > .tnode-line');
  if (!line) return;
  const mode = normMode(node.leaf[modeField()]);
  const sel = line.querySelector('select.mode[data-role="mode"]');
  if (sel && sel.value !== mode) sel.value = mode;
  const btn = line.querySelector('button[data-role="restore"]');
  if (mode === 'Inherit') {
    if (btn) btn.remove();
  } else if (!btn) {
    line.insertBefore(restoreInheritButton(node.leaf), sel ? sel.nextSibling : null);
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
  inp.addEventListener('change', () => controlRerender());
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
  box.appendChild(styleField('姿态', '风筝↔贴脸', '负 = 风筝（保持最远武器射程、敌近则拉开、更早撤）；正 = 贴脸（压近敌舰、打得更久）；0 = 基线', leaf.姿态, (v) => setStyleAxis(leaf, '姿态', v), node));
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
    // 选项文字只留**名词**（下拉宽度由最宽的选项决定）⇒「它在干什么」那句话移进 `title=`
    // （悬停才占地方，用户裁决：*「tooltip 又不占空间」*）。
    o.textContent = lbl;
    o.title = lbl + '：' + roleHint(v);
    o.selected = (leaf.角色 || 'War') === v;
    sel.appendChild(o);
  });
  sel.title = '角色只决定自动控制派哪种活：战舰找仗打、运输舰按积压跑集货路线、'
    + '观测舰驻到太阳系外缘的引力异常区（MOND）蹲着喂「掌握度」那条知识渠道。'
    + '它不是 [-1,1] 的连续轴，也不解除武装——运输舰 / 观测舰在射程内照样自动开火、'
    + '照样按 kiting 姿态软移动。';
  sel.addEventListener('change', () => {
    leaf.角色 = sel.value;
    // 与另两条风格轴同一条规则：值变了 ⇒ 写值即接管（把这片叶钉成玩家，AI 定编从此不碰它）。
    wroteValue(leaf);
    controlRerender();
  });
  box.appendChild(sel);
  return box;
}

function shipEditor(leaf, node) {
  const edit = el('div', { class: 'ship-editor' });
  const t = behaviorType(leaf.行为);
  const d = behaviorToInput(leaf.行为);
  // 改行为 = 变成你自己的指令（写值即接管）：否则这次编辑会被系统下一回合按自己的逻辑
  // 覆盖掉，而界面上看起来「我明明改了」。
  const commit = (b) => {
    leaf.行为 = b;
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
    st.bodies.filter((x) => x.定居点 && x.定居点.length).forEach((bd) => {
      const o = el('option', { value: bd.天体名 }); o.textContent = bd.天体名; o.selected = d.body === bd.天体名; s.appendChild(o);
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
  const inp = el('input', { type: 'number', class: 'num', value: leaf.值, step: '0.1' });
  // 只有「有效归属是玩家」的叶子才可编辑（可能继承自势力/天体/城市层的作用域）。
  // `force`（新控制行）跳过这道闸门：它的归属 chip 就在值旁边，而「还没有叶」的行更必须能
  // 写值——那是新建这片叶的唯一入口（写值即接管，与 `--apply` 同一条规则）。
  inp.disabled = !force && (!node || effectiveMode(node) !== 'Player');
  inp.addEventListener('input', () => {
    if (inp.disabled) return;
    leaf.值 = +inp.value || 0;
    // 写值即接管；值改回**载入时那个数**不算表态（note §8 第 2 条，与风格轴同一条规则）。
    wroteValue(leaf);
    syncOwnership(node); // 接管了就立刻把那一行的归属下拉跟上（输入框不重画）
  });
  wrap.appendChild(t);
  wrap.appendChild(inp);
  return wrap;
}

/// 建造区那两片**权重叶**（`建设权重` / `建造权重`）的一行：值 + **它自己的归属**。
/// ⚠ 2026-10 修掉一个真 bug：以前这里把 `leafValueEditor` 当纯显示调（第三个参数 `node` 不传），
/// 而那个函数在 `!force && !node` 时把输入框 `disabled` ⇒ **界面上这两格根本改不动**
/// （`views.json` 的 `write_omit` 却写着"住在建筑行里"——说谎的是界面）。现在它可编辑，
/// 且与别的控制行同一条规矩：写值即接管（叶被钉成 `Player`）。
function weightRow(leaf, field, label, owner) {
  // 与 `renderLeafNode` 同一条规矩（用户裁决：归属不许自己占一行）：标签 → 值 → 归属，
  // 装不下由 `.leaf-val` 的 `flex-wrap` 换行，绝不横向溢出。
  const box = el('div', { class: 'leaf-val' });
  box.appendChild(el('span', { class: 'lv-label' }, label + ' '));
  const node = { key: ((owner && owner.key) || '') + ':' + field, field, fid: owner && owner.fid, leaf, name: label };
  box.appendChild(leafValueEditor(leaf, '', node, { force: true }));
  const mt = modeToggleFor(node);
  if (mt) box.appendChild(mt);
  return box;
}

/// **迁都**（`capital` 这片叶，值 = 一个天体名）。旧控制树从来没给它入口（它只在读面里显示），
/// 新控制行把它补上：一个天体下拉 + 「写值即接管」。
/// ⚠ 控制叶没有删叶/`remove` 机制 ⇒ 这里不提供
/// 「不表态」那一档：发一个 `value: null` 进 presence-aware 的补丁等于**什么都没说**，
/// 给了那个选项才是骗人。要改就换一个天体，要撤就把归属改回「继承」。
function bodyEditor(leaf, node, opts) {
  const wrap = el('div', { class: 'leaf-val' });
  const sel = el('select', { class: 'body-pick', 'data-axis': 'body' });
  if (leaf.值 == null) {
    const o = el('option', { value: '' });
    o.textContent = '（这一层还没有说首都是谁）';
    o.selected = true;
    sel.appendChild(o);
  }
  (st.bodies || []).forEach((bd) => {
    const o = el('option', { value: bd.天体名 });
    o.textContent = bd.天体名 + (bd.定居点 && bd.定居点.length ? '' : '（无定居点）');
    o.selected = bd.天体名 === leaf.值;
    sel.appendChild(o);
  });
  sel.title = '这一档说的首都是哪个天体（`capital` 是一片叶：写值即接管；建世界时播种，之后由迁都步骤维护）';
  sel.addEventListener('change', () => {
    if (!sel.value) return;
    leaf.值 = sel.value;
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
  const i = fc.建筑.findIndex((p) => p.建筑 === bid && !p.拆掉);
  if (i >= 0) fc.建筑[i] = Object.assign({}, fc.建筑[i], attrs, { 城: cityId, 建筑: bid });
  else fc.建筑.push(Object.assign({ 城: cityId, 建筑: bid }, attrs));
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
function compName(id) { return (cfg.components && cfg.components[id] && cfg.components[id].label) || id; }
function compSlots(cls) {
  const s = cfg.ships && cfg.ships[cls];
  return s ? (+s.slots || 0) : 0;
}
function blueprintOf(fc, name) {
  return name ? (fc.设计图库 || []).find((b) => b.图名 === name) || null : null;
}
/// 本势力所有**指着这张图**的建造区（城名 / 下标 / 该区当前的舰级）。
function blueprintYards(fid, name) {
  const out = [];
  (st.cities || []).filter((c) => c.势力 === fid).forEach((c) => {
    (c.建筑 || []).forEach((b) => {
      if (b.设计图 === name) out.push({ city: c.城名, id: b.建筑编号, ship_type: b.建造舰级 || '' });
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
/// **买不起 ⇒ 未下水**（用户裁决 Q4(b)）落在**这一行**上的判据。
///
/// 读面的 `launch_waiting` 是**整张图**的派生列（引擎算："挂着它的某个城里，这个舰级的进度
/// 已经攒够 `build_points` 却没下水"）。这里再用**本城**的进度把它定位到具体一个建造区——
/// 同一张图可能挂在几座城里，只有进度攒够的那一座才该显示这个标记。两个条件都要：
///   * 图的派生列（引擎算的、唯一真值）为真；
///   * 本城 `ship_progress[本区舰级] ≥ config.ships[舰级].build_points`。
function yardWaiting(fc, city, b) {
  if (!city || !b.设计图) return false;
  const bp = blueprintOf(fc, b.设计图);
  if (!bp || !bp.launch_waiting) return false;
  const spec = (cfg.ships || {})[b.建造舰级];
  if (!spec) return false;
  const prog = ((city.造舰进度) || {})[b.建造舰级] || 0;
  return prog >= (+spec.build_points || 0) - 1e-9;
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
    o.selected = k === leaf.舰级;
    classSel.appendChild(o);
  });
  if (!(cfg.ships || {})[leaf.舰级]) classSel.value = leaf.舰级 || '';
  classSel.title = '这张图的舰级（`config.ships` 的 key）。口径 A：它必须与每个挂了这张图的建造区的「舰型」相等，否则引擎会拒（blueprint_class_mismatch）。';
  classSel.addEventListener('change', () => {
    leaf.舰级 = classSel.value;
    wroteValue(leaf); // 写值即接管：改配方 = 表态（与引擎的「写值即接管」同一条规则）
    controlRerender();
  });
  box.appendChild(labelWrap('舰级', classSel));

  const bad = blueprintYardMismatch(fid, leaf.图名, leaf.舰级);
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

  if (leaf.launch_waiting) {
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
  const slots = compSlots(leaf.舰级);
  const chosen = (leaf.选装 = leaf.选装 || []);
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
      ? ('⚠ 这张图装了 ' + chosen.length + ' 件，而 ' + shipClassName(leaf.舰级) + ' 只有 ' + slots
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
/// 链继续往下降到舰队默认。它和「删掉这张图」（`删除: true`）是两回事——删图会让
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
      o.selected = (leaf.角色 || '') === v;
      roleSel.appendChild(o);
    });
  roleSel.title = '这型舰的**角色**：战舰找仗打、运输舰按积压跑集货路线、观测舰驻到太阳系外缘的引力异常区。'
    + '选了它，之后按这张图造出来的新舰一出厂就是这个角色（角色是活层：改这张图，角色叶沉默的老舰也一起跟）。'
    + '「不表态」= 引擎里的 `role: null`：这一层没有说话，链往下降到舰队默认角色。';
  roleSel.addEventListener('change', () => {
    leaf.角色 = roleSel.value || null; // 空串 = 明确写 null（这一轴回到沉默；缺席才是"不动这一格"）
    commit();
  });
  box.appendChild(labelWrap('角色', roleSel));

  // ② 风格（两轴一片叶：理智↔热血 + 护航↔独狼）。
  const docOn = el('input', { type: 'checkbox', 'data-role': 'bp-doctrine-on' });
  docOn.checked = leaf.风格 != null;
  const docFields = el('span', { class: 'style-field' });
  const docVals = leaf.风格 || { temper: 0, lone_wolf: 0 };
  const temperInp = el('input', { type: 'number', class: 'num', step: '0.1', min: '-1', max: '1', value: (+docVals.temper || 0).toFixed(2), 'data-axis': 'temper' });
  temperInp.title = '负 = 欺软怕硬（挑威慑比自己低的）；正 = 飞蛾扑火（挑威慑比自己高的）；0 = 基线';
  const wolfInp = el('input', { type: 'number', class: 'num', step: '0.1', min: '-1', max: '1', value: (+docVals.lone_wolf || 0).toFixed(2), 'data-axis': 'lone_wolf' });
  wolfInp.title = '负 = 空闲时贴本势力旗舰护航；正 = 独狼（空闲时自行就近接战）；0 = 基线';
  const pushDoctrine = () => {
    if (!docOn.checked) { leaf.风格 = null; return; }
    leaf.风格 = {
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
  kitOn.checked = leaf.姿态 != null;
  const kitInp = el('input', { type: 'number', class: 'num', step: '0.1', min: '-1', max: '1', value: (+leaf.姿态 || 0).toFixed(2), 'data-axis': 'kiting' });
  kitInp.title = '负 = 风筝（保持最远武器射程、敌近则拉开、更早撤）；正 = 贴脸（压近敌舰、打得更久）；0 = 基线';
  const pushKiting = () => {
    leaf.姿态 = kitOn.checked ? Math.max(-1, Math.min(1, +kitInp.value || 0)) : null;
  };
  kitOn.addEventListener('change', () => { pushKiting(); kitInp.hidden = !kitOn.checked; commit(); });
  kitInp.addEventListener('input', pushKiting);
  kitInp.addEventListener('change', commit);
  kitInp.hidden = !kitOn.checked;
  const kitWrap = el('span', { class: 'bp-stance-row' });
  kitWrap.append(kitOn, kitInp);
  box.appendChild(labelWrap('风筝姿态', kitWrap));

  return box;
}
function yardStatusBox(fc, node, chosen) {
  const box = el('div', { class: 'bp-yard-status' });
  const b = node.b;
  const name = chosen === undefined ? b.设计图 : chosen;
  const bp = blueprintOf(fc, name);
  if (name && !bp) {
    box.appendChild(hintLine('⚠ 这个建造区指着一张库里没有的图「' + name + '」⇒ 本区停产（进度不涨）。两条出路：把指针拆回「（无：自动选装）」，或者在设计图库里新建一张同名的图。'));
    return box;
  }
  if (!bp) return box;
  if (bp.舰级 !== b.建造舰级) {
    box.appendChild(hintLine('⚠ 这个建造区产的是 ' + shipClassName(b.建造舰级) + '，而指针上的图「' + bp.name + '」是 ' + shipClassName(bp.舰级)
      + ' 级 ⇒ 引擎会拒这份补丁（`blueprint_class_mismatch`）：把「舰型」改成同一级，或在图上改（同一份改动里两处一起写也合法）。'));
  }
  // **买不起 ⇒ 未下水**（用户裁决 Q4(b) 的可见标记）。以前它只活在投影
  // （`idx/blueprints.jsonl.launch_waiting`），界面上看不见——于是「进度攒满了却不出舰」
  // 看起来像 bug。判据见 yardWaiting（引擎的派生列 + 本城的进度）。
  if (yardWaiting(fc, node.city, Object.assign({}, b, { 设计图: name }))) {
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

  wrap.appendChild(labelWrap('结构', optSelect(cfg.structures, Object.keys(cfg.structures || {}), b.结构, (v) => {
    b.结构 = v;
    pushModify(fid, cityId, b.建筑编号, { 结构: v });
    controlRerender();
  })));

  const spec = cfg.buildings[b.类型];
  if (spec && spec.role === 'shipyard') {
    wrap.appendChild(labelWrap('舰型', optSelect(cfg.ships, Object.keys(cfg.ships || {}), b.建造舰级, (v) => {
      b.建造舰级 = v;
      pushModify(fid, cityId, b.建筑编号, { 建造舰级: v });
      controlRerender();
    })));
    // **设计图**：这个建造区把「还不存在的舰」造成什么样。
    //
    // 读面（`world.control[势力].blueprints`）给的是图库；这一行只写**指针**
    // （`buildings[].blueprint`）——「（无：自动选装）」= 拆掉指针（写 `null`，不是
    // 缺席：缺席 = 不动这一格，两者后果不同）。图的内容（选装/倾向/归属）在图上改，
    // 引擎会在 `--apply` 时报 `blueprint_class_mismatch`（图的舰级必须与舰型相等）。
    const bps = fc.设计图库 || [];
    const bpSel = el('select', { 'data-key': 'blueprint-' + cityId + '-' + b.id });
    const none = el('option', { value: '' });
    none.textContent = '（无：自动选装）';
    none.selected = !b.设计图;
    bpSel.appendChild(none);
    bps.forEach((bp) => {
      const o = el('option', { value: bp.图名 });
      // 归属是本势力的 scope 链解析出来的（这里只有叶自己的表态，够用：Player = 系统不许动）。
      o.textContent = bp.图名 + '（' + shipClassName(bp.舰级) + '·' + normMode(bp[modeField()]) + '）';
      o.selected = bp.图名 === b.设计图;
      bpSel.appendChild(o);
    });
    if (b.设计图 && !bps.some((bp) => bp.图名 === b.设计图)) {
      // **悬空指针**（图被改名/删掉了）：读面原样输出它，这里也必须显示出来——它意味着
      // **这个建造区停产**，静默吞掉就等于「失败看起来像成功」。
      const o = el('option', { value: b.设计图 });
      o.textContent = b.设计图 + '（库里没有这张图 ⇒ 本区停产）';
      o.selected = true;
      bpSel.appendChild(o);
    }
    bpSel.disabled = !bps.length && !b.设计图;
    // 指针的状态**写在行上**，不藏在展开的下拉里（悬空 ⇒ 停产 / 舰级对不上 ⇒ 会被拒 /
    // 买不起 ⇒ 没下水）。它必须能**就地重算**：刚在下拉里换了图时，下拉里的选择领先于读面
    // （`b.blueprint` 还是载入时的值），所以这一格按"当前选中的名字"算，并整块换掉。
    let status = yardStatusBox(fc, node, b.设计图);
    bpSel.addEventListener('change', () => {
      // `''` ⇒ `null`（**拆掉指针**，回到自动选装），给名字 ⇒ 指过去。
      const chosen = bpSel.value || null;
      pushModify(fid, cityId, b.建筑编号, { 设计图: chosen });
      const fresh = yardStatusBox(fc, node, chosen);
      wrap.replaceChild(fresh, status);
      status = fresh;
      renderDiff();
    });
    wrap.appendChild(labelWrap('设计图', bpSel));
    wrap.appendChild(status);
    if (node.buildLeaf) wrap.appendChild(weightRow(node.buildLeaf, '建造权重', '建造权重', node));
  }

  if (node.leaf) wrap.appendChild(weightRow(node.leaf, '建设权重', '建设权重', node));

  const rm = el('button', { class: 'rm' }, '移除');
  rm.addEventListener('click', () => {
    const fc = getControl(fid);
    fc.建筑.push({ 城: cityId, 建筑: b.建筑编号, 拆掉: true });
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
    const patch = { 城: node.cityId, 建筑: null, 类型: kind, 结构: structSel.value, 面积: +areaInp.value || 4 };
    if (kind === 'mining') patch.资源 = resSel.value;
    if (kind === 'construction') patch.建造舰级 = shipSel.value;
    fc.建筑.push(patch);
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
    tip: nounTip,              // 字段名/表头/行名是名词 ⇒ 悬停弹解释（同一套 Tip.attach）
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
      if (k === '势力') return;
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
    box.appendChild(hintLine('删掉了 ' + r.removed.length + ' 张设计图：' + r.removed.join('、')));
  }
  if (!skipped.length && !(r.took_over || []).length && !(r.removed || []).length) {
    box.appendChild(hintLine('（没有需要你知道的边角：没有丢弃、没有隐含接管、没有删除。）'));
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
  selFaction = st.factions && st.factions.length ? st.factions[0][entityIdKey('faction')] : '';
  sel = selFaction ? { kind: 'faction', name: selFaction } : null;
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
