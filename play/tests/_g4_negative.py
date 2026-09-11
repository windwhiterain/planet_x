"""`g4_spec` 的**反向验证**：把每一族判据各注入一个错，看它是不是真的红。

为什么要有这一份：本仓反复拉黑「失败看起来像成功」，而**一条不会红的守卫**正是它的
温和版本——它看起来在守纪律，其实什么也没守。所以纪律本身也要有量具：
这里把 `views.json` 与 `--control-schema` 的**副本**逐个改坏喂给 `g4_spec.run`，
要求「该红的那条红、基线全绿」。**不碰工作树里的任何真文件。**

跑法（在 worktree 根目录）：

```bash
uv run --project play/planet_xq python play/tests/_g4_negative.py
```

2026-10 实测：**32 个注入错全部咬住**（每一个都红在该红的那条判据上），基线全绿。
其中 ⑳㉑㉒ 是第 7 条（`control` 表的 `kind` 词表 == 声明里的叶名）的量具：⑳在**真文件**上
把一片叶的 `kind` 改成一个声明里没有的词（走 `g4_spec.INDEX_HOOK`），㉑把一片真在表里的叶
标成"不在表里"，㉒把例外的理由改成空白——三条都要求**第 7 条自己**红（不是"碰巧别处红了"）。
㉓㉔ 是第 5b 条（文档对账）的量具：㉓在发射端把一条 `**加粗**` 开头的 `description` 剥掉一个
`*`（**复刻 schemars 0.8.22 那段 hack 的效果**，就是 46 条弹窗坏 markdown 的成因），
㉔把一条字段的 `description` 悄悄删掉——两条都要求「文档对账」那一族自己红。
㉕㉖ 是 §5d 条（名词覆盖率·**静态**）的量具：㉕剪掉 `jsonview.js` 的 tip 挂载（= 实机症状
「原始 JSON 视图里的字段名 hover 无反应」），㉖把宿主的 `Tip.attach` 改名（= 换翻译层时
最容易漏的那种断线：`ctx.tip` 转发到空处）——两条都要求 §5d 自己红。
⚠ ㉕㉖ 注入的是 **`web/static/*.js` 的 tempfile 拷贝**（`g4_spec.STATIC_JS` 指过去），
跑完还原——**真文件一个字节都不碰**（与上面那 30 条同一纪律）。
"""

import json
import shutil
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import g4_spec  # noqa: E402  （要它自己的 VIEWS_JSON / run）
from _harness import Checks, Harness  # noqa: E402

REPO = HERE.parents[1]
T = Path(tempfile.mkdtemp(prefix="g4-neg-"))
BASE = json.loads(g4_spec.VIEWS_JSON.read_text(encoding="utf-8"))

MISBEHAVED: list[str] = []
CASES = 0  # 注入错的数量（基线那一跑不算）


class FakeH:
    """真 harness + 两个钩子：篡改 `--control-schema` 与 `--nouns`（各只影响自己那一族判据）。"""

    def __init__(self, real, mutate=None, mutate_nouns=None):
        self.real, self.mutate, self.mutate_nouns = real, mutate, mutate_nouns
        self.path = real.path

    def capture(self, args):
        out = self.real.capture(args)
        if self.mutate and any("control-schema" in a for a in args):
            d = self.mutate(json.loads(out))
            return json.dumps(d, ensure_ascii=False)
        if self.mutate_nouns and any(a == "--nouns" for a in args):
            d = self.mutate_nouns(json.loads(out))
            return json.dumps(d, ensure_ascii=False)
        return out


def run_case(label, doc=None, mutate=None, expect="red", mutate_nouns=None):
    """喂一份被改坏的声明给 `g4_spec.run`，返回红的判据名。"""
    global CASES
    if expect == "red":
        CASES += 1
    if doc is not None:
        p = T / f"{label}.json"
        p.write_text(json.dumps(doc, ensure_ascii=False), encoding="utf-8")
        g4_spec.VIEWS_JSON = p
    else:
        g4_spec.VIEWS_JSON = REPO / "web" / "static" / "views.json"
    h = FakeH(Harness(kind="release"), mutate, mutate_nouns)
    ck = Checks("neg")
    g4_spec.run(h, ck)
    bad = [n for ok, n, _ in ck.rows if not ok]
    if expect == "red":
        if not bad:
            MISBEHAVED.append(f"{label}：全绿（**没咬住**）")
            verdict = "**全绿（没咬住！）**"
        else:
            verdict = f"红 {len(bad)} 条 ✓"
    else:
        if bad:
            MISBEHAVED.append(f"{label}：基线红了 {len(bad)} 条")
            verdict = f"红 {len(bad)} 条（**期望全绿！**）"
        else:
            verdict = "全绿 ✓"
    print(f"### {label}: {verdict}")
    for ok, n, d in ck.rows:
        if not ok:
            print("   FAIL", n[:80], "|", d[:220])
    return bad


def clone():
    return json.loads(json.dumps(BASE))


def walk_columns(doc):
    """所有视图的列（pages + select）。"""
    for page in doc.get("pages", []) + [{"views": doc.get("select", [])}]:
        for v in page.get("views", []):
            for c in v.get("columns") or []:
                yield v, c


def walk_views(doc):
    """所有视图（pages + select + inline）。"""
    for page in doc.get("pages", []) + [{"views": doc.get("select", [])}]:
        for v in page.get("views", []):
            yield v
    for v in doc.get("inline") or []:
        yield v


def main() -> int:
    # 基线：原样应该是全绿（否则这一份反向验证自己就不可信）
    run_case("baseline-out", expect="green")

    # ①–⑤ 静态纪律
    d = clone()
    d["pages"][0]["views"][1]["id"] = "story"
    run_case("dup-id", d)

    d = clone()
    d["inline"][0]["use_at"]["state.ships"] = "ship-tabel"
    run_case("bad-ref", d)

    d = clone()
    d["pages"][0]["views"][0]["source"] = "@state.chronicle[*"
    run_case("bad-path", d)

    d = clone()
    d["pages"][0]["views"][0]["source"] = "@world.chronicle[*]"
    run_case("bad-root", d)

    d = clone()
    d["pages"][0]["views"][1]["omit"].append({"path": "hegemon", "why": "试试"})
    run_case("omit-overlap", d)

    # ⑤b `source` 的三种形态（2026-10 新增）：键必须存在，值可以是字符串或**显式** null
    #     （null = 「这张卡故意不依赖记录」，例如全局归属那条）；漏写键仍然要红。
    d = clone()
    for v in walk_views(d):
        if v.get("id") == "global-scope":
            del v["source"]
    run_case("source-key-missing", d)

    d = clone()
    for v in walk_views(d):
        if v.get("source") is None and v.get("layout") != "table":
            v["layout"] = "table"  # 表没有来源没意义
    run_case("null-source-on-table", d)

    # ⑤d 名词覆盖率：把一个**裸字段列**的 path 改成引擎不认识的名词 ⇒ 要红
    #     （这条守的是"界面上当名词显示的每一列都弹得出解释"；改名/漏文档时它会先响）
    d = clone()
    for v in walk_views(d):
        for c in (v.get("columns") or []):
            if isinstance(c, dict) and isinstance(c.get("path"), str) and c["path"] == "hegemon":
                c["path"] = "这个名词引擎不认识"
    run_case("noun-without-doc", d)

    # ⑤e 同一条的另一面：把**控制行标签**改成语料里没有的名词也要红
    #     （控制行的字段名查得到就不算漏 —— 所以这里连字段一起换掉）
    d = clone()
    for v in walk_views(d):
        for c in (v.get("columns") or []):
            if isinstance(c, dict) and isinstance(c.get("owner"), str) and c.get("owner") == "global":
                c["owner"] = "不存在的键"
    run_case("owner-without-doc", d)

    # ⑤c `new: true` 只对多键叶成立：挂到势力级单叶（keys 为空）上要红
    d = clone()
    for v in walk_views(d):
        for c in v.get("columns") or []:
            if isinstance(c.get("leaf"), str) and c["leaf"].endswith(".舰队默认角色"):
                c["new"] = True
    run_case("new-on-single-leaf", d)

    # ⑤d 表达式列的 `noun` 写成语料里没有的名词 ⇒ 那一列会弹空框，必须红
    d = clone()
    for _, c in walk_columns(d):
        if isinstance(c.get("noun"), str):
            c["noun"] = "语料里没有这个名词"
            break
    run_case("bogus-noun", d)

    # ⑥ 认领完整性：把 `capital` 的全部 leaf 行删掉（它只在 sel-faction 里被认领）
    d = clone()
    for _, c in walk_columns(d):
        if isinstance(c.get("leaf"), str) and c["leaf"].endswith(".首都"):
            del c["leaf"]
            c["path"] = "resources"
    run_case("unclaimed-leaf", d)

    # ⑦ 认领完整性：leaf 行指向一片不存在的叶
    d = clone()
    for _, c in walk_columns(d):
        if isinstance(c.get("leaf"), str) and "舰队默认角色" in c["leaf"]:
            c["leaf"] = c["leaf"].replace("舰队默认角色", "舰队默认角色s")
    run_case("orphan-leaf-row", d)

    # ⑧ 认领完整性：leaf_ui 多了个孤儿键
    d = clone()
    d["leaf_ui"]["ship_ordeers"] = {"label": "打错的键", "editor": "behavior"}
    run_case("orphan-leaf-ui", d)

    # ⑨ 认领完整性：owner 行写了个不存在的作用域
    d = clone()
    for _, c in walk_columns(d):
        if c.get("owner"):
            c["owner"] = "planets"
            break
    run_case("bad-owner", d)

    # ⑩ 认领完整性：write_omit 少了 why
    d = clone()
    d["write_omit"][0]["why"] = "  "
    run_case("write-omit-no-why", d)

    # ⑪ 写面对账：声明少一片叶（= 加字段没写声明）
    def drop_leaf(s):
        s["leaves"] = [x for x in s["leaves"] if x["field"] != "城市福利预算"]
        return s

    run_case("decl-missing-leaf", None, drop_leaf)

    # ⑫ 写面对账：声明多一片不存在的叶
    def add_leaf(s):
        s["leaves"].append(
            {"field": "ghost_budget", "keys": [], "values": ["value"], "carries": [], "read_only": []}
        )
        return s

    run_case("decl-ghost-leaf", None, add_leaf)

    # ⑬ 读面对账：values 多了一个读面上没有的字段
    def bad_values(s):
        for x in s["leaves"]:
            if x["field"] == "首都":
                x["values"] = ["值", "ghost"]
        return s

    run_case("decl-ghost-value", None, bad_values)

    # ⑭ 读面对账：carries 漏了一个读面真有的字段（invest_weights 少了 structure）
    def bad_carries(s):
        for x in s["leaves"]:
            if x["field"] == "建设权重":
                x["carries"] = ["类型", "资源", "建造舰级"]
        return s

    run_case("decl-short-carries", None, bad_carries)

    # ⑮ 读面对账：把一片列表叶的 keys 清空
    def bad_keys(s):
        for x in s["leaves"]:
            if x["field"] == "指令":
                x["keys"] = []
        return s

    run_case("decl-keys-empty", None, bad_keys)

    # ⑯ 读面对账：把一片单叶的 keys 加一个
    def bad_keys2(s):
        for x in s["leaves"]:
            if x["field"] == "首都":
                x["keys"] = ["body"]
        return s

    run_case("decl-keys-nonempty", None, bad_keys2)

    # ⑰ 身份键：把某个结构体的身份字段改成一个**存在但不唯一**的字段（舰级：多艘舰同一级）
    def bad_identity_struct(d):
        d["identity"]["structs"]["Ship"] = "舰级"
        return d

    run_case("identity-struct-not-unique", None, None, mutate_nouns=bad_identity_struct)

    # ⑱ 身份键：把某张表的键改成一个**根本不存在的列**
    def bad_identity_table(d):
        d["identity"]["tables"]["ships"] = "根本没有这一列"
        return d

    run_case("identity-table-key-missing", None, None, mutate_nouns=bad_identity_table)

    # ⑲ 身份键：前端那边漂移 —— 视图声明一个**不是引擎身份字段**的 key
    d = clone()
    for v in walk_views(d):
        if v.get("id") == "ship-table":
            v["key"] = "舰级"
    run_case("identity-view-key-drift", d)

    # ⑳ kind 词表对账：把 `control` 表里某片叶的 `kind` 换成**声明里没有的词** ⇒ 必须红。
    #    注入点在真文件上（`g4_spec.INDEX_HOOK`）：判据读到的确实是那份被改坏的表。
    KIND_CHECK = "kind 词表对账"

    def rename_one_kind_in_index(path):
        lines = path.read_text(encoding="utf-8").splitlines()
        for i, line in enumerate(lines):
            if line.strip():
                row = json.loads(line)
                row["kind"] = "幽灵叶"          # ← 声明里没有这个词（也不在 actions 里）
                lines[i] = json.dumps(row, ensure_ascii=False)
                break
        path.write_text("\n".join(lines) + "\n", encoding="utf-8")

    g4_spec.INDEX_HOOK = rename_one_kind_in_index
    try:
        bad = run_case("kind-not-declared")
    finally:
        g4_spec.INDEX_HOOK = None
    if not any(n.startswith(KIND_CHECK) for n in bad):
        MISBEHAVED.append("kind-not-declared：红了，但**不是**第 7 条 kind 对账红的那一条"
                          f"（实际红：{bad}）")

    # ㉑ kind 词表对账（声明侧）：把一片**真的在表里**的叶标成 `not_in_index`（"它不在表里"）——
    #    声明与实测立刻对不上 ⇒ 第 7 条红（只有它红：别的判据不看这个键）。
    def mark_live_leaf_absent(s):
        for x in s["leaves"]:
            if x["field"] == "指令":
                x["not_in_index"] = "装作它不在 control 表里"
        return s

    bad = run_case("decl-excludes-live-leaf", None, mark_live_leaf_absent)
    if not any(n.startswith(KIND_CHECK) for n in bad):
        MISBEHAVED.append(f"decl-excludes-live-leaf：第 7 条没红（实际红：{bad}）")

    # ㉒ kind 词表对账（理由必须写出来）：把 `not_in_index` 的理由改成空白 ⇒ 逃生门 ⇒ 必须红
    def blank_exclusion_reason(s):
        for x in list(s["leaves"]) + list(s["actions"]):
            if x.get("not_in_index"):
                x["not_in_index"] = "   "
        return s

    bad = run_case("blank-exclusion-reason", None, blank_exclusion_reason)
    if not any(n.startswith(KIND_CHECK) for n in bad):
        MISBEHAVED.append(f"blank-exclusion-reason：第 7 条没红（实际红：{bad}）")

    # ㉓㉔ 文档对账（§5b）：量具自己也要有量具 —— 在**发射端**动一个字符 / 丢一段文档，
    #     要求「文档对账」那一族**自己**红（不是"碰巧别处红了"）。
    #     ㉓ 造的是**真实发生过的那个错**：把一条 `**加粗**` 开头的 description 剥掉一个 `*`
    #     ——schemars 0.8.22 的 `get_doc` 就是这么干的（46 条弹窗坏 markdown 的成因）。
    DOC_CHECK = "文档对账"

    def strip_one_bold_star(d):
        for sec in ("state", "view", "control"):
            for spec in ((d.get(sec) or {}).get("definitions") or {}).values():
                for f in (spec.get("properties") or {}).values():
                    desc = f.get("description")
                    if isinstance(desc, str) and desc.startswith("**"):
                        f["description"] = "*" + desc[2:]      # ← 0.8 那段 hack 的效果
                        return d
        raise AssertionError("语料里找不到以 `**` 开头的 description（判据换了口径？）")

    bad = run_case("doc-star-stripped", None, None, mutate_nouns=strip_one_bold_star)
    if not any(n.startswith(DOC_CHECK) for n in bad):
        MISBEHAVED.append(f"doc-star-stripped：文档对账没红（实际红：{bad}）")

    def drop_one_doc(d):
        for sec in ("state", "view", "control"):
            for spec in ((d.get(sec) or {}).get("definitions") or {}).values():
                for f in (spec.get("properties") or {}).values():
                    if isinstance(f.get("description"), str) and f["description"].strip():
                        del f["description"]                      # ← 文档被悄悄丢了
                        return d
        raise AssertionError("语料里找不到带 description 的字段（判据换了口径？）")

    bad = run_case("doc-silently-dropped", None, None, mutate_nouns=drop_one_doc)
    if not any(n.startswith(DOC_CHECK) for n in bad):
        MISBEHAVED.append(f"doc-silently-dropped：文档对账没红（实际红：{bad}）")

    # ㉕㉖ 名词覆盖率·**静态**（§5d）：量具自己也要有量具 —— 把「渲染字段名标签 ⇒ 挂 tip」
    #     这条接线从**拷贝**里剪断，要求 §5d 那一族**自己**红。
    #     ⚠ 同样只动 tempfile 里的拷贝（`g4_spec.STATIC_JS` 指向它），真文件一个字节不碰。
    #     ㉕ 剪掉 `jsonview.js` 的挂载（= 实机症状「原始 JSON 视图里的字段名 hover 无反应」）；
    #     ㉖ 把宿主里的 `Tip.attach` 改名（= 换翻译层时最容易漏的那种断线）。
    TIP_CHECK = "渲染字段名标签的 JS 模块都挂了 tip"
    static_real = g4_spec.STATIC_JS
    static_tmp = T / "static-inject"
    if static_tmp.exists():
        shutil.rmtree(static_tmp)
    shutil.copytree(static_real, static_tmp)
    g4_spec.STATIC_JS = static_tmp
    try:
        jv = static_tmp / "jsonview.js"
        orig_jv = jv.read_text(encoding="utf-8")
        assert "ctx.tip(node" in orig_jv, "jsonview.js 里没有 `ctx.tip(node`：判据/代码换了口径？"
        jv.write_text("\n".join(l for l in orig_jv.splitlines()
                                if "ctx.tip(" not in l) + "\n", encoding="utf-8")
        bad = run_case("tip-mount-removed")
        if not any(TIP_CHECK in n for n in bad):
            MISBEHAVED.append(f"tip-mount-removed：§5d 没红（实际红：{bad}）")
        jv.write_text(orig_jv, encoding="utf-8")

        app = static_tmp / "app.js"
        orig_app = app.read_text(encoding="utf-8")
        assert "Tip.attach(" in orig_app, "app.js 里没有 `Tip.attach(`：判据/代码换了口径？"
        app.write_text(orig_app.replace("Tip.attach(", "Tip.mountTip("), encoding="utf-8")
        bad = run_case("tip-attach-renamed")
        if not any(TIP_CHECK in n for n in bad):
            MISBEHAVED.append(f"tip-attach-renamed：§5d 没红（实际红：{bad}）")
        app.write_text(orig_app, encoding="utf-8")
    finally:
        g4_spec.STATIC_JS = static_real

    if MISBEHAVED:
        print("\n**反向验证失败**（说明上面这些判据里有不会红的）：")
        for m in MISBEHAVED:
            print("  -", m)
        return 1
    print(f"\n反向验证通过：基线与每个注入错都按期望红/绿（{CASES} 个注入错全部咬住）。")
    return 0


if __name__ == "__main__":
    sys.exit(main())
