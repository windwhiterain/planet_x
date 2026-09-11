"""快组（T0/T1）：`web/static/views.json` 的**声明纪律** —— 静态 + 写面对账 + 读面对账 + 认领完整性。

搬到这里的是 `web/src/views_tests.rs`（488 行 Rust 检查，已删除，`web/src/lib.rs` 原处留了一行
指针）。为什么搬：那份声明是**数据**，写错了不会编译报错——只会让某个视图静静地少一列、或者
让新加的控制叶在界面上**凭空消失**。而「对真实世界跑一遍」这件事在 Python 更划算：改断言不用
重编 8 个测试二进制（见 `.agents/notes/test-decoupled-suite.md`）。

判据（方案见 `.agents/notes/web-control-spec.md` §6）：

1. **静态纪律**（从 Rust 搬来）：id 唯一 / 引用完整（`use` / `use_at` / `map_ref` / `label_from`
   / `card` 指的都在）/ `omit` 不与列重叠 / 每条路径表达式合文法 / `@根` 都在已知根里；
2. **写面对账**（**双向**集合相等）：`--control-schema` 的 `leaves[].field` ∪ `actions[].field`
   ∪ `{势力}` **==** `schemars` 里 `FactionControlPatch.properties` 的键集合
   （加一个字段却不写声明 = 红；声明了一个不存在的叶 = 红）；
3. **读面对账**（**跑真世界**，不拿声明自证）：起一局（seed 42 / 40 回合），用 `--apply` 把
   **每一片叶都写一次**（14 片，每片带一个哨兵值），再 `--control` 读回来，逐片断言：
   * 条目字段集 **⊇** `keys ∪ values ∪ carries ∪ read_only ∪ {mode}`，且 **⊆** 那个集合 ∪ `{remove}`；
   * `keys` 空 ⇔ 该种类在 `FactionControlView` 里是**对象**；非空 ⇔ 是**数组**且每个条目带齐身份键；
   * **防空转**：这一局真的写进去了 14 片叶（每片按哨兵值验过，不是「表是空的所以通过」）。
4. **认领完整性**（铁律 R 的写面对偶）：每个 `leaves[].field` / `actions[].field` 要么被某条
   `leaf` / `action` 行认领，要么在 `write_omit` 里有一条**带非空 why** 的记录；
   反过来，`leaf` / `action` / `owner` 行与 `leaf_ui` / `action_ui` 的键**不许有孤儿**。
5. **`kind` 词表对账**（第 7 条，2026-10 新增）：`--index` 的 `control` 派生表里**出现过的
   `kind` 取值集合**，必须**逐字等于** `--control-schema` 声明的叶名
   （`leaves[].field ∪ actions[].field`）减去每条声明自报的 `not_in_index`（**带理由**的例外）。
   从前 `kind` 是发射器里手写的**英文**串（`ship_order`/`investment_budget`…），而同一片叶在
   patch / 读面上叫**中文**（`指令`/`投资预算`）——同一个概念两套名：Python 按英文 kind 筛、
   apply 用中文键，写错了哪一边都**不红**，只会静默查不到（"看起来有值"）。现在名字只在引擎的
   `control::leaves::LEAVES` 里声明一次（`kind_of` 是唯一翻译点），本条钉住它不再漂回去。
6. **文档对账**（§5b，2026-10 新增）：`--nouns` 里 state / view / control 三份 schema 的**每一条**
   `description`（根 / 定义级 / 字段级 / `oneOf` 变体级）都要在源码里找到那条 `///` 且**逐字相等**；
   反向再查一遍「源码带 `///` 的字段都真的发射了」。量的是「`///` → 弹窗文案」这条管线本身，
   不是某个症状——schemars 0.8.22 曾把单行 `/// **加粗**…` 剥成一个 `*`（46 条坏 markdown），
   就是这么被抓住的。见 `.agents/notes/doc-pipeline.md`。
7. **名词覆盖率**（§5 声明侧 + §5d 接线侧，2026-10 新增 §5d）：用户裁决「所有 UI 都用名词，
   **鼠标移上去弹窗显示注释/解释**」。§5 查**声明**（`views.json` 里当名词显示的列都得在
   `--nouns` 语料里查得到）；§5d 查**接线**（渲染字段名标签的 JS 模块都必须挂
   `Tip.attach`/`ctx.tip`）——通用 widget 自己渲染出来的字段名不在任何 `columns` 里，
   §5 够不着它们。两条合起来才是「界面上每个名词都弹得出解释」。

⚠ 实测（本轮 seed 42 / 40 回合）：读面条目**一个 `remove` 都没有**——`capital` 的读面是
`Control<天体名>`（`{值, 归属}`），而 `舰队默认*` 是 `{…, 归属, 删叶: false}` 且
`remove` 带 `skip_serializing_if = "is_false"` ⇒ 读面永远不发这个字段。所以上头的 `∪ {remove}`
是**允许集**（宽容那一侧），不是「应该有」；谁要在读面上真的看到它，本组的 detail 会报出来。
"""

from __future__ import annotations

import functools
import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _harness import REPO, group_main  # noqa: E402

VIEWS_JSON = REPO / "web" / "static" / "views.json"
# 前端**全部** JS 模块。§5d（名词覆盖率·静态）扫这里：`_g4_negative.py` 把它指到一份
# tempfile 拷贝上去注入错，**真文件一个字节都不碰**（同 `VIEWS_JSON` 的用法）。
STATIC_JS = REPO / "web" / "static"
# §5d 的两份「声明」——
#
# `FIELD_LABEL_CLASSES`：「字段名标签」的类名。这些元素里装的是**名词**（键名 / 表头 /
# 控制行标签），悬停就该弹解释。清单是**声明式**的：新写一个展示名词的视图，要么复用这些
# 类名（那它立刻被 §5d 咬住），要么把新类名加到这里——加这一行在 diff 里看得见。
FIELD_LABEL_CLASSES = ("jv-key", "jv-th", "sv-th", "sv-sheet-k")
# `TIP_MOUNTS`：「挂了 tip」的两种写法——宿主直接调 `Tip.attach`，或经求值器的钩子 `ctx.tip`。
# 用**词边界**匹配（不是子串）：把 `Tip.attach` 改名成 `Tip.attachX` 也算断线（写这条时的
# 实测：子串匹配会让「改名」骗过判据——`"Tip.attach" in "Tip.attachRenamed"` 为真）。
TIP_MOUNTS = (re.compile(r"\bTip\.attach\b"), re.compile(r"\bctx\.tip\b"))
# 那条链的**末端**（`Tip.attach`）单独留一份：`ctx.tip` 转发得再勤，末端没人接也是哑的。
TIP_HOST = re.compile(r"\bTip\.attach\b")
# §5d 只认**真的接线**、不认注释里提了一嘴：扫之前先把注释去掉（否则「把挂载删掉、注释里还写着
# `ctx.tip`」这种改动会骗过判据——写这条时的实测：`controls.js` 就只在注释里提过 `Tip.attach`）。
# ⚠ 这个剥离器的适用边界：它假设源码里没有把 `//` 写进字符串或正则（本目录实测 0 处）。
_JS_BLOCK_COMMENT = re.compile(r"/\*.*?\*/", re.S)
_JS_LINE_COMMENT = re.compile(r"//[^\n]*")


def _js_code(path: Path) -> str:
    """JS 源码**去掉注释**之后的样子（判据在这上面找类名与挂载点）。"""
    return _JS_LINE_COMMENT.sub("", _JS_BLOCK_COMMENT.sub("", path.read_text(encoding="utf-8")))

# 读面的根：与 `/api/state` 的 `info` 五个根 + 写面的两个读模板同批（`_frame` 侧同口径）。
ROOTS = ("state", "pre", "post", "config", "session", "control", "scope")
LAYOUTS = ("table", "sheet", "cards", "timeline", "pairs")
# 短局：40 回合就够长出舰/城/建筑/设计图，整组几秒跑完（不进长局、不读 170 MB 投影）。
SEED, ROUNDS = 42, 40


# --- 路径表达式（与 specview.js / 原 Rust 版同一套文法；这里只解析，不求值）---------
def split_segments(expr: str) -> list[str]:
    """按**不在方括号里**的 `.` 切段（`@control[?势力=x].a` → 2 段）。"""
    out: list[str] = []
    buf: list[str] = []
    depth = 0
    for ch in expr:
        if ch == "[":
            depth += 1
            buf.append(ch)
        elif ch == "]":
            depth -= 1
            buf.append(ch)
        elif ch == "." and depth == 0:
            out.append("".join(buf))
            buf = []
        else:
            buf.append(ch)
    out.append("".join(buf))
    return [s for s in out if s]


def parse_seg(raw: str) -> dict:
    """一段：`name` / `[*]`（spread）/ `[?f=v]`（pick）/ `[N]`（index）。不合文法就抛。"""
    i = raw.find("[")
    if i < 0:
        name, inside = raw, None
    else:
        if not raw.endswith("]"):
            raise ValueError(f"段 `{raw}` 的方括号没闭合")
        name, inside = raw[:i], raw[i + 1 : -1]
    if inside is None:
        return {"name": name, "spread": False, "index": None, "pick": None}
    if inside == "*":
        return {"name": name, "spread": True, "index": None, "pick": None}
    if inside.startswith("?"):
        if "=" not in inside[1:]:
            raise ValueError(f"段 `{raw}` 的 [?…] 少了 `=`")
        f, v = inside[1:].split("=", 1)
        return {"name": name, "spread": False, "index": None, "pick": (f, v)}
    if not inside.isdigit():
        raise ValueError(f"段 `{raw}` 的下标不是数字也不是 [*] / [?…]")
    return {"name": name, "spread": False, "index": int(inside), "pick": None}


def parse_path(expr: str) -> list[dict]:
    segs = split_segments(expr)
    if not segs:
        raise ValueError("空路径")
    return [parse_seg(s) for s in segs]


def is_absolute(expr: str) -> bool:
    return expr.startswith("@") and not expr.startswith("@key")


def root_of(expr: str) -> str | None:
    """绝对路径的根名（首段到 `[` 或 `.` 为止）；相对路径返回 None。"""
    if not is_absolute(expr):
        return None
    head = split_segments(expr)[0].split("[")[0]
    return head.lstrip("@")


def first_segment(expr: str) -> str | None:
    """相对路径的**首段**（列「认领」哪条记录的字段）；绝对路径返回 None。"""
    if is_absolute(expr):
        return None
    try:
        return parse_path(expr)[0]["name"]
    except ValueError:
        return None


# --- 遍历 ---------------------------------------------------------------------
def each_view(doc: dict):
    for page in doc.get("pages") or []:
        for v in page.get("views") or []:
            yield v
    for v in doc.get("select") or []:
        yield v
    for v in doc.get("inline") or []:
        yield v


def exprs_of(spec: dict) -> list[str]:
    """一条视图里出现的所有**路径表达式**（source / key / title_path / body / group / order /
    列（path / with / dot / **leaf**）/ meta）。"""
    out: list[str] = []

    def push(v):
        if isinstance(v, str):
            out.append(v)

    push(spec.get("source"))
    push(spec.get("key"))
    push(spec.get("title_path"))
    push(spec.get("body"))
    g = spec.get("group")
    if isinstance(g, dict):
        push(g.get("by"))
        push(g.get("dot"))
    for o in spec.get("order") or []:
        if isinstance(o, dict):
            push(o.get("by"))
    for c in spec.get("columns") or []:
        push(c.get("path"))
        push(c.get("with"))
        push(c.get("dot"))
        push(c.get("leaf"))  # ← v2 的写行：它也是一条路径表达式（住在 @control 上）
    for m in spec.get("meta") or []:
        push(m)
    return out


# --- 引擎 ---------------------------------------------------------------------
def _run(h, args: list[str]) -> tuple[int, str, str]:
    """跑一次二进制，**连 stderr 一起拿**（`--apply` 的回执只走 stderr）。"""
    p = subprocess.run(
        [str(h.path), *args],
        cwd=str(REPO),
        capture_output=True,
        text=True,
        encoding="utf-8",
    )
    return p.returncode, p.stdout, p.stderr


# --- 「源码 `///` → 发射的 description」对账用的源码扫描 -------------------------
#
# 为什么要**读源码**：`--nouns` 里的 `description` 是 schemars 从 `///` 派生的，中间隔着
# derive + JSON Schema 两层。这两层曾经**默默改字**——schemars 0.8.22 的
# `attr/doc.rs::get_doc` 里有一段向后兼容 hack：只要文档的**所有行**都以 `*` 开头，就当成
# `/** … */` 风格把每行首的 `*` 剥掉。于是**单行**注释若以 `**加粗**` 开头就被误判，
# 发射出来是 `*加粗**`（坏 markdown，实测 46 条）。只看 `--nouns` 自己**看不出对错**
# （它自洽），必须拿源码当尺子。
#
# 扫描口径：只认本仓的源码形状（`struct` / `enum` + 紧挨着的 `///`，`#[serde(rename = "…")]`
# 把 Rust 字段名换成读面的键名）。解析不出来的发射项**不许静默跳过**——调用方把它们记进
# `unmapped` 并判红（见 `run()` 的 §5b）。
_DOC_LINE = re.compile(r"^\s*///(.*)$")
_TYPE_DECL = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(struct|enum)\s+([A-Za-z_]\w*)")
_SERDE_RENAME = re.compile(r'#\[\s*serde\s*\(.*?rename\s*=\s*"([^"]+)"')
_FIELD_DECL = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?([A-Za-z_]\w*)\s*:")
_VARIANT_DECL = re.compile(r"^\s*([A-Za-z_]\w*)\s*([({,]|$)")


def _strip_strings(line: str) -> str:
    """挖掉字符串/字符字面量再数括号（`rename = "a{b"` 不许骗到深度）。"""
    out: list[str] = []
    i = 0
    while i < len(line):
        ch = line[i]
        if ch == '"':
            i += 1
            while i < len(line):
                if line[i] == "\\":
                    i += 2
                    continue
                if line[i] == '"':
                    i += 1
                    break
                i += 1
            continue
        if ch == "'" and i + 2 < len(line) and line[i + 2] == "'":
            i += 3          # 字符字面量 `'x'`；生命周期 `'a` 不会被误吃
            continue
        out.append(ch)
        i += 1
    return "".join(out)


@functools.lru_cache(maxsize=1)
def doc_containers() -> dict:
    """`src/**/*.rs` → `{类型名: {kind, doc, file, fields, variants}}`。

    * `doc`：类型自己的 `///` 块；
    * `fields`：`{读面键名: (注释, Rust 字段名)}`（有 `#[serde(rename)]` 就用它当键）；
    * `variants`：enum 的变体按**声明序**，每个带自己的 `doc` 与 `fields`（结构变体的字段）。
    """
    found: dict[str, dict] = {}
    for path in sorted((REPO / "src").rglob("*.rs")):
        rel = path.relative_to(REPO).as_posix()
        docs: list[str] = []
        attrs: list[str] = []
        cur: str | None = None
        var: dict | None = None
        depth = 0
        for lineno, raw in enumerate(path.read_text(encoding="utf-8").split("\n"), 1):
            m = _DOC_LINE.match(raw)
            if m:
                docs.append(m.group(1))
                continue
            code = _strip_strings(raw)
            if not code.strip():
                continue
            if code.lstrip().startswith("#["):
                attrs.append(raw)
                continue
            if cur is None:
                m = _TYPE_DECL.match(code)
                if m and "{" in code:
                    cur = m.group(2)
                    found[cur] = {"kind": m.group(1), "doc": "\n".join(docs).strip("\n"),
                                  "file": rel, "line": lineno, "fields": {}, "variants": []}
                    depth = code.count("{") - code.count("}")
                    var = None
                    if depth <= 0:
                        cur = None
                docs, attrs = [], []
                continue
            if depth == 1 and found[cur]["kind"] == "struct":
                m = _FIELD_DECL.match(code)
                if m and ":" in code:
                    key = m.group(1)
                    rn = _SERDE_RENAME.search(" ".join(attrs))
                    found[cur]["fields"][rn.group(1) if rn else key] = (
                        "\n".join(docs).strip("\n"), key)
            elif depth == 1:
                m = _VARIANT_DECL.match(code)
                if m:
                    found[cur]["variants"].append({
                        "name": m.group(1), "doc": "\n".join(docs).strip("\n"), "fields": {},
                        "file": rel, "line": lineno})
                    var = found[cur]["variants"][-1] if ("{" in code or "(" in code) else None
            elif depth == 2 and found[cur]["kind"] == "enum" and var is not None:
                m = _FIELD_DECL.match(code)
                if m and ":" in code:
                    key = m.group(1)
                    rn = _SERDE_RENAME.search(" ".join(attrs))
                    var["fields"][rn.group(1) if rn else key] = ("\n".join(docs).strip("\n"), key)
            depth += code.count("{") - code.count("}")
            docs, attrs = [], []
            if depth <= 0:
                cur, var = None, None
    return found


def _norm_text(text: str) -> str:
    """**空白归一**：换行/缩进是排版，不是文案（弹窗会把 `\\n\\n` 渲染成换行，但
    "同一段里的硬换行 vs 空格"不该让这条判据红）。归一之后只比**字**。"""
    return re.sub(r"\s+", " ", text).strip()


def _doc_to_description(doc: str) -> str:
    """源码 `///` 块 → 该发的 `description`。

    schemars 的规则：首行若是 `# 标题` 就当 `title`、**不进** `description`
    （0.8 与 1.x 都是这条规则）。本仓当前没有这种注释，但判据按规则写，
    免得将来有人加了一条标题行就以为判据坏了。
    """
    if doc.lstrip().startswith("#"):
        doc = doc.split("\n", 1)[1] if "\n" in doc else ""
    return _norm_text(doc)


# --- 一局要写的叶：每片一条 patch + 一条「读回来怎么认它」的哨兵 -------------------# 值都取互不相同的哨兵：写进去之后**从读面上按哨兵认领回来**，这样「这一局真的写进去了
# N 片叶」就不是靠回执自证，而是读面说了算。
D_TEMPER, D_LONE = 0.1, -0.2      # 舰队默认风格（两轴一起给）
D_KITING = 0.25                   # 舰队默认风筝姿态
D_ROLE = "Freight"                # 舰队默认角色
S_TEMPER, S_LONE = 0.3, 0.4       # 逐舰风格
S_KITING = -0.5                   # 逐舰姿态
S_ROLE = "War"                    # 逐舰角色
S_ORDER = "Idle"                  # 逐舰指令（无参变体 → JSON 里就是字符串）
V_INVEST = 1.5                    # 投资预算
V_CONSTRUCT = 2.5                 # 建造预算
W_INVEST = 1.25                   # 建设权重
W_BUILD = 1.75                    # 建造权重
V_LOYALTY = 3.5                   # 娱乐/福利预算
B_TEMPER, B_LONE = 0.5, -0.5      # 设计图的倾向两轴
B_KITING = -0.25
B_ROLE = "Observe"


def _keys_of(spec: dict) -> list[str]:
    return list(spec.get("keys") or [])


def _candidates(spec: dict, fac: dict) -> list[dict]:
    """这一局里能拿来写「片叶」的 key 组合。单值叶（`keys` 空）= 一条空 key（不用定位）。"""
    keys = _keys_of(spec)
    if not keys:
        return [{}]
    v = fac.get(spec["field"])
    if not isinstance(v, list):
        return []
    out = []
    for e in v:
        if isinstance(e, dict) and all(k in e for k in keys):
            out.append({k: e[k] for k in keys})
    return out


def _plan_leaves(leaves: list[dict], fac: dict) -> tuple[list[dict], list[str]]:
    """给「每一片叶都写一次」造 diff：返回 (plans, reasons)。

    每条 plan = `{field, keys, patch, sentinel}`：`patch` 写进 diff，`sentinel` 是读回来
    必须命中的 `{字段: 值}`（身份键之外的值字段），`keys` 是用来定位条目的身份键值。
    """
    plans: list[dict] = []
    missing: list[str] = []

    def first(spec):
        cands = _candidates(spec, fac)
        return cands[0] if cands else None

    def add(field, keys, patch, sentinel):
        plans.append({"field": field, "keys": keys, "patch": patch, "sentinel": sentinel})

    by_field = {s["field"]: s for s in leaves}
    for spec in leaves:
        f = spec["field"]
        keys = _keys_of(spec)

        if f == "首都":
            cap = fac.get("首都")
            body = (cap or {}).get("值") if isinstance(cap, dict) else None
            if not isinstance(body, str):
                missing.append("首都：这一局的读面里没有首都天体可写")
                continue
            add(f, {}, {"值": body, "归属": "Player"}, {"值": body, "归属": "Player"})

        elif f.startswith("舰队默认"):
            if f == "舰队默认风格":
                add(f, {}, {"temper": D_TEMPER, "lone_wolf": D_LONE, "归属": "Player"},
                    {"temper": D_TEMPER, "lone_wolf": D_LONE})
            elif f == "舰队默认姿态":
                add(f, {}, {"姿态": D_KITING, "归属": "Player"}, {"姿态": D_KITING})
            elif f == "舰队默认角色":
                add(f, {}, {"角色": D_ROLE, "归属": "Player"}, {"角色": D_ROLE})
            else:
                missing.append(f"{f}：本组不认识这片势力级单叶（新加的叶要在这里写上怎么写）")

        elif f in ("指令", "风格", "姿态", "角色"):
            k = first(spec) if keys else None
            if not k:
                missing.append(f"{f}：这一局该势力一条 {keys} 条目都没有，没法写")
                continue
            ship = k["舰"]
            if f == "指令":
                add(f, k, {"舰": ship, "行为": S_ORDER, "归属": "Player"},
                    {"行为": S_ORDER})
            elif f == "风格":
                add(f, k, {"舰": ship, "temper": S_TEMPER, "lone_wolf": S_LONE, "归属": "Player"},
                    {"temper": S_TEMPER, "lone_wolf": S_LONE})
            elif f == "姿态":
                add(f, k, {"舰": ship, "姿态": S_KITING, "归属": "Player"}, {"姿态": S_KITING})
            else:
                add(f, k, {"舰": ship, "角色": S_ROLE, "归属": "Player"}, {"角色": S_ROLE})

        elif f in ("投资预算", "建造预算", "福利预算"):
            k = first(spec)
            if not k and f == "福利预算":
                # `福利预算` 第一版 AI 不写回控制面 ⇒ 读面可能是空表；从同势力的
                # 投资/建造预算里借一个真实资源 key 来新建这片叶。
                for other in ("投资预算", "建造预算"):
                    src = first(by_field[other]) if other in by_field else None
                    if src and src.get("资源"):
                        k = {"资源": src["资源"]}
                        break
            if not k:
                missing.append(f"{f}：这一局该势力一个资源 key 都没有，没法写")
                continue
            if f == "投资预算":
                v = V_INVEST
            elif f == "建造预算":
                v = V_CONSTRUCT
            else:
                v = V_INVEST
            add(f, k, {**k, "值": v, "归属": "Player"}, {"值": v})

        elif f in ("建设权重", "建造权重"):
            k = first(spec)
            if not k:
                missing.append(f"{f}：这一局该势力一片建筑权重都没有，没法写")
                continue
            v = W_INVEST if f == "建设权重" else W_BUILD
            add(f, k, {**k, "值": v, "归属": "Player"}, {"值": v})

        elif f in ("城市福利预算", "开发货币预算", "建造货币预算"):
            # 这一局一开始一片都没有（省/市级 0 条）⇒ 必须**新建**一片：城从该势力自己的
            # 建筑权重里借一个（那些城一定属于它，写权重时引擎刚认过）。
            src = first(by_field["建设权重"]) if "建设权重" in by_field else None
            city = src.get("城") if src else None
            if not city:
                cands = [e.get("城") for e in (fac.get(f) or [])
                         if isinstance(e, dict) and e.get("城")]
                city = cands[0] if cands else None
            if not city:
                missing.append(f"{f}：这一局拿不到该势力的城名，没法写")
                continue
            add(f, {"城": city}, {"城": city, "值": V_LOYALTY, "归属": "Player"},
                {"值": V_LOYALTY})

        elif f == "设计图库":
            k = first(spec)
            if not k:
                missing.append(f"{f}：这一局该势力一张设计图都没有，没法写")
                continue
            # 只写**倾向三轴**（+ mode）：`class` / `components` 要过 `blueprint_class_mismatch`
            # 那道守卫（图与建造区的舰级必须一致），而我们只需要把这片叶**写一次**——
            # 那两个值字段在不在读面条目上由第 3 条判据（读面对账）负责，不靠这条写。
            add(f, k, {"图名": k["图名"],
                       "风格": {"temper": B_TEMPER, "lone_wolf": B_LONE},
                       "姿态": B_KITING, "角色": B_ROLE, "归属": "Player"},
                {"风格": {"temper": B_TEMPER, "lone_wolf": B_LONE},
                 "姿态": B_KITING, "角色": B_ROLE})

        else:
            missing.append(f"{f}：本组不认识这片叶（新加的叶要么在这里写上怎么写，要么进 write_omit）")

    return plans, missing


#: **测试注入点**：`_control_kinds_from_index` 在真文件上读到 `idx/control.jsonl` 之前会调一次它
#: （参数是那个文件路径）。生产路径上**没有人设置它**（默认 `None`），只有
#: `play/tests/_g4_negative.py` 用它把某片叶的 `kind` 改成一个**声明里没有的词**，
#: 验证第 7 条对账真的会红——一条不会红的守卫只是看起来在守纪律。
INDEX_HOOK = None


def _control_kinds_from_index(h, ckpt: Path, diff_path: Path, tmp: Path):
    """跑一局**每一片叶都写过**的世界，从 `--index` 的 `control` 表里取 `kind` 集合与行数。

    为什么不在第 6 条那趟 `--index`（seed 42 / 40 回合）上顺手取：那张表的行数是**元素**
    稀疏的——没人设过的叶（比如初期一条都没有的 `城市福利预算`）根本不会有行，
    于是「声明 16 片、表里 9 种」看起来像红，其实是**世界没写过**。
    所以这里复用第 3 节那份「把每一片叶都写一次」的 checkpoint + diff（`_plan_leaves`
    覆盖全部 17 片），再跑一趟 `--index`：写过的叶都在，稀疏性就从等式里消掉了。

    返回 `(kinds, rows)`：`kinds` = 出现过的 kind 集合，`rows` = 验证过的总行数
    （防空转的量：0 行 ⇒ 这条对账等于没跑）。
    """
    idx = tmp / "idx-kinds"
    rc, _, err = _run(h, ["--start", str(ckpt), "--apply", str(diff_path),
                          "--round", "1", "--index", str(idx), "--quiet"])
    path = idx / "idx" / "control.jsonl"
    if rc != 0 or not path.exists():
        return set(), 0
    if INDEX_HOOK is not None:
        INDEX_HOOK(path)
    kinds: set[str] = set()
    rows = 0
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        rows += 1
        kinds.add(json.loads(line).get("kind"))
    return kinds, rows


def _pick_entry(v, keys: list[str], keyvals: dict):
    """在读面上按身份键找回「我们写的那一条」；单值叶直接返回那个对象。"""
    if not keys:
        return v if isinstance(v, dict) else None
    if not isinstance(v, list):
        return None
    for e in v:
        if isinstance(e, dict) and all(e.get(k) == val for k, val in keyvals.items()):
            return e
    return None


def run(h, ck) -> None:
    # ══ 1. 静态纪律（原 web/src/views_tests.rs 的第一条测试）══════════════════════
    raw = VIEWS_JSON.read_text(encoding="utf-8")
    try:
        doc = json.loads(raw)
    except json.JSONDecodeError as e:
        ck.check("静态纪律：views.json 是合法 JSON", False, f"{VIEWS_JSON} 解析失败：{e}")
        return
    ck.check("静态纪律：views.json 是合法 JSON", isinstance(doc, dict),
             f"{VIEWS_JSON.name}（{len(raw)} 字节）")

    ver = doc.get("version")
    ck.check("静态纪律：version 是整数且 ≥ 2（v2 才有 leaf/owner/action 三种写行）",
             isinstance(ver, int) and not isinstance(ver, bool) and ver >= 2,
             f"version = {ver!r}")

    seen: set[str] = set()
    dups: list[str] = []
    shape_bad: list[str] = []
    views = list(each_view(doc))
    for v in views:
        vid = v.get("id")
        if not isinstance(vid, str) or not vid:
            shape_bad.append(f"一条视图没有 id：{json.dumps(v, ensure_ascii=False)[:120]}")
            continue
        if vid in seen:
            dups.append(vid)
        seen.add(vid)
        mount = v.get("mount")
        if not isinstance(mount, str):
            shape_bad.append(f"{vid}：缺 mount")
        layout = v.get("layout", "table")
        if layout not in LAYOUTS:
            shape_bad.append(f"{vid}：未知 layout `{layout}`（已知 {LAYOUTS}）")
        # `source` 的三种合法形态：
        #   * 字符串 = 常规来源；
        #   * **`null`（键必须在）** = 「这张卡故意不依赖任何记录」，只放不取记录的行
        #     （例如 `{ "owner": "global" }` 那条全局归属）——显式写 null 才允许，
        #     **漏写**仍然红（那多半是打错/漏了，而不是有意）；
        #   * inline 那条本来就不含列（它只是「路径 → 哪条视图」的映射表）。
        if mount != "inline":
            src = v.get("source", "<缺>")
            if src is None:
                if layout == "table":
                    shape_bad.append(f"{vid}：layout=table 不能 source: null（表没有来源没意义）")
            elif not isinstance(src, str):
                shape_bad.append(f"{vid}：source 既不是字符串也不是 null（{src!r}）")
            elif src == "<缺>":
                shape_bad.append(f"{vid}：缺 source（要「不依赖记录」就显式写 `\"source\": null`）")
        if mount == "select" and not isinstance(v.get("select_kind"), str):
            shape_bad.append(f"{vid}：select 挂载要声明 select_kind")
        if mount == "inline" and v.get("use_at") is None:
            shape_bad.append(f"{vid}：inline 挂载要声明 use_at")
    ck.check(f"静态纪律：{len(views)} 条视图的 id 唯一、mount/layout/source/select_kind 齐全",
             not dups and not shape_bad,
             "；".join(([f"id 重复：{dups}"] if dups else []) + shape_bad[:3])
             or f"{len(seen)} 个 id 全唯一")

    # 引用完整性：use / use_at / map_ref / label_from / card 指的都得存在。
    refs: list[tuple[str, str]] = []
    ref_bad: list[str] = []
    for v in views:
        vid = v.get("id", "?")
        if isinstance(v.get("use"), str):
            refs.append((vid, v["use"]))
        if isinstance(v.get("use_at"), dict):
            for t in v["use_at"].values():
                if isinstance(t, str):
                    refs.append((vid, t))
        if isinstance(v.get("card"), str):
            refs.append((vid, v["card"]))
        for c in v.get("columns") or []:
            m = c.get("map_ref")
            if isinstance(m, str) and m not in doc:
                ref_bad.append(f"{vid}：map_ref `{m}` 在 views.json 顶层不存在")
            if isinstance(c.get("label_from"), str):
                refs.append((vid, "label_from:" + c["label_from"]))
    for frm, to in refs:
        if to.startswith("label_from:"):
            root = to[len("label_from:"):].split(".")[0]
            if root not in ROOTS:
                ref_bad.append(f"{frm}：label_from 的根 `{root}` 不是已知根")
        elif to not in seen:
            ref_bad.append(f"{frm}：引用了不存在的视图 id `{to}`")
    ck.check(f"静态纪律：{len(refs)} 条引用（use/use_at/card/map_ref/label_from）都指的到",
             not ref_bad, "；".join(ref_bad[:3]) or f"{len(refs)} 条全部命中")

    # 每条路径表达式都合文法 + 绝对路径的根都在已知根里。
    n_expr, expr_bad = 0, []
    for v in views:
        vid = v.get("id", "?")
        for e in exprs_of(v):
            n_expr += 1
            try:
                parse_path(e)
            except ValueError as err:
                expr_bad.append(f"{vid}：路径 `{e}` 不合法——{err}")
                continue
            if is_absolute(e):
                r = root_of(e)
                if r not in ROOTS:
                    expr_bad.append(f"{vid}：路径 `{e}` 的根 `@{r}` 不是已知根（已知 {ROOTS}）")
    ck.check(f"静态纪律：{n_expr} 条路径表达式都合文法、@根 都在已知根里",
             not expr_bad, "；".join(expr_bad[:3]) or f"7 个已知根，{n_expr} 条表达式全部通过")

    # omit 与列**不许重叠**（「既声明显示、又声明不看」是自相矛盾的声明）。
    n_omit, omit_bad = 0, []
    for v in views:
        vid = v.get("id", "?")
        claimed = {s for s in (first_segment(c.get("path", "")) for c in v.get("columns") or []) if s}
        for o in v.get("omit") or []:
            n_omit += 1
            p = o.get("path") or ""
            why = (o.get("why") or "").strip()
            if not p:
                omit_bad.append(f"{vid}：omit 项缺 path")
            if not why:
                omit_bad.append(f"{vid}：omit `{p}` 必须写明理由（界面要把省略说出来）")
            if p and p in claimed:
                omit_bad.append(f"{vid}：omit `{p}` 与某一列重叠——要么显示、要么声明不看，不能两头都写")
    ck.check(f"静态纪律：{n_omit} 条 omit 都不与列重叠、且都写了理由",
             not omit_bad, "；".join(omit_bad[:3]) or f"{n_omit} 条全部合规")

    # ══ 2. 写面对账：leaves ∪ actions ∪ {势力} == FactionControlPatch.properties ══
    schema = json.loads(h.capture(["--control-schema"]))
    leaves = schema.get("leaves") or []
    actions = schema.get("actions") or []
    owner_field = schema.get("owner_field")
    remove_field = schema.get("remove_field")
    props = (((schema.get("definitions") or {}).get("FactionControlPatch") or {})
             .get("properties") or {})
    declared = {s["field"] for s in leaves} | {a["field"] for a in actions} | {"势力"}
    leaf_dups = sorted({s["field"] for s in leaves if [x["field"] for x in leaves].count(s["field"]) > 1})
    missing_decl = sorted(set(props) - declared)   # schemars 有、声明没有 = 加字段忘写声明
    extra_decl = sorted(declared - set(props))     # 声明有、schemars 没有 = 写了一个不存在的叶
    ck.check(f"写面对账：leaves({len(leaves)}) ∪ actions({len(actions)}) ∪ {{势力}} "
             f"== FactionControlPatch.properties({len(props)})，双向相等且 field 不重复",
             not missing_decl and not extra_decl and not leaf_dups and bool(props) and bool(leaves),
             "；".join(
                 ([f"引擎有而声明没写：{missing_decl}（加字段忘了写声明）"] if missing_decl else [])
                 + ([f"声明了引擎没有的叶：{extra_decl}"] if extra_decl else [])
                 + ([f"leaves[].field 重复：{leaf_dups}"] if leaf_dups else [])
             ) or (f"{len(declared)} 个键两边一模一样（{len(leaves)} 叶 + {len(actions)} 命令 + 势力）；"
                   f"owner_field={owner_field!r} remove_field={remove_field!r}"))

    # ══ 3. 读面对账：跑一局真世界，把每一片叶都写一次，再读回来 ══════════════════════
    tmp = Path(tempfile.mkdtemp(prefix="px-g4-"))
    ckpt, diff_path = tmp / "ckpt.json", tmp / "diff.json"
    rc, _, err = _run(h, ["--seed", str(SEED), "--round", str(ROUNDS), "--save", str(ckpt)])
    if rc != 0:
        ck.check("读面对账：起一局（seed 42 / 40 回合）", False,
                 f"planet_x 退出码 {rc}：{err[-400:]}")
        return
    pre = json.loads(h.capture(["--start", str(ckpt), "--control"]))

    # 选一势力：要它有舰、有城/建筑（多键叶的身份键得从这一局的真数据里拿）。
    def coverage(fac):
        return sum(1 for s in leaves if _candidates(s, fac))

    fac = max(pre["control"], key=lambda c: (coverage(c), len(json.dumps(c, ensure_ascii=False))))
    fid = fac["势力"]
    plans, unwritable = _plan_leaves(leaves, fac)
    ck.check(f"读面对账：选中的势力「{fid}」能给出全部 {len(leaves)} 片叶的写法（每片都要一个真实身份键）",
             not unwritable and len(plans) == len(leaves),
             "；".join(unwritable[:4]) or f"{len(plans)} 片叶各有一条写请求（seed {SEED} / {ROUNDS} 回合）")

    # 多键叶要写成**数组**（一条一条），单值叶写成对象——diff 的形状由 keys 决定。
    body: dict = {"势力": fid}
    for p in plans:
        spec = next(s for s in leaves if s["field"] == p["field"])
        body[p["field"]] = [p["patch"]] if _keys_of(spec) else p["patch"]
    diff = {"control": [body]}
    diff_path.write_text(json.dumps(diff, ensure_ascii=False, indent=1), encoding="utf-8")

    rc, out, err = _run(h, ["--start", str(ckpt), "--apply", str(diff_path), "--control"])
    receipt = err.strip()
    ck.check(f"读面对账：{len(plans)} 片叶的 diff 被引擎原样收下（rc 0、回执里没有 WARN_APPLY_SKIPPED）",
             rc == 0 and "WARN_APPLY_SKIPPED" not in receipt and "ERR_" not in receipt,
             f"rc={rc}；stderr={receipt[:500] or '（空 = 一片都没丢）'}")
    try:
        post = json.loads(out)
    except json.JSONDecodeError as e:
        ck.check("读面对账：--apply 之后的 --control 是合法 JSON", False,
                 f"{e}；stdout 前 300 字：{out[:300]!r}")
        return
    post_fac = next((c for c in post["control"] if c.get("势力") == fid), None)
    if post_fac is None:
        ck.check("读面对账：写完之后该势力还在读面上", False, f"读面里没有 {fid}")
        return

    # 3a. 防空转：这一局真的写进去了 N 片叶（按哨兵从**读面**里认领，不信回执）。
    lost: list[str] = []
    for p in plans:
        spec = next(s for s in leaves if s["field"] == p["field"])
        e = _pick_entry(post_fac.get(p["field"]), _keys_of(spec), p["keys"])
        if e is None:
            lost.append(f"{p['field']}：读面上找不到 {p['keys']}={p['keys'] and p['patch']}"
                        f" 那一条（写了却没出现）")
            continue
        wrong = {k: (v, e.get(k)) for k, v in p["sentinel"].items() if e.get(k) != v}
        if wrong:
            lost.append(f"{p['field']}：哨兵值没落地 {wrong}——条目原文 "
                        f"{json.dumps(e, ensure_ascii=False)[:200]}")
    n_written = len(plans) - len(lost)
    ck.check(f"读面对账（防空转）：这一局真的写进去了 {n_written} 片叶（声明 {len(leaves)} 片，"
             f"每片按哨兵值从读面上认领回来）",
             n_written == len(leaves) and not lost,
             "；".join(lost[:4]) or
             f"{fid}：{len(plans)} 片叶全部按哨兵命中（{SEED} / {ROUNDS} 回合，不是空表通过）")

    # 3b. 字段集：每条条目 ⊇ keys ∪ values ∪ carries ∪ read_only ∪ {mode}，⊆ 那个集合 ∪ {remove}。
    field_bad: list[str] = []
    remove_seen: list[str] = []
    n_entries = 0
    for c in post["control"]:
        for spec in leaves:
            v = c.get(spec["field"])
            entries = v if (_keys_of(spec) and isinstance(v, list)) else ([v] if isinstance(v, dict) else [])
            for e in entries:
                n_entries += 1
                required = set(_keys_of(spec)) | set(spec.get("values") or []) | \
                    set(spec.get("carries") or []) | set(spec.get("read_only") or []) | {owner_field}
                allowed = required | {remove_field}
                miss = sorted(required - set(e))
                extra = sorted(set(e) - allowed)
                if remove_field in e:
                    remove_seen.append(f"{c.get('势力')}.{spec['field']}")
                    if _keys_of(spec):
                        extra = extra + [f"{remove_field}（列表叶上不该有）"]
                if miss or extra:
                    field_bad.append(
                        f"{c.get('势力')}.{spec['field']}：缺 {miss}、多 {extra}；"
                        f"条目 {json.dumps(e, ensure_ascii=False)[:200]}；"
                        f"声明 {json.dumps(spec, ensure_ascii=False)}")
    ck.check(f"读面对账：{n_entries} 个读面条目的字段集 == keys ∪ values ∪ carries ∪ read_only "
             f"∪ {{{owner_field}}}（无缺、无多余）",
             not field_bad,
             f"实测 {remove_field!r} 出现 {len(remove_seen)} 次"
             f"{'（' + '、'.join(sorted(set(remove_seen))[:5]) + '）' if remove_seen else '（读面根本不发它）'}"
             + ("；" + "；".join(field_bad[:3]) if field_bad else ""))

    # 3c. 形状：keys 空 ⇔ 对象；非空 ⇔ 数组且每一条带齐身份键。
    shape_bad2: list[str] = []
    n_list_entries = 0
    for c in post["control"]:
        for spec in leaves:
            v = c.get(spec["field"])
            if not _keys_of(spec):
                if isinstance(v, list):
                    shape_bad2.append(f"{c.get('势力')}.{spec['field']}：keys 为空（势力级单叶）"
                                      f"但读面给的是**数组**")
                continue
            if v is None:
                continue
            if not isinstance(v, list):
                shape_bad2.append(f"{c.get('势力')}.{spec['field']}：keys={_keys_of(spec)} "
                                  f"但读面给的是 {type(v).__name__}，不是数组")
                continue
            for e in v:
                n_list_entries += 1
                miss = [k for k in _keys_of(spec) if not isinstance(e, dict) or k not in e]
                if miss:
                    shape_bad2.append(f"{c.get('势力')}.{spec['field']}：条目缺身份键 {miss}——"
                                      f"{json.dumps(e, ensure_ascii=False)[:200]}")
    ck.check("读面对账：keys 空 ⇔ 读面是对象；keys 非空 ⇔ 是数组且每条带齐身份键",
             not shape_bad2,
             "；".join(shape_bad2[:3]) or
             f"14 种叶形状全对（列表叶共 {n_list_entries} 条条目，身份键一条不缺）")

    # ══ 4. 认领完整性：铁律 R 的写面对偶（每个叶都得有人认领，否则界面上凭空消失）══════
    def leaf_field_of(path: str) -> str:
        segs = split_segments(path)
        if len(segs) < 2:
            raise ValueError(f"路径段不足：`{path}`")
        if segs[0].split("[")[0].lstrip("@") != "control":
            raise ValueError(f"leaf 行的路径要以 @control 开头：`{path}`")
        return segs[1].split("[")[0]

    leaf_rows: list[tuple[str, str, str]] = []
    action_rows: list[tuple[str, str]] = []
    owner_rows: list[tuple[str, str]] = []
    new_rows: list[str] = []
    row_bad: list[str] = []
    for v in views:
        vid = v.get("id", "?")
        for c in v.get("columns") or []:
            if isinstance(c.get("leaf"), str):
                try:
                    f = leaf_field_of(c["leaf"])
                    leaf_rows.append((vid, c["leaf"], f))
                    # `new: true` = 「这一行的身份键由人现填」（对偶于 `keys_from` 的「从名单里挑」）。
                    # 只对**多键叶**有意义：势力级单叶（`keys` 为空）本来就每势力一片，
                    # 「现造一个键」对它不成立 ⇒ 写了就是声明写错（静默忽略 = 又一个哑巴失败）。
                    if c.get("new"):
                        keys = {s["field"]: s.get("keys") or [] for s in leaves}.get(f)
                        if not keys:
                            new_rows.append(
                                f"{vid}：`{f}` 是势力级单叶（keys 为空），`new` 对它没有意义")
                except ValueError as e:
                    row_bad.append(f"{vid}：leaf 行 `{c['leaf']}` —— {e}")
            if isinstance(c.get("action"), str):
                action_rows.append((vid, c["action"]))
            if isinstance(c.get("owner"), str):
                owner_rows.append((vid, c["owner"]))

    declared_fields = {s["field"] for s in leaves}
    declared_actions = {a["field"] for a in actions}
    scope_props = set((((schema.get("definitions") or {}).get("ControlScopePatch") or {})
                       .get("properties") or {}))

    omit = doc.get("write_omit") or []
    omit_ok = {e.get("field") for e in omit
               if isinstance(e.get("why"), str) and e["why"].strip()}
    omit_no_why = [e.get("field") for e in omit
                   if not (isinstance(e.get("why"), str) and e["why"].strip())]

    claimed_leaves = {f for _, _, f in leaf_rows}
    claimed_actions = {a for _, a in action_rows}
    unclaimed = sorted((declared_fields | declared_actions) - claimed_leaves - claimed_actions - omit_ok)
    ck.check(f"认领完整性：{len(declared_fields)} 片叶 + {len(declared_actions)} 条命令，每一个都被 "
             f"leaf 行 / action 行认领，或在 write_omit 里写明理由（无凭空消失）",
             not unclaimed and not omit_no_why,
             "；".join(
                 ([f"没有任何 leaf/action 行认领、也不在 write_omit 里：{unclaimed}"] if unclaimed else [])
                 + ([f"write_omit 少了 why：{omit_no_why}"] if omit_no_why else [])
             ) or (f"leaf 行认领 {len(claimed_leaves)} 种、action 行认领 {len(claimed_actions)} 种、"
                   f"write_omit 带理由地免掉 {len(omit_ok)} 种"))

    leaf_ui = doc.get("leaf_ui") or {}
    action_ui = doc.get("action_ui") or {}
    orphans = sorted(
        [f"leaf 行 → {f}" for _, _, f in leaf_rows if f not in declared_fields]
        + [f"action 行 → {a}" for _, a in action_rows if a not in declared_actions]
        + [f"leaf_ui → {k}" for k in leaf_ui if k not in declared_fields]
        + [f"action_ui → {k}" for k in action_ui if k not in declared_actions]
        + [f"owner 行 → {o}（{vid}）" for vid, o in owner_rows if o not in scope_props]
        + row_bad
    )
    ck.check(f"认领完整性：{len(leaf_rows)} 条 leaf 行 / {len(action_rows)} 条 action 行 / "
             f"{len(leaf_ui)} 个 leaf_ui / {len(action_ui)} 个 action_ui / {len(owner_rows)} 条 owner 行 "
             f"都对得上引擎的真声明（无孤儿）",
             not orphans,
             "；".join(orphans[:4]) or
             (f"leaf_ui 覆盖 {len(leaf_ui)}/{len(declared_fields)} 片叶、action_ui "
              f"{len(action_ui)}/{len(declared_actions)} 条命令；owner 行的作用域键都在 "
              f"ControlScopePatch 里（{sorted(scope_props)}）"))

    # ══ 5. 名词覆盖率：**界面上当名词显示的每一列，都能弹出解释** ══════════════════
    #
    # 用户裁决：「所有 UI 都用名词，**鼠标移上去弹窗显示注释/解释**」。弹的那段文字**全部**
    # 来自引擎发的 `--nouns`（= web 的 `GET /api/schema`，同一份实现、两个出口）：
    # 实体字段的 `///`、回合视图字段的 `///`、投影每张表的 `column_docs` 与列内联 description。
    #
    # 判据：**当名词显示的列**（裸字段列 = `path` 是单个标识符；控制行的标签）必须能在语料里
    # 查到词条——查不到就是"弹不出东西"的哑巴失败（引擎那边缺文档，或键名对不上）。
    # ⚠ 口径**不是**"给 128 个投影列全写文档"：只要求**界面真的显示**的那些。
    rc, out, _ = _run(h, ["--nouns"])
    corpus: dict[str, str] = {}
    corpus_ok = False
    if rc == 0:
        try:
            doc_nouns = json.loads(out)
            corpus_ok = isinstance(doc_nouns, dict)

            def eat(node, into, depth=0):
                if depth > 64 or not isinstance(node, (dict, list)):
                    return
                if isinstance(node, list):
                    for x in node:
                        eat(x, into, depth + 1)
                    return
                props = node.get("properties")
                if isinstance(props, dict):
                    for k, v in props.items():
                        d = (v or {}).get("description")
                        if isinstance(d, str) and d.strip():
                            into.setdefault(k, d)
                for k, v in node.items():
                    if k == "columns" and isinstance(v, dict):
                        for ck_, cv in v.items():
                            if isinstance(cv, dict) and isinstance(cv.get("description"), str):
                                into.setdefault(ck_, cv["description"])
                    eat(v, into, depth + 1)

            if corpus_ok:
                eat(doc_nouns.get("state") or {}, corpus, 0)
                eat(doc_nouns.get("view") or {}, corpus, 0)
                # 控制面那半：控制行的**字段名**（`首都`/`投资预算`…）与
                # 作用域键（`global`/`factions`…）。界面显示的是中文标签，标签查不到时按字段名查。
                eat(doc_nouns.get("control") or {}, corpus, 0)
                for sec, tables in (doc_nouns.get("projection") or {}).items():
                    if not isinstance(tables, dict):
                        continue
                    for t, tv in tables.items():
                        if not isinstance(tv, dict):
                            continue
                        for k, v in (tv.get("column_docs") or {}).items():
                            corpus[k] = v
                        for k, v in (tv.get("columns") or {}).items():
                            if isinstance(v, dict) and isinstance(v.get("description"), str):
                                corpus.setdefault(k, v["description"])
        except json.JSONDecodeError:
            corpus_ok = False
    ck.check("名词覆盖率：`--nouns` 给得出语料（悬停弹窗的全部文字来源）",
             corpus_ok,
             f"名词 {len(corpus)} 个" if corpus_ok else f"退出码 {rc}：{out[:120]}")

    BARE = re.compile(r"^[A-Za-z_一-鿿][\w一-鿿]*$")
    considered = 0
    uncovered: list[str] = []
    for v in views:
        vid = v.get("id", "?")
        for c in v.get("columns") or []:
            if not isinstance(c, dict):
                continue
            is_control = any(isinstance(c.get(k), str) for k in ("leaf", "owner", "action"))
            if is_control:
                # 控制行的名字：`label`（views.json 里声明的中文名）或叶/命令的**字段名**。
                label = c.get("label")
                path = c.get("leaf") or c.get("owner") or c.get("action") or ""
                field = str(path).split(".")[-1].split("[")[0]
                keys = [k for k in (label, field) if k]
            else:
                path = c.get("path")
                # 表达式列（`@post.power_share.${势力}`）：它**自己**不是名词，但只要列上声明了
                # `noun`（「这一列说的是哪个名词」），它就跟裸字段列一样必须查得到解释。
                noun = c.get("noun")
                if isinstance(noun, str) and noun:
                    keys = [noun]
                elif not isinstance(path, str) or not BARE.match(path):
                    continue        # 既不是裸字段列、也没声明 noun ⇒ 表头是自由文本
                else:
                    keys = [k for k in (c.get("label"), path) if k]
            if not keys:
                continue
            considered += 1
            if not any(k in corpus for k in keys):
                uncovered.append(f"{vid}：`{keys[0]}`（也试过 {keys[1:] or '无'}）")
    ck.check(f"名词覆盖率：{considered} 个当名词显示的列全都能弹出解释"
             f"（悬停弹窗查得到词条，不是空框）",
             corpus_ok and not uncovered and considered >= 40,
             "；".join(uncovered[:5]) or (
                 f"语料 {len(corpus)} 个名词，覆盖 {considered} 个界面名词（下限 40）"
                 if considered >= 40 else f"只算到 {considered} 个名词，判据可能空转了"))

    # 表达式列的 `noun` 声明：**必须在语料里查得到**。
    #
    # 为什么需要这个字段：表达式是**取数路径**、不是名词（`@post.power_share.${势力}` 里没有
    # "名词"那一层），所以只有声明才知道该弹哪条解释。**不许去表达式里猜**——`@state.ships
    # [?舰名=…].势力` 的第一个裸段是 `ships`，猜出来必错。
    declared: list[tuple[str, str]] = []
    for v in views:
        for c in v.get("columns") or []:
            if isinstance(c, dict) and isinstance(c.get("noun"), str) and c["noun"]:
                declared.append((v.get("id", "?"), c["noun"]))
    bad_decl = [f"{vid}：`noun: {n}` 在语料里查不到（弹空框）" for vid, n in declared if n not in corpus]
    ck.check(f"名词覆盖率：{len(declared)} 条表达式列的 `noun` 声明都能查到解释"
             f"（这一列说的是哪个名词）",
             corpus_ok and not bad_decl and len(declared) >= 20,
             "；".join(bad_decl[:5]) or (
                 f"声明 {len(declared)} 条，全部命中语料（下限 20）"
                 if len(declared) >= 20 else f"只声明了 {len(declared)} 条，判据可能空转了"))

    # 另一半：**与引擎名词逐字相同**的 `label` 一律不写（"该退的都退了"）——
    # 列头默认就是键名，再抄一遍只会让"哪个是权威"变含糊；要换词（`舰名`→`舰`）或加格式时才写。
    dup_labels = []
    for v in views:
        for c in v.get("columns") or []:
            if not isinstance(c, dict):
                continue
            path, label = c.get("path"), c.get("label")
            if isinstance(path, str) and isinstance(label, str) and BARE.match(path) and path == label:
                dup_labels.append(f"{v.get('id', '?')}：`{path}` 的 label 与键名逐字相同（该退掉）")
    ck.check("名词覆盖率：裸字段列的 `label` 不与引擎键名逐字重复（列头默认就是键名）",
             not dup_labels,
             "；".join(dup_labels[:4]) or "0 条重复（`label` 只在要覆盖/加格式时才写）")

    # ══ 5d. 名词覆盖率（**静态**）：渲染字段名标签的 JS 模块都必须挂 tip ═══════════════
    #
    # 为什么要有这一条（§5 看不见的那半边）：上面 §5 查的是**声明**——`views.json` 里
    # `columns` 中当名词显示的列。可是**通用 widget 自己渲染出来的**字段名不在任何 `columns`
    # 里，§5 对它们**一条都不红**。`jsonview.js` 的原始 JSON 视图就是实机反例：
    # `{字段名: 值}` 的树里 `global`/`bodies`/`cities`/`factions`/`舰队默认姿态`/`福利预算`
    # 明明在界面上、语料（`--nouns`）里也有，却**从不挂弹窗**（hover 无反应）。
    # 这是**哑巴失败**的典型：没有报错、没有空白屏，只是「没反应」——本仓最拉黑的那种。
    #
    # 判据：扫 `web/static/*.js`（**去掉注释**后的代码），算出三份清单——
    #   * 「渲染字段名标签的文件」= 代码里出现 `FIELD_LABEL_CLASSES` 里任一**字段名标签类名**；
    #   * 「挂 tip 的文件」= 代码里出现 `TIP_MOUNTS` 里任一写法（`Tip.attach` / `ctx.tip`）；
    #   * 「真的调 `Tip.attach` 的文件」= 那条链的**末端**（widget → `ctx.tip` → 宿主 →
    #     `Tip.attach` → tip.js）。末端没人接，前面挂得再全也是哑的。
    # 要求：前者 **⊆** 中间那份（谁渲染了名词标签，谁就得把 tip 挂上——机制是同一套
    # `Tip.attach`，各家不许自己查表），且末端**至少有一个**。
    # **防空转**：清单非空 + 标签出现次数有下限，实测数字印在 detail 里。
    #
    # ⚠ 这条守的是「**接线**在不在」，不守「每个键在语料里查得到」——后者是 §5 的活。
    #   查不到的键 `Tip.attach` 静默不弹（与从前行为一致），那是引擎缺文档，由 §5/§5b 报。
    label_files, tip_files, host_files, label_hits = [], [], [], 0
    for js_path in sorted(STATIC_JS.glob("*.js")):
        src = _js_code(js_path)
        found = [c for c in FIELD_LABEL_CLASSES if c in src]
        if found:
            label_files.append(js_path.name)
            label_hits += sum(src.count(c) for c in found)
        if any(m.search(src) for m in TIP_MOUNTS):
            tip_files.append(js_path.name)
        if TIP_HOST.search(src):
            host_files.append(js_path.name)
    unmounted = [f for f in label_files if f not in tip_files]
    ck.check(f"名词覆盖率：{len(label_files)} 个渲染字段名标签的 JS 模块都挂了 tip"
             f"（{label_hits} 处标签、{len(host_files)} 个模块真的调了 `Tip.attach`；"
             f"没有「看得见却弹不出解释」的视图）",
             not unmounted and bool(host_files) and len(label_files) >= 2 and label_hits >= 8,
             "；".join(
                 [f"`{f}` 渲染字段名标签却不挂 tip ⇒ 界面上那些名词 hover 没反应" for f in unmounted]
                 + ([] if host_files else
                    ["没有任何模块调 `Tip.attach` ⇒ `ctx.tip` 转发到空处，全部弹窗哑掉"])
             ) or
             (f"标签模块 {label_files} ⊆ 挂 tip 的模块 {tip_files}（宿主 {host_files}）；"
              f"标签 {label_hits} 处 / 模块 {len(label_files)} 个（下限 8 处 / 2 个）"))

    # `new: true`（身份键由人现填）只对**多键叶**成立；写在势力级单叶上是声明写错。
    new_rows_total = sum(1 for v in views for c in (v.get("columns") or []) if c.get("new"))
    ck.check(f"认领完整性：{new_rows_total} 条 `new: true` 行都挂在多键叶上"
             f"（单叶每势力一片，「现造一个身份键」对它不成立）",
             not new_rows,
             "；".join(new_rows[:3]) or f"本帧 {new_rows_total} 条，全部合法（防空转）")


    # ══ 5b. 文档对账：**发射的每一条 `description` == 源码里那条 `///`** ════════════
    #
    # 为什么单看 `--nouns` 不够：它**自洽**。真实的腐蚀长这样——schemars 0.8.22 的
    # `attr/doc.rs::get_doc` 里有一段向后兼容 hack：只要文档的**所有行**都以 `*` 开头，
    # 就当成 `/** … */` 风格逐行剥掉一个 `*`。于是**单行**注释若以 `**加粗**` 开头
    # （`/// **完整的世界快照**（…`）就被误判，弹窗里收到的是 `*完整的世界快照**（…`
    # ——**坏掉的 markdown**（实测 46 条，且 46/46 全是单行注释）。语料本身看不出对错，
    # 必须拿**源码**当尺子。（那段 hack 已随 schemars 1.2.2 消失；这条判据是**量具**，
    # 防的是这一类：剥字符、吞行、把 A 结构体的注释发射到 B 上、静默丢文档……）
    #
    # 两个方向都查：
    #   * 正向：state / view / control 三份 schemars schema 里**每一条** `description`
    #     （三份的根 / 定义级 / 字段级 / `oneOf` 变体级）都要在源码里找到出处，
    #     且**空白归一后逐字相等**；
    #   * 反向：schema 里出现的每个结构体，**源码里带 `///` 的字段都真的发射了**
    #     （丢一段文档 = 弹窗少一段话，正是"失败看起来像成功"）。
    #
    # ⚠ 口径与边界（写清楚，免得下一个人以为它没在守）：
    #   * 只认本仓的源码形状（`struct`/`enum` + 紧挨着的 `///`；`#[serde(rename)]` 换键名）；
    #     **解析不出来的发射项一律判红**（`unmapped`），不许静默跳过；
    #   * `projection` 那半（`column_docs` / 列内联 `description`）是**手写**文案、不走
    #     schemars，不在本判据范围——它们由上面 §5 的覆盖率判据盯着；
    #   * 枚举**变体级**的 `///` 只查正向：`#[serde(into/try_from)]` 的 enum
    #     （`DeathCause` / `SpawnVia` / `FoundingHow`）在新版 schemars 下不再发射变体级
    #     schema（见 `.agents/notes/doc-pipeline.md`），源码有注释而发射端没有 ⇒
    #     反向着对变体不成立。
    try:
        doc_nouns = json.loads(h.capture(["--nouns"]))
    except Exception as e:  # noqa: BLE001  （拿不到语料本身就是红）
        doc_nouns = None
        nouns_fail = f"{type(e).__name__}: {e}"
    else:
        nouns_fail = ""

    containers = doc_containers()
    _GENERIC_N = re.compile(r"^(.+?)\d+$")     # schemars 给重名实例编的号（`Control` → `Control2`…）

    pairs: list[tuple[str, str, str]] = []      # (位置, 源码期望, 发射原文)
    unmapped: list[str] = []                    # 发射了、但源码里找不到出处的
    src_files: set[str] = set()                 # 贡献过对账的源文件
    structs_seen: set[str] = set()              # schema 里出现过的结构体（反向判据用）

    def resolve(name: str) -> dict | None:
        cont = containers.get(name)
        if cont is None:
            m = _GENERIC_N.match(name)
            if m:
                cont = containers.get(m.group(1))   # `Control2` → `Control`
        return cont

    def add_pair(where: str, doc: str, emitted: str, file: str) -> None:
        src_files.add(file)
        pairs.append((where, _doc_to_description(doc), emitted))

    for sec in ("state", "view", "control"):
        sch = (doc_nouns or {}).get(sec) if isinstance(doc_nouns, dict) else None
        if not isinstance(sch, dict):
            continue
        # 根：schemars 把根的类型名写在 `title` 里
        root = resolve(str(sch.get("title"))) if isinstance(sch.get("title"), str) else None
        if root is not None and isinstance(sch.get("description"), str):
            add_pair(f"{sec}（根 {sch['title']}）", root["doc"], sch["description"], root["file"])
        for name, spec in sorted((sch.get("definitions") or {}).items()):
            spec = spec if isinstance(spec, dict) else {}
            cont = resolve(name)
            if cont is None:
                if isinstance(spec.get("description"), str):
                    unmapped.append(f"{sec}:{name}（定义）")
                continue
            structs_seen.add(f"{sec}:{name}")
            if isinstance(spec.get("description"), str):
                add_pair(f"{sec}:{name}", cont["doc"], spec["description"], cont["file"])
            one = spec.get("oneOf")
            # 枚举变体按**声明序**对齐 `oneOf`（两边长度不等就不猜，那些项落进 unmapped）
            aligned = (cont["kind"] == "enum" and isinstance(one, list)
                       and len(one) == len(cont["variants"]))
            for i, sub in enumerate(one if isinstance(one, list) else []):
                sub = sub if isinstance(sub, dict) else {}
                var = cont["variants"][i] if aligned else None
                if isinstance(sub.get("description"), str):
                    if var is None:
                        unmapped.append(f"{sec}:{name}#{i}（变体）")
                    else:
                        add_pair(f"{sec}:{name}#{i}（{var['name']}）", var["doc"],
                                 sub["description"], var["file"])
                for key, field in sorted((sub.get("properties") or {}).items()):
                    field = field if isinstance(field, dict) else {}
                    src = (var or {}).get("fields", {}).get(key)
                    if not isinstance(field.get("description"), str):
                        continue
                    if src is None:
                        unmapped.append(f"{sec}:{name}#{i}.{key}（变体字段）")
                    else:
                        add_pair(f"{sec}:{name}#{i}.{key}", src[0], field["description"],
                                 (var or {}).get("file", cont["file"]))
            for key, field in sorted((spec.get("properties") or {}).items()):
                field = field if isinstance(field, dict) else {}
                src = cont["fields"].get(key)
                if not isinstance(field.get("description"), str):
                    continue
                if src is None:
                    unmapped.append(f"{sec}:{name}.{key}（字段）")
                else:
                    add_pair(f"{sec}:{name}.{key}", src[0], field["description"], cont["file"])

    drifted = [(w, want, got) for w, want, got in pairs if want != _norm_text(got)]
    ck.check(f"文档对账：{len(pairs)} 条 description 逐字 == 源码 `///`"
             f"（{len(src_files)} 个源文件）",
             bool(nouns_fail) is False and len(pairs) >= 550 and len(src_files) >= 10
             and not drifted,
             nouns_fail or "；".join(
                 f"{w}：源码 `{want[:40]}` ≠ 发射 `{got[:40]}`" for w, want, got in drifted[:5]
             ) or (f"实测 {len(pairs)} 条全部逐字相等（下限 550），来自 {len(src_files)} 个源文件"
                   f"（下限 10）" if len(pairs) >= 550 and len(src_files) >= 10 else
                   f"只对到 {len(pairs)} 条 / {len(src_files)} 个源文件 ⇒ 判据可能空转了"))

    # 反向：源码写了注释的字段，发射端**不许悄悄丢**（丢文档 = 弹窗少一段话）。
    dropped: list[str] = []
    reverse_checked = 0
    for tag in sorted(structs_seen):
        sec, name = tag.split(":", 1)
        cont = resolve(name)
        spec = (((doc_nouns or {}).get(sec) or {}).get("definitions") or {}).get(name) or {}
        emitted = spec.get("properties") or {}
        for key, (doc, _rust) in cont["fields"].items():
            if not _doc_to_description(doc):
                continue
            reverse_checked += 1
            got = (emitted.get(key) or {}).get("description")
            if not isinstance(got, str) or not got.strip():
                dropped.append(f"{sec}:{name}.{key}")
    ck.check(f"文档对账：{reverse_checked} 个带 `///` 的字段都真的发射了（无静默丢文档）",
             not nouns_fail and not dropped and reverse_checked >= 150,
             nouns_fail or "；".join(f"{d} 有注释没发射" for d in dropped[:5])
             or (f"实测 {reverse_checked} 个字段带注释、全部发射（下限 150）"
                 if reverse_checked >= 150 else
                 f"只查到 {reverse_checked} 个带注释的字段 ⇒ 判据可能空转了"))

    ck.check(f"文档对账：发射的 {len(pairs)} 条 description 都能在源码里找到出处"
             f"（{len(containers)} 个类型里查）",
             not nouns_fail and not unmapped,
             nouns_fail or "；".join(unmapped[:5]) or
             f"{len(pairs)} 条全部对上了源头的 `///`（无凭空出现的文案）")


    # ══ 6. 身份键：谁靠哪个字段认人 —— 引擎**一处**声明，且在真世界里**存在且唯一** ══
    #
    # 「一张表/一个结构体靠哪个字段认人」以前在 Python（`_harness._ID_KEY`）、kit、JS 三处
    # 各自维护**镜像小表**：引擎改了字段名或换了身份键，它们**不会红**，只会悄悄用旧键去
    # 找记录——典型的"失败看起来像成功"。现在唯一真值是 `model::IDENTITY`（结构体）与投影
    # 的 `LAZY`（表），随 `--nouns`（= web 的 `GET /api/schema`）一起发出来，三端都来问它。
    #
    # 这条守卫不验"声明是否自洽"，而是验**声明与真实世界对不对得上**：
    #   ① 指名的字段在 schema 里**真的存在**；
    #   ② 在一局真跑的世界上**真的唯一**（重复 ⇒ 红，打印前几个冲突）；
    #   ③ 防空转：结构体/表/行数都得 > 0。
    try:
        corpus = json.loads(h.capture(["--nouns"]))
        nouns_err = None
    except Exception as e:  # noqa: BLE001  （拿不到语料本身就是红）
        corpus, nouns_err = None, f"{type(e).__name__}: {e}"
    if corpus is None:
        ck.check("身份键：`--nouns` 发得出 identity 声明（谁靠哪个字段认人）", False, nouns_err or "")
    else:
        ident = corpus.get("identity") or {}
        id_structs = ident.get("structs") or {}
        id_tables = ident.get("tables") or {}
        defs = (corpus.get("state") or {}).get("definitions") or {}
        props = (corpus.get("state") or {}).get("properties") or {}
        lazy = ((corpus.get("projection") or {}).get("lazy") or {})

        missing_attr = []
        for struct, field in sorted(id_structs.items()):
            spec = (defs.get(struct) or {}).get("properties") or {}
            if field not in spec:
                missing_attr.append(f"`{struct}.{field}` 不在 schema 的属性里（有：{sorted(spec)[:6]}）")
        ck.check(f"身份键：{len(id_structs)} 个结构体声明的身份字段在 schema 里真的存在",
                 bool(id_structs) and not missing_attr,
                 "；".join(missing_attr[:4]) or "、".join(f"{k}→{v}" for k, v in sorted(id_structs.items())))

        # ②a 结构体侧：拿 §3 存下来的那局 state，逐实体数组验唯一。
        state = ((json.loads(ckpt.read_text(encoding="utf-8")).get("round_state") or {})
                 .get("state") or {})
        dup_rows: list[str] = []
        checked_rows = 0
        for kind, spec in props.items():
            ref = ((spec or {}).get("items") or {}).get("$ref") or ""
            field = id_structs.get(ref.rsplit("/", 1)[-1])
            rows = state.get(kind)
            if not field or not isinstance(rows, list):
                continue
            checked_rows += len(rows)
            vals = [r.get(field) for r in rows]
            dups = sorted({str(v) for v in vals if v is not None and vals.count(v) > 1})
            if dups:
                dup_rows.append(f"{kind}.{field} 有重复：{dups[:3]}")
        ck.check(f"身份键：声明的身份字段在真世界里唯一（{checked_rows} 行实体）",
                 not dup_rows and checked_rows > 0,
                 "；".join(dup_rows[:3]) or
                 (f"{checked_rows} 行里没有重复（seed {SEED} / {ROUNDS} 回合的 state）"
                  if checked_rows else "一行都没验到 ⇒ 判据空转了"))

        # ②b 表侧：跑一次 --index，**逐轮**验声明的键唯一（表是 `(round, key)` 索引的）。
        idx = tmp / "idx-identity"
        rc2, _, err2 = _run(h, ["--seed", str(SEED), "--round", str(ROUNDS), "--index", str(idx)])
        if rc2 != 0:
            ck.check("身份键：投影表的身份键在真世界里唯一", False,
                     f"`--index` 退出码 {rc2}：{err2[-300:]}")
        else:
            bad_tab: list[str] = []
            tabs, rows_tab = 0, 0
            for name, t in sorted(lazy.items()):
                key = id_tables.get(name)
                if not key:
                    continue                      # 这张表没声明键（内联/聚合表）
                if key not in ((t or {}).get("columns") or {}):
                    bad_tab.append(f"{name}.{key} 不在 columns 里")
                    continue
                # `table` 是**相对 index 输出目录**的路径（`idx/ships.jsonl`）
                path = idx / (t.get("table") or "")
                if not path.exists():
                    bad_tab.append(f"{name} 的 table 文件不在：{t.get('table')}")
                    continue
                rows = [json.loads(l) for l in path.read_text(encoding="utf-8").splitlines() if l.strip()]
                tabs += 1
                rows_tab += len(rows)
                groups: dict = {}
                if t.get("round"):
                    for r in rows:
                        groups.setdefault(r.get("round"), []).append(r)
                else:
                    groups[None] = rows
                for rnd, rs in groups.items():
                    vals = [r.get(key) for r in rs]
                    dups = sorted({str(v) for v in vals if v is not None and vals.count(v) > 1})
                    if dups:
                        bad_tab.append(f"{name} r{rnd}：{key} 重复 {dups[:3]}")
            ck.check(f"身份键：{tabs} 张投影表声明的键在真世界里**逐轮唯一**（{rows_tab} 行）",
                     bool(id_tables) and not bad_tab and tabs >= 5 and rows_tab > 0,
                     "；".join(bad_tab[:4]) or
                     (f"{tabs} 张表 / {rows_tab} 行，按 (round, key) 分组都没有重复"
                      if tabs >= 5 and rows_tab else
                      f"只验到 {tabs} 张表 / {rows_tab} 行 ⇒ 判据可能空转了"))

        # ③ 前端那半：`views.json` 的视图 `key` 是**它自己认人的字段**（JS 读它来配对行）。
        # 对 `@state.<实体>[*]` 这类视图，它必须**就是引擎声明的那个身份字段**——两边漂移
        # 的话，选行/配对会悄悄错位（前端"看起来有值"）。这样三端就都问同一处了：
        # Python 问 `identity_keys()`、kit 问 manifest 的 `keys`、JS 问声明的 `key`（在这里被钉住）。
        drift, views_checked = [], 0
        for v in views:
            src = v.get("source") or ""
            m = re.fullmatch(r"@state\.([a-z_]+)\[\*\]", src)
            k = v.get("key")
            if not m or not isinstance(k, str):
                continue
            struct = ""
            for kind, spec in props.items():
                if kind == m.group(1):
                    struct = (((spec or {}).get("items") or {}).get("$ref") or "").rsplit("/", 1)[-1]
            want = id_structs.get(struct)
            if not want:
                continue
            views_checked += 1
            if k != want:
                drift.append(f"{v.get('id')}（{src}）声明 key=`{k}`，引擎说 `{struct}` 的身份是 `{want}`")
        ck.check(f"身份键：{views_checked} 个实体视图的 `key` 与引擎的身份声明一致（前端不再各认各的）",
                 views_checked > 0 and not drift,
                 "；".join(drift[:4]) or
                 (f"{views_checked} 个视图逐字一致（如 ship-table→舰名）"
                  if views_checked else "一个实体视图都没验到 ⇒ 判据空转了"))

    # ══ 7. `control` 表的 `kind` 词表 == 声明里的叶名（逐字、双向）══════════════════
    #
    # 见模块文档第 5 条。这里只说**为什么口径是「声明 − not_in_index」而不是「等于全部声明」**：
    # 设计图库（结构叶，住 `derived.blueprints`）与 `建筑`（命令，不是叶）**结构上就不该**出现在
    # 这张标量表里，把它们算进来等于逼引擎发一份注定漂移的第二表示（`value: any` 列塞不下结构）。
    # 所以例外不是"测试放过"，而是**引擎声明里带理由的事实**（`leaves[].not_in_index`），
    # 本判据逐条查它的理由非空，并用下限挡住"把所有叶都标成例外"这条逃生门。
    ctrl_kinds, ctrl_rows = _control_kinds_from_index(h, ckpt, diff_path, tmp)
    declared_all = {s["field"] for s in leaves} | {a["field"] for a in actions}
    absent, why_bad = {}, []
    for spec in list(leaves) + list(actions):
        if "not_in_index" not in spec:
            why_bad.append(f"{spec.get('field')}：声明里没有 `not_in_index` 这个键"
                           f"（引擎的 `LeafSpec::not_in_index` 被删了？）")
            continue
        why = spec["not_in_index"]
        if why is None:
            continue
        if not isinstance(why, str) or not why.strip():
            why_bad.append(f"{spec.get('field')}：`not_in_index` 必须有非空理由"
                           f"（空/空白 = 拿「排除」当逃生门）")
            continue
        absent[spec["field"]] = why
    declared_index = declared_all - set(absent)
    extra = sorted(ctrl_kinds - declared_index)   # 表里有、声明没有 = 第二套词
    missing = sorted(declared_index - ctrl_kinds)  # 声明有、表里没发 = 悄悄少一片叶
    ck.check(f"kind 词表对账：`control` 表实测 {len(ctrl_kinds)} 种 kind == 声明 "
             f"{len(declared_index)} 片（leaves∪actions 减 {len(absent)} 条自报例外），逐字双向相等",
             not extra and not missing and not why_bad
             and bool(ctrl_kinds) and bool(declared_index) and ctrl_rows > 0
             and len(declared_index) >= 14 and len(absent) >= 1,
             "；".join(
                 ([f"表里有而声明没有（第二套词）：{extra}"] if extra else [])
                 + ([f"声明有而表里没发（悄悄少一片叶）：{missing}"] if missing else [])
                 + why_bad[:3]
             ) or (f"{ctrl_rows} 行、{len(ctrl_kinds)} 种 kind："
                   f"{' / '.join(sorted(ctrl_kinds))}；"
                   f"自报例外 {len(absent)} 条（{'、'.join(f'{k}（{v[:24]}…）' for k, v in absent.items())}）"))


if __name__ == "__main__":
    sys.exit(group_main("g4_spec", run))
