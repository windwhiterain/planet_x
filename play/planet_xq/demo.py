"""Run me from the project dir: `uv run python demo.py <projection-dir>`.

Shows how an agent reads the projection schema (which fields are lazy), then fetches heavy
detail by id from the index and joins it back onto the lean main stream.
"""
import sys

import planet_xq

q = planet_xq.load(sys.argv[1] if len(sys.argv) > 1 else ".")

print("--- agent reads schema.json: which fields are lazy, and how ---")
for name, cfg in q.schema["lazy"].items():
    print(f"{name}: table={cfg['table']} key={cfg['key']} round={cfg['round']}")
    print("   ", cfg["description"][:70], "…")

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
print("--- pull a specific round's ship ids from the main stream, then the detail ---")
ids = q.ids("ships", 3)
print("ship_ids at round 3:", ids[:8], "… total", len(ids))
print("q.cities(round=6):", q.cities(round=6).shape)
print("q.bodies():", q.bodies().shape, "cols", list(q.bodies().columns))
