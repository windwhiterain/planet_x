"""`g4_spec` 的**反向验证**：把每一族判据各注入一个错，看它是不是真的红。

为什么要有这一份：本仓反复拉黑「失败看起来像成功」，而**一条不会红的守卫**正是它的
温和版本——它看起来在守纪律，其实什么也没守。所以纪律本身也要有量具：
这里把 `views.json` 与 `--control-schema` 的**副本**逐个改坏喂给 `g4_spec.run`，
要求「该红的那条红、基线全绿」。**不碰工作树里的任何真文件。**

跑法（在 worktree 根目录）：

```bash
uv run --project play/planet_xq python play/tests/_g4_negative.py
```

2026-10 实测：**16 个注入错全部咬住**（每一个都红在该红的那条判据上），基线全绿。
"""

import json
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
    """真 harness + 一个可以篡改 `--control-schema` 的钩子（只影响那一族的判据）。"""

    def __init__(self, real, mutate=None):
        self.real, self.mutate = real, mutate
        self.path = real.path

    def capture(self, args):
        out = self.real.capture(args)
        if self.mutate and any("control-schema" in a for a in args):
            d = self.mutate(json.loads(out))
            return json.dumps(d, ensure_ascii=False)
        return out


def run_case(label, doc=None, mutate=None, expect="red"):
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
    h = FakeH(Harness(kind="release"), mutate)
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

    # ⑤c `new: true` 只对多键叶成立：挂到势力级单叶（keys 为空）上要红
    d = clone()
    for v in walk_views(d):
        for c in v.get("columns") or []:
            if isinstance(c.get("leaf"), str) and c["leaf"].endswith(".default_role"):
                c["new"] = True
    run_case("new-on-single-leaf", d)

    # ⑥ 认领完整性：把 `capital` 的全部 leaf 行删掉（它只在 sel-faction 里被认领）
    d = clone()
    for _, c in walk_columns(d):
        if isinstance(c.get("leaf"), str) and c["leaf"].endswith(".capital"):
            del c["leaf"]
            c["path"] = "resources"
    run_case("unclaimed-leaf", d)

    # ⑦ 认领完整性：leaf 行指向一片不存在的叶
    d = clone()
    for _, c in walk_columns(d):
        if isinstance(c.get("leaf"), str) and "default_role" in c["leaf"]:
            c["leaf"] = c["leaf"].replace("default_role", "default_roles")
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
        s["leaves"] = [x for x in s["leaves"] if x["field"] != "loyalty_budget"]
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
            if x["field"] == "capital":
                x["values"] = ["value", "ghost"]
        return s

    run_case("decl-ghost-value", None, bad_values)

    # ⑭ 读面对账：carries 漏了一个读面真有的字段（invest_weights 少了 structure）
    def bad_carries(s):
        for x in s["leaves"]:
            if x["field"] == "invest_weights":
                x["carries"] = ["kind", "resource", "ship_type"]
        return s

    run_case("decl-short-carries", None, bad_carries)

    # ⑮ 读面对账：把一片列表叶的 keys 清空
    def bad_keys(s):
        for x in s["leaves"]:
            if x["field"] == "ship_orders":
                x["keys"] = []
        return s

    run_case("decl-keys-empty", None, bad_keys)

    # ⑯ 读面对账：把一片单叶的 keys 加一个
    def bad_keys2(s):
        for x in s["leaves"]:
            if x["field"] == "capital":
                x["keys"] = ["body"]
        return s

    run_case("decl-keys-nonempty", None, bad_keys2)

    if MISBEHAVED:
        print("\n**反向验证失败**（说明上面这些判据里有不会红的）：")
        for m in MISBEHAVED:
            print("  -", m)
        return 1
    print(f"\n反向验证通过：基线与每个注入错都按期望红/绿（{CASES} 个注入错全部咬住）。")
    return 0


if __name__ == "__main__":
    sys.exit(main())
