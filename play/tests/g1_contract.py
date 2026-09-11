"""快组（T0/T1，≤60 回合）：**读面契约**——不测世界的内容，测「读面说的事是不是真的」。

搬过来的是 `tests/projection_derived.rs`（6 条：跨进程、跨两条代码路径的一致）、
`tests/horizon_mid.rs` 的确定性守卫、`tests/control_read_face.rs`（读面即写面的不动点）。
它们全都只看**跑出来的数据**，所以最适合住在这里：

1. **同 seed 重跑逐字节一致**（spec 的硬要求，也锁住「新增机制没破坏可复现性」）；
2. **两个读面给同一个值**：`--start <ckpt> --derived` 的 `post` ≡ 同一回合 `main.jsonl` 的
   `view`；过程量表（`faction_process` / `city_process`）、判定表（`decisions`）、
   贸易两张表（`market_trades` / `haul_steps`）、输入面（`round_inputs` ≡ `pre`）逐值对得上。
   若两个读面对同一回合各说各话，agent 会照着「另一个世界」的数字施政——**失败看起来像成功**
   里最贵的一种；
3. **`--control` 是不动点**：dump → 原样回传 → 再 dump，两次逐字节相同；
4. **中性值表里没有已经死掉的路径**（反向那一半——「读面每个叶子都声明了中性值」——要走
   schemars 的类型遍历，仍在 `src/tests/model/neutral.rs`）。

⚠ **这里一律用 `json` 逐行读投影**，不走 pandas：这些断言要的是**逐值相等**，而 pandas 的
浮点解析曾经在这里制造过 1 ULP 的假红（见笔记 §10.3）。文件都小（≤60 回合），直读也不亏。
"""

from __future__ import annotations

import json
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _harness import KIT, group_main, projection_hashes  # noqa: E402

DET_SEEDS = (7, 42)                   # 确定性：两个种子各跑两遍
DET_ROUNDS = 12
FACE_SEED, FACE_ROUNDS = 7, 6         # 两个读面的对账（与 Rust 版同一个种子/回合数）
DECISIONS_ROUNDS = 60                 # 判定表：跑到「确实有仗打」的回合（Rust 版同）
B3_ROUNDS = 30                        # 贸易两张表：够到跨天体成交与在途/等待
INPUT_ROUNDS = 12                     # 输入面：够掷满一回合的骰子
WRITE_SEED, WRITE_ROUNDS = 42, 12     # 写面不动点：一个小而真实的世界

# 过程量表要逐值比的列（表列名 → `view.*[]` 里的字段名）。
FACTION_COLUMNS = {
    "upkeep": "upkeep",
    "production": "production",
    "governance_total": "governance_cost",
    "governance_coverage": "governance_coverage",
    "governance_admin": "governance_admin",
    "governance_entertainment": "governance_entertainment",
    "governance_scale": "governance_scale",
    "ideology_loyalty_penalty": "ideology_loyalty_penalty",
    "capital_loyalty_bonus": "capital_loyalty_bonus",
    "investment_spent": "investment_spent",
    "construction_spent": "construction_spent",
    "upkeep_unpaid": "upkeep_unpaid",
    "fleet_rust": "fleet_rust",
}
CITY_COLUMNS = {
    "production": "production",
    "loyalty_target_effective": "loyalty_target.effective",
    "loyalty_target_distance": "loyalty_target.distance",
    "labor": "labor",
    "housing_capacity": "housing_capacity",
    "is_hub": "is_hub",
    "build": "build",
}
TRADE_COLUMNS = ("buyer", "seller", "moved", "dist_au", "depth", "mond_extra", "freight_rate",
                 "rel_mult", "mastery", "loss")
SHIP_ORDER_WHITELIST = {"withdraw", "engage", "colonize", "bombard", "move", "haul", "hold"}


# ── 直读 JSONL 的小工具（不用 pandas，原因见模块文档）──────────────────────


def _lines(path: Path) -> list:
    if not path.exists():
        return []
    return [json.loads(l) for l in path.read_text(encoding="utf-8").splitlines() if l.strip()]


def _main_rows(proj: Path) -> list:
    return _lines(proj / "main.jsonl")


def _table(proj: Path, name: str) -> list:
    return _lines(proj / "idx" / f"{name}.jsonl")


def _rounds(rows: list, rnd: int) -> list:
    return [r for r in rows if r.get("round") == rnd]


def read_round(proj: Path, index: int) -> dict:
    """主流里第 index 行（-1 = 最后一回合）。"""
    return _main_rows(proj)[index]


def _dig(node, path: str):
    """按 `a.b.c` 取值（`loyalty_target.effective` 这种）。"""
    for seg in path.split("."):
        if not isinstance(node, dict) or seg not in node:
            return None
        node = node[seg]
    return node


def _resolve(view, path: str):
    """中性值路径解析：True 活着 / False 死了 / None 这一段是空 map（这次看不出来）。"""
    node = view
    for seg in path.split("."):
        if seg.endswith("[]"):
            node = node.get(seg[:-2]) if isinstance(node, dict) else None
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


# ── 各段契约 ────────────────────────────────────────────────────────────────


def determinism(h, ck, tmp: Path) -> None:
    """同 seed 两次运行的投影逐字节一致。"""
    diff_bad, nfiles = [], 0
    for seed in DET_SEEDS:
        cached = h.projection(seed, DET_ROUNDS)
        rerun = tmp / f"det{seed}"
        h.run_into(rerun, seed, DET_ROUNDS)
        a, b = projection_hashes(cached), projection_hashes(rerun)
        nfiles += len(a)
        if set(a) != set(b):
            diff_bad.append(f"seed {seed}: 文件集合不同（{sorted(set(a) ^ set(b))}）")
            continue
        bad = [n for n in a if a[n] != b[n]]
        if bad:
            diff_bad.append(f"seed {seed}: {len(bad)} 个文件逐字节不同（{bad[:3]}）")
    ck.check("同 seed 重跑逐字节一致", not diff_bad,
             "；".join(diff_bad) or f"seed {list(DET_SEEDS)} × {DET_ROUNDS} 回合，{nfiles} 个文件全等")


def derived_vs_index(h, ck, tmp: Path) -> None:
    """`--derived` 的 `post` ≡ `--index` 同一回合的 `view`，且过程量表逐列同源。"""
    proj, ckpt = tmp / "face", tmp / "face.json"
    h.run_into(proj, FACE_SEED, FACE_ROUNDS, extra=("--save", str(ckpt)))
    v = json.loads(h.capture(["--start", str(ckpt), "--derived"]))
    last = _main_rows(proj)[-1]
    post = v.get("post") or {}
    ck.check("--derived 的 post ≡ --index 的 view（同回合逐值）",
             v.get("source") == "checkpoint" and "note" not in v
             and last["round"] == v["round"] and last["view"] == post,
             f"seed {FACE_SEED} r{FACE_ROUNDS}：source={v.get('source')}、逐值相等={last['view'] == post}")

    def compare(rows, view_rows, columns, label, key_col):
        bad = []
        for row in rows:
            key = row.get(key_col)
            if key not in view_rows:
                bad.append(f"{key} 在 view.{label} 里没有行")
                continue
            for col, field in columns.items():
                got, want = row.get(col), _dig(view_rows[key], field)
                if got != want:
                    bad.append(f"{key}.{col}={got} ≠ view 的 {want}")
        return bad

    rows = _rounds(_table(proj, "faction_process"), FACE_ROUNDS)
    bad = compare(rows, post.get("factions") or {}, FACTION_COLUMNS, "factions", "faction_id")
    ck.check("过程量表 faction_process ≡ 视图里那一行", not bad,
             "；".join(bad[:3]) or f"{len(rows)} 个势力行 × {len(FACTION_COLUMNS)} 列全等")
    governance_ran = sum(1 for f in (post.get("factions") or {}).values()
                         if (f.get("governance_cost") or 0.0) > 0.0)
    ck.check("对账没有空转（这一回合真跑过治理）", len(rows) >= 2 and governance_ran >= 1,
             f"{len(rows)} 个势力行、其中 {governance_ran} 家治理开销非零")

    # ⚠ city_process **含已夷平的空白城**，而 `view.cities` 跳过它们（那张表只列活城）
    #   ⇒ 只能比**有产出的那些行**（Rust 版同口径）。
    crows = _rounds(_table(proj, "city_process"), FACE_ROUNDS)
    with_prod = [r for r in crows if r.get("production")]
    cbad = compare(with_prod, post.get("cities") or {}, CITY_COLUMNS, "cities", "city_id")
    ck.check("过程量表 city_process ≡ 视图里那一行（有产出的城）", not cbad,
             "；".join(cbad[:3]) or f"{len(with_prod)}/{len(crows)} 个城行 × {len(CITY_COLUMNS)} 列全等")
    targets = sum(1 for r in with_prod
                  if (_dig(post.get("cities", {}).get(r.get("city_id"), {}), "loyalty_target.effective") or 0) > 0)
    hubs = any((post.get("cities", {}).get(r.get("city_id"), {}).get("is_hub")) for r in with_prod)
    labor_ok = all((r.get("labor") or 0.0) > 0.0 for r in with_prod)
    ck.check("城过程量表没有空转（有产出/有忠诚目标/有集散地/用工系数不为 0）",
             bool(with_prod) and targets >= 1 and hubs and labor_ok,
             f"{len(with_prod)} 个城有产出、{targets} 个有忠诚目标、集散地={hubs}、用工系数全 >0={labor_ok}")
    ck.check("用工系数永远不为 0（中性值是 1.0，0 会被读成「全城没人上工」）",
             all((r.get("labor") or 0.0) > 0.0 for r in crows),
             f"{len(crows)} 个城行（含夷平城）的 labor 全 > 0")

    ctl = _rounds(_table(proj, "control"), FACE_ROUNDS)
    scp = _rounds(_table(proj, "scope"), FACE_ROUNDS)
    ck.check("控制面两张表在该回合有行（读面即写面的 tidy 版）",
             bool(ctl) and bool(scp), f"control {len(ctl)} 行 / scope {len(scp)} 行")


def checkpoint_flow(h, ck, tmp: Path) -> None:
    """起点那一行的过程量必须是**档里的真数**（退回重算会把过程量抹成 0）。"""
    run, ckpt = tmp / "flow", tmp / "flow.json"
    h.run_into(run, FACE_SEED, 4, extra=("--save", str(ckpt)))
    v = json.loads(h.capture(["--start", str(ckpt), "--derived"]))
    rnd = v["round"]
    proj = tmp / "flow0"
    h.run_into(proj, FACE_SEED, 0, extra=("--start", str(ckpt)))
    last = _main_rows(proj)[-1]
    ck.check("从档投影的起点行 = 档里那一回合的视图（不是重算）",
             last["round"] == rnd and last["view"] == v["post"],
             f"r{rnd}：起点视图与档里的 post 逐值相等={last['view'] == v['post']}")
    rows = _rounds(_table(proj, "faction_process"), rnd)
    bad = [f"{r['faction_id']}.upkeep={r['upkeep']} ≠ {v['post']['factions'][r['faction_id']]['upkeep']}"
           for r in rows if r["upkeep"] != v["post"]["factions"][r["faction_id"]]["upkeep"]]
    nonzero = sum(1 for r in rows if (r["upkeep"] or 0.0) > 0.0)
    ck.check("起点行的维护费是档里的真数（且真的非零）", bool(rows) and not bad and nonzero > 0,
             "；".join(bad[:2]) or f"{len(rows)} 行、{nonzero} 家维护费非零")

    fresh = tmp / "fresh"
    h.run_into(fresh, FACE_SEED, 0)
    fresh_rows = _rounds(_table(fresh, "faction_process"), 0)
    ck.check("全新开局的回合 0 没有过程量（那是初始世界）",
             bool(fresh_rows) and all((r["upkeep"] or 0.0) == 0.0 for r in fresh_rows),
             f"{len(fresh_rows)} 行，维护费全 0")


def spending_within_batch(h, ck, tmp: Path) -> None:
    """**花掉的钱不超过批的额度**（`src/tests/sim/spending.rs` 那条搬过来）。

    两个读面各给一半：`factions[].investment_spent` / `construction_spent` 是**真花掉的**
    （投影的过程量表），`--control` 的 `investment_budget` / `construction_budget` 是**批了多少**
    （控制面的**有效**值）。差额就是文档承诺的「批了却没花掉的那部分」，它必须 ≥ 0。

    ⚠ 必须跑够回合再看：开局那一回合既没有在建建筑、也没有攒到启封的造舰进度，**谁都还没花钱**
    ——拿回合 0 当样本就是「空表比空表」（Rust 版用 8 回合，这里用 60 回合那份档）。
    """
    proj, ckpt = tmp / "dec", tmp / "dec.json"       # 复用 decisions 那一段跑出来的档（60 回合）
    rnd = read_round(proj, -1)
    surface = json.loads(h.capture(["--start", str(ckpt), "--control"]))
    spent = {"investment_budget": "investment_spent", "construction_budget": "construction_spent"}
    bad, flowed, unspent = [], 0, 0
    for face in surface["control"]:
        fid = face["faction_id"]
        row = (rnd["view"].get("factions") or {}).get(fid)
        if row is None:
            bad.append(f"{fid} 在投影里没有势力行")
            continue
        for limit_kind, spent_field in spent.items():
            limits = {e["resource"]: e["value"] for e in (face.get(limit_kind) or [])}
            used = row.get(spent_field) or {}
            for rt, amt in used.items():
                limit = limits.get(rt, 0.0)
                if amt > limit + 1e-9:
                    bad.append(f"{fid}.{limit_kind}[{rt}]：花了 {amt} > 批的 {limit}")
                if amt > 0.0:
                    flowed += 1
            for rt, limit in limits.items():
                if limit - used.get(rt, 0.0) < -1e-9:
                    bad.append(f"{fid}.{limit_kind}[{rt}]：出现了负余额")
                if limit - used.get(rt, 0.0) > 1e-9:
                    unspent += 1
    ck.check("花掉的钱不超过批的额度（逐势力逐资源）", not bad,
             "；".join(bad[:3]) or f"第 {rnd['round']} 回合：{len(surface['control'])} 个势力的两本账都对得上")
    ck.check("预算守卫没有空转（真的花过钱、也真的有没花掉的）", flowed >= 1 and unspent >= 1,
             f"{flowed} 处真的花了钱、{unspent} 处有没花掉的余额")


def derived_without_checkpoint(h, ck, tmp: Path) -> None:
    """没有档时 `--derived` 必须**明说自己是从当前状态重算的**，而不是给一份像事实的空壳。"""
    v = json.loads(h.capture(["--seed", str(FACE_SEED), "--derived"]))
    rows = (v.get("post") or {}).get("factions") or {}
    empty = all((r.get("upkeep") or 0.0) == 0.0 and not (r.get("production") or {})
                for r in rows.values())
    ck.check("无档的 --derived 自报重算（source=state + note）",
             v.get("source") == "state" and isinstance(v.get("note"), str),
             f"source={v.get('source')}、note={'有' if isinstance(v.get('note'), str) else '没有'}")
    ck.check("无档时不凭空出现过程量，但观测行仍在", len(rows) >= 2 and empty,
             f"{len(rows)} 个势力行、过程量全 0/空={empty}")


def decisions_table(h, ck, tmp: Path) -> None:
    """判定表（`idx/decisions.jsonl`）≡ `post.decisions`，且**必须真有东西**。

    这是一张「空白也有意义」的表（`hold` = 这回合 AI 没派活），最容易悄悄退化成永远为空——
    那比没有表更坏，它看起来像「AI 这一回合什么也没决定」。
    """
    proj, ckpt = tmp / "dec", tmp / "dec.json"
    h.run_into(proj, FACE_SEED, DECISIONS_ROUNDS, extra=("--save", str(ckpt)))
    v = json.loads(h.capture(["--start", str(ckpt), "--derived"]))
    rnd = v["round"]
    dec = v["post"]["decisions"]
    rows = _rounds(_table(proj, "decisions"), rnd)
    order_rows = [r for r in rows if r["kind"] == "ship_order"]
    retool_rows = [r for r in rows if r["kind"] == "retool"]
    bad = []
    if len(order_rows) != len(dec["ships"]):
        bad.append(f"逐舰判定条数 {len(order_rows)} ≠ view 的 {len(dec['ships'])}")
    if len(retool_rows) != len(dec["retools"]):
        bad.append(f"改装判定条数 {len(retool_rows)} ≠ view 的 {len(dec['retools'])}")
    for d in dec["ships"]:
        # 一艘舰一回合**最多两行**（先机动、到位后再判一次）⇒ 配对键是 (actor, verdict, after_move)。
        hit = next((r for r in order_rows
                    if r["actor"] == d["ship"] and r["verdict"] == d["verdict"]
                    and _dig(r, "detail.after_move") == d["after_move"]), None)
        if hit is None:
            bad.append(f"表里缺 {d['ship']} 的判定行")
            continue
        for col, want in (("faction_id", d["faction"]), ("target", d["target"]),
                          ("detail.hull_ratio", d["hull_ratio"]), ("detail.retreat_hull", d["retreat_hull"]),
                          ("detail.kiting", d["kiting"]), ("detail.enemy_in_range", d["enemy_in_range"]),
                          ("detail.destination", d["destination"]), ("detail.order", d["order"])):
            got = hit[col] if col in hit else _dig(hit, col)
            if got != want:
                bad.append(f"{d['ship']}.{col}={got} ≠ {want}")
        if hit["verdict"] not in SHIP_ORDER_WHITELIST:
            bad.append(f"出现了没声明过的判定：{hit['verdict']}")
    for r in dec["retools"]:
        hit = next((x for x in retool_rows if x["actor"] == r["city"]), None)
        if hit is None:
            bad.append(f"表里缺 {r['city']} 的改装行")
            continue
        for col, want in (("faction_id", r["faction"]), ("target", r["to"]),
                          ("detail.from", r["from"]), ("detail.building", r["building"])):
            got = hit[col] if col in hit else _dig(hit, col)
            if got != want:
                bad.append(f"{r['city']}.{col}={got} ≠ {want}")
    ck.check("判定表 ≡ view.decisions（逐行字段）", not bad,
             "；".join(bad[:3]) or f"{len(order_rows)} 条逐舰 + {len(retool_rows)} 条改装，字段全等")

    # 防空转：范围取**整局**，不取最后一回合（最后一帧有没有仗打取决于当回合态势，
    # 钉死单帧会随轨迹漂移而随机翻车——Rust 版就是这么改过来的）。
    all_rows = _table(proj, "decisions")
    verdicts = {r["verdict"] for r in all_rows if r["kind"] == "ship_order"}
    fought = bool(verdicts & {"engage", "bombard"})
    proc = _table(proj, "faction_process")
    spent = any(any((v or 0.0) > 0.0 for v in (r.get(k) or {}).values())
                for r in proc for k in ("investment_spent", "construction_spent"))
    rust_unpaid = any((r.get("fleet_rust") or 0.0) > 0.0 and (r.get("upkeep_unpaid") or 0.0) > 0.0
                      for r in proc)
    lines = any(r.get("build") for r in _table(proj, "city_process"))
    trades = _table(proj, "market_trades")
    steps = {r["step"] for r in _table(proj, "haul_steps")}
    freight = any(g.get("uncovered", 0.0) > 0.0
                  for r in proc for g in (r.get("freight_gap") or {}).values())
    ck.check("判定表没有空转（整局里出现过打仗/花钱/欠费/造舰/贸易/装卸/在途/积压）",
             len(order_rows) >= 4 and fought and spent and rust_unpaid and lines
             and trades and steps & {"loaded", "delivered"} and steps & {"waiting", "en_route"} and freight,
             f"最后一回合 {len(order_rows)} 条逐舰判定；整局：打仗={fought} 花钱={spent} 欠费生锈={rust_unpaid} "
             f"造舰行={lines} 成交={len(trades)} 运输档={sorted(steps)} 有积压缺口={freight}")


def b3_tables(h, ck, tmp: Path) -> None:
    """贸易两张表 ≡ `post`（跨进程、跨两条代码路径给同一份数）。"""
    proj, ckpt = tmp / "b3", tmp / "b3.json"
    h.run_into(proj, FACE_SEED, B3_ROUNDS, extra=("--save", str(ckpt)))
    v = json.loads(h.capture(["--start", str(ckpt), "--derived"]))
    rnd = v["round"]
    want = v["post"]["market_trades"]
    rows = _rounds(_table(proj, "market_trades"), rnd)
    bad = [] if len(rows) == len(want) else [f"成交笔数 {len(rows)} ≠ view 的 {len(want)}"]
    for row, w in zip(rows, want):
        for col in TRADE_COLUMNS:
            if row.get(col) != w.get(col):
                bad.append(f"{row.get('buyer')}×{row.get('seller')}.{col}={row.get(col)} ≠ {w.get(col)}")
    ck.check("成交表 market_trades ≡ view（逐字段）", not bad,
             "；".join(bad[:3]) or f"第 {rnd} 回合 {len(rows)} 笔，{len(TRADE_COLUMNS)} 列全等")

    steps = v["post"]["haul_steps"]
    hrows = _rounds(_table(proj, "haul_steps"), rnd)
    hbad = [] if len(hrows) == len(steps) else [f"运输动作条数 {len(hrows)} ≠ view 的 {len(steps)}"]
    for row in hrows:
        ship = row["ship_id"]
        w = steps.get(ship)
        if w is None:
            hbad.append(f"view 里没有 {ship} 的动作")
            continue
        if row["step"] != w["step"] or row["body"] != w["body"]:
            hbad.append(f"{ship}: {row['step']}/{row['body']} ≠ {w['step']}/{w['body']}")
        # ⚠ 视图是**tag 枚举**（变体专属载荷只在它自己那一档出现），表是**平铺列**（每行都有）：
        # 非装卸档要求表里是「没有」的中性值（0 / false），且视图里**不该出现**那个字段。
        if w["step"] in ("loaded", "delivered"):
            if row["units"] != w["units"]:
                hbad.append(f"{ship}.units={row['units']} ≠ {w['units']}")
        elif row["units"] != 0.0 or "units" in w:
            hbad.append(f"{ship}: 等待/在途不该有件数（表 {row['units']}，视图里有 units={'units' in w}）")
        if w["step"] == "delivered":
            if row["into_pool"] != w["into_pool"]:
                hbad.append(f"{ship}.into_pool={row['into_pool']} ≠ {w['into_pool']}")
        elif row["into_pool"] is not False:
            hbad.append(f"{ship}: 非卸货档的 into_pool 应为 false，实为 {row['into_pool']}")
    ck.check("运输表 haul_steps ≡ view（含 tag 枚举 ↔ 平铺列 的接缝）", not hbad,
             "；".join(hbad[:3]) or f"第 {rnd} 回合 {len(hrows)} 条动作全等")

    all_trades = _table(proj, "market_trades")
    kinds = {r["step"] for r in _table(proj, "haul_steps")}
    ck.check("贸易两张表没有空转（有跨天体成交 + 有 waiting/en_route）",
             any((r.get("dist_au") or 0.0) > 0.0 for r in all_trades)
             and bool(kinds & {"waiting", "en_route"}),
             f"{len(all_trades)} 笔成交、运输档 {sorted(kinds)}")


def input_face(h, ck, tmp: Path) -> None:
    """输入面（B5）：`--index` 的 `round_inputs` ≡ `--derived` 的 `pre`。"""
    proj, ckpt = tmp / "b5", tmp / "b5.json"
    h.run_into(proj, FACE_SEED, INPUT_ROUNDS, extra=("--save", str(ckpt)))
    v = json.loads(h.capture(["--start", str(ckpt), "--derived"]))
    rnd = v["round"]
    pre = v["pre"]
    banned = [k for k in ("factions", "cities", "wars", "power_share", "fleet_value", "decisions")
              if k in pre]
    ck.check("输入面里没有观测字段（它属于 post）", not banned, f"混进了 {banned}" if banned else "干净")

    rows = _rounds(_table(proj, "round_inputs"), rnd)
    bad = [] if len(rows) == 1 else [f"第 {rnd} 回合的输入面应当是 1 行，实为 {len(rows)}"]
    if rows:
        for col in ("order", "relation_noise", "rolls"):
            if rows[0].get(col) != pre.get(col):
                bad.append(f"{col} 两个读面不一致")
    order = pre.get("order") or []
    noise = pre.get("relation_noise") or {}
    rolls = pre.get("rolls") or []
    ck.check("输入面 round_inputs ≡ view 的 pre（逐字段）", not bad,
             "；".join(bad[:3]) or f"{len(order)} 个名字的解算顺序、{len(noise)} 对关系噪声、{len(rolls)} 条抽签全等")
    ck.check("输入面没有空转（真掷过：顺序是整份名单、噪声覆盖每一对）",
             len(order) >= 5 and len(noise) >= 2, f"order={len(order)}、noise={len(noise)}")

    # 起点行（round 0）没有掷过骰子 ⇒ 输入面是空的（「没跑」而不是「掷出了 0」）。
    zero = _rounds(_table(proj, "round_inputs"), 0)
    ck.check("round 0 的输入面是空的那一行",
             len(zero) == 1 and zero[0].get("order") == [] and zero[0].get("relation_noise") == {},
             f"{len(zero)} 行，order={zero[0].get('order') if zero else None}")


def control_fixed_point(h, ck, tmp: Path) -> None:
    """`--control` 是不动点：dump → 原样回传 → 再 dump 必须逐字节相同（`control_read_face.rs`）。

    它同时钉住两件事：指令读面**每舰一行**（叶被删掉之后那艘舰不许从控制树里消失）、
    以及 `behavior: null`（链上没人说话）回传时**不许建叶**。
    """
    ck0, ck1, ck2 = tmp / "w0.json", tmp / "w1.json", tmp / "w2.json"
    out = h.capture(["--seed", str(WRITE_SEED), "--round", str(WRITE_ROUNDS),
                     "--save", str(ck0)])
    state = json.loads(h.capture(["--start", str(ck0), "--round", "0"]).splitlines()[0])
    ships = state["ships"]
    fid = next((f for f in {s["势力"] for s in ships}
                if sum(1 for s in ships if s["势力"] == f) >= 2), None)
    if fid is None:
        ck.check("写面不动点：有可用的势力", False, "没有任何势力有 2 艘以上舰——守卫会空转")
        return
    ours = [s["舰名"] for s in ships if s["势力"] == fid]
    vanished = ours[0]

    rm = tmp / "rm.json"
    rm.write_text(json.dumps({"control": [{"faction_id": fid,
                                           "ship_orders": [{"ship": vanished, "remove": True}]}]}),
                  encoding="utf-8")
    _, err = h.capture(["--start", str(ck0), "--apply", str(rm), "--round", "0", "--save", str(ck1)],
                       stderr=True)
    ck.check("删叶会留下回执（删的是叶，不是值）", "NOTE_APPLY_REMOVED" in err,
             "stderr 里有 NOTE_APPLY_REMOVED" if "NOTE_APPLY_REMOVED" in err else f"stderr={err[-200:]}")

    def orders(surface, faction):
        for f in surface["control"]:
            if f["faction_id"] == faction:
                return f["ship_orders"]
        raise AssertionError(f"控制面里没有势力 {faction}")

    before = h.capture(["--start", str(ck1), "--control"])
    surface = json.loads(before)
    rows = [r["ship"] for r in orders(surface, fid)]
    ck.check("指令读面每舰一行且顺序与 state.ships 一致（含叶被删掉的那艘）", rows == ours,
             f"{fid}：读面 {len(rows)} 行 / 世界 {len(ours)} 艘" + ("" if rows == ours else f"，差异 {set(rows) ^ set(ours)}"))
    row = next((r for r in orders(surface, fid) if r["ship"] == vanished), None)
    ck.check("叶被删掉的舰：behavior 是 null、mode 是 Inherit",
             row == {"ship": vanished, "behavior": None, "mode": "Inherit"}, f"{row}")
    others_ok = all(next(r for r in orders(surface, fid) if r["ship"] == s)["behavior"] is not None
                    for s in ours if s != vanished)
    ck.check("叶还在的舰：有效值必须是一个真行为（不是 null）", others_ok,
             f"{len(ours) - 1} 艘有叶的舰都给了真行为")

    t0 = tmp / "t0.json"
    t0.write_text(before, encoding="utf-8")
    _, err = h.capture(["--start", str(ck1), "--apply", str(t0), "--round", "0", "--save", str(ck2)],
                       stderr=True)
    after = h.capture(["--start", str(ck2), "--control"])
    ck.check("原样回传模板不丢叶子", "WARN_APPLY_SKIPPED" not in err,
             "stderr 干净" if "WARN_APPLY_SKIPPED" not in err else f"stderr={err[-200:]}")
    ck.check("读面是不动点（原样回传不改变它自己的形状）", before == after,
             "逐字节相同" if before == after else "回传后读面变了")
    surface2 = json.loads(after)
    row2 = next((r for r in orders(surface2, fid) if r["ship"] == vanished), None)
    state2 = json.loads(h.capture(["--start", str(ck2), "--round", "0"]).splitlines()[0])
    ck.check("回传不许偷偷把 null 变成 Idle（那一行还必须说「链上没人说话」）",
             row2 == {"ship": vanished, "behavior": None, "mode": "Inherit"},
             f"{row2}")
    ck.check("回传不增删舰，且仍然每舰一行",
             [s["舰名"] for s in state2["ships"] if s["势力"] == fid] == ours
             and len(orders(surface2, fid)) == len(ours),
             f"{len(orders(surface2, fid))} 行 / {len(ours)} 艘")


def neutral_paths(h, ck, tmp: Path) -> None:
    """`schema.json` 的 `neutral.fields` 每条路径都要能在读面上解析出来（没有死字段）。"""
    proj = h.projection(FACE_SEED, DET_ROUNDS)
    schema = json.loads((proj / "schema.json").read_text(encoding="utf-8"))
    fields = (schema.get("neutral") or {}).get("fields") or {}
    sample = _main_rows(proj)[-1]["view"]
    dead = [p for p in fields if _resolve(sample, p) is False]
    skipped = [p for p in fields if _resolve(sample, p) is None]
    ck.check("中性值表里的路径都活在读面上", bool(fields) and not dead,
             "；".join(dead[:3]) or f"{len(fields)} 条路径全部解析成功"
             + (f"（{len(skipped)} 条所在的 map 这一帧是空的，看不出来）" if skipped else ""))


def run(h, ck) -> None:
    tmp = Path(tempfile.mkdtemp(prefix="px-face-"))
    determinism(h, ck, tmp)
    derived_vs_index(h, ck, tmp)
    checkpoint_flow(h, ck, tmp)
    derived_without_checkpoint(h, ck, tmp)
    decisions_table(h, ck, tmp)
    spending_within_batch(h, ck, tmp)
    b3_tables(h, ck, tmp)
    input_face(h, ck, tmp)
    control_fixed_point(h, ck, tmp)
    neutral_paths(h, ck, tmp)


if __name__ == "__main__":
    sys.exit(group_main("g1_contract", run))
