"""`g4_spec` 的**反向验证**：把每一族判据各注入一个错，看它是不是真的红。

为什么要有这一份：本仓反复拉黑「失败看起来像成功」，而**一条不会红的守卫**正是它的
温和版本——它看起来在守纪律，其实什么也没守。所以纪律本身也要有量具：
这里把 `views.json` 与 `--control-schema` 的**副本**逐个改坏喂给 `g4_spec.run`，
要求「该红的那条红、基线全绿」。**不碰工作树里的任何真文件。**

跑法（在 worktree 根目录）：

```bash
uv run --project play/planet_xq python play/tests/_g4_negative.py
```

2026-10 实测：**40 个注入错全部咬住**（每一个都红在该红的那条判据上），基线全绿。
其中 ⑳㉑㉒ 是第 7 条（`control` 表的 `kind` 词表 == 声明里的叶名）的量具：⑳在**真文件**上
把一片叶的 `kind` 改成一个声明里没有的词（走 `g4_spec.INDEX_HOOK`），㉑把一片真在表里的叶
标成"不在表里"，㉒把例外的理由改成空白——三条都要求**第 7 条自己**红（不是"碰巧别处红了"）。
㉓㉔ 是第 5b 条（文档对账）的量具：㉓在发射端把一条 `**加粗**` 开头的 `description` 剥掉一个
`*`（**复刻 schemars 0.8.22 那段 hack 的效果**，就是 46 条弹窗坏 markdown 的成因），
㉔把一条字段的 `description` 悄悄删掉——两条都要求「文档对账」那一族自己红。
㉕㉖ 是 §5d 条（名词覆盖率·**静态**）的量具：㉕剪掉 `jsonview.js` 的 tip 挂载（= 实机症状
「原始 JSON 视图里的字段名 hover 无反应」），㉖把宿主的 `Tip.attach` 改名（= 换翻译层时
最容易漏的那种断线：`ctx.tip` 转发到空处）——两条都要求 §5d 自己红。
㉗㉘ 是 §5 的**控制行**那一支（2026-10 补）的量具：㉗把某个 `owner` 行的 `noun` 改成语料里
不存在的词，㉘把某个 `owner` 行的 `noun` **删掉**（前端兜底也解析不出来——它只会去查显示标签
「归谁」）——两条都要求「名词覆盖率·控制行」**自己**红，因为那个缺口正是从这里漏出来的
（判据按作用域键判绿，而前端不查那个词，实机 hover 无反应）。
㉙㉚㉛ 是 §8 条（**追加机制**：没被声明的字段 = 普通列/普通行，2026-10 第 8 步）的量具
——用户裁决 *「我不希望有『其余』这样的栏目」*：
㉙ 把 `specview.js` 的 `residualCols` 剪成 `return []`（= **追加路径坏了**）⇒「读面覆盖」
   判据**自己**红（差集就是那些被吞掉的字段：「situation：没被任何一列覆盖 [cities, decisions…]」）；
㉚ 把追加列的类名改回 `sv-th-res`、并把「其余」塞回代码里（= **折叠桶回来了**）⇒
   「没有折叠桶」那条静态判据自己红；
㉛ 把 `faction-table` 的 `资源` 那一列删掉（= **未声明**）⇒ 覆盖判据**不红**（这是对的：
   未声明 = 自动追加，用户要的正是这个），但它必须**在判据文本里指出**这一列现在只能靠追加
   （声明 3→2、追加 7→8，且 `资源` 落在追加串里**引擎字段序**的位置上）。㉛ 走 `expect="evidence"`
   ——它验的是「判据把这件事**说出来了**」，不是"恰好没红"。
㉜㉝ 是 §9 条（**页级兜底桶**，2026-10 第 9 步）的量具——同一个用户裁决的**另一层**：
第 8 步删的是列/行级的桶，第 9 步删的是页级的（左栏那张自动生成的「未组织」页）。
㉜ 把那张页塞回**声明**（`views.json` 的 `pages`）；㉝ 把页塞回**前端代码**（复刻第 9 步以前
`sidePages()` 里那段 `.concat([{id:'leftover', …}])`）——两条都要求「没有页级兜底桶」
判据**自己**红（页的两个来源各堵一面，只堵一面的守卫是假守卫）。㉚ 顺带也把 §9 夹带的
「追加接线仍在」那一支点亮（它把 `sv-th-auto` 改回 `sv-th-res` = 断了一根接线）。
⚠ ㉕㉖㉙㉚㉝ 注入的是 **`web/static/*.js` 的 tempfile 拷贝**（`g4_spec.STATIC_JS` 指过去），
跑完还原——**真文件一个字节都不碰**（与上面那些 views.json 的注入同一纪律）。
㉞㉟ 是 §8e 条（**读面自检·数据级**，2026-10 第 10 步：声明过的读列在真世界里必须至少取到过
一次非空值——接手第 9 步随「未组织」页删掉的**运行时**自检 `renderSpecCheck`）的量具：
㉞ 把一条**裸字段列**的 `path` 改坏（`trades.moved` → `moved_typo`）；㉟ 把一条**表达式列**
的 `path` 改坏（`@post.power_share.${势力}` 后面接一个不存在的子字段）——后者**只有 §8e
看得见**（表达式列不进 §8a 的 `claimed` 口径），是「这一条不是恒绿」的最强证据。
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
LAST_ROWS: list[tuple[bool, str, str]] = []   # 最近一次跑的全部判据（`evidence` 模式要读文本）


class FakeH:
    """真 harness + 两个钩子：篡改 `--control-schema` 与 `--nouns`（各只影响自己那一族判据）。"""

    def __init__(self, real, mutate=None, mutate_nouns=None):
        self.real, self.mutate, self.mutate_nouns = real, mutate, mutate_nouns
        self.path = real.path

    def projection(self, *args, **kwargs):
        """§8d 的投影对账要读 `--index` 的 `schema.json`（真跑，不走 Fake 的注入）。"""
        return self.real.projection(*args, **kwargs)

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
    """喂一份被改坏的声明给 `g4_spec.run`，返回红的判据名。

    `expect`：
      * `"red"`      —— 这一跑必须红（算一个注入错）；
      * `"green"`    —— 基线：必须全绿；
      * `"evidence"` —— **证据案**（不算注入错）：必须**不红**，但判据文本里必须出现
        某些字串（`EVIDENCE`）：用来钉「判据真的把这件事**说出来**了」，而不只是"没红"。
    """
    global CASES, LAST_ROWS
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
    LAST_ROWS = list(ck.rows)
    bad = [n for ok, n, _ in ck.rows if not ok]
    if expect == "red":
        if not bad:
            MISBEHAVED.append(f"{label}：全绿（**没咬住**）")
            verdict = "**全绿（没咬住！）**"
        else:
            verdict = f"红 {len(bad)} 条 ✓"
    elif expect == "evidence":
        want = EVIDENCE.get(label, ())
        text = " ".join(d for _, _, d in ck.rows)
        miss = [w for w in want if w not in text]
        if bad:
            MISBEHAVED.append(f"{label}：期望不红（未声明 = 自动追加），实际红了 {bad}")
            verdict = f"红 {len(bad)} 条（**期望不红！**）"
        elif miss:
            MISBEHAVED.append(f"{label}：判据文本里没说出这件事：{miss}")
            verdict = f"文本里没写 {miss}（**没指出**）"
        else:
            verdict = "不红，且判据文本里指出了 ✓"
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


# `evidence` 模式的期望文本：判据**必须自己把这件事说出来**（不是"恰好没红"）。
EVIDENCE: dict[str, tuple[str, ...]] = {
    # ㉛：把 `资源` 这一列从 `faction-table` 里删掉（= 未声明）之后，读面覆盖判据仍然绿
    # （这是对的：未声明 = 自动追加），但它必须**指出**：那一列现在只能靠追加 ——
    # 证据就是同一张表的计数从「声明3+追加7」变成「声明2+追加8」，且 `资源` 落在追加串里
    # **引擎字段序**的位置上（记录里 `资源` 在 `名声` 与 `关系` 之间）。
    "undeclare-column": ("faction-table 声明2+追加8+不看3=13", "追加 好战度、思潮、名声、资源、关系"),
}


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

    # ㉗㉘ 名词覆盖率·**控制行**（§5 的控制面那一支，2026-10 补）：量具自己也要有量具。
    #     那一支是为这个缺口补的——`owner` 行从前按「作用域键 `factions` 也在语料里」判绿，
    #     而前端 `app.js::nounTip` **根本不拿 `owner` 去查**（field 链是 `noun` → 叶字段名 →
    #     裸字段名）⇒ 实机 hover 无反应、判据却全绿（判据与实际解析路径不一致）。
    #     现在口径统一到前端（**声明优先**）：
    #     ㉗ 把某个 owner 行的 `noun` 改成语料里不存在的词（弹空框）；
    #     ㉘ 把某个 owner 行的 `noun` **删掉** —— 前端兜底也解析不出来（它只会去查显示标签
    #        「归谁」，语料里没有）。两条都要求「名词覆盖率·控制行」那一族**自己**红
    #        （不是碰巧别处红了）。
    CTL_CHECK = "名词覆盖率·控制行"

    d = clone()
    hit = 0
    for _, c in walk_columns(d):
        if isinstance(c, dict) and isinstance(c.get("owner"), str) and c.get("noun"):
            c["noun"] = "语料里没有这个名词"
            hit += 1
            break
    assert hit == 1, "views.json 里没有带 `noun` 的 owner 行：判据/声明换了口径？"
    bad = run_case("owner-noun-bogus", d)
    if not any(n.startswith(CTL_CHECK) for n in bad):
        MISBEHAVED.append(f"owner-noun-bogus：控制行判据没红（实际红：{bad}）")

    d = clone()
    hit = 0
    for _, c in walk_columns(d):
        if isinstance(c, dict) and isinstance(c.get("owner"), str) and c.get("noun"):
            del c["noun"]        # ← 只留显示标签「归谁」：前端兜底查不到任何词
            hit += 1
            break
    assert hit == 1, "views.json 里没有带 `noun` 的 owner 行：判据/声明换了口径？"
    bad = run_case("owner-noun-dropped", d)
    if not any(n.startswith(CTL_CHECK) for n in bad):
        MISBEHAVED.append(f"owner-noun-dropped：控制行判据没红（实际红：{bad}）")

    # ㉙㉚㉛ 追加机制（第 8 步）的量具：用户裁决 *「我不希望有『其余』这样的栏目」*
    #     *「你就不能直接把没组织的并在后面吗，你把它藏起来我看都看不见」* ——
    #     ①「没被声明的字段 = 普通列」这条机制自己必须**会红**（否则它就是一条永远绿的守卫）；
    #     ② 折叠桶（「其余」那套）**回来**了也必须红。
    #     ⚠ 注入的是 `web/static/*.js` 的 tempfile 拷贝（`g4_spec.STATIC_JS` 指过去），
    #     真文件一个字节都不碰（与 ㉕㉖ 同一纪律）。
    APPEND_COVER = "张读面表/卡片"
    APPEND_BUCKET = "代码里没有折叠桶"
    # 第 9 步那条（页级兜底桶）——㉚ 也会把它夹带的「追加接线仍在」那一支点亮，一并断言。
    PAGE_BUCKET_CHECK = "第 9 步·没有页级兜底桶"
    static8 = T / "static-append-inject"
    if static8.exists():
        shutil.rmtree(static8)
    shutil.copytree(static_real, static8)
    g4_spec.STATIC_JS = static8
    try:
        sv = static8 / "specview.js"
        orig_sv = sv.read_text(encoding="utf-8")

        # ㉙ **追加路径坏了**（残差算出来但一列都不摆）：读面覆盖判据必须红
        #    ——「声明 ∪ 追加 == 全部引擎字段」的差集就是那些被吞掉的字段。
        head = "  function residualCols(rows, spec) {\n"
        assert head in orig_sv, "specview.js 里没有 `residualCols`：判据/代码换了口径？"
        sv.write_text(orig_sv.replace(head, head + "    return [];   // ← 注入：追加路径坏了\n"),
                      encoding="utf-8")
        bad = run_case("append-path-broken")
        if not any(APPEND_COVER in n for n in bad):
            MISBEHAVED.append(f"append-path-broken：追加完整性没红（实际红：{bad}）")
        sv.write_text(orig_sv, encoding="utf-8")

        # ㉚ **折叠桶回来了**（把追加列的类名改回 `sv-th-res`，并把「其余」塞回代码里）：
        #    「没有折叠桶」那条静态判据必须红（用户要的正是"别再藏起来"）。
        assert "'sv-th sv-th-auto'" in orig_sv, "specview.js 里没有 `'sv-th sv-th-auto'`：换了口径？"
        sv.write_text(orig_sv.replace("'sv-th sv-th-auto'", "'sv-th sv-th-res'")
                              .replace("function autoTh(key) {",
                                       "function autoTh(key) {\n    const other = '其余';   // ← 注入：桶回来了\n"),
                      encoding="utf-8")
        bad = run_case("bucket-back")
        if not any(APPEND_BUCKET in n for n in bad):
            MISBEHAVED.append(f"bucket-back：没有折叠桶判据没红（实际红：{bad}）")
        # 这一注入同时把 `sv-th-auto` 改成了 `sv-th-res` = **追加接线断了一根** ⇒ 第 9 步那条
        # 判据夹带的「不许把上一步的机制一起删掉」那一支也必须跟着红（否则那一支是恒绿的）。
        if not any(PAGE_BUCKET_CHECK in n for n in bad):
            MISBEHAVED.append(f"bucket-back：§9 的「追加接线仍在」那一支没红（实际红：{bad}）")
        sv.write_text(orig_sv, encoding="utf-8")
    finally:
        g4_spec.STATIC_JS = static_real

    # ㉛ **未声明一列**（把 `faction-table` 里 `资源` 那一列删掉——它原本是「首都库存」）。
    #     期望：**不红**——「没被声明」不是错误，它就该自动追加成普通列（用户要的正是这个）；
    #     但判据文本必须**指出**这一列现在只能靠追加：声明 3→2、追加 7→8，而且它落在追加串里
    #     **引擎字段序**的位置上（`资源` 在记录里排在 `名声` 与 `关系` 之间，不是排在末尾）。
    #     这一条不是注入错，是**口径的量具**：它证明「整理过 / 靠兜底」在输出里分得开。
    d = clone()
    hit = 0
    for v in walk_views(d):
        if v.get("id") != "faction-table":
            continue
        before = list(v.get("columns") or [])
        v["columns"] = [c for c in before if not (isinstance(c, dict) and c.get("path") == "资源")]
        hit += len(before) - len(v["columns"])
    assert hit == 1, "faction-table 里没有 `资源` 那一列：判据/声明换了口径？"
    run_case("undeclare-column", d, expect="evidence")

    # ㉜㉝ **页级兜底桶**（第 9 步，2026-10）的量具：用户裁决 *「我不希望有『其余』这样的栏目」*
    #     —— 第 8 步删的是**列级 / 行级**的桶（㉙㉚㉛ 盯着），第 9 步删的是**页级**的同一个概念：
    #     左栏那张自动生成的「未组织」页（`id: 'leftover'`）。它不是一张声明出来的页，而是
    #     `app.js::sidePages()` 里 `.concat([{id:'leftover', …}])` 现拼的 —— 所以**两个面**
    #     都要有量具（只堵一面的话，另一面怎么写都绿）：
    #     ㉜ 把那张页塞回**声明**（`views.json` 的 `pages`）；
    #     ㉝ 把页塞回**前端代码**（复刻第 9 步以前 `sidePages()` 的那两行）。
    #     两条都要求「没有页级兜底桶」这条判据**自己**红（不是碰巧别处红了）。
    #     ⚠ ㉝ 注入的是 `web/static/*.js` 的 tempfile 拷贝（`g4_spec.STATIC_JS` 指过去，
    #     与 ㉕㉖㉙㉚ 同一纪律），真文件一个字节都不碰。
    d = clone()
    d["pages"].append({"id": "leftover", "title": "未组织", "views": []})
    bad = run_case("leftover-page-back-in-decl", d)
    if not any(PAGE_BUCKET_CHECK in n for n in bad):
        MISBEHAVED.append(f"leftover-page-back-in-decl：§9 没红（实际红：{bad}）")

    static9 = T / "static-page-inject"
    if static9.exists():
        shutil.rmtree(static9)
    shutil.copytree(static_real, static9)
    g4_spec.STATIC_JS = static9
    try:
        app9 = static9 / "app.js"
        orig_app9 = app9.read_text(encoding="utf-8")
        anchor = "function sidePages() {\n  return viewsDoc.pages || [];\n}"
        assert anchor in orig_app9, "app.js 的 `sidePages()` 换了形状：判据/代码换了口径？"
        app9.write_text(orig_app9.replace(
            anchor,
            "function sidePages() {\n"
            "  return (viewsDoc.pages || []).concat([\n"
            "    { id: 'leftover', title: '未组织', hint: '← 注入：页级兜底桶回来了' },\n"
            "  ]);\n"
            "}"), encoding="utf-8")
        bad = run_case("leftover-page-back-in-code")
        if not any(PAGE_BUCKET_CHECK in n for n in bad):
            MISBEHAVED.append(f"leftover-page-back-in-code：§9 没红（实际红：{bad}）")
        app9.write_text(orig_app9, encoding="utf-8")
    finally:
        g4_spec.STATIC_JS = static_real

    # ㉞㉟ **读面自检（数据级）**（§8e，2026-10 第 10 步）的量具。这一条接手第 9 步随
    #     「未组织」页删掉的**运行时**自检 `renderSpecCheck`：声明过的读列在真世界里必须
    #     至少取到过一次非空值。它的失败样子最安静——引擎改了字段名 / 声明写错一个词，
    #     界面不报错，只是整列印一排「·」，所以这条守卫自己必须会红：
    #     ㉞ 把 `trades` 表一条**裸字段列**的 `path` 改坏（`moved` → `moved_typo`）——
    #        那一列在全表里一格非空都没有 ⇒ §8e 红（同时 §8a 会因"声明了记录上没有的字段"红，
    #        这是对的：两条从不同角度看同一件事）；
    #     ㉟ 把 `faction-table` 一条**表达式列**的 `path` 改坏（在真实路径后面接一个不存在的
    #        子字段）——表达式列**不进** §8a 的 `claimed` 口径（`claimedKeys` 跳过 `@` 开头的
    #        路径），所以这一条**只有 §8e 看得见**：它是"这一条不是恒绿"的最强证据。
    COL_CHECK = "读面自检（数据级）"

    d = clone()
    hit = 0
    for _, c in walk_columns(d):
        if isinstance(c, dict) and c.get("path") == "moved":
            c["path"] = "moved_typo"
            hit += 1
    assert hit == 1, "views.json 里没有 `moved` 那一列：判据/声明换了口径？"
    bad = run_case("declared-column-path-broken", d)
    if not any(COL_CHECK in n for n in bad):
        MISBEHAVED.append(f"declared-column-path-broken：§8e 没红（实际红：{bad}）")

    d = clone()
    hit = 0
    for _, c in walk_columns(d):
        if isinstance(c, dict) and c.get("path") == "@post.power_share.${势力}":
            c["path"] = "@post.power_share.${势力}.并不存在的子字段"
            hit += 1
            break
    assert hit == 1, "views.json 里没有 `@post.power_share.${势力}` 那一列：判据/声明换了口径？"
    bad = run_case("declared-expr-path-broken", d)
    if not any(COL_CHECK in n for n in bad):
        MISBEHAVED.append(f"declared-expr-path-broken：§8e 没红（实际红：{bad}）")

    if MISBEHAVED:
        print("\n**反向验证失败**（说明上面这些判据里有不会红的）：")
        for m in MISBEHAVED:
            print("  -", m)
        return 1
    print(f"\n反向验证通过：基线与每个注入错都按期望红/绿（{CASES} 个注入错全部咬住）。")
    return 0


if __name__ == "__main__":
    sys.exit(main())
