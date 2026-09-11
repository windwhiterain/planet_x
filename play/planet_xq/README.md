# planet_xq

Read a `planet_x --index` projection into pandas.

The Rust generator projects a run into a **lean main stream** + **id-indexed lazy tables** so the
agent's default view stays small (only the round's `view` + id-arrays are inline). Use this kit to
fetch a heavy field (a ship's full object, a city's details, a faction's stockpile + relations) by
id, and to join it back to the main facts. **Entity ids are always string names** (ship/city/
building/faction/body/settlement = its unique name), never integer offsets.

## The round view: one shape, twice

A round's world is **one object, `RoundView`**, and the projection hands you it twice with the
**same shape**:

- `pre` — the world at the round's **start** (the process quantities are 0/empty there);
- `post` — the world at the round's **end** *plus* that round's **process quantities**
  (production / upkeep / governance / trade / the AI's judgments).

`main.jsonl` carries `post` as the row's **`view`** key, flat and eager: `view.city_count` /
`view.ship_count` / `view.fleet_value` / `view.population`, the politics block
(`view.power_share` / `view.hegemon` / `view.coalition_members` / `view.sanctioned` / `view.wars`),
the market tables (`view.market_price` / `view.market_settled` / `view.market_offered`), one row
per faction (`view.factions[<势力>]`) and per city (`view.cities[<城>]`), and the AI's own
judgments (`view.decisions`). Any dotted path below starts at `view` for exactly this reason.

## Layout

```
planet_x --seed 7 --round 12 --index out/
# out/main.jsonl        lean per-round facts (round, time_month, chronicle, view,
#                       event_ids[], 舰名表[], 城名表[], 势力表[], 天体名表[], 定居点表[])
# out/schema.json       agent-readable projection contract (eager / lazy / columns / read_order)
# out/meta.json         static rules dictionary (ships/buildings/components/structures/economy/…,
#                       same source as `planet_x --meta`) — loadable as DataFrames
# out/idx/events.jsonl     (round, seq, event_id, ...) the sparse event log: one row per event,
#                          normalized participant slots + a human-readable `headline`
#                          (see "History" below)
# out/idx/ships.jsonl      (round, 舰名, ...) per-round ship detail (+ effective panel)
# out/idx/cities.jsonl     (round, 城名, ...) per-round city detail (+ 建筑 list)
# out/idx/factions.jsonl   (round, 势力, ...) per-round faction detail (资源/关系/
#                           城名表 + 舰名表)
# out/idx/bodies.jsonl     (天体名, ...) global master table
# out/idx/settlements.jsonl (定居点, ...) global 定居点 master
```

`schema.json` is the single contract both Rust and this kit share: it declares which fields are
**eager** (inline in `main.jsonl`) vs **lazy** (in a table keyed by id), plus each lazy field's
table / key / id-column / whether it is per-round, and the column types — so an agent can navigate
without reverse-engineering the JSON. Since the **derived** tables landed it also carries a
`derived` section (see below): quantities the **engine computed** rather than state that it stored.

## Derived tables: what the engine computed (not state it stored)

`idx/faction_process.jsonl`, `idx/city_process.jsonl`, `idx/control.jsonl`, `idx/scope.jsonl`,
`idx/decisions.jsonl`, `idx/blueprints.jsonl` are **not** lazy
fields: they are not reached by exploding an id-array from `main.jsonl`, because their data
**is not in the state at all** — it is what the round's step functions computed and applied
(production, fleet upkeep, governance cost/coverage), the control surface (who owns which
leaf), **the AI's own judgments** (`decisions`: why a ship withdrew / engaged / did nothing,
and which shipyard was retooled), and the **ship-blueprint library** (`blueprints`: one row per
design — class / components / default order / ownership / how many ships came off it). Their
schema entry therefore carries `join_on` (a column that
*already exists* in `main.jsonl`, usually `势力表`/`城名表`) instead of `id_col`.

```python
q = planet_xq.load("out")
q.derived("faction_process")   # generic accessor: q.derived(name, round=None)
q.faction_process(round=12)    # per-round × faction: production{} / upkeep / governance_total / governance_coverage
                               #   + the B1 split: governance_admin / governance_entertainment /
                               #     governance_scale / ideology_loyalty_penalty
q.city_process(round=12)       # per-round × city: production{} (razed cities included, `razed` column)
                               #   + loyalty_target_* : WHY this city's loyalty is dropping
q.control(round=12)            # one row per control leaf: kind / key / sub / value / mode
                               #   ⚠ `kind` 是**控制叶的中文名词**（= --control-schema 的 leaves[].field）
q.scope(round=12)              # explicit scope nodes only: level (global/faction/body/city) / key / mode
q.decisions(round=12)          # one row per AI judgment: kind / actor / verdict / target / detail
                               #   kind: ship_order / retool / style_retune / blueprint / capital
                               #   (capital rows are sparse: only review rounds and relocations)
q.blueprints(round=12)         # one row per design blueprint: class / components / order / mode / effective_mode / …
q.market_trades(round=12)      # ONE ROW PER REALIZED TRADE (buyer × seller): moved{} / dist_au / depth /
                               #   mond_extra / freight_rate / rel_mult / mastery / loss  — sparse (no trade ⇒ no row)
q.haul_steps(round=12)         # ONE ROW PER SHIP THAT RAN A HAUL ROUTE: step / body / units / into_pool
q.salvos(round=12)             # ★ PER-SHOT combat breakdown (B4): one row per weapon per shot —
                               #   WHY it aimed there (score_basic / score_temper / score_spread → score)
                               #   + WHAT it did (hit / def_mult / pd / pd_absorbed / absorbed / soak /
                               #     armor_soak / hull_pen / damage / killed / skipped)
q.round_inputs(round=12)       # ★ THE INPUT FACE (B5): what the round *consumed* rather than produced —
                               #   order (C7: the per-ship resolution order) / relation_noise (C13) / rolls
```

### 批量读：`load(dir, only=(...))`

一份 1000 回合的投影有 ~170 MB，**全装 ≈ 12 s**（`round_inputs` / `control` / `blueprints`
就占一半多），而多数问题只碰两三张表。`only` 只装点名的表（没装到的表访问时会明确报
「这次没装」，不会让人误以为投影里没有）：

```python
q = planet_xq.load("out", only=("events", "ships", "factions"))   # 长局扫描的常用组合
```

另外：**浮点解析是精确的**（`precise_float=True`，不是默认值）。pandas 默认的快速解析器对
某些需要 16 位有效数字的值会落到**相邻的 double** 上（实测 `1.989510667009421` 读成
`1.9895106670094211`）——那正是引擎开 serde_json `float_roundtrip` 要掐掉的漂移。

**Two faces, one round (B5).** `view` is the **settled** face (observations + the round's process
quantities); `round_inputs` is the **consumed** face — the dice the engine rolled and the inputs its
judgments saw. The rule is by *kind*, not by which values exist today: anything that is (or could
later become) an input or a roll lives on the input face, everything the round produced lives on the
settled face. Two things this buys you:

- **`order` answers "why did my ship die without firing once"**: ships resolve in a shuffled order
  (`main Prng`, recorded per round); a ship that is resolved *after* the one that sank it never gets
  its shot. ⚠ the list is the ship set **at shuffle time** — it can include a ship sunk later that
  same round and omit one launched later, so it may be longer than the round's closing roster.
- **`relation_noise` answers "why did that relation wobble for no reason"**: `aff` and the drift
  rate are deterministic; the ±noise roll is the only unmotivated move.
- **`rolls` answers "why did nobody take my contract / why was that ship made a freighter"**: each
  entry is one `derived_roll` draw with **the value rolled and the bar it was compared against**.
  Gates (`threshold` set, `picked` = which branch: `heard`/`unheard`, `willing`/`unwilling`,
  `assigned`/`skipped`, `quit`/`kept`, `good`/`poor`, `renewed`/`replaced`,
  `freight`/`war`/`observe`) vs weighted draws (`pool_total` set, `picked` = the lane/faction
  chosen). Only decisions are recorded — the same die is often also consulted by an *estimate*
  (e.g. sizing your own hauling capacity), and that is not a decision.

⚠ The input face is **not** in `main.jsonl` and **not** in `--derived`'s `post` — it is the
`pre` side (what `--derived` prints as `pre`, and what `round_inputs` holds in an `--index`
projection). Empty means the round never ran (round 0 / a `--start` row), not "rolled a zero".

`decisions` is the one table that answers "**why** did my ship do that": `verdict` is one of
`withdraw` / `engage` / `colonize` / `bombard` / `move` / `hold` (`hold` = the AI **did not give
this ship an order** this round — not "it is idling"), and `detail` carries the inputs that decided
it (`hull_ratio` vs `retreat_hull`, `kiting`, `enemy_in_range`). A ship can appear **twice** in one
round (it moved, then re-judged on arrival) — check `detail.after_move` before aggregating.

`blueprints` is the yard's catalog of designs for ships that **do not exist yet**: a design is
`class` + `components` (empty = the generator picks at launch) + the default `order` a new ship
inherits. `mode` is that design leaf's own three-state opinion and `effective_mode` is the engine's
resolved ownership (leaf → faction scope → global); `ship_count` says how many hulls came off it,
and `launch_waiting` flags the "progress is full but the components cannot be paid for" case (a
`Player`-owned design never silently overdraws the stockpile). Join the ships side on `q.ships()`'s
`blueprint` column (which design printed this hull; `null` = none) and the yard side on the inline
`buildings[].blueprint` in `q.cities()`.

`salvos` is the **B4 payoff** ("why did that shot do almost nothing / why was my missile volley eaten
alive"): `attack` events carry a per-shot record, and this flattens it. `命中折减` is the engine's
**deterministic** tracking reduction (`hit_factor(weapon tracking, target speed)` — not a dice roll),
`点防拦截`/`点防吃掉` is point defence (own `intercept` + nearby friendly ships' screen),
`护盾吸收`/`护盾吸收比` is the shield layer, `护甲减伤` the hardness reduction, `实入船体` what actually
reached the hull.
`未击发=True` means that shot **never fired** (its target was already dead when its turn came).
⚠ two traps: (1) an `attack` event with **`magnitude == 0` is real** — a salvo fully intercepted by
point defence leaves no damage, and before B4 it left **no event at all**, so "no attack rows" never
meant "no shooting"; (2) `Σ 逐发[].伤害` equals the event's `magnitude` only up to the table's
2-decimal rounding of `magnitude` (it is exact in the engine's own state).

Two things worth knowing:

- **B4 put the combat internals in the *event* layer, not in the per-faction view** (a deliberate
  ruling, see `step-intermediates.md` §7 Q1): the per-shot detail rides in
  `events[type='attack'].data.逐发`, so `main.jsonl` pays only the event *id* (~7 B) instead of
  every row carrying the numbers. Consequences to know: **`--derived` does not include events**, so
  a checkpoint's `post` view has no shot data — read it with `q.salvos()` / `q.events(type='attack')`
  / `q.history('ship', …)` instead. The world itself is unchanged: only **damage > 0** adjusts
  relations or counts as "engaged" for war-fatigue purposes.
- **The same process numbers are also inside `main.jsonl`** as the nested
  `view.factions[<faction>]` object (`production`, `production_value`, `upkeep`,
  `governance_cost`, `governance_coverage`, the **B1 governance split** `governance_admin` /
  `governance_entertainment` / `governance_scale` / `ideology_loyalty_penalty` /
  `capital_loyalty_bonus`, the trade terms `freight_paid` / `carrier_income` / `net_import`, and the
  **B2 money** terms `investment_spent` / `construction_spent` / `upkeep_unpaid` / `fleet_rust`);
  the per-city ore output, the **`loyalty_target` object** and the **B2 build terms** `labor` /
  `housing_capacity` / `is_hub` / `build` likewise sit in `view.cities[<city>]`
  (that map skips razed cities, while `city_process` keeps their rows with `production = {}`).
  These tables are the **joinable reshape** of the same numbers (stable dtypes, one row per
  `(round, name)`), which is what you want for pandas work.
  Two shape rules worth knowing:
  **①「one number, one place」**: the two faction-wide terms of the loyalty equation
  (`capital_loyalty_bonus`, `ideology_loyalty_penalty`) are stored **once per faction** — city rows
  carry only the per-city terms. `q.view_loyalty()` joins them back for you.
  **② sparse judgments live in `decisions`**, not in the per-faction row: the capital review /
  relocation (`kind="capital"`) only exists on rounds where something happened
  (`11/12` rounds have no row at all — read `q.view_economy(round, f)["capital"]`, which is `None`
  then, instead of expecting a per-faction object full of `null`s).
  **③「what was spent」 is in the view, 「what was granted」 is in `control`** (B2): the batch limits
  are persistent control leaves (`投资预算` / `建造预算` —— **中文叶名**，one row per resource in
  `derived.control`), and the view carries only `investment_spent` / `construction_spent`. Subtract
  them to get "granted but not spent" — `q.view_spending(round, f)["budget"]` does the join for you.
  Storing the limit in the view too would be a second copy of the same number.
  **④ B3 — a trade row is a (buyer, seller) pair, not a (pair × resource) row**: everything in
  `market_trades` except `moved` depends only on the pair (distance / anomaly depth / relation), so
  the effective price of *any* resource on that deal is
  `view.market_price[resource] × (rel_mult + freight_rate)` (`q.view_trade()` hands you that sum as
  `price_mult`). `moved` is what the **seller gave up** — the buyer receives
  `moved[rt] × (1 − loss)`, which is exactly what `view.market_settled` totals world-wide.
  **⑤ B3 — `haul_steps` is per ship, and `waiting` / `en_route` have no other read path**: a hauler
  parked at an **empty depot** and a hauler that is simply *in transit* leave no trace in state and
  emit no event, so without this table "I sent it for cargo and nothing came back" is unanswerable.
  Both execution paths write it (the AI brain *and* player orders), so player ships are in there too.
  ⚠ the ledger's `own` can read **`-0.0`**: that is Rust's empty-sum sign bit and means "no own
  freighters available this round" (e.g. they are all hired out to someone else) — compare with
  `== 0.0`, not with a sign test.
- **过程量 — the round's production / upkeep / governance / trade / AI judgments — only exists for
  rounds the engine actually advanced; in `pre` it is 0/empty.** A projection started from a
  checkpoint (`--start ckpt.ron --round 0 --index out/`) therefore puts that checkpoint's **stored
  view** into the start row, because that row's state *is* the result of that round; a fresh `--seed`
  run has no process quantities at round 0 (`0` / `{}`) — the initial world has no previous round.
  **What "0/empty" means for each field is declared once**, in `schema.json`'s `neutral` section —
  `q.neutral("factions[].governance_scale")` is `1.0` (not 0: "no bill to pay" is not "governance
  capacity is zero"). Read a default from there rather than hardcoding one: the engine substitutes
  from the *same* declarations, and that is exactly what keeps two read faces from disagreeing
  (`flow.jsonl` used to say "0% covered" while `metrics` said "100% covered" for the same round).
- For **per-ship effective intent** read the `ships` table columns
  `order_leaf_mode` / `order_effective_mode` / `order_effective` / `order_source` /
  `风格` / `姿态` / `角色` / `role_mode` — the engine resolves the ownership chain, so
  **do not re-implement it** (a Python re-implementation is a drift source).
  ⚠ `角色` is a **three-valued string** (`"War"` 战舰 / `"Freight"` 运输舰 / `"Observe"` 观测舰);
  it replaced the old **boolean** `freighter` column (and `freighter_mode` → `role_mode`), so a
  recipe that filtered `df["freighter"] == True` must now filter `df["角色"] == "Freight"`.
  `role_mode` is that leaf's effective ownership (`Auto` = the automatic controller wrote this
  conclusion, `Player` = a player pinned it).


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
print(q.faction_snapshot(10, '中国'))   # one-call decision view (view row + stockpile + relations)
print(q.ships(round=10))       # ships at round 10 (from the index), with effective panel
print(q.city_buildings(10, '中国'))     # that faction's cities, each with a buildings list
merged = q.join('ships', round=10)   # explode main 舰名表 and merge with ship detail
print(merged[['舰名','舰级','船体','x','y','attack','upkeep']])
print(q.bodies())              # global master table
print(q.settlements())         # global 定居点 master (area/capacity/resources)
print(q.ships_spec())          # static rules: ship class -> spec (hull/upkeep/build_points/…)
print(q.resource_value())      # resource key (可读名) -> value
print(q.yearly_avg('view.city_count'))     # 年均 (1 回合 = 1 月, 12 月/年)
print(q.decadal_avg('view.factions.中国.market_value'))  # 十年均 (120 月)
"
```

## Reading a faction (diplomacy + economy + fleet in one call)

```python
q = planet_xq.load("out")
snap = q.faction_snapshot(12, "中国")
snap["resources"]              # {resource: amount} 库存
snap["relations"]              # {faction: rel} 两两外交关系
snap["view"]                   # this faction's row of the round view:
                               # city_count / ship_count / production_value / upkeep / …
snap["城名表"], snap["舰名表"]       # it owns these cities / ships (names)
snap["observer_quota"]             # 观测配额（目标头数）：该派几艘舰去 MOND 异常区蹲着喂掌握度
snap["observer_count"], snap["observer_target"]   # 现在真在观测的舰数 / 编队驻地天体
snap["freighter_quota"], snap["freighter_count"]  # 集货那条同形的一对（目标条数 / 现状条数）

# buildable insight: which of my shipyards make what
cities = q.city_buildings(12, "中国")
ships = q.fleet(12, "中国")[["舰名", "舰级", "attack", "attack_range", "upkeep"]]
```

Key ideas:

- **eager fields** are inline in `main.jsonl`; an agent reads them as the lightweight decision view.
- **lazy fields** are NOT inline. `main.jsonl` carries their id-array (`舰名表`/`城名表`/
  `势力表`/`天体名表`/`定居点表`/`event_ids`); the full objects live in the `idx/*.jsonl` table keyed by id.
  `q.join(field, round)` explodes the id-array and merges the detail in one call.
- **per-round** lazy tables merge on `(round, key)`; the global `bodies` / `settlements` masters
  merge on `key` only.
- **factions** is the diplomacy + economy table: per-faction `resources` (stockpile), `relations`
  (toward every other faction), and the faction's own `城名表`/`舰名表`. `faction_snapshot(r, name)`
  merges it with that faction's row of the round `view` (`view.factions[<faction>]`) into one read.
  It also carries the engine's own **编队配额** columns (per faction, per round — read them instead of
  re-deriving the automatic controller's judgements; the full column docs are in `schema.json`):
  `observer_quota` (观测配额 = **target head count** of ships that should sit in the MOND anomaly
  band; `0` once 掌握度 is maxed out), `observer_count` (ships actually observing right now, i.e.
  effective `role == "Observe"` — read it next to the quota to tell 「不想学」 from 「没人可派」),
  `observer_target` (the band body the observer flotilla garrisons; `null` = no candidate),
  `mond_ships_in_band`, and the freight twin `freighter_quota` / `freighter_count`.
- **ships** carries the effective panel: `attack`, `attack_range`, `speed`, `accel`, `hardness`,
  `intercept`, `shield_regen`, `hull_regen`, `upkeep`, plus `components`/`component_hp` — so an agent
  can plan/engage without recomputing. It also carries the three style axes the engine resolved:
  `doctrine` / `kiting` / `role` (＋ that leaf's ownership in `role_mode`), where `role` is the
  string `"War" | "Freight" | "Observe"` (战舰 / 运输舰 / 观测舰).
- **rules** (the static tuning dictionary) live in `meta.json` and load as DataFrames: `q.meta`
  (raw dict), `q.ships_spec()` / `q.buildings_spec()` / `q.components_spec()` /
  `q.structures_spec()` (index = spec name/key; nested `build_cost`/`cost` stay as dict-valued
  cells), and `q.resource_value()` (resource → value). This lets an agent cross-query rules against
  world facts in the same pandas session, e.g. `q.ships_spec().merge(q.join('ships', round=r)).`
- **mean / resample**: `q.metric_series(path)` gives a monthly Series (indexed by `round`);
  `q.window_avg(path, size, agg='mean')` averages it into windows of `size` rounds (1 round = 1
  month, so `size=12` is 年均 and `size=120` is 十年均), indexed by each window's start round.
  `q.yearly_avg(path)` / `q.decadal_avg(path)` are the 12 / 120 shortcuts. `path` is dotted and
  may be a world metric (`view.population`, `view.city_count`) or per-faction
  (`view.factions.中国.production_value`).

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
wrote into the round's `view` / the lazy tables** (pure retrieval, can never drift from the rules).
**Any judgement that needs a game formula stays in Rust** (`--control-plan` for the
economy-sustainability verdict; governance-distance / power-share / coalition are computed & emitted
by the sim itself).

```python
q.view_sitrep(12)                # world politics: totals + hegemon/coalition/sanction/wars/power_share + per-faction
q.view_frontier(12, "中国")       # my risky cities (loyalty / gov_distance / revolt_risk), sorted by loyalty
q.view_frontier(12, min_loyalty=0.5)   # any city about to revolt
q.view_loyalty(12, "中国")        # WHY each city's loyalty is dropping (engine's loyalty-target split)
                               #   = 3 per-city columns + the 2 faction-wide ones joined in
q.view_market(12, "中国")         # my stockpile valued at market prices (per-resource + total)
q.view_economy(12, "中国")        # production vs upkeep vs governance (+ admin/entertainment split), net flow,
                               #   coverage, purchasing power & buying rank, haul_gap, "who won't sell to me"
q.view_spending(12, "中国")       # WHERE THE MONEY WENT: batch − spent per resource, build bottleneck, fleet rust
q.view_trade(12, "中国")          # WHY THIS PRICE / WHY MY CARGO SHRANK: per-pair price split + loss, blocked list
q.view_freight(12, "中国")        # WHICH DEPOT IS BACKED UP (need/own/hired/uncovered) + what each hauler did
q.resource_series("中国", "铁")     # my 铁 stockpile over time (monthly, indexed by round) — e.g. is it being drained?
```

- `view_sitrep` / `view_frontier` / `view_loyalty` / `view_market` / `view_economy` /
  `view_spending` / `view_trade` / `view_freight` / `resource_series` are all **pure retrieval** —
  they re-read the round `view` (mostly `view.factions[<faction>]`) / the lazy + derived tables and do
  trivial arithmetic (`net = production − upkeep − governance`, `unspent = batch − spent`,
  `price_mult = rel_mult + freight_rate`).
- `view_loyalty` is the **B1 payoff**: one row per city with the engine's own loyalty-target split —
  `loyalty_target_distance` (too far from the capital, amplified by population overload) and
  `loyalty_target_entertainment` (that city's entertainment budget × governance coverage) per city,
  plus `capital_loyalty_bonus` (the capital's population share, a faction-wide buff) and
  `ideology_loyalty_penalty` (the faction-wide "monopolist ideology vs behaviour" penalty) joined
  from `faction_process`. The identity is
  `loyalty_target_effective = clamp(distance + entertainment + capital_loyalty_bonus − ideology_loyalty_penalty)`.
  Loyalty moves *toward* `loyalty_target_effective` **only while governance is covered**; when
  `coverage < 1` the city bleeds by a shortfall penalty instead, so read it together with `loyalty`
  and `view_economy(round, f)["governance_coverage"]`. Both `view_loyalty` and `view_economy` read
  numbers the **engine captured mid-step** — Python never re-derives the loyalty or cost formula.
- ⚠ `view_economy` returns **two nets**, and they are different things: `net_flow` is computed here
  (`production_value − upkeep − governance_cost`), while `net_import` is the engine's own **trade**
  net for the round (bought − sold by market value, `> 0` = net importer) lifted straight out of
  this faction's `view` row.
- `view_spending` is the **B2 payoff** ("批了钱为什么没花掉 / 造舰慢是缺钱还是缺产能 / 我的船为什么在掉血"):
  * `budget` — one row per resource, `investment_batch − investment_spent = investment_unspent`
    (same for `construction_`). The **batch** is a control leaf, the **spent** is a captured
    mid-step local; the join happens here because the read face deliberately stores only one of them.
  * `build` — one row per (city, class): `rate` is the capacity ceiling, `increment` the progress
    actually paid for, and `bottleneck` is the verdict — `money` (the budget ran out: `increment < rate`),
    `capacity` (money was left over but the shipyard/crew capped it), `idle` (capacity exists and
    **not one unit of budget reached it**). A class with no row means this city has no shipyard for it.
  * `upkeep_unpaid` / `fleet_rust` — the maintenance the faction could not pay, and the **hull fraction
    every ship lost** because of it (`hull_max × fleet_rust`). Rusting to zero is the only case that
    emits an event, so these two columns are the only place the bleeding is visible.
    ⚠ `fleet_rust` is **not** `upkeep_unpaid ÷ upkeep`: the engine has a visibility floor (a tiny
    shortfall still rusts 0.2), so read the column instead of recomputing it.
    ⚠ `upkeep_unpaid` is *"unpaid **and therefore rusting**"*, not the complement of "what was paid":
    a landless (exiled) faction is exempt from the stockpile drain, so its `upkeep_unpaid` is `0`
    even though it pays nothing — don't read `upkeep − upkeep_unpaid` as "what was actually paid".
- `view_trade` / `view_freight` are the **B3 payoff** ("为什么是这个价 / 我买到的货为什么少了 /
  有货在卖我却没买到 / 这趟货为什么没运回来 / 哪处货栈在积压"):
  * `trades` — one row per **realized** deal (`buyer` × `seller`), with the whole price split
    (`price_mult = rel_mult + freight_rate`, plus `dist_au` / `depth` / `mond_extra` / `mastery` /
    `loss`) and `delivered_units`. `loss > 0` means the shipment crossed an anomaly band and part of
    it **went missing** — that is a deterministic fraction, not a dice roll.
  * `blocked` — **who refuses to sell to me and why**: `cause` ∈ `war` / `cold` / `coalition`.
    The three tiers need different answers (make peace / warm relations / break the coalition), so a
    bare count would not have been enough.
  * `depots` — the per-depot capacity ledger: `need` (throughput required to clear that depot's
    backlog in one round-trip), `own` (this faction's own freighters' *expected* share of it — the
    dispatch is a lottery weighted by backlog share, so the expectation is the same yardstick),
    `hired` (contracted capacity: accepting a contract *is* a commitment) and
    `uncovered = max(0, need − own − hired)`. The faction-wide ratio is `haul_gap`
    (`Σuncovered ÷ Σneed`) — this table is that ratio broken down per depot, i.e. "which depot is
    rotting and why".
  * `steps` — what each of this faction's haulers did this round: `loaded` / `delivered` / `waiting`
    (parked at an **empty depot** — not a malfunction, it is "no cargo, no trip") / `en_route`.
    Both of the latter are invisible in state and emit no event, so this is their only read path.
- The one thing they deliberately **don't** judge is *"is my commanded build budget sustainable?"*
  — that's `planet_x --control-plan <faction>` (game logic: the upkeep-reserve cap + a dry-run
  `advance`). Python `view_economy` gives the raw process quantities; the verdict comes from Rust.
- `revolt_risk` / `gov_distance` on each city are game-derived and **emitted by the simulation**
  (`agent::governance_distance`), so the Python frontier view stays correct without re-deriving the
  governance distance formula.
