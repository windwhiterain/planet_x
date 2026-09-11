// 悬停弹窗：**名词 → 解释**。
//
// 用户裁决（2026-10）：*「所有 UI 都用名词，**鼠标移上去弹窗显示注释/解释**」*。
//
// 这份代码**不认识任何领域词**：它只做一件事——拿一个名词（就是读面上的中文键名）
// 去 `GET /api/schema` 那份数据里查解释，弹出来。数据来自两处、都由引擎发：
//
//   * `view`：`schemars` 收的 `///` 字段文档（`$defs.<结构体>.properties.<字段>.description`）；
//   * `projection`：投影每张表的 `column_docs`（列级解释，是**给读这两张表的人**写的）。
//
// 于是「注释就是产品文案」：写文档的人不必再去另一张表里抄一遍，前端也不必维护翻译。
// 查不到就**不弹**（而不是弹一个空框）——覆盖不到的名词由 `g4_spec.py` 静态判红。
window.Tip = (function () {
  /// 名词 → 解释（后写的覆盖先写的：`column_docs` 优先于字段文档，它更贴近读面）。
  const nouns = new Map();
  /// **字段名 → 解释**。给控制行留的兜底：那一片叶的键名现在还是英文
  /// （`投资预算`…，2026-10 起控制面的字段名本身就是中文名词），而界面显示的是中文标签
  /// ⇒ 用叶的字段名去查这一份。等控制面也改了名，这张表自然就退化成"和 nouns 同一批键"。
  const fields = new Map();
  let box = null;
  let showTimer = 0;
  let hideTimer = 0;
  let attached = 0;

  function ensureBox() {
    if (box) return box;
    box = document.createElement('div');
    box.className = 'tip';
    box.setAttribute('role', 'tooltip');
    // `pointer-events: none` 在 CSS 里：鼠标穿过它 ⇒ 不会自己把自己 mouseleave 掉。
    document.body.appendChild(box);
    return box;
  }

  /// 极简排版：先转义，再认 **粗体** 与 `代码`，换行保留。
  /// （文档注释是写给读者的中文散文，直接 textContent 会把 `**` 和反引号一起印出来。）
  function render(text) {
    const esc = text
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;');
    return esc
      .replace(/`([^`]+)`/g, '<code>$1</code>')
      .replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
      .replace(/\n/g, '<br>');
  }

  /// 收集一份 schema 里**所有**「字段名 → description」，写进 `into`。
  /// （`schemars` 的 `$defs.<结构体>.properties.<字段>.description`；字段名就是读面的键名，
  /// 因为结构体上写了 `#[serde(rename = "中文名")]`。）
  function eatAll(node, into, depth) {
    if (!node || typeof node !== 'object' || depth > 64) return;
    if (Array.isArray(node)) {
      node.forEach((x) => eatAll(x, into, depth + 1));
      return;
    }
    const props = node.properties;
    if (props && typeof props === 'object') {
      Object.keys(props).forEach((key) => {
        const d = props[key] && props[key].description;
        if (typeof d === 'string' && d.trim() && !into.has(key)) into.set(key, d);
      });
    }
    Object.keys(node).forEach((k) => eatAll(node[k], into, depth + 1));
  }

  /// 吃进 `GET /api/schema` 的响应。**只调一次**（启动时），此后纯查表。
  function useSchema(json) {
    if (!json) return { nouns: nouns.size };
    const proj = json.projection || {};
    // ① 投影的列解释（更贴近读面：它说的是"这一列在你读的那张表里是什么"）。
    Object.keys(proj).forEach((sec) => {
      const tables = proj[sec];
      if (!tables || typeof tables !== 'object') return;
      Object.keys(tables).forEach((t) => {
        const docs = (tables[t] || {}).column_docs;
        if (docs && typeof docs === 'object') {
          Object.keys(docs).forEach((k) => {
            const d = docs[k];
            if (typeof d === 'string' && d.trim()) nouns.set(k, d);
          });
        }
      });
    });
    // ② 实体字段（`state` 那半：舰名/忠诚度/所属天体…）与回合视图（`view` 那半：
    //    产出/维护/治理…）的字段文档——补投影没写到的那些名词。
    eatAll(json.state || {}, nouns, 0);
    eatAll(json.view || {}, nouns, 0);
    // ③ 投影的表列里**直接内联**的 description（`eager`/`derived` 那种写法：
    //    `columns: {"产出": {"type":"number","description":"…"}}`）。
    eatColumnDescriptions(proj, nouns);
    return { nouns: nouns.size };
  }

  /// 吃进 `GET /api/control-schema`（写面那份：叶的种类与它们的文档）。
  /// 只用来兜底查**叶的字段名**（界面显示的是中文标签，标签查不到时再按字段名查）。
  function useControlSchema(json) {
    if (!json) return { fields: fields.size };
    eatAll(json, fields);
    return { fields: fields.size };
  }

  /// 投影里 `columns` 的**内联 description**（`eager`/`derived` 的写法；`lazy` 表用的是
  /// 旁边的 `column_docs`，两处都吃）。
  function eatColumnDescriptions(proj, into) {
    Object.keys(proj).forEach((sec) => {
      const tables = proj[sec];
      if (!tables || typeof tables !== 'object') return;
      Object.keys(tables).forEach((t) => {
        const cols = (tables[t] || {}).columns;
        if (!cols || typeof cols !== 'object') return;
        Object.keys(cols).forEach((k) => {
          const d = cols[k] && cols[k].description;
          if (typeof d === 'string' && d.trim() && !into.has(k)) into.set(k, d);
        });
      });
    });
  }

  function place(ev) {
    if (!box) return;
    const pad = 14;
    const r = box.getBoundingClientRect();
    let x = ev.clientX + pad;
    let y = ev.clientY + pad;
    if (x + r.width > window.innerWidth - 8) x = Math.max(8, ev.clientX - r.width - pad);
    if (y + r.height > window.innerHeight - 8) y = Math.max(8, window.innerHeight - r.height - 8);
    box.style.left = x + 'px';
    box.style.top = y + 'px';
  }

  function show(node, text, ev) {
    ensureBox();
    box.innerHTML = render(text);
    box.classList.add('on');
    if (ev) place(ev);
    node.setAttribute('aria-describedby', 'tip');
    box.id = 'tip';
  }

  function hide() {
    clearTimeout(showTimer);
    clearTimeout(hideTimer);
    if (box) box.classList.remove('on');
  }

  /// 把弹窗挂到一个节点上。`noun` 为空 / 查不到解释 / 节点已有原生 `title` ⇒ 不挂。
  ///
  /// `field`（可选）= 这片叶/这个字段的**字段名**（控制行的键名可能还是英文）——名词查不到
  /// 时用它兜底。
  ///
  /// ⚠ 「已有 `title` 就不挂」是**有意的**：归属那类 `<select>` 自带 `title`（引擎发的
  /// 长解释），再叠一层自定义弹窗只会两个框一起冒出来。要改的是把 `title` 拿掉。
  function attach(node, noun, field) {
    if (!node) return false;
    const key = noun == null ? '' : String(noun).trim();
    const fld = field == null ? '' : String(field).trim();
    // 查词顺序：**显示的那个词** → **字段名**（语料里也有它：`state`/`view` 的键就是字段名，
    // `label` 覆盖了名词时才走到这一步） → 控制面那张字段表（叶的键名现在还是英文）。
    const doc = nouns.get(key)
      || (fld ? nouns.get(fld) : undefined)
      || (fld ? fields.get(fld) : undefined);
    if (!doc) return false;
    if (node.getAttribute && node.getAttribute('title')) return false;
    attached += 1;
    node.classList.add('tip-host');
    node.addEventListener('pointerenter', (ev) => {
      if (ev.pointerType && ev.pointerType !== 'mouse') return; // 触摸不弹（不炸即可）
      clearTimeout(hideTimer);
      clearTimeout(showTimer);
      showTimer = setTimeout(() => show(node, doc, ev), 250);
    });
    node.addEventListener('pointermove', (ev) => {
      if (box && box.classList.contains('on')) place(ev);
    });
    node.addEventListener('pointerleave', hide);
    node.addEventListener('pointerdown', hide);
    return true;
  }

  /// 离开页面/滚动/按 Esc 都要收掉（否则弹窗会挂在半空）。
  window.addEventListener('scroll', hide, true);
  window.addEventListener('blur', hide);
  window.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') hide();
  });

  return {
    useSchema,
    useControlSchema,
    attach,
    hide,
    /// 给实机自检/调试用：某个名词有没有解释，以及挂了多少个节点。
    describe: (noun) => nouns.get(String(noun).trim()),
    describeField: (field) => fields.get(String(field).trim()),
    stats: () => ({ nouns: nouns.size, fields: fields.size, attached }),
  };
})();
