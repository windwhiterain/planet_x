"""Run me from the project dir: `uv run python demo.py <projection-dir>`.

Shows how an agent reads the projection schema (which fields are lazy), then fetches heavy
detail by id from the index and joins it back onto the lean main stream — the per-faction
diplomacy/economy/fleet view, a faction's cities/buildings, and a ship's effective panel.
Also shows how the static rules dictionary (``meta.json``) is loadable as DataFrames so you
can cross-query rules against world facts in the same session.
"""
import sys

import planet_xq

q = planet_xq.load(sys.argv[1] if len(sys.argv) > 1 else ".")

print("--- agent reads schema.json: which fields are lazy, and how ---")
for name, cfg in q.schema["lazy"].items():
    print(f"{name}: table={cfg['table']} key={cfg['key']} round={cfg['round']}")
    print("   ", cfg["description"][:70], "…")

print()
print("--- rules dictionary (meta.json): sections + spec tables ---")
if q.meta is None:
    print("no meta.json in this projection — rerun `planet_x --index` on a fresh build")
else:
    print("sections:", list(q.meta))
    print("ships_spec():", q.ships_spec().shape, "classes:", list(q.ships_spec().index))
    print("resource_value():")
    print(q.resource_value().head(4).to_string())
    print("ship build_cost (dict-valued cells → use .meta for the raw dict):")
    print(q.ships_spec()[["label", "hull", "upkeep", "build_points"]].to_string())

print()
print("--- main facts (lean): rounds + view shape ---")
print("rounds:", list(q.facts["round"]))
print("one fact row keys:", list(q.facts.columns))

print()
print("--- faction snapshot (diplomacy + economy + fleet in one call) ---")
rid = int(q.facts.iloc[0]["round"])
snap = q.faction_snapshot(rid, "中国")
if snap.get("exists"):
    print("中国 @ round", rid)
    print("  resources:", snap.get("resources"))
    print("  relations:", snap.get("relations"))
    print("  view (this faction's row):", snap.get("view"))
    print("  cities:", snap.get("city_ids"), "ships:", snap.get("ship_ids"))
else:
    print("no 中国 row (or projection produced before factions existed)")

print()
print("--- join('ships', round=6): main fact row exploded onto ship detail (with panel) ---")
merged = q.join("ships", round=6)
cols = [c for c in ("round", "ship_id", "faction_id", "class", "hull", "hull_max", "attack", "upkeep", "x", "y") if c in merged.columns]
print(merged[cols].head(8).to_string(index=False))

print()
print("--- rules x facts: fleet upkeep on hand at round 6 ---")
spec = q.ships_spec()
# ⚠ 两边都有 `upkeep` 列（ships 表带的是**实测面板**的维护费，spec 表带的是**规则**里的），
# 直接 merge 会得到 `upkeep_x`/`upkeep_y`，然后 `fleet["upkeep"]` 当场 KeyError（这条踩过：
# main 上一直是坏的，与本 demo 无关）。改名之后正好能顺手把「规则 vs 事实」对照打出来。
fleet = merged.merge(
    spec[["upkeep"]].rename(columns={"upkeep": "spec_upkeep"}),
    left_on="class",
    right_index=True,
)
print(
    "ships in fleet:",
    len(fleet),
    "| total upkeep/month:",
    round(float(fleet["upkeep"].sum()), 2),
    f"(舰级基础值合计 {round(float(fleet['spec_upkeep'].sum()), 2)}——面板值含舰级/组件系数，故更大)",
)

print()
print("--- pull a specific round's ship ids from the main stream, then the detail ---")
ids = q.ids("ships", 3)
print("ship_ids at round 3:", ids[:8], "… total", len(ids))
print("q.cities(round=6):", q.cities(round=6).shape)
print("q.bodies():", q.bodies().shape, "cols", list(q.bodies().columns))
print("q.settlements():", q.settlements().shape, "cols", list(q.settlements().columns))

print()
print("--- long-run stats: 年均 / 十年均 (round = 1 month) ---")
cities = q.yearly_avg("view.city_count")
print("年均 world cities:", {int(y): round(float(v), 2) for y, v in cities.items()} if len(cities) else "…")
print("十年均 world cities:", {int(y): round(float(v), 2) for y, v in q.decadal_avg("view.city_count").items()})
fak = (q.facts.iloc[0].get("view") or {}).get("factions", {})
if fak:
    fid = next(iter(fak))
    p = q.yearly_avg(f"view.factions.{fid}.production_value")
    print(f"年均 {fid} production_value:", {int(y): round(float(v), 1) for y, v in p.items()})

print()
print("--- B1: 治理/忠诚的中间量（「钱花在哪」「这座城的忠诚为什么在掉」）---")
last_round = int(q.facts["round"].max())
econ = q.view_economy(last_round, "中国")
parts = econ["governance_admin"] + econ["governance_entertainment"]
print(
    f"中国 @ round {last_round}: 治理总开销 {round(econ['governance_cost'], 2)} ="
    f" (行政 {round(econ['governance_admin'], 2)} + 娱乐 {round(econ['governance_entertainment'], 2)})"
    f" × 制裁倍率 | 覆盖率 {econ['governance_coverage']} | 人口超载倍率 {round(econ['governance_scale'], 3)}"
    f" | 思潮忠诚惩罚 {round(econ['ideology_loyalty_penalty'], 4)}"
)
# 勾稽：「行政 + 娱乐」乘上制裁倍率才是总开销 ⇒ 两项之和 ≤ 总（倍率 ≥ 1）。这里把比值也打出来，
# 它就是 1/倍率；不相等就说明读面把两个来源说岔了。
assert parts > 0.0 and econ["governance_cost"] >= parts - 1e-9, (parts, econ["governance_cost"])
print("  两项之和 ÷ 总开销 =", round(parts / econ["governance_cost"], 4), "（= 1 ÷ 制裁倍率）")
print("  迁都判据（只在评估回合有数）：", econ["capital"])

lt = q.view_loyalty(last_round, "中国")
cols = [
    "city_id", "loyalty", "loyalty_target_effective", "loyalty_target_distance",
    "loyalty_target_entertainment", "loyalty_target_capital_share",
    "loyalty_target_ideology_penalty",
]
print(lt[[c for c in cols if c in lt.columns]].to_string(index=False))
# 勾稽：目标忠诚 = 四项相加并 clamp[0,1]（这是引擎自己的分解，Python 侧只做加法，不重算公式）。
row = lt.iloc[0]
s = (
    row["loyalty_target_distance"]
    + row["loyalty_target_entertainment"]
    + row["loyalty_target_capital_share"]
    - row["loyalty_target_ideology_penalty"]
)
assert abs(row["loyalty_target_effective"] - min(max(s, 0.0), 1.0)) < 1e-12, (
    s, row["loyalty_target_effective"],
)
print("勾稽通过：effective = clamp(distance + entertainment + capital_share − ideology_penalty)")
