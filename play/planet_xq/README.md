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
"
```

Key ideas:

- **eager fields** are inline in `main.jsonl`; an agent reads them as the lightweight decision view.
- **lazy fields** are NOT inline. `main.jsonl` carries their id-array (`ship_ids`/`city_ids`/
  `body_ids`); the full objects live in the `idx/*.jsonl` table keyed by id. `q.join(field, round)`
  explodes the id-array and merges the detail in one call.
- **per-round** lazy tables merge on `(round, key)`; the global `bodies` master merges on `key` only.
