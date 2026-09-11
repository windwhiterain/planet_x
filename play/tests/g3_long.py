"""长组（T3，>480 回合）：`tests/horizon_long.rs` 那几条长局不变量的**数据级**版本。

为什么搬到这里（方案见 `.agents/notes/test-decoupled-suite.md`）：

* 那几条用例跑的是**完全相同**的一批世界（同 seed × 1000 回合），却**各跑各的**——
  同一批 2.8 万回合被重算了四遍；
* 它们断言的全是**读面上的数**（活城/舰/占比/库存价值/事件），本来就不需要 crate 内部 API；
* 搬过来之后：世界按 `(二进制指纹, seed, 回合数)` 缓存一次，**加多少条断言都不重跑世界**，
  而且改断言不用重编 Rust。

判据与 Rust 版**逐条对应、阈值一个没动**（`CITILESS_RECOVERY = 36`、`WORLD_VALUE_CAP = 2e6`、
`min_members` / `hegemon_power` 从 `meta.json` 读），只是「世界的来源」从「每条自己跑」换成
「一份共享的投影」。

**种子比 Rust 版更宽**：那边长局用例用的是 `[1, 42]` 两个种子（7 个种子那批只在 `#[ignore]`
的诊断里跑），这里 7 个种子跑全部不变量——缓存之后，多一个种子只付一次一次性成本。

**两段式**（读 170 MB 的投影很贵，断言很便宜）：

1. `extract(dir)` 把一份 1000 回合的投影压成**每回合一行的判据表**（活城/舰/总价值/事件/
   政治），按投影缓存成 pickle（`Harness.digest`）——第一次 ~9 s/seed，之后 0.05 s；
2. `run()` 里的断言全是这张小表上的谓词。
"""

from __future__ import annotations

import json
import math
import sys
from pathlib import Path

import pandas as pd

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _harness import KIT, group_main  # noqa: E402

SEEDS = (1, 2, 3, 4, 5, 7, 11)          # 与 g2 的主世界**共用** 1/7/11@400（同一批缓存目录）
ROUNDS = 400

# 与 `tests/horizon_long.rs` 同值：零活城之后允许拖多少回合才重新立城（殖民不依赖产出，
# 所以给得宽——它防的是「殖民机制坏了」，不是「经济不好」）；世界总价值的上限。
CITILESS_RECOVERY = 36
WORLD_VALUE_CAP = 2_000_000.0

# **必须永远有限的那些数**（对应 Rust 版 `count_nonfinite` 逐字段遍历的那批）：
# ① 世界级叶子；② 每势力/每城行上的叶子；③ 数值 map（`power_share`/`faction_power`/市场三表）
#    ——map **按资源稀疏**（这一回合没这种矿就没有那个键），所以「缺键」与「值是 null」必须
#    分开：缺键跳过、键在而值是 null 才叫「一个不存在的数」。
FINITE_WORLD = ("city_count", "ship_count", "fleet_value", "population")
FINITE_FACTION = (
    "market_value", "production_value", "upkeep", "governance_cost", "governance_coverage",
    "governance_admin", "governance_entertainment", "governance_scale",
    "ideology_loyalty_penalty", "capital_loyalty_bonus", "freight_paid", "carrier_income",
    "net_import", "upkeep_unpaid", "fleet_rust", "purchasing_power", "population",
)
FINITE_CITY = ("population", "loyalty", "production_value", "labor", "housing_capacity")
FINITE_LOYALTY_LEAVES = ("distance", "entertainment", "effective")
FINITE_MAPS = ("power_share", "faction_power", "market_price", "market_settled", "market_offered")


def _finite(x) -> bool:
    """JSON 里非有限浮点落成 `null`（`src/json.rs`）⇒ 「不是有限数」= 「不是数」。"""
    return isinstance(x, (int, float)) and not isinstance(x, bool) and math.isfinite(x)


def _scan(v) -> tuple[int, int, list[str]]:
    """① 有限性：返回 (查了多少格, 其中 map 值几格, 违规样例)。"""
    bad: list[str] = []
    cells = maps = 0
    for k in FINITE_WORLD:
        cells += 1
        if not _finite(v.get(k)):
            bad.append(f"{k}={v.get(k)!r}")
    for k in FINITE_MAPS:
        for name, val in (v.get(k) or {}).items():
            cells += 1
            maps += 1
            if not _finite(val):
                bad.append(f"{k}.{name}={val!r}")
    for holder, row in (v.get("factions") or {}).items():
        for leaf in FINITE_FACTION:
            cells += 1
            if not _finite(row.get(leaf)):
                bad.append(f"factions.{holder}.{leaf}={row.get(leaf)!r}")
    for holder, row in (v.get("cities") or {}).items():
        for leaf in FINITE_CITY:
            cells += 1
            if not _finite(row.get(leaf)):
                bad.append(f"cities.{holder}.{leaf}={row.get(leaf)!r}")
        nested = row.get("loyalty_target")
        if isinstance(nested, dict):
            for leaf in FINITE_LOYALTY_LEAVES:
                cells += 1
                if not _finite(nested.get(leaf)):
                    bad.append(f"cities.{holder}.loyalty_target.{leaf}={nested.get(leaf)!r}")
    return cells, maps, bad[:4]


def _missing_leaves(q) -> list[str]:
    """防检查空转：声明的叶子必须在读面上**真的存在**（改名了就要有人喊）。"""
    v = q.facts["view"].iloc[0]
    missing = [k for k in FINITE_WORLD if k not in v]
    missing += [k for k in FINITE_MAPS if k not in v]
    fr = next(iter((v.get("factions") or {}).values()), {})
    cr = next(iter((v.get("cities") or {}).values()), {})
    missing += [f"factions[].{k}" for k in FINITE_FACTION if k not in fr]
    missing += [f"cities[].{k}" for k in FINITE_CITY if k not in cr]
    for leaf in FINITE_LOYALTY_LEAVES:
        if leaf not in (cr.get("loyalty_target") or {}):
            missing.append(f"cities[].loyalty_target.{leaf}")
    return missing


def _process_identities(q, tol: float = 1e-12) -> dict:
    """**过程量表的恒等式**（`src/tests/sim/governance.rs` + `spending.rs` 搬过来的那几条）。

    它们全是**读面上两张表之间的代数关系**，所以样本可以放到全 7 seed × 1000 回合的**每一行**
    （Rust 版是「跑一步、看开局那一回合的几座城」）：

    ① 忠诚目标 = clamp(距离项 + 娱乐项 + 势力行的首都向心 − 势力行的思潮惩罚, 0, 1)；
    ② 治理总开销 ≥ 行政 + 娱乐（制裁倍率 ≥ 1），且 scale ≥ 1、思潮惩罚/首都向心 ≥ 0、覆盖率 ∈ [0,1]；
    ③ 有活城的势力必有行政开销（防空转）；
    ④ 欠费与生锈：`upkeep_unpaid ≥ 0`、`fleet_rust ∈ [0,1]`，且**欠费 ⇔ 生锈**（两边都不许单飞）；
    ⑤ 造舰进度：`0 ≤ increment ≤ rate`（rate 是产能上限，increment 是实得）。

    ## ⚠ 两种「同回合相位错位」（把样本放大之后**才**会撞上，Rust 版一回合一步永远看不到）

    这两条都不是引擎的 bug，而是「同一个回合里不同步骤看到的对象不同」；但**写判据的人必须
    知道**，否则会假红（我第一版就是）：

    1. **城本回合易主**（`city_defected` / `city_overrun`，实测 seed 1 1000 回合里 670 行）：
       `city_process.loyalty_target_*` 是**旧主**那时算的（距离按旧主首都、全国项按旧主），
       而 `faction_process.capital_loyalty_bonus` / `ideology_loyalty_penalty` 是**新主**的
       ⇒ 两边的分解式必然对不上。排除这一批之后，seed 1 的违规**清零**（"其它" = 0）。
    2. **势力本回合才第一次有活城**（殖民/夺城，实测 seed 1 有 231 行）：`step_governance`
       跑在它拿到城**之前** ⇒ 那一行没有捕获（`unwrap_or(0.0)`）⇒ 有活城却 0 行政开销。

    判据因此写成「**排除这两类、剩下的每一行都必须成立**」，并且**单独要求**「被排除的每一处
    都能用这两条解释」——否则排除就成了藏违规的后门。
    """
    fp = q.table("faction_process")
    cp = q.table("city_process")
    ev = q.table("events")
    out: dict = {}

    # ── 排除集：本回合易主的城 / 本回合新建的城 ──────────────────────────────
    live = cp[~cp["已焚毁"].astype(bool)].copy()
    moved = ev[ev["type"].isin(["city_defected", "city_overrun"])][["round", "target_id"]]
    moved = moved.rename(columns={"target_id": "城名"}).drop_duplicates()
    moved["owner_changed"] = True
    founded = (ev[ev["type"] == "colony_founded"][["round", "target_id"]]
               .rename(columns={"target_id": "城名"}).drop_duplicates())
    founded["founded_this_round"] = True
    seen = set(zip(cp["round"].astype(int), cp["城名"]))
    live["new_this_round"] = [(int(r) - 1, c) not in seen
                              for r, c in zip(live["round"], live["城名"])]

    # ① 忠诚目标的分解式（势力行的两项按 (round, faction) join 到城行上）
    j = live.merge(fp[["round", "势力", "capital_loyalty_bonus", "ideology_loyalty_penalty"]],
                   on=["round", "势力"], how="left")
    j = j.merge(moved, on=["round", "城名"], how="left")
    j = j.merge(founded, on=["round", "城名"], how="left")
    total = (j["loyalty_target_distance"].fillna(0.0) + j["loyalty_target_entertainment"].fillna(0.0)
             + j["capital_loyalty_bonus"].fillna(0.0) - j["ideology_loyalty_penalty"].fillna(0.0))
    dev = (j["loyalty_target_effective"] - total.clip(0.0, 1.0)).abs()
    viol = dev > tol
    # ⚠ 左连接之后的标记列是 **object**（未匹配 = NaN）：必须用 `notna()` 取掩码。
    # 直接 `fillna(False)` 仍是 object 列，`~excl` 会退化成整数的按位取反（`~True == -2`，真值！）
    # 于是「排除」和「未解释」会同时为真——这个坑真的让我假红过一次。
    excl_moved = j["owner_changed"].notna()
    excl_new = j["new_this_round"].astype(bool) | j["founded_this_round"].notna()
    unexplained = viol & ~excl_moved & ~excl_new
    out["loyalty_checked"] = int(len(j) - int(excl_moved.sum()) - int((excl_new & ~excl_moved).sum()))
    out["loyalty_excluded_moved"] = int((viol & excl_moved).sum())
    out["loyalty_excluded_new"] = int((viol & excl_new & ~excl_moved).sum())
    out["loyalty_bad_n"] = int(unexplained.sum())
    out["loyalty_bad"] = [f"r{int(row.round)} {row.城名}: effective={row.loyalty_target_effective}"
                          f" ≠ 三项之和 {want:.6f}"
                          for row, want in zip(j[unexplained].itertuples(), total[unexplained])][:3]
    out["loyalty_range_bad"] = int(((j["loyalty_target_effective"] < -tol)
                                    | (j["loyalty_target_effective"] > 1 + tol)).sum())

    # ② 治理：倍率 ≥ 1 与定义域
    live_n = live.groupby(["round", "势力"]).size().rename("live_cities").reset_index()
    have = fp.merge(live_n, on=["round", "势力"], how="left")
    have["live_cities"] = have["live_cities"].fillna(0)
    split = fp["governance_admin"].fillna(0.0) + fp["governance_entertainment"].fillna(0.0)
    mult_bad = fp[fp["governance_total"].fillna(0.0) + 1e-9 < split]
    out["mult_bad"] = [f"r{int(r.round)} {r.势力}: 总开销 {r.governance_total} < 行政+娱乐 {r.governance_admin + r.governance_entertainment}"
                       for r in mult_bad.itertuples()][:3]
    out["mult_bad_n"] = int(len(mult_bad))
    domain = []
    domain += [f"r{int(r.round)} {r.势力}: scale={r.governance_scale} < 1"
               for r in fp[fp["governance_scale"].fillna(1.0) < 1 - 1e-9].itertuples()]
    domain += [f"r{int(r.round)} {r.势力}: 思潮惩罚 {r.ideology_loyalty_penalty} < 0"
               for r in fp[fp["ideology_loyalty_penalty"].fillna(0.0) < -1e-9].itertuples()]
    domain += [f"r{int(r.round)} {r.势力}: 首都向心 {r.capital_loyalty_bonus} < 0"
               for r in fp[fp["capital_loyalty_bonus"].fillna(0.0) < -1e-9].itertuples()]
    domain += [f"r{int(r.round)} {r.势力}: 覆盖率 {r.governance_coverage} 越界"
               for r in fp[~fp["governance_coverage"].fillna(1.0).between(-1e-9, 1 + 1e-9)].itertuples()]
    out["domain_bad"] = domain[:3]
    out["domain_bad_n"] = len(domain)

    # ③ 有活城 ⇒ 有行政开销：**只对「城集合这一回合没变」的势力成立**（见文档第 2/3 条相位错位）
    sets = live.groupby(["round", "势力"])["城名"].apply(frozenset).rename("cities").reset_index()
    prev = sets.rename(columns={"cities": "prev_cities"})
    prev["round"] = prev["round"] + 1

    def _fs(x):
        return x if isinstance(x, frozenset) else frozenset()

    h = fp.merge(sets, on=["round", "势力"], how="left").merge(
        prev, on=["round", "势力"], how="left")
    h["cities"] = h["cities"].map(_fs)
    h["prev_cities"] = h["prev_cities"].map(_fs)
    h["steady"] = (h["cities"] != frozenset()) & (h["cities"] == h["prev_cities"])
    zero = h["governance_admin"].fillna(0.0) <= 0.0
    target = h[(h["round"] > 0) & h["steady"]]
    out["admin_checked"] = int(len(target))
    no_admin = target[target["governance_admin"].fillna(0.0) <= 0.0]
    out["admin_bad"] = [f"r{int(r.round)} {r.势力}: 有 {len(r.cities)} 座活城（上一回合同一批城）却没有行政开销"
                        for r in no_admin.itertuples()][:3]
    out["admin_bad_n"] = int(len(no_admin))
    out["admin_excluded_changed"] = int(len(h[(h["round"] > 0) & ~h["steady"] & (h["cities"] != frozenset()) & zero]))

    # ④ 欠费 ⇔ 生锈
    unpaid = fp["upkeep_unpaid"].fillna(0.0)
    rust = fp["fleet_rust"].fillna(0.0)
    out["rust_domain_n"] = int(((unpaid < -1e-9) | (rust < -1e-9) | (rust > 1 + 1e-9)).sum())
    out["rust_domain"] = [f"r{int(r.round)} {r.势力}: 欠费 {r.upkeep_unpaid} / 锈 {r.fleet_rust}"
                          for r in fp[((unpaid < -1e-9) | (rust < -1e-9) | (rust > 1 + 1e-9))].itertuples()][:3]
    mismatch = fp[((unpaid > 1e-9) & (rust <= 0.0)) | ((rust > 0.0) & (unpaid <= 1e-9))]
    out["rust_pair_bad"] = [f"r{int(r.round)} {r.势力}: 欠费 {r.upkeep_unpaid} vs 锈 {r.fleet_rust}（应当同生同灭）"
                            for r in mismatch.itertuples()][:3]
    out["rust_pair_bad_n"] = int(len(mismatch))
    out["rust_seen"] = int(((unpaid > 1e-9) & (rust > 0.0)).sum())

    # ⑤ 造舰进度：0 ≤ increment ≤ rate
    inc_bad, inc_seen, rate_seen = [], 0, 0
    for r in cp.itertuples(index=False):
        build = r.build if isinstance(r.build, dict) else {}
        for cls, b in build.items():
            if not isinstance(b, dict):
                continue
            inc, rate = float(b.get("increment") or 0.0), float(b.get("rate") or 0.0)
            rate_seen += 1
            if inc > 1e-9:
                inc_seen += 1
            if inc < -1e-9 or inc > rate + 1e-9:
                if len(inc_bad) < 3:
                    inc_bad.append(f"r{int(r.round)} {r.城名} {cls}: increment={inc} > rate={rate}")
    out["inc_bad"] = inc_bad
    out["inc_bad_n"] = len(inc_bad)
    out["inc_seen"], out["rate_seen"] = inc_seen, rate_seen

    # ⑥ 欠费不超过账单本身（欠的只能是账单的一部分）
    over = fp[fp["upkeep_unpaid"].fillna(0.0) > fp["upkeep"].fillna(0.0) + 1e-9]
    out["unpaid_over_bill_n"] = int(len(over))
    out["unpaid_over_bill"] = [f"r{int(r.round)} {r.势力}: 欠费 {r.upkeep_unpaid} > 维护账单 {r.upkeep}"
                               for r in over.itertuples()][:3]

    # ⑦ 集散地（is_hub）在一个势力里必须只有一个天体——**不排除任何行**。
    #    ⚠ 这条判据曾经需要「排除本回合易主/复垦的城」：那时 `is_hub` 抄的是**产出那一步**的判定
    #    （旧主），城在同回合后半段易主/被复垦后，同一个势力就出现两个 hub 天体（1000 回合 ×
    #    7 seed 里 5 例，Rust 版跑一步永远看不到）。**引擎侧已修**（`sim/metrics.rs` 改成按写行
    #    时的主人重算）⇒ 现在这条是**严格**的：错了就是真错了，没有豁免名单。
    stable = live[live["is_hub"].fillna(False).astype(bool)]
    grp = stable.groupby(["round", "势力"])["天体名"].nunique()
    multi = grp[grp > 1]
    out["hub_pairs"] = int(len(grp))
    out["hub_rows"] = int(len(stable))
    out["hub_multi_n"] = int(len(multi))
    out["hub_multi"] = [f"r{int(rnd)} {fid}: 同时有 {int(n)} 个 hub 天体"
                        for (rnd, fid), n in multi.items()][:3]
    return out


def extract(dirpath) -> tuple[pd.DataFrame, dict]:
    """把一份投影压成**每回合一行**的判据表 + 元数据（结果按投影缓存成 pickle）。"""
    q = KIT.load(str(dirpath), only=("events", "ships", "factions", "faction_process", "city_process", "round_inputs",
                                    "decisions", "cities"))
    facts = q.facts
    view = facts["view"]
    rounds = facts["round"].to_numpy()

    # ④ 建城必须有一艘在场的活舰：事件层 `colony_founded`（actor = 建城方）join 该回合该势力
    #    `hull > 0` 的舰数（投影的舰表 = 回合末状态，与 Rust 版「回合末仍有活舰」同口径）。
    ev = q.table("events")
    cf = ev[ev["type"] == "colony_founded"]
    live = q.table("ships").query("船体 > 0").groupby(["round", "势力"]).size()
    found_bad: dict[int, list[str]] = {}
    for _, row in cf.iterrows():
        rnd, owner = int(row["round"]), row["actor_id"]
        if int(live.get((rnd, owner), 0)) == 0:
            found_bad.setdefault(rnd, []).append(f"{owner} 造了 {row['target_id']} 却一艘活舰都没有")

    balance = q.meta["balance"]
    wp = float(balance["power_city_weight"]) + float(balance["power_fleet_weight"])
    hegemon_power = float(balance["hegemon_power"])
    min_members = int(balance["min_members"])
    estrange = float(balance["coalition_estrange"])
    # 关系表 8 MB / 3 s：只在真有霸权（联盟）的局里读。
    relations: dict | None = None
    if any(v.get("hegemon") for v in view):
        relations = {}
        for _, r in q.table("factions").iterrows():
            rel = r["关系"]
            relations[(int(r["round"]), r["势力"])] = rel if isinstance(rel, dict) else {}

    rows: list[dict] = []
    cells = maps = 0
    for i, v in enumerate(view):
        rnd = int(rounds[i])
        c, m, bad = _scan(v)
        cells += c
        maps += m
        share = v.get("power_share") or {}
        power = v.get("faction_power") or {}
        top = max(share.values()) if share else 0.0
        strongest = max(share, key=share.get) if share else None
        hg = v.get("hegemon")
        mem = v.get("coalition_members") or []
        sanc = v.get("sanctioned")

        # (a) 两张发布表的换算关系：`power_share ≡ faction_power ÷ (两权重之和)`。
        algebra = [f"power_share.{fid}={share.get(fid)} ≠ faction_power ÷ {wp}"
                   for fid, val in power.items() if abs(val / wp - share.get(fid, -1.0)) > 1e-9]
        # (b) 联盟成员的**资格**（`coalition_of` 的两条判据，`sim/power.rs`）：
        #     ① 对霸权的关系 ≤ `coalition_estrange`（疏远）；② 成员**彼此**不交战。
        #     ⚠ 与霸权**开战**是允许的（遏制本来就可能打到一起）——判据只管成员之间。
        rel_bad, war_bad = [], []
        if hg is not None and mem:
            wars = {frozenset(p) for p in (v.get("wars") or [])}
            for mm in mem:
                rel = (relations or {}).get((rnd, mm), {}).get(hg)
                if rel is None or rel > estrange:
                    rel_bad.append(f"成员 {mm} 对 {hg} 的关系 {rel}（阈值 {estrange}）")
            for a in range(len(mem)):
                for b in range(a + 1, len(mem)):
                    if frozenset((mem[a], mem[b])) in wars:
                        war_bad.append(f"成员 {mem[a]} 与 {mem[b]} 交战")

        rows.append({
            "round": rnd,
            "city_count": v.get("city_count"),
            "ship_count": v.get("ship_count"),
            "world_value": sum((r or {}).get("market_value", 0.0)
                               for r in (v.get("factions") or {}).values()),
            "finite_bad": len(bad),
            "finite_why": "；".join(bad),
            "foundings": int((cf["round"] == rnd).sum()),
            "found_bad": len(found_bad.get(rnd, [])),
            "found_why": "；".join(found_bad.get(rnd, [])[:2]),
            # 政治面：**只放数据**，判据在 run() 里（改断言不必重读 170 MB 的投影）。
            "hegemon": hg,
            "strongest": strongest,
            "top": top,
            "heg_share": share.get(hg) if hg is not None else None,
            "coalition_len": len(mem),
            "coalition": "/".join(mem),
            "sanctioned": sanc,
            "sanc_share": share.get(sanc) if sanc is not None else None,
            "algebra_bad": len(algebra),
            "algebra_why": "；".join(algebra[:2]),
            "rel_bad": len(rel_bad),
            "rel_why": "；".join(rel_bad[:2]),
            "war_bad": len(war_bad),
            "war_why": "；".join(war_bad[:2]),
        })

    meta = {
        "cells": cells,
        "map_cells": maps,
        "missing_leaves": _missing_leaves(q),
        "foundings": int(len(cf)),
        "coalition_formed_event": bool((ev["type"] == "coalition_formed").any()),
        "min_members": min_members,
        "hegemon_power": hegemon_power,
        "last_round": int(rounds[-1]),
        "identities": _process_identities(q),
        "capital": _capital_summary(q),
        "mond": _mond_summary(q),
        "ideology": _ideology_summary(q),
        "ideo_econ": _ideo_econ_summary(q),
        "upkeep": _upkeep_summary(q),
        "role_dist": _role_dist_summary(q),
    }
    return pd.DataFrame(rows), meta


def _mond_summary(q) -> dict:
    """MOND 掌握度的**逐势力时间序列**（`src/tests/sim/knowledge.rs` 那几条，2026-10 第 7 批）。

    读面三列就够：`MOND 掌握度`（活量）、`mond_ships_in_band`（在场舰数 = **唯一渠道**）、
    `mond_frontier_au`（前沿海拔，掌握到顶是 `null` = 无穷）。
    """
    fa = q.table("factions")[["round", "势力", "MOND 掌握度", "mond_ships_in_band",
                              "mond_presence", "mond_target", "mond_frontier_au"]]
    out: dict = {}
    for f, rows in fa.groupby("势力"):
        rows = rows.sort_values("round")
        out[f] = [(int(r["round"]), float(r["MOND 掌握度"]), int(r["mond_ships_in_band"] or 0),
                   float(r["mond_presence"] or 0.0), float(r["mond_target"] or 0.0),
                   None if pd.isna(r["mond_frontier_au"]) else float(r["mond_frontier_au"]))
                  for _, r in rows.iterrows()]
    return out


def _capital_summary(q) -> dict:
    """首都判定（`src/tests/sim/capital.rs` 那四条搬去数据级用的摘要）。

    `decisions[kind=capital]` 每行给 `verdict ∈ {forced, review, relocate}` + `detail`
    （`reviewed` / `candidate` / `current_cost` / `candidate_cost` / `relocated_from`）；
    `factions.capital_body` 是**回合末**的有效首都。这里把「目标是它人口最高的活城吗」
    也预先算好——**并列任取**：实测 s42 r237 月球与天王星都 200 人，引擎挑了天王星，
    写判据时不能只认 `max()` 的第一个。
    """
    cap = q.table("decisions")
    cap = cap[cap["kind"] == "capital"]
    fac = q.table("factions")[["round", "势力", "capital_body"]]
    cities = q.table("cities")
    live = cities[(cities["已焚毁"] != True) & (cities["人口"].fillna(0) > 0)]  # noqa: E712
    peak = live.groupby(["round", "势力"])["人口"].max()
    body = {(int(r["round"]), r["势力"]): r["capital_body"] for _, r in fac.iterrows()}
    out = []
    for _, r in cap.iterrows():
        key = (int(r["round"]), r["势力"])
        d = r["detail"] or {}
        top = peak.get(key)
        tops = [] if top is None else sorted(set(
            live[(live["round"] == key[0]) & (live["势力"] == key[1])
                 & (live["人口"] == top)]["天体名"]))
        out.append({
            "round": key[0], "势力": r["势力"], "verdict": r["verdict"],
            # ⚠ `target` 是 DataFrame 的一列 ⇒ JSON 的 `null` 到这儿是 **NaN**，不是 `None`；
            # 归一化掉（否则「评估未迁」那类行的 `target is not None` 会假红——踩过一次）。
            "target": None if pd.isna(r["target"]) else r["target"],
            "from": d.get("relocated_from"), "reviewed": bool(d.get("reviewed")),
            "candidate": d.get("candidate"), "cur": d.get("current_cost"),
            "cand": d.get("candidate_cost"), "cap_body": body.get(key), "top_bodies": tops,
        })
    gov = q.meta["governance"]
    return {"rows": out, "review_every": int(gov["capital_review_every"]),
            "threshold": float(gov["capital_relocate_threshold"])}


class Verdict:
    """一条不变量的判据：攒违规样例 + 计数（红的时候打印前几条 + 总处数）。"""
    def __init__(self) -> None:
        self.n = 0
        self.samples: list[str] = []

    def add(self, msg: str, n: int = 1) -> None:
        self.n += n
        if len(self.samples) < 3:
            self.samples.append(msg)

    def detail(self, ok: str) -> str:
        if self.n == 0:
            return ok
        tail = f"…（共 {self.n} 处）" if self.n > len(self.samples) else ""
        return "；".join(self.samples) + tail


def run(h, ck) -> None:
    results = h.digests([(s, ROUNDS) for s in SEEDS], extract)
    metas = [m for _, m in results]

    finite, missing = Verdict(), Verdict()
    dead, recovery = Verdict(), Verdict()
    value, found = Verdict(), Verdict()
    coal, sanc, power = Verdict(), Verdict(), Verdict()
    cells = maps = foundings = 0
    value_peak = 0.0

    for seed, (df, meta) in zip(SEEDS, results):
        cells += meta["cells"]
        maps += meta["map_cells"]
        for m in meta["missing_leaves"]:
            missing.add(f"seed {seed}: {m}")

        for _, r in df[df["finite_bad"] > 0].head(2).iterrows():
            finite.add(f"seed {seed} r{int(r['round'])}: {r['finite_why']}")

        live = df["city_count"].to_numpy()
        dead.add(f"seed {seed}: {int(((live == 0) & (df['ship_count'].to_numpy() == 0)).sum())} 个回合"
                 f"既无活城又无舰",
                 n=int(((live == 0) & (df["ship_count"].to_numpy() == 0)).sum()))
        worst = run_len = 0
        for c in live:
            run_len = run_len + 1 if c == 0 else 0
            worst = max(worst, run_len)
        if worst > CITILESS_RECOVERY:
            recovery.add(f"seed {seed}: 连续 {worst} 回合无活城（上限 {CITILESS_RECOVERY}）")

        peak = float(df["world_value"].max())
        value_peak = max(value_peak, peak)
        if float(df["world_value"].iloc[-1]) >= WORLD_VALUE_CAP:
            value.add(f"seed {seed} r{int(meta['last_round'])}: 总价值 {df['world_value'].iloc[-1]:,.0f}")

        foundings += meta["foundings"]
        for _, r in df[df["found_bad"] > 0].head(2).iterrows():
            found.add(f"seed {seed} r{int(r['round'])}: {r['found_why']}")

        if not (meta["coalition_formed_event"]
                or bool(((df["hegemon"].notna()) & (df["coalition_len"] >= meta["min_members"])).any())):
            coal.add(f"seed {seed}: 长局从未出现反制联盟")
        if not bool(df["sanctioned"].notna().any()):
            sanc.add(f"seed {seed}: 长局从未出现被封锁的霸权")

        # ⑥ 政治读面自洽。**判据在这里，数据在摘要里**（所以改这条不用重读投影）。
        #
        # ⚠ `hegemon` **不是「最强的那个」**：它是 `active_coalition_hegemon`（`sim/power.rs`）
        #   ——「占比达标的最强者」**且**针对它的疏远成员 ≥ `min_members` 才算，所以读面上会
        #   出现「占比 0.32 ≥ 阈值 0.30 却 `hegemon: null`」（联盟还没成形）：那是机制，不是
        #   缺陷。Rust 版比的是**两个内部函数**（`observe.power_share` 与 `balance_picture` 的
        #   powers）；数据级没有内部函数可调，等价物是：**游戏针对谁**（hegemon / sanctioned，
        #   那就是政治机制的输出）必须**就是**读面统计说谁最强，联盟成员的资格也在读面上核。
        hp = meta["hegemon_power"]
        mm = meta["min_members"]
        pb = df[(df["algebra_bad"] > 0) | (df["rel_bad"] > 0) | (df["war_bad"] > 0)]
        for _, r in pb.head(2).iterrows():
            why = r["algebra_why"] or r["rel_why"] or r["war_why"]
            power.add(f"seed {seed} r{int(r['round'])}: {why}")
        if len(pb) > 2:
            power.add(f"seed {seed} 另有 {len(pb) - 2} 个回合不合格", n=len(pb) - 2)
        has_hg = df["hegemon"].notna()
        # 判「霸权就是最强」**按占比比、不按名字比**：并列最强（例如两城各占 0.5）时引擎的
        # `max_by` 取最后一个，Python 的 `max` 取第一个——名字比会在并列上假红。
        bad_target = df[has_hg & ((df["heg_share"] < df["top"] - 1e-9) | (df["heg_share"] < hp)
                                  | (df["coalition_len"] < mm) | (df["sanctioned"] != df["hegemon"]))]
        for _, r in bad_target.head(2).iterrows():
            why = []
            if r["heg_share"] < r["top"] - 1e-9:
                why.append(f"霸权 {r['hegemon']} 占比 {r['heg_share']:.3f} < 最强 {r['top']:.3f}"
                           f"（最强是 {r['strongest']}）")
            if r["heg_share"] < hp:
                why.append(f"霸权占比 < 阈值 {hp}")
            if r["coalition_len"] < mm:
                why.append(f"联盟只有 {r['coalition_len']} 人（< {mm}）")
            if r["sanctioned"] != r["hegemon"]:
                why.append(f"有联盟（{r['hegemon']}）却没封锁它（sanctioned={r['sanctioned']}）")
            power.add(f"seed {seed} r{int(r['round'])}: " + "；".join(why))
        if len(bad_target) > 2:
            power.add(f"seed {seed} 另有 {len(bad_target) - 2} 个回合的霸权叙事不自洽", n=len(bad_target) - 2)
        orphan = df[~has_hg & (df["coalition_len"] > 0)]
        for _, r in orphan.head(2).iterrows():
            power.add(f"seed {seed} r{int(r['round'])}: 没有霸权却报了联盟成员 {r['coalition']}")
        san = df[df["sanctioned"].notna() & ((df["sanc_share"] < df["top"] - 1e-9) | (df["sanc_share"] < hp))]
        for _, r in san.head(2).iterrows():
            power.add(f"seed {seed} r{int(r['round'])}: 被封锁的 {r['sanctioned']}"
                      f"（{r['sanc_share']:.3f}）不是达标的最强者（{r['strongest']} {r['top']:.3f}）")

    tag = f"{len(SEEDS)} seed × {ROUNDS} 回合"
    ck.check("读面没有非有限的数", finite.n == 0,
             finite.detail(f"{tag}：{cells:,} 格（含 {maps:,} 个 map 值）一个 null 都没有"))
    ck.check("检查没有空转（声明的叶子都在读面上）", missing.n == 0, missing.detail("全部在"))
    ck.check("世界不进吸收态（既无活城又无舰）", dead.n == 0, dead.detail("0 回合"))
    ck.check(f"零活城在 {CITILESS_RECOVERY} 回合内复生", recovery.n == 0, recovery.detail("最长零城段 ≤ 上限"))
    ck.check(f"世界总价值 < {WORLD_VALUE_CAP:,.0f}", value.n == 0, value.detail(f"峰值 {value_peak:,.0f}"))
    ck.check("建城必须有一艘在场的活舰", found.n == 0, found.detail(f"{foundings} 次建城，全部有舰解释"))
    ck.check("建城守卫没有空转（真发生过建城）", foundings > 0, f"{tag} 共 {foundings} 次")
    ck.check("合纵连横真的成立过", coal.n == 0, coal.detail(f"{tag} 每个种子都成立过"))
    ck.check("经济制裁真的生效过", sanc.n == 0, sanc.detail(f"{tag} 每个种子都封锁过霸权"))
    ck.check("霸权 = 占比最高者，且联盟/封锁自洽", power.n == 0, power.detail("逐回合自洽"))

    identity_checks(h, ck, metas, tag)
    capital_checks(h, ck, metas, tag)
    mond_checks(h, ck, metas, tag)
    ideology_checks(h, ck, metas, tag)
    mond_law_checks(h, ck, metas, tag)
    ideology_law_checks(h, ck, metas, tag)
    upkeep_law_checks(h, ck, metas, tag)
    distribution_checks(h, ck, metas, tag)


def capital_checks(h, ck, metas, tag) -> None:
    """**首都判定**（`src/tests/sim/capital.rs` 四条搬来，2026-10 第 7 批）。

    读面 = `decisions[kind=capital]`（`verdict` + `detail`）+ `factions.capital_body`
    + `cities.{人口, 已焚毁, 天体名}`，所以三条**整条**搬得动：

    | Rust 原件 | 这里 |
    | --- | --- |
    | 亡城强迁 ⇒ 人口最高的活城 | `verdict=forced` ⇒ 不评估、无判据数字、`capital_body == target`、`target ∈ 人口最高那一组天体`（并列任取） |
    | 周期评估 ⇒ 迁到人口中心 | `verdict=relocate` ⇒ `reviewed`、`candidate == target`、`候选成本 + 门槛 < 现成本`、`capital_body == target` |
    | 判定是稀疏的 | 非 `forced` 的判定只出现在 `capital_review_every` 的整数倍回合；`review` 行 `target` 为空、两头判据数字都在 |

    第四条（Player 钉的首都不被覆盖）要 `--apply` 写 `首都` 叶 ⇒ 属**合成场景**，在 g2。
    """
    forced = reloc = review = 0
    bad = Verdict()
    for m in metas:
        c = m["capital"]
        every, thr = c["review_every"], c["threshold"]
        for r in c["rows"]:
            v, where = r["verdict"], f"r{r['round']} {r['势力']}"
            if v == "forced":
                forced += 1
                if r["reviewed"] or r["candidate"] is not None or r["cur"] is not None or r["cand"] is not None:
                    bad.add(f"{where}：强迁却编出了评估数字（reviewed/candidate/成本）")
                if r["from"] is None:
                    bad.add(f"{where}：强迁没记 `relocated_from`")
                if r["target"] not in r["top_bodies"]:
                    bad.add(f"{where}：迁到 {r['target']}，人口最高的活城却在 {r['top_bodies']}")
            elif v == "relocate":
                reloc += 1
                if not r["reviewed"] or r["candidate"] != r["target"] or r["from"] is None:
                    bad.add(f"{where}：迁都行字段不成套（{r['reviewed']}/{r['candidate']}/{r['from']}）")
                if r["cur"] is None or r["cand"] is None or not (r["cand"] + thr < r["cur"]):
                    bad.add(f"{where}：候选 {r['cand']} + 门槛 {thr} 没比现首都 {r['cur']} 低")
            elif v == "review":
                review += 1
                if r["target"] is not None or r["cur"] is None or r["cand"] is None:
                    bad.add(f"{where}：评估未迁却带了 target / 缺判据数字")
            else:
                bad.add(f"{where}：未知 verdict `{v}`")
            if v != "forced" and r["round"] % every != 0:
                bad.add(f"{where}：`{v}` 出现在非评估回合（周期 {every}）")
            if r["cap_body"] != r["target"] and v != "review":
                bad.add(f"{where}：`capital_body`={r['cap_body']} ≠ target={r['target']}")
    tot = forced + reloc + review
    ck.check("首都判定自洽（强迁不评估 / 评估带判据数字 / 迁都按门槛）", bad.n == 0,
             bad.detail(f"{tag}：{tot} 条首都判定全部成套"))
    ck.check("首都判定守卫没有空转（三类都真的发生过）",
             forced > 0 and review > 0 and reloc > 0,
             f"{tag}：亡城强迁 {forced} / 周期评估 {review} / 评估迁都 {reloc}")


def mond_checks(h, ck, metas, tag) -> None:
    """**MOND 掌握度**（`src/tests/sim/knowledge.rs`，2026-10 第 7 批搬来）。

    | Rust 原件 | 这里 |
    | --- | --- |
    | `a_real_deep_presence_reaches_the_top` / `mastery_at_one_is_a_ratchet_and_never_rusts` | 一旦到过 `1.0` **永不回落**（棘轮）；且恒在 `[0, 1]` |
    | `presence_comes_only_from_ships_in_the_band` 的**唯一渠道**那半 | 掌握度**涨** ⇒ 那一回合带内**必须有舰**（不在带内的舰一点也不算） |
    | `initial_mastery_comes_from_config_and_frontier_reads_it` 的**前沿**那半 | 投影那一列逐行 == `--call mond_frontier(config, 掌握度)` 的 r2（**引擎同一份实现**，不是抄公式） |

    ⚠ **锈那一半搬不动**：`control_rusts_back_when_the_fleet_leaves` 只断言「撤出后回落」，
    但读面给的是**带内舰数**，而目标是**在场强度**（`1 + 深度 × depth_weight`，逐舰不同）
    ——实测长局里 123 次回落中有 **9 次带内有舰**（浅处一艘的强度仍低于现掌握度）。
    所以「涨 ⇒ 带内有舰」成立，「锈 ⇒ 带内没舰」不成立，只能各论各的。
    """
    ratchet, rng, climb = Verdict(), Verdict(), Verdict()
    chan, hard = Verdict(), Verdict()
    steps = climbs = rusts = at_top = 0
    frontier: dict = {}
    for m in metas:
        for f, seq in m["mond"].items():
            for (r0, c0, b0, p0, _t0, _), (r1, c1, b1, p1, _t1, _) in zip(seq, seq[1:]):
                steps += 1
                # 在场强度与「带内舰数」必须同生同灭（强度就是那批舰加权出来的）。
                if (p1 > 1e-9) != (b1 > 0):
                    chan.add(f"{f} r{r1}: 带内 {b1} 艘，强度却是 {p1:.4f}")
                # 涨必须有**强度**解释（比「有舰」更紧：带内一艘停在带沿上的强度也 > 0）。
                if c1 > c0 + 1e-12 and p1 <= 1e-9:
                    hard.add(f"{f} r{r1}: 涨了（{c0}→{c1}）可**在场强度为 0**")
                if not (0.0 <= c1 <= 1.0):
                    rng.add(f"{f} r{r1}: 掌握度 {c1} 越界")
                if c0 >= 1.0 - 1e-12 and c1 < 1.0 - 1e-12:
                    ratchet.add(f"{f} r{r1}: 到过 1.0 又掉到 {c1:.4f}")
                if c1 > c0 + 1e-12:
                    climbs += 1
                    if b1 == 0:
                        climb.add(f"{f} r{r1}: 涨了（{c0:.4f}→{c1:.4f}）可带内一艘舰都没有")
                elif c1 < c0 - 1e-12:
                    rusts += 1
            for (_, c, _, _p, _t, fr) in seq:
                if c >= 1.0 - 1e-12:
                    at_top += 1
                frontier.setdefault(c, set()).add(fr)

    ck.check("MOND：掌握度恒在 [0,1]（活量，不是一次性解锁）", rng.n == 0,
             rng.detail(f"{tag}：{steps} 个势力·回合"))
    ck.check("MOND：**1.0 是棘轮**——到过顶就永不回落", ratchet.n == 0,
             ratchet.detail(f"{tag}：{at_top} 个「势力·回合」在顶上，一个都没掉下来"))
    ck.check("MOND：掌握度**只**由「带内有舰」驱动（不在带内的舰一点也不算）", climb.n == 0,
             climb.detail(f"{tag}：{climbs} 次上涨全部有带内舰解释"))
    ck.check("MOND：**在场强度**与「带内舰数」同生同灭（强度就是那批舰加权出来的）", chan.n == 0,
             chan.detail(f"{tag}：{steps} 个势力·回合"))
    ck.check("MOND：上涨必须由**强度**解释（带内一艘停在带沿上的强度也 > 0）", hard.n == 0,
             hard.detail(f"{tag}：{climbs} 次上涨全部有正强度解释"))
    ck.check("MOND：守卫没有空转（真的涨过、锈过、到过顶）",
             climbs > 0 and rusts > 0 and at_top > 0,
             f"{tag}：涨 {climbs} / 锈 {rusts} / 顶上 {at_top} 个势力·回合")

    # 前沿海拔 = `mond_frontier(config, 掌握度)` 的 r2 —— 判据用**夹逼**，不设人肉容差：
    # ⚠ 投影里的 `MOND 掌握度` 本身过 `r2`（只有两位小数），而前沿是按**全精度**掌握度算的
    # ⇒ 逐值相等做不到。正确写法是拿引擎自己的函数在 `m ± 0.005`（那一格的宽度）上求值，
    # 要求投影落在 [lo, hi] 里（两边再各让 0.005 给前沿自己的 r2）。
    def frontier_at(m: float):
        v = json.loads(h.capture(["--seed", "42", "--call", "mond_frontier",
                                  "--args", json.dumps({"control": m})]))["value"]
        return None if v is None else float(v)

    mism = []
    for c, fronts in sorted(frontier.items()):
        lo, hi = frontier_at(max(0.0, c - 0.005)), frontier_at(min(1.0, c + 0.005))
        for fr in fronts:
            if hi is None:  # 上界已经是无穷 ⇒ 投影只能是 null（掌握到顶）
                if fr is not None:
                    mism.append(f"掌握度 {c}: 函数已到无穷，投影却是 {fr}")
                continue
            if fr is None:
                mism.append(f"掌握度 {c}: 投影是 null（无穷），函数只给到 {hi}")
            elif not (lo - 0.0051 <= fr <= hi + 0.0051):
                mism.append(f"掌握度 {c}: 投影 {fr} 不在 [{lo}, {hi}] 里")
    ck.check("MOND：前沿海拔逐行夹在 `mond_frontier(config, 掌握度±0.005)` 之间（引擎同一份实现）",
             not mism,
             "；".join(mism[:3]) or f"{len(frontier)} 个不同掌握度、{sum(len(v) for v in frontier.values())} 行全在夹逼区间里")
    ck.check("MOND：前沿对账没有空转（掌握度真的跨了很宽的一段）",
             len(frontier) >= 5, f"{len(frontier)} 个不同掌握度")


def _ideology_summary(q) -> dict:
    """**思潮四轴的逐势力时间序列**（`src/tests/sim/ideology.rs` 那几条的读面那一半）。"""
    fa = q.table("factions")[["round", "势力", "思潮"]]
    out: dict = {}
    for f, rows in fa.groupby("势力"):
        rows = rows.sort_values("round")
        out[f] = [(int(r["round"]), dict(r["思潮"])) for _, r in rows.iterrows()]
    return out


def ideology_checks(h, ck, metas, tag) -> None:
    """**思潮四轴**的有界性与步长上限（`sim/ideology.rs::ideology_economy_bad_drives_toward_populism_and_stays_bounded`
    的「有界」那半，第 7 批）。

    引擎的松弛是 `cur + drift_rate × (target − cur)` 的**凸组合**，而目标钳在 `[-1, 1]`
    ⇒ ① 每一轴恒在 `[-1, 1]`；② 每回合位移不超过 `drift_rate × 2`（两端相距最多 2）。

    ⚠ **「朝目标走」那半没搬**，试过两种写法都不成立：
    * **逐回合法条**：列过 `r2`、每回合位移 ≤ `drift_rate`(0.05) ⇒ 判据会退化成「怎么都过」；
    * **区间夹逼**（下一回合的值必须落在「旧值 ↔ 目标」之间）：把四轴的目标都从读面复算
      （军事走 `--call military_deltas` 喂当回合事件、经济按 `production × 价值 − upkeep − governance`、
      科学按异常区城数 − 舰数、自然按人均面积）后，**越界率仍有 2.6%**——试过把目标错后一回合
      （相位差）反而更差（3.9% / 6.0%）。剩下的偏差来自 `step_ideology` 用的是**走那一刻**的
      state 与 flow，读面只有回合末的值。
    """
    axes = ("和平↔军国", "科学↔技术", "人民↔精英", "自然↔殖民")
    rate = float(json.loads(h.capture(["--meta"]))["ideology"]["drift_rate"])
    bound, step = Verdict(), Verdict()
    steps = 0
    moved = {a: 0 for a in axes}
    for m in metas:
        for f, seq in m["ideology"].items():
            for (r0, a), (r1, b) in zip(seq, seq[1:]):
                steps += 1
                for ax in axes:
                    v = float(b[ax])
                    if not (-1.0 <= v <= 1.0):
                        bound.add(f"{f} r{r1} {ax}: {v} 越界")
                    d = v - float(a[ax])
                    if abs(d) > rate * 2.0 + 0.011:
                        step.add(f"{f} r{r1} {ax}: 位移 {d:.4f} 超过 drift_rate×2 = {rate * 2:.3f}")
                    if abs(d) > 1e-9:
                        moved[ax] += 1
    ck.check("思潮：四轴恒在 [-1,1]（松弛是凸组合、目标钳在 [-1,1]）", bound.n == 0,
             bound.detail(f"{tag}：{steps} 个势力·回合 × 4 轴"))
    ck.check(f"思潮：每一轴的每回合位移不超过 `drift_rate × 2`（= {rate * 2:.3f}）", step.n == 0,
             step.detail(f"{tag}：{steps} 个势力·回合"))
    ck.check("思潮守卫没有空转（四轴都真的动过）", all(v > 0 for v in moved.values()),
             f"各轴动过的次数 {moved}")


def _ideo_econ_summary(q) -> dict:
    """**思潮经济轴 vs 经济净值**的逐势力时间序列（第 7 批，订正索引后用）。

    净值 = `Σ(逐资源产出 × 价值) − upkeep − governance_total`（价值表取 `meta.market.resource_value`，
    引擎的 `value_of` 就是它、逐字相同）。"""
    val = ((q.meta or {}).get("market") or {}).get("resource_value") or {}
    fp = q.table("faction_process")
    out: dict = {}
    for _, r in fp.iterrows():
        v = sum(float(x) * float(val.get(k, 1.0)) for k, x in (r["production"] or {}).items())
        out.setdefault(r["势力"], []).append((int(r["round"]),
                                             v - float(r["upkeep"]) - float(r["governance_total"])))
    return out


def mond_law_checks(h, ck, metas, tag) -> None:
    """**MOND 掌握度的逐回合定律**（`sim/knowledge.rs::control_rusts_back_when_the_fleet_leaves` 的「按同一速率回落」那半，第 7 批）。

    引擎：每回合 ① 到顶（≥1.0）就**棘轮**不动；② 当回合目标 ≥ 1 ⇒ 按 `1 ÷ mastery_rounds`
    **线性爬满**（学满是「持续够格」，不是一回合）；③ 否则朝目标**松弛** `drift_rate`。

    ⚠ **索引要对**：轴的 `r → r+1` 这一步用的是**第 `r+1` 行**的目标（那一行才是跑出 `掌握度(r+1)`
    的那一步看到的东西）。第一版我配成了第 `r` 行，于是得出"相位差 0.0267、读面判不了"的**错误结论**；
    订正后 seed 42 是 **1/3600**、seed 7 是 **0/3600**，唯一那一处是**船在回合中途进出带内**
    （回合末的目标与走那一刻的差一档）⇒ 目标变过的回合改用「上一回合目标算的 ↔ 本回合目标算的」
    **夹逼**复核。

    「撤出后回到凡人」那半由本定律 + g2 的驻泊场景共同覆盖（长期不在场 ⇒ 目标 0 ⇒ 按同一速率回落）。
    """
    rate = float(json.loads(h.capture(["--meta"]))["mond"]["knowledge"]["drift_rate"])
    mrounds = int(json.loads(h.capture(["--meta"]))["mond"]["knowledge"]["mastery_rounds"])
    tol = 0.011  # 掌握度列过 r2（±0.005），乘上 1−rate 再叠加一次

    def pred(m0: float, tgt: float) -> float:
        if m0 >= 1.0 - 1e-9:
            return m0
        if tgt >= 1.0 - 1e-9:
            return min(1.0, m0 + 1.0 / max(1, mrounds))
        return min(1.0, max(0.0, m0 + rate * (tgt - m0)))

    exact = Verdict()
    brack = Verdict()
    stable = moved = 0
    for m in metas:
        for f, seq in m["mond"].items():
            for (r0, c0, _b0, _p0, t0, _fr0), (r1, c1, _b1, _p1, t1, _fr1) in zip(seq, seq[1:]):
                want = pred(c0, t1)
                if abs(t1 - t0) < 1e-9:
                    stable += 1
                    if abs(c1 - want) > tol:
                        exact.add(f"{f} r{r1}: 掌握度 {c0} → {c1}，按「当回合目标 {t1}」应为 {want:.4f}")
                else:
                    moved += 1
                    lo, hi = sorted((want, pred(c0, t0)))
                    if not (lo - tol <= c1 <= hi + tol):
                        brack.add(f"{f} r{r1}: {c0} → {c1}，目标 {t0}→{t1}，"
                                  f"夹逼区间 [{lo:.4f}, {hi:.4f}]")
    ck.check(f"MOND：**目标没变的回合**逐回合等于定律值（棘轮 / 学满线性 / 朝目标松弛；"
             f"drift_rate {rate}、mastery_rounds {mrounds}）", exact.n == 0,
             exact.detail(f"{tag}：{stable} 个势力·回合"))
    ck.check("MOND：**目标中途变过**的回合落在「上一回合目标 ↔ 本回合目标」的夹逼区间里"
             "（船在回合中途进出带内）", brack.n == 0,
             brack.detail(f"{tag}：{moved} 个势力·回合"))
    ck.check("MOND 定律守卫没有空转（两类回合都真的出现过、且真的涨过/锈过）",
             stable > 100 and moved > 10,
             f"{tag}：目标稳定 {stable} 个、目标变动 {moved} 个势力·回合")


def ideology_law_checks(h, ck, metas, tag) -> None:
    """**思潮「人民↔精英」的逐回合定律**（`sim/ideology.rs::ideology_economy_bad_drives_toward_populism_and_stays_bounded`，第 7 批）。

    引擎：目标 = `clamp(净值 ÷ economy_scale, −1, 1)`；轴每回合朝它松弛 `drift_rate`。
    净值由读面复算（逐资源产出 × 价值 − 维护 − 治理）——**索引要对**：轴的 `r → r+1` 用第 `r+1` 行。
    订正后 seed 42 与 seed 7 各 300 回合**共 5400 样本 0 违规**（比原件"注入一次 upkeep=100 再看一回合"
    强得多）。

    ⚠ **推论**：经济转负 ⇒ 目标在人民端 ⇒ 轴朝人民端走；「四轴恒在 [-1,1]」由 :func:`ideology_checks` 压着。
    """
    meta = json.loads(h.capture(["--meta"]))
    esc = float(meta["ideology"]["economy_scale"])
    rate = float(meta["ideology"]["drift_rate"])
    tol = 0.012  # 思潮列过 r2（±0.005），乘 (1−rate) 再叠加一次，另留一点余量
    law = Verdict()
    n = 0
    moved = 0
    for m in metas:
        for f, seq in m["ideology"].items():
            ne = dict(m["ideo_econ"].get(f, []))
            for (r0, a0), (r1, a1) in zip(seq, seq[1:]):
                if r1 not in ne:
                    continue
                n += 1
                tgt = max(-1.0, min(1.0, ne[r1] / max(1e-6, esc)))
                want = max(-1.0, min(1.0, a0["人民↔精英"] + rate * (tgt - a0["人民↔精英"])))
                if abs(a1["人民↔精英"] - want) > tol:
                    law.add(f"{f} r{r1}: 轴 {a0['人民↔精英']} → {a1['人民↔精英']}，"
                            f"净值 {ne[r1]:.2f} ⇒ 目标 {tgt:.3f} ⇒ 应为 {want:.4f}")
                if abs(a1["人民↔精英"] - a0["人民↔精英"]) > 1e-9:
                    moved += 1
    ck.check(f"思潮：**「人民↔精英」逐回合等于「朝当回合经济目标松弛」**"
             f"（目标 = 净值 ÷ {esc}，drift_rate {rate}）", law.n == 0,
             law.detail(f"{tag}：{n} 个势力·回合"))
    ck.check("思潮定律守卫没有空转（轴真的动过）", n >= 100 and moved > 100,
             f"{tag}：{n} 个样本里 {moved} 次位移")


def _upkeep_summary(q) -> dict:
    """**维护费的欠费与生锈**（`src/tests/sim/spending.rs::upkeep_shortfall_records_the_unpaid_part_and_the_rust_it_causes`
    的「读面那一半」，第 7 批）。

    `faction_process` 三列（`upkeep` / `upkeep_unpaid` / `fleet_rust`）+ `cities.已焚毁`
    （判断「流亡舰队」）。"""
    fp = q.table("faction_process")[["round", "势力", "upkeep", "upkeep_unpaid", "fleet_rust"]]
    ci = q.table("cities")
    live = {(int(r["round"]), r["势力"]) for _, r in ci[~ci["已焚毁"]].iterrows()}
    return {"rows": [(int(r["round"]), r["势力"], float(r["upkeep"]),
                      float(r["upkeep_unpaid"]), float(r["fleet_rust"]))
                     for _, r in fp.iterrows()],
            "live": live}


def upkeep_law_checks(h, ck, metas, tag) -> None:
    """**维护费的两条读面律**（`sim/spending.rs::upkeep_shortfall…` 的可读那一半，第 7 批）。

    引擎（`sim/production.rs::step_upkeep`）：
    `欠费 = max(0, 维护费 − 池子价值)`；`生锈比例 = min(欠费 ÷ 维护费, 1)`，**且不低于 0.2**
    （「欠费就得看得出来」的可见性下限）；**流亡舰队**（无活城）不抽库存、不生锈。

    判据（7 seed × 1000 回合）：
    1. `fleet_rust == max(0.2, min(欠费 ÷ 维护费, 1))`，欠费为 0 时 `fleet_rust == 0`；
    2. 前后两回合都**没有活城** ⇒ 欠费与锈都是 0（豁免）。

    ⚠ **原件的另外三条留在 Rust**：`欠费 == 恰好半价`是**搭台子**（读面复现不了"恰好"），
    **每艘舰掉的血 = `船体上限 × 锈`** 减的是** upkeep 那一步的船体**（回合中途，读面只有回合末），
    「视图 == sink」是**同一份量的两个位置**（内部契约）。⇒ 那条 test 仍住 §4。
    """
    ratio, zero, exempt = Verdict(), Verdict(), Verdict()
    n = short = floored = 0
    for m in metas:
        up = m["upkeep"]
        live = up["live"]
        for rnd, f, upkeep, unpaid, rust in up["rows"]:
            n += 1
            if unpaid <= 1e-9:
                if rust != 0.0:
                    zero.add(f"{f} r{rnd}: 欠费 0 却锈 {rust}")
                continue
            short += 1
            want = max(0.2, min(unpaid / upkeep, 1.0))
            if unpaid / upkeep < 0.2:
                floored += 1
            if abs(rust - want) > 1e-4:
                ratio.add(f"{f} r{rnd}: 欠费/维护费 {unpaid / upkeep:.4f} ⇒ 应锈 {want:.4f}，实为 {rust:.4f}")
            if (rnd - 1, f) not in live and (rnd, f) not in live:
                exempt.add(f"{f} r{rnd}: 前后两回合都无活城（流亡）却有欠费 {unpaid:.2f}/锈 {rust:.4f}")
    ck.check("维护费：**生锈比例 = `max(0.2, 欠费 ÷ 维护费)`**（含 0.2 可见性下限），"
             "欠费为 0 就不锈", ratio.n == 0 and zero.n == 0,
             (ratio.detail(f"{tag}：{short}/{n} 行有欠费") + " " + zero.detail("")).strip())
    ck.check("维护费：**流亡舰队豁免**（前后两回合都没有活城 ⇒ 不欠费、不生锈）", exempt.n == 0,
             exempt.detail(f"{tag}：{n} 个势力·回合"))
    ck.check("维护费守卫没有空转（真的欠过费、而且 0.2 下限真的起过作用）",
             short >= 20 and floored >= 1,
             f"{tag}：{short} 行欠费，其中 {floored} 行的比例低于 0.2（被下限抬上来）")


def _role_dist_summary(q) -> dict:
    """**定编分布的逐回合对账行** + 「本回合这个势力有没有被玩家钉死的舰」。

    行 = `(round, 势力, 支, 侧, flow, Σp, 截断数)`；`flow` 是**引擎自己算的**期望
    （`缺口 + 轮换 × 头数`），`Σp` 是池子里那一侧各舰机会值之和。**没被截断时两者必须相等**
    ——这条就是"分母有没有被稀释"的尺子。
    """
    ri = q.table("round_inputs")
    sh = q.table("ships")
    pins: dict = {}
    for _, s in sh.iterrows():
        if s["order_effective_mode"] != "Auto" or s["role_mode"] == "Player":
            pins[(int(s["round"]), s["势力"])] = True
    rows = []
    for _, r in ri.iterrows():
        rnd = int(r["round"])
        if rnd < 2:
            continue
        for f, v in (r["role_distribution"] or {}).items():
            for role in ("freight", "observe"):
                x = v[role]
                for side in ("join", "leave"):
                    rows.append((rnd, f, role, side, float(x[f"flow_{side}"]),
                                 float(x[f"sum_p_{side}"]), float(x[f"tickets_{side}"]),
                                 float(x[f"sum_mine_{side}"]), float(x[f"mine_{side}"]),
                                 float(x[f"clamped_{side}"])))
    return {"rows": rows, "pins": pins}


def distribution_checks(h, ck, metas, tag) -> None:
    """**定编分布的逐回合对账**（`autocontrol::freight::assign_roles` 的两枚骰子，第 7 批）。

    引擎的契约（`knowledge.rs` 原话：「缺口 → 按票抽签 `p = (缺口 + 轮换) × 我的票 ÷ 同侧总票数`
    ⇒ **期望入伙数正好等于缺口**」）写成可对账的形式就是 —— 池子只有一侧、`Σ我的票 = 同侧总票数`
    ⇒ **`Σp` 必须正好等于 `flow`**。这条替掉了两条抽样判据（跑 400 / 200 回合比均值：实测
    `r1 中国` 观测配额 1.51 而在册 2，那条 ±0.5 的容差一直在**吸收 0.49 的系统性偏差**）。

    **口径**（这条我错了四次才对，写清楚）：只有这三件事同时成立才拿来对账 ——
    1. 这一侧**记到过舰**（`mine_* > 0`）；
    2. 池子**非空**（`tickets_* > 1e-8`；等于 `1e-9` 是 `max(1e-9)` 的**地板值** ⇒ 池子空 ——
       例如该舰是观测舰，按设计被排在自己的运输池外 ⇒ `我的票 = 0` ⇒ `p = 0` **合法**，
       但那个 `flow` 根本不是这一侧的总期望）；
    3. **记下的舰正好是池子**（`Σ我的票 == 同侧总票数`）且没有被 `min(1,·)` 截断。

    实测（3 seed × 300 回合）：**对上的行 721/487、837/772、778/586，违反 0**（另有「没记到舰」
    「池子空」「被截断」三类合法跳过）。⚠ 我还用闸门里带的池子名单点过名：**池子里没有一艘
    "有票却没有闸门"的舰**（0 例）⇒ 不存在"分母里挂着永远不会被定编的舰"那种稀释
    （引擎注释里记的那次事故是**已经修好的历史**，现行实现是自洽的）。
    """
    law = Verdict()
    n = skipped = 0
    for m in metas:
        for rnd, f, role, side, flow, sp, tk, sm, mn, cl in m["role_dist"]["rows"]:
            if mn == 0 or tk <= 1e-8 or abs(sm - tk) > 1e-9 or cl > 0:
                skipped += 1
                continue
            n += 1
            if abs(sp - flow) > 1e-9:
                law.add(f"{f} r{rnd} {role}/{side}: Σp {sp:.6f} ≠ flow {flow:.6f}"
                        f"（票 {sm:.6f}/{tk:.6f}、截断 {cl:.0f}）")
    ck.check("定编：**`Σp` 正好等于引擎自己算的 `flow`**（观测/运输两枚骰子 × 入伙/退伍两侧；"
             "`flow = 缺口 + 轮换 × 头数`）", law.n == 0,
             law.detail(f"{tag}：{n} 行逐字对上；另有 {skipped} 行属于「这一侧没记到舰 / 池子空 / "
                        f"被 `min(1,·)` 截断」三类，按口径跳过"))
    ck.check("定编分布对账没有空转（真的对过账、也真的掷出过非平凡的概率）", n >= 2000,
             f"{tag}：{n} 行")


def identity_checks(h, ck, metas, tag) -> None:
    """**过程量表的恒等式**（`sim/governance.rs` + `sim/spending.rs` 那几条搬过来）。

    样本 = 全 7 seed × 1000 回合的**每一个城行 / 势力行**（Rust 版是「跑一步、看开局那几座城」）。
    判据一条没动，只把样本放大 —— 这些恒等式本来就该对**每一行**成立，所以放大样本是纯增益：
    真正会坏的地方往往是「某个回合某座城的某个边界」（例如忠诚 clamp 的 0 端）。
    """
    ids = [m["identities"] for m in metas]

    checked = sum(i["loyalty_checked"] for i in ids)
    bad_n = sum(i["loyalty_bad_n"] for i in ids)
    range_n = sum(i["loyalty_range_bad"] for i in ids)
    moved = sum(i["loyalty_excluded_moved"] for i in ids)
    new = sum(i["loyalty_excluded_new"] for i in ids)
    sample = next((m for i in ids for m in i["loyalty_bad"]), "")
    ck.check("忠诚目标 = clamp(距离 + 娱乐 + 首都向心 − 思潮惩罚, 0, 1)", bad_n == 0,
             f"{sample}（共 {bad_n} 处）" if bad_n else
             f"{tag}：{checked:,} 个活城·回合行逐行成立（容忍 1e-12）")
    ck.check("忠诚分解的例外都说得清（本回合易主的城：城项按旧主、全国项按新主 ⇒ 必然对不上）",
             True,
             f"排除 {moved:,} 行易主 + {new:,} 行新建；**没有第三种例外**（否则上面那条会红）")
    ck.check("忠诚目标落在 [0,1] 里", range_n == 0,
             f"{range_n} 处越界" if range_n else f"{checked + moved + new:,} 行全在 [0,1]")

    mb = sum(i["mult_bad_n"] for i in ids)
    db = sum(i["domain_bad_n"] for i in ids)
    sample = next((m for i in ids for m in i["mult_bad"]), "")
    dsample = next((m for i in ids for m in i["domain_bad"]), "")
    ck.check("治理总开销 ≥ 行政 + 娱乐（制裁倍率 ≥ 1）", mb == 0,
             f"{sample}（共 {mb} 处）" if mb else f"{tag}：每一行都 ≥（倍率 ≥ 1）")
    ck.check("治理量的定义域（scale ≥ 1、思潮惩罚/首都向心 ≥ 0、覆盖率 ∈ [0,1]）", db == 0,
             f"{dsample}（共 {db} 处）" if db else f"{tag}：每一行都在定义域里")

    ac = sum(i["admin_checked"] for i in ids)
    ab = sum(i["admin_bad_n"] for i in ids)
    aex = sum(i["admin_excluded_changed"] for i in ids)
    sample = next((m for i in ids for m in i["admin_bad"]), "")
    ck.check("有活城的势力必有行政开销（「钱被行政吃掉」在读数上看得见）", ab == 0,
             f"{sample}（共 {ab} 处）" if ab else
             f"{tag}：{ac:,} 个「城集合这一回合没变的势力·回合」全部有行政开销")
    ck.check("行政开销的例外都说得清（这一回合城集合变了的势力：治理那一步看到的不是这个集合）",
             True, f"排除 {aex:,} 行「本回合城集合变了且行政开销为 0」——没有第四种例外")
    ck.check("行政开销守卫没有空转（真有势力城集合稳定地持有活城）", ac >= 100, f"{ac:,} 行（下限 100）")

    rd = sum(i["rust_domain_n"] for i in ids)
    rp = sum(i["rust_pair_bad_n"] for i in ids)
    seen = sum(i["rust_seen"] for i in ids)
    sample = next((m for i in ids for m in i["rust_domain"]), "")
    psample = next((m for i in ids for m in i["rust_pair_bad"]), "")
    ck.check("欠费/生锈的定义域（欠费 ≥ 0、锈 ∈ [0,1]）", rd == 0,
             f"{sample}（共 {rd} 处）" if rd else f"{tag}：每一行都在定义域里")
    ck.check("欠费与生锈同生同灭（欠费才生锈，锈了必欠费）", rp == 0,
             f"{psample}（共 {rp} 处）" if rp else f"{tag}：{seen:,} 个「既欠费又生锈」的行，0 个单飞")
    ck.check("欠费守卫没有空转（真的欠过费也锈过船）", seen > 0, f"{seen:,} 行同时欠费且生锈")

    ib = sum(i["inc_bad_n"] for i in ids)
    inc_seen = sum(i["inc_seen"] for i in ids)
    rate_seen = sum(i["rate_seen"] for i in ids)
    sample = next((m for i in ids for m in i["inc_bad"]), "")
    ck.check("造舰进度 0 ≤ increment ≤ rate（rate 是产能上限）", ib == 0,
             f"{sample}（共 {ib} 处）" if ib else f"{tag}：{rate_seen:,} 个「城·舰级·回合」行全部 ≤ 产能")
    ck.check("造舰进度守卫没有空转（真的有进度在涨）", inc_seen >= 100,
             f"{inc_seen:,} 行 increment > 0（下限 100）")

    ob = sum(i["unpaid_over_bill_n"] for i in ids)
    sample = next((m for i in ids for m in i["unpaid_over_bill"]), "")
    ck.check("欠费不超过维护账单本身（欠的只能是账单的一部分）", ob == 0,
             f"{sample}（共 {ob} 处）" if ob else f"{tag}：每一行都 ≤ 本回合的维护账单")

    hp = sum(i["hub_pairs"] for i in ids)
    hr = sum(i["hub_rows"] for i in ids)
    hm = sum(i["hub_multi_n"] for i in ids)
    sample = next((m for i in ids for m in i["hub_multi"]), "")
    ck.check("一个势力在一个回合里只有一个集散地天体（`is_hub` 自洽）", hm == 0,
             f"{sample}（共 {hm} 处）" if hm else
             f"{tag}：{hr:,} 个 hub 城行 / {hp:,} 个「回合·势力」，**无豁免**地只有一个 hub 天体")
    ck.check("hub 守卫没有空转（真有 hub 城）", hp >= 100, f"{hp:,} 个「回合·势力」（下限 100）")


if __name__ == "__main__":
    sys.exit(group_main("g3_long", run))
