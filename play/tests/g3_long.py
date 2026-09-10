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

import math
import sys
from pathlib import Path

import pandas as pd

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _harness import KIT, group_main  # noqa: E402

SEEDS = (1, 2, 3, 4, 5, 7, 11)
ROUNDS = 1000

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


def extract(dirpath) -> tuple[pd.DataFrame, dict]:
    """把一份投影压成**每回合一行**的判据表 + 元数据（结果按投影缓存成 pickle）。"""
    q = KIT.load(str(dirpath), only=("events", "ships", "factions"))
    facts = q.facts
    view = facts["view"]
    rounds = facts["round"].to_numpy()

    # ④ 建城必须有一艘在场的活舰：事件层 `colony_founded`（actor = 建城方）join 该回合该势力
    #    `hull > 0` 的舰数（投影的舰表 = 回合末状态，与 Rust 版「回合末仍有活舰」同口径）。
    ev = q.table("events")
    cf = ev[ev["type"] == "colony_founded"]
    live = q.table("ships").query("hull > 0").groupby(["round", "faction_id"]).size()
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
            rel = r["relations"]
            relations[(int(r["round"]), r["faction_id"])] = rel if isinstance(rel, dict) else {}

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
    }
    return pd.DataFrame(rows), meta


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


if __name__ == "__main__":
    sys.exit(group_main("g3_long", run))
