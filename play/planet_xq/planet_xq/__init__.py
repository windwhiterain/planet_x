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
    DIR/idx/ships.jsonl  (round, ship_id, ...)  per-round ship detail
    DIR/idx/cities.jsonl (round, city_id, ...)  per-round city detail
    DIR/idx/bodies.jsonl (body_id, ...)         global master table

Typical use::

    import planet_xq
    q = planet_xq.load("out")
    q.facts                 # pandas DataFrame: the lean main stream
    q.ships(round=10)       # ships at round 10 (from the index, joined by id)
    q.cities(round=10)
    q.bodies()              # global master
    q.join("ships", round=10)  # explode main.ship_ids and merge with the ship detail table
    q.ships_spec()          # config: ship class -> spec as a DataFrame (joinable with q.facts)
    q.resource_value()      # config: resource key (可读名) -> value
    q.yearly_avg("metrics.cities")              # 年均 (round = 1 month, 12/年)
    q.decadal_avg("metrics.factions.中国.market_value")  # 十年均 (120 月)
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

    def ships(self, round: int | None = None) -> pd.DataFrame:
        return self.table("ships", round)

    def cities(self, round: int | None = None) -> pd.DataFrame:
        return self.table("cities", round)

    def bodies(self) -> pd.DataFrame:
        return self.table("bodies")

    def ids(self, field: str, round: int) -> list[int]:
        """The id-array of a lazy field for one round (from the lean main row)."""
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
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
