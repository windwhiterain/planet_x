"""Self-asserting end-to-end demo for ``planet_x_ctl`` — no network, no manual setup.

    cd play/planet_x_ctl && uv run python demo.py

What it does (all on a **fixture checkpoint it generates itself**, so it is reproducible):

1. ``planet_x --seed 7 --round 12 --save ckpt.ron`` — a small but real world;
2. bulk **ownership** on a whole fleet: every ship of one faction → ``Auto``, then back to
   ``Player`` (the "wildcard" that lives in Python, expanded to N explicit ``{ship, mode}`` leaves);
3. a **statistical policy**: compare ``upkeep`` against ``production_value`` from the projection's
   metrics and cap the construction budget of every faction that is over-extended;
4. one **deliberate** fleet-default takeover (``take_over=True``), so the receipt's takeover list is
   a known, intended set rather than a surprise;
5. a **no-op** write (a value that already holds), reported as ``noop`` rather than as a failure;
6. the **guards**: a value write with no mode, a stale ship name, a foreign building index, an unknown
   resource and a hand-written malformed diff are all refused / reported honestly;
7. ``verify`` each diff with the two read-only engine runs, then ASSERT:
   no ``WARN_APPLY_SKIPPED`` · every requested leaf landed · ``took_over`` is exactly the intended
   leaves (nothing incidental) · the same checkpoint + same recipe emits a **byte-identical** diff.

Exit code is non-zero if any assertion fails. ``--planet-x PATH`` overrides the engine binary,
``--work DIR`` keeps the scratch directory (otherwise a temp dir is used).
"""
from __future__ import annotations

import argparse
import shutil
import sys
import tempfile
from pathlib import Path

import pandas as pd

import planet_x_ctl as ctl

# ---- policy knobs (the recipe's own choices, written down so it can be replayed) -------------
OVEREXTENDED_RATIO = 0.5   # upkeep / production_value above this = 养不起的舰队
DEFAULT_ORDER_BEHAVIOR = "Dock:地球"
#: 紧缩时一起收缩的两类预算（造舰 + 建设）。引擎把两类预算做成了同一个形状，所以同一条配方
#: 能一次管住两边；`construction_budget` 常在 AI 手里是 0，只有建设预算在动，所以两类都要看。
BUDGET_KINDS = ("construction_budget", "investment_budget")

FAILURES: list[str] = []


def check(label: str, condition: bool, detail: str = "") -> None:
    mark = "ok  " if condition else "FAIL"
    print(f"  [{mark}] {label}" + (f" — {detail}" if detail else ""))
    if not condition:
        FAILURES.append(label if not detail else f"{label} ({detail})")


# ---------------------------------------------------------------------------------------------
# helpers used by the recipes below
# ---------------------------------------------------------------------------------------------

def census(s: ctl.Surface) -> pd.DataFrame:
    """How many leaves of each kind the read face lists, and how many of them already speak."""
    rows = []
    for kind in ctl.LEAF_KINDS:
        leaves = s.leaves(kind) if ctl.LEAF_KINDS[kind] else [s.leaf(f, kind) for f in s.factions]
        rows.append({"kind": kind, "leaves": len(leaves),
                     "spoken": sum(1 for lf in leaves if lf.mode != ctl.INHERIT),
                     "player": sum(1 for lf in leaves if lf.mode == ctl.PLAYER)})
    return pd.DataFrame(rows)


def economy_table(proj_dir) -> pd.DataFrame:
    """Per-faction ``upkeep`` vs ``production_value``, straight out of the projection metrics.

    Read from a projection that came out of a **real run** (``--index``), not from re-projecting the
    checkpoint — see ``README`` §"投影的 flow 数字". Flow numbers (产出 / 维护 / 治理) only exist for
    rounds the engine actually advanced.
    """
    metrics = (ctl.projection(proj_dir).facts.iloc[-1].get("metrics") or {}).get("factions") or {}
    rows = []
    for fid, m in metrics.items():
        prod = float(m.get("production_value") or 0.0)
        up = float(m.get("upkeep") or 0.0)
        rows.append({"faction_id": fid, "production_value": prod, "upkeep": up,
                     "upkeep_ratio": (up / prod) if prod > 0 else float("nan"),
                     "ship_count": m.get("ship_count"), "city_count": m.get("city_count")})
    return pd.DataFrame(rows).sort_values("faction_id").reset_index(drop=True)


# ---------------------------------------------------------------------------------------------
# the recipes  (a recipe is `(surface, world data) -> diff`, replayable and byte-for-byte stable)
# ---------------------------------------------------------------------------------------------

def recipe_ownership(ckpt, names: list[str], mode: str, *, index_dir=None) -> dict:
    """纯归属改写：只写 mode，不写值 —— 所以**不可能**触发「写值即接管」。"""
    s = ctl.surface(ckpt, index_dir=index_dir)
    s.set_mode(names, mode)
    return s.emit()


def recipe_policy(ckpt, proj_dir, *, surface: ctl.Surface | None = None) -> tuple[dict, pd.DataFrame]:
    """统计施政：upkeep 占比过高的势力，其造舰 / 建设预算按 ``(1 - 占比)`` 等比封顶。

    再补**一片**刻意的舰队默认叶（``take_over=True``），让「新舰自动跟随意图」有一个去处 ——
    这份 diff 的回执因此应当**恰好**只有那一处接管。
    """
    s = surface if surface is not None else ctl.surface(ckpt, index_dir=proj_dir)
    eco = economy_table(proj_dir)
    target = eco[(eco["production_value"] > 0) & (eco["upkeep_ratio"] > OVEREXTENDED_RATIO)]
    rows, takeover_faction = [], None
    for _, f in target.sort_values("faction_id").iterrows():
        fac = f["faction_id"]
        scale = max(0.0, min(1.0, 1.0 - f["upkeep_ratio"]))
        for kind in BUDGET_KINDS:
            current = {lf.key[0]: float(lf.value or 0.0)
                       for lf in s.leaves(kind) if lf.faction == fac}
            # 只写真的会变的数字 —— 写一个等值的数只是 no-op（第 6 节专门演示这一点）。
            new = {res: round(v * scale, 4) for res, v in sorted(current.items())
                   if abs(v * scale - v) > 1e-9}
            for res, val in sorted(new.items()):
                rows.append({"faction_id": fac, "kind": kind, "resource": res,
                             "upkeep_ratio": round(f["upkeep_ratio"], 3), "scale": round(scale, 3),
                             "old": current[res], "new": val})
            if new:
                s.set_budget(fac, kind, new, mode=ctl.PLAYER)
        if takeover_faction is None and int(f["ship_count"] or 0) > 0:
            takeover_faction = fac
    if takeover_faction is not None:
        # 唯一一处刻意接管：舰队默认是一片叶，不写 mode 就等于接管 —— 这里明说 take_over=True。
        s.set_default_ship_order(takeover_faction, behavior=DEFAULT_ORDER_BEHAVIOR, take_over=True)
    cols = ["faction_id", "kind", "resource", "upkeep_ratio", "scale", "old", "new"]
    return s.emit(), pd.DataFrame(rows, columns=cols)


# ---------------------------------------------------------------------------------------------

def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--planet-x", default=None,
                    help="引擎可执行文件（默认 <worktree>/target/debug/planet_x.exe）")
    ap.add_argument("--work", default=None, help="工作目录（默认临时目录；给了就保留）")
    ap.add_argument("--seed", type=int, default=7)
    ap.add_argument("--round", type=int, default=12)
    args = ap.parse_args(argv)

    engine = ctl.resolve_engine(args.planet_x)
    keep = args.work is not None
    work = Path(args.work).resolve() if keep else Path(tempfile.mkdtemp(prefix="planet_x_ctl_demo_"))
    work.mkdir(parents=True, exist_ok=True)

    print("=" * 78)
    print("planet_x_ctl demo — pandas 分析 → 合法控制 diff（同回合变换）")
    print(f"engine : {engine}")
    print(f"work   : {work}")
    print("=" * 78)

    # ---------------------------------------------------------------- 0. fixture
    # 一次运行同时产出 checkpoint 与投影：投影的最后一回合 == checkpoint 的状态，
    # 所以「同回合变换」这条硬约束在这里天然成立（building 下标只在同回合自洽）。
    print(f"\n[0] 生成 fixture：planet_x --seed {args.seed} --round {args.round} "
          f"--index proj/ --save ckpt.ron")
    proj = work / "proj"
    ckpt = Path(ctl.new_checkpoint(work / "ckpt.ron", seed=args.seed, rounds=args.round,
                                   planet_x=engine, index_dir=proj))
    print(f"    {ckpt}（{ckpt.stat().st_size} bytes）+ {proj}")

    # ---------------------------------------------------------------- 1. read
    print("\n[1] 读面：--control（读面即写面） + --index 投影（按名字 join）")
    s0 = ctl.surface(ckpt, index_dir=proj)
    cen = census(s0)
    print(cen.to_string(index=False))
    check("读面列出了势力", len(s0.factions) > 0, f"{len(s0.factions)} 个势力")
    schema = ctl.control_schema(planet_x=engine)
    check("--control-schema 可读（可写字段的机器可读定义）",
          "FactionControlPatch" in schema.get("definitions", {}))

    df_ships = ctl.ships(ckpt, index_dir=proj)
    df_cities = ctl.cities(ckpt, index_dir=proj)
    print(f"    ctl.ships(): {df_ships.shape}    ctl.cities(): {df_cities.shape}")
    check("ships 表已 join 控制叶",
          {"order_mode", "order_value", "default_ship_order_mode", "doctrine_temper", "kiting"}
          <= set(df_ships.columns))
    check("本地近似的 effective 列有 _approx 标名", all(c in df_ships.columns for c in ctl.APPROX_COLUMNS))

    counts = df_ships.groupby("faction_id").size().sort_values(ascending=False)
    faction = sorted(counts[counts == int(counts.max())].index)[0]   # 并列时按名字定序
    names = sorted(df_ships.loc[df_ships["faction_id"] == faction, "ship_id"].tolist())
    print(f"    施政对象：{faction}（{len(names)} 艘：{'、'.join(names)}）")

    # 编制表：slot 名是意图，活过名字换代。刷新规则写在配方里（排序键），不留在脑子里。
    # 注意 `class` 是引擎列名，也是 Python 关键字 —— ctl.query 会替你把反引号补上。
    fleet = ctl.query(df_ships, "faction_id == @faction")
    flagship_hull = float(fleet["hull_max"].max())
    classes = (fleet.groupby("class")["hull_max"].max()
               .sort_values(ascending=False).index.tolist())          # 决定性顺序
    spec = [("旗舰", f"faction_id == '{faction}'")] + [
        (f"{cls} 队", f"faction_id == '{faction}' and class == '{cls}'") for cls in classes[:2]]
    ros = ctl.roster(ckpt, spec, index_dir=proj)
    print(ros[["slot", "matched", "candidates", "ship_id", "class", "hull", "hull_max",
               "refresh_rule"]].to_string(index=False))
    check("编制表把每个 slot 映射到现役舰（刷新规则写在配方里）",
          bool(ros["matched"].all()) and ros["slot"].tolist() == [s for s, _ in spec])
    check("编制表槽位是确定的（同分按名字）",
          ros.iloc[0]["ship_id"]
          == fleet.sort_values(["hull", "hull_max", "ship_id"],
                               ascending=[False, False, True]).iloc[0]["ship_id"],
          f"旗舰={ros.iloc[0]['ship_id']}，最高 hull={flagship_hull:g}")

    # ---------------------------------------------------------------- 2. bulk ownership → Auto
    print("\n[2] 批量归属：整队 → Auto（引擎没有通配，这里展开成 N 条只写 mode 的叶）")
    diff_a = recipe_ownership(ckpt, names, ctl.AUTO, index_dir=proj)
    path_a = ctl.write(diff_a, work / "steer_a.json")
    print(f"    {Path(path_a).name}：{faction} 的 "
          f"{len(diff_a['control'][0]['ship_orders'])} 条叶，每条只有 ship+mode")
    rep_a = ctl.verify(ckpt, path_a)
    print(rep_a.describe())
    expected_a = {(f"{faction}.ship_orders[{n}]", "mode") for n in names}
    check("A: 没有 WARN_APPLY_SKIPPED",
          not rep_a.skipped and "WARN_APPLY_SKIPPED" not in rep_a.stderr_text)
    check("A: 一处接管都没有", rep_a.took_over == [] and rep_a.incidental == [])
    check("A: 请求的叶全部落地", all(r.landed for r in rep_a.requests), f"{len(rep_a.requests)} 条")
    check("A: 写面只碰了 mode（没碰任何值）",
          {(r.leaf, r.field) for r in rep_a.requests} == expected_a)
    check("A: 读面确认全队已是 Auto",
          {r.after for r in rep_a.requests if r.field == "mode"} == {ctl.AUTO})
    check("A: 叶上记录的值逐条未动",
          {n: rep_a.before.leaf(faction, "ship_orders", n).value for n in names}
          == {n: rep_a.after.leaf(faction, "ship_orders", n).value for n in names})

    # 真的落地（--save），证明这不是只在内存里演一遍
    ckpt_a = work / "ckpt_a.ron"
    app = ctl.apply(ckpt, path_a, save=ckpt_a)
    check("A: 真实 --apply --save 成功（回执无 skipped）",
          app.ok and not app.skipped and ckpt_a.exists())
    check("A: 存下来的 checkpoint 真的持有 Auto",
          all(ctl.surface(ckpt_a).leaf(faction, "ship_orders", n).mode == ctl.AUTO for n in names))

    # ---------------------------------------------------------------- 3. bulk ownership → Player
    print("\n[3] 批量归属：整队 → Player（同一张编制表，交回玩家）")
    s_b = ctl.surface(ckpt_a, index_dir=proj)
    s_b.set_mode(names, ctl.PLAYER)
    path_b = ctl.write(s_b.emit(), work / "steer_b.json")
    rep_b = ctl.verify(ckpt_a, path_b)
    print(rep_b.describe())
    check("B: 没有 WARN_APPLY_SKIPPED", not rep_b.skipped)
    check("B: 一处接管都没有", rep_b.took_over == [] and rep_b.incidental == [])
    check("B: 全队已归玩家",
          {r.after for r in rep_b.requests if r.field == "mode"} == {ctl.PLAYER})

    # ---------------------------------------------------------------- 4. statistical policy
    print(f"\n[4] 统计施政：upkeep/production_value > {OVEREXTENDED_RATIO} 的势力，"
          f"{' / '.join(BUDGET_KINDS)} 按 (1 − 占比) 等比封顶")
    print(economy_table(proj).to_string(index=False))
    diff_c, table = recipe_policy(ckpt, proj)
    path_c = ctl.write(diff_c, work / "steer_c.json")
    print()
    print(table.to_string(index=False) if len(table) else "    （没有势力超标）")
    expected_took = [f"{e['faction_id']}.default_ship_order"
                     for e in diff_c["control"] if "default_ship_order" in e]
    print(f"    刻意接管 {len(expected_took)} 处：{expected_took}")
    rep_c = ctl.verify(ckpt, path_c)
    print(rep_c.describe())
    check("C: 没有 WARN_APPLY_SKIPPED",
          not rep_c.skipped and "WARN_APPLY_SKIPPED" not in rep_c.stderr_text)
    check("C: 请求的叶全部落地", all(r.landed for r in rep_c.requests),
          f"{len(rep_c.requests)} 条，未落地="
          f"{[f'{r.leaf}.{r.field}' for r in rep_c.failed_requests]}")
    check("C: 预算值真的变了",
          bool(table is not None) and all(r.changed for r in rep_c.requests
                                          if r.field == "value" and "budget" in r.leaf))
    check("C: took_over 恰好是刻意的那几片叶", rep_c.took_over == expected_took, f"{rep_c.took_over}")
    check("C: took_over_leafs 也一一对应", rep_c.took_over_leafs == expected_took,
          f"{rep_c.took_over_leafs}")
    check("C: 顺带变动只有那几处隐含的 mode 翻转（+ 那片叶从无到有）",
          {(c.leaf, c.field, c.after) for c in rep_c.incidental}
          == ({(leaf, "mode", ctl.PLAYER) for leaf in rep_c.took_over_leafs}
              | {(leaf, "exists", True) for leaf in rep_c.took_over_leafs}),
          f"{[(c.leaf, c.field, c.before, c.after) for c in rep_c.incidental]}")

    # ---------------------------------------------------------------- 5. determinism
    print("\n[5] 确定性：同一个 ckpt + 同一份配方 → 逐字节一致的 diff")
    d1, _ = recipe_policy(ckpt, proj)
    d2, _ = recipe_policy(ckpt, proj)
    p1 = ctl.write(d1, work / "det1.json")
    p2 = ctl.write(d2, work / "det2.json")
    b1, b2 = Path(p1).read_bytes(), Path(p2).read_bytes()
    check("两次独立跑配方 → 逐字节一致", b1 == b2, f"{len(b1)} bytes")
    check("同一个 Surface 上重跑归属配方 → 逐字节一致",
          ctl.dumps(recipe_ownership(ckpt, names, ctl.AUTO, index_dir=proj))
          == ctl.dumps(recipe_ownership(ckpt, names, ctl.AUTO, index_dir=proj)))
    check("落盘文件再序列化是不动点（json.loads→dumps 幂等）",
          ctl.dumps(ctl.load_json(p1)) == b1.decode("utf-8"))
    check("两次表面读取互不污染（各自都是干净的待写状态）",
          ctl.dumps(recipe_policy(ckpt, proj)[0]) == ctl.dumps(recipe_policy(ckpt, proj)[0]))

    # ---------------------------------------------------------------- 6. no-op honesty
    print("\n[6] 诚实汇报：写一个「已是当前值」的数字 = no-op，不是失败")
    lf0 = next(lf for kind in BUDGET_KINDS for lf in s0.leaves(kind) if float(lf.value or 0.0) > 0)
    s_n = ctl.surface(ckpt, index_dir=proj)
    s_n.set_budget(lf0.faction, lf0.kind, {lf0.key[0]: float(lf0.value)}, mode=ctl.PLAYER)
    rep_n = ctl.verify(ckpt, s_n.emit())
    print(rep_n.describe())
    check("no-op 被报成 no-op 而不是失败",
          len(rep_n.noop_requests) == 1 and rep_n.ok and not rep_n.skipped,
          f"{lf0.faction}.{lf0.kind}[{lf0.key[0]}] = {lf0.value}")
    check("no-op 没有改动任何 value（只把归属钉成 Player）",
          all(c.field != "value" for c in rep_n.changes)
          and {c.field for c in rep_n.changes} <= {"mode"})
    check("未污染的 Surface 不会带上别人的待写编辑",
          len(ctl.surface(ckpt, index_dir=proj).emit()["control"]) == 0)

    # ---------------------------------------------------------------- 7. never emit a rejected diff
    print("\n[7] 质量栏：宁可在配方期报错，也不产出一份引擎会拒绝的 diff")

    def refuses(label, fn, needle):
        try:
            fn()
        except ValueError as exc:
            check(label, needle in str(exc), str(exc)[:70] + "…")
            return
        check(label, False, "没有报错")

    refuses("写值不写 mode → 当场拒绝（写值即接管）",
            lambda: ctl.surface(ckpt, index_dir=proj).set_behavior(names[:1], "Idle"),
            "接管")
    refuses("手抄的死舰名 → 当场拒绝",
            lambda: ctl.surface(ckpt).set_mode(["方舟3"], ctl.PLAYER),
            "没有名为")
    refuses("跨回合/换城的 building 下标 → 当场拒绝",
            lambda: ctl.surface(ckpt, index_dir=proj).set_build_weights(
                faction, {(ctl.buildings(ckpt, index_dir=proj).iloc[0]["city"], 999): 1.0}, mode=ctl.PLAYER),
            "没有 building=999")
    refuses("不存在的资源 key → 当场拒绝",
            lambda: ctl.surface(ckpt, index_dir=proj).set_budget(
                faction, "construction_budget", {"不存在": 1.0}, mode=ctl.PLAYER),
            "不在已知表里")

    hand_written = {"control": [{"faction_id": faction, "ship_orders": [
        {"ship": names[0], "mode": "Auto"}, {"ship": "方舟3", "mode": "Player"}]}]}
    rep_bad = ctl.verify(ckpt, hand_written)
    check("手写的陈旧叶：verify 只报 skipped、不假装成功",
          not rep_bad.ok and len(rep_bad.skipped) == 1 and rep_bad.skipped[0]["code"] == "no_such_ship",
          rep_bad.skipped[0]["code"] if rep_bad.skipped else "no skip reported")
    rep_err = ctl.verify(ckpt, {"control": [{"faction_id": faction, "ship_order": []}]})
    check("非法 diff（字段名打错）→ exit 10，ok=False，且 not raise",
          rep_err.exit_code == 10 and not rep_err.ok and bool(rep_err.error),
          (rep_err.error or "")[:60] + "…")

    # ---------------------------------------------------------------- summary
    print("\n" + "=" * 78)
    print("摘要")
    print(f"  势力 {len(s0.factions)} · 舰 {len(df_ships)} · 城 {len(df_cities)} · "
          f"叶 {int(cen['leaves'].sum())}（已有人表态 {int(cen['spoken'].sum())}）")
    print(f"  A 批量→Auto  : requested={len(rep_a.requests)} changed={len(rep_a.changed)} "
          f"noop={len(rep_a.noop_requests)} skipped={len(rep_a.skipped)} "
          f"took_over={len(rep_a.took_over)} incidental={len(rep_a.incidental)} ok={rep_a.ok}")
    print(f"  B 批量→Player: requested={len(rep_b.requests)} changed={len(rep_b.changed)} "
          f"noop={len(rep_b.noop_requests)} skipped={len(rep_b.skipped)} "
          f"took_over={len(rep_b.took_over)} incidental={len(rep_b.incidental)} ok={rep_b.ok}")
    print(f"  C 统计封顶   : requested={len(rep_c.requests)} changed={len(rep_c.changed)} "
          f"noop={len(rep_c.noop_requests)} skipped={len(rep_c.skipped)} "
          f"took_over={len(rep_c.took_over)} incidental={len(rep_c.incidental)} ok={rep_c.ok}")
    print(f"    封顶 {len(table)} 条预算，涉 {table['faction_id'].nunique() if len(table) else 0} 个势力；"
          f"新增舰队默认 {len(expected_took)} 处")
    if len(table):
        print(f"    合计削减 {float((table['old'] - table['new']).sum()):.2f}"
              "（按各资源自身单位求和，仅作量级参考）")
    print(f"    新增的舰队默认：{ {e['faction_id']: e['default_ship_order'] for e in diff_c['control'] if 'default_ship_order' in e} }")
    print("=" * 78)

    if FAILURES:
        print(f"\n失败 {len(FAILURES)} 项：")
        for f in FAILURES:
            print(f"  - {f}")
        return 1
    print("\n全部断言通过。")
    if not keep:
        shutil.rmtree(work, ignore_errors=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
