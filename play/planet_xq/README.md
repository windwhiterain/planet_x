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
# out/main.jsonl        lean per-round facts (round, time_month, chronicle, metrics,
#                       event_ids[], ship_ids[], city_ids[], faction_ids[], body_ids[],
#                       settlement_ids[])
# out/schema.json       agent-readable projection contract (eager / lazy / columns / read_order)
# out/meta.json         static rules dictionary (ships/buildings/components/structures/economy/…,
#                       same source as `planet_x --meta`) — loadable as DataFrames
# out/idx/events.jsonl     (round, seq, event_id, ...) the sparse event log: one row per event,
#                          normalized participant slots + a human-readable `headline`
#                          (see "History" below)
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

## History: the sparse event log, and the three storage tiers

A trajectory answers *"what is the world now"*. The event log answers **"why is it like this"** —
which event made this city change hands, which ship killed that one. `idx/events.jsonl` is one row
per event with **normalized participant slots**, so any entity joins its own history the same way:

```python
q = planet_xq.load("out")

q.events(round=47)                  # every event of one round (long form: one row per event)
q.events(type="city_razed")         # ONE type -> payload flattened into DENSE columns
q.events(salience="detail")         # filter by *storage tier*, not by importance (see below)

q.history("city", "冥王星前哨")      # ★ everything that ever happened to one entity
q.history("ship", "长征-7")          #   any kind: city / ship / faction / body / settlement
q.history("faction", "中国", since=40, types=["war_started"])

q.cause("ship", "星环")              # ★ {'death_cause':'combat', 'killer':'天工',
                                    #    'killer_faction':'中国', 'weapon':'kinetic',
                                    #    'assists':['镇岳','长城'], ...}
q.cause("city", "冥王星前哨")         # the last ownership/death event ("被夷平后复垦，还是倒戈？")

q.fates(kind="ship")                # every ship death in the window: cause + killer + weapon
q.fates(kind="city", since=40)      # every city ownership/death event in the window

q.actors()                          # long-form (round, seq, event_id, kind, id, role) index
q.changes("city", "冥王星前哨")      # pure dense-diff of the snapshot table (independent cross-check)
q.audit()                           # completeness self-check: unexplained city changes (want 0)

q.notables()                        # the *windowed* tier: what the sim looks back at (wars)
q.milestones()                      # the *unbounded* tier: EMPTY by the current criterion (see below)
q.storyboard(window=100)            # ★ top events by `weight`, compressed to one row per 100 rounds
```

### `salience` is a storage tier, not importance — and `weight` is importance

Every row carries a **`headline`**: one human-readable sentence, rendered by the Rust side's single
`GameEvent::headline()` — the *same* sentence CLI `--digest` shows, so the surfaces can never
disagree. It is deliberately **self-contained**: built only from the event's own fields, never by
looking the entity up in today's state — an entity in old history may be long dead or renamed since,
so re-reading the world would produce *today's* answer, not the true one. It is also guaranteed to
name **every** participant that the query index points at (a Rust guard pins this:
`headline_names_every_participant`).

Two columns that are easy to confuse, and are deliberately different things:

| column | question it answers | who defines it |
|---|---|---|
| `salience` | **which storage tier holds this event** — i.e. does later simulation logic need to look back at it, and how far? | the criterion in `Salience` (`milestone` = unbounded past / `notable` = a bounded window / `detail` = previous frame only, or nobody) |
| `weight` | **does a human/agent care when reading?** — an **ordinal 0–9 ladder** (9 = wars/coalitions/capital moves, 8 = city ownership & existence, 7 = resurgence/story, 5 = ship birth & death, 2 = withdraw/stale order, 0 = per-shot fire/siege) | `GameEvent::weight()`, a display-only ranking key |

So **do not filter by `salience` to find "the interesting events."** It will pick wrong, and in the
current state it picks *nothing*: the criterion is reader-driven, and a reader inventory shows **no
simulation logic reads unbounded history**, so no variant is `milestone` — `q.milestones()` returns
an empty frame, correctly. The only `notable` variants today are `war_started` / `war_ended`, kept
because the "grudge floor" (`sim::war_scar_floor`) looks back `history.notable_window` rounds.

Pick by **`weight`** instead (`q.storyboard()` does, with a `min_weight=8` threshold — remember the
scale is 0–9, not 0–100), or just read
one entity's whole life — that needs no ranking at all:

```python
q.history("city", "大红斑科学站")   # every event naming that city, oldest first
```

```
 round         type                                      headline
     4  city_defected        无国界科学组织 的 大红斑科学站 倒戈至 星系矿业（忠诚 0.2）
    11     city_razed   中国 的 北斗 夷平 联合国 的 大红斑科学站（人口 240 → 0，3.1 伤害）
    11 colony_founded                  中国 在 木星 复垦 联合国 留下的废墟 大红斑科学站
    33     city_razed  欧盟 的 联盟 夷平 联合国 的 大红斑科学站（人口 240 → 0，19.1 伤害）
    33 colony_founded                 联合国 在 木星 复垦 联合国 留下的废墟 大红斑科学站
```

(headline 只给人读；机器查询仍走 `actor_*` / `target_*` / `data` 这些结构化列。)

**The two in-state tiers survive checkpoints; the projection does not necessarily.** `State::notables`
holds the windowed tier and `State::milestones` the unbounded one, both written by the single event
funnel (`sim::ev`), so an event cannot be emitted without entering the tier it belongs to. The window
tier reports its **window width** rather than a drop count — expiry there is by design, not loss:

```
planet_x --start ckpt.ron --round 0 --notables 40    # what this save is currently at war over
planet_x --start ckpt.ron --round 0 --milestones     # count: 0 — by criterion, see above
```

> **The Python readers read the projection, not the in-state tiers** — those agree only for a
> *single-segment* run. `--index` truncates its directory (`File::create`), so after a segmented run
> (`--round 30 --save ckpt` then `--start ckpt --round 30`) the second dir covers only its own
> rounds: measured, `load("seg2").events()` = 357 rows (rounds 30→60) vs 671 for the whole run. To
> reconstruct the whole run in Python, union and dedupe on `event_id`:
>
> ```python
> pd.concat([load("seg1").events(), load("seg2").events()]).drop_duplicates(subset=["event_id"])
> # 671 events — the two segments together
> ```
>
> Forgetting `drop_duplicates` **silently double-counts the seam round** (the two dirs overlap on
> round 30 with identical `event_id`s — 9 rows here). Per-shot detail (`attack`/`siege`) only exists
> in the segment that produced it.


> The event log is backed by **structural funnels** in the simulation (`kill_ship` / `spawn_ship` /
> `raze_city` / `reseed_city` / `found_city` / `overrun_city` / `defect_city`): every ownership or
> existence change goes through one place that *both* mutates the state and records the event, so a
> new code path cannot silently skip the history. Two Rust guards — and `q.audit()` — verify it
> against the dense tables on every test run (measured: 242 ship deaths / 249 ship births / 145 city
> ownership changes over 120 rounds, **all** explained).

Why the shape is what it is (each point measured on a real 715-event projection):

| | serialized tagged union (old) | normalized event log (now) |
|---|---|---|
| mean null ratio | **74.8%** | **4.7%** |
| variant-specific columns | 23 | **0** |
| one role, many spellings | `faction`/`owner`/`fallen_to`/`from`/`to`/`a`/`b` | 1 (`actor_id` + `target_id` + `extra`) |
| one column, two meanings | `from`/`to` = faction in `city_defected`, **body** in `capital_relocated` | none |

- **Long form, not wide**: "missing" means *no such row*, never NaN. So sparse-field statistics
  (counts, per-window rates, first/last, fates) are one-liners — `groupby(...).size()`.
- **Filter by type first.** Mixing all types in one frame is what makes it sparse
  (`q.events(type='city_razed')` → its fields are 0% null); the full frame is for counting/scanning.
- **No variant-specific columns.** The per-type payload lives in one `data` object column (one
  column carries one type of value — no column that is sometimes a scalar and sometimes a list,
  which makes `isna()`/`sum()`/`dropna()` unreliable).
- **Never re-derives game logic**: the event log records what the simulation *did*
  (`CityRazed.by_ship` + `CityRazed.owner` = who razed it and who lost it, `ShipDestroyed.by` = the
  killing blow, `ColonyFounded.how`/`prev_owner`, `DeathCause` = combat vs upkeep-shortfall). Facts
  that only exist *at that moment* must be recorded then — e.g. "who lost this city" cannot be
  recovered later, because a razed city keeps its last owner as a diaspora claim only until someone
  re-founds it the same round.
- `q.audit()` mirrors the Rust guard `every_city_state_change_is_explained_by_an_event`: if a
  snapshot-visible city change has no explaining event, it is listed. Empty = the history is
  complete, so "why did this city change hands?" always has an answer.

> Note: `events` is no longer inlined in `main.jsonl` (the round row now carries `event_ids`).
> Read it via `q.events(...)`, not `q.facts["events"]`.

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
q.resource_series("中国", "铁")     # my 铁 stockpile over time (monthly, indexed by round) — e.g. is it being drained?
```

- `view_sitrep` / `view_frontier` / `view_market` / `view_economy` / `resource_series` are all
  **pure retrieval** — they re-read `metrics`/lazy tables and do trivial arithmetic
  (`net = production − upkeep − governance`).
- The one thing they deliberately **don't** judge is *"is my commanded build budget sustainable?"*
  — that's `planet_x --control-plan <faction>` (game logic: the upkeep-reserve cap + a dry-run
  `advance`). Python `view_economy` gives the raw flow; the verdict comes from Rust.
- `revolt_risk` / `gov_distance` on each city are game-derived and **emitted by the simulation**
  (`agent::governance_distance`), so the Python frontier view stays correct without re-deriving the
  governance distance formula.
