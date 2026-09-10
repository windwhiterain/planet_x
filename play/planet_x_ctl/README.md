# planet_x_ctl

Turn pandas analysis of a `planet_x` checkpoint into a **legal `--apply` control diff**.

This is the **write-side twin** of [`play/planet_xq`](../planet_xq/README.md) (the read-side kit).
`planet_xq` answers *"what is the world?"*; `planet_x_ctl` answers *"given that, what do I write?"*
— and then proves the engine accepted it.

```bash
cd play/planet_x_ctl
uv sync                              # .venv + pandas (works offline; the sibling kit's lock is enough)
uv run python demo.py                # self-asserting end-to-end run on a fixture it generates itself
uv run planet-x-ctl verify ckpt.ron steer.json    # read-only rehearsal of a diff
```

`demo.py` generates its own fixture (`--seed 7 --round 12`), then walks the whole loop and asserts
every step: bulk ownership `Auto` and back (including a real `--apply --save` round trip), a
statistical budget cap driven by `upkeep` / `production_value`, one deliberate fleet-default
takeover, a no-op write, the refusal guards, and byte-identical diffs on replay. It exits non-zero if
any assertion fails. `--planet-x PATH` overrides the engine; `--work DIR` keeps the scratch dir.

## Division of labour: engine = data plane, kit = policy plane

The line is a ruling, not a preference (`.agents/notes/engine-data-plane.md`):

> **The engine does exactly two things: it emits standard tidy data tables (joined by name), and it
> accepts a diff of the same shape. Wildcards, rosters, statistical selection, recipes and
> verification are all Python's job — the engine grows no features for them.**

| layer | owns | why |
|---|---|---|
| **engine** (chain + templates) | the rules for entities that **do not exist yet**: `leaf → fleet default → faction scope → global` ownership resolution, what a newly-built ship inherits, the implied **writing a value takes over** rule, ship blueprints | the kit can only speak about ships that exist *now*; "new ships inherit intent" is only true on a chain |
| **Python kit** (this package) | bird's-eye operations over entities that **do exist**: wildcards, name rosters, filtering, aggregation, recipes, `verify` | that is pandas' job; the engine should not grow a pile of one-shot verbs |

Two things are therefore **deliberately absent** from the engine, and present here instead:

* ❌ engine-side `{"ship": "*"}` wildcard → ✅ `s.set_mode(df, "Auto")` expands to N explicit
  `{ship, mode}` leaves (the same thing as hand-writing them, minus the typos);
* ❌ `clear_ship_orders` → ✅ `{"ship": …, "mode": "Inherit"}` means "this leaf stops speaking".

### Ownership is tri-state, and the kit never takes a fleet over by accident

Every controllable leaf carries a **mode**: `Inherit` (this layer says nothing) / `Auto` (the system
decides, and rewrites the leaf every round) / `Player` (the player decides; the system only reads).
Serialized names are exactly `"Inherit" | "Auto" | "Player"`.

**Writing a value while omitting `mode` silently implies `Player`** (写值即接管; the engine reports
`NOTE_APPLY_TOOKOVER` on stderr). That is why every bulk helper here either takes an explicit
`mode="Player"/"Auto"/"Inherit"` or demands `take_over=True`:

```python
s.set_mode(fleet, "Auto")                                  # only writes `mode`; can NEVER take over
s.set_behavior(fleet, "Dock:地球", mode="Player")           # explicit: unchanged, self-documenting
s.set_behavior(fleet, "Dock:地球", take_over=True)          # acknowledged takeover → receipt says so
s.set_behavior(fleet, "Dock:地球")                          # ✗ ValueError, with the reason
```

## API

```python
import planet_x_ctl as ctl

ckpt = "ckpt_r12.ron"
s = ctl.surface(ckpt)                 # `planet_x --start ckpt --control` (read face == write face)
s.factions                            # faction names, in the engine's order
s.faction("中国")                      # {kind: {key: Leaf}} — the whole faction's leaves
s.leaf("中国", "ship_orders", "长城")   # one leaf: `.value` + `.mode` (+ `.exists`)
s.leaf("中国", "build_weights", ("珠三角", 7))
s.scope_of("factions", "中国")         # the scope tree's opinion at one node (Inherit if silent)

ships  = ctl.ships(ckpt)              # projection ships × their control leaves (+ faction default)
cities = ctl.cities(ckpt)             # projection cities × loyalty_budget / weight aggregates
both   = ctl.ships_and_cities(ckpt)    # one frame for a whole empire (tagged by `kind`)
bs     = ctl.buildings(ckpt)          # the (city, building) index table

mine = ctl.query(ships, "faction_id == '中国' and hull > 0")   # `class` is auto back-quoted

s.set_mode(mine, "Auto")                                       # 通配：N × {ship, mode}
s.set_mode(mine, "Player")
s.set_behavior(mine, "Dock:地球", mode="Player")                # "Idle" / "Follow:星环" / "Move:1.5,-2.0"
s.set_default_ship_order("中国", behavior="Dock:地球", mode="Player")   # ONE leaf, new ships follow
s.set_kiting(mine, -1.0)                                        # 贴脸（clamped to [-1, 1]）
s.set_doctrine(mine, temper=0.4)
s.set_budget("中国", "construction_budget", {"硅": 4.0, "铁": 12.0}, mode="Player")
s.set_loyalty_budget("中国", {"珠三角": 2.5}, mode="Player")
s.set_invest_weights("中国", {("珠三角", "construction:destroyer"): 2.0}, mode="Player")
s.set_capital("中国", "月球", mode="Player")
s.set_scope(factions={"中国": "Player"})                        # scope nodes carry ownership, not values

diff = s.emit()                       # {"control": […], "scope": {…}} → ready for `--apply`
ctl.write(diff, "steer.json")         # canonical, deterministic JSON (byte-identical on replay)
rep = ctl.verify(ckpt, "steer.json")  # read-only rehearsal; Report
assert rep.ok
ctl.apply(ckpt, diff, save="ckpt2.ron")   # now it is real (--round 0 = overlay without advancing)

# 编制表 (roster): stable slot names that survive name generations
r = ctl.roster(ckpt, [("旗舰", "faction_id == '中国'"),
                      ("护卫队", "faction_id == '中国' and class == 'corvette'")])
```

`ctl.projection(x)` accepts **either** a checkpoint (projected on the fly) **or** an existing
`--index` directory; `ships()` / `cities()` / `buildings()` / `roster()` all take `index_dir=` too.

### Recipes: replayable, previewable, byte-stable

A recipe is a plain function `(surface, world data) -> diff`. Rerun it on the *same* checkpoint and
you get a byte-identical diff (the demo asserts this), so a recipe can be committed to git, replayed
against an old checkpoint ("what would I have written back then?"), and attached to the chronicle as
the reason for a decision. Putting the recipe in a file is also where the **roster refresh rule**
belongs — see `roster()` below.

### `verify()` — closing "只报丢弃不报生效"

The engine's `--apply` overlays **in memory**, prints the post-overlay truth on stdout, a receipt on
stderr, and **never writes a file** (only `--save` writes). So verification is a pure read-only loop:

```
planet_x --start ckpt --apply my.json --control   # stdout = read face AFTER, stderr = receipt
planet_x --start ckpt --control                   # stdout = read face BEFORE
```

`ctl.verify(ckpt, diff)` runs both, parses the receipts, and diffs the two read faces structurally
(order-independent, keyed by leaf identity). It reports per requested field:

| | meaning |
|---|---|
| `changed` | the field genuinely moved |
| `noop` | it already held the requested value — reported, **not** treated as a failure |
| `skipped` | the engine rejected the leaf (`WARN_APPLY_SKIPPED`), with its own reason |
| `failed_requests` | neither landed nor skipped — the alarming case; `rep.ok` is `False` |
| `incidental` | leaves that moved **without being asked** — the honest takeover list |
| `took_over` | raw engine paths (`中国.ship_orders[2].behavior`); `took_over_leafs` = leaf names |

```python
rep.summary()          # tidy DataFrame: one row per requested field
rep.changes_frame()    # every field the overlay moved (`requested` marks the intended ones)
rep.skipped_frame()    # engine's skip codes + reasons
rep.raw_receipt        # the untouched stderr JSONL lines
rep.describe()         # one readable paragraph
```

> ⚠ **`verify` only proves the ENGINE ACCEPTED THE DIFF.** It never advances a round, so it cannot
> prove the game will behave as intended. For consequences, re-read the world after
> `--round K` or ask the engine's own cost→benefit preview (`planet_x --control-plan`).
>
> ⚠ It also can only see what the **read face** shows. That face is now **lossless** (the old 2-decimal
> rounding of every numeric leaf is gone), so `--control` reproduces the stored value bit for bit and
> `verify` compares floats exactly (bar float-repr epsilon). What you cannot see there is the
> **effective** value behind an `Inherit` leaf — see the gaps list below.

## Hard constraint: **same-round transform**

Read a checkpoint → emit a diff → apply it to **that same checkpoint**. Nothing else is correct.

`building` inside `build_weights` / `invest_weights` is a **per-city `u32` index**
(`InvestKey = BuildKey = (CityId, BuildingId)`), so it is only self-consistent inside one round. The
engine validates the pair and answers a wrong one with `WARN_APPLY_SKIPPED … no_such_building`
(helpfully listing the city's real indices), but by then you have already shipped a broken recipe.
This kit enforces the constraint instead: every `(city, building)` write is resolved against the
projection **of the very checkpoint the surface was read from**, and an unresolved selector raises:

```python
s.set_build_weights("中国", {("珠三角", 999): 1.0}, mode="Player")
# ValueError: 「珠三角」里没有 building=999；它的建筑下标是 [4, 5, 6, 7]（下标只在城内部唯一，换城要换下标，而且只在同回合自洽）。
s.set_build_weights("中国", {("珠三角", "construction:destroyer"): 2.0}, mode="Player")  # 解析成功
```

The same guard covers stale names (a `set_mode` on a dead ship raises instead of emitting a leaf the
engine will skip), unknown resources, cities that belong to another faction, and unknown bodies.

## Two pitfalls you must know (`python-control-authoring.md` §1.2 / §1.3)

### §1.2 — a leaf saying `Inherit` falls back to a *stale recorded value*

`State::ship_behavior` reads:

```rust
if leaf.mode == Inherit {
    if let Some(d) = &c.default_ship_order { if d.mode.is_player() { return Some(d.value) } }
}
leaf.value      // ← otherwise: the leaf's own record, which may be an expired AI writing
```

So **"release to the upper layer" (`mode: "Inherit"`) is only clean when the fleet default is itself
`Player`.** When ownership is `Auto` the stale value self-heals (the system rewrites the leaf next
round); when the leaf is `Player` but the fleet default is **not** `Player`, the old value lingers on
screen for a long time — looking like an order nobody gave.

**What this kit does about it:** releasing is a two-leaf operation, and the recipe should say both.

```python
s.set_default_ship_order(fac, mode="Player")                 # the layer that will now speak…
s.set_mode(fleet, "Inherit")                                 # …and the leaves that stop speaking
```

The kit never hides this: `ships()["order_behavior"]` is the leaf's *record*, and
`effective_order_value_approx` shows what the chain would resolve to. Which brings us to the next
warning.

> ⚠ **The `*_approx` columns are the kit's LOCAL APPROXIMATION, not the engine's answer.** The engine
> has no per-entity `effective` column yet (`engine-data-plane.md` §2), so `effective_order_mode_approx`
> / `effective_order_value_approx` / `effective_authority_approx` re-implement the resolution chain
> from the read face. **That is a drift source**: the moment the engine's chain changes, these columns
> are wrong while everything else stays right. Treat them as a hint for recipes, never as authority —
> and when the engine starts emitting `effective`, this kit will switch to reading it (one accessor).

### §1.3 — the kit can only produce **one-shot numbers**

`set_budget(..., {"硅": 4.0})` writes a *number*, valid for the moment you computed it. "Keep
construction at 30 % of production" and "upkeep must not exceed production × 0.8" are **persistent
intents**, and the engine already has the vocabulary for them (`BudgetPatch.value` accepts
`{"frac_of_production": 0.3}`; there is an `upkeep_ceiling`). Python cannot express those through a
static diff, and re-running the recipe every round is not the same thing — it is a fake formula that
breaks the moment nobody runs it.

So: **the kit is a preview calculator for one-shot decisions; anything that must persist belongs on
the engine side.** Do not grow a "policy engine" in Python that pretends to be one — say which half
you are implementing and let the other half do its job.

## Roster (编制表) — slot names that outlive names

Entity identity is always the unique **name string** (never a machine id), and names change
generation (沉了一艘才换代: `方舟` → `方舟2` → `方舟3`). That is exactly when hand-copied names fail.
A roster pins a *slot* to a *query*, and the refresh rule lives in the recipe:

```python
spec = [("第1舰队·旗舰", "class=='cruiser' and faction_id=='中国'"),
        ("第1舰队·护卫", "class=='corvette' and faction_id=='中国'")]
ctl.roster(ckpt, spec)   # → slot, query, refresh_rule, matched, candidates, + the matched ship's columns
```

**Deterministic tie-break** (`DEFAULT_REFRESH_RULE = ("-hull", "-hull_max", "ship_id")`): highest
current hull → highest `hull_max` → name ascending. Two notes on that choice:

* "oldest" would need a birth round, and the projection's ships table does not carry one (no
  `spawned_round` column) — so name order stands in for seniority. **Engine gap**, see below.
* pass `rule=` (or a third element in a spec entry) to override, using `"-col"` for descending.

A roster is an **intent, not a standing promise**: "the flagship is replaced automatically when it
sinks" only happens because the recipe re-runs. `strict=True` turns an unfilled slot into an error
instead of a `matched=False` row.

## Environment notes for this worktree

* `uv sync` **works offline here** (the resolution comes out of the local uv cache), producing
  `.venv/` with `pandas 3.0.5`. Nothing is installed globally.
* If `uv sync` ever cannot resolve, run with the sibling kit's interpreter instead — it already has
  pandas, and `planet_x_ctl` locates `planet_xq` by sibling path anyway:

  ```bash
  PYTHONPATH=play/planet_x_ctl ../planet_xq/.venv/Scripts/python.exe play/planet_x_ctl/demo.py
  ```

* No absolute path is baked into the package. The engine is found as: explicit `planet_x=` argument →
  `$PLANET_X_BIN` → `<worktree>/target/debug/planet_x[.exe]` → `planet_x` on `PATH`. The engine also
  needs `config/game.ron`, so the kit runs it from a directory that has one (derived from the
  checkpoint's or this kit's location, unless `$PLANET_X_CONFIG` is set).
* `_repo_root()` = `Path(__file__).resolve().parents[3]`; `planet_xq` is imported if installed, else
  loaded from `../planet_xq/planet_xq/__init__.py`. **The projection parsing/join logic is never
  duplicated here.**

## Engine gaps this kit ran into (feedback for the Rust side)

All of these were *measured*, and each one is a place where the data-plane-only ruling leaves the kit
guessing. They are listed because they are cheap to close and expensive to work around:

1. **Flow metrics only exist for rounds the engine actually advanced.** `--start ckpt --round 0
   --index DIR` (the natural way to project a checkpoint) emits a correct **snapshot** but reports
   `production_value = 0`, `upkeep = 0`, `governance_cost = 0` for every faction — those come from
   `RoundFlow`, which is not persisted. Any statistical policy therefore needs a projection produced
   by a real run: `planet_x --seed S --round N --index DIR --save ckpt.ron` — one pass, so the
   projection's last round and the checkpoint describe the same state. `ctl.new_checkpoint(...,
   index_dir=…)` does the one-pass thing for you.
   *(As of this writing `idx/flow.jsonl` and `idx/city_flow.jsonl` are already being written, which
   is half of the `engine-data-plane.md` §2 fix — see the next point for what is still missing.)*
2. **The new read-face tables exist on disk but are not declared in `schema.json`.** A recent build
   writes `idx/flow.jsonl`, `idx/city_flow.jsonl`, `idx/control.jsonl`, `idx/scope.jsonl`, yet
   `schema.json`'s `lazy` map still lists only
   `bodies / cities / events / factions / settlements / ships` — so `planet_xq.load()` cannot see the
   new tables at all, and no consumer can rely on them. That is precisely the "每加一张表，两处都要动，
   别漏 schema" hazard from `engine-data-plane.md` §2. `idx/control.jsonl` also has no `effective`
   column yet (`{round, faction_id, kind, key, sub, mode, value}`, with `kind` in the singular, e.g.
   `"ship_order"`). This kit keeps its `--control` backend until `effective` lands; switching is then
   a one-accessor change, exactly as `python-control-authoring.md` §3 step 4 predicts.
3. **No per-entity `effective` on the control read face.** `--control` shows the leaf's *recorded*
   value; when the leaf says `Inherit` the effective order may come from the fleet default or a scope
   node. Python must re-implement `resolve_chain`, which is a drift source (see §1.2 above).
4. **~~The control read face rounds every numeric leaf value to 2 decimals~~** — **fixed** (the
   rounding is gone: `--control` is now bit-for-bit the stored value, so "dump → edit → send back" is
   lossless; guard: `src/control.rs::the_control_template_never_rounds_a_leaf_value`). Historical note
   kept because the diagnosis is the interesting part: a **lossy** transform inside a face that the
   docs call "read face = write face" made `0.125` come back as `0.13` — a silent write nobody asked
   for. Measured before removing it: **0 of 470** numeric leaves in a real round-120 control surface
   would have changed under that rounding, i.e. it bought no token savings and only carried risk.
5. **~~`ship_kiting` / `ship_doctrine` are not live layers yet~~** — **fixed upstream**
   (`control-live-layers.md` §4.1): both are tri-state leaves with a faction-level default
   (`default_kiting` / `default_doctrine`), so "the whole fleet goes 贴脸" is **one leaf** that new
   ships inherit too. This kit now lists those two kinds in `LEAF_KINDS` (leaving them out made them
   vanish from `surface()` silently — the §8.1 lesson: the contract has two ends, emitter *and*
   consumer) and `set_kiting` / `set_doctrine` demand an explicit ownership (`mode=` or
   `take_over=True`) like every other value write.
   ⚠ One engine-side trap this exposed: creating a **two-axis** leaf (`default_doctrine` /
   `ship_doctrine`) from a single-axis patch initializes the *other* axis to `0.0`
   (`Control::inherit(ShipDoctrine::default())`, `src/control.rs:770`), not to the ship's record value
   — a fleet-wide change that looks perfectly normal afterwards. The kit refuses that patch
   (`_require_both_axes`); whether the *engine* should instead seed the missing axis is an open
   question recorded in `control-live-layers.md`.
6. **`(city, building)` is a per-city `u32` index.** Correct and documented, but it forces every
   recipe to be a same-round transform and makes any cross-round diff silently wrong. A stable
   building identity (or an `--index` column naming it) would remove a whole class of footguns.
7. **No `spawned_round` on the projection's ships table**, so "oldest" (the natural roster tie-break
   for a flagship) is unavailable and name order has to stand in.

## Layout

```
planet_x_ctl/__init__.py   the kit (Surface, Leaf, Report, recipes helpers, engine I/O)
demo.py                    self-asserting end-to-end demo (generates its own fixture, no network)
README.md                  this file
pyproject.toml             uv project: name planet-x-ctl, dependency pandas>=2.0, script planet-x-ctl
```

`planet-x-ctl` on the command line:

```
planet-x-ctl surface ckpt.ron      # dump the control leaves (value + mode) as JSON
planet-x-ctl verify  ckpt.ron steer.json   # read-only rehearsal; exit non-zero if it did not land
planet-x-ctl roster  ckpt.ron      # the ships table with leaf mode / default mode columns
```
