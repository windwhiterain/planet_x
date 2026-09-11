"""Self-asserting end-to-end demo for ``planet_x_ctl`` — no network, no manual setup.

    cd play/planet_x_ctl && uv run python demo.py

What it does (all on a **fixture checkpoint it generates itself**, so it is reproducible):

1. ``planet_x --seed 7 --round 12 --save ckpt.json`` — a small but real world;
2. bulk **ownership** on a whole fleet: every ship of one faction → ``Auto``, then back to
   ``Player`` (the "wildcard" that lives in Python, expanded to N explicit ``{ship, mode}`` leaves);
3. a **statistical policy**: compare ``upkeep`` against ``production_value`` from the round's
   ``view`` in the projection and cap the construction budget of every faction that is
   over-extended;
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
import json
import shutil
import sys
import tempfile
from pathlib import Path

import pandas as pd

import planet_x_ctl as ctl

# ---- policy knobs (the recipe's own choices, written down so it can be replayed) -------------
OVEREXTENDED_RATIO = 0.5   # upkeep / production_value above this = 养不起的舰队
#: 紧缩时给**全舰队**下的一条即时指令（逐舰点名）——2026-10 起指令**没有**"舰队默认叶"了
#: （指令是即时操作），所以这条配方把同一句话写给每一艘舰（`order()` 的舰队选择器展开成 N 片叶）。
DEFAULT_ORDER_BEHAVIOR = "Dock:地球"
#: 紧缩时一起收缩的两类预算（造舰 + 建设）。引擎把两类预算做成了同一个形状，所以同一条配方
#: 能一次管住两边；`construction_budget` 常在 AI 手里是 0，只有建设预算在动，所以两类都要看。
BUDGET_KINDS = ("建造预算", "投资预算")

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


#: The columns a projection written by an engine **before** the blueprint round does not have
#: (`SCHEMA_VERSION` 10 加的 `order_effective*` / `order_source` / `blueprint*` / `spawned_round`)。
#: 删掉它们 = 一份**旧引擎**的索引目录，用来演一遍 kit 的降级路径（本地 `*_approx`）。
#: ⚠ `order_default_mode` / `order_blueprint_mode` 两列**已经不在引擎里**（2026-10：指令只剩逐舰叶），
#: 所以它们不在这张单子上——那不是"旧引擎缺的列"，是"没有这个列了"。
_OLD_ENGINE_DROPS = ("order_effective_mode", "order_effective", "order_source",
                     "order_leaf_mode",
                     "风格", "姿态", "角色", "role_mode",
                     "出厂图", "blueprint_mode", "下水回合")


def _old_engine_index(src, dst) -> Path:
    """A copy of ``src`` with the post-blueprint engine columns stripped out of ``idx/ships.jsonl``.

    这就是「旧引擎写的索引目录」的本体：`ships()` 认不出 ``order_effective_mode``，于是退回
    本地近似并**把列名标成 `_approx`**、把来源布尔列置 False。降级路径没人跑过就会悄悄烂掉，
    所以这里**真的**造一份出来跑。
    """
    dst = Path(dst)
    if dst.exists():
        shutil.rmtree(dst)
    shutil.copytree(src, dst)
    p = dst / "idx" / "ships.jsonl"
    rows = []
    for line in p.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        row = json.loads(line)
        rows.append(json.dumps({k: v for k, v in row.items() if k not in _OLD_ENGINE_DROPS},
                               ensure_ascii=False))
    p.write_text("\n".join(rows) + "\n", encoding="utf-8")
    return dst


def economy_table(proj_dir) -> pd.DataFrame:
    """Per-faction ``upkeep`` vs ``production_value``, straight out of the round ``view``.

    Read from a projection that came out of a **real run** (``--index``), not from re-projecting the
    checkpoint — see ``README`` §"过程量只在引擎真跑过的回合里才有". The process quantities
    (产出 / 维护 / 治理 / 贸易 / 判定) only exist for rounds the engine actually advanced; in a
    round's ``pre`` view they are 0/empty.
    """
    view = (ctl.projection(proj_dir).facts.iloc[-1].get("view") or {}).get("factions") or {}
    rows = []
    for fid, m in view.items():
        prod = float(m.get("production_value") or 0.0)
        up = float(m.get("upkeep") or 0.0)
        rows.append({"势力": fid, "production_value": prod, "upkeep": up,
                     "upkeep_ratio": (up / prod) if prod > 0 else float("nan"),
                     "ship_count": m.get("ship_count"), "city_count": m.get("city_count")})
    return pd.DataFrame(rows).sort_values("势力").reset_index(drop=True)


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

    再补一份**刻意的全舰队指令**（逐舰写 ``behavior`` + ``take_over=True``），让「一次说清全舰队
    干什么」有一个去处 —— 这份 diff 的回执因此应当**恰好**只有那 N 处接管（一艘一处）。
    ⚠ 2026-10 起指令**没有**势力级默认叶了，所以"全舰队"必须**逐舰点名**（`set_behavior` 收
    一个舰名选择器，展开成 N 片叶）。
    """
    s = surface if surface is not None else ctl.surface(ckpt, index_dir=proj_dir)
    eco = economy_table(proj_dir)
    target = eco[(eco["production_value"] > 0) & (eco["upkeep_ratio"] > OVEREXTENDED_RATIO)]
    rows, takeover_faction = [], None
    for _, f in target.sort_values("势力").iterrows():
        fac = f["势力"]
        scale = max(0.0, min(1.0, 1.0 - f["upkeep_ratio"]))
        for kind in BUDGET_KINDS:
            current = {lf.key[0]: float(lf.value or 0.0)
                       for lf in s.leaves(kind) if lf.faction == fac}
            # 只写真的会变的数字 —— 写一个等值的数只是 no-op（第 6 节专门演示这一点）。
            new = {res: round(v * scale, 4) for res, v in sorted(current.items())
                   if abs(v * scale - v) > 1e-9}
            for res, val in sorted(new.items()):
                rows.append({"势力": fac, "kind": kind, "resource": res,
                             "upkeep_ratio": round(f["upkeep_ratio"], 3), "scale": round(scale, 3),
                             "old": current[res], "new": val})
            if new:
                s.set_budget(fac, kind, new, mode=ctl.PLAYER)
        if takeover_faction is None and int(f["ship_count"] or 0) > 0:
            takeover_faction = fac
    if takeover_faction is not None:
        # 唯一一处刻意接管：**逐舰**把同一句话写给全舰队（指令没有舰队默认叶了，所以这是 N 片叶），
        # 不写 mode 就等于接管 —— 这里明说 take_over=True，好让回执里的接管名单是**已知**的。
        fleet = sorted(lf.key[0] for lf in s.leaves("指令")
                       if lf.faction == takeover_faction)
        s.set_behavior(fleet, DEFAULT_ORDER_BEHAVIOR, take_over=True)
    cols = ["势力", "kind", "resource", "upkeep_ratio", "scale", "old", "new"]
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
          f"--index proj/ --save ckpt.json")
    proj = work / "proj"
    ckpt = Path(ctl.new_checkpoint(work / "ckpt.json", seed=args.seed, rounds=args.round,
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
          {"order_leaf", "order_mode", "order_value", "default_role_mode",
           "doctrine_temper", "doctrine_lone_wolf", "kiting"} <= set(df_ships.columns))
    # 设计图那一轮新增的引擎列（**引擎的答案**，不是本地近似）：出处列 + 出厂图 + 下水回合。
    check("ships 表带上了设计图/出处/下水回合列",
          {"出厂图", "blueprint_mode", "order_source", "下水回合"}
          <= set(df_ships.columns),
          f"order_source={df_ships['order_source'].dropna().unique()[:3].tolist()}…")

    # **有效指令 = 引擎的答案**（本轮修的洞）。以前这三列是 kit 在 Python 里**重算**的
    # （`*_approx`），而那条链**没有设计图层**（`叶 → 出厂图 → 舰队默认 → …`），所以对"按图造的舰"
    # 给出的是**错的**答案。现在直接读引擎自己的 `order_effective_mode` / `order_effective` /
    # `order_source`（`State::ship_control` / `ship_behavior` / `ship_behavior_source`）。
    check("有效指令列来自**引擎**（含设计图层），不是本地重算",
          bool(df_ships[ctl.EFFECTIVE_PROVENANCE_COLUMN].all())
          and all(c in df_ships.columns for c in ctl.ENGINE_EFFECTIVE_COLUMNS)
          and list(df_ships["effective_order_mode"]) == list(df_ships["order_effective_mode"])
          and list(df_ships["effective_order_value"])
          == [ctl.behavior_str(v) for v in df_ships["order_effective"]],
          f"{ctl.EFFECTIVE_PROVENANCE_COLUMN}=True，{len(df_ships)} 行与引擎列逐值相同")
    check("引擎列在时**没有** `_approx` 列（两种来源不穿同一件衣服）",
          not any(c in df_ships.columns for c in ctl.APPROX_COLUMNS),
          f"缺席：{list(ctl.APPROX_COLUMNS)}")

    # 降级路径：旧引擎写的索引目录**没有**那几列 ⇒ 退回本地近似，**列名带 `_approx`** +
    # 布尔列标明来源。这条路径仍然通（模拟法：把本轮那几列从 ships 表里删掉），而且在这一帧里
    # 两套链给出的结论一致——**因为这一帧里没有任何"按图造的舰"**：本地近似缺的正是蓝图层那一格
    # （`ship-blueprint.md` §6，`README` 第 8 条）。
    df_old = ctl.ships(ckpt, index_dir=_old_engine_index(proj, work / "proj_oldengine"))
    check("旧索引目录 ⇒ 退回本地近似，且读面说得出「这一帧是谁算的」",
          not any(df_old[ctl.EFFECTIVE_PROVENANCE_COLUMN])
          and all(c in df_old.columns for c in ctl.APPROX_COLUMNS)
          and not any(c in df_old.columns for c in ctl.ENGINE_EFFECTIVE_COLUMNS),
          f"{ctl.EFFECTIVE_PROVENANCE_COLUMN}={sorted(set(df_old[ctl.EFFECTIVE_PROVENANCE_COLUMN]))}")
    check("降级路径与引擎在**这一帧**给出同样的结论（差别只在蓝图层，而这一帧没有按图造的舰）",
          list(df_old["effective_order_mode_approx"])
          == list(df_ships["effective_order_mode"])
          and list(df_old["effective_order_value_approx"])
          == list(df_ships["effective_order_value"]),
          f"{len(df_old)} 行逐值相同")

    check("--control-schema 里有 blueprints（agent 才知道能往 --apply 写什么）",
          "BlueprintPatch" in schema.get("definitions", {}),
          f"{len(schema.get('definitions', {}))} 个定义")

    counts = df_ships.groupby("势力").size().sort_values(ascending=False)
    faction = sorted(counts[counts == int(counts.max())].index)[0]   # 并列时按名字定序
    names = sorted(df_ships.loc[df_ships["势力"] == faction, "舰名"].tolist())
    print(f"    施政对象：{faction}（{len(names)} 艘：{'、'.join(names)}）")

    # 编制表：slot 名是意图，活过名字换代。刷新规则写在配方里（排序键），不留在脑子里。
    # 注意：舰级列现在叫 `舰级`（引擎的字段名就是给人看的名词），不再是 Python 关键字。
    fleet = ctl.query(df_ships, "势力 == @faction")
    flagship_hull = float(fleet["船体上限"].max())
    classes = (fleet.groupby("舰级")["船体上限"].max()
               .sort_values(ascending=False).index.tolist())          # 决定性顺序
    spec = [("旗舰", f"势力 == '{faction}'")] + [
        (f"{cls} 队", f"势力 == '{faction}' and 舰级 == '{cls}'") for cls in classes[:2]]
    ros = ctl.roster(ckpt, spec, index_dir=proj)
    print(ros[["slot", "matched", "candidates", "舰名", "舰级", "船体", "船体上限",
               "refresh_rule"]].to_string(index=False))
    check("编制表把每个 slot 映射到现役舰（刷新规则写在配方里）",
          bool(ros["matched"].all()) and ros["slot"].tolist() == [s for s, _ in spec])
    check("编制表槽位是确定的（同分：**最老的先**，再名字序）",
          ros.iloc[0]["舰名"]
          == fleet.sort_values(["船体", "船体上限", "下水回合", "舰名"],
                               ascending=[False, False, True, True],
                               na_position="last").iloc[0]["舰名"],
          f"旗舰={ros.iloc[0]['舰名']}，最高 hull={flagship_hull:g}")

    # **「同分取最老的」真的成立了吗**（Q9 的目的：`Ship.spawned_round` 进投影 ships 表）。
    # 光有「编制表选中的 == 最老的」还不够——如果那一组里"最老的"恰好也是名字序第一，
    # 这条断言什么都没证明。所以要**构造一对「同分（hull/hull_max 相同）、年龄不同、
    # 且名字序会挑另一艘」**的舰：只有这时"取最老的"与"取名字序"给出不同答案。
    def _tie_pairs(fixture, idx_dir=None):
        d = ctl.ships(fixture, index_dir=idx_dir)
        out = []
        key = d["船体"].astype(str) + "/" + d["船体上限"].astype(str)
        for _, g in d.groupby(key):
            known = g[g["下水回合"].notna()]
            if len(known) >= 2 and known["下水回合"].nunique() >= 2:
                srt = known.sort_values(["下水回合", "舰名"])
                old, new = srt.iloc[0], srt.iloc[-1]
                if str(old["舰名"]) > str(new["舰名"]):   # 名字序会挑 new ⇒ 这一对能证明规则
                    out.append((str(old["舰名"]), str(new["舰名"]),
                                int(old["下水回合"]), int(new["下水回合"])))
        return out

    tie_ckpt, tie_dir, pairs = ckpt, proj, _tie_pairs(ckpt, proj)
    if not pairs:
        # 12 回合的 fixture 里可能只有开局舰队（全是第 0 回合下水）⇒ 跑长一点再找。
        tie_ckpt = Path(ctl.new_checkpoint(work / "ckpt_tie.json", seed=args.seed,
                                          rounds=max(args.round, 60), planet_x=engine))
        tie_dir, pairs = None, _tie_pairs(tie_ckpt)
    print(f"    同分且年龄不同的对：{pairs[:3]}{'…' if len(pairs) > 3 else ''}"
          f"（fixture={Path(tie_ckpt).name}）")
    check("构造出「同分 / 年龄不同 / 名字序会挑错」的一对（守卫不许空转）", bool(pairs),
          f"{len(pairs)} 对")
    if pairs:
        old_id, new_id, old_r, new_r = pairs[0]
        r_tie = ctl.roster(tie_ckpt, [("配对", f"舰名 in ['{old_id}', '{new_id}']")],
                           index_dir=tie_dir)
        check("同分取**最老的**（spawned_round 升序），而不是名字序",
              r_tie.iloc[0]["舰名"] == old_id,
              f"选中={r_tie.iloc[0]['舰名']}（最老={old_id}@r{old_r}；"
              f"名字序会给={new_id}@r{new_r}；规则={r_tie.iloc[0]['refresh_rule']}）")

    # ---------------------------------------------------------------- 2. bulk ownership → Auto
    print("\n[2] 批量归属：整队 → Auto（引擎没有通配，这里展开成 N 条只写 mode 的叶）")
    diff_a = recipe_ownership(ckpt, names, ctl.AUTO, index_dir=proj)
    path_a = ctl.write(diff_a, work / "steer_a.json")
    print(f"    {Path(path_a).name}：{faction} 的 "
          f"{len(diff_a['control'][0]['指令'])} 条叶，每条只有 舰+归属")
    rep_a = ctl.verify(ckpt, path_a)
    print(rep_a.describe())
    expected_a = {(f"{faction}.指令[{n}]", "归属") for n in names}
    check("A: 没有 WARN_APPLY_SKIPPED",
          not rep_a.skipped and "WARN_APPLY_SKIPPED" not in rep_a.stderr_text)
    check("A: 一处接管都没有", rep_a.took_over == [] and rep_a.incidental == [])
    check("A: 请求的叶全部落地", all(r.landed for r in rep_a.requests), f"{len(rep_a.requests)} 条")
    check("A: 写面只碰了 mode（没碰任何值）",
          {(r.leaf, r.field) for r in rep_a.requests} == expected_a)
    check("A: 读面确认全队已是 Auto",
          {r.after for r in rep_a.requests if r.field == "归属"} == {ctl.AUTO})
    check("A: 叶上记录的值逐条未动",
          {n: rep_a.before.leaf(faction, "指令", n).value for n in names}
          == {n: rep_a.after.leaf(faction, "指令", n).value for n in names})

    # 真的落地（--save），证明这不是只在内存里演一遍
    ckpt_a = work / "ckpt_a.json"
    app = ctl.apply(ckpt, path_a, save=ckpt_a)
    check("A: 真实 --apply --save 成功（回执无 skipped）",
          app.ok and not app.skipped and ckpt_a.exists())
    check("A: 存下来的 checkpoint 真的持有 Auto",
          all(ctl.surface(ckpt_a).leaf(faction, "指令", n).mode == ctl.AUTO for n in names))

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
          {r.after for r in rep_b.requests if r.field == "归属"} == {ctl.PLAYER})

    # ---------------------------------------------------------------- 4. statistical policy
    print(f"\n[4] 统计施政：upkeep/production_value > {OVEREXTENDED_RATIO} 的势力，"
          f"{' / '.join(BUDGET_KINDS)} 按 (1 − 占比) 等比封顶")
    print(economy_table(proj).to_string(index=False))
    diff_c, table = recipe_policy(ckpt, proj)
    path_c = ctl.write(diff_c, work / "steer_c.json")
    print()
    print(table.to_string(index=False) if len(table) else "    （没有势力超标）")
    # 刻意接管：**逐舰**写 behavior ⇒ 每艘舰的那片叶各算一处接管（N 片叶）。
    # ⚠ 2026-10 起指令**没有**舰队默认叶了（`set_default_ship_order` 已删）："全舰队听我的"
    # 现在展开成 N 片逐舰叶，而不是"一片叶 + 一条链上的默认"。
    _fac_c = next((e["势力"] for e in diff_c["control"]
                   if any("行为" in o for o in e.get("指令", []))), None)
    _n_written = sum(1 for o in next(e["指令"] for e in diff_c["control"]
                                     if e["势力"] == _fac_c) if "行为" in o)
    print(f"    刻意接管：{_fac_c} 全舰队 {_n_written} 艘（逐舰写 behavior ⇒ {_n_written} 处接管）")
    rep_c = ctl.verify(ckpt, path_c)
    print(rep_c.describe())
    check("C: 没有 WARN_APPLY_SKIPPED",
          not rep_c.skipped and "WARN_APPLY_SKIPPED" not in rep_c.stderr_text)
    check("C: 请求的叶全部落地", all(r.landed for r in rep_c.requests),
          f"{len(rep_c.requests)} 条，未落地="
          f"{[f'{r.leaf}.{r.field}' for r in rep_c.failed_requests]}")
    check("C: 预算值真的变了",
          bool(table is not None) and all(r.changed for r in rep_c.requests
                                          if r.field == "值" and "预算" in r.leaf))
    check("C: took_over 恰好是那 N 片逐舰指令叶",
          len(rep_c.took_over) == _n_written
          and all(p.startswith(f"{_fac_c}.指令[") for p in rep_c.took_over),
          f"{len(rep_c.took_over)} 处：{rep_c.took_over[:3]}…")
    check("C: took_over_leafs 也一一对应（叶名 = 势力.指令[舰名]）",
          len(rep_c.took_over_leafs) == _n_written
          and all(p.startswith(f"{_fac_c}.指令[") and not p.endswith("].行为")
                  for p in rep_c.took_over_leafs),
          f"{rep_c.took_over_leafs[:3]}…")
    # 顺带变动 = 「那片叶从无到有」（`exists` 翻转）**或**隐含的 `mode` 翻转 —— 船坞下水时就给
    # 每艘舰写过一片 `Inherit` 的叶，所以这里通常只看到 `mode` 翻转。
    # ⚠ 断言里不要拿 `c.after` 当集合元素：`behavior` 是个 dict，塞进 set 会 TypeError。
    _leaf_flips = {(c.leaf, c.field, json.dumps(c.after, ensure_ascii=False, sort_keys=True))
                   for c in rep_c.incidental if c.leaf in rep_c.took_over_leafs}
    check("C: 顺带变动只有「那片叶的 mode 翻转成 Player」（值翻转不是顺带变动，是我们请求的）",
          all(f == "归属" and v == json.dumps(ctl.PLAYER) for (_, f, v) in _leaf_flips),
          f"{sorted(_leaf_flips)[:4]}…")
    # 每片叶里躺着的就是那句话本身——而且从 2026-10 起**叶就是唯一供值者**（指令链上没有
    # 舰队默认叶、也没有图上意图，见 `State::ship_behavior`）⇒ 叶里的值**就是**有效值。
    # 所以这条配方不需要再"等一回合看默认生效"。
    _written = next(o["行为"] for e in diff_c["control"] if e["势力"] == _fac_c
                    for o in e["指令"] if "行为" in o)
    _ships_written = [o["舰"] for e in diff_c["control"] if e["势力"] == _fac_c
                      for o in e["指令"] if "行为" in o]
    check("C: 每艘舰叶里的值就是那句话（叶是唯一供值者 ⇒ 也就是有效值）",
          bool(_ships_written) and all(
              rep_c.after.leaf(_fac_c, "指令", n).value == _written
              for n in _ships_written),
          f"{len(_ships_written)} 艘 = {ctl.behavior_str(_written)}")

    # ---------------------------------------------------------------- 4b. the third style axis
    print("\n[4b] 第三条风格轴**角色**（战舰 War / 运输舰 Freight / 观测舰 Observe）："
          "写值即接管 · AI 定编的闸门 · 删叶 = 交回定编")
    hero = names[0]
    s_r = ctl.surface(ckpt, index_dir=proj)
    check("角色轴进得了 surface()（读面每艘舰一行 `角色`）",
          all(s_r.leaf(faction, "角色", n).value is not None for n in names),
          f"{len(names)} 艘")
    # ① 写值即接管：这艘舰归玩家，自动控制的逐舰定编从此不碰它。
    #    ⚠ 值是三值字符串（`War`/`Freight`/`Observe`），不是旧版的 True/False。
    s_r.set_role(hero, "Freight", take_over=True)
    path_r = ctl.write(s_r.emit(), work / "steer_r.json")
    rep_r = ctl.verify(ckpt, path_r)
    print(rep_r.describe())
    check("R: 写角色值 = 一次接管（回执指的就是那片叶）",
          rep_r.took_over_leafs == [f"{faction}.角色[{hero}]"], f"{rep_r.took_over_leafs}")
    check("R: 值真的写成了运输舰",
          rep_r.after.leaf(faction, "角色", hero).value == "Freight",
          f"{rep_r.after.leaf(faction, '角色', hero).value!r}")

    # 真的落地：verify 只是演习，而删叶必须对着「那片叶真的在」的 checkpoint 来。
    ckpt_r = work / "ckpt_r.json"
    app_r = ctl.apply(ckpt, path_r, save=ckpt_r)
    check("R: --apply --save 成功，checkpoint 里这艘舰的角色已归玩家",
          app_r.ok and not app_r.skipped
          and ctl.surface(ckpt_r).leaf(faction, "角色", hero).mode == ctl.PLAYER,
          f"{ctl.surface(ckpt_r).leaf(faction, '角色', hero)}")

    # ② 删叶 = **交回自动定编**（不是"锁成某个值"）。逐舰叶的存在性读面看不出来，
    #    所以"落地了没有"以引擎的 NOTE_APPLY_REMOVED 回执为准。
    #    ⚠ `verify` 是**只读演习**：要验证"再删一次是幂等的"，必须先把第一次删叶真的
    #    apply 下来——否则第二次删的仍然是"那片叶还在"的 checkpoint（这个坑笔记里记过，
    #    这里当场演示一遍）。
    s_r2 = ctl.surface(ckpt_r)
    s_r2.remove_role(hero)
    path_r2 = ctl.write(s_r2.emit(), work / "steer_r2.json")
    rep_r2 = ctl.verify(ckpt_r, path_r2)
    print(rep_r2.describe())
    check("R: 删叶在引擎回执里出现（读面看不出来，靠回执）",
          rep_r2.removed_leafs == [f"{faction}.角色[{hero}]"], f"{rep_r2.removed_leafs}")
    ckpt_r2 = work / "ckpt_r2.json"
    check("R: 删叶真的落地（--apply --save：这一步之后那片叶才真的没了）",
          ctl.apply(ckpt_r, path_r2, save=ckpt_r2).ok)
    rep_r3 = ctl.verify(ckpt_r2, ctl.surface(ckpt_r2).remove_role(hero).emit())
    check("R: 再删同一片叶 = 幂等成功（不进回执、也不算丢弃）",
          rep_r3.ok and rep_r3.removed_leafs == [] and not rep_r3.skipped, str(rep_r3.removed_leafs))

    # ③ 势力级默认角色叶：一片叶管住全舰队；`Player` 就是「AI 定编别碰我的舰队」那道闸门。
    #    这里用第三态 `Observe`——它是本轮新加的那一档（观测舰去异常区蹲着喂 MOND 掌握度）。
    s_r3 = ctl.surface(ckpt_r)
    s_r3.set_default_role(faction, "Observe", mode=ctl.PLAYER)
    path_r3 = ctl.write(s_r3.emit(), work / "steer_r3.json")
    rep_r4 = ctl.verify(ckpt_r, path_r3)
    d_leaf = rep_r4.after.leaf(faction, "舰队默认角色")
    check("R: 势力级默认角色叶建成（读面里出现，值是玩家钉的观测舰）",
          d_leaf.value == "Observe" and d_leaf.mode == ctl.PLAYER, f"{d_leaf}")
    ckpt_r3 = work / "ckpt_r3.json"
    check("R: 势力级默认叶真的落地", ctl.apply(ckpt_r, path_r3, save=ckpt_r3).ok)
    rep_r5 = ctl.verify(ckpt_r3, ctl.surface(ckpt_r3).remove_default_role(faction).emit())
    check("R: 删势力级默认角色叶也在回执里（`exists` 由 True 翻回 False）",
          rep_r5.removed_leafs == [f"{faction}.舰队默认角色"], f"{rep_r5.removed_leafs}")

    # ---------------------------------------------------------------- 4c. 舰船设计图
    print("\n[4c] 设计图**blueprint**（「还不存在的舰」的出厂规格）：建图 · 建造区指针 · 删图")
    bld = ctl.buildings(ckpt_r3, index_dir=proj)
    check("建造区表带上了设计图指针列", "blueprint" in bld.columns)
    yards = bld[(bld["势力"] == faction) & (bld["kind"] == "construction")]
    check("施政对象有建造区可挂图", len(yards) > 0, f"{len(yards)} 个建造区")
    if len(yards):
        y = yards.iloc[0]
        ycity, ybid, ycls = y["city"], int(y["building"]), y["ship_type"]
        # ① 建图 + **把图与建造区的舰级一起写**（口径 A 的正解：只改一处会被引擎拒）。
        # ⚠ 图带的是**长期倾向**（角色 / 风格 / 风筝姿态），不是指令（2026-10 用户裁决）：
        # 这张"重甲护卫"图表态 **角色 = 战舰**（它的活就是找仗打）。
        s_bp = ctl.surface(ckpt_r3, index_dir=proj)
        s_bp.set_blueprint_and_retool(faction, "重甲护卫", class_=ycls, city=ycity, building=ybid,
                                      components=["kinetic", "ion_drive"], role="War",
                                      mode=ctl.PLAYER)
        path_bp = ctl.write(s_bp.emit(), work / "steer_bp.json")
        print(f"    载荷：建图「重甲护卫」({ycls}) + 把 {ycity}/{ybid} 指过去")
        rep_bp = ctl.verify(ckpt_r3, path_bp)
        print(rep_bp.describe())
        check("BP: 建图 + 挂指针一次成功、无丢弃", rep_bp.ok and not rep_bp.skipped,
              f"{rep_bp.skipped}")
        lf_bp = rep_bp.after.leaf(faction, "设计图库", "重甲护卫")
        check("BP: 图叶的值是**复合**的（class + components + 倾向三轴）",
              (lf_bp.value or {}).get("舰级") == ycls
              and (lf_bp.value or {}).get("选装") == ["kinetic", "ion_drive"]
              and (lf_bp.value or {}).get("角色") == "War"
              and (lf_bp.value or {}).get("风格") is None,
              f"{lf_bp.value}")
        check("BP: 显式 mode=Player ⇒ 这张图归玩家、且**没有**意外接管",
              lf_bp.mode == ctl.PLAYER and rep_bp.took_over_leafs == [],
              f"mode={lf_bp.mode} took_over={rep_bp.took_over_leafs}")

        # 真的落地（verify 只是只读演习）：读面/投影里必须看得见指针。
        ckpt_bp = work / "ckpt_bp.json"
        check("BP: --apply --save 成功", ctl.apply(ckpt_r3, path_bp, save=ckpt_bp).ok)
        # ⚠ 换了一个 checkpoint 就要**重新投影**：`index_dir=` 是「直接用这个目录」，
        # 传上一份 ckpt 的投影目录会让读面全是旧值（本 demo 也踩过：指针显示 None）。
        got = ctl.buildings(ckpt_bp)
        row = got[(got["city"] == ycity) & (got["building"] == ybid)].iloc[0]
        check("BP: 建造区的**指针**落到了这张图上（重新投影后看得见）", row["blueprint"] == "重甲护卫",
              f"{row['blueprint']}")

        # ② 蓝图表的读面（`q.blueprints(round)`）：一行一图 + **引擎算的**列。
        q = ctl.projection(ckpt_bp)
        bp_df = q.blueprints(round=ctl._last_round(q))
        check("BP: 投影蓝图表可读（q.blueprints 泛化自 schema.derived）",
              len(bp_df) >= 1 and {"effective_mode", "class_slots", "component_cost",
                                   "launch_waiting", "ship_count"} <= set(bp_df.columns),
              f"{bp_df.shape}")
        mine = bp_df[bp_df["图名"] == "重甲护卫"].iloc[0]
        slots = int(q.ships_spec().loc[ycls, "slots"])
        check("BP: 有效归属由**引擎**解析（不是本地近似）",
              mine["effective_mode"] == ctl.PLAYER and mine["mode"] == ctl.PLAYER
              and int(mine["class_slots"]) == slots,
              f"effective_mode={mine['effective_mode']} slots={mine['class_slots']}（配置表 {slots}）")
        # ⚠ 蓝图表（派生表）的列名已是中文名词：`角色` / `风格` / `姿态`。
        check("BP: 图上写了角色 ⇒ 蓝图表看得见它；没写的两条轴仍是 null（链继续下降到舰队默认）",
              mine["角色"] == "War" and pd.isna(mine["风格"]) and pd.isna(mine["姿态"]),
              f"role={mine['角色']!r} doctrine={mine['风格']!r} kiting={mine['姿态']!r}")
        check("BP: 组件成本 / 造过多少艘是引擎算的派生列",
              float(mine["component_cost"].get("铁", 0.0)) > 0 and int(mine["ship_count"]) >= 0,
              f"component_cost={dict(mine['component_cost'])} ship_count={mine['ship_count']}")

        # ③ 拆指针 = 回到自动选装（写的是 `null`，不是"缺席"：缺席 = 不动这一格）。
        s_bp2 = ctl.surface(ckpt_bp)
        s_bp2.set_blueprint_pointer(faction, ycity, ybid, None)
        path_bp2 = ctl.write(s_bp2.emit(), work / "steer_bp2.json")
        rep_bp2 = ctl.verify(ckpt_bp, path_bp2)
        check("BP: 拆指针（null）落地", rep_bp2.ok and not rep_bp2.skipped)
        ckpt_bp2 = work / "ckpt_bp2.json"
        check("BP: 拆指针真的落地", ctl.apply(ckpt_bp, path_bp2, save=ckpt_bp2).ok)
        got2 = ctl.buildings(ckpt_bp2)
        # ⚠ `pd.isna` 而不是 `is None`：引擎给的确实是 JSON `null`（Python 侧就是 `None`），但
        # pandas 3 的 `str` dtype 会把「有字符串、也有缺值」的列统一成 `str` + `NaN` —— 于是这一格
        # 读出来是 `nan`（float），`is None` 恒 False。同一个坑这份 demo 在 `mine["doctrine"]` 那处
        # 已经用 `pd.isna` 绕过（见上面「图的意图轴默认沉默」那条）。
        check("BP: 指针回到「无」（= 走 ship_type + choose_loadout）",
              pd.isna(got2[(got2["city"] == ycity)
                           & (got2["building"] == ybid)].iloc[0]["blueprint"]))

        # ④ 删图（改名/换代的正规路径）：回执点名到叶。⚠ 删图之后挂它的区是**悬空指针 ⇒ 停产**
        #    （进度不再增加），所以正确用法是**先拆指针再删图**——读面会把悬空指针原样输出。
        s_bp3 = ctl.surface(ckpt_bp2)
        s_bp3.remove_blueprint(faction, "重甲护卫")
        path_bp3 = ctl.write(s_bp3.emit(), work / "steer_bp3.json")
        rep_bp3 = ctl.verify(ckpt_bp2, path_bp3)
        check("BP: 删图在引擎回执里（读面看不出来，靠回执）",
              rep_bp3.removed_leafs == [f"{faction}.设计图库[重甲护卫]"], f"{rep_bp3.removed_leafs}")

        # ⑤ 质量栏：图的舰级与建造区对不上 ⇒ 引擎**响亮**拒绝（口径 A），绝不静默。
        other = next((c for c in sorted(set(df_ships["舰级"])) if c != ycls), None)
        if other:
            s_bp5 = ctl.surface(ckpt_bp2)
            s_bp5.set_blueprint(faction, "错级图", class_=other, components=[], mode=ctl.PLAYER)
            s_bp5.set_blueprint_pointer(faction, ycity, ybid, "错级图")
            rep_bp5 = ctl.verify(ckpt_bp2, ctl.write(s_bp5.emit(), work / "steer_bp5.json"))
            codes = [s["code"] for s in rep_bp5.skipped]
            check("BP: 图的舰级与建造区对不上 ⇒ 引擎报 blueprint_class_mismatch（建造区侧）",
                  "blueprint_class_mismatch" in codes, f"{codes}（图={other}，区={ycls}）")
            # **另一侧**的同一道守卫：在还挂着图的 checkpoint 上只改**图的** `class` ⇒ 报在图上，
            # 而且路径要从列表下标映射回**叶名**（`中国.blueprints[重甲护卫]`）——新 kind 走的是
            # 同一条正则 + 新 diff 里的 `name`（`_engine_path_to_leaf`）。漏了它，报告里就只有
            # 一行看不懂的下标（`中国.blueprints[0].class`）。
            s_bp7 = ctl.surface(ckpt_bp)
            s_bp7.set_blueprint(faction, "重甲护卫", class_=other, components=["kinetic"],
                                mode=ctl.PLAYER)
            rep_bp7 = ctl.verify(ckpt_bp, ctl.write(s_bp7.emit(), work / "steer_bp7.json"))
            check("BP: 只改图的舰级同样被拒（图侧，双向守卫的第二条路）",
                  any(s["code"] == "blueprint_class_mismatch" for s in rep_bp7.skipped),
                  f"{[(s['code'], s.get('path')) for s in rep_bp7.skipped]}")
            check("BP: 丢弃路径映射回**叶名**（新 kind 走同一条正则）",
                  any(s.get("leaf") == f"{faction}.设计图库[重甲护卫]" for s in rep_bp7.skipped),
                  f"{[(s['code'], s.get('path'), s.get('leaf')) for s in rep_bp7.skipped]}")
            # 指向一张**不存在**的图：同样响亮（而且那个区会停产）——绝不静默回落生成器。
            s_bp6 = ctl.surface(ckpt_bp2)
            s_bp6.set_blueprint_pointer(faction, ycity, ybid, "这张图不存在")
            rep_bp6 = ctl.verify(ckpt_bp2, ctl.write(s_bp6.emit(), work / "steer_bp6.json"))
            check("BP: 悬空指针 ⇒ no_such_blueprint（不是静默回落生成器）",
                  any(s["code"] == "no_such_blueprint" for s in rep_bp6.skipped),
                  f"{[s['code'] for s in rep_bp6.skipped]}")

    # ---------------------------------------------------------------- 4d. leaf record = effective value
    print("\n[4d] 指令的「**叶里的值**」与「**有效值**」现在是**同一个东西**（叶是唯一供值者）")
    # 场景：给全舰队**逐舰**写一条 `Dock:月球`。三种读数必须互相印证：
    #   叶里的记录值（`order_behavior`）= 我们写进去的那句
    #   有效值（`effective_order_value`）= 同一句（2026-10 起指令链上没有更高的一层）
    #   谁供的值（`order_source`）    = `leaf`
    #
    # ⚠ 这条以前是**两件事**（叶里躺着一句旧记录、有效值由"舰队默认叶"供给，`order_source`
    # 报 `fleet_default`）——舰队默认那片叶与图上的 `order` 一起被裁决删除之后，这个二分不再存在。
    # 留着这一节是为了把**新**的不变式钉住：谁再往指令链上加一层"更高默认"，这里就红。
    s_lo = ctl.surface(ckpt)
    _fleet_lo = sorted(lf.key[0] for lf in s_lo.leaves("指令") if lf.faction == faction)
    check("LO: 这一势力有舰可写（否则这一节什么都没证明）", bool(_fleet_lo), f"{len(_fleet_lo)} 艘")
    s_lo.set_behavior(_fleet_lo, "Dock:月球", mode=ctl.PLAYER)
    ckpt_lo = work / "ckpt_leaforder.json"
    app_lo = ctl.apply(ckpt, ctl.write(s_lo.emit(), work / "steer_leaforder.json"), save=ckpt_lo)
    check("LO: --apply --save 成功（全舰队逐舰 = 玩家 + Dock 月球）", app_lo.ok and not app_lo.skipped)

    mine_lo = ctl.ships(ckpt_lo)
    mine_lo = mine_lo[mine_lo["势力"] == faction]
    check("LO: 每艘舰都有自己那片叶（投影 derived.control 里有它们的行）",
          bool(mine_lo["order_leaf"].all()), f"{int(mine_lo['order_leaf'].sum())}/{len(mine_lo)}")
    check("LO: `order_behavior` = 我们写进去的那句",
          set(mine_lo["order_behavior"]) == {"Dock:月球"},
          f"叶里={sorted(set(mine_lo['order_behavior']))}")
    check("LO: `effective_order_value` 与它**逐行相同**，`order_source` 一律 `leaf`",
          list(mine_lo["effective_order_value"]) == list(mine_lo["order_behavior"])
          and set(mine_lo["order_source"]) == {"leaf"},
          f"source={sorted(set(mine_lo['order_source']))}")

    # 再把其中一艘舰的叶**删掉**：`order_leaf` 必须翻成 False，而且**没有人接手**
    # （没有舰队默认叶、也没有图上意图）⇒ 有效值是空的，调用方按 `Idle` 兜底。
    gone = sorted(mine_lo["舰名"])[0]
    s_lo2 = ctl.surface(ckpt_lo)
    s_lo2.remove(faction, "指令", gone)
    ckpt_lo2 = work / "ckpt_leaforder2.json"
    check("LO: 删叶真的落地",
          ctl.apply(ckpt_lo, ctl.write(s_lo2.emit(), work / "steer_leaforder2.json"),
                    save=ckpt_lo2).ok)
    row_gone = ctl.ships(ckpt_lo2)
    row_gone = row_gone[row_gone["舰名"] == gone].iloc[0]
    check("LO: 叶被删过 ⇒ `order_leaf` 是 **False**、`order_behavior` 空、`order_mode` 回 Inherit",
          not bool(row_gone["order_leaf"]) and pd.isna(row_gone["order_behavior"])
          and row_gone["order_mode"] == ctl.INHERIT,
          f"{gone}: leaf={row_gone['order_leaf']} mode={row_gone['order_mode']} "
          f"behavior={row_gone['order_behavior']!r}")
    check("LO: 没有叶 ⇒ **没有任何一层说话**（有效值空、出处空；旧行为的「舰队默认接手」已不存在）",
          pd.isna(row_gone["effective_order_value"]) and pd.isna(row_gone["order_source"]),
          f"effective={row_gone['effective_order_value']!r} source={row_gone['order_source']!r}")
    check("LO: 别的舰照旧有自己的叶（只删了一艘）",
          int(ctl.ships(ckpt_lo2).query("势力 == @faction")["order_leaf"].sum()) == len(mine_lo) - 1)

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
          all(c.field != "值" for c in rep_n.changes)
          and {c.field for c in rep_n.changes} <= {"归属"})
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
                faction, "建造预算", {"不存在": 1.0}, mode=ctl.PLAYER),
            "不在已知表里")
    refuses("角色轴喂一个数字 → 当场拒绝（它是三值字符串枚举，不是数值轴）",
            lambda: ctl.surface(ckpt).set_role(names[:1], 1),
            "只接受")

    hand_written = {"control": [{"势力": faction, "指令": [
        {"舰": names[0], "归属": "Auto"}, {"舰": "方舟3", "归属": "Player"}]}]}
    rep_bad = ctl.verify(ckpt, hand_written)
    check("手写的陈旧叶：verify 只报 skipped、不假装成功",
          not rep_bad.ok and len(rep_bad.skipped) == 1 and rep_bad.skipped[0]["code"] == "no_such_ship",
          rep_bad.skipped[0]["code"] if rep_bad.skipped else "no skip reported")
    rep_err = ctl.verify(ckpt, {"control": [{"势力": faction, "ship_order": []}]})
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
    print(f"  R 角色轴     : requested={len(rep_r.requests)} took_over={len(rep_r.took_over)} "
          f"removed={len(rep_r2.removed_leafs)}+{len(rep_r5.removed_leafs)} "
          f"ok={rep_r.ok and rep_r2.ok and rep_r5.ok}")
    print(f"    封顶 {len(table)} 条预算，涉 {table['势力'].nunique() if len(table) else 0} 个势力；"
          f"逐舰下发指令 {_n_written} 处（全舰队听同一句话）")
    if len(table):
        print(f"    合计削减 {float((table['old'] - table['new']).sum()):.2f}"
              "（按各资源自身单位求和，仅作量级参考）")
    print(f"    逐舰下发的指令：{_n_written} 艘 ⇒ {ctl.behavior_str(_written)}")
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
