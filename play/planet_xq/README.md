# planet_xq

Read a `planet_x --index` projection into pandas.

The Rust generator projects a run into a **lean main stream** + **id-indexed lazy tables** so the
agent's default view stays small (only `metrics` + id-arrays are inline). Use this kit to fetch a
heavy field (a ship's full object, a city's details) by id, and to join it back to the main facts.

## Layout

```
planet_x --seed 7 --round 12 --index out/
# out/main.jsonl        lean per-round facts (round, time_month, events, chronicle, metrics,
#                       ship_ids[], city_ids[], body_ids[])
# out/schema.json       agent-readable projection contract (eager / lazy / columns / read_order)
# out/meta.json         static rules dictionary (ships/buildings/components/structures/economy/…,
#                       same source as `planet_x --meta`) — loadable as DataFrames
# out/idx/ships.jsonl   (round, ship_id, ...) per-round ship detail
# out/idx/cities.jsonl  (round, city_id, ...) per-round city detail
# out/idx/bodies.jsonl  (body_id, ...) global master table
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
print(q.ships(round=10))       # ships at round 10 (from the index)
merged = q.join('ships', round=10)   # explode main ship_ids and merge with ship detail
print(merged[['ship_id','class','hull','x','y']])
print(q.bodies())              # global master table
print(q.ships_spec())          # static rules: ship class -> spec (hull/upkeep/build_points/…)
print(q.resource_value())      # resource key (可读名) -> value
print(q.yearly_avg('metrics.cities'))      # 年均 (1 回合 = 1 月, 12 月/年)
print(q.decadal_avg('metrics.factions.中国.market_value'))  # 十年均 (120 月)
"
```

Key ideas:

- **eager fields** are inline in `main.jsonl`; an agent reads them as the lightweight decision view.
- **lazy fields** are NOT inline. `main.jsonl` carries their id-array (`ship_ids`/`city_ids`/
  `body_ids`); the full objects live in the `idx/*.jsonl` table keyed by id. `q.join(field, round)`
  explodes the id-array and merges the detail in one call.
- **per-round** lazy tables merge on `(round, key)`; the global `bodies` master merges on `key` only.
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
