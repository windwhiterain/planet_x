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
    bad = compare(rows, post.get("factions") or {}, FACTION_COLUMNS, "factions", "势力")
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
    cbad = compare(with_prod, post.get("cities") or {}, CITY_COLUMNS, "cities", "城名")
    ck.check("过程量表 city_process ≡ 视图里那一行（有产出的城）", not cbad,
             "；".join(cbad[:3]) or f"{len(with_prod)}/{len(crows)} 个城行 × {len(CITY_COLUMNS)} 列全等")
    targets = sum(1 for r in with_prod
                  if (_dig(post.get("cities", {}).get(r.get("城名"), {}), "loyalty_target.effective") or 0) > 0)
    hubs = any((post.get("cities", {}).get(r.get("城名"), {}).get("is_hub")) for r in with_prod)
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
    bad = [f"{r['势力']}.upkeep={r['upkeep']} ≠ {v['post']['factions'][r['势力']]['upkeep']}"
           for r in rows if r["upkeep"] != v["post"]["factions"][r["势力"]]["upkeep"]]
    nonzero = sum(1 for r in rows if (r["upkeep"] or 0.0) > 0.0)
    ck.check("起点行的维护费是档里的真数（且真的非零）", bool(rows) and not bad and nonzero > 0,
             "；".join(bad[:2]) or f"{len(rows)} 行、{nonzero} 家维护费非零")

    fresh = tmp / "fresh"
    h.run_into(fresh, FACE_SEED, 0)
    fresh_rows = _rounds(_table(fresh, "faction_process"), 0)
    ck.check("全新开局的回合 0 没有过程量（那是初始世界）",
             bool(fresh_rows) and all((r["upkeep"] or 0.0) == 0.0 for r in fresh_rows),
             f"{len(fresh_rows)} 行，维护费全 0")
    # 事件层那两半（原 Rust `sim/tests/fleet.rs::advance_populates_round_events`，2026-10 搬来：
    # 那边只断言「回合 0 空 → 跑几回合非空」，而这两半在读面上都在）。
    ev0 = _rounds(_table(fresh, "events"), 0)
    ck.check("全新开局的回合 0 没有事件（初始世界什么都没发生过）",
             ev0 == [], f"回合 0 竟有 {len(ev0)} 条事件")
    ev_all = _table(proj, "events")
    ck.check("推进过就有事件（事件层真的在写，不是空表通过）", len(ev_all) > 0,
             f"r{rnd} 的投影里累计 {len(ev_all)} 条事件")


def spending_within_batch(h, ck, tmp: Path) -> None:
    """**花掉的钱不超过批的额度**（`src/tests/sim/spending.rs` 那条搬过来）。

    两个读面各给一半：`factions[].investment_spent` / `construction_spent` 是**真花掉的**
    （投影的过程量表），`--control` 的 `投资预算` / `建造预算` 是**批了多少**
    （控制面的**有效**值）。差额就是文档承诺的「批了却没花掉的那部分」，它必须 ≥ 0。

    ⚠ 必须跑够回合再看：开局那一回合既没有在建建筑、也没有攒到启封的造舰进度，**谁都还没花钱**
    ——拿回合 0 当样本就是「空表比空表」（Rust 版用 8 回合，这里用 60 回合那份档）。
    """
    proj, ckpt = tmp / "dec", tmp / "dec.json"       # 复用 decisions 那一段跑出来的档（60 回合）
    rnd = read_round(proj, -1)
    surface = json.loads(h.capture(["--start", str(ckpt), "--control"]))
    spent = {"投资预算": "investment_spent", "建造预算": "construction_spent"}
    bad, flowed, unspent = [], 0, 0
    for face in surface["control"]:
        fid = face["势力"]
        row = (rnd["view"].get("factions") or {}).get(fid)
        if row is None:
            bad.append(f"{fid} 在投影里没有势力行")
            continue
        for limit_kind, spent_field in spent.items():
            limits = {e["资源"]: e["值"] for e in (face.get(limit_kind) or [])}
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
        for col, want in (("势力", d["faction"]), ("target", d["target"]),
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
        for col, want in (("势力", r["faction"]), ("target", r["to"]),
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
        ship = row["舰名"]
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


def world_shape(h, ck, tmp: Path) -> None:
    """**世界形状**：城 ↔ 定居点一一对应、矿藏按定居点隔离
    （`src/tests/sim/mod.rs::settlements_and_cities_are_one_to_one`，2026-10 第 7 批）。

    读面 = `bodies.settlement_count` + `settlements.资源` + `cities.{天体名, 定居点}`，全是回合 0 的
    静态形状 ⇒ 与 Rust 原件（`fresh_world(42)`）同一口径。
    """
    proj = tmp / "shape"
    h.run_into(proj, FACE_SEED, 0)
    bodies = _table(proj, "bodies")
    settlements = _table(proj, "settlements")
    cities = _rounds(_table(proj, "cities"), 0)

    on_body: dict[str, set] = {}
    for s in settlements:
        on_body.setdefault(s["天体名"], set()).add(s["定居点"])
    counts = {b["天体名"]: b["settlement_count"] for b in bodies}

    stray = [f"{c['城名']} 占的 {c['定居点']} 不在 {c['天体名']} 上" for c in cities
             if c["定居点"] not in on_body.get(c["天体名"], set())]
    pairs = [(c["天体名"], c["定居点"]) for c in cities]
    dup = sorted({p for p in pairs if pairs.count(p) > 1})
    over = [f"{b}: {sum(1 for c in cities if c['天体名'] == b)} 座城 > {n} 个定居点"
            for b, n in counts.items() if sum(1 for c in cities if c["天体名"] == b) > n]
    ck.check("世界形状：每座城占的定居点都在它自己的天体上、不重号、不超过该天体的定居点数",
             not (stray or dup or over),
             "；".join((stray + [f"重号 {d}" for d in dup] + over)[:3])
             or f"{len(cities)} 座城 ↔ {len(settlements)} 个定居点（{len(bodies)} 个天体）全自洽")
    ck.check("世界形状守卫没有空转（真有多定居点天体、也有真城）",
             len(cities) >= 5 and any(n >= 2 for n in counts.values()),
             f"{len(cities)} 座城；定居点数分布 {sorted(counts.values())}")

    earth = [s for s in settlements if s["天体名"] == "地球"]
    paris = next((s for s in earth if s["定居点"] == "巴黎"), None)
    cn = next((s for s in earth if s["定居点"] == "长三角"), None)
    paris_res = sorted(d["resource"] for d in ((paris or {}).get("资源") or []))
    cn_res = {d["resource"] for d in ((cn or {}).get("资源") or [])}
    ck.check("世界形状：矿藏按定居点隔离（地球 5 个定居点、巴黎只产自己的矿、长三角有铁/硅/水冰）",
             len(earth) == 5 and paris_res == ["铀", "铂"] and {"铁", "硅", "水冰"} <= cn_res,
             f"地球 {len(earth)} 个定居点；巴黎 {paris_res}；长三角 {sorted(cn_res)}")


def mond_start_checks(h, ck, tmp: Path) -> None:
    """**MOND 开局打点**（`sim/tests/knowledge.rs::initial_mastery_comes_from_config_and_frontier_reads_it`）。

    「只有崇拜教天生 1.0，其余一个都不白拿」——判据**从 `meta.mond.initial` 读**，不写死名字：
    配置里写谁就照给谁，没写的一个都不许有。防空转 = 配置里**真的点了名**（否则「全 0」也是「全对」）。
    """
    proj = tmp / "mond0"
    h.run_into(proj, FACE_SEED, 0)
    facs = _rounds(_table(proj, "factions"), 0)
    initial = {k: float(v) for k, v in ((json.loads(h.capture(["--meta"])).get("mond") or {})
                                        .get("initial") or {}).items()}
    bad = [f"{r['势力']}：读面 {r['MOND 掌握度']} ≠ 配置 {initial.get(r['势力'], 0.0)}"
           for r in facs if abs(float(r["MOND 掌握度"]) - initial.get(r["势力"], 0.0)) > 1e-12]
    ck.check("MOND 开局打点从配置读（表里写谁的名字就照给，其余一个都不白拿）",
             bool(facs) and not bad, "；".join(bad[:3]) or f"{len(facs)} 个势力的开局掌握度全对")
    named = {r["势力"] for r in facs if float(r["MOND 掌握度"]) > 0.0}
    ck.check("MOND 开局打点守卫没有空转（配置里真的点名了天生掌握者）",
             named == set(initial) and bool(initial),
             f"配置点名 {sorted(initial)}，读面上有掌握度的 {sorted(named)}")


def neutral_defaults(h, ck, tmp: Path) -> None:
    """**`pre` 面的中性缺省**（`sim/tests/governance.rs::pre_view_has_neutral_b1_defaults`，第 7 批）。

    缺省值是**读面契约的一半**：`pre`/`post` 同形的代价就是那几个缺省值必须说话算话——
    **倍率类的中性值是 1.0 而不是 0**（0 会被读成「治理能力归零」）。回合 0 = 治理/娱乐都没跑过。
    """
    proj = tmp / "neutral"
    h.run_into(proj, FACE_SEED, 0)
    fp = _rounds(_table(proj, "faction_process"), 0)
    bad = [f"{r['势力']}: admin={r['governance_admin']} 娱乐={r['governance_entertainment']} "
           f"scale={r['governance_scale']} 思潮罚={r['ideology_loyalty_penalty']} "
           f"首都向心={r['capital_loyalty_bonus']}" for r in fp
           if (r["governance_admin"] or 0) != 0 or (r["governance_entertainment"] or 0) != 0
           or (r["governance_scale"] or 0) != 1.0 or (r["ideology_loyalty_penalty"] or 0) != 0
           or (r["capital_loyalty_bonus"] or 0) != 0]
    ck.check("回合 0 的治理量是中性缺省（倍率 1.0，不是 0）", bool(fp) and not bad,
             "；".join(bad[:3]) or f"{len(fp)} 个势力全是中性缺省")

    cp = _rounds(_table(proj, "city_process"), 0)
    cbad = [f"{r['城名']}: 有效={r['loyalty_target_effective']} 距离={r['loyalty_target_distance']}"
            for r in cp if (r["loyalty_target_effective"] or 0) != 0
            or (r["loyalty_target_distance"] or 0) != 0]
    ck.check("回合 0 的忠诚目标过程量是 0（没跑治理 ⇒ 没有「距离目标」）", bool(cp) and not cbad,
             "；".join(cbad[:3]) or f"{len(cp)} 座城全 0")

    cap = [r for r in _rounds(_table(proj, "decisions"), 0) if r.get("kind") == "capital"]
    ck.check("回合 0 一条首都判定都没有（首都判定是稀疏的）", not cap, f"{len(cap)} 行")


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

    input_face_shape(h, ck, proj, ckpt)


def input_face_shape(h, ck, proj: Path, ckpt: Path) -> None:
    """输入面**逐回合**的三条形状判据（`src/tests/sim/inputs.rs` 那三条搬来，2026-10 第 7 批）。

    读面 = `round_inputs.{order, relation_noise, rolls}` + `ships`（回合末、**保持世界的舰序**）
    + `events[ship_destroyed]` + `factions` + `meta.diplomacy.noise`：

    | Rust 原件 | 怎么判 |
    | --- | --- |
    | 解算顺序是不重不漏的名单 | 逐回合：无重复、覆盖回合末还活着的舰、**多出来的每一个都是本回合 `ship_destroyed` 的**，且顺序真的被打乱过（≠ 舰表顺序） |
    | 关系噪声覆盖每一对、落在 `±noise` | 逐回合：对数 = `n(n-1)/2`、无自环、`|v| ≤ meta.diplomacy.noise`（`noise = 0` ⇒ 这一节必须为空） |
    | `rolls` 的形状与内容 | 逐条：`value ∈ [0,1)`、`faction`/`subject` 不空、闸门 xor 加权抽签（导航是第三种：幅度骰）、抽签池非空且**权重和 = `pool_total`**、`picked` 在池里 |
    """
    ri = _table(proj, "round_inputs")
    ships = _table(proj, "ships")
    ev = _table(proj, "events")
    facs = _table(proj, "factions")
    noise_cfg = float(json.loads(h.capture(["--start", str(ckpt), "--meta"]))["diplomacy"]["noise"])

    world_order: dict[int, list[str]] = {}
    alive: dict[int, list[str]] = {}
    for s in ships:
        world_order.setdefault(s["round"], []).append(s["舰名"])
        if (s.get("船体") or 0.0) > 0.0:
            alive.setdefault(s["round"], []).append(s["舰名"])
    fids: dict[int, set[str]] = {}
    for f in facs:
        fids.setdefault(f["round"], set()).add(f["势力"])
    sunk = {(e["round"], e["target_id"]) for e in ev if e["type"] == "ship_destroyed"}

    order_bad: list[str] = []
    noise_bad: list[str] = []
    roll_bad: list[str] = []
    shuffled = dead_extra = 0
    purposes: dict[str, int] = {}
    for r in ri:
        rnd = r["round"]
        if rnd == 0:  # 起点行没掷过骰子，另有判据盯着
            continue
        o = r.get("order") or []
        if not o:
            order_bad.append(f"r{rnd}: 解算顺序是空的")
        if len(set(o)) != len(o):
            order_bad.append(f"r{rnd}: 顺序里有重复的舰")
        live, listed = set(alive.get(rnd, [])), set(o)
        miss = sorted(live - listed)
        if miss:
            order_bad.append(f"r{rnd}: 幸存者 {miss[:2]} 不在顺序里")
        extra = sorted(listed - live)
        dead_extra += len(extra)
        not_dead = [n for n in extra if (rnd, n) not in sunk]
        if not_dead:
            order_bad.append(f"r{rnd}: {not_dead[:2]} 不在世界末态里、也不是本回合沉的")
        if len(o) > 3:
            head = [n for n in world_order.get(rnd, []) if n in listed]
            if o == head:
                order_bad.append(f"r{rnd}: 顺序与舰表逐字相同（洗牌没生效？）")
            else:
                shuffled += 1

        noise = r.get("relation_noise") or {}
        pairs = [(a, b, v) for a, row in noise.items() for b, v in row.items()]
        if noise_cfg > 0.0:
            n_fid = len(fids.get(rnd, ()))
            if len(pairs) != n_fid * (n_fid - 1) // 2:
                noise_bad.append(f"r{rnd}: 只记了 {len(pairs)} 对，{n_fid} 个势力应有 {n_fid * (n_fid - 1) // 2} 对")
            for a, b, val in pairs:
                if a == b:
                    noise_bad.append(f"r{rnd}: {a} 跟自己有噪声")
                elif abs(val) > noise_cfg + 1e-9:
                    noise_bad.append(f"r{rnd}: {a}→{b} 的噪声 {val} 超出 ±{noise_cfg}")
        elif pairs:
            noise_bad.append(f"r{rnd}: 配置里 noise = 0，却记了 {len(pairs)} 对")

        for x in r.get("rolls") or []:
            purposes[x["purpose"]] = purposes.get(x["purpose"], 0) + 1
            where = f"r{rnd} {x['purpose']}"
            if not (0.0 <= x["value"] < 1.0):
                roll_bad.append(f"{where}: value={x['value']} 不在 [0,1)")
            if not x["faction"] or not x["subject"]:
                roll_bad.append(f"{where}: 指不回「谁、对什么」")
            th, pt = x["threshold"], x["pool_total"]
            if th is not None and pt is not None:
                roll_bad.append(f"{where}: 既是闸门又是抽签")
            elif th is not None:
                if not (0.0 <= th <= 1.0) or x["picked"] is None:
                    roll_bad.append(f"{where}: 闸门的机会值/走了哪一支不对（{th}/{x['picked']}）")
            elif pt is not None:
                pool = x.get("pool") or []
                if pt <= 0.0 or not pool:
                    roll_bad.append(f"{where}: 抽签池空或总权重 ≤ 0")
                else:
                    tot = sum(e["weight"] for e in pool)
                    if abs(tot - pt) > 1e-9:
                        roll_bad.append(f"{where}: 池内权重和 {tot} ≠ pool_total {pt}")
                    if x["picked"] not in {e["name"] for e in pool}:
                        roll_bad.append(f"{where}: 抽中的 {x['picked']} 不在候选池里")
            elif x["purpose"] != "nav":
                roll_bad.append(f"{where}: 除导航外不该有第三种形状（既非闸门也非抽签）")

    ck.check("输入面：解算顺序是不重不漏的名单（多出来的都是本回合战沉）", not order_bad,
             "；".join(order_bad[:3]) or f"{len(ri) - 1} 个回合全部自洽")
    ck.check("输入面：解算顺序守卫没有空转（洗牌真打乱过、也真有战沉）",
             shuffled > 0 and dead_extra > 0, f"{shuffled} 个回合顺序被打乱，多出来的战沉共 {dead_extra} 舰")
    ck.check("输入面：关系噪声覆盖每一对、且落在 ±%g 里" % noise_cfg, not noise_bad,
             "；".join(noise_bad[:3]) or f"{len(ri) - 1} 个回合逐对成立")
    ck.check("输入面：抽签记录形状自洽（闸门 / 加权抽签 / 导航各按各的规矩）", not roll_bad,
             "；".join(roll_bad[:3]) or f"共 {sum(purposes.values())} 条抽签，形状全对")
    want = ("gate", "accept", "assign", "role", "review", "renew", "retool", "nav",
            "style_chance", "blueprint_intent")
    missing = [w for w in want if w not in purposes]
    ck.check("输入面：抽签守卫没有空转（十个用途都真的掷过）", not missing,
             f"缺 {missing}" if missing else f"{len(purposes)} 个用途都出现了：{sorted(purposes)}")


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
    rm.write_text(json.dumps({"control": [{"势力": fid,
                                           "指令": [{"舰": vanished, "删叶": True}]}]}),
                  encoding="utf-8")
    _, err = h.capture(["--start", str(ck0), "--apply", str(rm), "--round", "0", "--save", str(ck1)],
                       stderr=True)
    ck.check("删叶会留下回执（删的是叶，不是值）", "NOTE_APPLY_REMOVED" in err,
             "stderr 里有 NOTE_APPLY_REMOVED" if "NOTE_APPLY_REMOVED" in err else f"stderr={err[-200:]}")

    def orders(surface, faction):
        for f in surface["control"]:
            if f["势力"] == faction:
                return f["指令"]
        raise AssertionError(f"控制面里没有势力 {faction}")

    before = h.capture(["--start", str(ck1), "--control"])
    surface = json.loads(before)
    rows = [r["舰"] for r in orders(surface, fid)]
    ck.check("指令读面每舰一行且顺序与 state.ships 一致（含叶被删掉的那艘）", rows == ours,
             f"{fid}：读面 {len(rows)} 行 / 世界 {len(ours)} 艘" + ("" if rows == ours else f"，差异 {set(rows) ^ set(ours)}"))
    row = next((r for r in orders(surface, fid) if r["舰"] == vanished), None)
    ck.check("叶被删掉的舰：behavior 是 null、mode 是 Inherit",
             row == {"舰": vanished, "行为": None, "归属": "Inherit"}, f"{row}")
    others_ok = all(next(r for r in orders(surface, fid) if r["舰"] == s)["行为"] is not None
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
    row2 = next((r for r in orders(surface2, fid) if r["舰"] == vanished), None)
    state2 = json.loads(h.capture(["--start", str(ck2), "--round", "0"]).splitlines()[0])
    ck.check("回传不许偷偷把 null 变成 Idle（那一行还必须说「链上没人说话」）",
             row2 == {"舰": vanished, "行为": None, "归属": "Inherit"},
             f"{row2}")
    ck.check("回传不增删舰，且仍然每舰一行",
             [s["舰名"] for s in state2["ships"] if s["势力"] == fid] == ours
             and len(orders(surface2, fid)) == len(ours),
             f"{len(orders(surface2, fid))} 行 / {len(ours)} 艘")


def call_functions(h, ck, tmp: Path) -> None:
    """`--call <fn> --args <json>`：把引擎里**已经在用**的纯函数直接暴露给 Python 判据。

    这批把 Rust 侧「没有跑出来的数据可测」的类 B 纯函数搬成数据级：Python 拿真配置 + 真参数
    调**同一份实现**（`haul_split` / `hit_factor` / `home_defense_mult` / `ship_panel` /
    `cargo_capacity`）——不是抄公式，所以不会漂移。
    """

    def call(name, args=None, seed=42):
        argv = ["--seed", str(seed), "--call", name]
        if args is not None:
            argv += ["--args", json.dumps(args, ensure_ascii=False)]
        return json.loads(h.capture(argv))["value"]

    # ⓪ 货栈账（第 7 批，`autocontrol/freight.rs`）：`site_reserve` 的乘数是**往返回合数**，
    #    所以「离首都越远的站点该囤越多」可以用两个真天体比出方向、不写死数字
    #    （原 `src/tests/sim/site_supply.rs::the_reserve_grows_with_the_round_trip_time`）。
    facs = _rounds(_table(h.projection(42, 0), "factions"), 0)
    cap = next(r["capital_body"] for r in facs if r["势力"] == "中国")
    near, far = "水星", "金星"
    hops = {b: call("lane_rounds", {"from": b, "to": cap}) for b in (near, far)}
    res = {b: call("site_reserve", {"faction": "中国", "body": b}) for b in (near, far)}
    ck.check("--call lane_rounds：前提成立（金星那条线的一个往返更久，防空转）",
             hops[far] > hops[near], f"往返回合数：{near} {hops[near]} / {far} {hops[far]}（首都 {cap}）")
    ck.check("--call site_reserve：同一份建设活，离首都越远该囤的料越多",
             bool(res[near]) and bool(res[far]) and sum(res[far].values()) > sum(res[near].values()),
             f"保留量合计：{near} {sum(res[near].values()):.2f} / {far} {sum(res[far].values()):.2f}")

    # ⓪′ MOND 前沿（第 7 批）：`mond_frontier(config, control)` 是**纯函数**
    #     （`radius + arrival_eps/(drift_per_au × (1 − 掌握度))`），掌握到顶 = 无穷（JSON 给 null）。
    sweep = [(m, call("mond_frontier", {"control": m})) for m in (0.0, 0.25, 0.5, 0.75, 1.0)]
    finite = [(m, v) for m, v in sweep if v is not None]
    ck.check("--call mond_frontier：掌握度越高前沿越远（单调不减），到顶是无穷（null）",
             len(finite) == len(sweep) - 1 and sweep[-1][1] is None
             and all(b >= a - 1e-9 for (_, a), (_, b) in zip(finite, finite[1:])),
             f"扫描 {sweep}")

    # ① haul_split：max-min 公平分配（原 `src/tests/sim/haul.rs`）。
    ck.check("--call haul_split：三种货、舱容 6 ⇒ 每种 2",
             call("haul_split", {"need": {"铁": 10, "碳": 10, "硅": 10}, "room": 6})
             == {"铁": 2.0, "碳": 2.0, "硅": 2.0}, "逐值相等")
    ck.check("--call haul_split：铂只有 1 ⇒ 它拿 1、余量给另两种平摊",
             call("haul_split", {"need": {"铁": 10, "铂": 1, "碳": 10}, "room": 6})
             == {"铁": 2.5, "铂": 1.0, "碳": 2.5}, "逐值相等")
    ck.check("--call haul_split：舱容 ≥ 总存量 ⇒ 全装走",
             call("haul_split", {"need": {"铁": 1, "碳": 2}, "room": 100})
             == {"铁": 1.0, "碳": 2.0}, "逐值相等")
    ck.check("--call haul_split：空货栈 / 零舱容 ⇒ 空（不是 panic）",
             call("haul_split", {"need": {}, "room": 20}) == {}
             and call("haul_split", {"need": {"铁": 5}, "room": 0}) == {}, "两种边界都空")

    # ② hit_factor：目标越快、低追踪武器越难命中（原 `src/tests/sim/combat.rs`）。
    fast = call("hit_factor", {"tracking": 2.0, "target_speed": 2.6})
    slow = call("hit_factor", {"tracking": 2.0, "target_speed": 1.2})
    ck.check("--call hit_factor：目标越快命中折减越低、且折减在 (0,1]",
             fast < slow and 0.0 < fast <= 1.0, f"fast={fast:.4f} < slow={slow:.4f}")

    # ③ home_defense_mult：首都即强弩（原 `src/tests/sim/combat.rs`）。
    near = call("home_defense_mult", {"faction": "中国", "body": "地球"})
    far = call("home_defense_mult", {"faction": "中国", "pos": [80.0, 80.0]})
    ck.check("--call home_defense_mult：首都附近被削弱、远处为 1",
             near < 1.0 and far == 1.0, f"near={near} far={far}")

    # ④ ship_panel / cargo_capacity：面板公式与舰级舱容（原 combat.rs / haul.rs）。
    meta = json.loads(h.capture(["--meta"]))
    ships, comps = meta["ships"], meta["components"]
    spec = ships["corvette"]
    panel = call("ship_panel", {"class": "corvette",
                                "components": ["shield", "railgun", "ion_drive"]})
    want = {
        "hull_max": spec["hull"],
        "shield_max": comps["shield"]["shield"] * spec["shield_mult"],
        "attack": comps["railgun"]["damage"] * spec["attack_mult"],
        "attack_range": comps["railgun"]["range"] * spec["range_mult"],
        "speed": comps["ion_drive"]["speed"] * spec["speed_mult"],
        "accel": comps["ion_drive"]["accel"] * spec["accel_mult"],
        "hardness": 0.0,
    }
    bad = [f"{k}: {panel[k]} ≠ {v}" for k, v in want.items() if abs(panel[k] - v) > 1e-9]
    ck.check("--call ship_panel：船体=舰级直接属性、护盾/攻击/射程/速度=组件×舰级倍率",
             not bad and panel["upkeep"] > spec["upkeep"],
             "；".join(bad[:3]) or "逐项咬合，组件也抬高了维护费")

    pd = comps["point_defense"]["intercept"]
    intercept_bad = []
    for cls, s in ships.items():
        got = call("ship_panel", {"class": cls, "components": ["point_defense"]})["intercept"]
        if abs(got - pd * s["pd_mult"]) > 1e-9:
            intercept_bad.append(f"{cls}: {got} ≠ {pd}×{s['pd_mult']}")
    distinct = {round(s["pd_mult"], 6) for s in ships.values()}
    ck.check("--call ship_panel：intercept = 组件 × 舰级 pd_mult，且舰级之间确有差异",
             not intercept_bad and len(distinct) >= 2,
             "；".join(intercept_bad[:3]) or f"{len(ships)} 个舰级全咬合；pd_mult 取 {len(distinct)} 种")

    cc = ships["cruiser"]["cargo"]
    ck.check("--call cargo_capacity：舰级舱容 × hull/hull_max",
             abs(call("cargo_capacity", {"class": "cruiser", "hull": 6.0, "hull_max": 12.0}) - cc / 2) < 1e-9,
             f"半血巡洋舰 = {cc / 2}")
    ck.check("--call cargo_capacity：壳打光 ⇒ 0、旧档 hull_max≤0 ⇒ 满舱",
             call("cargo_capacity", {"class": "cruiser", "hull": 0.0, "hull_max": 12.0}) == 0.0
             and abs(call("cargo_capacity", {"class": "cruiser", "hull": 6.0, "hull_max": 0.0}) - cc) < 1e-9,
             "两个边界都对")


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
    world_shape(h, ck, tmp)
    mond_start_checks(h, ck, tmp)
    neutral_defaults(h, ck, tmp)
    control_fixed_point(h, ck, tmp)
    call_functions(h, ck, tmp)
    neutral_paths(h, ck, tmp)


if __name__ == "__main__":
    sys.exit(group_main("g1_contract", run))
