# planet_xq

Read a `planet_x --index` projection into pandas.

The Rust generator projects a run into a **lean main stream** + **id-indexed lazy tables** so the
agent's default view stays small (only `metrics` + id-arrays are inline). Use this kit to fetch a
heavy field (a ship's full object, a city's details, a faction's stockpile + relations) by id, and
to join it back to the main facts. **Entity ids are always string names** (ship/city/building/
faction/body/settlement = its unique name), never integer offsets.

## Layout

```
planet_x --seed 7 --round 12 --index out/
# out/main.jsonl        lean per-round facts (round, time_month, events, chronicle, metrics,
#                       ship_ids[], city_ids[], faction_ids[], body_ids[], settlement_ids[])
# out/schema.json       agent-readable projection contract (eager / lazy / columns / read_order)
# out/meta.json         static rules dictionary (ships/buildings/components/structures/economy/…,
#                       same source as `planet_x --meta`) — loadable as DataFrames
# out/idx/ships.jsonl      (round, ship_id, ...) per-round ship detail (+ effective panel)
# out/idx/cities.jsonl     (round, city_id, ...) per-round city detail (+ buildings list)
# out/idx/factions.jsonl   (round, faction_id, ...) per-round faction detail (resources/relations/
#                           own city + ship ids)
# out/idx/bodies.jsonl     (body_id, ...) global master table
# out/idx/settlements.jsonl (settlement_id, ...) global 定居点 master
```

`schema.json` is the single contract both Rust and this kit share: it declares which fields are
**eager** (inline in `main.jsonl`) vs **lazy** (in a table keyed by id), plus each lazy field's
table / key / id-column / whether it is per-round, and the column types — so an agent can navigate
without reverse-engineering the JSON.

## Usage (uv)

```bash
uv sync                                  # create .venv (pandas) + uv.lock
uv run planet-xq out                     # console script: print facts + a join('ships', round=0)
# or
uv run python -c "
import planet_xq
q = planet_xq.load('out')
print(q.facts)                 # lean main stream
print(q.factions(round=10))    # per-faction resources + relations + own city/ship ids
print(q.faction_snapshot(10, '中国'))   # one-call decision view (metrics+stockpile+relations)
print(q.ships(round=10))       # ships at round 10 (from the index), with effective panel
print(q.city_buildings(10, '中国'))     # that faction's cities, each with a buildings list
merged = q.join('ships', round=10)   # explode main ship_ids and merge with ship detail
print(merged[['ship_id','class','hull','x','y','attack','upkeep']])
print(q.bodies())              # global master table
print(q.settlements())         # global 定居点 master (area/capacity/resources)
print(q.ships_spec())          # static rules: ship class -> spec (hull/upkeep/build_points/…)
print(q.resource_value())      # resource key (可读名) -> value
print(q.yearly_avg('metrics.cities'))      # 年均 (1 回合 = 1 月, 12 月/年)
print(q.decadal_avg('metrics.factions.中国.market_value'))  # 十年均 (120 月)
"
```

## Reading a faction (diplomacy + economy + fleet in one call)

```python
q = planet_xq.load("out")
snap = q.faction_snapshot(12, "中国")
snap["resources"]              # {resource: amount} 库存
snap["relations"]              # {faction: rel} 两两外交关系
snap["metrics"]                # city_count / ship_count / production_value / upkeep / …
snap["city_ids"], snap["ship_ids"]  # it owns these cities / ships (names)

# buildable insight: which of my shipyards make what
cities = q.city_buildings(12, "中国")
ships = q.fleet(12, "中国")[["ship_id", "class", "attack", "attack_range", "upkeep"]]
```

Key ideas:

- **eager fields** are inline in `main.jsonl`; an agent reads them as the lightweight decision view.
- **lazy fields** are NOT inline. `main.jsonl` carries their id-array (`ship_ids`/`city_ids`/
  `faction_ids`/`body_ids`); the full objects live in the `idx/*.jsonl` table keyed by id.
  `q.join(field, round)` explodes the id-array and merges the detail in one call.
- **per-round** lazy tables merge on `(round, key)`; the global `bodies` / `settlements` masters
  merge on `key` only.
- **factions** is the diplomacy + economy table: per-faction `resources` (stockpile), `relations`
  (toward every other faction), and the faction's own `city_ids`/`ship_ids`. `faction_snapshot(r, name)`
  merges it with the lean `metrics.factions` numbers into one read.
- **ships** carries the effective panel: `attack`, `attack_range`, `speed`, `accel`, `hardness`,
  `intercept`, `shield_regen`, `hull_regen`, `upkeep`, plus `components`/`component_hp` — so an agent
  can plan/engage without recomputing.
- **rules** (the static tuning dictionary) live in `meta.json` and load as DataFrames: `q.meta`
  (raw dict), `q.ships_spec()` / `q.buildings_spec()` / `q.components_spec()` /
  `q.structures_spec()` (index = spec name/key; nested `build_cost`/`cost` stay as dict-valued
  cells), and `q.resource_value()` (resource → value). This lets an agent cross-query rules against
  world facts in the same pandas session, e.g. `q.ships_spec().merge(q.join('ships', round=r)).`
- **mean / resample**: `q.metric_series(path)` gives a monthly Series (indexed by `round`);
  `q.window_avg(path, size, agg='mean')` averages it into windows of `size` rounds (1 round = 1
  month, so `size=12` is 年均 and `size=120` is 十年均), indexed by each window's start round.
  `q.yearly_avg(path)` / `q.decadal_avg(path)` are the 12 / 120 shortcuts. `path` is dotted and
  may be a world metric (`metrics.population`, `metrics.cities`) or per-faction
  (`metrics.factions.中国.production_value`).

> **Playing loop** (the whole reason this kit exists): `planet_x --seed S --index out/` →
> read `faction_snapshot` → write a control diff → `planet_x --start ckpt.ron --apply diff.json
> --round K --save ckpt.ron` → re-read → adjust. Checkpoints preserve the RNG, so the run is
> deterministic and rewindable.

## Semantic views: the "common read" in Python, "game logic" in Rust

The split is a hard boundary. **Python views only pack what the simulation already computed and
wrote into `metrics` / the lazy tables** (pure retrieval, can never drift from the rules). **Any
judgement that needs a game formula stays in Rust** (`--control-plan` for the economy-sustainability
verdict; governance-distance / power-share / coalition are computed & emitted by the sim itself).

```python
q.view_sitrep(12)                # world politics: totals + hegemon/coalition/sanction/wars/power_share + per-faction
q.view_frontier(12, "中国")       # my risky cities (loyalty / gov_distance / revolt_risk), sorted by loyalty
q.view_frontier(12, min_loyalty=0.5)   # any city about to revolt
q.view_market(12, "中国")         # my stockpile valued at market prices (per-resource + total)
q.view_economy(12, "中国")        # production vs upkeep vs governance, net flow, coverage, bleeding flag
```

- `view_sitrep` / `view_frontier` / `view_market` / `view_economy` are all **pure retrieval** —
  they re-read `metrics`/lazy tables and do trivial arithmetic (`net = production − upkeep −
  governance`).
- The one thing they deliberately **don't** judge is *"is my commanded build budget sustainable?"*
  — that's `planet_x --control-plan <faction>` (game logic: the upkeep-reserve cap + a dry-run
  `advance`). Python `view_economy` gives the raw flow; the verdict comes from Rust.
- `revolt_risk` / `gov_distance` on each city are game-derived and **emitted by the simulation**
  (`agent::governance_distance`), so the Python frontier view stays correct without re-deriving the
  governance distance formula.
