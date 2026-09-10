"""planet_xq — read a `planet_x --index` projection into pandas.

The Rust tool projects the world into a **lean main stream** (``main.jsonl``: eager fields +
``metrics`` + id-arrays) and **id-indexed lazy tables** (``idx/*.jsonl``: the heavy entity
objects). This kit loads both and exposes join helpers so an agent fetches heavy detail by id
without hand-rolling the merge.

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
    q.yearly_avg("metrics.cities")              # 年均 (round = 1 month, 12/年)
    q.decadal_avg("metrics.factions.中国.market_value")  # 十年均 (120 月)

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
    #   flow       每回合 × 势力：production{} / upkeep / governance_total / governance_coverage
    #   city_flow  每回合 × 城  ：production{}（含 razed 空城）
    #   control    每回合 × 叶片：kind / key / sub / value / mode（谁在控制什么）
    #   scope      每回合 × 显式作用域节点：level / key / mode
    #
    # ⚠ `flow` 的**数值**在 mainstream 的 `metrics.factions[<势力>]` 里也有一份（嵌套对象）；
    #   这些表的价值是**形状**——可直接 join、列类型稳定、按 (round, 名字) 对齐。
    # ⚠ 从 checkpoint 起跑的投影（`--start ckpt --round 0 --index`）那一行的 flow 是
    #   「产生这个状态的那一回合」的流量（引擎现在还额外把档里存的派生态写进回合 0 行）；
    #   全新开局（`--seed`）的回合 0 没有流量，是 `{}`。

    def derived(self, name: str, round: int | None = None) -> pd.DataFrame:
        """One derived table (``flow`` / ``city_flow`` / ``control`` / ``scope``), by round."""
        if name not in self.schema.get("derived", {}):
            raise KeyError(
                f"'{name}' is not a derived table。该投影的 derived 段是 "
                f"{list(self.schema.get('derived', {}))}；旧版投影没有这一段，"
                f"请用新版 planet_x 重新 `--index`。"
            )
        return self.table(name, round)

    def flow(self, round: int | None = None) -> pd.DataFrame:
        """每回合每势力的流量中间量：产出（按资源）/ 舰队维护费 / 治理成本与覆盖率。

        这是「预算压顶」判据的分子分母：`upkeep / production_value` —— 后者可由
        `production` 按 `q.resource_value()` 加权得到，或直接读 `q.factions()` 的同名列。
        """
        return self.derived("flow", round)

    def city_flow(self, round: int | None = None) -> pd.DataFrame:
        """每回合每城的开采产出（含已夷平的空白城，`razed` 列筛）。"""
        return self.derived("city_flow", round)

    def control(self, round: int | None = None) -> pd.DataFrame:
        """**控制面的 tidy 行**：一行一个叶片（`kind`/`key`/`sub`/`value`/`mode`）。

        `mode` 是叶片自己的三态表态（`Player`/`Auto`/`Inherit`），**不是**有效归属；
        舰的有效指令看 `q.ships()` 的 `order_effective*` / `doctrine` / `kiting` 列
        （引擎解析，别自己重算链）。`sub` 只对权重叶有意义（城内建筑下标）。
        """
        return self.derived("control", round)

    def scope(self, round: int | None = None) -> pd.DataFrame:
        """作用域树的**显式**表态：`level`（global/faction/body/city）+ `key` + `mode`。"""
        return self.derived("scope", round)

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

    def ships(self, round: int | None = None) -> pd.DataFrame:
        return self.table("ships", round)

    def cities(self, round: int | None = None) -> pd.DataFrame:
        return self.table("cities", round)

    def factions(self, round: int | None = None) -> pd.DataFrame:
        """Factions per round: identity + 库存(resources) + 外交(relations) + 自有城/舰清单。
        ``relations``/``resources``/``city_ids``/``ship_ids`` stay dict- / list-valued cells
        (use :meth:`faction` or :meth:`faction_snapshot` to unpack into a plain read)."""
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
        """A single faction's decision view at a round: the lean ``metrics.factions`` numbers
        merged with the ``factions`` detail (stockpile + relations + its city/ship ids). This is
        the one-call "what's on my mind" — resources, diplomacy, economy, fleet and cities."""
        frow = self.faction(round, name)
        if frow is None:
            return {"faction": name, "exists": False}
        out = dict(frow)
        out["exists"] = True
        # lean metrics per faction (if the main row carries them)
        fact = self.facts[self.facts["round"] == round]
        if len(fact):
            m = fact.iloc[0].get("metrics") or {}
            out["metrics"] = (m.get("factions") or {}).get(name, {})
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

    def _faction_metrics(self, round: int, name: str) -> dict:
        """The lean ``metrics.factions[name]`` numbers for one round (sim-computed)."""
        fact = self.facts[self.facts["round"] == round]
        if len(fact) == 0:
            return {}
        return (fact.iloc[0].get("metrics") or {}).get("factions", {}).get(name, {})

    # --- semantic views (pure retrieval: pack the sim's own computed output) -----------------
    # These only re-read what the simulation already computed and wrote into `metrics` / the lazy
    # tables. They never re-derive a game rule (power_share/coalition/governance budget verdicts
    # stay in Rust — see `--control-plan`); they are the "common read" layer.

    def view_sitrep(self, round: int) -> dict | None:
        """The world political picture at a round (sim's own aggregate): totals + hegemon /
        coalition / sanction / wars / power_share + one row per faction."""
        fact = self.facts[self.facts["round"] == round]
        if len(fact) == 0:
            return None
        m = fact.iloc[0].get("metrics") or {}
        fid_f = fact.iloc[0].get("faction_ids") or []
        factions = []
        for fid in fid_f:
            fm = (m.get("factions") or {}).get(fid, {})
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
            "cities": m.get("cities"), "ships": m.get("ships"),
            "fleet_value": m.get("fleet_value"), "population": m.get("population"),
            "power_share": m.get("power_share"),
            "hegemon": m.get("hegemon"), "sanctioned": m.get("sanctioned"),
            "coalition_members": m.get("coalition_members"), "wars": m.get("wars"),
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

    def view_economy(self, round: int, faction: str) -> dict:
        """A faction's economy read (sim-computed numbers + trivial arithmetic): production vs
        upkeep vs governance, net flow, market value, governance coverage. `net < 0` means the
        current fleet/governance is outrunning production (a bleed). NOTE: the *judgement* of
        whether a commanded build budget is sustainable (verdict / sustainable-upkeep /
        rounds-to-insolvency) is game logic — read it from the Rust `--control-plan`, not here."""
        fm = self._faction_metrics(round, faction)
        prod = fm.get("production_value", 0.0) or 0.0
        upkeep = fm.get("upkeep", 0.0) or 0.0
        gov = fm.get("governance_cost", 0.0) or 0.0
        net = prod - upkeep - gov
        return {
            "round": round, "faction": faction,
            "production_value": prod, "upkeep": upkeep, "governance_cost": gov,
            "net_flow": net, "bleeding": net < -1e-9,
            "market_value": fm.get("market_value"),
            "governance_coverage": fm.get("governance_coverage"),
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
        ``"metrics.population"``, ``"metrics.cities"``, ``"metrics.fleet_value"``, or a per-faction
        metric like ``"metrics.factions.中国.production_value"``. Missing steps yield NaN.

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
