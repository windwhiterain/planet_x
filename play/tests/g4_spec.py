"""快组（T0/T1）：`web/static/views.json` 的**声明纪律** —— 静态 + 写面对账 + 读面对账 + 认领完整性。

搬到这里的是 `web/src/views_tests.rs`（488 行 Rust 检查，已删除，`web/src/lib.rs` 原处留了一行
指针）。为什么搬：那份声明是**数据**，写错了不会编译报错——只会让某个视图静静地少一列、或者
让新加的控制叶在界面上**凭空消失**。而「对真实世界跑一遍」这件事在 Python 更划算：改断言不用
重编 8 个测试二进制（见 `.agents/notes/test-decoupled-suite.md`）。

判据（方案见 `.agents/notes/web-control-spec.md` §6）：

1. **静态纪律**（从 Rust 搬来）：id 唯一 / 引用完整（`use` / `use_at` / `map_ref` / `label_from`
   / `card` 指的都在）/ `omit` 不与列重叠 / 每条路径表达式合文法 / `@根` 都在已知根里；
2. **写面对账**（**双向**集合相等）：`--control-schema` 的 `leaves[].field` ∪ `actions[].field`
   ∪ `{faction_id}` **==** `schemars` 里 `FactionControlPatch.properties` 的键集合
   （加一个字段却不写声明 = 红；声明了一个不存在的叶 = 红）；
3. **读面对账**（**跑真世界**，不拿声明自证）：起一局（seed 42 / 40 回合），用 `--apply` 把
   **每一片叶都写一次**（14 片，每片带一个哨兵值），再 `--control` 读回来，逐片断言：
   * 条目字段集 **⊇** `keys ∪ values ∪ carries ∪ read_only ∪ {mode}`，且 **⊆** 那个集合 ∪ `{remove}`；
   * `keys` 空 ⇔ 该种类在 `FactionControlView` 里是**对象**；非空 ⇔ 是**数组**且每个条目带齐身份键；
   * **防空转**：这一局真的写进去了 14 片叶（每片按哨兵值验过，不是「表是空的所以通过」）。
4. **认领完整性**（铁律 R 的写面对偶）：每个 `leaves[].field` / `actions[].field` 要么被某条
   `leaf` / `action` 行认领，要么在 `write_omit` 里有一条**带非空 why** 的记录；
   反过来，`leaf` / `action` / `owner` 行与 `leaf_ui` / `action_ui` 的键**不许有孤儿**。

⚠ 实测（本轮 seed 42 / 40 回合）：读面条目**一个 `remove` 都没有**——`capital` 的读面是
`Control<BodyId>`（`{value, mode}`），而 `default_*` 是 `{…, mode, remove: false}` 且
`remove` 带 `skip_serializing_if = "is_false"` ⇒ 读面永远不发这个字段。所以上头的 `∪ {remove}`
是**允许集**（宽容那一侧），不是「应该有」；谁要在读面上真的看到它，本组的 detail 会报出来。
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _harness import REPO, group_main  # noqa: E402

VIEWS_JSON = REPO / "web" / "static" / "views.json"

# 读面的根：与 `/api/state` 的 `info` 五个根 + 写面的两个读模板同批（`_frame` 侧同口径）。
ROOTS = ("state", "pre", "post", "config", "session", "control", "scope")
LAYOUTS = ("table", "sheet", "cards", "timeline", "pairs")
# 短局：40 回合就够长出舰/城/建筑/设计图，整组几秒跑完（不进长局、不读 170 MB 投影）。
SEED, ROUNDS = 42, 40


# --- 路径表达式（与 specview.js / 原 Rust 版同一套文法；这里只解析，不求值）---------
def split_segments(expr: str) -> list[str]:
    """按**不在方括号里**的 `.` 切段（`@control[?faction_id=x].a` → 2 段）。"""
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


# --- 一局要写的叶：每片一条 patch + 一条「读回来怎么认它」的哨兵 -------------------
# 值都取互不相同的哨兵：写进去之后**从读面上按哨兵认领回来**，这样「这一局真的写进去了
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

        if f == "capital":
            cap = fac.get("capital")
            body = (cap or {}).get("value") if isinstance(cap, dict) else None
            if not isinstance(body, str):
                missing.append("capital：这一局的读面里没有首都天体可写")
                continue
            add(f, {}, {"value": body, "mode": "Player"}, {"value": body, "mode": "Player"})

        elif f.startswith("default_"):
            if f == "default_doctrine":
                add(f, {}, {"temper": D_TEMPER, "lone_wolf": D_LONE, "mode": "Player"},
                    {"temper": D_TEMPER, "lone_wolf": D_LONE})
            elif f == "default_kiting":
                add(f, {}, {"kiting": D_KITING, "mode": "Player"}, {"kiting": D_KITING})
            elif f == "default_role":
                add(f, {}, {"role": D_ROLE, "mode": "Player"}, {"role": D_ROLE})
            else:
                missing.append(f"{f}：本组不认识这片势力级单叶（新加的叶要在这里写上怎么写）")

        elif f in ("ship_orders", "ship_doctrine", "ship_kiting", "ship_role"):
            k = first(spec) if keys else None
            if not k:
                missing.append(f"{f}：这一局该势力一条 {keys} 条目都没有，没法写")
                continue
            ship = k["ship"]
            if f == "ship_orders":
                add(f, k, {"ship": ship, "behavior": S_ORDER, "mode": "Player"},
                    {"behavior": S_ORDER})
            elif f == "ship_doctrine":
                add(f, k, {"ship": ship, "temper": S_TEMPER, "lone_wolf": S_LONE, "mode": "Player"},
                    {"temper": S_TEMPER, "lone_wolf": S_LONE})
            elif f == "ship_kiting":
                add(f, k, {"ship": ship, "kiting": S_KITING, "mode": "Player"}, {"kiting": S_KITING})
            else:
                add(f, k, {"ship": ship, "role": S_ROLE, "mode": "Player"}, {"role": S_ROLE})

        elif f in ("investment_budget", "construction_budget", "welfare_budget"):
            k = first(spec)
            if not k:
                missing.append(f"{f}：这一局该势力一个资源 key 都没有，没法写")
                continue
            if f == "investment_budget":
                v = V_INVEST
            elif f == "construction_budget":
                v = V_CONSTRUCT
            else:
                v = V_INVEST
            add(f, k, {**k, "value": v, "mode": "Player"}, {"value": v})

        elif f in ("invest_weights", "build_weights"):
            k = first(spec)
            if not k:
                missing.append(f"{f}：这一局该势力一片建筑权重都没有，没法写")
                continue
            v = W_INVEST if f == "invest_weights" else W_BUILD
            add(f, k, {**k, "value": v, "mode": "Player"}, {"value": v})

        elif f in ("loyalty_budget", "development_money", "construction_money"):
            # 这一局一开始一片都没有（省/市级 0 条）⇒ 必须**新建**一片：城从该势力自己的
            # 建筑权重里借一个（那些城一定属于它，写权重时引擎刚认过）。
            src = first(by_field["invest_weights"]) if "invest_weights" in by_field else None
            city = src.get("city") if src else None
            if not city:
                cands = [e.get("city") for e in (fac.get(f) or [])
                         if isinstance(e, dict) and e.get("city")]
                city = cands[0] if cands else None
            if not city:
                missing.append(f"{f}：这一局拿不到该势力的城名，没法写")
                continue
            add(f, {"city": city}, {"city": city, "value": V_LOYALTY, "mode": "Player"},
                {"value": V_LOYALTY})

        elif f == "blueprints":
            k = first(spec)
            if not k:
                missing.append(f"{f}：这一局该势力一张设计图都没有，没法写")
                continue
            # 只写**倾向三轴**（+ mode）：`class` / `components` 要过 `blueprint_class_mismatch`
            # 那道守卫（图与建造区的舰级必须一致），而我们只需要把这片叶**写一次**——
            # 那两个值字段在不在读面条目上由第 3 条判据（读面对账）负责，不靠这条写。
            add(f, k, {"name": k["name"],
                       "doctrine": {"temper": B_TEMPER, "lone_wolf": B_LONE},
                       "kiting": B_KITING, "role": B_ROLE, "mode": "Player"},
                {"doctrine": {"temper": B_TEMPER, "lone_wolf": B_LONE},
                 "kiting": B_KITING, "role": B_ROLE})

        else:
            missing.append(f"{f}：本组不认识这片叶（新加的叶要么在这里写上怎么写，要么进 write_omit）")

    return plans, missing


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

    # ══ 2. 写面对账：leaves ∪ actions ∪ {faction_id} == FactionControlPatch.properties ══
    schema = json.loads(h.capture(["--control-schema"]))
    leaves = schema.get("leaves") or []
    actions = schema.get("actions") or []
    owner_field = schema.get("owner_field")
    remove_field = schema.get("remove_field")
    props = (((schema.get("definitions") or {}).get("FactionControlPatch") or {})
             .get("properties") or {})
    declared = {s["field"] for s in leaves} | {a["field"] for a in actions} | {"faction_id"}
    leaf_dups = sorted({s["field"] for s in leaves if [x["field"] for x in leaves].count(s["field"]) > 1})
    missing_decl = sorted(set(props) - declared)   # schemars 有、声明没有 = 加字段忘写声明
    extra_decl = sorted(declared - set(props))     # 声明有、schemars 没有 = 写了一个不存在的叶
    ck.check(f"写面对账：leaves({len(leaves)}) ∪ actions({len(actions)}) ∪ {{faction_id}} "
             f"== FactionControlPatch.properties({len(props)})，双向相等且 field 不重复",
             not missing_decl and not extra_decl and not leaf_dups and bool(props) and bool(leaves),
             "；".join(
                 ([f"引擎有而声明没写：{missing_decl}（加字段忘了写声明）"] if missing_decl else [])
                 + ([f"声明了引擎没有的叶：{extra_decl}"] if extra_decl else [])
                 + ([f"leaves[].field 重复：{leaf_dups}"] if leaf_dups else [])
             ) or (f"{len(declared)} 个键两边一模一样（{len(leaves)} 叶 + {len(actions)} 命令 + faction_id）；"
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
    fid = fac["faction_id"]
    plans, unwritable = _plan_leaves(leaves, fac)
    ck.check(f"读面对账：选中的势力「{fid}」能给出全部 {len(leaves)} 片叶的写法（每片都要一个真实身份键）",
             not unwritable and len(plans) == len(leaves),
             "；".join(unwritable[:4]) or f"{len(plans)} 片叶各有一条写请求（seed {SEED} / {ROUNDS} 回合）")

    # 多键叶要写成**数组**（一条一条），单值叶写成对象——diff 的形状由 keys 决定。
    body: dict = {"faction_id": fid}
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
    post_fac = next((c for c in post["control"] if c.get("faction_id") == fid), None)
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
                    remove_seen.append(f"{c.get('faction_id')}.{spec['field']}")
                    if _keys_of(spec):
                        extra = extra + [f"{remove_field}（列表叶上不该有）"]
                if miss or extra:
                    field_bad.append(
                        f"{c.get('faction_id')}.{spec['field']}：缺 {miss}、多 {extra}；"
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
                    shape_bad2.append(f"{c.get('faction_id')}.{spec['field']}：keys 为空（势力级单叶）"
                                      f"但读面给的是**数组**")
                continue
            if v is None:
                continue
            if not isinstance(v, list):
                shape_bad2.append(f"{c.get('faction_id')}.{spec['field']}：keys={_keys_of(spec)} "
                                  f"但读面给的是 {type(v).__name__}，不是数组")
                continue
            for e in v:
                n_list_entries += 1
                miss = [k for k in _keys_of(spec) if not isinstance(e, dict) or k not in e]
                if miss:
                    shape_bad2.append(f"{c.get('faction_id')}.{spec['field']}：条目缺身份键 {miss}——"
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

    # `new: true`（身份键由人现填）只对**多键叶**成立；写在势力级单叶上是声明写错。
    new_rows_total = sum(1 for v in views for c in (v.get("columns") or []) if c.get("new"))
    ck.check(f"认领完整性：{new_rows_total} 条 `new: true` 行都挂在多键叶上"
             f"（单叶每势力一片，「现造一个身份键」对它不成立）",
             not new_rows,
             "；".join(new_rows[:3]) or f"本帧 {new_rows_total} 条，全部合法（防空转）")


if __name__ == "__main__":
    sys.exit(group_main("g4_spec", run))
