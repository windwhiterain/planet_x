// 写面的一半：**声明式控制行**（`leaf` / `owner` / `action`）怎么渲染、怎么写回。
//
// 与读面（`specview.js`）的分工：
//   * 读面是**通用求值器**：路径 / 格式化器 / 布局 / 组织——一个领域词都没有；
//   * 写面**必然认识领域**（「一片叶靠哪几个字段定位、值写在哪几个字段、编辑器长什么样」），
//     所以它单独住一个文件。但**每片叶的事实不在本文件里手抄**：
//       - **结构事实**（身份键 / 值字段 / 只读派生列 / 归属字段名）来自引擎发的
//         `GET /api/control-schema`（`src/control/leaves.rs`）——一份事实，web 与 Python kit 共用；
//       - **呈现**（中文标签、编辑器种类、候选键从哪来、跟随哪片默认叶）来自 `views.json`
//         的 `leaf_ui` / `action_ui`；
//       - 本文件里剩下的只有**界面词汇**：哪种编辑器、一片空叶长什么样。
//
// 四种行住在 views.json 的**同一个 `columns` 数组**里（`path` 读 / `leaf` 叶 / `owner` 归属 /
// `action` 命令）⇒ 「读与控制穿插」不是额外机制，是那条数组顺序的直接结果。
//
// ⚠ 差异回传的五条语义**一条没改**（只回传变过的字段 / 写值即接管 / 原地改回则撤回界面自己钉的
// `Player` / 壳被碰过就整片发 / 设计图「删除」与值不可同条）——它们住在 `app.js`（`diffLeaf` 等），
// 本文件只负责**让那些函数找得到该找的东西**：每片叶都住在 `edControl[势力][字段]` 里。
'use strict';

(function () {
  // --- 引擎发来的结构事实（启动拉**一次**） ------------------------------------
  let MANIFEST = null;      // {leaves:{field:{keys,values,carries,read_only}}, actions:{...}, ownerField}
  let MANIFEST_ERR = null;  // 拉不到就**响亮**：控制行渲染成红字，底部「应用」也点不动

  async function load() {
    try {
      const r = await fetch('/api/control-schema');
      if (!r.ok) throw new Error('HTTP ' + r.status + ' ' + r.statusText);
      const doc = await r.json();
      MANIFEST = build(doc);
      MANIFEST_ERR = null;
      // 顺手把这份 schema 喂给悬停弹窗：控制行的**名词**来自 `leaf_ui`（中文标签），
      // 而它对应的**叶字段名**现在还是英文 ⇒ 标签查不到解释时按字段名兜底查
      // （`Tip.attach` 的第三个参数）。等控制面那批也改了名，这张兜底表自然退化。
      if (window.Tip) window.Tip.useControlSchema(doc);
    } catch (e) {
      MANIFEST = null;
      MANIFEST_ERR = String((e && e.message) || e);
      // 控制台也要留一条（界面上的红字是给人看的，这条是给查问题的人看的）。
      console.error('控制面结构事实（/api/control-schema）拉不到：', e);
    }
    return MANIFEST;
  }

  function build(doc) {
    const leaves = {};
    (doc.leaves || []).forEach((l) => { leaves[l.field] = l; });
    const actions = {};
    (doc.actions || []).forEach((a) => { actions[a.field] = a; });
    return {
      leaves,
      actions,
      ownerField: doc.owner_field || '归属',
      raw: doc,
    };
  }

  function manifest() { return MANIFEST; }
  function manifestError() { return MANIFEST_ERR; }
  /// 一片叶的**结构事实**（身份键 / 值字段 / 只读派生列）。app.js 的差异回传按它找叶。
  function leafSpec(field) {
    return (MANIFEST && MANIFEST.leaves[field]) || null;
  }
  function actionSpec(field) {
    return (MANIFEST && MANIFEST.actions[field]) || null;
  }
  function ownerField() { return (MANIFEST && MANIFEST.ownerField) || '归属'; }
  /// `shellLeaf` / `rememberOrigin` 要的那点东西（身份键 + 值字段）。
  function specOf(field) {
    const s = leafSpec(field);
    return s ? { keys: s.keys, values: s.values } : { keys: [], values: [] };
  }
  function uiFor(field) {
    const d = window.SpecView && window.SpecView.doc ? window.SpecView.doc() : null;
    return (d && d.leaf_ui && d.leaf_ui[field]) || {};
  }

  // 路径的最后一段**去掉方括号**就是叶的字段名：
  // `@control[?势力=${name}].投资预算` → `投资预算`
  // `@control[?势力=${势力}].指令[?舰=${name}]` → `指令`
  function fieldOf(path) {
    const segs = String(path || '').split('.');
    return String(segs[segs.length - 1] || '').replace(/\[.*$/, '');
  }

  /// 路径最后一段里的 `[?k=v]` ⇒ 这条控制行指的是**一片具体的叶**（不是整个列表）。
  /// `v` 里的 `${…}` 相对当前记录求值（与求值器的模板同一条规则）。
  /// 例：`指令[?舰=${name}]` ⇒ `{field: '舰', value: <这艘舰的名字>}`。
  function pickOf(path, rec, recKey) {
    const segs = String(path || '').split('.');
    const m = /\[\?([^=\]]+)=([^\]]*)\]/.exec(segs[segs.length - 1] || '');
    if (!m) return null;
    const value = m[2].replace(/\$\{([^}]*)\}/g, (all, e) => {
      const v = window.SpecView.evalPath(e, rec, recKey);
      return v == null ? all : String(v);
    });
    return { field: m[1], value: value };
  }

  const MODES = [['Inherit', '继承'], ['Auto', '自动'], ['Player', '玩家']];
  const OWNER_LABEL = { factions: '这个势力归谁', bodies: '这个天体归谁', cities: '这座城归谁' };
  const OWNER_HELP = {
    factions: '这一档管「这个势力下的键」：它的舰、城、设计图…',
    bodies: '这一档管这个天体',
    cities: '这一档管这座城（城里的建筑权重等）',
  };

  // --- 声明装饰：给每条控制行补上「界面那一半」 --------------------------------
  // 在 `SpecView.setSpecs` **之前**调用：补默认标签、算出 `field`、把 `leaf_ui` 挂到行上。
  function decorateDoc(doc) {
    const leafUi = (doc && doc.leaf_ui) || {};
    const actUi = (doc && doc.action_ui) || {};
    const walk = (spec) => {
      (spec.columns || []).forEach((col) => {
        if (col.leaf != null) {
          col.field = fieldOf(col.leaf);
          col.ui = leafUi[col.field] || {};
          if (!col.label) col.label = col.ui.label || col.field;
        } else if (col.owner != null) {
          if (!col.label) col.label = OWNER_LABEL[col.owner] || '归谁';
        } else if (col.action != null) {
          col.field = col.action;
          col.ui = actUi[col.action] || {};
          if (!col.label) col.label = col.ui.label || col.action;
        }
      });
    };
    ((doc && doc.pages) || []).forEach((p) => (p.views || []).forEach(walk));
    ((doc && doc.select) || []).forEach(walk);
    return doc;
  }

  // ⚠ 2026-10 第 9 步：这里以前有一个 `audit()`（写面自检：`--control-schema` 里每个叶要么被
  // 某条 `leaf` 行认领、要么在 `write_omit` 里写了理由）。它**唯一的调用者**是
  // `app.js::renderWriteCheck`，而那只挂在自动生成的「未组织」页上（页级兜底桶，已整条删除）
  // ⇒ 这里也删掉（**只服务它**：全仓 grep 只有那一处调用）。
  // **信息没丢，只是换了层**：同一件事现在由数据级守卫盯着 —— `g4_spec.py` §4「认领完整性」
  // （每个 `leaves[].field`/`actions[].field` 要么被行认领、要么在 `write_omit` 里有非空 why，
  // 且 `leaf`/`action`/`owner` 行与 `leaf_ui`/`action_ui` 不许有孤儿），非恒真由
  // `_g4_negative.py` ⑥⑦⑧⑨⑩ 盯着。运行时不再重复报一遍。

  // --- 宿主钩子：一行控制行 → 一个节点 ----------------------------------------
  function controlNode(col, rec, recKey, opts) {
    const where = (opts && opts.where) || 'sheet';
    if (MANIFEST_ERR) return errBox('控制面结构事实拉不到（/api/control-schema）：' + MANIFEST_ERR);
    if (!MANIFEST) return el('span', 'sv-missing', '（控制面结构事实还没拉回来）');
    try {
      if (col.owner != null) return ownerRow(col, rec, recKey, where);
      if (col.action != null) return actionRow(col, rec, recKey, where);
      if (col.leaf != null) return leafRow(col, rec, recKey, where);
    } catch (e) {
      return errBox('控制行渲染抛错：' + (e && e.message ? e.message : e));
    }
    return null;
  }

  function errBox(text) {
    const d = el('div', 'ctl-err');
    d.textContent = '⚠ ' + text;
    return d;
  }

  /// `el(tag, cls, text)`——`cls` 既可以是**类名字符串**，也可以是**属性对象**
  /// （`{class, 'data-role':…, type:…, value:…}`，与 `app.js` 那边的写法一致）。
  /// ⚠ 2026-10 修：以前只认字符串，而几处调用按 `app.js` 的习惯传了对象 ⇒ 元素上真的写着
  /// `class="[object Object]"`：**看着像设了类名，其实一个类都没有**（样式/脚本都找不到它）。
  /// 这种"哑巴失败"正是本仓拉黑的那一类，所以在这里一次兼容两种写法，而不是改所有调用点。
  function el(tag, cls, text) {
    const e = document.createElement(tag);
    if (typeof cls === 'string') {
      if (cls) e.className = cls;
    } else if (cls && typeof cls === 'object') {
      Object.keys(cls).forEach((k) => {
        const v = cls[k];
        if (k === 'class' || k === 'className') e.className = v;
        else if (k === 'text') e.textContent = v;
        else if (v != null) e.setAttribute(k, v);
      });
    }
    if (text != null) e.textContent = text;
    return e;
  }

  // --- 当前记录 → 它属于哪个势力 / 它叫什么 ------------------------------------
  // 表里的记录要么自己就是势力（`势力` = 势力名），要么带着 `势力`（城的势力字段）；
  // `@control[*]` 那种**读面**记录也带着 `势力`（2026-10 起读写面同名，键名就是它）。
  function fidOf(rec) { return rec ? rec.势力 : null; }
  // ⚠ 记录叫什么**不在这里猜**：`recKey` 由 specview 按视图声明的 `key` 求好传进来
  //（`recordKeyOf`，与 `views.json` 同源），旧代码那句 `rec.name` 已经不再成立。
  function fcOf(rec) { return getControl(fidOf(rec)); }

  // --- 一片叶的**可编辑副本** --------------------------------------------------
  // 关键：每片叶都必须住在 `edControl[势力][字段]` 里，因为
  //   ① `pairOrigins` 在那儿配对原值；② `buildCommandDiff` 在那儿找变过的叶。
  // 读面里**没有**这片叶 ⇒ 造一片**壳**（`shellLeaf`：原值 = 它自己）⇒ **没被动过就永不进 diff**
  // （`diffLeaf` 对壳只在"真的变了"时才发）。这正是「还没有叶也能设预算」能安全存在的原因。
  function editableLeaf(fc, field, keyVals, blank) {
    const spec = leafSpec(field);
    const single = !spec || !spec.keys.length;
    const mode = ownerField();
    if (single) {
      if (fc[field] == null || typeof fc[field] !== 'object') {
        const shot = Object.assign({}, blank);
        shot[mode] = 'Inherit';
        fc[field] = shellLeaf ? shellLeaf(shot, specOf(field)) : shot;
      }
      return fc[field];
    }
    const list = (fc[field] = fc[field] || []);
    let e = list.find((x) => spec.keys.every((k) => String(x[k]) === String(keyVals[k])));
    if (!e) {
      e = Object.assign({}, blank, keyVals);
      e[mode] = 'Inherit';
      list.push(e);
      if (shellLeaf) shellLeaf(e, specOf(field));
    }
    return e;
  }

  /// 一片**空叶**长什么样：只有值是界面词汇（数字 0 / 角色默认战舰 / 天体还没选…），
  /// 身份键与值字段都由引擎的 manifest 给。壳的「值」不会被回传（除非你真的改了它）。
  function blankOf(field, ui) {
    const ed = (ui && ui.editor) || 'number';
    if (ed === 'doctrine') return { temper: 0, lone_wolf: 0 };
    if (ed === 'kiting') return { 姿态: 0 };
    if (ed === 'role') return { 角色: 'War' };
    if (ed === 'body') return { 值: null };
    if (ed === 'behavior') return { 行为: null };
    if (ed === 'blueprint') return { 舰级: '', 选装: [], 风格: null, 姿态: null, 角色: null };
    return { 值: 0 };
  }

  // --- `leaf` 行 --------------------------------------------------------------
  function leafRow(col, rec, recKey, where) {
    const field = col.field || fieldOf(col.leaf);
    const spec = leafSpec(field);
    if (!spec) return errBox('引擎的 leaves 里没有「' + field + '」这片叶（views.json 写了个不存在的字段？）');
    const ui = col.ui || {};
    const fc = fcOf(rec);
    const raw = window.SpecView.evalPath(col.leaf, rec, recKey);
    if (col.compact && where === 'table') return compactCell(col, field, spec, fc, raw, rec, recKey);
    // 「一行一片叶」的三种情形：
    //   ① `keys` 为空 = 势力级**单叶**（`首都` / `舰队默认角色`…）；
    //   ② 路径带 `[?k=v]` = **已经挑出来**的那一片叶（逐舰四叶、逐城娱乐预算）；
    //   ③ 读面给回来的**就是一个对象**（路径本身就把叶挑出来了，与 ② 等价但更保险）。
    // 其余情形（路径指的是一整个列表）走下面的候选网格。
    const pick = pickOf(col.leaf, rec, recKey);
    if (!spec.keys.length || pick || (raw && !Array.isArray(raw))) {
      const entry = raw && !Array.isArray(raw) ? raw : null;
      const kv = {};
      if (pick) kv[pick.field] = pick.value;
      else spec.keys.forEach((k) => { if (entry && entry[k] != null) kv[k] = entry[k]; });
      const leaf = editableLeaf(fc, field, kv, blankOf(field, ui));
      const key = pick ? pick.value : (entry ? spec.keys.map((k) => entry[k]).join('/') : '');
      const rowInfo = { kv, entry, key, label: key ? rowLabel(col, key, entry) : (ui.label || field), showLabel: false };
      return leafNodeBox(col, field, leaf, rowInfo, rec, recKey, where);
    }
    // 多键叶：**读面里已有的叶** ∪ **候选键里还没有叶的那些** —— 一行一片叶。
    // 后者是主路径（r30 实测 `城市福利预算`/`设计图库` 是空数组、`舰队默认角色` 是 null）：
    // 今天的界面根本没法给一个还没有叶的资源设预算，这里补上。
    const box = el('div', 'ctl-grid');
    const rows = candidates(col, spec, raw, rec, recKey);
    if (!rows.length && !col.new) {
      box.appendChild(el('span', 'ctl-none', '（没有可配的键）'));
    } else {
      rows.forEach((r) => {
        const leaf = editableLeaf(fc, field, r.kv, blankOf(field, ui));
        r.showLabel = true;   // 网格里一行一片叶 ⇒ 这一行必须自己带标签（资源名 / 城名 / 楼）
        box.appendChild(leafNodeBox(col, field, leaf, r, rec, recKey, where));
      });
      // `new: true` ⇒ 身份键没有名单可枚举，给一个「现造一个」的入口（设计图库的图名）
      if (col.new) box.appendChild(newKeyForm(col, field, spec, fc, rec, recKey, ui));
    }
    return box;
  }

  /// 一行一片叶的**行**：复用 app.js 里那一段（旧控制树与新控制行共用同一套措辞与规则）。
  function leafNodeBox(col, field, leaf, rowInfo, rec, recKey, where) {
    const label = rowInfo ? rowInfo.label : ((col.ui && col.ui.label) || field);
    const node = {
      key: 'ctl:' + fidOf(rec) + ':' + field + ':' + (rowInfo ? rowInfo.key : ''),
      field,
      name: label,
      leaf,
      fid: fidOf(rec),
      editor: (col.ui && col.ui.editor) || 'number',
      ctl: true,
    };
    // `renderLeafNode` 在 app.js 里（它认识旧控制树那套结构）；这里只喂一个同形的节点。
    const box = renderLeafNode(node, {
      where,
      noLabel: !rowInfo || !rowInfo.showLabel,
      alwaysEditable: true,
      carry: rowInfo && rowInfo.entry ? carryText(rowInfo.entry, leafSpec(field)) : null,
    });
    const ro = readOnlyNote(rowInfo && rowInfo.entry, leafSpec(field), col.ui || {});
    if (ro) box.appendChild(ro);
    return box;
  }

  /// **只读派生列**（manifest 的 `read_only`：引擎现算、写面收下但**不写回**）。
  /// 列名与单位是**呈现**（`views.json` 的 `leaf_ui.<field>.read_only`）——本文件只说
  /// 「把它摆出来」，不说它叫什么。`false` 的开关不占地方（`launch_waiting: false` 不是新闻）。
  function readOnlyNote(entry, spec, ui) {
    if (!entry || !spec || !spec.read_only || !spec.read_only.length) return null;
    const labels = (ui && ui.read_only) || {};
    const bits = [];
    spec.read_only.forEach((k) => {
      const v = entry[k];
      if (v == null) return;
      if (typeof v === 'boolean' && !v) return;
      const n = typeof v === 'number' ? window.SpecView.num(v, 0) : (v === true ? '是' : String(v));
      bits.push((labels[k] ? labels[k] + ' ' : k + ' ') + n);
    });
    if (!bits.length) return null;
    return el('div', 'ctl-note', '引擎现算（写面不写回）：' + bits.join(' · '));
  }

  /// `leaf` 行上的 `new: true`：这片叶的**身份键没有名单可枚举**（设计图库的「图名」是开放
  /// 集合），由人现填 —— 它是 `keys_from` 的对偶：那个是「从名单里挑」，这个是「现造一个」。
  /// 身份键的**名字**来自引擎的 manifest（`keys`），这里只给一个输入框，不写死任何领域字段。
  /// 新建出来的是一片**壳**（原值 = 它自己）⇒ 没动过就不会进 diff（与「还没有叶」那条同一条规矩）。
  function newKeyForm(col, field, spec, fc, rec, recKey, ui) {
    const box = el('div', 'ctl-new');
    box.appendChild(el('div', 'ctl-new-title', '＋ 新建一片叶（' + (ui.label || field) + '）'));
    const inputs = spec.keys.map((k) => {
      const line = el('label', 'ctl-new-line');
      line.appendChild(el('span', 'lv-label', (ui.key_label || k) + ' '));
      const inp = el('input', { type: 'text', class: 'ctl-new-key', placeholder: k });
      inp.title = '这一片叶的身份键「' + k + '」：本势力内唯一';
      line.appendChild(inp);
      box.appendChild(line);
      return { k, inp };
    });
    const btn = el('button', {}, '新建');
    const stat = el('span', 'ctl-new-stat');
    btn.addEventListener('click', () => {
      const kv = {};
      let bad = null;
      inputs.forEach((x) => {
        const v = String(x.inp.value || '').trim();
        if (!v) bad = bad || x.k;
        kv[x.k] = v;
      });
      if (bad) { stat.textContent = '「' + bad + '」不能空'; return; }
      const list = (fc[field] = fc[field] || []);
      if (list.some((x) => spec.keys.every((k) => String(x[k]) === String(kv[k])))) {
        stat.textContent = '已经有一片同名的叶了（在上面那几行里）';
        return;
      }
      const e = Object.assign({}, blankOf(field, ui), kv);
      e[ownerField()] = 'Inherit';
      list.push(e);
      if (shellLeaf) shellLeaf(e, specOf(field));
      stat.textContent = '';
      if (controlRerender) controlRerender();
    });
    const bar = el('div', 'ctl-new-bar');
    bar.append(btn, stat);
    box.appendChild(bar);
    return box;
  }

  /// 候选键：读面里已有的叶 + `keys_from` 给的候选（`where` 相对**当前记录**求值）
  /// + **编辑面里已经有、读面里还没有的叶**。
  ///
  /// ⚠ 最后那一条是 2026-10 补的（`new: true` 那个「＋ 新建」把它们造出来）：新造的身份键
  /// 读面**不可能知道**（服务器上还没有这片叶），所以只按读面列候选人 ⇒ 你刚新建的那一行会在
  /// 下一次重画时**凭空消失**（数据还在 `edControl` 里，界面上没了）。编辑面是"读面 + 我改过的"，
  /// 候选键必须按这个并集来（与 `diffLeaf` 认壳、`pairOrigins` 配对同一个道理）。
  function candidates(col, spec, raw, rec, recKey) {
    const ui = col.ui || {};
    const arr = Array.isArray(raw) ? raw : [];
    const kf = spec.keys[0];
    const out = [];
    const seen = new Set();
    arr.forEach((e) => {
      const kv = {};
      spec.keys.forEach((k) => { kv[k] = e[k]; });
      const key = String(e[kf]);
      seen.add(key);
      out.push({ kv, entry: e, key, label: rowLabel(col, key, e) });
    });
    // 编辑面里多出来的那些（新建的叶 / 别处写过的叶）：读面里没有，但**必须**留在列表里。
    const fc = fcOf(rec);
    (fc && Array.isArray(fc[fieldOf(col.leaf)]) ? fc[fieldOf(col.leaf)] : []).forEach((e) => {
      const key = String(e[kf]);
      if (seen.has(key)) return;
      const kv = {};
      spec.keys.forEach((k) => { kv[k] = e[k]; });
      seen.add(key);
      out.push({ kv, entry: e, key, label: rowLabel(col, key, e), local: true });
    });
    const kfInfo = ui.keys_from;
    if (kfInfo) {
      // `keys_from.path` 与读面的路径语言同一套：**首段以 `@` 开头 = 绝对路径**。这里补一次
      // 前缀，是因为声明里写的是 `config.resources` 这种"从根名开始"的写法（读面列里的
      // `@根` 是显式的；这里两处都收）。
      const p = String(kfInfo.path || '').charAt(0) === '@' ? kfInfo.path : '@' + kfInfo.path;
      window.SpecView.expand(p).forEach((c) => {
        if (!whereOk(kfInfo.where, c.value, c.key, rec, recKey)) return;
        const v = kfInfo.key === '@key' ? c.key : window.SpecView.evalPath(kfInfo.key, c.value, c.key);
        if (v == null) return;
        const key = String(v);
        if (seen.has(key)) return;
        seen.add(key);
        out.push({ kv: buildKv(spec, kf, key, c), entry: null, key, label: rowLabel(col, key, null) });
      });
    }
    return out;
  }

  /// 候选里那些键 → 一片叶的身份键。第一个键来自 `keys_from`，其余键（如 `建设权重`
  /// 的 `城`/`建筑`）从当前记录里取。
  function buildKv(spec, kf, key, cand) {
    const kv = {};
    spec.keys.forEach((k) => {
      if (k === kf) { kv[k] = key; return; }
      const v = cand.value && cand.value[k] != null ? cand.value[k] : null;
      kv[k] = v;
    });
    return kv;
  }

  /// `where: "势力=${势力}"` —— 左边相对候选求值，`${…}` 相对当前记录求值。
  function whereOk(where, cand, candKey, rec, recKey) {
    if (!where) return true;
    const m = /^([^=]+)=(.*)$/.exec(String(where));
    if (!m) return true;
    const lhs = window.SpecView.evalPath(m[1].trim(), cand, candKey);
    const rhs = m[2].replace(/\$\{([^}]*)\}/g, (all, e) => {
      const v = window.SpecView.evalPath(e, rec, recKey);
      return v == null ? all : String(v);
    });
    return String(lhs) === String(rhs);
  }

  /// 一行那片的**标签**：`key_label_from` 指到 config 表里就取它的中文名（引擎里已有的那份，
  /// 不再抄第二份）；没有就原样用键。
  function rowLabel(col, key, entry) {
    const ui = col.ui || {};
    if (ui.key_label_from) {
      const m = window.SpecView.evalPath('@' + ui.key_label_from);
      const hit = m && m[key];
      if (hit) return hit.name || hit.label || key;
    }
    return key;
  }

  /// 身份是数字 id 的叶（`建设权重`：`城` + 数字 `建筑`）不能直接印数字——
  /// 用读面顺带带回来的 state 属性（`carries`）说清「这是座什么楼」。
  function carryText(entry, spec) {
    const c = (spec && spec.carries) || [];
    if (!c.length || !entry) return null;
    const bits = [];
    if (entry.类型 && kindName) bits.push(kindName(entry.类型));
    if (entry.资源 && resName) bits.push(resName(entry.资源));
    if (entry.建造舰级 && shipClassName) bits.push(shipClassName(entry.建造舰级));
    if (entry.结构 && structName) bits.push(structName(entry.结构));
    return bits.length ? bits.join('·') : null;
  }

  // --- `compact` 格：表格里只给「值 + 归属 chip」，完整编辑器在卡片里 ------------
  function compactCell(col, field, spec, fc, raw, rec, recKey) {
    const ui = col.ui || {};
    const box = el('div', 'ctl-compact');
    const pick = pickOf(col.leaf, rec, recKey);
    // 单叶 / 挑出来的那一片叶：一行一片，值 + 归属 chip。
    if (!spec.keys.length || pick || (raw && !Array.isArray(raw))) {
      const entry = raw && !Array.isArray(raw) ? raw : null;
      const kv = {};
      if (pick) kv[pick.field] = pick.value;
      else spec.keys.forEach((k) => { if (entry && entry[k] != null) kv[k] = entry[k]; });
      const leaf = editableLeaf(fc, field, kv, blankOf(field, ui));
      box.appendChild(el('span', 'ctl-v', valueText(leaf, field)));
      box.appendChild(modeChip([leaf], fc, field, false));
      return box;
    }
    const arr = Array.isArray(raw) ? raw : [];
    if (!arr.length) {
      box.appendChild(el('span', 'ctl-none', '还没有叶'));
      // 没有叶 ⇒ 没有可写的对象（compact 里不新建叶：那会一次给每个候选都造一片壳）。
      box.appendChild(modeChip([], fc, field, true));
      return box;
    }
    const bits = arr.slice(0, 3).map((e) => rowLabel(col, String(e[spec.keys[0]]), e) + ' ' + valueText(e, field));
    box.appendChild(el('span', 'ctl-v', bits.join(' · ') + (arr.length > 3 ? ' …（共 ' + arr.length + ' 项）' : '')));
    box.appendChild(modeChip(arr, fc, field, false));
    return box;
  }

  function valueText(leaf, field) {
    if (!leaf) return '—';
    const s = leafSpec(field);
    const k = (s && s.values && s.values[0]) || 'value';
    const v = leaf[k];
    if (v == null) return '—';
    if (typeof v === 'number') return window.SpecView.num(v, 2);
    if (typeof v === 'object') return JSON.stringify(v);
    return String(v);
  }

  /// 归属 chip：这片叶（这几片叶）现在归谁。多片叶表态不一致时**不假装它们一样**。
  function modeChip(leaves, fc, field, disabled) {
    const mode = ownerField();
    const modes = leaves.map((l) => normMode(l[mode]));
    const mixed = modes.some((m) => m !== modes[0]);
    const sel = el('select', { class: 'mode mini', 'data-role': 'mode-chip' });
    if (mixed) {
      const o = el('option', { value: '' });
      o.textContent = leaves.length + ' 片：各自不同';
      o.selected = true;
      sel.appendChild(o);
    }
    MODES.forEach(([v, l]) => {
      const o = el('option', { value: v });
      o.textContent = l;
      o.selected = !mixed && modes[0] === v;
      sel.appendChild(o);
    });
    sel.disabled = !!disabled;
    sel.title = disabled
      ? '这一档还没有叶：先在卡片里给某一项写一个值（写值即新建这片叶）'
      : '归属：继承 = 这一层不表态（向上层要答案）；自动 = 系统每回合可改写它；玩家 = 系统只读';
    if (!disabled) {
      sel.addEventListener('change', () => {
        if (!sel.value) return;
        leaves.forEach((l) => {
          l[mode] = sel.value;
          if (autoPinned) autoPinned.delete(l);
        });
        controlRerender();
      });
    }
    return sel;
  }

  // --- `owner` 行：作用域归属（不是叶） ----------------------------------------
  function ownerRow(col, rec, recKey, where) {
    // ⚠ 没有 `global` 那一档了（2026-10 用户裁决）⇒ 作用域行的三档都挂在一条记录上，`id` 恒为 recKey。
    const k = col.owner;
    const id = recKey;
    const box = el('div', 'ctl-owner');
    const sel = el('select', { class: 'mode owner', 'data-role': 'owner', 'data-scope': k });
    MODES.forEach(([v, l]) => {
      const o = el('option', { value: v });
      o.textContent = l;
      o.selected = normMode(scopeGet(k, id)) === v;
      sel.appendChild(o);
    });
    sel.title = '「' + (OWNER_LABEL[k] || k) + '」=' + (OWNER_HELP[k] || '') + '。它写的是**作用域**（这一层负不负责），不是某一片叶的值。';
    // 以前这里还挂一句「（cities：长三角 现在 = Inherit）」——它与**下拉本身**说的是同一件事
    // （用户裁决：描述文字删掉），所以只留下拉。
    sel.addEventListener('change', () => { scopeSet(k, id, sel.value); });
    box.append(sel);
    return box;
  }

  // ⚠ `edScope` 是 app.js 的 `let`（`buildEdits` 会整个替换它）⇒ 引用**活绑定**，不取快照。
  function scopeGet(k, id) {
    if (!edScope) return 'Inherit';
    return scopeVal(edScope[k] || [], id);
  }

  function scopeSet(k, id, v) {
    if (!edScope) edScope = { factions: [], bodies: [], cities: [] };
    edScope[k] = edScope[k] || [];
    setScopeVal(edScope[k], id, v);
  }

  // --- `action` 行：命令列表（不是叶） -----------------------------------------
  // 今天只有一条：`buildings`（新建 / 拆掉 / 改属性）。它与叶的区别是**存在性**：
  // 每一条只该执行一次，所以它不进「原值配对」那套，也不该被复制两遍。
  function actionRow(col, rec, recKey, where) {
    const field = col.action;
    if (!actionSpec(field)) return errBox('引擎的 actions 里没有「' + field + '」');
    if (field !== '建筑') return errBox('views.json 写了一条还没有渲染器的命令列表：「' + field + '」');
    const fid = fidOf(rec);
    const cityId = recKey;
    if (!fid || !cityId) return el('span', 'sv-missing', '（这条记录不是一座城：命令列表按城给）');
    const fc = getControl(fid);
    const box = el('div', 'ctl-action');
    const list = (rec.建筑 || []);
    if (!list.length) box.appendChild(el('span', 'ctl-none', '这座城还没有建筑'));
    list.forEach((b) => {
      const node = {
        key: 'bld' + fid + ':' + cityId + ':' + b.建筑编号,
        kind: 'building', id: b.建筑编号, name: buildingLabel(b),
        leaf: investLeaf(fc, cityId, b.建筑编号), buildLeaf: buildLeaf(fc, cityId, b.建筑编号),
        b, fid, cityId, city: rec,
      };
      box.appendChild(buildingEditor(node));
    });
    box.appendChild(addBuildingButton({ fid: fid, cityId: cityId }));
    return box;
  }

  window.Controls = {
    load,
    decorateDoc,
    controlNode,
    manifest,
    manifestError,
    leafSpec,
    actionSpec,
    specOf,
    uiFor,
    fieldOf,
    ownerField,
  };
})();
