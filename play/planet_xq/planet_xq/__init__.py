"""planet_xq — read a `planet_x --index` projection into pandas.

The Rust tool projects the world into a **lean main stream** (``main.jsonl``: eager fields +
the round's **``view``** — one :class:`RoundView` per round — plus id-arrays) and **id-indexed
lazy tables** (``idx/*.jsonl``: the heavy entity objects). This kit loads both and exposes join
helpers so an agent fetches heavy detail by id without hand-rolling the merge.

The round's world has exactly **one shape, twice**: ``pre`` = the world at the round's start
(the process quantities are 0/empty there) and ``post`` = the world at the round's end **+ that
round's process quantities** (production / upkeep / governance / trade / the AI's judgments).
``main.jsonl`` carries the ``post`` one as the row's ``view`` key; everything under it is flat —
``view.city_count`` / ``view.ship_count`` / ``view.factions[<势力>]`` / ``view.cities[<城>]`` /
``view.market_price`` / ``view.decisions``.

Directory layout written by ``planet_x --round N --index DIR``:

    DIR/schema.json      agent-readable projection contract (eager / lazy / columns / read_order)
    DIR/meta.json        the static rules dictionary (ship/building/component/structure specs,
                         market.resource_value, economy/combat/diplomacy/… tuning) — same source
                         as `--meta`
    DIR/main.jsonl       one lean fact row per round
    DIR/idx/events.jsonl (round, seq, event_id, ...)  the **sparse event milestones**: one row per
                                                      event, normalized participant slots
    DIR/idx/ships.jsonl  (round, ship_id, ...)        per-round ship detail (+ effective panel)
    DIR/idx/cities.jsonl (round, city_id, ...)        per-round city detail (+ buildings list)
    DIR/idx/factions.jsonl (round, faction_id, ...)   per-round faction detail (resources/relations/
                                                      own cities/ships)
    DIR/idx/bodies.jsonl (body_id, ...)               global master table
    DIR/idx/settlements.jsonl (settlement_id, ...)    global 定居点 master

Typical use::

    import planet_xq
    q = planet_xq.load("out")
    q.facts                 # pandas DataFrame: the lean main stream
    q.ships(round=10)       # ships at round 10 (from the index, joined by id)
    q.cities(round=10)
    q.factions(round=10)    # per-faction resources + relations + own city/ship ids
    q.faction_snapshot(10, "中国")  # one-call decision view (meta + stockpile + relations)
    q.bodies()              # global master
    q.settlements()         # global 定居点 master
    q.join("ships", round=10)  # explode main.ship_ids and merge with the ship detail table
    q.ships_spec()          # config: ship class -> spec as a DataFrame (joinable with q.facts)
    q.resource_value()      # config: resource key (可读名) -> value
    q.yearly_avg("view.city_count")             # 年均 (round = 1 month, 12/年)
    q.decadal_avg("view.factions.中国.market_value")     # 十年均 (120 月)

History / event queries (the sparse milestones)::

    q.events(round=47)                  # every event of one round (long form: one row/event)
    q.events(type="city_razed")         # ONE type -> its payload is flattened into dense columns
    q.history("city", "火星-殖民城")     # ★ everything that ever happened to one entity
    q.history("ship", "长征-7")          #   … any kind: city/ship/faction/body/settlement
    q.cause("ship", "长征-7")            # ★ resolved fate: killer ship / faction / weapon / assists
    q.cause("city", "冥王星前哨")         #   the last ownership/death event (why it changed hands)
    q.fates(kind="ship")                # every ship death in the window, with cause + killer
    q.actors()                          # long-form (round, seq, kind, id, role) participant index
    q.changes("city", "冥王星前哨")       # pure dense-diff of the snapshot table (cross-check)
    q.milestones()                      # 里程碑层（判据：无限过去）——**按当前判据为空**，见下
    q.notables()                        # 窗口层（判据：一定窗口）——目前只有开战/停战
    q.storyboard(window=100)            # ★ 故事板: weight 最高的事件压成每 100 回合一行
    q.audit()                           # completeness self-check: unexplained city changes (want 0)

Every event row also carries a **`headline`** column: one human-readable sentence rendered by the
Rust side's single ``GameEvent::headline`` (the same sentence CLI ``--notables`` / ``--digest`` show).
It is *self-contained* (built only from the event's own fields, never by looking the entity up in
today's state) so it stays true for archived history — an entity may be long dead or renamed since.
Machine queries should still use ``actor_*``/``target_*``/``data``; the headline is for reading.

Design notes for the milestones (each backed by measurement):

* It is a **long table**: "missing" means *no such row*, not NaN. So sparse-field statistics
  (counts / per-window rates / first-last / fates) are one-liners: ``groupby(...).size()``.
* Participants always use the same slots (``actor_*`` / ``target_*`` / ``extra``) and there are
  **no variant-specific columns** — otherwise you get the two diseases of a serialized tagged
  union: one column with two meanings (``from``/``to`` is a faction in ``city_defected`` but a
  body in ``capital_relocated``) and one role with many spellings (faction/owner/fallen_to/…).
* **Filter by type first** (``q.events(type=...)``): the union frame is ~75% null, a single-type
  frame is dense. Use the full frame only for counting/scanning.

Entity identity: every id (ship/city/building/faction/body/settlement) is a **string name**,
never an integer offset — ``q.ids("ships", r)`` returns a list of names, and ``--apply`` diff ids
use the same names.
"""
from __future__ import annotations

import json
import os

import pandas as pd

__all__ = ["load", "PlanetXQ"]


class PlanetXQ:
    """A loaded ``planet_x --index`` projection: the lean main stream + lazy tables."""

    def __init__(self, dirpath: str):
        self.dir = os.path.abspath(dirpath)
        with open(os.path.join(self.dir, "schema.json"), encoding="utf-8") as fh:
            self.schema = json.load(fh)
        self.facts = pd.read_json(os.path.join(self.dir, self.schema["main_stream"]), lines=True)
        self._tables: dict[str, pd.DataFrame] = {}
        for name, cfg in self.schema.get("lazy", {}).items():
            self._tables[name] = pd.read_json(os.path.join(self.dir, cfg["table"]), lines=True)
        # **派生表**（`schema.derived`）：数据不在状态里、是引擎算出来的量（本回合流量中间量、
        # 控制面）。它们不像 lazy 表那样由 main 的某个 id 数组索引，而是用 `join_on` 指向
        # main 已有的列（`faction_ids`/`city_ids`）——所以它们单独一段，但读法与 lazy 表一样。
        # 旧版投影没有这一段（`.get(..., {})` ⇒ 空，方法会报出「该投影没有这张表」）。
        for name, cfg in self.schema.get("derived", {}).items():
            path = os.path.join(self.dir, cfg["table"])
            if os.path.exists(path):
                self._tables[name] = pd.read_json(path, lines=True)
        # Static rules dictionary (written by --index alongside schema.json); `None` if absent.
        self.meta: dict | None = None
        meta_path = os.path.join(self.dir, "meta.json")
        if os.path.exists(meta_path):
            with open(meta_path, encoding="utf-8") as fh:
                self.meta = json.load(fh)

    def table(self, name: str, round: int | None = None) -> pd.DataFrame:
        """A lazy table, optionally filtered to one round (for per-round tables)."""
        if name not in self._tables:
            raise KeyError(f"'{name}' is not a lazy table (have: {list(self._tables)})")
        df = self._tables[name]
        if round is not None and "round" in df.columns:
            df = df[df["round"] == round]
        return df

    # --- 派生表：引擎算出来的量（不在状态里，只能由引擎产出）-----------------------------
    #
    # 四张表（见 `schema.json` 的 `derived` 段与笔记 `engine-data-plane.md`）：
    #   faction_process  每回合 × 势力：production{} / upkeep / governance_total / governance_coverage
    #   city_process     每回合 × 城  ：production{}（含 razed 空城）
    #   control          每回合 × 叶片：kind / key / sub / value / mode（谁在控制什么）
    #   scope            每回合 × 显式作用域节点：level / key / mode
    #
    # ⚠ `faction_process` 的**数值**在 mainstream 的 `view.factions[<势力>]` 里也有一份（嵌套对象）；
    #   这些表的价值是**形状**——可直接 join、列类型稳定、按 (round, 名字) 对齐。
    # ⚠ **过程量只在引擎真跑过的回合里才有**（`pre` 里它是 0/空）。从 checkpoint 起跑的投影
    #   （`--start ckpt --round 0 --index`）那一行的过程量是「产生这个状态的那一回合」的量
    #   （引擎把档里存的那份视图写进回合 0 行）；全新开局（`--seed`）的回合 0 只有初始世界，
    #   过程量是 0/空。

    def derived(self, name: str, round: int | None = None) -> pd.DataFrame:
        """One derived table (``faction_process`` / ``city_process`` / ``control`` / ``scope``), by round."""
        if name not in self.schema.get("derived", {}):
            raise KeyError(
                f"'{name}' is not a derived table。该投影的 derived 段是 "
                f"{list(self.schema.get('derived', {}))}；旧版投影没有这一段，"
                f"请用新版 planet_x 重新 `--index`。"
            )
        return self.table(name, round)

    def faction_process(self, round: int | None = None) -> pd.DataFrame:
        """每回合每势力的**过程量**：产出（按资源）/ 舰队维护费 / 治理成本与覆盖率。

        这是「预算压顶」判据的分子分母：`upkeep / production_value` —— 后者可由
        `production` 按 `q.resource_value()` 加权得到，或直接读 `view.factions[<势力>]`
        里的 `production_value`（`q.faction_snapshot()` / :meth:`view_economy` 给的就是那一行）。
        """
        return self.derived("faction_process", round)

    def city_process(self, round: int | None = None) -> pd.DataFrame:
        """每回合每城的**过程量**：开采产出 + 忠诚目标值分项 + 产出/建造的中间量
        （含已夷平的空白城，`razed` 列筛）。

        B2 那四列：`labor`（用工系数，**中性值 1.0**，乘在采矿产出与造舰速率上）、
        `housing_capacity`（人口天花板）、`is_hub`（本城天体是不是首都集散地 ⇒ 产出直进势力池
        还是先落产地货栈）、`build`（`{舰级: {rate, increment}}`——缺钱还是缺产能，用
        :meth:`view_spending` 直接读判据）。
        """
        return self.derived("city_process", round)

    def control(self, round: int | None = None) -> pd.DataFrame:
        """**控制面的 tidy 行**：一行一个叶片（`kind`/`key`/`sub`/`value`/`mode`）。

        `mode` 是叶片自己的三态表态（`Player`/`Auto`/`Inherit`），**不是**有效归属；
        舰的有效指令/风格/角色看 `q.ships()` 的 `order_effective*` / `doctrine` / `kiting` /
        `role` 列（引擎解析，别自己重算链）。`role` 是**三值字符串**（`"War"` 战舰 /
        `"Freight"` 运输舰 / `"Observe"` 观测舰）——旧列名 `freighter` 是布尔，已随
        `ShipRole` 改名，同一行的 `role_mode` 是那片叶的有效归属。
        `sub` 只对权重叶有意义（城内建筑下标）。
        """
        return self.derived("control", round)

    def scope(self, round: int | None = None) -> pd.DataFrame:
        """作用域树的**显式**表态：`level`（global/faction/body/city）+ `key` + `mode`。"""
        return self.derived("scope", round)

    def market_trades(self, round: int | None = None) -> pd.DataFrame:
        """**本回合真的成交的星际贸易**（B3）：一笔一对（买方 × 卖方）一行。

        一行里同时有「价格是怎么算出来的」与「路上丢了多少」：

        * `dist_au` / `depth` / `mond_extra` / `freight_rate` / `rel_mult` / `mastery` / `loss`
          全部**只由这一对决定**（与该笔买的是哪种矿无关）——每种矿的成交价就是
          `q.facts` 里那一行的 `view.market_price[资源] × (rel_mult + freight_rate)`；
        * `moved` 是这一对之间**卖方交出的件数**（按资源）：买方收到的是
          `moved[资源] × (1 − loss)`，`view.market_settled` 记的正是**收到**的那份；
        * **稀疏**：没成交的回合零行（不是「成交了 0 件」）。

        ⚠ 粒度是**一对一行**，不是「一对 × 一资源一行」：想按资源展开就自己拆 `moved`
        （`q.view_trade()` 顺手做了这件事）。
        """
        return self.derived("market_trades", round)

    def haul_steps(self, round: int | None = None) -> pd.DataFrame:
        """**本回合每艘在跑运输的舰走了哪一步**（B3）：`step` ∈ loaded / delivered / waiting / en_route。

        `waiting`（停在**空货栈**干等）与 `en_route`（在路上）**既不落 state 也不发事件**——
        这张表是它们唯一的读法：「我派它去拉货，为什么一件没运回来」= 看 `step` 与
        `q.ships()` 里那艘舰的 `cargo`（舱里空 + `waiting` ⇒ 那处货栈没货）。
        AI 与**玩家指令**两条执行路径都写它，所以玩家舰也在表里。
        """
        return self.derived("haul_steps", round)

    def decisions(self, round: int | None = None) -> pd.DataFrame:
        """**本回合 AI 的判定**（“掷了什么”）：`kind`/`actor`/`verdict`/`target`/`detail`。

        * `kind="ship_order"`：逐舰判定。`verdict` ∈ withdraw / engage / colonize / bombard /
          move / **hold**（`hold` = 这回合 AI 没给这艘舰派活——不是"它在待命"）。
        * `kind="retool"`：船坞改装（`actor` = 城名，`target` = 新舰级）。
        * `detail` 是各 kind 的专属事实：逐舰判定带 `hull_ratio` / `retreat_hull` / `kiting` /
          `enemy_in_range` / `after_move` / `destination` / `order`；改装带 `from` / `building`。

        **一艘舰一回合最多两行**（先机动、到位后再判一次）——按 `actor` 聚合前先看 `after_move`。
        这些判定**既不发事件也不落状态**（指令叶只留结果），所以这是唯一能回答
        「我的舰为什么跑到那儿去送死」的地方；`detail` 里的两个数（血量比 vs 撤退阈值）就是判据。
        """
        return self.derived("decisions", round)

    def blueprints(self, round: int | None = None) -> pd.DataFrame:
        """**舰船设计图库**：一行一张图（`blueprint_id`/`class`/`components`/`order`/`mode`/…）。

        * `mode` = 图叶**自己的**三态表态；`effective_mode` = **引擎解析**的归属
          （图叶 → 势力 scope → 全局；全继承 ⇒ Auto）——别自己重算链。
        * `components` 是**全量**列表（空 = 出厂那一刻交给 `choose_loadout` 现算）；
          `order` 是本图给**新舰**的默认意图（`null` = 本图对意图没有说话）。
        * `ship_count` = 世界上有多少艘舰出自这张图（引擎算）。
        * `launch_waiting` = 这张（玩家归属的）图此刻「进度满了却因买不起选装没下水」。
        * 舰那一侧看 `q.ships()` 的 `blueprint` 列（出厂图名，`null` = 无图）；
          建造区那一侧看 `q.cities()` 的内联 `buildings[].blueprint`（悬空指针 ⇒ 那个区停产）。
        """
        return self.derived("blueprints", round)

    def ships(self, round: int | None = None) -> pd.DataFrame:
        return self.table("ships", round)

    def cities(self, round: int | None = None) -> pd.DataFrame:
        return self.table("cities", round)

    def factions(self, round: int | None = None) -> pd.DataFrame:
        """Factions per round: identity + 库存(resources) + 外交(relations) + 自有城/舰清单。
        ``relations``/``resources``/``city_ids``/``ship_ids`` stay dict- / list-valued cells
        (use :meth:`faction` or :meth:`faction_snapshot` to unpack into a plain read).

        引擎还在这张表上放了**编队配额**那几列（一行一势力、每回合重算），读它们就不用自己
        复算自动控制的判据（列级说明见投影的 ``schema.json``）：

        * ``observer_quota`` —— **观测配额**（目标头数，连续量）：本势力该有几艘舰去太阳系外缘的
          引力异常区蹲着（``autocontrol::knowledge``）；**掌握度到顶 ⇒ 0**（没东西可学了）。
        * ``observer_count`` —— 此刻**真的在观测**的舰数（有效角色 = ``Observe``）。与配额一起读
          就能分开「不想学」（配额 0）与「没人可派」（配额 > 0 而这列跟不上）。
        * ``observer_target`` —— 观测编队的**驻地天体**（``null`` = 异常区里没有候选天体）。
        * ``mond_ships_in_band`` —— 此刻在异常区里的自己的活舰数（掌握度唯一的知识来源）。
        * ``freighter_quota`` / ``freighter_count`` —— 集货那条同形的一对（目标条数 / 现状条数）。
        """
        return self.table("factions", round)

    def bodies(self) -> pd.DataFrame:
        return self.table("bodies")

    def settlements(self) -> pd.DataFrame:
        """Global 定居点 master table (site name / area / ecological capacity / 建设修正 / 矿藏)."""
        return self.table("settlements")

    def faction(self, round: int, name: str) -> dict | None:
        """One faction's row at a round, as a plain dict (relations / resources / city/ship ids)."""
        df = self.factions(round)
        if df.empty:
            return None
        row = df[df["faction_id"] == name]
        return row.iloc[0].to_dict() if len(row) else None

    def faction_snapshot(self, round: int, name: str) -> dict:
        """A single faction's decision view at a round: that faction's row of the round ``view``
        (``view.factions[<faction>]``) merged with the ``factions`` detail (stockpile + relations +
        its city/ship ids). This is the one-call "what's on my mind" — resources, diplomacy,
        economy, fleet and cities.

        ``snap["view"]`` is **that one faction's row** (city_count / ship_count / production_value /
        upkeep / governance_cost / market_value / net_import / …), not the whole round view.
        """
        frow = self.faction(round, name)
        if frow is None:
            return {"faction": name, "exists": False}
        out = dict(frow)
        out["exists"] = True
        # that faction's row of the round view (if the main row carries one)
        fact = self.facts[self.facts["round"] == round]
        if len(fact):
            v = fact.iloc[0].get("view") or {}
            out["view"] = (v.get("factions") or {}).get(name, {})
        return out

    def fleet(self, round: int | None, faction: str) -> pd.DataFrame:
        """A faction's ships at a round, with the effective panel columns the projection now carries."""
        return self.ships(round).query("faction_id == @faction")

    def city_buildings(self, round: int | None, faction: str) -> pd.DataFrame:
        """A faction's cities at a round, including each city's ``buildings`` list (dict-valued)."""
        return self.cities(round).query("faction_id == @faction")

    def relations(self, round: int | None = None) -> pd.DataFrame:
        """Every faction's diplomacy as a **long** table: ``(round, faction_id, other, relation)``.

        ``relations`` in the factions table is a dict-valued cell; this explodes it into one row
        per (source faction → target faction) so you can query "who is hostile to whom" directly,
        e.g. ``q.relations(round=12).query("faction_id=='中国' and relation < -20")``.
        """
        df = self.factions(round)
        if df.empty:
            return df
        rows = []
        for _, r in df.iterrows():
            rel = r.get("relations") or {}
            for other, v in rel.items():
                rows.append({"round": r["round"], "faction_id": r["faction_id"], "other": other, "relation": v})
        return pd.DataFrame(rows, columns=["round", "faction_id", "other", "relation"])

    def _faction_row(self, round: int, name: str) -> dict:
        """That faction's row of the round ``view`` (``view.factions[name]``, sim-computed)."""
        fact = self.facts[self.facts["round"] == round]
        if len(fact) == 0:
            return {}
        return (fact.iloc[0].get("view") or {}).get("factions", {}).get(name, {})

    # --- semantic views (pure retrieval: pack the sim's own computed output) -----------------
    # These only re-read what the simulation already computed and wrote into the round's `view`
    # / the lazy tables. They never re-derive a game rule (power_share/coalition/governance budget
    # verdicts stay in Rust — see `--control-plan`); they are the "common read" layer.

    def view_sitrep(self, round: int) -> dict | None:
        """The world political picture at a round (sim's own aggregate, i.e. the round ``view``):
        totals + hegemon / coalition / sanction / wars / power_share + one row per faction."""
        fact = self.facts[self.facts["round"] == round]
        if len(fact) == 0:
            return None
        v = fact.iloc[0].get("view") or {}
        fid_f = fact.iloc[0].get("faction_ids") or []
        factions = []
        for fid in fid_f:
            fm = (v.get("factions") or {}).get(fid, {})
            factions.append({
                "faction": fid,
                "city_count": fm.get("city_count"),
                "ship_count": fm.get("ship_count"),
                "fleet_value": fm.get("fleet_value"),
                "production_value": fm.get("production_value"),
                "upkeep": fm.get("upkeep"),
                "governance_cost": fm.get("governance_cost"),
                "market_value": fm.get("market_value"),
                "at_war": fm.get("at_war"),
            })
        return {
            "round": round,
            "city_count": v.get("city_count"), "ship_count": v.get("ship_count"),
            "fleet_value": v.get("fleet_value"), "population": v.get("population"),
            "power_share": v.get("power_share"),
            "hegemon": v.get("hegemon"), "sanctioned": v.get("sanctioned"),
            "coalition_members": v.get("coalition_members"), "wars": v.get("wars"),
            "factions": factions,
        }

    def view_frontier(self, round: int, faction: str | None = None, min_loyalty: float | None = None) -> pd.DataFrame:
        """Cities at governance risk (a "frontier" read). Filters by ``faction`` and/or
        ``loyalty <= min_loyalty``, sorted ascending by loyalty. Uses the sim-emitted ``loyalty``
        / ``gov_distance`` / ``revolt_risk`` — never recomputed here."""
        c = self.cities(round)
        if c is None or c.empty:
            return c
        if faction is not None:
            c = c[c["faction_id"] == faction]
        if min_loyalty is not None:
            c = c[c["loyalty"] <= min_loyalty]
        return c.sort_values("loyalty")

    def neutral(self, path: str):
        """A read-face field's **neutral value** (its default), straight out of `schema.json`
        (`neutral.fields`) — declared by the engine, **never invented here**.

        Why this exists: a missing key used to mean different things to different readers
        (`flow.jsonl` substituted `0` for governance coverage while `metrics` substituted `1.0`,
        so the same round read as "0% covered" and "100% covered"). The engine now declares every
        read-face field's neutral value in one place and *uses the same declarations itself*;
        consumers substitute from here instead of hardcoding.

        Use it when a key may be missing (a projection written by an older build, or a field the
        sparse storage omitted)::

            fm.get("governance_scale", q.neutral("factions[].governance_scale"))   # 1.0, not 0
        """
        section = (self.schema or {}).get("neutral") or {}
        return (section.get("fields") or {}).get(path)

    def view_loyalty(self, round: int, faction: str | None = None) -> pd.DataFrame:
        """**「这座城的忠诚为什么在掉」**：每城一行，带上引擎算出的忠诚目标值分项。

        `loyalty_target_*` 来自 `derived.city_process`（引擎在 `step_governance` 里**捕获**的中间量，
        不在这里重算）：`effective` 是这一回合的目标忠诚（实际忠诚朝它恢复），分项就是它为什么低——
        `distance`（离首都太远，再乘人口超载倍率）/ `entertainment`（娱乐预算 × 治理覆盖率）。
        `capital_loyalty_bonus`（首都人口占比带来的向心 buff）与 `ideology_loyalty_penalty`
        （思潮优势端言行不符的扣分）**按势力算一次**，所以它们是从 `faction_process` join 来的，
        完整式子是 `effective = clamp(distance + entertainment + bonus − penalty)`。

        ⚠ `coverage < 1` 时忠诚**改走欠费惩罚**（不朝目标恢复），此时 `effective` 只是「本该到的值」——
        要和 `loyalty`（现状）与 `view_economy()["governance_coverage"]` 一起读。按 `effective` 升序
        （最危险的在最上面）。
        """
        c = self.cities(round)
        p = self.city_process(round)
        f = self.faction_process(round)
        if c is None or c.empty or p is None or p.empty or f is None or f.empty:
            return pd.DataFrame()
        cols = [
            "round", "city_id",
            "loyalty_target_effective", "loyalty_target_distance",
            "loyalty_target_entertainment",
        ]
        # 另两项**按势力算一次**（首都向心项、思潮优势端惩罚）⇒ join `faction_process` 拿，
        # 不在城表里重复存（「同一个数只有一个位置」，见 schema 的 city_process 说明）。
        fcols = ["round", "faction_id", "capital_loyalty_bonus", "ideology_loyalty_penalty"]
        for table, need, cols_ in (("city_process", cols, p.columns), ("faction_process", fcols, f.columns)):
            missing = [x for x in need if x not in cols_]
            if missing:
                raise KeyError(
                    f"derived.{table} 缺列 {missing}——这份投影是「B1 中间量」之前的构建产出的，"
                    f"请用当前 planet_x 重新 `--index`（各字段的中性值见 schema.json 的 neutral 段）"
                )
        out = c.merge(p[cols], on=["round", "city_id"], how="left")
        out = out.merge(f[fcols], on=["round", "faction_id"], how="left")
        if faction is not None:
            out = out[out["faction_id"] == faction]
        return out.sort_values("loyalty_target_effective")

    def view_spending(self, round: int, faction: str) -> dict:
        """**「钱去哪了」**（B2）：本回合该势力的预算去向 + 造舰是缺钱还是缺产能 + 舰队为什么掉血。

        三个读面在这里合成一屏（都是引擎算的中间量，Python 只做减法与 join）：

        * `budget`（DataFrame，逐资源）：**批了多少 − 花了多少 = 没花掉的**。
          限额来自 `derived.control` 的 `investment_budget`/`construction_budget` 叶（引擎每回合把
          当回合用的额度写回去），已花来自 `view.factions[].investment_spent`/`construction_spent`
          ——**同一个数不在两个读面各存一份**，所以这里才要 join。
        * `build`（DataFrame，逐城 × 舰级）：`rate` 是产能上限、`increment` 是实得进度，
          `bottleneck` 直接给出判据——`money`（钱批光了）/ `capacity`（产能封顶）/ `idle`
          （有产能却一分钱没批到）/ `-`（进度满仓在等下水）。
        * `upkeep_unpaid` / `fleet_rust`：付不起的维护费与由此**每艘舰被锈掉的船体比例**
          （锈到 0 才发事件，所以掉血只有这两个数看得见）。

        示例：``q.view_spending(round=30, faction="中国")["budget"]``。
        """
        fm = self._faction_row(round, faction)
        inv_spent = dict(fm.get("investment_spent", self.neutral("factions[].investment_spent")) or {})
        con_spent = dict(fm.get("construction_spent", self.neutral("factions[].construction_spent")) or {})

        ctl = self.control(round)
        def batch(kind: str) -> dict:
            if ctl is None or ctl.empty or "kind" not in ctl.columns:
                return {}
            sel = ctl[(ctl["kind"] == kind) & (ctl["faction_id"] == faction)]
            return {r["key"]: float(r["value"]) for _, r in sel.iterrows()}

        inv_lim, con_lim = batch("investment_budget"), batch("construction_budget")
        rows = []
        for rt in sorted(set(inv_lim) | set(inv_spent) | set(con_lim) | set(con_spent)):
            row = {"resource": rt}
            for kind, lim, spent in (("investment", inv_lim, inv_spent), ("construction", con_lim, con_spent)):
                want = float(lim.get(rt, 0.0))
                got = float(spent.get(rt, 0.0))
                row[f"{kind}_batch"] = want
                row[f"{kind}_spent"] = got
                row[f"{kind}_unspent"] = want - got
            rows.append(row)
        budget = pd.DataFrame(rows)

        # 造舰：每城每舰级的速率与实得进度。判据只用两列比较，不在这里重算引擎公式。
        cs = self.city_process(round)
        build = pd.DataFrame()
        if cs is not None and not cs.empty and "build" in cs.columns:
            missing = [c for c in ("labor", "housing_capacity", "is_hub", "build") if c not in cs.columns]
            if missing:
                raise KeyError(
                    f"derived.city_process 缺列 {missing}——这份投影是「B2 中间量」之前的构建产出的，"
                    f"请用当前 planet_x 重新 `--index`"
                )
            sel = cs[cs["faction_id"] == faction]
            recs = []
            for _, crow in sel.iterrows():
                for cls, line in (crow["build"] or {}).items():
                    rate = float(line.get("rate", 0.0))
                    inc = float(line.get("increment", 0.0))
                    if inc <= 0.0:
                        why = "idle"          # 有产能、一分钱没批到
                    elif inc >= rate - 1e-9:
                        why = "capacity"      # 顶到产能上限（钱还剩着）
                    else:
                        why = "money"         # 钱批光了
                    recs.append({
                        "city_id": crow["city_id"], "class": cls,
                        "rate": rate, "increment": inc, "bottleneck": why,
                    })
            build = pd.DataFrame(recs)

        return {
            "round": round, "faction": faction,
            "budget": budget,
            "build": build,
            "upkeep_unpaid": fm.get("upkeep_unpaid", self.neutral("factions[].upkeep_unpaid")),
            "fleet_rust": fm.get("fleet_rust", self.neutral("factions[].fleet_rust")),
            # B3（市场与运输）：购买力与**买方名次**（0 = 第一个挑）、集货总缺口（`Σ缺口 ÷ Σ要求`）、
            # 以及「谁不卖给我、为什么」（三档：war / cold / coalition）。
            "purchasing_power": fm.get(
                "purchasing_power", self.neutral("factions[].purchasing_power")
            ),
            "market_rank": fm.get("market_rank", self.neutral("factions[].market_rank")),
            "haul_gap": fm.get("haul_gap"),
            "trade_blocked_by": fm.get("trade_blocked_by", self.neutral("factions[].trade_blocked_by")),
        }

    def view_trade(self, round: int, faction: str | None = None) -> dict:
        """**「为什么是这个价 / 我买到的货为什么少了」**（B3）：本回合的成交清单。

        返回 `{"trades": DataFrame, "blocked": DataFrame}`：

        * `trades` —— 一行 = 一笔成交（`buyer` × `seller`），带 `price_mult`（= `rel_mult +
          freight_rate`，**这就是成交价相对市场价的倍数**）与拆开的 `dist_au` / `depth` /
          `mond_extra` / `freight_rate` / `rel_mult` / `mastery` / `loss`；`moved` 是那一对之间
          卖方交出的件数（按资源，**保留原样**——想按资源展开就自己拆，`delivered_units` 一列
          给出「买方收到」的总量 = `Σ moved × (1 − loss)`）。
        * `blocked` —— **谁不卖给我、为什么**（`blocker` / `cause` ∈ war / cold / coalition）。
          三档的对策不同：战争要停战、冷关系要缓和、联盟封锁要拆联盟。
          `cause` 来自势力行（`view.factions[].trade_blocked_by`），是**引擎的判据**，不是这里推的。

        `faction` 给了就只看与它有关的行（买方或卖方 = 它；禁运只看「不卖给它」那些）。
        """
        fm = self._faction_row(round, faction) if faction else None
        rows = []
        blocked = []
        if fm is not None:
            for blocker, cause in (fm.get("trade_blocked_by") or {}).items():
                blocked.append({"blocked_faction": faction, "blocker": blocker, "cause": cause})
        blocked = pd.DataFrame(blocked)

        t = self.market_trades(round)
        if t is None or t.empty:
            return {"round": round, "faction": faction, "trades": t if t is not None else pd.DataFrame(),
                    "blocked": blocked}
        missing = [c for c in ("buyer", "seller", "moved", "dist_au", "depth", "mond_extra",
                               "freight_rate", "rel_mult", "mastery", "loss") if c not in t.columns]
        if missing:
            raise KeyError(
                f"derived.market_trades 缺列 {missing}——这份投影是「B3 中间量」之前的构建产出的，"
                f"请用当前 planet_x 重新 `--index`"
            )
        t = t.copy()
        # 这一列的加法是**文档承诺的那条恒等式**（成交价 = 市场价 × 本列），不是平台重算游戏公式。
        t["price_mult"] = t["rel_mult"] + t["freight_rate"]
        t["delivered_units"] = t.apply(
            lambda r: sum(v * (1.0 - float(r["loss"])) for v in (r["moved"] or {}).values()), axis=1
        )
        if faction is not None:
            rows = t[(t["buyer"] == faction) | (t["seller"] == faction)]
        else:
            rows = t
        return {"round": round, "faction": faction, "trades": rows, "blocked": blocked}

    def view_freight(self, round: int, faction: str) -> dict:
        """**「哪处货栈在积压、我的船这一回合在干什么」**（B3）：集货的两半合在一屏。

        * `depots`（DataFrame，逐货栈）：`need`（要求运力）/ `own`（自有运力的期望份额）/
          `hired`（已雇运力）/ `uncovered`（缺口 = need − own − hired，连续量）。
          势力级的总账是 `haul_gap`（= `Σuncovered ÷ Σneed`，见 `view_economy`）——本表是它的
          **逐货栈**展开：「我的货为什么一直躺在产地」= 哪一处的 `uncovered` 长期不为 0。
        * `steps`（DataFrame，逐舰）：本势力**在跑运输的舰**这一回合走了哪一步
          （loaded / delivered / waiting / en_route）+ 它的舰级/位置/货舱。`waiting` = 停在
          **空货栈**干等（不是故障），`en_route` = 在路上——两者都不落 state、不发事件。
        """
        frow = self._faction_row(round, faction)
        gap = frow.get("freight_gap", self.neutral("factions[].freight_gap")) or {}
        depots = pd.DataFrame(
            [{"body_id": b, **{k: float(v.get(k, 0.0)) for k in ("need", "own", "hired", "uncovered")}}
             for b, v in gap.items()]
        )
        hs = self.haul_steps(round)
        if hs is None or hs.empty or "ship_id" not in hs.columns:
            return {"round": round, "faction": faction, "depots": depots, "steps": pd.DataFrame()}
        ships = self.ships(round)
        cols = [c for c in ("round", "ship_id", "faction_id", "class", "hull", "cargo", "x", "y")
                if c in ships.columns]
        mine = hs.merge(ships[cols], on=["round", "ship_id"], how="left")
        mine = mine[mine["faction_id"] == faction]
        return {"round": round, "faction": faction, "depots": depots, "steps": mine}

    def view_market(self, round: int, faction: str) -> dict | None:
        """A faction's stockpile valued at market prices: per-resource amount & value + total.
        Reads ``resources`` (sim stockpile) and the ``meta.resource_value`` table."""
        frow = self.faction(round, faction)
        if frow is None:
            return None
        res = frow.get("resources") or {}
        rv = self.resource_value()
        rv_map = rv["value"].to_dict() if rv is not None and len(rv) else {}
        entries = []
        for k, v in sorted(res.items(), key=lambda kv: -kv[1] * rv_map.get(kv[0], 1.0)):
            value = rv_map.get(k, 1.0)
            entries.append({"resource": k, "amount": v, "unit_value": value, "value": v * value})
        return {
            "round": round, "faction": faction,
            "resources": entries,
            "total_value": sum(e["value"] for e in entries),
        }

    def _capital_decision(self, round: int, faction: str) -> dict | None:
        """This faction's **capital review/relocation** decision for the round, or `None`.

        `None` means it neither reviewed nor moved — that is what the sparse absence of a row in
        `derived.decisions` (`kind="capital"`) means (not "a review with no numbers"). The numbers
        the events never carry live in the returned dict: `current_cost` / `candidate_cost`
        ("why it did **not** move" is the gap between them) and `relocate_loyalty_cost`.
        """
        d = self.decisions(round)
        if d is None or d.empty or "kind" not in d.columns:
            return None
        sel = d[(d["kind"] == "capital") & (d["faction_id"] == faction)]
        if sel.empty:
            return None
        row = sel.iloc[-1]
        detail = row["detail"] if isinstance(row["detail"], dict) else {}
        return {"verdict": row["verdict"], "target": row["target"], **detail}

    def view_economy(self, round: int, faction: str) -> dict:
        """A faction's economy read (sim-computed numbers + trivial arithmetic): production vs
        upkeep vs governance, net flow, market value, governance coverage. `net < 0` means the
        current fleet/governance is outrunning production (a bleed). NOTE: the *judgement* of
        whether a commanded build budget is sustainable (verdict / sustainable-upkeep /
        rounds-to-insolvency) is game logic — read it from the Rust `--control-plan`, not here.

        ⚠ Two different nets, deliberately named after their source fields: `net_flow` is **computed
        here** (production_value − upkeep − governance_cost, a stock-flow reading), while
        `net_import` is the **engine's own** trade net for the round (bought − sold, by market
        value; >0 = net importer) straight out of this faction's ``view`` row.
        """
        fm = self._faction_row(round, faction)
        prod = fm.get("production_value", 0.0) or 0.0
        upkeep = fm.get("upkeep", 0.0) or 0.0
        gov = fm.get("governance_cost", 0.0) or 0.0
        net = prod - upkeep - gov
        return {
            "round": round, "faction": faction,
            "production_value": prod, "upkeep": upkeep, "governance_cost": gov,
            "net_flow": net, "bleeding": net < -1e-9,
            "market_value": fm.get("market_value"),
            "net_import": fm.get("net_import"),
            # 缺省值**从 schema.json 取**（`q.neutral`），不在这里写 0/1：那两个 1.0（覆盖率、
            # 超载倍率）的中性含义是「没有账要付」，与「能力归零 = 0」是两件事——历史上正是
            # 各读者自己编缺省，才让同一回合的两个读面说 0% 与 100%。
            "governance_coverage": fm.get(
                "governance_coverage", self.neutral("factions[].governance_coverage")
            ),
            # B1：治理开销的**两个来源**（行政 vs 娱乐，乘制裁倍率 = `governance_cost`）、
            # 人口超载倍率、思潮优势端的全国忠诚惩罚，以及本回合的迁都判据（`capital` 是对象：
            # `reviewed` / `candidate` / `current_cost` / `candidate_cost` / `relocated_*`）。
            "governance_admin": fm.get("governance_admin", self.neutral("factions[].governance_admin")),
            "governance_entertainment": fm.get(
                "governance_entertainment", self.neutral("factions[].governance_entertainment")
            ),
            "governance_scale": fm.get("governance_scale", self.neutral("factions[].governance_scale")),
            "ideology_loyalty_penalty": fm.get(
                "ideology_loyalty_penalty", self.neutral("factions[].ideology_loyalty_penalty")
            ),
            "capital_loyalty_bonus": fm.get(
                "capital_loyalty_bonus", self.neutral("factions[].capital_loyalty_bonus")
            ),
            # B2（钱去哪了）：**已花**的预算（缺省从 `schema.json` 取）、付不起的维护费与由此
            # 每艘舰被锈掉的比例。「批了多少」不在这里——它住在控制面，用 `view_spending()` 相减。
            "investment_spent": fm.get(
                "investment_spent", self.neutral("factions[].investment_spent")
            ),
            "construction_spent": fm.get(
                "construction_spent", self.neutral("factions[].construction_spent")
            ),
            "upkeep_unpaid": fm.get("upkeep_unpaid", self.neutral("factions[].upkeep_unpaid")),
            "fleet_rust": fm.get("fleet_rust", self.neutral("factions[].fleet_rust")),
            # 首都评估/迁都是**稀疏判定**（大多数回合 `None`）⇒ 从 derived.decisions 取，不在
            # 每势力一行里。`None` = 这一回合既没评估也没迁。
            "capital": self._capital_decision(round, faction),
            "fleet_value": fm.get("fleet_value"),
            "city_count": fm.get("city_count"), "ship_count": fm.get("ship_count"),
            "at_war": fm.get("at_war"),
        }

    def resource_series(self, faction: str, resource: str) -> pd.Series:
        """A faction's stockpile of one resource over time (monthly, indexed by ``round``).

        Pure retrieval: reads the ``resources`` dict cell in the per-round ``factions`` table.
        Good for spotting a scarcity/hoard trend (e.g. "is my 铁 stockpile being drained?")."""
        df = self.factions()
        if df.empty:
            return pd.Series(dtype=float, name=resource)
        sub = df[df["faction_id"] == faction]
        s = pd.Series(
            [ (r.get("resources") or {}).get(resource, 0.0) for _, r in sub.iterrows() ],
            index=sub["round"].to_numpy(),
            name=resource,
        )
        return s

    def ids(self, field: str, round: int) -> list[str]:
        """The id-array of a lazy field for one round (from the lean main row). Ids are **names**
        (strings), never integers: city/building/faction/ship identity = its unique name."""
        cfg = self.schema["lazy"][field]
        row = self.facts[self.facts["round"] == round]
        return list(row[cfg["id_col"]].iloc[0]) if len(row) else []

    def join(self, field: str, round: int | None = None) -> pd.DataFrame:
        """Explode ``main.facts.<field>_ids`` and merge with the lazy table's full objects.

        Per-round tables (``round == true``) merge on ``(round, <key>)``; global master tables
        (``round == false``) merge on ``<key>`` only.
        """
        cfg = self.schema["lazy"][field]
        key, id_col = cfg["key"], cfg["id_col"]
        detail = self.table(field, round)
        facts = self.facts if round is None else self.facts[self.facts["round"] == round]
        exploded = facts.explode(id_col).rename(columns={id_col: key})
        if cfg.get("round", False):
            return exploded.merge(detail, on=["round", key], how="inner")
        return exploded.merge(detail, on=key, how="inner")

    # --- 事件历史（稀疏里程碑）----------------------------------------------------
    # 设计要点（每条都有实测依据）：
    #   * 事件表是**长表**：一行一事件，「缺失」表现为「没有那一行」，不是 NaN。所以稀疏
    #     字段的统计（计数 / 窗口率 / 首末次 / 结局）都是 groupby().size() 一行的事。
    #   * 参与方一律走统一槽位 `actor_*` / `target_*` / `extra`，**没有 variant 专属列**
    #     （否则会回到「同名多义」：`from`/`to` 在 city_defected 是势力、在
    #     capital_relocated 是天体；以及「同角色多名」：faction/owner/fallen_to 五种拼写）。
    #   * **先按类型取**：`events(type='city_razed')` 会把该类型的载荷摊成稠密列；全集帧
    #     只用于计数/扫描（实测：混帧时 74.8% 单元格是 null，按类型取则相关字段 0% null）。

    def _events_table(self) -> pd.DataFrame:
        if "events" not in self._tables:
            raise KeyError(
                "该投影没有 events 表（可能是旧版 planet_x 写出的、事件仍内联在 main.jsonl "
                "的 facts['events'] 里）。请用新版重新生成：planet_x --round N --index DIR"
            )
        return self._tables["events"]

    @staticmethod
    def _flatten_data(df: pd.DataFrame) -> pd.DataFrame:
        """把 `data` 对象列摊成普通列。只在按**单一类型**取时才做——那时这些字段是稠密的。"""
        if df.empty or "data" not in df.columns:
            return df
        payload = pd.json_normalize(df["data"].tolist())
        if payload.empty:
            return df.drop(columns=["data"])
        payload.index = df.index
        return pd.concat([df.drop(columns=["data"]), payload], axis=1)

    def events(
        self,
        round: int | None = None,
        type: str | None = None,
        types: list[str] | None = None,
        since: int | None = None,
        until: int | None = None,
        salience: str | None = None,
        entity: tuple[str, str] | None = None,
        flatten: bool = True,
    ) -> pd.DataFrame:
        """稀疏事件历史（一行一事件）。

        `type` 取**单个**类型（且 `flatten=True`，默认）时，把该类型的专属载荷 `data` 摊成
        普通列 → 一个**稠密**帧。这是推荐的读法。
        `entity=('city', 城名)` 只保留与该实体相关的事件（等价于 :meth:`history`）。
        """
        df = self._events_table()
        if df.empty:
            return df
        if round is not None:
            df = df[df["round"] == round]
        if since is not None:
            df = df[df["round"] >= since]
        if until is not None:
            df = df[df["round"] <= until]
        if type is not None:
            df = df[df["type"] == type]
        elif types is not None:
            df = df[df["type"].isin(list(types))]
        if salience is not None:
            df = df[df["salience"] == salience]
        if entity is not None:
            kind, eid = entity
            hit = self.actors()
            hit = hit[(hit["entity_kind"] == kind) & (hit["entity_id"] == eid)][["round", "seq"]]
            df = df.merge(hit.drop_duplicates(), on=["round", "seq"], how="inner")
        df = df.sort_values(["round", "seq"]).reset_index(drop=True)
        if flatten and type is not None:
            df = self._flatten_data(df)
        return df

    def round_inputs(self, round: int | None = None) -> dict | None:
        """★ **本回合的输入面**（B5）：引擎这一回合**消费掉**了什么 —— 掷出的随机数 + 判定输入。

        它与同一行的 `view`（**结算面**：观测 + 过程量）是一对，分工由用户裁决：
        *「凡是可能未来与随机/输入有关的东西都放 `pre`」*。这一面**只存在于回合中段**
        （主 `Prng` 的流一旦前进就再也拿不回来），所以它是唯一记下来的地方。

        返回 `{"round", "order": [...], "relation_noise": {...}, "rolls": [...]}`：

        * `order` —— **C7 · 本回合的逐舰解算顺序**：它是「**为什么这艘舰一炮未发就被击沉**」
          的答案（互杀时它排在击沉它的那艘舰**之后**）。⚠ 名单是**洗牌那一刻**的舰集：
          含这一回合稍后被打沉的舰、不含稍后才下水的舰，所以长度可以**大于**回合末的舰数。
        * `relation_noise` —— **C13 · 每对势力的关系噪声**（`{势力: {势力: 增量}}`）：
          「关系为什么**无端抖了一下**」。
        * `rolls` —— `derived_roll` 家族的抽签记录（定编/派单/合同闸门/风格/蓝图/知识……），
          每条含掷出的值**与当时的判据**（骰子可重算，判据不可）。

        ⚠ 这一面**不在 `main.jsonl` 里**（按回合 join 这张表更省），也**不在 `--derived` 的
        `post` 里**——问「谁先手 / 掷了什么」就来这里。空 = 这一回合没跑（round 0 / 起点行）。
        """
        df = self.derived("round_inputs", round)
        if df is None or df.empty:
            return None
        # 一行一回合 ⇒ 交出一个**普通 dict**（调用方要的是 `r["order"]`，不是 pandas 标量）。
        return {k: v for k, v in df.iloc[0].to_dict().items()}

    def salvos(
        self,
        round: int | None = None,
        attacker: str | None = None,
        target: str | None = None,
        since: int | None = None,
        until: int | None = None,
    ) -> pd.DataFrame:
        """★ **逐发分解**（B4）：一行 = 一件武器的一发，把 `attack` 事件的 `data.shots` 摊平。

        这是「**我为什么打不中 / 这一炮为什么没伤害**」的最终答案面——那些数在 B4 之前全部活在
        `resolve_shot` 的栈上，算完就扔：

        * **选择输入**（为什么瞄它）：`score_basic`（距离/克制基础分）、`score_temper`
          （理智↔热血项）、`score_spread`（火力分配**乘数**）——总分 =
          `(score_basic + score_temper) × score_spread`，`q` 顺手算好放进 `score` 列。
        * **结算分解**：`hit`（命中折减 = `hit_factor(武器追踪, 目标速度)`，**确定性折减不是掷骰**）、
          `def_mult`（本土防御）、`pd` / `pd_absorbed`（点防拦截量 / 被它吃掉多少）、
          `absorbed` / `soak`（护盾）、`armor_soak`（护甲硬度减伤）、`hull_pen`（真进船体的伤害）、
          `damage`（这一发总伤害）、`killed`（是不是补刀）。
        * `skipped=True` = **这一发根本没打出去**（目标在它轮到之前就沉了）。
          `target_hull_before` 记着当时目标还剩多少船体。

        ⚠ **`aggregate_damage = 0` 的行是真的**（齐射被点防吃光：`pd_absorbed > 0`、
        `damage = 0`）——B4 之前这种交火**一条事件都不留**。整条事件的总伤害记在
        `aggregate_damage` 列上，逐发是它的下钻：同一 `event_id` 内 `damage` 之和 = 它
        （**本表里差在 `aggregate_damage` 被规整到两位小数上**，最多 0.005；state 里精确相等）。
        """
        ev = self.events(round=round, type="attack", since=since, until=until)
        if ev is None or ev.empty:
            return pd.DataFrame()
        if "shots" not in ev.columns:
            raise KeyError(
                "events.data 里没有 shots 列——这份投影是「B4 战斗中间量」之前的构建产出的，"
                "请用当前 planet_x 重新 `--index`"
            )
        rows = []
        for _, r in ev.iterrows():
            for s in (r["shots"] or []):
                rows.append(
                    {
                        "round": r["round"],
                        "event_id": r["event_id"],
                        "attacker": r["actor_id"],
                        "target": r["target_id"],
                        "aggregate_damage": r["magnitude"],
                        **s,
                    }
                )
        df = pd.DataFrame(rows)
        if df.empty:
            return df
        # 总分是**文档承诺的那条恒等式**（引擎给的三项，Python 只做一次乘加），不是重算判据。
        df["score"] = (df["score_basic"] + df["score_temper"]) * df["score_spread"]
        if attacker is not None:
            df = df[df["attacker"] == attacker]
        if target is not None:
            df = df[df["target"] == target]
        return df.sort_values(["round", "event_id", "weapon"]).reset_index(drop=True)

    def milestones(
        self,
        since: int | None = None,
        until: int | None = None,
        limit: int | None = None,
        entity: tuple[str, str] | None = None,
    ) -> pd.DataFrame:
        """**里程碑层**（判据：后续计算需要访问**无限过去**）——按当前判据**是空的**。

        这一层装的是 Rust 侧 `Salience::Milestone` 的事件。**分层判据不是「重要性」，而是
        「后续计算需要回看多长的历史」**；读者盘点（见 Rust `GameEvent::salience` 的文档）表明
        **没有任何生产逻辑读无限过去**，所以当前没有事件属于这一层——本函数**正常返回空表**。

        它没坏，但**不要**拿它当「重要事件」的过滤器（重要 ≠ 分层）：

        * 想按「读起来重不重要」挑事件 → 用 `weight` 列，或直接 `q.storyboard()`。
        * 想查某个实体的完整历史 → `q.history(kind, id)`（不分层，全都要）。
        * 想查因果（谁打沉的 / 谁夷平的） → `q.cause(kind, id)`。
        * 「史上第一次」这类无限过去的需求由 Rust 侧 `State::chronicle` 承担，与此无关。

        ⚠ **它读的是投影，不是 `State::milestones`**。`--index` 每跑一次都会**截断**目录
        （`File::create`），所以**分段续玩**时投影只覆盖那一段；跨段的完整历史要在 Python 侧
        自己拼：`pd.concat([a.events(), b.events()]).drop_duplicates(subset=['event_id'])`
        ——两个目录在衔接回合上会重叠（实测 9 条 `event_id` 完全相同），忘了去重会静默多算。
        """
        df = self.events(salience="milestone", since=since, until=until, entity=entity)
        if df.empty:
            return df
        cols = ["round", "seq", "event_id", "type", "headline",
                "actor_kind", "actor_id", "target_kind", "target_id"]
        cols = [c for c in cols if c in df.columns]
        out = df[cols].copy()
        if limit is not None and limit >= 0:
            out = out.tail(limit)
        return out.reset_index(drop=True)

    def notables(
        self,
        since: int | None = None,
        until: int | None = None,
        limit: int | None = None,
        entity: tuple[str, str] | None = None,
    ) -> pd.DataFrame:
        """**窗口层**（判据：后续计算需要访问**一定窗口**）——目前只有开战/停战。

        这一层装的是 Rust 侧 `Salience::Notable` 的事件。它当前唯一的生产者与消费者都是**战争**：
        `war_started`/`war_ended` 被记进 `State::notables`，而 Rust 的「记恨地板」
        （`sim::war_scar_floor`）回头看「最近 `history.notable_window` 回合我们打过仗吗」——
        那正是这一层存在的理由（窗口之外的那场战争不再影响任何计算，所以不必长存）。

        ⚠ 这是**模拟内部分层**的直接观测，不是「重要事件列表」：**窗口过期是设计，不是丢失**
        （所以 Rust 侧不报 `dropped`，只报窗口宽度）。要读完整历史请用 `q.history()`。

        列与 `events()` 相同（`round`/`headline`/`type`/`actor_*`/`target_*` + `weight`）。
        """
        df = self.events(salience="notable", since=since, until=until, entity=entity)
        if df.empty:
            return df
        cols = ["round", "seq", "event_id", "type", "weight", "headline",
                "actor_kind", "actor_id", "target_kind", "target_id"]
        cols = [c for c in cols if c in df.columns]
        out = df[cols].copy()
        if limit is not None and limit >= 0:
            out = out.tail(limit)
        return out.reset_index(drop=True)

    def storyboard(self, window: int = 50, min_weight: int = 8) -> pd.DataFrame:
        """**故事板**：把最值得读的事件压成「每 `window` 回合一段」的可读摘要。

        `min_weight` 是**显示门槛**，按投影的 `weight` 列过滤——**那是 0–9 的序数阶梯，不是
        0–100 的分数**：`9`=开战/停战/结盟/迁都、`8`=城市易主或毁灭、`7`=势力重建/剧情、
        `5`=舰的存亡、`2`=撤退/降级、`0`=逐发流水。默认 **8** = 「格局 + 地图要重画的事」
        （实测 seed 7 走 200 回合：2273 条事件里 ≈709 条 ≥8）；调到 7 会把势力重建 churn 也纳入，
        调到 0 = 每一条都列。**踩过的坑**：这个门槛一度写成 60（照着「0–100 分数」的错觉），
        于是故事板**静默返回空表**。

        **这里刻意不按 `salience` 过滤**：分层判据是「后续计算要回看哪段历史」，与「读起来重不
        重要」无关——按它过滤会让故事板在里程碑层清空后再次静默变空。

        返回 `round_from`/`round_to`/`events`（该段的全部标题，换行连接）/`count`。超长轨迹
        （几千回合）里，这是比逐回合快照省几百倍上下文的读法——与 CLI 的 `--digest K` 同构。
        """
        df = self.events()
        if df.empty or "weight" not in df.columns:
            return df
        led = df[df["weight"] >= int(min_weight)]
        if led.empty:
            return led
        w = max(1, int(window))
        led = led.copy()
        led["round_from"] = (led["round"] // w) * w
        led["round_to"] = led["round_from"] + w
        grouped = led.groupby("round_from", sort=True)
        out = grouped.agg(
            round_to=("round_to", "first"),
            count=("headline", "size"),
            events=("headline", lambda s: "\n".join(s.to_list())),
        ).reset_index()
        return out

    def actors(self) -> pd.DataFrame:
        """长表参与方索引 `(round, seq, event_id, entity_kind, entity_id, role)`。

        从统一的 `actor_*` / `target_*` / `extra` 槽位**通用地**展开——**不需要任何 variant
        的字段知识**（这正是归一化的目的）。任意实体的历史 = 在这张表上过滤一次。
        结果缓存（`q._actors_cache`）。
        """
        cached = getattr(self, "_actors_cache", None)
        if cached is not None:
            return cached
        df = self._events_table()
        cols = ["round", "seq", "event_id", "entity_kind", "entity_id", "role"]
        if df.empty:
            out = pd.DataFrame(columns=cols)
            self._actors_cache = out
            return out
        frames = []
        for kc, ic, role in (("actor_kind", "actor_id", "actor"),
                             ("target_kind", "target_id", "target")):
            sub = pd.DataFrame({
                "round": df["round"].to_numpy(),
                "seq": df["seq"].to_numpy(),
                "event_id": df["event_id"].to_numpy(),
                "entity_kind": df[kc].to_numpy(),
                "entity_id": df[ic].to_numpy(),
                "role": role,
            })
            frames.append(sub)
        extra = df[["round", "seq", "event_id", "extra"]].explode("extra")
        # explode 空数组/None 会造出幽灵 NaN 行 —— 必须显式滤掉。
        extra = extra[extra["extra"].notna()]
        if len(extra):
            ex = pd.json_normalize(extra["extra"].tolist())
            ex = ex.rename(columns={"kind": "entity_kind", "id": "entity_id"})
            ex["round"] = extra["round"].to_numpy()
            ex["seq"] = extra["seq"].to_numpy()
            ex["event_id"] = extra["event_id"].to_numpy()
            frames.append(ex[cols])
        out = pd.concat(frames, ignore_index=True)
        out = out[out["entity_id"].notna()].reset_index(drop=True)
        self._actors_cache = out
        return out

    def history(
        self,
        kind: str,
        entity_id: str,
        since: int | None = None,
        until: int | None = None,
        types: list[str] | None = None,
    ) -> pd.DataFrame:
        """★ 一个实体的历史。

        ``q.history('city', '火星-殖民城')`` / ``q.history('ship', '长征-7')`` /
        ``q.history('faction', '中国')``。`kind` ∈ city/ship/faction/body/settlement。
        返回该实体参与过的全部事件（含它作为 actor/target/victim/beneficiary/third 的），
        按时间排序。
        """
        alias = {"cities": "city", "ships": "ship", "factions": "faction",
                 "bodies": "body", "settlements": "settlement"}
        kind = alias.get(kind, kind)
        hit = self.actors()
        hit = hit[(hit["entity_kind"] == kind) & (hit["entity_id"] == entity_id)][["round", "seq"]]
        ev = self._events_table()
        df = ev.merge(hit.drop_duplicates(), on=["round", "seq"], how="inner")
        if since is not None:
            df = df[df["round"] >= since]
        if until is not None:
            df = df[df["round"] <= until]
        if types is not None:
            df = df[df["type"].isin(list(types))]
        return df.sort_values(["round", "seq"]).reset_index(drop=True)

    def cause(self, kind: str, entity_id: str) -> dict:
        """★ 一个实体的**结局**，已解析成可直读的形状。

        * ``kind='ship'``：死因 `death_cause`、**凶手** `killer`/`killer_faction`/`weapon`、
          以及同回合的助攻方 `assists` —— 答「一艘舰被击毁，是被哪艘舰击毁？」。
          仍在役则返回 `alive=True` 与它的出厂记录。
        * ``kind='city'``：最近一次归属/存亡事件（夷平 / 倒戈 / 夺城 / 复垦 / 叛乱）
          —— 答「这座城为什么换主人？被夷平后复垦，还是改旗易帜？」。
        """
        ev = self.history(kind, entity_id)
        if ev.empty:
            return {"kind": kind, "id": entity_id, "found": False}
        if kind in ("ship", "ships"):
            dead = ev[ev["type"] == "ship_destroyed"]
            if dead.empty:
                born = ev[ev["type"] == "ship_spawned"]
                return {"kind": "ship", "id": entity_id, "found": True, "alive": True,
                        "spawned": (born.iloc[0]["data"] if len(born) else None)}
            row = dead.iloc[-1]
            data = row["data"] or {}
            by = data.get("by") or {}
            atk = ev[(ev["type"] == "attack") & (ev["round"] == row["round"])]
            return {
                "kind": "ship", "id": entity_id, "found": True, "alive": False,
                "round": int(row["round"]), "death_cause": data.get("cause"),
                "killer": by.get("ship"), "killer_faction": by.get("faction"),
                "weapon": by.get("weapon"),
                "assists": sorted({a for a in atk["actor_id"].tolist()
                                   if isinstance(a, str) and a != by.get("ship")}),
            }
        rel = ev[ev["type"].isin(["city_razed", "city_defected", "city_overrun",
                                  "colony_founded", "revolt", "resurgence"])]
        if rel.empty:
            return {"kind": kind, "id": entity_id, "found": True, "changes": 0}
        row = rel.iloc[-1]
        return {"kind": kind, "id": entity_id, "found": True, "round": int(row["round"]),
                "type": row["type"], "data": row["data"]}

    def fates(self, since: int | None = None, until: int | None = None,
              kind: str = "ship") -> pd.DataFrame:
        """窗口内的**结局清单**（一次读完，不用手工 join）。

        ``kind='ship'`` → 每艘被击毁的舰一行：`ship/owner/death_cause/killer/killer_faction/weapon`
        （战死 vs 维护费报废由 `death_cause` 区分）。
        ``kind='city'`` → 城的变化事件（夷平/倒戈/夺城/复垦/叛乱）。
        """
        if kind != "ship":
            return self.events(types=["city_razed", "city_defected", "city_overrun",
                                      "colony_founded", "revolt"], since=since, until=until)
        df = self.events(type="ship_destroyed", since=since, until=until, flatten=False)
        if df.empty:
            return pd.DataFrame(columns=["round", "ship", "owner", "death_cause",
                                         "killer", "killer_faction", "weapon"])

        def pick(d, *path):
            cur = d or {}
            for p in path:
                cur = (cur or {}).get(p) if isinstance(cur, dict) else None
            return cur

        out = pd.DataFrame({
            "round": df["round"].to_numpy(),
            "ship": [pick(d, "ship") for d in df["data"]],
            "owner": [pick(d, "owner") for d in df["data"]],
            "death_cause": [pick(d, "cause") for d in df["data"]],
            "killer": [pick(d, "by", "ship") for d in df["data"]],
            "killer_faction": [pick(d, "by", "faction") for d in df["data"]],
            "weapon": [pick(d, "by", "weapon") for d in df["data"]],
        })
        return out

    def changes(self, kind: str, entity_id: str) -> pd.DataFrame:
        """**纯 dense-diff 视图**：某个实体在密集表里的关键列**发生变化的那些回合**。

        与事件历史互证——这是「不靠事件、只看快照差异」的独立口径（:meth:`audit` 就是两者的
        差集）。城默认比较 `faction_id`/`razed`/`population`；舰比较 `faction_id`/`hull`/`class`。

        **它单独用是不够的**：dense-diff **因果盲**（说不出被谁击毁 / 被谁夷平），而且
        **同回合的 raze→recolonize 差异为空**（事件才是正本）。对舰它还会显式给出
        「消失的那一回合」（密集表里被毁舰直接没有行）。
        """
        alias = {"cities": "city", "ships": "ship", "factions": "faction"}
        kind = alias.get(kind, kind)
        spec = {
            "city": ("cities", "city_id", ["faction_id", "razed", "population"]),
            "ship": ("ships", "ship_id", ["faction_id", "hull", "class"]),
            "faction": ("factions", "faction_id", ["capital_body"]),
        }
        if kind not in spec:
            raise ValueError(f"changes() 支持 city/ship/faction，收到 {kind!r}")
        table, key, cols = spec[kind]
        df = self.table(table)
        if df is None or df.empty:
            return df
        df = df[df[key] == entity_id].sort_values("round")
        if df.empty:
            return df
        cols = [c for c in cols if c in df.columns]
        # 重索引到「首次出现 → 全局末回合」，让**消失**也表现为一次变化（NaN）。
        first, last = int(df["round"].min()), int(self.facts["round"].max())
        df = df.set_index("round").reindex(range(first, last + 1))
        present = df[key].notna()
        prev = df[cols].shift()
        same = df[cols].eq(prev) | (df[cols].isna() & prev.isna())
        changed = (~same.all(axis=1)) | (present != present.shift())
        out = df[changed].reset_index()
        return out[["round", key] + cols].reset_index(drop=True)

    def audit(self) -> pd.DataFrame:
        """**完备性自查**：密集快照里可见、却没有任何事件解释的城状态变化（**应为空**）。

        与 Rust 侧守卫 ``every_city_state_change_is_explained_by_an_event`` 是同一个不变量，
        这里把它暴露给 agent：若返回非空，说明当前投影回答不了「这座城市为什么变了」。
        """
        cities = self.cities()
        cols = ["round", "city_id", "was", "now"]
        if cities is None or cities.empty:
            return pd.DataFrame(columns=cols)
        named = self.actors()
        named = named[named["entity_kind"] == "city"][["round", "entity_id"]].drop_duplicates()
        out = []
        prev: dict[str, tuple] = {}
        prev_round = None
        for _, r in cities.sort_values(["round", "city_id"]).iterrows():
            if r["round"] != prev_round:
                prev, prev_round = {}, r["round"]
            now = (r["faction_id"], bool(r["razed"]))
            was = prev.get(r["city_id"])
            if was is not None and was != now:
                hit = named[(named["round"] == r["round"]) & (named["entity_id"] == r["city_id"])]
                if hit.empty:
                    out.append({"round": int(r["round"]), "city_id": r["city_id"],
                                "was": was, "now": now})
            prev[r["city_id"]] = now
        return pd.DataFrame(out, columns=cols)

    def spec(self, section: str) -> pd.DataFrame | None:
        """One config section of ``meta.json`` as a DataFrame (index = spec name/key).

        E.g. ``spec("ships")`` → rows = ship class, columns = hull/upkeep/build_points/…,
        ``build_cost``/``cost`` stay as dict-valued cells (use ``.meta`` for the raw dict).
        Returns ``None`` when the section is absent (no ``meta.json`` written).
        """
        if self.meta is None:
            return None
        kv = self.meta.get(section)
        if not isinstance(kv, dict) or not kv:
            return None
        df = pd.DataFrame.from_dict(kv, orient="index")
        df.index.name = section
        return df

    def ships_spec(self) -> pd.DataFrame | None:
        return self.spec("ships")

    def buildings_spec(self) -> pd.DataFrame | None:
        return self.spec("buildings")

    def components_spec(self) -> pd.DataFrame | None:
        return self.spec("components")

    def structures_spec(self) -> pd.DataFrame | None:
        return self.spec("structures")

    def resource_value(self) -> pd.DataFrame | None:
        """``market.resource_value`` as a DataFrame: index = resource key (可读名), col ``value``."""
        if self.meta is None:
            return None
        rv = self.meta.get("market", {}).get("resource_value")
        if not isinstance(rv, dict) or not rv:
            return None
        df = pd.DataFrame({"value": pd.Series(rv)})
        df.index.name = "resource"
        return df

    # --- statistics (mean / resample) ----------------------------------------

    def metric_series(self, path: str) -> pd.Series:
        """A monthly Series of a metric from the lean main stream, indexed by ``round``.

        ``path`` is dot-separated into each fact row: e.g. ``"population"`` (a facts column),
        ``"view.population"``, ``"view.city_count"``, ``"view.fleet_value"``, or a per-faction
        metric like ``"view.factions.中国.production_value"``. Missing steps yield NaN.

        Every round = 1 month, so ``round`` is already a fine month index to resample.
        """
        parts = path.split(".")

        def get(row):
            cur = row
            for p in parts:
                if isinstance(cur, dict) and p in cur:
                    cur = cur[p]
                else:
                    return float("nan")
            return cur

        s = pd.Series(
            [get(r) for r in self.facts.to_dict("records")],
            index=self.facts["round"].to_numpy(),
            name=path,
        )
        return s

    def window_avg(self, path: str, size: int, agg: str = "mean") -> pd.Series:
        """Aggregate a metric into windows of ``size`` rounds (1 round = 1 month), indexed by each
        window's starting ``round``. Default ``agg='mean'`` (年均 / 十年均); pass ``'sum'``,
        ``'max'``, ``'min'``, ``'last'`` for other window aggregations.
        """
        s = self.metric_series(path)
        g = s.index // size  # window start round labels (0, size, 2*size, …)
        out = s.groupby(g).agg(agg)
        out.index.name = "round"
        return out

    def yearly_avg(self, path: str, agg: str = "mean") -> pd.Series:
        """年均：把逐月指标按年（12 回合 = 12 月）平均，index = 该年的起始回合 (round)。"""
        return self.window_avg(path, 12, agg)

    def decadal_avg(self, path: str, agg: str = "mean") -> pd.Series:
        """十年均：按 120 个月（10 年）平均，index = 该十年的起始回合 (round)。"""
        return self.window_avg(path, 120, agg)


def load(dirpath: str) -> PlanetXQ:
    return PlanetXQ(dirpath)


def main(argv: list[str] | None = None) -> int:
    import argparse

    p = argparse.ArgumentParser(description="Read a planet_x --index projection into pandas.")
    p.add_argument("dir", nargs="?", default=".", help="projection dir written by --index")
    a = p.parse_args(argv)
    q = load(a.dir)
    print("# main facts:", q.facts.shape, "rounds:", list(q.facts["round"]))
    for name in q.schema.get("lazy", {}):
        tbl = q.table(name)
        print(f"# {name} rows: {tbl.shape}  cols: {list(tbl.columns)}")
    if q.meta is not None:
        ss = q.ships_spec()
        rv = q.resource_value()
        print(f"# meta.json loaded: {len(q.meta)} sections; ships_spec={ss.shape}; resources={rv.shape[0] if rv is not None else 0}")
    if len(q.facts):
        r = 0
        joined = q.join("ships", round=r)
        print(f"# join('ships', round={r}): {joined.shape}")
        cols = [c for c in ("round", "ship_id", "faction_id", "class", "hull", "x", "y") if c in joined.columns]
        print(joined[cols].head(5).to_string(index=False))
    # The sparse event history + its completeness self-check (empty = all history is explained).
    try:
        ev = q.events()
        print(f"# events 历史: {ev.shape}; types={dict(ev['type'].value_counts())}")
        print(f"# audit() 未解释的城状态变化: {len(q.audit())} 条（应为 0）")
        nb = q.notables()
        print(f"# 窗口层 notables: {nb.shape}")
        for line in nb.tail(8).itertuples():
            print(f"#   r{line.round}  {line.headline}")
        ms = q.milestones()
        print(f"# 里程碑层 milestones: {ms.shape}（按当前判据为空 = 正常，见 docstring）")
        sb = q.storyboard()
        print(f"# 故事板 storyboard: {sb.shape}（weight>=60 压成每 50 回合一行）")
    except KeyError as exc:
        print(f"# (no event history in this projection: {exc})")
    except AttributeError as exc:
        print(f"# (history helpers unavailable: {exc})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
