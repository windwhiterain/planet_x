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

* ❌ engine-side `{"舰": "*"}` wildcard → ✅ `s.set_mode(df, "Auto")` expands to N explicit
  `{舰, 归属}` leaves (the same thing as hand-writing them, minus the typos);
* ❌ `clear_ship_orders` → ✅ `{"舰": …, "归属": "Inherit"}` means "this leaf stops speaking".

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
s.leaf("中国", "指令", "长城")           # one leaf: `.value` + `.mode` (+ `.exists`)
s.leaf("中国", "建造权重", ("珠三角", 7))  # ⚠ `kind` 是**中文叶名**（= --control-schema 的 leaves[].field）
s.scope_of("factions", "中国")         # the scope tree's opinion at one node (Inherit if silent)

ships  = ctl.ships(ckpt)              # projection ships × their control leaves (+ faction default role)
                                      # ⚠ the **effective order** columns are the ENGINE's answer
                                      #   (`effective_order_mode` / `effective_order_value` /
                                      #   `order_source`) — see §"effective columns" below
cities = ctl.cities(ckpt)             # projection cities × `城市福利预算` / `建设权重`·`建造权重` aggregates
both   = ctl.ships_and_cities(ckpt)    # one frame for a whole empire (tagged by `kind`)
bs     = ctl.buildings(ckpt)          # the (city, building) index table

mine = ctl.query(ships, "势力 == '中国' and 船体 > 0")   # 中文列名不必 back-quote

s.set_mode(mine, "Auto")                                       # 通配：N × {ship, mode}
s.set_mode(mine, "Player")
s.set_behavior(mine, "Dock:地球", mode="Player")                # "Idle" / "Follow:星环" / "Move:1.5,-2.0"
# ⚠ 指令**没有**势力级默认叶（2026-10 裁决：指令是**即时操作**）——"全舰队听我的"就是下面这行：
#   `set_behavior` 把同一句话写给**每一艘**舰（通配选择器展开成 N 片叶）。
s.set_kiting(mine, -1.0)                                        # 贴脸（clamped to [-1, 1]）
s.set_doctrine(mine, temper=0.4)
s.set_role(mine, "Freight", mode="Player")                      # 角色：三值字符串 ——
                                                                #   "War" 战舰 / "Freight" 运输舰 /
                                                                #   "Observe" 观测舰（去异常区蹲着喂 MOND 掌握度）
s.set_default_role("中国", "Freight", mode="Player")            # 一片叶：全舰队转运输，且 AI 定编不碰
s.set_budget("中国", "建造预算", {"硅": 4.0, "铁": 12.0}, mode="Player")
s.set_loyalty_budget("中国", {"珠三角": 2.5}, mode="Player")
s.set_invest_weights("中国", {("珠三角", "construction:destroyer"): 2.0}, mode="Player")
s.set_capital("中国", "月球", mode="Player")
s.set_scope(factions={"中国": "Player"})                        # scope nodes carry ownership, not values

# **设计图（blueprint）** —— 「还不存在的舰」的出厂规格：舰级 + 选装 + 该图给这型舰的**倾向**
# （角色 / 风格 / 风筝姿态）。⚠ 图**不能**指定"指令"（2026-10 裁决：指令是即时操作）——
# "这型舰一造出来就跑运输"写的是 `role="Freight"`；具体去哪、跟随谁，逐舰现下。
# 建造区**指向**一张图（结构叶），下水那一刻把图**印成**一艘舰（`components` 是快照，
# 之后改图**不动**已有的舰）。口径 A：图的 `class` 必须 == 该建造区的 `ship_type`，
# 所以**图与建造区要一起写**（`set_blueprint_and_retool` 就是那条正解）。
s.set_blueprint("中国", "重甲护卫", class_="corvette", components=["kinetic", "ion_drive"],
                mode="Player")                                  # 写值必须明说归属（写值即接管）
s.set_blueprint("中国", "守家护卫", class_="corvette", role="Freight", mode="Player")
s.set_blueprint_and_retool("中国", "重甲护卫", class_="destroyer",
                           city="珠三角", building=7, mode="Player")   # 图 + 该区的 ship_type 一起改
s.silence_blueprint_stance("中国", "守家护卫", role=True)         # 某条倾向轴回到沉默（**不是**删图！）
s.set_blueprint_pointer("中国", "珠三角", 7, "重甲护卫")           # 指过去
s.set_blueprint_pointer("中国", "珠三角", 7, None)                # 拆掉指针（写 `null`，不是"缺席"）
s.remove_blueprint("中国", "重甲护卫")                            # 删整张图（⚠ 挂它的区变悬空指针 ⇒ 停产）

# 删叶（`remove: true`）：这一层**不再说话**，而且叶里的值也不再参与取值 ——
# 这是「恢复出厂值」的唯一做法（`mode: "Inherit"` 做不到，见 §1.2 与下面的 "删叶" 一节）
s.remove("中国", "风格", "长城")
s.remove_doctrine(mine)                                         # 通配：这些舰的风格回出厂快照/舰队默认
s.remove_kiting(mine)
s.remove_role(mine)                                             # ⚠ 角色轴：删叶 = **交回自动定编**（不是冻结）
s.remove_default_doctrine("中国")                                # 势力级默认叶：删了就不再供值
s.remove_default_role("中国")

diff = s.emit()                       # {"control": […], "scope": {…}} → ready for `--apply`
ctl.write(diff, "steer.json")         # canonical, deterministic JSON (byte-identical on replay)
rep = ctl.verify(ckpt, "steer.json")  # read-only rehearsal; Report
assert rep.ok
ctl.apply(ckpt, diff, save="ckpt2.ron")   # now it is real (--round 0 = overlay without advancing)

# 编制表 (roster): stable slot names that survive name generations
r = ctl.roster(ckpt, [("旗舰", "势力 == '中国'"),
                      ("护卫队", "势力 == '中国' and 舰级 == 'corvette'")])
```

`ctl.projection(x)` accepts **either** a checkpoint (projected on the fly) **or** an existing
`--index` directory; `ships()` / `cities()` / `buildings()` / `roster()` all take `index_dir=` too.

### 角色轴（`角色` / `舰队默认角色`）是**三值字符串枚举**

第三条风格轴不再是 `true`/`false` 的开关，而是 serde 的 `ShipRole`，JSON 形态就是三个字符串
（`ctl.ROLES`）：

| 值 | 自动控制派它干什么 |
|---|---|
| `"War"` | 战舰：找仗打（接战 / 轰炸 / 殖民）——旧 `false` |
| `"Freight"` | 运输舰：按积压跑集货路线（`autocontrol::freight`）——旧 `true` |
| `"Observe"` | **观测舰**：驻在太阳系外缘的引力异常区（MOND）蹲着，喂「掌握度」那条知识渠道（`autocontrol::knowledge`）——MOND 掌握度的**唯一**知识来源 |

三态互斥（一艘舰同一时刻只有一种活），且**都不解除武装**：运输舰 / 观测舰在射程内照样自动开火、
照样按 `kiting` 姿态软移动。所以 `s.set_role(mine, "Freight")` 是"派它去跑集货"，不是"把它变成民船"。
喂别的东西（`True` / `1` / `"freighter"`）会被 `_check_role` 在配方期当场拒绝——旧写法在这里
不会"悄悄还能用"，因为引擎那边已经被 serde 拒了。

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
| `took_over` | raw engine paths (`中国.指令[2].行为`，**中文叶名**); `took_over_leafs` = leaf names |

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
> `verify` compares floats exactly (bar float-repr epsilon). Two things it *still* cannot see, and both
> are reported deliberately instead of guessed at: **whether a per-ship style leaf exists at all**
> (those rows are listed for every ship — 删叶 的落地以引擎回执 `NOTE_APPLY_REMOVED` 为准), and a
> **derived** column the engine adds for its own convenience (e.g. a blueprint's `ship_count`).
> The effective order **is** visible now — both in `--control` (`behavior`) and in `ships()`
> (`effective_order_value` + `order_source`).

## Hard constraint: **same-round transform**

Read a checkpoint → emit a diff → apply it to **that same checkpoint**. Nothing else is correct.

`building` inside `建造权重` / `建设权重` is a **per-city `u32` index**
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

### §1.2 — 指令**没有**更高的一层了；倾向三轴有（而且图在最前面）

**指令**（Rust 侧字段 `ship_orders`，读面/写面上叫 `指令`）的取值链在 2026-10 之后只剩**那一片逐舰叶**（用户裁决：指令是**即时操作**）：

```rust
// State::ship_behavior（Rust 侧字段名仍是 `ship_orders`）
c.ship_orders.get(&ship_id).map(|l| l.value.clone())
```

于是两条推论，都很容易踩：

* **叶里写着什么，就是这艘舰在干什么**（与 `mode` 无关）。`Inherit` 不是"回到上层"，而是"叶里那句旧记录继续算数" ——
  旧的「舰队默认指令」与「图上 order」两片叶都已删除，**没有东西能盖掉它**。
* **叶不存在** = 没有任何一层说话 ⇒ 有效值是空的，调用方按 `Idle` 兜底（读面 `order_effective` / `effective_order_value` 给 `null`）。
  想表达"待命"就**明确写** `Idle`，不要靠删叶。

**风格 / 风筝姿态 / 角色**（长期倾向）则**有**更高的层，而且是**三层**：

```rust
// State::ship_doctrine / ship_kiting / ship_role（三条轴同形）
if leaf.mode == Inherit {
    // ① 出厂图上这条轴（图上写了它、且那张图归 Player）
    // ② 势力级舰队默认叶（只在它自己是 Player 时供值）
}
leaf.map(|l| l.value).unwrap_or(record)   // ← 叶**存在**就用叶里的值（与 mode 无关）
```

所以那边仍然有"叶在、叶说 `Inherit`，但叶里躺着一句旧值"的现象。**别在 Python 里重算这条链**：
`ships()` 直接给引擎的答案（`effective_order_value` + `order_source`，见下一节）。

要让一队舰真的"跟着舰队默认走"，两件事都要做（`ships()` 的 `order_source` 会告诉你现在是谁在供值）：

```python
s.set_default_role(fac, "Freight", mode="Player")   # 现在说话的那一层…
s.remove_role(fleet)                                # …以及**不再供值**的那些叶（删叶，不是写 Inherit）
```

### 删叶 (`remove`): `mode: "Inherit"` 撤不掉叶里的值

This is the trap §1.2 of `python-control-authoring.md` hides one level deeper, and it is worth
spelling out because the symptom is「我改了舰队默认，这艘舰却不跟」:

```rust
// State::ship_doctrine — the *value* rule, and it does not look at `mode`
if leaf.mode == Inherit {
    if let Some(d) = &c.default_doctrine { if d.mode.is_player() { return d.value } }
}
leaf.map(|l| l.value).unwrap_or(record)   // ← 叶**存在**就用叶里的值
```

So an existing leaf supplies its value **whatever its mode says**, while a *missing* leaf falls back
to `Ship.doctrine` (the factory record). 「叶不存在」and「叶写着 `Inherit`」are therefore equal for
*ownership* and different for *value*.

```python
s = ctl.surface(ckpt)
s.set_doctrine("长城", temper=0.7, lone_wolf=-0.4, mode="Player")   # pins the leaf…
s.set_doctrine("长城", mode="Inherit")                              # …this only releases ownership
# 有效风格**仍然是** 0.7 / -0.4（叶里的值优先于出厂快照）
s.remove_doctrine("长城")                                           # ← 这才是「恢复出厂值」
```

Two things worth knowing about the kit's side of `remove`:

* 删一片**本来就不存在**的叶是**幂等成功**（引擎既不报丢弃，也不进 `NOTE_APPLY_REMOVED`）；
  `Report.requests` 里那条请求仍然是 `satisfied`，所以 `rep.ok` 不会因为它变红。
* `remove` **不能**和值 / `mode` 同时写（引擎报 `remove_conflicts_with_value`）：一条同时说着
  "删掉它"和"把它设成 0.5"的补丁没有正确答案，所以两件事请分两条补丁发。
* `Report.removed` / `removed_leafs` 给出真的被删掉的那些叶；`describe()` 会把它们列出来。
  逐舰叶的**存在性**在 `--control` 上读不出来（那片现在对每艘舰都有一行，列的是有效值），
  所以逐舰删叶的"落地了没有"以**引擎回执**为准，不以 `--control` 为准。要问「这片叶还在不在」
  用投影的 **`q.control()`**（`idx/control.jsonl`，只列真实存在的叶）——`ships()` 的
  `order_leaf` 就是从那里来的（见下）。
* ⚠ **角色轴（`角色`，Rust 侧 `ship_role`）上「删叶」的含义不一样**：那片叶**自动控制每回合也会写**
  （按积压定编谁去跑集货路线 + 派观测舰去异常区蹲着喂 MOND 掌握度），所以删掉它是**放手**
  ——AI 下回合可能立刻又写下它的结论，
  而不是"从此冻结"。想让某个角色稳定下来就写 `mode="Player"`（那才是闸门）。另两条风格轴
  没有这个执行者，删掉就等于回到出厂快照。

The kit never hides this: `ships()["order_behavior"]` is the leaf's *record*, while
`effective_order_value` is what the chain actually resolves to (`order_source` says **who supplied
it**). Which brings us to the next warning.

> ### Where each of those two answers comes from (and what `--control` can no longer tell you)
>
> | question | columns | source |
> |---|---|---|
> | 「本舰**那片叶**还在吗？它自己记着什么？」 | `order_leaf` / `order_mode` / `order_value` / `order_behavior` | the projection's **`derived.control`** table (`idx/control.jsonl`, `kind == "指令"`) — the engine emits one row per **real** leaf by walking `ControllableState::ship_orders` |
> | 「**有效**指令是什么？**归谁**？这条值**谁供的**？」 | `effective_order_mode` / `effective_order_value` / `order_source` | the projection's ships table: `order_effective_mode` / `order_effective` / `order_source` (`State::ship_control` / `ship_behavior` / `ship_behavior_source`) |
>
> ⚠ **`--control` is not a leaf-existence face.** Since that read face went "one row per ship"
> (`control-live-layers.md` §13) it lists **every** ship, and the `behavior` it shows is the
> **effective** value. Reading leaf existence off it (as this kit did for one commit) made
> `order_leaf` permanently `True` and silently re-pointed `order_behavior` at the effective value
> while keeping the old column name — the exact failure mode `.agents/notes/engine-data-plane.md`
> calls out (「引擎给答案，Python 只负责筛」), except this time the answer was *the kit's own*.
> The authoritative existence face is **`derived.control`**; a projection old enough to lack the
> `derived` section raises there instead of guessing.
>
> ⚠ **对指令而言，两组列现在是同一个答案**（2026-10：舰队默认指令与图上 `order` 两片叶都已删除）：
> `order_behavior` 与 `effective_order_value` 逐行相同、`order_source` 只会是 `leaf` 或空。
> `demo.py` §[4d] 钉的就是这条新不变式（还有「删叶 ⇒ 有效值变空、**没有**任何一层接手」）。
> 两组列在**倾向三轴**上仍然是两件事（那边的链有图层与舰队默认，见 §1.2）。

> ### `ships()`: the effective columns are the **engine's** answer (the `*_approx` hole is closed)
>
> `effective_order_mode` / `effective_order_value` / `order_source` are read **straight off** the
> projection's own ships columns `order_effective_mode` / `order_effective` / `order_source` —
> i.e. `State::ship_control` / `State::ship_behavior` / `State::ship_behavior_source`. That code
> knows the real chains (指令：`叶 → 势力 → 全局`；倾向三轴：`叶 → 出厂图 → 舰队默认 → 势力 → 全局`),
> so it is the only correct answer — never re-derive it in Python.
>
> This kit used to **re-implement** that chain in Python and hand back the result as
> `effective_order_*_approx` / `effective_authority_approx`. That re-implementation predated the
> blueprint layer, so it gave **wrong** answers for blueprint-built ships — a drift source by
> construction (`.agents/notes/engine-data-plane.md`: 「引擎给答案，Python 只负责筛」).
> **`APPROX_COLUMNS` are now a fallback only**, used when the projection has no
> `order_effective_mode` at all (an index directory written by an **older** engine). The face never
> lies about which one you are holding:
>
> | frame | columns | `effective_order_from_engine` |
> |---|---|---|
> | engine columns present | `effective_order_mode` / `effective_order_value` / `order_source` | `True` |
> | engine columns absent | `effective_order_mode_approx` / `effective_order_value_approx` / `effective_authority_approx` | `False` |
>
> The names differ **on purpose** (two answers must not wear the same column name), and
> `ctl.EFFECTIVE_PROVENANCE_COLUMN` lets a recipe ask the question in one `df.query(...)`. The
> fallback is also *documented as wrong in one specific place*: it has no blueprint step, so treat it
> as a hint, never as authority. `demo.py` runs both paths and asserts them.
>
> `effective_authority_approx` and `effective_order_mode_approx` were **two columns answering one
> question**, and neither was the engine's; in the engine branch they are replaced by the single pair
> (`order_source` = who supplied the **value**, `effective_order_mode` = who **owns** the ship).
> The engine's `order_source` also separates「叶**不存在**」from「叶写着 `Inherit`」— the former is
> `null` (nobody spoke), the latter honestly reports `leaf`（值就是叶里那个值）。

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
spec = [("第1舰队·旗舰", "舰级=='cruiser' and 势力=='中国'"),
        ("第1舰队·护卫", "舰级=='corvette' and 势力=='中国'")]
ctl.roster(ckpt, spec)   # → slot, query, refresh_rule, matched, candidates, + the matched ship's columns
```

**Deterministic tie-break** (`DEFAULT_REFRESH_RULE = ("-船体", "-船体上限", "下水回合", "舰名")` ——
元组里的名字**就是投影读面上的列名**，中文): highest current hull → highest hull_max →
**oldest first**（`下水回合` ascending）→ name ascending. Three notes on that choice:

* "oldest" **is** available: the engine ships `Ship.下水回合` as the projection's `下水回合`
  column (the roster tie-break was its stated purpose). `null` = the ship predates
  the column (an old checkpoint) ⇒ **unknown**, and pandas sorts NaN last, so a known age always
  beats an unknown one and those ties fall through to name order.
* a rule column the frame does not carry (an index directory written by an **older** engine) is
  **skipped**, not an error — the roster degrades to the remaining columns; the row's
  `rule_columns_missing` says which. It never raises.
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

1. **过程量 — the round's production / upkeep / governance / trade / AI judgments — only exists for
   rounds the engine actually advanced; in a round's `pre` view it is 0/empty.** `--start ckpt
   --round 0 --index DIR` (the natural way to project a checkpoint) emits a correct **snapshot**, but
   the numbers a statistical policy needs are not in the persisted state — `production_value = 0`,
   `upkeep = 0`, `governance_cost = 0` for every faction. They are computed *during* the round and
   folded into that round's `post` view, so any statistical policy needs a projection produced by a
   real run: `planet_x --seed S --round N --index DIR --save ckpt.ron` — one pass, so the
   projection's last round and the checkpoint describe the same state. `ctl.new_checkpoint(...,
   index_dir=…)` does the one-pass thing for you.
   *(The process tables `idx/faction_process.jsonl` / `idx/city_process.jsonl` are written every
   round and declared in the schema's `derived` section — see the next point. That does not soften
   the rule above: a round the engine never advanced has no process quantities to write.)*
2. **~~The new read-face tables exist on disk but are not declared in `schema.json`.~~** — **closed**:
   the derived tables live in the schema's own `derived` section (not `lazy` — they join on columns
   `main.jsonl` already carries), and `planet_xq.load()` reads it, so `q.derived(name, round)` /
   `q.faction_process()` / `q.control()` / `q.blueprints()` all work. `idx/control.jsonl` has no
   `effective` column — it lists **leaves** (kind/key/sub/value/mode), and "which layer wins" is a
   **per-ship** answer that belongs on the ships table, where the engine now puts it (next point).
   What that table *is* good for: it is the one read face that answers **「这片叶真的存在吗」** for
   per-ship leaves (`--control` lists every ship since §13) — `ships()` reads it for `order_leaf`
   for exactly that reason, and `q.control()` is the public accessor.
3. **~~No per-entity `effective` on the control read face.~~** — **closed** (blueprint round,
   `SCHEMA_VERSION` 9 → 10): the projection's ships table carries the engine's own
   `order_effective_mode` / `order_effective` / `order_source`, and `ships()` now **reads** them
   instead of re-implementing `resolve_chain` (see the "effective columns" note above). The old
   `*_approx` trio survives only as the fallback for index directories written by an older engine.
   ⚠ `--control` shows the *effective* order on that row too (`control-live-layers.md` §13): a ship
   whose whole chain is silent reports `"behavior": null`, and one whose leaf is `Inherit` but whose
   fleet default is `Player` reports the **default's** value. That is deliberate — it matches the
   projection — and the template is still a fixed point (`--control` → `--apply` → `--control` is
   byte-identical; a `null` row does not invent a leaf).
4. **~~The control read face rounds every numeric leaf value to 2 decimals~~** — **fixed** (the
   rounding is gone: `--control` is now bit-for-bit the stored value, so "dump → edit → send back" is
   lossless; guard: `src/control.rs::the_control_template_never_rounds_a_leaf_value`). Historical note
   kept because the diagnosis is the interesting part: a **lossy** transform inside a face that the
   docs call "read face = write face" made `0.125` come back as `0.13` — a silent write nobody asked
   for. Measured before removing it: **0 of 470** numeric leaves in a real round-120 control surface
   would have changed under that rounding, i.e. it bought no token savings and only carried risk.
5. **~~`ship_kiting` / `ship_doctrine` are not live layers yet~~** — **fixed upstream**
   (`control-live-layers.md` §4.1): both are tri-state leaves with a faction-level default
   （读面上叫 `姿态` / `舰队默认姿态`、`风格` / `舰队默认风格`）, so "the whole fleet goes 贴脸" is **one leaf** that new
   ships inherit too. `set_kiting` / `set_doctrine` demand an explicit ownership (`mode=` or
   `take_over=True`) like every other value write.
   ⚠ **The lesson this cost is now structural (2026-10)**: leaving a kind out of this kit's table made
   it vanish from `surface()` *silently* — the contract has two ends, emitter *and* consumer. So the
   table is no longer ours: `LEAF_KINDS` is a **lazy `Mapping` read from the engine's
   `--control-schema`** (`src/control/leaves.rs`), and the same declaration feeds the WebUI. Adding a
   leaf means touching the engine once; this kit (and the UI) follow automatically.
   `play/tests/g4_spec.py` guards it on the data: `leaves` ∪ `actions` ∪ `{势力}` must equal
   `FactionControlPatch.properties` **both ways**.
   ⚠ One engine-side trap this exposed: creating a **two-axis** leaf（`舰队默认风格` / `风格`，
   Rust 侧 `default_doctrine` / `ship_doctrine`）from a single-axis patch used to initialize the *other* axis to `0.0`
   (`Control::inherit(ShipDoctrine::default())`, `src/control.rs`), not to the ship's record value
   — a fleet-wide change that looks perfectly normal afterwards. **Now settled on both ends**: the
   *engine* rejects such a patch (code `partial_doctrine_leaf`, `control-live-layers.md` §3.1 —
   the fleet-level leaf needs both axes; a per-ship leaf seeds the missing axis from the value the
   ship is actually using, i.e. the record value when no default is `Player`), and the kit still
   refuses it at recipe time so the author sees it while writing, not at apply time.
   The same ruling added `remove: true` (删叶) — the only way to get a style leaf **back** to the
   factory record (see the 「删叶」 section above).
6. **`(city, building)` is a per-city `u32` index.** Correct and documented, but it forces every
   recipe to be a same-round transform and makes any cross-round diff silently wrong. A stable
   building identity (or an `--index` column naming it) would remove a whole class of footguns.
7. ~~**No `spawned_round` on the projection's ships table**, so "oldest" (the natural roster tie-break
   for a flagship) is unavailable and name order has to stand in.~~ **Closed** (blueprint round,
   `SCHEMA_VERSION` 9 → 10): the engine records `Ship.spawned_round` and the projection ships table
   carries it as `spawned_round` (`null` = old checkpoint ⇒ unknown). `DEFAULT_REFRESH_RULE` now
   uses it before name order, and `demo.py` proves the tie-break with a pair of same-score ships of
   different ages whose name order would pick the other one.
8. ~~**The blueprint layer is not modeled by the kit's `*_approx` columns.**~~ — **closed**: `ships()`
   reads the engine's own `order_effective_mode` / `order_effective` / `order_source` (which include
   the blueprint step) and exposes them as `effective_order_mode` / `effective_order_value` /
   `order_source`. The local chain is now only the **fallback** for index directories written by an
   older engine, where it keeps the `_approx` names and `effective_order_from_engine = False`. So the
   remaining honest caveat is narrow and stated on the face of the frame: **the fallback is
   blueprint-blind** (`leaf → fleet default → faction scope → global`, with no `blueprint` step), which
   is exactly why it may not wear the engine's column names.
   *Not staged in `demo.py`*: a blueprint-built ship (the one case where the two disagree). Getting one
   needs a real shipyard launch — a Player-owned blueprint that the faction cannot afford deliberately
   waits (Q4(b)), the pointer lives on a yard whose city defect/raze churn is heavy in a 12-round
   fixture, and a cheap corvette still needs ~dozens of rounds of build points. The demo therefore
   asserts the **contract** (engine fresh, fallback labelled, values identical on a frame with no
   blueprint-built ship) instead of staging the divergence. See
   `notes/control-live-layers.md` §13「未做」.

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
