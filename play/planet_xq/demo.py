"""Run me from the project dir: `uv run python demo.py <projection-dir>`.

Shows how an agent reads the projection schema (which fields are lazy), then fetches heavy
detail by id from the index and joins it back onto the lean main stream — and how the static
rules dictionary (``meta.json``, same source as ``--meta``) is loadable as DataFrames so you
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
print("--- main facts (lean): rounds + metrics shape ---")
print("rounds:", list(q.facts["round"]))
print("one fact row keys:", list(q.facts.columns))

print()
print("--- join('ships', round=6): main fact row exploded onto ship detail ---")
merged = q.join("ships", round=6)
cols = [c for c in ("round", "ship_id", "faction_id", "class", "hull", "hull_max", "x", "y") if c in merged.columns]
print(merged[cols].head(8).to_string(index=False))

print()
print("--- rules x facts: fleet upkeep on hand at round 6 ---")
spec = q.ships_spec()
fleet = merged.merge(spec[["upkeep"]], left_on="class", right_index=True)
print("ships in fleet:", len(fleet), "| total upkeep/month:", round(float(fleet["upkeep"].sum()), 2))

print()
print("--- pull a specific round's ship ids from the main stream, then the detail ---")
ids = q.ids("ships", 3)
print("ship_ids at round 3:", ids[:8], "… total", len(ids))
print("q.cities(round=6):", q.cities(round=6).shape)
print("q.bodies():", q.bodies().shape, "cols", list(q.bodies().columns))

print()
print("--- long-run stats: 年均 / 十年均 (round = 1 month) ---")
# World metrics (metrics.population / metrics.cities / metrics.fleet_value …) via dotted path.
cities = q.yearly_avg("metrics.cities")
print("年均 world cities:", {int(y): round(float(v), 2) for y, v in cities.items()} if len(cities) else "…")
print("十年均 world cities:", {int(y): round(float(v), 2) for y, v in q.decadal_avg("metrics.cities").items()})
# Per-faction metric: path = metrics.factions.<faction>.<metric>.
fak = (q.facts.iloc[0].get("metrics") or {}).get("factions", {})
if fak:
    fid = next(iter(fak))
    p = q.yearly_avg(f"metrics.factions.{fid}.production_value")
    print(f"年均 {fid} production_value:", {int(y): round(float(v), 1) for y, v in p.items()})
