"""快组（T0/T1，≤48 回合）：**读面契约**——不测世界的内容，测「读面说的事是不是真的」。

搬过来的是 `tests/projection_derived.rs`（跨进程、跨两条代码路径的一致）与
`tests/horizon_mid.rs` 里那条确定性守卫。它们全都只看**跑出来的数据**，所以最适合住在这里：

1. **同 seed 重跑逐字节一致**：`--index` 的每个文件（main.jsonl / idx/*.jsonl / schema / meta）
   两次运行的 sha256 必须完全相同——这是 spec 的硬要求，也锁住「新增机制没破坏可复现性」。
2. **两个读面给同一个值**：`--start <ckpt> --derived` 的 `post` 必须**逐值等于**同一回合
   `main.jsonl` 的 `view`；过程量表（`faction_process`）的列也必须等于 `post.factions[]` 的
   对应字段。若两个读面对同一回合各说各话，agent 会照着「另一个世界」的数字施政。
3. **中性值表里没有已经死掉的字段**：`schema.json` 的 `neutral.fields` 每条路径都要能在
   读面上解析出来（Rust 侧那条「反向」——读面每个叶子都声明了中性值——仍在 `src/tests/`，
   那一条要走类型 schema，搬过来要重写 schemars 的遍历）。
"""

from __future__ import annotations

import json
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _harness import KIT, group_main, projection_hashes  # noqa: E402

DET_SEEDS = (7, 42)          # 确定性：两个种子各跑两遍
DET_ROUNDS = 12
DERIVED_SEED, DERIVED_ROUNDS = 7, 6   # 两个读面的对账（与 Rust 版同一个种子/回合数）
# 过程量表要逐值比的列（表列名 → `post.factions[]` 里的字段名）。
PROC_COLUMNS = {
    "upkeep": "upkeep",
    "production": "production",
    "governance_total": "governance_cost",
    "governance_coverage": "governance_coverage",
    "governance_admin": "governance_admin",
    "governance_entertainment": "governance_entertainment",
    "governance_scale": "governance_scale",
    "ideology_loyalty_penalty": "ideology_loyalty_penalty",
    "capital_loyalty_bonus": "capital_loyalty_bonus",
}


def _resolve(view, path: str):
    """在示例视图上解析一条中性值路径（`factions[].governance_scale` / `decisions.capital`）。

    返回 True（解析到了）/ False（路径死了）/ None（这一段是空 map，这次看不出来）。
    """
    node = view
    for seg in path.split("."):
        if seg.endswith("[]"):
            key = seg[:-2]
            node = node.get(key) if isinstance(node, dict) else None
            if not isinstance(node, dict):
                return False
            if not node:
                return None
            node = next(iter(node.values()))
        else:
            if not isinstance(node, dict) or seg not in node:
                return False
            node = node[seg]
    return True


def run(h, ck) -> None:
    # ── ① 确定性：同 seed 两次运行的投影逐字节一致 ──────────────────────────
    tmp = Path(tempfile.mkdtemp(prefix="px-det-"))
    diff_bad: list[str] = []
    nfiles = 0
    for seed in DET_SEEDS:
        cached = h.projection(seed, DET_ROUNDS)
        rerun = tmp / f"seed{seed}"
        h.run_into(rerun, seed, DET_ROUNDS)
        a, b = projection_hashes(cached), projection_hashes(rerun)
        nfiles += len(a)
        if set(a) != set(b):
            diff_bad.append(f"seed {seed}: 两次跑出来的文件集合不同（{sorted(set(a) ^ set(b))}）")
            continue
        bad = [name for name in a if a[name] != b[name]]
        if bad:
            diff_bad.append(f"seed {seed}: {len(bad)} 个文件逐字节不同（{bad[:3]}）")
    ck.check("同 seed 重跑逐字节一致", not diff_bad,
             "；".join(diff_bad) or f"seed {list(DET_SEEDS)} × {DET_ROUNDS} 回合，{nfiles} 个文件全等")

    # ── ② 两个读面（--derived 的 post vs --index 的 view）逐值一致 ────────────
    proj = tmp / "derived"
    ckpt = tmp / "ckpt.ron"
    h.run_into(proj, DERIVED_SEED, DERIVED_ROUNDS, extra=("--save", str(ckpt)))
    v = json.loads(h.capture(["--start", str(ckpt), "--derived"]))
    note = json.loads((proj / "main.jsonl").read_text(encoding="utf-8").strip().splitlines()[-1])

    src_ok = v.get("source") == "checkpoint" and "note" not in v
    same = note["round"] == v["round"] and note["view"] == v["post"]
    ck.check("--derived 的 post ≡ --index 的 view（同回合逐值）", src_ok and same,
             f"seed {DERIVED_SEED} r{DERIVED_ROUNDS}：source={v.get('source')}、逐值相等={note['view'] == v['post']}")

    # 过程量表：平铺的列必须等于视图里那一行（两个读面读的是同一份 RoundView）。
    q = KIT.load(str(proj), only=("faction_process",))
    proc = q.faction_process(round=DERIVED_ROUNDS)
    rows = v["post"]["factions"]
    mism: list[str] = []
    governance_ran = 0
    for _, row in proc.iterrows():
        fid = row["faction_id"]
        if fid not in rows:
            mism.append(f"{fid} 在 view.factions 里没有行")
            continue
        for col, field in PROC_COLUMNS.items():
            if row[col] != rows[fid][field]:
                mism.append(f"{fid}.{col}={row[col]} ≠ view 的 {rows[fid][field]}")
        if (rows[fid]["governance_cost"] or 0.0) > 0.0:
            governance_ran += 1
    ck.check("过程量表的列 ≡ 视图里那一行", not mism,
             "；".join(mism[:3]) or f"{len(proc)} 个势力行、{len(PROC_COLUMNS)} 列全等")
    # 防空转：这一回合必须真跑过治理，否则上面比的是「零对零」。
    ck.check("对账没有空转（这一回合真跑过治理）", len(proc) >= 2 and governance_ran >= 1,
             f"{len(proc)} 个势力行、其中 {governance_ran} 家治理开销非零")

    # ── ③ 中性值表里没有死掉的路径 ────────────────────────────────────────────
    schema = json.loads((proj / "schema.json").read_text(encoding="utf-8"))
    fields = (schema.get("neutral") or {}).get("fields") or {}
    sample = json.loads((proj / "main.jsonl").read_text(encoding="utf-8").strip().splitlines()[-1])["view"]
    dead = [p for p in fields if _resolve(sample, p) is False]
    skipped = [p for p in fields if _resolve(sample, p) is None]
    ck.check("中性值表里的路径都活在读面上", bool(fields) and not dead,
             "；".join(dead[:3]) or f"{len(fields)} 条路径全部解析成功"
             + (f"（{len(skipped)} 条所在的 map 这一帧是空的，看不出来）" if skipped else ""))


if __name__ == "__main__":
    sys.exit(group_main("g1_contract", run))
