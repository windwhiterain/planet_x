"""planet_xq — read a `planet_x --index` projection into pandas.

The Rust tool projects the world into a **lean main stream** (``main.jsonl``: eager fields +
``metrics`` + id-arrays) and **id-indexed lazy tables** (``idx/*.jsonl``: the heavy entity
objects). This kit loads both and exposes join helpers so an agent fetches heavy detail by id
without hand-rolling the merge.

Directory layout written by ``planet_x --round N --index DIR``:

    DIR/schema.json      agent-readable projection contract (eager / lazy / columns / read_order)
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
    if len(q.facts):
        r = 0
        joined = q.join("ships", round=r)
        print(f"# join('ships', round={r}): {joined.shape}")
        cols = [c for c in ("round", "ship_id", "faction_id", "class", "hull", "x", "y") if c in joined.columns]
        print(joined[cols].head(5).to_string(index=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
