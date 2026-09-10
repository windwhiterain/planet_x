"""planet_x_ctl — turn pandas analysis into a **legal ``--apply`` control diff**.

This is the **write-side** twin of :mod:`planet_xq` (the read-side kit). The engine
(``planet_x``) owns exactly two things (see ``.agents/notes/engine-data-plane.md``):

1. it **emits** standard tidy tables (``--index`` → ``main.jsonl`` + ``idx/*.jsonl``);
2. it **accepts** a diff of the same shape (``--apply`` → ``{control:[…], scope:{…}}``).

Everything else — wildcards, rosters (编制表), statistical selection, recipes, verification —
lives here, in Python. There is deliberately **no** engine-side ``{"ship":"*"}`` wildcard and no
``clear_ship_orders`` verb: "把中国的舰全设成自动" is this kit expanding one call into N explicit
``{ship, mode}`` leaves, which is the same thing as handwriting them.

Tri-state ownership (``.agents/notes/control-live-layers.md``)
------------------------------------------------------------

Every controllable leaf carries a **mode**: ``Inherit`` (this layer says nothing) / ``Auto`` (the
system decides) / ``Player`` (the player decides). The ownership chain for a ship's order is
``leaf → faction default_ship_order → faction scope → global scope``; the most specific layer that
is not ``Inherit`` wins, and all-``Inherit`` falls back to ``Auto``. The **three style** axes have the
same shape with their own faction-level default: ``ship_doctrine → default_doctrine``,
``ship_kiting → default_kiting`` and ``ship_freighter → default_freighter`` (角色：运输舰↔战舰);
``default_doctrine`` speaks **two axes in one leaf** (``temper`` + ``lone_wolf``), which is why this
kit refuses to create it from a single-axis patch (see ``Surface._require_both_axes``).

⚠ One axis is **alive**: ``ship_freighter`` is written by the automatic controller every round
(集货定编, ``autocontrol::freight``), but only under ``Inherit`` — a ``Player`` leaf freezes it. So on
that axis "不表态" means "AI 可以每回合重新决定", and deleting the leaf means **放手**, not freezing
(``Surface.remove_freighter``).

**写值即接管 (writing a value takes over).** In a diff, writing a leaf's ``value``/``behavior``
while omitting ``mode`` silently turns that leaf into ``Player`` (the engine reports it on stderr as
``NOTE_APPLY_TOOKOVER``). Writing **only** ``mode`` is legal and leaves the value untouched. Every
bulk helper in this kit therefore *requires* an explicit ``mode`` — or an explicit
``take_over=True`` acknowledgement — so a fleet can never be taken over by accident.

Same-round transform (hard constraint)
--------------------------------------

``building`` inside ``build_weights`` / ``invest_weights`` is a **per-city ``u32`` index**
(``InvestKey = BuildKey = (CityId, BuildingId)``). Indices are only self-consistent within one
round, so the only correct workflow is **read a checkpoint → emit a diff → apply it to that same
checkpoint**. Cross-round mixing of building indices is silently wrong. This kit enforces it: every
``(city, building)`` write is resolved against the projection of *the very checkpoint the surface
was read from*.

Typical use::

    import planet_x_ctl as ctl

    ckpt = "ckpt_r12.ron"
    s = ctl.surface(ckpt)                    # `--control` (read face == write face)
    s.factions                               # faction names
    s.leaf("中国", "ship_orders", "长城")      # one leaf: value + mode
    ships = ctl.ships(ckpt)                  # projection ships × their control leaves
    mine = ctl.query(ships, "faction_id == '中国'")   # (extra columns include `class`, a keyword)

    s.set_mode(mine, "Auto")                 # wildcard, client-side: N × {ship, mode}
    s.set_behavior(mine, "Dock:地球", mode="Player")
    s.set_default_ship_order("中国", behavior="Idle", mode="Player")
    s.set_budget("中国", "construction_budget", {"硅": 4.0}, mode="Player")

    diff = s.emit()                          # {"control":[…], "scope":{…}} → `--apply`
    ctl.write(diff, "steer.json")
    rep = ctl.verify(ckpt, diff)             # read-only: before/after read face + receipts
    assert rep.ok

See ``README.md`` for the division of labour, the two documented pitfalls, and the
``_approx`` caveat on the local ``effective`` columns.
"""
from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass, field as _dc_field
from pathlib import Path
from typing import Any, Mapping, Sequence

import pandas as pd

__all__ = [
    "INHERIT", "AUTO", "PLAYER", "MODES", "LEAF_KINDS", "APPROX_COLUMNS",
    "Leaf", "Surface", "Report", "Request", "LeafChange", "Applied",
    "surface", "ships", "cities", "ships_and_cities", "buildings", "projection",
    "roster", "write", "dumps", "verify", "apply", "load_json", "query",
    "resolve_engine", "control_schema", "run_engine", "new_checkpoint",
    "normalize_behavior", "behavior_str", "main",
]

INHERIT, AUTO, PLAYER = "Inherit", "Auto", "Player"
MODES = (INHERIT, AUTO, PLAYER)
"""The three serialized mode names — exactly ``"Inherit" | "Auto" | "Player"`` in JSON."""

#: Leaf kinds of the control face, and the fields that form a leaf's **identity**.
#: ``()`` = the kind is a single per-faction leaf (an object, not a list).
LEAF_KINDS: dict[str, tuple[str, ...]] = {
    "capital": (),
    "default_ship_order": (),
    # 势力级默认风格两片（`control-live-layers.md` §4.1）：与 `default_ship_order` 同形，
    # 只是各有自己的轴——`default_doctrine` 是 temper/lone_wolf，`default_kiting` 是 kiting。
    # ⚠ 少了这两项时**不会报错**，只会静静地从 surface() 里消失（读面有、这里看不见）——
    # 那正是 `engine-data-plane.md` §8.1 的教训：契约是发射端 + 消费者两处。
    "default_doctrine": (),
    "default_kiting": (),
    # 势力级默认**角色**（运输舰↔战舰，第三条风格轴）、逐舰角色叶。与另两条风格轴同形，
    # 但有一条轴间差别：**逐舰那片叶自动控制每回合也会写**（按积压定编，
    # `autocontrol::freight`）；玩家钉住（`mode='Player'`）之后 AI 不再碰它。
    "default_freighter": (),
    "ship_orders": ("ship",),
    "ship_doctrine": ("ship",),
    "ship_kiting": ("ship",),
    "ship_freighter": ("ship",),
    "investment_budget": ("resource",),
    "construction_budget": ("resource",),
    "invest_weights": ("city", "building"),
    "build_weights": ("city", "building"),
    "loyalty_budget": ("city",),
    # **设计图库**（势力级）：图名 → 图纸。设计图是「还不存在的舰」的出厂规格——建造区**指向**
    # 一张图（结构叶，见 `Surface.set_blueprint_pointer`），下水那一刻把图**印成**一艘舰。
    #
    # 它是**复合值**叶（class + components[] + order{}），所以 `_leaf_value` 有一个专用分支；
    # 三态语义照旧：`Player` = 系统不许重估这张图（出厂按图装配）／`Auto` = 系统可重估
    # （`retool_shipyards` 会改它的舰级）／`Inherit` = 这一层没有说话（沿 scope 链上溯）。
    # ⚠ 图的**意图轴**默认沉默：只有图上真写了 `order`（且归属是 Player）才遮住舰队默认。
    "blueprints": ("name",),
}

_KIND_ORDER = tuple(LEAF_KINDS)
_KEY_FIELDS = frozenset({"ship", "city", "building", "resource", "name"})
#: Which read-face field carries a leaf's **value**. Three kinds spell it something other than
#: ``value``: `ShipOrderPatch`/`DefaultShipOrder` write ``behavior``, `ShipKitingPatch`/`DefaultKiting`
#: write ``kiting``. `ShipDoctrinePatch`/`DefaultDoctrine` have two axes at once
#: (``temper`` / ``lone_wolf``), and ``blueprints`` is a **composite** (class + components + order).
_VALUE_FIELD = {"ship_orders": "behavior", "default_ship_order": "behavior",
                "ship_kiting": "kiting", "default_kiting": "kiting", "capital": "value",
                "ship_freighter": "freighter", "default_freighter": "freighter"}

_TWO_AXIS_KINDS = ("ship_doctrine", "default_doctrine")
#: 复合值叶（一张设计图 = 舰级 + 选装 + 意图）。与两轴风格叶同理：值不止一个字段，所以
#: `_leaf_value` 不能靠一个字段名取。
_COMPOSITE_KINDS = ("blueprints",)
_BLUEPRINT_FIELDS = ("class", "components", "order")


def _leaf_value(kind: str, entry: Mapping) -> Any:
    if kind in _TWO_AXIS_KINDS:
        return {k: entry.get(k) for k in ("temper", "lone_wolf")}
    if kind in _COMPOSITE_KINDS:
        return {k: entry.get(k) for k in _BLUEPRINT_FIELDS}
    return entry.get(_VALUE_FIELD.get(kind, "value"))


def _component_id(c: Any) -> str:
    """组件 id 的一个便宜校验：配方期挡住 ``1`` / ``None`` 这类噪声（引擎那边只看表里有不有）。"""
    if not isinstance(c, str) or not c:
        raise ValueError(
            f"组件 id 必须是非空字符串（收到 {c!r}）——可选值见 `q.components_spec()`（`--meta` 的 components 表）。"
        )
    return c


def _fix_query(expr: str) -> str:
    """Back-quote the engine's keyword-named ``class`` column so ``DataFrame.query`` accepts it.

    The projection's ships table has a column literally named ``class`` — a Python keyword, which
    ``DataFrame.query`` refuses unless it is back-quoted (``SyntaxError: Python keyword not valid
    identifier in numexpr query``). The recipes in the notes are written as
    ``"class=='cruiser' and faction_id=='中国'"``, so :func:`query` (and :func:`roster`) add the
    backticks for you. Anything already back-quoted is left alone.
    """
    return re.sub(r"(?<![\w`])class(?![\w`])", "`class`", expr)


def query(df: pd.DataFrame, expr: str, **names) -> pd.DataFrame:
    """``df.query(expr)``, but tolerant of the engine's keyword-named ``class`` column.

    ``@name`` references are resolved against **your** frame (not this helper's), so recipes can keep
    writing ``df.query("faction_id == @faction")``-style expressions.
    """
    if not len(df):
        return df
    fixed = _fix_query(expr)
    if "@" in fixed:
        import inspect

        frame = inspect.currentframe().f_back
        try:
            scope = dict(frame.f_globals)
            scope.update(frame.f_locals)
        finally:
            del frame
        scope.update(names)
        return df.query(fixed, local_dict=scope)
    return df.query(fixed, **names)


_WEIGHT_KINDS = ("invest_weights", "build_weights")
_BUDGET_KINDS = ("investment_budget", "construction_budget")

#: Columns this kit computes **locally** instead of reading them from the engine.
#: They are an *approximation* of the engine's chain resolution — see the README.
APPROX_COLUMNS = (
    "effective_order_mode_approx",
    "effective_order_value_approx",
    "effective_authority_approx",
)

#: Float tolerance for `verify`. The control **read face is lossless**: it used to print every numeric
#: leaf rounded to 2 decimals (so writing `0.7131` showed up as `0.71`, and a verbatim round-trip
#: silently quantized it to `0.01`); that rounding is **gone** — the template is now the stored value
#: bit for bit (guard: `src/control.rs::the_control_template_never_rounds_a_leaf_value`). So a float
#: mismatch beyond float-repr slop here means the write really did **not** land.
#:
#: (If you point this kit at a binary older than that change you will see "did not land" for values
#: whose only difference is the old rounding — that is the old engine, not your recipe.)
_FLOAT_EPS = 1e-9


def _values_match(requested: Any, after: Any) -> bool:
    """Did the read face end up holding exactly what we asked for? (lossless read face ⇒ exact.)"""
    if isinstance(requested, bool) or isinstance(after, bool):
        return requested == after
    if isinstance(requested, (int, float)) and isinstance(after, (int, float)):
        return abs(float(requested) - float(after)) <= _FLOAT_EPS
    return requested == after


_HONESTY = (
    "verify() proves the ENGINE ACCEPTED THE DIFF (every requested leaf landed, nothing was "
    "skipped, and the incidental takeovers are exactly the ones you asked for). It does NOT prove "
    "the game will behave as intended: it never advances a round. Use `planet_x --apply … --round "
    "K` / `--control-plan` for consequences."
)


# --------------------------------------------------------------------------------------
# engine discovery / invocation
# --------------------------------------------------------------------------------------

def _repo_root() -> Path:
    """``<worktree>/play/planet_x_ctl/planet_x_ctl/__init__.py`` → ``<worktree>``."""
    return Path(__file__).resolve().parents[3]


def resolve_engine(planet_x: str | os.PathLike | None = None) -> str:
    """Locate the ``planet_x`` binary.

    Order: explicit ``planet_x=`` argument → ``$PLANET_X_BIN`` → ``<this worktree>/target/debug/
    planet_x[.exe]`` → ``planet_x`` on ``PATH``. No absolute path is baked into the package.
    """
    if planet_x:
        return str(planet_x)
    env = os.environ.get("PLANET_X_BIN")
    if env:
        return env
    debug = _repo_root() / "target" / "debug"
    for name in ("planet_x.exe", "planet_x"):
        cand = debug / name
        if cand.exists():
            return str(cand)
    return "planet_x"


class EngineError(RuntimeError):
    """The engine exited non-zero (e.g. exit code 10 = the diff was invalid)."""


_WORKDIR: dict[str, str | None] = {}


def _engine_cwd(exe: str, hint: str | os.PathLike | None = None) -> str | None:
    """A directory from which ``planet_x`` can find ``config/game.ron``.

    The engine reads its rules from ``config/game.ron`` relative to the **current directory** (or
    ``$PLANET_X_CONFIG``). Rather than making every caller ``cd`` into the worktree, this walks up
    from the **binary itself** (a checkpoint may well live in another worktree, whose config would be
    the wrong one for this binary), then from the checkpoint, then from this kit. Honours
    ``$PLANET_X_CONFIG``, which needs no cwd at all.
    """
    if os.environ.get("PLANET_X_CONFIG"):
        return None
    if exe in _WORKDIR:
        return _WORKDIR[exe]
    starts: list[Path] = []
    resolved = shutil.which(exe) or exe
    if os.path.exists(resolved):
        starts.append(Path(resolved).resolve().parent)
    if hint and os.path.exists(str(hint)):
        starts.append(Path(hint).resolve().parent)
    starts.append(_repo_root())
    found: str | None = None
    for start in starts:
        for base in [start, *start.parents]:
            if (base / "config" / "game.ron").exists():
                found = str(base)
                break
        if found:
            break
    _WORKDIR[exe] = found
    return found


def _run(args: Sequence[str], *, planet_x=None, timeout: float = 600.0,
         cwd: str | os.PathLike | None = None) -> subprocess.CompletedProcess:
    exe = resolve_engine(planet_x)
    workdir = str(cwd) if cwd is not None else _engine_cwd(
        exe, args[args.index("--start") + 1] if "--start" in args else None)
    try:
        return subprocess.run(
            [exe, *args], capture_output=True, encoding="utf-8", errors="replace",
            timeout=timeout, cwd=workdir,
        )
    except FileNotFoundError as exc:  # pragma: no cover - environment problem
        raise EngineError(
            f"找不到引擎可执行文件 {exe!r}；构造 kit 对象时传 planet_x=… 或设 $PLANET_X_BIN。"
        ) from exc


def _run_json(args: Sequence[str], *, planet_x=None, **kw) -> Any:
    proc = _run(args, planet_x=planet_x, **kw)
    if proc.returncode != 0:
        raise EngineError(f"planet_x {' '.join(args)} 退出码 {proc.returncode}：{proc.stderr.strip()}")
    return json.loads(proc.stdout)


# --------------------------------------------------------------------------------------
# planet_xq (the read-side kit) — reused, never re-implemented
# --------------------------------------------------------------------------------------

_PLANET_XQ = None


def _planet_xq():
    """Import the sibling read-side kit ``planet_xq`` (installed, or by sibling path).

    The projection parsing/join logic lives there and is deliberately **not** duplicated here.
    """
    global _PLANET_XQ
    if _PLANET_XQ is not None:
        return _PLANET_XQ
    try:
        import planet_xq  # type: ignore
        _PLANET_XQ = planet_xq
        return _PLANET_XQ
    except ImportError:
        pass
    sibling = Path(__file__).resolve().parents[2] / "planet_xq" / "planet_xq" / "__init__.py"
    if sibling.exists():
        spec = importlib.util.spec_from_file_location("planet_xq", sibling)
        assert spec and spec.loader
        mod = importlib.util.module_from_spec(spec)
        sys.modules["planet_xq"] = mod
        spec.loader.exec_module(mod)
        _PLANET_XQ = mod
        return _PLANET_XQ
    raise ImportError(
        "读侧 kit planet_xq 不可用。请 `cd play/planet_xq && uv sync`（或在 sys.path 上提供 "
        "planet_xq），本套件刻意不复制投影解析逻辑。"
    )


_INDEX_CACHE: dict[tuple, str] = {}


def _projection_dir(ckpt: str | os.PathLike, *, planet_x=None, index_dir=None) -> str:
    """A projection directory describing **this checkpoint's current state**.

    Built with ``planet_x --start CKPT --round 0 --index DIR`` (pure read: it projects round 0 =
    the checkpoint itself and never advances the RNG). Cached per (path, mtime, size).
    """
    if index_dir is not None:
        return str(index_dir)
    p = Path(ckpt).resolve()
    st = p.stat()
    key = (str(p), st.st_mtime_ns, st.st_size, str(planet_x))
    hit = _INDEX_CACHE.get(key)
    if hit and os.path.exists(os.path.join(hit, "main.jsonl")):
        return hit
    tag = hashlib.sha1(f"{key[0]}|{key[1]}|{key[2]}".encode("utf-8")).hexdigest()[:16]
    out = Path(tempfile.gettempdir()) / "planet_x_ctl" / tag
    out.mkdir(parents=True, exist_ok=True)
    proc = _run(["--start", str(p), "--round", "0", "--index", str(out)], planet_x=planet_x)
    if proc.returncode != 0:
        raise EngineError(f"--index 失败（退出码 {proc.returncode}）：{proc.stderr.strip()}")
    _INDEX_CACHE[key] = str(out)
    return str(out)


def projection(source: str | os.PathLike, *, planet_x=None, index_dir=None):
    """A :class:`planet_xq.PlanetXQ` projection.

    ``source`` is either a **checkpoint** (projected on the fly with ``--start``/``--round 0``) or an
    already-written ``--index`` **directory** (pass that whenever you need the flow metrics — see the
    README note on ``production_value`` / ``upkeep``).
    """
    if index_dir is not None:
        d = str(index_dir)
    elif os.path.isdir(source) and os.path.exists(os.path.join(str(source), "main.jsonl")):
        d = str(source)
    else:
        d = _projection_dir(source, planet_x=planet_x)
    return _planet_xq().load(d)


def _last_round(q) -> int:
    return int(q.facts["round"].max())


# --------------------------------------------------------------------------------------
# small value helpers
# --------------------------------------------------------------------------------------

_BEHAVIOR_KEYS = {"Move": "position", "Follow": "ship", "DockCity": "city", "Dock": "body",
                  "Colonize": "body"}


def _check_mode(mode: str) -> str:
    if mode not in MODES:
        raise ValueError(f"mode 必须是 {MODES} 之一，收到 {mode!r}")
    return mode


def _clamp(x: float, lo: float = -1.0, hi: float = 1.0) -> float:
    return float(min(hi, max(lo, float(x))))


def _check_bool(v: Any, what: str) -> bool:
    """`角色` 轴是个开关：只收真正的 ``bool``。

    ``True``/``False`` 在 Python 里是 ``int`` 的子类，所以 ``1``/``0`` 会**悄悄**通过 ``bool()``
    转换——但这条轴的读面写的是 ``true``/``false``，而 ``1`` 这种写法在补丁里几乎总是"我抄了
    别的轴的例子"。宁可在这里响亮地拒绝，也不要在引擎里变成一个看不懂的类型错。
    """
    if not isinstance(v, bool):
        raise ValueError(f"{what} 只接受 True/False（收到 {v!r}）——角色轴是个开关，不是数值轴")
    return v


def normalize_behavior(behavior: Any) -> Any:
    """Normalize a ship behavior to the engine's wire form.

    Accepts the raw wire form (``"Idle"`` / ``{"Dock": {"body": "地球"}}``) plus three shorthands
    this kit adds so recipes stay readable::

        "Idle"
        "Dock:地球"          "DockCity:长三角"     "Follow:星环"
        "Colonize:火星"      "Move:1.5,-2.0"

    Anything malformed raises ``ValueError`` *here*, at recipe time — never as a rejected diff later.
    """
    if isinstance(behavior, str):
        text = behavior.strip()
        if text == "Idle":
            return "Idle"
        if ":" in text:
            verb, arg = text.split(":", 1)
            verb, arg = verb.strip(), arg.strip()
            if verb == "Move":
                parts = [p for p in re.split(r"[,\s]+", arg) if p]
                if len(parts) != 2:
                    raise ValueError(f"Move 需要两个坐标：{behavior!r}")
                return {"Move": {"position": [float(parts[0]), float(parts[1])]}}
            if verb in _BEHAVIOR_KEYS:
                return {verb: {_BEHAVIOR_KEYS[verb]: arg}}
            raise ValueError(f"未知行为 {verb!r}（可用：Idle/Move/Follow/DockCity/Dock/Colonize）")
        raise ValueError(
            f"行为字符串 {behavior!r} 无法解析：要么是 'Idle'，要么写成 'Dock:地球' 这种 "
            "'动词:参数' 形状（Move 用 'Move:x,y'）。"
        )
    if isinstance(behavior, Mapping):
        if len(behavior) != 1:
            raise ValueError(f"行为对象必须只有一个键（如 {{'Dock':{{'body':'地球'}}}}）：{behavior!r}")
        verb, arg = next(iter(behavior.items()))
        if verb == "Idle":
            return "Idle"
        if verb not in _BEHAVIOR_KEYS:
            raise ValueError(f"未知行为 {verb!r}（可用：Idle/Move/Follow/DockCity/Dock/Colonize）")
        if not isinstance(arg, Mapping) or _BEHAVIOR_KEYS[verb] not in arg:
            raise ValueError(
                f"行为 {verb!r} 需要 {{'{_BEHAVIOR_KEYS[verb]}': …}}，收到 {arg!r}"
            )
        return {verb: dict(arg)}
    raise ValueError(f"不支持的行为表示：{behavior!r}")


def behavior_str(behavior: Any) -> str | None:
    """A compact, stable one-line rendering of a behavior cell (for DataFrame columns / receipts)."""
    if behavior is None:
        return None
    if behavior == "Idle":
        return "Idle"
    if isinstance(behavior, Mapping) and len(behavior) == 1:
        verb, arg = next(iter(behavior.items()))
        if isinstance(arg, Mapping):
            inner = arg.get(_BEHAVIOR_KEYS.get(verb, ""), "")
            if isinstance(inner, (list, tuple)):
                return f"{verb}:[{', '.join(str(v) for v in inner)}]"
            return f"{verb}:{inner}"
        return f"{verb}:{arg}"
    return json.dumps(behavior, ensure_ascii=False, sort_keys=False)


def _entry_key(kind: str, entry: Mapping) -> tuple:
    keys = []
    for f in LEAF_KINDS[kind]:
        v = entry.get(f)
        keys.append(int(v) if f == "building" and v is not None else v)
    return tuple(keys)


def _key_label(key: tuple) -> str:
    return "/".join("" if k is None else str(k) for k in key)


def _leaf_name(faction: str, kind: str, key: tuple) -> str:
    return f"{faction}.{kind}" if not key else f"{faction}.{kind}[{_key_label(key)}]"


# --------------------------------------------------------------------------------------
# Leaf
# --------------------------------------------------------------------------------------

@dataclass(frozen=True)
class Leaf:
    """One controllable leaf as the engine's **read face** shows it: a value plus its mode."""

    faction: str
    kind: str
    key: tuple = ()
    exists: bool = False
    mode: str = INHERIT
    value: Any = None
    raw: dict | None = None
    scope_origin: str | None = None

    @property
    def name(self) -> str:
        return _leaf_name(self.faction, self.kind, self.key)

    @property
    def is_player(self) -> bool:
        return self.mode == PLAYER

    @property
    def behavior(self) -> str | None:
        """``value`` rendered as a behavior string (only meaningful for ``ship_orders``)."""
        return behavior_str(self.value)

    def as_dict(self) -> dict:
        out = {"leaf": self.name, "faction": self.faction, "kind": self.kind,
               "exists": self.exists, "mode": self.mode, "value": self.value}
        if self.key:
            out["key"] = list(self.key)
        return out

    def __repr__(self) -> str:  # pragma: no cover - display only
        return f"Leaf({self.name!r}, mode={self.mode!r}, value={self.value!r}, exists={self.exists})"


# --------------------------------------------------------------------------------------
# Surface
# --------------------------------------------------------------------------------------

class Surface:
    """The control face of one checkpoint (``--control``) plus pending, un-emitted edits.

    The read face *is* the write face: ``--control`` prints exactly the leaf shape ``--apply``
    accepts, so "dump → edit → send back" is always legal and never silently clears a layer (the
    scope tree only lists nodes that have an opinion).
    """

    def __init__(self, raw: Mapping, ckpt: str | os.PathLike | None = None, *,
                 planet_x=None, index_dir=None, source: str = "--control"):
        self.raw: dict = copy.deepcopy(dict(raw))
        self.ckpt = str(ckpt) if ckpt is not None else None
        self.planet_x = planet_x
        self.index_dir = str(index_dir) if index_dir is not None else None
        self.source = source
        self.scope: dict = dict(self.raw.get("scope") or {})
        self._factions: list[dict] = list(self.raw.get("control") or [])
        self._index: dict[tuple, dict] = {}
        self._rank: dict[tuple, int] = {}
        for fac in self._factions:
            fid = fac.get("faction_id")
            for kind in _KIND_ORDER:
                if not LEAF_KINDS[kind]:
                    continue
                for i, entry in enumerate(fac.get(kind) or []):
                    self._index[(fid, kind, _entry_key(kind, entry))] = entry
                    self._rank[(fid, kind, _entry_key(kind, entry))] = i
        # pending edits: faction -> kind -> key -> partial patch entry
        self._pending: dict[str, dict[str, dict[tuple, dict]]] = {}
        # pending **structural** edits: faction -> [buildings[] 补丁]（设计图指针走这条路：
        # 它不是控制叶，而是建造区上的结构字段，写面形状见 `set_blueprint_pointer`）。
        self._pending_buildings: dict[str, list[dict]] = {}
        self._scope_pending: dict = {}
        self._projection_cache = None
        self._building_index_cache: pd.DataFrame | None = None

    # -- read face ---------------------------------------------------------------------

    @property
    def factions(self) -> list[str]:
        """Faction names, in the order the engine listed them (canonical diff order)."""
        return [f.get("faction_id") for f in self._factions]

    def raw_faction(self, faction: str) -> dict | None:
        """The untouched read-face entry for one faction (as ``--control`` printed it)."""
        for f in self._factions:
            if f.get("faction_id") == faction:
                return f
        return None

    def has_faction(self, faction: str) -> bool:
        return self.raw_faction(faction) is not None

    def _require_faction(self, faction: str) -> None:
        if not self.has_faction(faction):
            known = ", ".join(repr(x) for x in self.factions)
            raise ValueError(
                f"控制面里没有势力 {faction!r}（名字即唯一键，别手抄）。现有：{known}"
            )

    def leaf(self, faction: str, kind: str, key: Any = None) -> Leaf:
        """One leaf: its **value and its mode** (三态).

        ``key`` is the leaf's identity — the ship name for ``ship_orders``/``ship_doctrine``/
        ``ship_kiting``, the resource key for the budgets, the city name for ``loyalty_budget``, a
        ``(city, building)`` tuple for the weight kinds, and ``None`` for the per-faction singletons
        ``default_ship_order`` / ``capital``.

        A leaf the read face did not list is **not an error**: it means "nobody has spoken here"
        (``mode == "Inherit"``), which is the same as an explicitly-written ``Inherit``.
        """
        if kind not in LEAF_KINDS:
            raise ValueError(f"未知叶种类 {kind!r}（可用：{list(LEAF_KINDS)}）")
        self._require_faction(faction)
        kt = _normalize_key(kind, key)
        if LEAF_KINDS[kind]:
            entry = self._index.get((faction, kind, kt))
            if entry is None:
                return Leaf(faction, kind, kt, exists=False)
            return Leaf(faction, kind, kt, exists=True,
                        mode=entry.get("mode", INHERIT), value=_leaf_value(kind, entry), raw=entry)
        entry = self.raw_faction(faction).get(kind)
        if entry is None:
            return Leaf(faction, kind, (), exists=False)
        return Leaf(faction, kind, (), exists=True, mode=entry.get("mode", INHERIT),
                    value=_leaf_value(kind, entry), raw=entry)

    def faction(self, faction: str) -> dict:
        """One faction's leaves, keyed by kind — ``{kind: {key: Leaf}}`` (singletons → ``Leaf``)."""
        self._require_faction(faction)
        out: dict[str, Any] = {"faction_id": faction}
        for kind in _KIND_ORDER:
            if not LEAF_KINDS[kind]:
                out[kind] = self.leaf(faction, kind)
                continue
            out[kind] = {
                k[0] if len(k) == 1 else k: self.leaf(faction, kind, k)
                for (fac, kd, k) in self._index
                if fac == faction and kd == kind
            }
        return out

    def scope_of(self, kind: str, name: str | None = None) -> str:
        """The scope tree's opinion at one node, or ``Inherit`` when the node says nothing.

        ``kind`` ∈ ``global`` / ``factions`` / ``bodies`` / ``cities``.
        """
        if kind == "global":
            return self.scope.get("global") or INHERIT
        for entry in self.scope.get(kind) or []:
            if entry[0] == name:
                return entry[1]
        return INHERIT

    def leaves(self, kind: str | None = None) -> list[Leaf]:
        """Every leaf the read face lists (optionally filtered to one kind)."""
        kinds = [kind] if kind else [k for k in _KIND_ORDER if LEAF_KINDS[k]]
        return [self.leaf(fac, kd, key) for (fac, kd, key) in list(self._index) if kd in kinds]

    # -- lazy auxiliary indices (same checkpoint!) -------------------------------------

    def projection(self):
        """The projection of **this same checkpoint** (used to resolve names/indices)."""
        if self._projection_cache is None:
            if self.ckpt is None:
                raise ValueError("这个 Surface 没有 ckpt（无法解析 (city, building) 这类下标）")
            self._projection_cache = projection(self.ckpt, planet_x=self.planet_x,
                                                index_dir=self.index_dir)
        return self._projection_cache

    def building_index(self) -> pd.DataFrame:
        """``(city, building)`` → building attributes, resolved against this checkpoint.

        ``building`` is a **per-city ``u32`` index**; only the same-round projection can resolve it.
        """
        if self._building_index_cache is None:
            q = self.projection()
            r = _last_round(q)
            rows = []
            for _, c in q.cities(round=r).iterrows():
                for b in (c.get("buildings") or []):
                    rows.append({
                        "city": c["city_id"], "building": int(b["id"]),
                        "faction_id": c["faction_id"], "kind": b.get("kind"),
                        "resource": b.get("resource"), "ship_type": b.get("ship_type"),
                        "structure": b.get("structure"), "area": b.get("area"),
                        "deployed": b.get("deployed"), "armor": b.get("armor"),
                    })
            self._building_index_cache = pd.DataFrame(
                rows, columns=["city", "building", "faction_id", "kind", "resource",
                               "ship_type", "structure", "area", "deployed", "armor"])
        return self._building_index_cache

    def resolve_building(self, city: str, selector: Any) -> int:
        """Resolve a building selector to its per-city index, **against this checkpoint**.

        ``selector`` may be an ``int`` index, or a mapping of attributes to match (e.g.
        ``{"kind": "construction", "ship_type": "corvette"}``). An index that does not exist in that
        city — or an ambiguous attribute selector — raises instead of emitting a diff the engine
        would reject with ``WARN_APPLY_SKIPPED: no_such_building``.
        """
        idx = self.building_index()
        here = idx[idx["city"] == city]
        if here.empty:
            raise ValueError(f"控制面所属的 checkpoint 里没有城 {city!r}（名字即唯一键，别手抄）")
        present = sorted(int(x) for x in here["building"])
        is_selector_map = isinstance(selector, Mapping)
        if not is_selector_map:
            if isinstance(selector, bool) or isinstance(selector, (list, tuple, set)):
                raise ValueError(
                    f"building 选择器必须是下标整数、属性映射或 'kind[:ship_type|resource]' 字符串，"
                    f"收到 {selector!r}"
                )
            if isinstance(selector, str) and not selector.lstrip("-").isdigit():
                parts = selector.split(":", 1)
                sel = {"kind": parts[0].strip()}
                if len(parts) == 2 and parts[1].strip():
                    tail = parts[1].strip()
                    if tail in set(here["ship_type"].dropna()):
                        sel["ship_type"] = tail
                    else:
                        sel["resource"] = tail
                return self.resolve_building(city, sel)
            try:
                want = int(selector)
            except (TypeError, ValueError) as exc:
                raise ValueError(f"building 选择器 {selector!r} 既不是整数下标也不是属性映射") from exc
            if want not in set(present):
                raise ValueError(
                    f"「{city}」里没有 building={want}；它的建筑下标是 {present}"
                    "（下标只在城内部唯一，换城要换下标，而且只在同回合自洽）。"
                )
            return want
        mask = pd.Series(True, index=here.index)
        for attr, want in selector.items():
            col = "building" if attr == "id" else attr
            if col not in here.columns:
                raise ValueError(f"建筑没有属性 {attr!r}（可用：{list(here.columns)}）")
            mask &= here[col].astype(object) == want
        hit = here[mask]
        if len(hit) == 0:
            raise ValueError(f"「{city}」里没有匹配 {dict(selector)} 的建筑（候选："
                             f"{here[['building', 'kind', 'resource', 'ship_type']].to_dict('records')}）")
        if len(hit) > 1:
            raise ValueError(f"「{city}」里 {dict(selector)} 匹配到 {len(hit)} 座建筑，选择器必须唯一："
                             f"{sorted(int(x) for x in hit['building'])}")
        return int(hit.iloc[0]["building"])

    def _ship_faction(self, ship: str) -> str:
        """Which faction owns a ship — from the surface's own leaves, else the same checkpoint."""
        for (fac, kind, key) in self._index:
            if kind in ("ship_orders", "ship_doctrine", "ship_kiting") and key and key[0] == ship:
                return fac
        if self.ckpt is not None:
            try:
                q = self.projection()
                df = q.ships(round=_last_round(q))
            except Exception:  # pragma: no cover - projection unavailable
                df = None
            if df is not None and len(df):
                hit = df[df["ship_id"] == ship]
                if len(hit):
                    return str(hit.iloc[0]["faction_id"])
        raise ValueError(
            f"控制面里没有名为「{ship}」的舰。舰名会换代（「方舟」战沉后重建的是「方舟2」「方舟3」），"
            "别手抄——用 ctl.ships(ckpt) / ctl.roster(ckpt, spec) 拿现名。"
        )

    def _ship_pairs(self, selection: Any) -> list[tuple[str, str]]:
        """Normalize a selection to ``[(faction, ship), …]`` (deduped, deterministic order)."""
        pairs: list[tuple[str, str]] = []
        if isinstance(selection, pd.DataFrame):
            if selection.empty:
                return []
            col = next((c for c in ("ship_id", "name", "ship") if c in selection.columns), None)
            if col is None:
                raise ValueError(
                    "传入的 DataFrame 没有舰名列（ship_id / name / ship）——用 ctl.ships(ckpt) "
                    "或 ctl.ships_and_cities(ckpt) 筛出来的帧。"
                )
            has_fac = "faction_id" in selection.columns
            for _, row in selection.iterrows():
                ship = row[col]
                if not isinstance(ship, str) or not ship:
                    continue
                fac = str(row["faction_id"]) if has_fac else None
                pairs.append((fac or self._ship_faction(ship), ship))
        elif isinstance(selection, str):
            pairs = [(self._ship_faction(selection), selection)]
        else:
            for ship in selection:
                if not isinstance(ship, str) or not ship:
                    continue
                pairs.append((self._ship_faction(ship), ship))
        seen, out = set(), []
        for fac, ship in pairs:
            if (fac, ship) in seen:
                continue
            seen.add((fac, ship))
            if not self.has_faction(fac):
                raise ValueError(f"舰「{ship}」解析到势力 {fac!r}，但控制面里没有这个势力")
            out.append((fac, ship))
        return out

    # -- pending-edit bookkeeping ------------------------------------------------------

    def _add(self, faction: str, kind: str, key: tuple, patch: Mapping) -> None:
        self._require_faction(faction)
        if kind not in LEAF_KINDS:
            raise ValueError(f"未知叶种类 {kind!r}")
        bucket = self._pending.setdefault(faction, {}).setdefault(kind, {})
        bucket.setdefault(key, {})
        bucket[key].update(dict(patch))

    def _mode_or_takeover(self, mode: str | None, take_over: bool, what: str) -> str | None:
        """Enforce "写值即接管": never take over a leaf without saying so.

        * ``mode`` given → written explicitly (the engine reports **no** takeover note);
        * ``mode=None`` + ``take_over=True`` → the ``mode`` key is omitted, so the engine implies
          ``Player`` and says so in the receipt (``NOTE_APPLY_TOOKOVER``) — this is the *only* way
          this kit will let a value write take ownership of a leaf.
        """
        if mode is not None:
            _check_mode(mode)
            if take_over:
                raise ValueError("要么给 mode=…，要么给 take_over=True —— 两者不能同时给")
            return mode
        if not take_over:
            raise ValueError(
                f"{what}：写值而不写 mode 等于**接管**这片叶（引擎的 `写值即接管`，会回报 "
                "NOTE_APPLY_TOOKOVER）。请显式传 mode='Player'（明说归玩家），或传 "
                "mode='Auto'/'Inherit'（把决定权让出去），或——如果你确实要接管——传 take_over=True。"
            )
        return None

    def _require_both_axes(self, faction: str, kind: str, key: Any,
                           temper: float | None, lone_wolf: float | None, what: str) -> None:
        """Refuse to create a **two-axis** leaf from a single-axis patch.

        The engine speaks two axes in one leaf (``temper`` + ``lone_wolf``) and, when the leaf does
        **not exist yet**, creating it from a single axis used to fill the axis you did *not* mention
        with ``0.0`` — a silent fleet-wide change (and 0.0 is a meaningful, "textbook" temperament, so
        nothing looks wrong afterwards). The engine now **rejects** that patch itself
        (``partial_doctrine_leaf``, see ``notes/control-live-layers.md`` §3.1); this check stays
        because a recipe should fail while you are still writing it, not at apply time.
        """
        if temper is not None and lone_wolf is not None:
            return
        if self.leaf(faction, kind, key).exists:
            return
        raise ValueError(
            f"{what}：`{faction}.{kind}` 这片叶**还不存在**，而这片叶装的是**两条轴**——"
            f"只写一条会把另一条静默定成 0.0（不是「保留出厂值」），0.0 又是个正常取值，"
            f"事后看不出问题，所以这里直接拒绝（引擎也会拒绝，码 `partial_doctrine_leaf`）。"
            f"三条路：① 两条轴一起给（temper=…, lone_wolf=…）；② 先只写 mode=…（先表态归属）；"
            f"③ {kind}=… 这片叶本来就不该存在 ⇒ 用 clear_* / remove 删掉它。"
        )

    # -- mutations: ship ownership & behavior ------------------------------------------

    def set_mode(self, selection: Any, mode: str) -> "Surface":
        """**Wildcard, client-side**: set the *ownership* of many ships in one call.

        ``selection`` is a DataFrame from :func:`ships` / :func:`ships_and_cities`, or a list of
        ship names. Emits one ``{"ship": …, "mode": …}`` leaf per ship — a leaf that writes **only**
        the mode, so the recorded value is left untouched and **no takeover can ever happen**.

        * ``"Auto"``   → hand the fleet back to the system (it rewrites the leaves each round);
        * ``"Player"`` → claim it (the system stops overwriting);
        * ``"Inherit"``→ this layer withdraws its opinion (it then falls through to the faction's
          ``default_ship_order`` / scope — see the README pitfall §1.2).
        """
        mode = _check_mode(mode)
        for faction, ship in self._ship_pairs(selection):
            self._add(faction, "ship_orders", (ship,), {"ship": ship, "mode": mode})
        return self

    def set_behavior(self, selection: Any, behavior: Any,
                     *, mode: str | None = None, take_over: bool = False) -> "Surface":
        """Give many ships an instruction (a *value* write — hence the takeover guard)."""
        value = normalize_behavior(behavior)
        m = self._mode_or_takeover(mode, take_over, f"set_behavior({behavior_str(value)!r})")
        for faction, ship in self._ship_pairs(selection):
            patch = {"ship": ship, "behavior": value}
            if m is not None:
                patch["mode"] = m
            self._add(faction, "ship_orders", (ship,), patch)
        return self

    def set_default_ship_order(self, faction: str, behavior: Any = None,
                               *, mode: str | None = None, take_over: bool = False) -> "Surface":
        """The faction-level fleet default — **one leaf** that new ships inherit.

        This is what makes "意图" survive name generations, and the answer to "一次性指令收尾有去处":
        a leaf saying ``Inherit`` (or a brand-new ship with no leaf at all) resolves to this value.
        At least one of ``behavior`` / ``mode`` must be given.
        """
        if behavior is None and mode is None:
            raise ValueError("set_default_ship_order 至少要给 behavior= 或 mode= 之一")
        patch: dict = {}
        if behavior is not None:
            patch["behavior"] = normalize_behavior(behavior)
        if mode is not None:
            patch["mode"] = _check_mode(mode)
        else:
            m = self._mode_or_takeover(None, take_over, f"set_default_ship_order({faction!r})")
            if m is not None:  # pragma: no cover - _mode_or_takeover returns None here
                patch["mode"] = m
        self._add(faction, "default_ship_order", (), patch)
        return self

    def set_default_kiting(self, faction: str, kiting: float | None = None, *,
                           mode: str | None = None, take_over: bool = False) -> "Surface":
        """势力级默认风筝姿态——**一片叶**管住全舰队里「没有自己表态」的舰（含新下水的）。

        与 :meth:`set_default_ship_order` 同形（同一层、同样的三态）。只写值必须明说归属：
        要么 ``mode=``，要么 ``take_over=True``（引擎会回 ``NOTE_APPLY_TOOKOVER``）；
        只写 ``mode`` 是合法的（值不动）——那正是"整支舰队交还/收回"的用法，一片叶顶 N 片。
        """
        if kiting is None and mode is None:
            raise ValueError("set_default_kiting 至少要给 kiting= 或 mode= 之一")
        patch: dict = {}
        if kiting is not None:
            patch["kiting"] = _clamp(kiting)
        if mode is not None:
            patch["mode"] = _check_mode(mode)
        else:
            m = self._mode_or_takeover(None, take_over, f"set_default_kiting({faction!r})")
            if m is not None:  # pragma: no cover - _mode_or_takeover returns None here
                patch["mode"] = m
        self._add(faction, "default_kiting", (), patch)
        return self

    def set_default_freighter(self, faction: str, freighter: bool | None = None, *,
                              mode: str | None = None, take_over: bool = False) -> "Surface":
        """势力级默认**角色**（``True`` = 全舰队转运输）——**一片叶**管住「没有自己表态」的舰。

        与 :meth:`set_default_kiting` 同形，但它有一条别的轴没有的用法：把它设成
        ``mode='Player'`` 就是「**自动控制的逐舰定编别再碰我的舰队**」那道闸门。自动控制每回合
        按积压改写逐舰角色叶，但它只写 ``Inherit`` 的叶，而舰队默认归玩家时玩家的值压过那些
        结论（``State::ship_freighter``）——所以"整支舰队交给我定"是**一片叶**的事。
        """
        if freighter is None and mode is None:
            raise ValueError("set_default_freighter 至少要给 freighter= 或 mode= 之一")
        patch: dict = {}
        if freighter is not None:
            patch["freighter"] = _check_bool(freighter, f"set_default_freighter({faction!r})")
        if mode is not None:
            patch["mode"] = _check_mode(mode)
        else:
            m = self._mode_or_takeover(None, take_over, f"set_default_freighter({faction!r})")
            if m is not None:  # pragma: no cover - _mode_or_takeover returns None here
                patch["mode"] = m
        self._add(faction, "default_freighter", (), patch)
        return self

    def set_default_doctrine(self, faction: str, temper: float | None = None,
                             lone_wolf: float | None = None, *,
                             mode: str | None = None, take_over: bool = False) -> "Surface":
        """势力级默认**行为风格**（``temper`` / ``lone_wolf``，各自 clamp 到 ``[-1, 1]``）。

        与 :meth:`set_default_kiting` 同形。想「让全势力的舰都更保守」——改这一片叶就够了，
        不必逐舰点名（叶沉默的舰，包括还没造出来的，都跟着变）。
        """
        if temper is None and lone_wolf is None and mode is None:
            raise ValueError("set_default_doctrine 至少要给 temper= / lone_wolf= / mode= 之一")
        patch: dict = {}
        if temper is not None:
            patch["temper"] = _clamp(temper)
        if lone_wolf is not None:
            patch["lone_wolf"] = _clamp(lone_wolf)
        if patch:
            self._require_both_axes(faction, "default_doctrine", None, temper, lone_wolf,
                                    f"set_default_doctrine({faction!r})")
        if mode is not None:
            patch["mode"] = _check_mode(mode)
        elif patch:
            m = self._mode_or_takeover(None, take_over, f"set_default_doctrine({faction!r})")
            if m is not None:  # pragma: no cover - _mode_or_takeover returns None here
                patch["mode"] = m
        self._add(faction, "default_doctrine", (), patch)
        return self

    def set_kiting(self, selection: Any, kiting: float, *,
                   mode: str | None = None, take_over: bool = False) -> "Surface":
        """**逐舰**风筝↔贴脸姿态（clamp 到 ``[-1, 1]``）。

        引擎侧它现在是**活层**（三态叶 + 势力级默认，见 ``default_kiting``）：写值必须明说归属
        （``mode=`` 或 ``take_over=True``），否则你是在**静默接管**这艘舰的姿态——那正是
        这个套件存在的意义所在。只想让"叶沉默的舰"跟着变，就用
        :meth:`set_default_kiting`（一片叶，新舰自动继承）。
        """
        k = _clamp(kiting)
        for faction, ship in self._ship_pairs(selection):
            patch = {"ship": ship, "kiting": k}
            if mode is not None:
                patch["mode"] = _check_mode(mode)
            else:
                m = self._mode_or_takeover(None, take_over, f"set_kiting({ship!r})")
                if m is not None:  # pragma: no cover
                    patch["mode"] = m
            self._add(faction, "ship_kiting", (ship,), patch)
        return self

    def set_doctrine(self, selection: Any, temper: float | None = None,
                     lone_wolf: float | None = None, *,
                     mode: str | None = None, take_over: bool = False) -> "Surface":
        """**逐舰**行为风格（``temper`` / ``lone_wolf``，各自 clamp 到 ``[-1, 1]``）。

        与 :meth:`set_kiting` 同一套归属纪律（引擎侧已是活层）；想改全舰队就用
        :meth:`set_default_doctrine`。
        """
        if temper is None and lone_wolf is None:
            raise ValueError("set_doctrine 至少要给 temper= 或 lone_wolf= 之一")
        for faction, ship in self._ship_pairs(selection):
            patch = {"ship": ship}
            if temper is not None:
                patch["temper"] = _clamp(temper)
            if lone_wolf is not None:
                patch["lone_wolf"] = _clamp(lone_wolf)
            self._require_both_axes(faction, "ship_doctrine", (ship,), temper, lone_wolf,
                                    f"set_doctrine({ship!r})")
            if mode is not None:
                patch["mode"] = _check_mode(mode)
            else:
                m = self._mode_or_takeover(None, take_over, f"set_doctrine({ship!r})")
                if m is not None:  # pragma: no cover
                    patch["mode"] = m
            self._add(faction, "ship_doctrine", (ship,), patch)
        return self

    # -- mutations: budgets / weights / capital ----------------------------------------

    def set_freighter(self, selection: Any, freighter: bool, *,
                      mode: str | None = None, take_over: bool = False) -> "Surface":
        """**逐舰角色**：``True`` = 运输舰（自动控制给它排集货路线），``False`` = 战舰（找仗打）。

        写值必须明说归属（``mode=`` 或 ``take_over=True``），与另两条风格轴同一条纪律。
        ⚠ 这条轴上"不表态"的代价比别处大：**这片叶自动控制本来每回合也会写**，所以含糊的写法
        会被定编覆盖，而现象是「我明明设了它却没用」。

        角色只决定自动控制**派哪种活**，不解除武装：运输舰在射程内照样自动开火、照样按
        ``kiting`` 姿态软移动。它是"同一个舰长的两种活"，不是"军舰/民船"的军备差别。
        """
        f = _check_bool(freighter, f"set_freighter({selection!r})")
        for faction, ship in self._ship_pairs(selection):
            patch = {"ship": ship, "freighter": f}
            if mode is not None:
                patch["mode"] = _check_mode(mode)
            else:
                m = self._mode_or_takeover(None, take_over, f"set_freighter({ship!r})")
                if m is not None:  # pragma: no cover
                    patch["mode"] = m
            self._add(faction, "ship_freighter", (ship,), patch)
        return self

    def set_budget(self, faction: str, kind: str, values: Mapping[str, float],
                   *, mode: str | None = None, take_over: bool = False,
                   strict: bool = True) -> "Surface":
        """Set budget amounts per resource.

        ``kind`` ∈ ``construction_budget`` (造舰) / ``investment_budget`` (建设) — same wire shape.
        ``values`` is ``{resource_raw_key: amount}`` (e.g. ``{"硅": 4.0}``).

        A budget value is a **one-shot number**, not a persistent intent: if you want "跟着产出走",
        the engine has to own it (``BudgetPatch.value`` accepts ``{"frac_of_production": 0.3}``) —
        Python cannot express that through this helper. See README §"边界".
        """
        if kind not in _BUDGET_KINDS:
            raise ValueError(f"kind 必须是 {_BUDGET_KINDS} 之一，收到 {kind!r}")
        self._require_faction(faction)
        m = self._mode_or_takeover(mode, take_over, f"set_budget({faction!r}, {kind!r})")
        known = self._known_resources(faction, kind)
        for resource, value in values.items():
            if strict and known and resource not in known:
                raise ValueError(
                    f"资源 key {resource!r} 不在已知表里（引擎会回 WARN_APPLY_SKIPPED: "
                    f"no_such_resource）。现有：{sorted(known)}；传 strict=False 可绕过。"
                )
            patch = {"resource": resource, "value": float(value)}
            if m is not None:
                patch["mode"] = m
            self._add(faction, kind, (resource,), patch)
        return self

    def set_loyalty_budget(self, faction: str, values: Mapping[str, float],
                           *, mode: str | None = None, take_over: bool = False,
                           strict: bool = True) -> "Surface":
        """娱乐/福利预算, keyed by **city name** (validate against this checkpoint's cities)."""
        self._require_faction(faction)
        m = self._mode_or_takeover(mode, take_over, f"set_loyalty_budget({faction!r})")
        known = self._known_cities(faction) if strict else None
        for city, value in values.items():
            if known is not None and city not in known:
                raise ValueError(
                    f"城 {city!r} 不属于 {faction!r}（引擎会回 WARN_APPLY_SKIPPED）。"
                    f"它的城是：{sorted(known)}"
                )
            patch = {"city": city, "value": float(value)}
            if m is not None:
                patch["mode"] = m
            self._add(faction, "loyalty_budget", (city,), patch)
        return self

    def set_weights(self, faction: str, kind: str, values: Any,
                    *, mode: str | None = None, take_over: bool = False) -> "Surface":
        """Investment / build weights, keyed by ``(city, building)``.

        The building half is a **per-city ``u32`` index**, resolved here against *the checkpoint this
        surface was read from* (the same-round transform). ``values`` is a ``{key: weight}`` mapping
        whose keys are ``(city, selector)``, or an iterable of ``(city, selector, weight)`` triples
        when the selector is unhashable. ``selector`` may be an index, an attribute mapping, or a
        ``"kind"`` / ``"kind:ship_type_or_resource"`` string — e.g.::

            s.set_invest_weights("中国", {("珠三角", "construction:destroyer"): 2.0}, mode="Player")
            s.set_build_weights("中国", [("珠三角", {"kind": "construction"}, 3.0)], mode="Player")
        """
        if kind not in _WEIGHT_KINDS:
            raise ValueError(f"kind 必须是 {_WEIGHT_KINDS} 之一，收到 {kind!r}")
        self._require_faction(faction)
        m = self._mode_or_takeover(mode, take_over, f"set_weights({faction!r}, {kind!r})")
        items = list(values.items()) if isinstance(values, Mapping) else list(values)
        for item in items:
            if len(item) != 2:
                raise ValueError(
                    f"{kind} 的键必须是 (城名, building)。building 是**城内下标**，只能对着同一个 "
                    "ckpt 的建筑表解析；用 ctl.buildings(ckpt) 看全表，或写成属性映射/字符串："
                    "('珠三角', {'kind':'construction','ship_type':'destroyer'}) / "
                    "('珠三角', 'construction:destroyer')。"
                )
            (city, selector), value = item
            idx = self.resolve_building(city, selector)
            patch = {"city": city, "building": idx, "value": float(value)}
            if m is not None:
                patch["mode"] = m
            self._add(faction, kind, (city, idx), patch)
        return self

    def set_invest_weights(self, faction: str, values: Any, **kw) -> "Surface":
        return self.set_weights(faction, "invest_weights", values, **kw)

    def set_build_weights(self, faction: str, values: Any, **kw) -> "Surface":
        return self.set_weights(faction, "build_weights", values, **kw)

    def set_capital(self, faction: str, body: str | None = None,
                    *, mode: str | None = None, take_over: bool = False) -> "Surface":
        """Move the capital (or just change who decides). ``body`` is a **body name**."""
        if body is None and mode is None:
            raise ValueError("set_capital 至少要给 body= 或 mode= 之一")
        self._require_faction(faction)
        patch: dict = {}
        if body is not None:
            known = self._known_bodies()
            if known and body not in known:
                raise ValueError(f"天体 {body!r} 不存在。现有：{sorted(known)}")
            patch["value"] = body
        if mode is not None:
            patch["mode"] = _check_mode(mode)
        else:
            m = self._mode_or_takeover(None, take_over, f"set_capital({faction!r})")
            if m is not None:  # pragma: no cover
                patch["mode"] = m
        self._add(faction, "capital", (), patch)
        return self

    # -- mutations: deleting leaves ("remove") ----------------------------------------

    def remove(self, faction: str, kind: str, key: Any = None) -> "Surface":
        """**删掉一片控制叶**（引擎的 `remove: true`）：这一层回到「没有说话」。

        与「把 `mode` 改回 `Inherit`」**不是**一回事：只要那片叶还在，取值就优先用**叶里的值**
        （``State::ship_doctrine`` 是 ``leaf.map(|l| l.value).unwrap_or(record)``，与 `mode` 无关）。
        所以想让一艘舰**真的回到出厂快照**（或让势力级默认不再供值），只能删叶。
        删一片本来就不存在的叶是幂等的（引擎既不报丢弃、也不进 `NOTE_APPLY_REMOVED`）。

        ``key`` 与读面同形：逐舰叶给舰名、预算叶给资源名、权重叶给 ``(城, 建筑下标)``、
        势力级单片叶不给（``None``）。名字用 :meth:`remove` 而不是 ``clear`` 是为了和引擎
        补丁里的字段同名——同一件事在两处叫两个名字正是这个仓库反复吃亏的地方。
        """
        self._require_faction(faction)
        key = _normalize_key(kind, key)
        patch: dict = {"remove": True}
        for f, v in zip(LEAF_KINDS[kind], key):
            patch[f] = v
        self._add(faction, kind, key, patch)
        return self

    def remove_doctrine(self, selection: Any) -> "Surface":
        """逐舰：删掉**风格叶** ⇒ 这些舰的风格回到「舰队默认 / 出厂快照」。"""
        for faction, ship in self._ship_pairs(selection):
            self._add(faction, "ship_doctrine", (ship,), {"ship": ship, "remove": True})
        return self

    def remove_kiting(self, selection: Any) -> "Surface":
        """逐舰：删掉**风筝姿态叶** ⇒ 回到「舰队默认 / 出厂快照」。"""
        for faction, ship in self._ship_pairs(selection):
            self._add(faction, "ship_kiting", (ship,), {"ship": ship, "remove": True})
        return self

    def remove_freighter(self, selection: Any) -> "Surface":
        """逐舰：删掉**角色叶** ⇒ **交回自动定编**（AI 下回合可能立刻又写下它的结论）。

        ⚠ 这与另两条风格轴上的"删叶"含义不同：那里删掉 = 回到出厂快照并**从此冻结**；
        这里删掉 = **放手**。想让某个角色稳定下来就写 ``mode='Player'``，而不是删叶。
        """
        for faction, ship in self._ship_pairs(selection):
            self._add(faction, "ship_freighter", (ship,), {"ship": ship, "remove": True})
        return self

    def remove_default_freighter(self, faction: str) -> "Surface":
        """势力级：删掉**默认角色叶** ⇒ 这一层不再供值，自动控制的逐舰定编重新说了算。"""
        self._add(faction, "default_freighter", (), {"remove": True})
        return self

    def remove_default_doctrine(self, faction: str) -> "Surface":
        """势力级：删掉**默认风格叶** ⇒ 这一层不再供值（回落到作用域链 / 舰上记录值）。"""
        self._add(faction, "default_doctrine", (), {"remove": True})
        return self

    def remove_default_kiting(self, faction: str) -> "Surface":
        """势力级：删掉**默认风筝姿态叶** ⇒ 这一层不再供值。"""
        self._add(faction, "default_kiting", (), {"remove": True})
        return self

    def remove_default_ship_order(self, faction: str) -> "Surface":
        """势力级：删掉**舰队默认指令叶** ⇒ 没有自己叶片的舰回落到 `Idle` / 作用域链。"""
        self._add(faction, "default_ship_order", (), {"remove": True})
        return self

    # -- 设计图（还不存在的舰的出厂规格）----------------------------------------------

    def set_blueprint(self, faction: str, name: str, *, class_: str | None = None,
                      components: Sequence[str] | None = None, order: Any = None,
                      mode: str | None = None, take_over: bool = False) -> "Surface":
        """建/改一张**设计图**：舰级 + 选装 + 该图给新舰的默认意图。

        * ``class_``：舰级（``ShipSpec`` 的 key）。**新建时必须给**，而且必须与该建造区的
          ``ship_type`` 相等——口径 A：``Building.ship_type`` 仍是「这个区造哪一级」的
          唯一真相，图的舰级对不上会被引擎拒（``blueprint_class_mismatch``）。两处要一起改
          （同一份 diff 里给 ``buildings[].ship_type``），见 :meth:`set_blueprint_pointer`。
        * ``components``：选装（组件 id 顺序 = 槽位）。**空列表 = 交给生成器**
          （``choose_loadout`` 在出厂那一刻按当时库存现算）。不许重复（``duplicate_component``），
          不许超过该舰级的槽位（``too_many_components``），组件必须存在（``no_such_component``）。
        * ``order``：本图给**新舰**的默认意图（``{"type":"dock","body":"地球"}`` 这种 tagged
          写法与 ``"Idle"`` 都收）。⚠ 图的**意图轴默认沉默**——只有真写了它（且图的归属是
          ``Player``）才遮住舰队默认。要**清空**这一层（回到沉默）用 :meth:`silence_blueprint_order`。
        * 写值必须明说归属（``mode=…`` 或 ``take_over=True``）——「写值即接管」的守卫。

        新建图必须把 ``class_`` 一起给（引擎只写 `mode` 时不会凭空造图，会报
        ``no_such_blueprint``）。
        """
        _BLUEPRINT_CLASS = class_
        patch: dict = {"name": name}
        wrote = False
        if _BLUEPRINT_CLASS is not None:
            patch["class"] = _BLUEPRINT_CLASS
            wrote = True
        if components is not None:
            patch["components"] = [_component_id(c) for c in components]
            wrote = True
        if order is not None:
            patch["order"] = normalize_behavior(order)
            wrote = True
        m = self._mode_or_takeover(mode, take_over, f"set_blueprint({name!r})") if wrote else (
            _check_mode(mode) if mode is not None else None)
        if m is not None:
            patch["mode"] = m
        self._add(faction, "blueprints", (name,), patch)
        return self

    def silence_blueprint_order(self, faction: str, name: str) -> "Surface":
        """让这张图的**意图轴沉默**（写 ``"order": null``）——建图时没写、现在收回这一层。

        ⚠ 这与 :meth:`remove_blueprint` 是**两件事**：清空 ``order`` 只是这一层不再说话
        （链继续往下降到舰队默认），图与建造区指针都还在；删图会让挂它的建造区变成
        **悬空指针 ⇒ 停产**。
        """
        self._add(faction, "blueprints", (name,), {"name": name, "order": None})
        return self

    def remove_blueprint(self, faction: str, name: str) -> "Surface":
        """删掉**整张图**（= 改名/换代的正规路径）。

        ⚠ 挂它的建造区**不会**被自动改指针：它们随后是**悬空指针 ⇒ 停产**（进度不再增加），
        读面照旧把那个指针原样输出。想让它们回去自动选装，逐个
        :meth:`set_blueprint_pointer(..., blueprint=None)`。
        """
        return self.remove(faction, "blueprints", name)

    def set_blueprint_pointer(self, faction: str, city: str, building: Any,
                              blueprint: str | None) -> "Surface":
        """把一个**建造区**指向一张图（``blueprint=名字``）或拆掉指针（``blueprint=None``）。

        这是**结构叶**补丁（``buildings[]``），不是控制叶——所以它与 ``Surface.remove`` 那套
        叶键无关。两道校验在引擎侧：库里没有那个名字 ⇒ ``no_such_blueprint``（**绝不静默回落
        生成器**）；图与该区的 ``ship_type`` 对不上 ⇒ ``blueprint_class_mismatch``。
        ``None`` 写的是 ``null``（**拆掉**），不是"缺席"（缺席 = 不动这一格）。
        """
        idx = self.resolve_building(city, building)
        entry = {"city": city, "building": idx, "blueprint": blueprint}
        self._require_faction(faction)
        self._pending_buildings.setdefault(faction, []).append(entry)
        return self

    def set_blueprint_and_retool(self, faction: str, name: str, *, class_: str, city: str,
                                 building: Any, components: Sequence[str] | None = None,
                                 order: Any = None, mode: str | None = None,
                                 take_over: bool = False) -> "Surface":
        """**一条命令把图与建造区的舰级一起改**（口径 A 的正解：两处一起写）。

        引擎的守卫是双向的：只改图的 ``class`` 或只改区的 ``ship_type`` 都会报
        ``blueprint_class_mismatch``——因为那会让图与它自己的建造区对不上（= 把玩家的图作废）。
        这两笔必须在**同一份 diff** 里，所以这里合成一个调用。
        """
        self.set_blueprint(faction, name, class_=class_, components=components, order=order,
                           mode=mode, take_over=take_over)
        idx = self.resolve_building(city, building)
        self._require_faction(faction)
        self._pending_buildings.setdefault(faction, []).append(
            {"city": city, "building": idx, "ship_type": class_, "blueprint": name})
        return self

    def set_scope(self, *, global_mode: str | None = None,
                  factions: Mapping[str, str] | None = None,
                  bodies: Mapping[str, str] | None = None,
                  cities: Mapping[str, str] | None = None) -> "Surface":
        """Edit the scope tree (谁负责 — the layer that decides ownership, not values).

        ⚠ A scope node **carries no value** (``ControlScope`` is ownership-only): setting a faction's
        scope to ``Player`` hands new ships to you but leaves them recording ``Idle`` until a leaf (or
        the fleet default) says otherwise. That is exactly why the fleet-level
        ``default_ship_order`` leaf exists.
        """
        if global_mode is not None:
            self._scope_pending["global"] = _check_mode(global_mode)
        for key, table in (("factions", factions), ("bodies", bodies), ("cities", cities)):
            for node, mode in (table or {}).items():
                self._scope_pending.setdefault(key, {})[node] = _check_mode(mode)
        return self

    def set_scope_mode(self, kind: str, name: str, mode: str) -> "Surface":
        if kind == "global":
            return self.set_scope(global_mode=mode)
        return self.set_scope(**{kind: {name: mode}})

    def _known_resources(self, faction: str, kind: str) -> set[str]:
        """Resource keys we can vouch for: the faction's existing leaves, else the rules table."""
        known = {k[0] for (f, kd, k) in self._index if f == faction and kd == kind}
        if known:
            return known
        if self.ckpt is not None:
            try:
                rv = self.projection().resource_value()
                if rv is not None and len(rv):
                    return set(map(str, rv.index))
            except Exception:  # pragma: no cover
                pass
        return set()

    def _known_cities(self, faction: str) -> set[str]:
        if self.ckpt is None:
            return set()
        q = self.projection()
        r = _last_round(q)
        return set(q.cities(round=r).query("faction_id == @faction")["city_id"])

    def _known_bodies(self) -> set[str]:
        if self.ckpt is None:
            return set()
        return set(self.projection().bodies()["body_id"])

    # -- emit --------------------------------------------------------------------------

    def emit(self) -> dict:
        """The pending edits as an ``--apply`` diff: ``{"control": […], "scope": {…}}``.

        **Deterministic**: factions come out in the engine's own order, and every keyed leaf list is
        ordered by the read face's order (unknown keys last, alphabetically). The same checkpoint
        plus the same recipe therefore produces a byte-identical diff.
        """
        order = list(self.factions)
        extra = sorted(f for f in self._pending if f not in order)
        for f in self._pending_buildings:
            if f not in order and f not in extra:
                extra.append(f)
        control = []
        for faction in order + extra:
            kinds = self._pending.get(faction)
            buildings = self._pending_buildings.get(faction) or []
            if not kinds and not buildings:
                continue
            entry: dict[str, Any] = {"faction_id": faction}
            for kind in _KIND_ORDER:
                if not kinds or kind not in kinds:
                    continue
                bucket = kinds[kind]
                if not LEAF_KINDS[kind]:
                    entry[kind] = copy.deepcopy(bucket[next(iter(bucket))])
                    continue
                keys = sorted(bucket, key=lambda k: (self._rank.get((faction, kind, k), 10 ** 9),
                                                     _key_label(k)))
                entry[kind] = [copy.deepcopy(bucket[k]) for k in keys]
            # 结构叶（设计图指针）：**追加顺序**即调用顺序（同一格被写两次时，后一条覆盖前一条
            # ——引擎按顺序应用，所以 emit 的顺序就是语义）。
            if buildings:
                entry["buildings"] = copy.deepcopy(buildings)
            if len(entry) > 1:
                control.append(entry)
        return {"control": control, "scope": self._scope_patch()}

    def _scope_patch(self) -> dict:
        out: dict[str, Any] = {}
        if "global" in self._scope_pending:
            out["global"] = self._scope_pending["global"]
        for key in ("factions", "bodies", "cities"):
            if key in self._scope_pending:
                out[key] = [[node, mode] for node, mode in sorted(self._scope_pending[key].items())]
        return out

    def __repr__(self) -> str:  # pragma: no cover - display only
        return (f"Surface(ckpt={self.ckpt!r}, factions={len(self.factions)}, "
                f"pending={sum(len(v) for v in self._pending.values())} kinds)")


def _normalize_key(kind: str, key: Any) -> tuple:
    fields = LEAF_KINDS[kind]
    if not fields:
        if key is not None:
            raise ValueError(f"{kind} 是势力级单片叶，不需要 key（收到 {key!r}）")
        return ()
    if len(fields) == 1:
        if isinstance(key, (tuple, list)):
            if len(key) != 1:
                raise ValueError(f"{kind} 的 key 只有一个字段 {fields[0]}，收到 {key!r}")
            key = key[0]
        return (key,)
    if not isinstance(key, (tuple, list)) or len(key) != len(fields):
        raise ValueError(f"{kind} 的 key 需要 {len(fields)} 个字段 {fields}，收到 {key!r}")
    vals = []
    for f, v in zip(fields, key):
        vals.append(int(v) if f == "building" else v)
    return tuple(vals)


# --------------------------------------------------------------------------------------
# module-level readers
# --------------------------------------------------------------------------------------

#: Cache of the **raw** ``--control`` documents (never of :class:`Surface` objects: a Surface carries
#: pending edits, so handing the same instance to two recipes would leak one recipe's intent into the
#: next — a bug this kit's own demo caught).
_SURFACE_CACHE: dict[tuple, dict] = {}


def surface(ckpt: str | os.PathLike | None = None, *, planet_x=None,
            control: Mapping | None = None, index_dir=None, cache: bool = True) -> Surface:
    """Read the control face of a checkpoint (``planet_x --start CKPT --control``).

    Every call returns a **fresh** Surface with no pending edits, so two recipes over the same
    checkpoint cannot contaminate each other. Pass ``control=`` (a parsed ``--control`` document)
    instead of ``ckpt`` when you already have one; ``ckpt`` is then only used to resolve
    ``(city, building)`` indices, so pass both if you intend to write building-keyed weights.
    ``index_dir=`` points the auxiliary lookups at an existing ``--index`` projection of the same
    checkpoint (useful for the economy tables — see the README note on flow metrics).
    """
    if control is not None:
        return Surface(control, ckpt, planet_x=planet_x, index_dir=index_dir, source="(in-memory)")
    if ckpt is None:
        raise ValueError("surface() 需要 ckpt=（或直接给 control=）")
    p = Path(ckpt).resolve()
    st = p.stat()
    key = (str(p), st.st_mtime_ns, st.st_size, str(planet_x))
    raw = _SURFACE_CACHE.get(key) if cache else None
    if raw is None:
        raw = _run_json(["--start", str(p), "--control"], planet_x=planet_x)
        if cache:
            _SURFACE_CACHE[key] = raw
    return Surface(raw, str(p), planet_x=planet_x, index_dir=index_dir)


def control_schema(*, planet_x=None) -> dict:
    """``planet_x --control-schema`` — the machine-readable definition of a legal diff."""
    return _run_json(["--control-schema"], planet_x=planet_x)


def run_engine(args: Sequence[str], *, planet_x=None, timeout: float = 600.0):
    """Escape hatch: run ``planet_x`` with arbitrary flags and get the raw process back."""
    return _run(list(args), planet_x=planet_x, timeout=timeout)


def new_checkpoint(path: str | os.PathLike, *, seed: int | None = None, rounds: int = 0,
                   planet_x=None, index_dir: str | os.PathLike | None = None) -> str:
    """Generate a fresh checkpoint (``--seed S --round N --save FILE``) and return its path.

    With ``index_dir=`` the same run also writes a projection (``--index DIR``) — one pass, so the
    projection's last round and the checkpoint describe **the same state**. Do this when you need
    the flow metrics (``production_value`` / ``upkeep`` / ``governance_cost``); see the README note
    "投影的 flow 数字只在真跑过的回合里才有".
    """
    args = []
    if seed is not None:
        args += ["--seed", str(int(seed))]
    args += ["--round", str(int(rounds))]
    if index_dir is not None:
        args += ["--index", str(index_dir)]
    args += ["--save", str(path)]
    proc = _run(args, planet_x=planet_x)
    if proc.returncode != 0:
        raise EngineError(f"生成 checkpoint 失败（退出码 {proc.returncode}）：{proc.stderr.strip()}")
    return str(path)


def _leaf_lookup(s: Surface) -> dict[tuple, Leaf]:
    out: dict[tuple, Leaf] = {}
    for (fac, kind, key) in s._index:
        out[(fac, kind, key)] = s.leaf(fac, kind, key)
    return out


def ships(ckpt: str | os.PathLike, *, round: int | None = None, planet_x=None,
          index_dir=None) -> pd.DataFrame:
    """The projection's ``ships`` table **joined against the control leaves**.

    Per ship: ``ship_id``/``name``/``class``/``faction_id``/``hull`` + the ship's own leaf
    (``order_mode`` / ``order_value`` / ``order_leaf``) + the faction's ``default_ship_order_mode`` /
    ``default_ship_order_value`` + its ``doctrine_*`` / ``kiting`` record.

    The three ``*_approx`` columns are the kit's **local approximation** of the engine's chain
    resolution — see ``APPROX_COLUMNS`` and the README. Do not treat them as authoritative.
    """
    q = projection(ckpt, planet_x=planet_x, index_dir=index_dir)
    r = _last_round(q) if round is None else int(round)
    df = q.ships(round=r).copy()
    if df.empty:
        return df
    s = surface(ckpt, planet_x=planet_x, index_dir=index_dir)
    leaves = _leaf_lookup(s)

    def L(fac, kind, key):
        return leaves.get((fac, kind, key))

    defaults = {f: s.leaf(f, "default_ship_order") for f in s.factions}
    gscope = s.scope_of("global")
    fscope = {f: s.scope_of("factions", f) for f in s.factions}

    order_leaf_, order_mode, order_value, order_behavior = [], [], [], []
    dso_mode, dso_value = [], []
    dt, dlw, kit = [], [], []
    eff_mode, eff_value, eff_auth = [], [], []
    for _, row in df.iterrows():
        fac, ship = row["faction_id"], row["ship_id"]
        lf = L(fac, "ship_orders", (ship,))
        exists = lf is not None and lf.exists
        omode = lf.mode if lf is not None else INHERIT
        oval = lf.value if lf is not None else None
        order_leaf_.append(bool(exists))
        order_mode.append(omode)
        order_value.append(oval)
        order_behavior.append(behavior_str(oval))
        d = defaults.get(fac)
        dso_mode.append(d.mode if d is not None else INHERIT)
        dso_value.append(behavior_str(d.value) if d is not None else None)
        doc = L(fac, "ship_doctrine", (ship,))
        dt.append((doc.raw or {}).get("temper") if doc is not None and doc.exists else None)
        dlw.append((doc.raw or {}).get("lone_wolf") if doc is not None and doc.exists else None)
        kk = L(fac, "ship_kiting", (ship,))
        kit.append(kk.value if kk is not None and kk.exists else None)
        # ---- local approximation of `leaf → fleet default → faction scope → global` ----
        if omode != INHERIT:
            authority, mode = "leaf", omode
        elif d is not None and d.mode != INHERIT:
            authority, mode = "fleet_default", d.mode
        elif fscope.get(fac, INHERIT) != INHERIT:
            authority, mode = "faction_scope", fscope[fac]
        elif gscope != INHERIT:
            authority, mode = "global_scope", gscope
        else:
            authority, mode = "auto_fallback", AUTO
        # value rule (`State::ship_behavior`): an Inherit leaf only takes the fleet default's VALUE
        # when that default is itself `Player`; otherwise the leaf's (possibly stale) record is used.
        if omode == INHERIT and d is not None and d.mode == PLAYER:
            eff_val = behavior_str(d.value)
        else:
            eff_val = behavior_str(oval)
        eff_mode.append(mode)
        eff_value.append(eff_val)
        eff_auth.append(authority)

    df["order_leaf"] = order_leaf_
    df["order_mode"] = order_mode
    df["order_value"] = order_value
    df["order_behavior"] = order_behavior
    df["default_ship_order_mode"] = dso_mode
    df["default_ship_order_value"] = dso_value
    df["doctrine_temper"] = dt
    df["doctrine_lone_wolf"] = dlw
    df["kiting"] = kit
    df["effective_order_mode_approx"] = eff_mode
    df["effective_order_value_approx"] = eff_value
    df["effective_authority_approx"] = eff_auth
    return df


def cities(ckpt: str | os.PathLike, *, round: int | None = None, planet_x=None,
           index_dir=None) -> pd.DataFrame:
    """The projection's ``cities`` table joined against the city-keyed control leaves.

    Adds ``loyalty_budget_mode`` / ``loyalty_budget_value`` plus per-city weight aggregates
    (``invest_weight_*`` / ``build_weight_*``: count, sum, and how many leaves are ``Player``).
    """
    q = projection(ckpt, planet_x=planet_x, index_dir=index_dir)
    r = _last_round(q) if round is None else int(round)
    df = q.cities(round=r).copy()
    if df.empty:
        return df
    s = surface(ckpt, planet_x=planet_x, index_dir=index_dir)
    leaves = _leaf_lookup(s)
    lb_mode, lb_value = [], []
    agg: dict[str, dict] = {}
    for kind in _WEIGHT_KINDS:
        for (fac, kd, key) in leaves:
            if kd != kind:
                continue
            st = agg.setdefault(key[0], {"invest_n": 0, "invest_sum": 0.0, "invest_player": 0,
                                         "build_n": 0, "build_sum": 0.0, "build_player": 0})
            pre = "invest" if kind == "invest_weights" else "build"
            leaf = leaves[(fac, kd, key)]
            st[f"{pre}_n"] += 1
            st[f"{pre}_sum"] += float(leaf.value or 0.0)
            if leaf.mode == PLAYER:
                st[f"{pre}_player"] += 1
    for _, row in df.iterrows():
        city = row["city_id"]
        lf = s.leaf(row["faction_id"], "loyalty_budget", (city,))
        lb_mode.append(lf.mode if lf.exists else INHERIT)
        lb_value.append(lf.value if lf.exists else None)
    df["loyalty_budget_mode"] = lb_mode
    df["loyalty_budget_value"] = lb_value
    for col in ("invest_n", "invest_sum", "invest_player", "build_n", "build_sum", "build_player"):
        df[col] = [agg.get(c, {}).get(col, 0) for c in df["city_id"]]
    return df


def buildings(ckpt: str | os.PathLike, *, round: int | None = None, planet_x=None,
              index_dir=None) -> pd.DataFrame:
    """**The ``(city, building)`` index table** — one row per building, same checkpoint.

    ``building`` is a per-city ``u32`` index: ``InvestKey = BuildKey = (CityId, BuildingId)``. It is
    only meaningful for the round it was read from, which is exactly why the kit is a *same-round
    transform* (read a ckpt → emit a diff → apply it to that same ckpt).
    """
    q = projection(ckpt, planet_x=planet_x, index_dir=index_dir)
    r = _last_round(q) if round is None else int(round)
    rows = []
    for _, c in q.cities(round=r).iterrows():
        for b in (c.get("buildings") or []):
            rows.append({"city": c["city_id"], "building": int(b["id"]),
                         "faction_id": c["faction_id"], "kind": b.get("kind"),
                         "resource": b.get("resource"), "ship_type": b.get("ship_type"),
                         # **设计图指针**（原样输出：指向一张不存在/被改名的图时也照样在这里，
                         # 那个建造区**停产**——见 Q10(a)）。
                         "blueprint": b.get("blueprint"),
                         "structure": b.get("structure"), "area": b.get("area"),
                         "deployed": b.get("deployed"), "armor": b.get("armor")})
    return pd.DataFrame(rows, columns=["city", "building", "faction_id", "kind", "resource",
                                       "ship_type", "blueprint", "structure", "area", "deployed",
                                       "armor"])


def ships_and_cities(ckpt: str | os.PathLike, *, round: int | None = None, planet_x=None,
                     index_dir=None) -> pd.DataFrame:
    """One frame for "everything I control": ships and cities, tagged by ``kind``.

    Columns the two tables share (``name``/``faction_id``/``hull``/…) line up; the rest are padded
    with ``NaN``. Handy for one ``df.query(...)`` over a whole empire.
    """
    sh = ships(ckpt, round=round, planet_x=planet_x, index_dir=index_dir)
    ci = cities(ckpt, round=round, planet_x=planet_x, index_dir=index_dir)
    sh = sh.rename(columns={"ship_id": "entity_id"}).assign(kind="ship")
    ci = ci.rename(columns={"city_id": "entity_id"}).assign(kind="city")
    return pd.concat([sh, ci], ignore_index=True, sort=False)


# --------------------------------------------------------------------------------------
# roster (编制表)
# --------------------------------------------------------------------------------------

#: The default **refresh rule** for a roster slot: highest current ``hull``, then ``hull_max``,
#: then **oldest first** (``spawned_round`` ascending — the engine's ``Ship.spawned_round``, exposed
#: as the ships table's ``spawned_round`` column), then name ascending (names carry the generation
#: suffix: 方舟 / 方舟2 / 方舟3).
#:
#: ⚠ ``spawned_round`` is ``null`` for ships that predate the column (old checkpoints) — pandas sorts
#: NaN **last** in ascending order, so "unknown" never wins the tie-break; those ties fall through to
#: the name order, which is what the rule did before this column existed.
DEFAULT_REFRESH_RULE: tuple[str, ...] = ("-hull", "-hull_max", "spawned_round", "ship_id")


def roster(ckpt: str | os.PathLike, spec: Sequence, *, planet_x=None, index_dir=None,
           rule: Sequence[str] | None = None, strict: bool = False) -> pd.DataFrame:
    """The **编制表**: stable slot names → the ship that currently fills them.

    ``spec`` is an ordered list of ``(slot_name, query)`` (optionally ``(slot, query, rule)``).
    A slot is an **intent**, not a promise: the refresh rule is part of the recipe and re-runs every
    turn, which is what makes the roster survive name generations — a sunk 旗舰 is back-filled from
    the query, without anyone hand-copying a name::

        spec = [("第1舰队·旗舰", "class=='cruiser' and faction_id=='中国'"),
                ("第1舰队·护卫", "class=='corvette' and faction_id=='中国'")]
        r = ctl.roster(ckpt, spec)

    **Deterministic tie-break** (``DEFAULT_REFRESH_RULE``): highest ``hull`` → highest ``hull_max``
    → **oldest first** (``spawned_round`` ascending, unknown/``null`` last) → name ascending.
    ``spawned_round`` is the engine's own birth round (``Ship.spawned_round``, shipped as the ships
    table's ``spawned_round`` column); ties that involve ships from an old checkpoint (``null``) fall
    back to name order, exactly as before that column existed. Pass ``rule=`` to override, using
    ``"-col"`` for descending.

    A rule column the frame does not carry (e.g. an index directory written by an **older** engine,
    which has no ``spawned_round``) is **skipped** rather than raising — the roster then degrades to
    the remaining columns (name order), never to an exception.

    Returns one row per slot: ``slot``/``query``/``refresh_rule``/``matched``/``candidates`` plus the
    matched ship's columns; ``matched=False`` means nothing currently fills the slot (``strict=True``
    turns that into an error instead — use it when a missing slot must stop the recipe).
    """
    ships_df = ships(ckpt, planet_x=planet_x, index_dir=index_dir)
    rows = []
    for item in spec:
        if len(item) == 2:
            slot, query_expr = item
            slot_rule = rule
        elif len(item) == 3:
            slot, query_expr, slot_rule = item
        else:
            raise ValueError(f"spec 条目必须是 (slot, query) 或 (slot, query, rule)：{item!r}")
        slot_rule = tuple(slot_rule or rule or DEFAULT_REFRESH_RULE)
        cand = query(ships_df, query_expr)
        # 规则里可能有这一帧没有的列（老引擎写的索引目录没有 `spawned_round`）：**跳过它**、
        # 用剩下的列排序（回落名字序），而不是抛 KeyError —— 编制表在旧索引目录上还得能用。
        usable = tuple(c for c in slot_rule if c.lstrip("-") in cand.columns)
        skipped_cols = tuple(c for c in slot_rule if c not in usable)
        if usable:
            cand = cand.sort_values(by=[c.lstrip("-") for c in usable],
                                    ascending=[not c.startswith("-") for c in usable],
                                    kind="mergesort",
                                    # 未知的下水回合（旧档 = null）排在**最后**：不许用 NaN 赢得
                                    # tie-break（那会让"未知"变成"最老"）。
                                    na_position="last")
        base = {"slot": slot, "query": query_expr, "refresh_rule": ",".join(slot_rule),
                "rule_columns_missing": ",".join(skipped_cols),
                "candidates": int(len(cand))}
        if len(cand) == 0:
            if strict:
                raise ValueError(f"编制表槽位 {slot!r} 没有匹配的舰：query={query_expr!r}")
            rows.append({**base, "matched": False})
            continue
        top = cand.iloc[0]
        rows.append({**base, "matched": True, **{k: top[k] for k in cand.columns}})
    out = pd.DataFrame(rows)
    for col in ("matched",):
        if col in out.columns:
            out[col] = out[col].fillna(False).astype(bool)
    return out


# --------------------------------------------------------------------------------------
# writing / verifying
# --------------------------------------------------------------------------------------

def dumps(diff: Mapping) -> str:
    """Canonical, deterministic serialization of a diff (UTF-8, 2-space indent, trailing newline)."""
    return json.dumps(diff, ensure_ascii=False, indent=2, sort_keys=False) + "\n"


def write(diff: Mapping, path: str | os.PathLike) -> str:
    """Write a diff to disk in the canonical form (``json.loads(open(p)) == diff``)."""
    text = dumps(diff)
    Path(path).write_text(text, encoding="utf-8", newline="\n")
    return str(path)


def load_json(path: str | os.PathLike) -> Any:
    with open(path, encoding="utf-8") as fh:
        return json.load(fh)


@dataclass(frozen=True)
class Request:
    """One field the diff asked the engine to write, and what actually happened to it.

    The distinction matters and is the whole point of :func:`verify` closing the
    "只报丢弃不报生效" hole:

    * ``changed``   — the field genuinely moved (before ≠ after);
    * ``noop``      — the field already held the requested value, so the write was a no-op;
    * ``skipped``   — the engine **rejected** the leaf (``WARN_APPLY_SKIPPED``);
    * ``satisfied`` — the post-overlay read face shows the requested value (changed or no-op);
    * **not satisfied and not skipped** — the write silently failed to land. That is a bug in the
      diff; :attr:`Report.ok` is ``False``.
    """

    leaf: str
    field: str
    value: Any
    before: Any = None
    after: Any = None
    changed: bool = False
    satisfied: bool = False
    skipped: bool = False
    skip_reason: str = ""
    took_over: bool = False

    @property
    def noop(self) -> bool:
        return self.satisfied and not self.changed

    @property
    def landed(self) -> bool:
        return self.satisfied and not self.skipped

    def as_dict(self) -> dict:
        return {"leaf": self.leaf, "field": self.field, "requested": self.value,
                "before": self.before, "after": self.after,
                "changed": self.changed, "noop": self.noop, "satisfied": self.satisfied,
                "skipped": self.skipped, "skip_reason": self.skip_reason,
                "took_over": self.took_over}


@dataclass(frozen=True)
class LeafChange:
    """One leaf field that the overlay actually moved."""

    leaf: str
    field: str
    before: Any
    after: Any
    requested: bool = False

    def as_dict(self) -> dict:
        return {"leaf": self.leaf, "field": self.field, "before": self.before,
                "after": self.after, "requested": self.requested}


@dataclass
class Report:
    """The result of :func:`verify` — what the engine *accepted*, not what the game will *do*."""

    ckpt: str
    diff: dict
    exit_code: int
    before: Surface | None = None
    after: Surface | None = None
    raw_receipt: list[str] = _dc_field(default_factory=list)
    receipt: list[dict] = _dc_field(default_factory=list)
    stderr_text: str = ""
    error: str | None = None
    requests: list[Request] = _dc_field(default_factory=list)
    changes: list[LeafChange] = _dc_field(default_factory=list)
    skipped: list[dict] = _dc_field(default_factory=list)
    took_over: list[str] = _dc_field(default_factory=list)
    took_over_leafs: list[str] = _dc_field(default_factory=list)
    removed: list[str] = _dc_field(default_factory=list)
    removed_leafs: list[str] = _dc_field(default_factory=list)
    applied: int = 0
    applied_from: str = "engine"
    honesty: str = _HONESTY

    # -- derived views -----------------------------------------------------------------

    @property
    def ok(self) -> bool:
        """The engine accepted the whole diff: exit 0, nothing skipped, nothing silently lost.

        A **no-op** write (the leaf already held that value) counts as accepted — it is reported,
        not punished. A request that neither landed nor was skipped fails ``ok``.
        """
        return (self.exit_code == 0 and not self.skipped and self.error is None
                and all(r.landed for r in self.requests))

    @property
    def changed(self) -> list[LeafChange]:
        """Requested leaves that genuinely moved."""
        return [c for c in self.changes if c.requested]

    @property
    def unchanged_requests(self) -> list[Request]:
        """Requested fields that did **not** move — the "只报丢弃不报生效" blind spot, closed."""
        return [r for r in self.requests if not r.changed and not r.skipped]

    @property
    def noop_requests(self) -> list[Request]:
        """Requested fields that already held the requested value (harmless, but worth knowing)."""
        return [r for r in self.requests if r.noop and not r.skipped]

    @property
    def failed_requests(self) -> list[Request]:
        """Requested fields the engine neither rejected nor landed — the alarming case."""
        return [r for r in self.requests if not r.skipped and not r.satisfied]

    @property
    def incidental(self) -> list[LeafChange]:
        """Leaves that moved **without being asked** — the honest takeover list."""
        return [c for c in self.changes if not c.requested]

    def summary(self) -> pd.DataFrame:
        """A **tidy summary**: one row per requested field, plus whether it landed."""
        cols = ["leaf", "field", "requested", "before", "after", "changed", "noop",
                "satisfied", "skipped", "skip_reason", "took_over"]
        return pd.DataFrame([r.as_dict() for r in self.requests], columns=cols)

    def changes_frame(self) -> pd.DataFrame:
        """Every leaf field the overlay moved (``requested`` marks the intended ones)."""
        cols = ["leaf", "field", "before", "after", "requested"]
        return pd.DataFrame([c.as_dict() for c in self.changes], columns=cols)

    def skipped_frame(self) -> pd.DataFrame:
        cols = ["code", "path", "reason", "value", "leaf"]
        return pd.DataFrame(self.skipped, columns=cols)

    def to_dict(self) -> dict:
        return {
            "ckpt": self.ckpt, "ok": self.ok, "exit_code": self.exit_code, "error": self.error,
            "applied": self.applied, "skipped": self.skipped, "took_over": self.took_over,
            "took_over_leafs": self.took_over_leafs, "removed": self.removed,
            "removed_leafs": self.removed_leafs,
            "requests": [r.as_dict() for r in self.requests],
            "changes": [c.as_dict() for c in self.changes],
            "receipt": self.receipt, "honesty": self.honesty,
        }

    def describe(self) -> str:
        """A short human/agent-readable paragraph (what the demo prints)."""
        lines = [f"verify {os.path.basename(self.ckpt)}: exit={self.exit_code} "
                 f"applied={self.applied} ({self.applied_from}) requested={len(self.requests)} "
                 f"changed={len(self.changed)} noop={len(self.noop_requests)} "
                 f"skipped={len(self.skipped)} took_over={len(self.took_over)} "
                 f"removed={len(self.removed)} "
                 f"incidental={len(self.incidental)} ok={self.ok}"]
        if self.error:
            lines.append(f"  ERROR: {self.error}")
        for leaf in self.removed_leafs or self.removed:
            lines.append(f"  removed: {leaf}（这片叶被真的删掉了 ⇒ 取值换来源）")
        for r in self.failed_requests:
            lines.append(f"  REQUESTED-BUT-NOT-LANDED: {r.leaf}.{r.field} "
                         f"(asked {r.value!r}, read face now {r.after!r})")
        for r in self.unchanged_requests:
            if not r.noop:
                continue
            lines.append(f"  no-op: {r.leaf}.{r.field} already {r.value!r}")
        for s in self.skipped:
            lines.append(f"  skipped: {s.get('code')} @ {s.get('leaf') or s.get('path')} — "
                         f"{(s.get('reason') or '')[:120]}")
        for t in self.took_over:
            lines.append(f"  took over: {t}")
        return "\n".join(lines)


_ENGINE_PATH_RE = re.compile(r"^(?P<fac>[^.]+)\.(?P<kind>[a-z_]+)(\[(?P<i>\d+)\])?(\.(?P<field>.+))?$")


def _leaf_fields(s: Surface) -> dict[str, dict[str, Any]]:
    """``leaf name → {field: value}`` for a whole surface (the structural-diff basis).

    A leaf the read face does not list is normalized to ``Inherit`` — 「叶不存在」≡「显式写
    Inherit」as far as *ownership* goes, so an absent leaf and an explicitly-silent leaf compare equal
    and neither shows up as a spurious change. Field **names are kept exactly as the engine spells
    them** (``behavior`` for the orders, ``value`` for capital/budgets/weights, ``kiting`` for the
    kiting leaf).

    ⚠ That equivalence does **not** hold for the *value*: an existing style leaf supplies its value
    even when its mode is ``Inherit`` (``State::ship_doctrine`` is
    ``leaf.map(|l| l.value).unwrap_or(record)``), while a missing leaf falls back to the ship's factory
    record. Deleting a leaf therefore really changes behaviour, and this function has to be able to
    see it:

    * **势力级单片叶**（`default_*` / `capital`）: 读面给 `null` 就是"没有这片叶" ⇒ 多报一个
      ``exists`` 字段（`False ⇄ True` 是一次真实的"叶被删掉/被建出来"）。
    * **逐舰叶**：读面**看不出来**（`ship_doctrine`/`ship_kiting` 对每艘舰都有一行，列的是**有效
      值**，哪怕叶并不存在）。所以这里不猜：逐舰删叶是否落地看引擎的
      `NOTE_APPLY_REMOVED` 回执（[`Report.removed`]），不看读面。
    """
    out: dict[str, dict[str, Any]] = {}
    for fac in s.raw.get("control") or []:
        fid = fac.get("faction_id")
        for kind in _KIND_ORDER:
            if not LEAF_KINDS[kind]:
                raw = fac.get(kind)
                name = f"{fid}.{kind}"
                if raw is None:
                    out[name] = {"exists": False, "mode": INHERIT,
                                 _VALUE_FIELD.get(kind, "value"): None}
                else:
                    out[name] = {"exists": True, **raw}
                continue
            for entry in fac.get(kind) or []:
                key = _entry_key(kind, entry)
                out[_leaf_name(fid, kind, key)] = {
                    k: v for k, v in entry.items() if k not in _KEY_FIELDS
                }
    return out


def _diff_fields(diff: Mapping) -> dict[str, dict[str, Any]]:
    """``leaf name → {field: value}`` for the **fields the diff writes** (absent = untouched)."""
    out: dict[str, dict[str, Any]] = {}
    for fac in (diff or {}).get("control") or []:
        fid = fac.get("faction_id")
        for kind in _KIND_ORDER:
            if kind not in fac:
                continue
            if not LEAF_KINDS[kind]:
                raw = fac.get(kind)
                if raw is None:
                    continue
                # `remove` 也算一个被请求的字段：删叶请求没有值可写（`_leaf_fields` 那边靠
                # `exists` 翻转看结果），漏掉它会让"删一片势力级叶"在 `requests` 里**消失**，
                # 于是 `verify` 看上去"什么都没请求"——静默的成功比失败更难查。
                # ⚠ 值字段名**按 kind 查表**（`capital`→`value`、`default_freighter`→`freighter`…）：
                # 写成一张写死的名单时，新轴的写值会在 `requests` 里静默消失，与漏掉 `remove` 同一个坑。
                wanted = {"mode", "remove", _VALUE_FIELD.get(kind, "value")}
                if kind in _TWO_AXIS_KINDS:
                    wanted |= {"temper", "lone_wolf"}
                out.setdefault(f"{fid}.{kind}", {}).update(
                    {k: v for k, v in raw.items() if k in wanted})
                continue
            for entry in fac.get(kind) or []:
                key = _entry_key(kind, entry)
                out.setdefault(_leaf_name(fid, kind, key), {}).update(
                    {k: v for k, v in entry.items() if k not in _KEY_FIELDS})
    return out


def _engine_path_to_leaf(diff: Mapping, path: str) -> str | None:
    """``中国.ship_orders[2].behavior`` → ``中国.ship_orders[长城]`` (index → identity)."""
    m = _ENGINE_PATH_RE.match(path)
    if not m:
        return None
    fac, kind, idx = m.group("fac"), m.group("kind"), m.group("i")
    if kind not in LEAF_KINDS or not LEAF_KINDS[kind] or idx is None:
        return path
    entry = None
    for f in (diff or {}).get("control") or []:
        if f.get("faction_id") != fac:
            continue
        lst = f.get(kind) or []
        i = int(idx)
        if 0 <= i < len(lst):
            entry = lst[i]
        break
    if entry is None:
        return path
    return _leaf_name(fac, kind, _entry_key(kind, entry))


def verify(ckpt: str | os.PathLike, diff: Mapping | str | os.PathLike, *,
           planet_x=None) -> Report:
    """Prove the engine **accepted** a diff — two read-only runs, no file ever written.

    Confirmed ordering inside the engine: the ``--apply`` overlay happens **before** the dump flags,
    and ``--control`` prints the post-overlay surface and returns immediately. So::

        planet_x --start ckpt --apply my.json --control   # stdout = read face AFTER, stderr = receipt
        planet_x --start ckpt --control                   # stdout = read face BEFORE

    which makes the loop **pure read-only** (only ``--save`` writes a file). The report answers three
    questions the raw receipts only half-answer:

    * which requested leaves actually changed (a requested-but-unchanged leaf is invisible in the
      receipts — that is the "只报丢弃不报生效" hole this closes);
    * which were skipped, with the engine's own reason (``WARN_APPLY_SKIPPED``);
    * which leaves were taken over **incidentally** (a value write with no ``mode`` implicitly makes
      the leaf ``Player``; ``NOTE_APPLY_TOOKOVER``).

    ⚠ It only proves the *engine accepted the diff*. It never advances a round, so it cannot prove
    the fleet will do what you meant — for that, re-read the world after ``--round K`` or ask the
    engine's own cost→benefit preview (``planet_x --control-plan``).
    """
    if isinstance(diff, (str, os.PathLike)):
        diff_dict = load_json(diff)
    else:
        diff_dict = copy.deepcopy(dict(diff))
    tmp = None
    if isinstance(diff, (str, os.PathLike)):
        diff_path = str(diff)
    else:
        fd, tmp = tempfile.mkstemp(prefix="planet_x_ctl_", suffix=".json")
        os.close(fd)
        diff_path = write(diff_dict, tmp)

    before_raw = _run_json(["--start", str(ckpt), "--control"], planet_x=planet_x)
    proc = _run(["--start", str(ckpt), "--apply", diff_path, "--control"], planet_x=planet_x)

    raw_lines = [ln for ln in (proc.stderr or "").splitlines() if ln.strip()]
    receipt: list[dict] = []
    for ln in raw_lines:
        try:
            receipt.append(json.loads(ln))
        except json.JSONDecodeError:
            receipt.append({"code": "UNPARSED_STDERR", "raw": ln})

    rep = Report(ckpt=str(ckpt), diff=diff_dict, exit_code=proc.returncode,
                 raw_receipt=raw_lines, receipt=receipt, stderr_text=proc.stderr or "",
                 before=Surface(before_raw, str(ckpt), planet_x=planet_x, source="--control"))
    if tmp:
        try:
            os.unlink(tmp)
        except OSError:  # pragma: no cover
            pass

    if proc.returncode != 0:
        for r in receipt:
            if r.get("code") == "ERR_APPLY":
                rep.error = r.get("message") or r.get("raw")
        if rep.error is None:
            rep.error = (proc.stderr or f"退出码 {proc.returncode}").strip()
        return rep

    after_raw = json.loads(proc.stdout)
    rep.after = Surface(after_raw, str(ckpt), planet_x=planet_x, source="--control --apply")

    # ---- receipt → structured facts ----
    for r in receipt:
        code = r.get("code")
        if code == "WARN_APPLY_SKIPPED":
            for s in r.get("skipped") or []:
                s = dict(s)
                s["leaf"] = _engine_path_to_leaf(diff_dict, str(s.get("path", ""))) or s.get("path")
                rep.skipped.append(s)
        elif code == "NOTE_APPLY_TOOKOVER":
            rep.took_over.extend(r.get("took_over") or [])
        elif code == "NOTE_APPLY_REMOVED":
            rep.removed.extend(r.get("removed") or [])
        if isinstance(r.get("applied"), int):
            rep.applied = r["applied"]

    # ---- structural before/after diff (order-independent, name-keyed) ----
    before_f, after_f = _leaf_fields(rep.before), _leaf_fields(rep.after)
    requested = _diff_fields(diff_dict)
    req_pairs = {(leaf, fld) for leaf, fields in requested.items() for fld in fields}
    for leaf in sorted(set(before_f) | set(after_f)):
        b, a = before_f.get(leaf, {}), after_f.get(leaf, {})
        for fld in sorted(set(b) | set(a)):
            if b.get(fld, "<absent>") != a.get(fld, "<absent>"):
                rep.changes.append(LeafChange(leaf, fld, b.get(fld), a.get(fld),
                                              requested=(leaf, fld) in req_pairs))

    # The engine reports takeover paths by *diff index*; translate them back to leaf identities.
    rep.took_over_leafs = sorted({_engine_path_to_leaf(diff_dict, p) or p for p in rep.took_over})
    took_leafs = set(rep.took_over_leafs)
    # 删叶同理：回执里给的是 `中国.ship_doctrine[0].ship` 这种**下标路径**，翻成叶名再比。
    rep.removed_leafs = sorted({_engine_path_to_leaf(diff_dict, p) or p for p in rep.removed})
    removed_leafs = set(rep.removed_leafs)
    for leaf, fields in requested.items():
        skip_hit = next((s for s in rep.skipped if s.get("leaf") == leaf), None)
        for fld, val in fields.items():
            b = before_f.get(leaf, {}).get(fld, "<absent>")
            a = after_f.get(leaf, {}).get(fld, "<absent>")
            # 「删叶」是**另一种**请求：它的成功不是"字段变成了这个值"，而是"这片叶没了"
            # （`remove` 只存在于写面，读面永远没有这个字段）。权威信号是引擎的回执：
            # 没被丢弃 = 落地（幂等删除也算落地，只是不进 `NOTE_APPLY_REMOVED` 的名单）。
            is_remove = fld == "remove" and bool(val)
            removed_here = leaf in removed_leafs
            rep.requests.append(Request(
                leaf=leaf, field=fld, value=val, before=b, after=a,
                changed=(b != a) or removed_here,
                satisfied=(skip_hit is None) if is_remove else _values_match(val, a),
                skipped=skip_hit is not None,
                skip_reason=(skip_hit or {}).get("reason", ""),
                took_over=leaf in took_leafs,
            ))
    # The engine only prints an `applied` count when it also prints a receipt line; a clean apply
    # prints nothing at all (stderr empty). Fall back to counting the leaves that actually landed.
    if not any("applied" in r for r in receipt):
        rep.applied = sum(1 for r in rep.requests if r.landed)
        rep.applied_from = "derived from the before/after read face (clean apply: stderr is empty)"
    return rep


@dataclass
class Applied:
    """The outcome of a **real** ``--apply`` (the only kit call that can write a file)."""

    exit_code: int = 0
    saved: str | None = None
    receipt: list[dict] = _dc_field(default_factory=list)
    raw_receipt: list[str] = _dc_field(default_factory=list)
    stdout: str = ""
    stderr: str = ""
    error: str | None = None

    @property
    def ok(self) -> bool:
        return self.exit_code == 0 and self.error is None

    @property
    def skipped(self) -> list[dict]:
        out = []
        for r in self.receipt:
            if r.get("code") == "WARN_APPLY_SKIPPED":
                out.extend(r.get("skipped") or [])
        return out

    @property
    def took_over(self) -> list[str]:
        out: list[str] = []
        for r in self.receipt:
            if r.get("code") == "NOTE_APPLY_TOOKOVER":
                out.extend(r.get("took_over") or [])
        return out

    @property
    def removed(self) -> list[str]:
        """Paths of the control leaves that were **actually deleted** by this apply.

        The engine only lists leaves that really existed; a delete of a leaf that was already gone is
        an idempotent success and is deliberately *not* listed (see `apply_diff`'s 「删叶」).
        """
        out: list[str] = []
        for r in self.receipt:
            if r.get("code") == "NOTE_APPLY_REMOVED":
                out.extend(r.get("removed") or [])
        return out


def apply(ckpt: str | os.PathLike, diff: Mapping | str | os.PathLike, *,
          save: str | os.PathLike | None = None, rounds: int = 0,
          planet_x=None) -> Applied:
    """Run the engine's ``--apply`` **for real** — the step after :func:`verify`.

    Without ``save=`` the overlay lives only in memory and dies with the process (which is exactly
    what :func:`verify` exploits to stay read-only). With ``save=`` the post-overlay state plus the
    PRNG position is written to a checkpoint, which is how a verified diff actually takes effect::

        rep = ctl.verify(ckpt, diff)          # read-only rehearsal
        assert rep.ok
        ctl.apply(ckpt, diff, save="ckpt2.ron")   # now it is real

    ``rounds`` defaults to 0: overlay and persist *without advancing time*.
    """
    if isinstance(diff, (str, os.PathLike)):
        diff_path = str(diff)
    else:
        fd, tmp = tempfile.mkstemp(prefix="planet_x_ctl_", suffix=".json")
        os.close(fd)
        diff_path = write(diff, tmp)
    args = ["--start", str(ckpt), "--apply", diff_path, "--round", str(int(rounds))]
    if save is not None:
        args += ["--save", str(save)]
    proc = _run(args, planet_x=planet_x)
    res = Applied(exit_code=proc.returncode, saved=str(save) if save is not None else None,
                  stdout=proc.stdout or "", stderr=proc.stderr or "")
    res.raw_receipt = [ln for ln in (proc.stderr or "").splitlines() if ln.strip()]
    for ln in res.raw_receipt:
        try:
            res.receipt.append(json.loads(ln))
        except json.JSONDecodeError:
            res.receipt.append({"code": "UNPARSED_STDERR", "raw": ln})
    for r in res.receipt:
        if r.get("code") == "ERR_APPLY":
            res.error = r.get("message")
    return res


# --------------------------------------------------------------------------------------
# tiny CLI
# --------------------------------------------------------------------------------------

def main(argv: list[str] | None = None) -> int:
    import argparse

    p = argparse.ArgumentParser(
        description="planet_x_ctl — inspect a control surface / verify an --apply diff (read-only).")
    sub = p.add_subparsers(dest="cmd")
    ps = sub.add_parser("surface", help="print a checkpoint's control leaves as JSON")
    ps.add_argument("ckpt")
    pv = sub.add_parser("verify", help="read-only dry run of a diff (before/after + receipts)")
    pv.add_argument("ckpt")
    pv.add_argument("diff")
    pr = sub.add_parser("roster", help="print the ships of a checkpoint (for writing a roster)")
    pr.add_argument("ckpt")
    a = p.parse_args(argv)
    if a.cmd == "surface":
        s = surface(a.ckpt)
        out: dict = {}
        for name in s.factions:
            entry = {"faction_id": name}
            for kind, val in s.faction(name).items():
                if kind == "faction_id":
                    continue
                if isinstance(val, Leaf):
                    entry[kind] = val.as_dict()
                else:
                    entry[kind] = {str(k): leaf.as_dict() for k, leaf in val.items()}
            out[name] = entry
        print(json.dumps(out, ensure_ascii=False, indent=2))
        return 0
    if a.cmd == "verify":
        rep = verify(a.ckpt, a.diff)
        print(rep.describe())
        print(rep.summary().to_string(index=False))
        return 0 if rep.ok else 1
    if a.cmd == "roster":
        df = ships(a.ckpt)
        cols = [c for c in ("ship_id", "faction_id", "class", "hull", "hull_max",
                            "order_mode", "order_behavior", "default_ship_order_mode")
                if c in df.columns]
        print(df[cols].to_string(index=False))
        return 0
    p.print_help()
    return 2


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
