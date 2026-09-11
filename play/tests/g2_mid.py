"""中组（T2，49–480 回合）：`src/tests/sim/horizon_mid.rs` 里那几条**事件/快照可判**的机制不变量。

搬过来的（其余留在 Rust，理由见每组 doc）：

* **同回合自我复垦**：判据完全在事件层（`city_razed` + `colony_founded` 的先后与归属）；
* **定制化舰真的出现过**：判据在舰表（`hull > 0` + `components` 非空）；
* **剧情编年史**：节拍触发 / 顺序单调 / id 唯一 / 参与者具体——编年史就在主流里；
* **记恨地板的「真实长局」那一半**：把 `war_started`/`war_ended` 配对算每场战争的时长，
  与 `meta.json` 里的 `war_scar_rounds`/`war_scar_relation`/`war_threshold` 算出的最短回合比。
  （**形状**那一半——随年龄抬高、窗口内始终敌意、出了窗口消失、只属于那一对——要内部函数
  `war_scar_floor` 与手工世界，留在 Rust。）

⚠ 口径与 Rust 版**逐字相同**：种子 `[1, 7, 42]`、400 回合、拆平数 ≥ 20（防空转）、
「整局里出现过」而不是「末回合还剩着」、战争打完的场次 ≥ 5。

⚠ 两处**刻意不写死**（写死就是给下一个人埋雷）：RoundAt 节拍的清单从 `meta.json` 的
`story` 读（不写「prologue 在 1 回合、planet_x_arrives 在 60 回合」），疤痕的最短回合从
配置算（不写常量）。
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _harness import KIT, group_main  # noqa: E402

SEEDS = (1, 7, 42)
ROUNDS = 400
MIN_RAZINGS = 20          # 与 Rust 版同阈值：样本太小 ⇒ 守卫会空转，得报出来


def _named_entities(ev):
    """每回合「被事件命名过的实体」集合：`(kind, id)` —— 来自 actor / target / extra 三处。

    这是完备性审计的另一半：密集快照说「变了什么」，这里说「引擎自己说了什么原因」。
    """
    named: dict[int, set] = {}
    for r in ev.itertuples(index=False):
        s = named.setdefault(int(r.round), set())
        for kc, ic in (("actor_kind", "actor_id"), ("target_kind", "target_id")):
            k, i = getattr(r, kc), getattr(r, ic)
            if isinstance(k, str) and isinstance(i, str):
                s.add((k, i))
        for p in (r.extra if isinstance(r.extra, list) else []):
            if isinstance(p, dict) and isinstance(p.get("kind"), str) and isinstance(p.get("id"), str):
                s.add((p["kind"], p["id"]))
    return named


def reconcile_cities(q, ev):
    """**城的完备性审计**：密集快照里每一次「归属 / 存亡」变化，都要有事件命名这座城。

    这是「历史能证明自己什么都没丢」的那条证明（Rust 侧 `every_city_state_change_…`）。
    """
    named = _named_entities(ev)
    cities = q.table("cities")[["round", "city_id", "faction_id", "razed"]]
    prev: dict[str, tuple] = {}
    checked, unexplained = 0, []
    for rnd, grp in cities.groupby("round", sort=True):
        cur = {r.city_id: (r.faction_id or "", bool(r.razed)) for r in grp.itertuples(index=False)}
        for cid, now in cur.items():
            was = prev.get(cid)
            if was is not None and was != now:
                checked += 1
                if ("city", cid) not in named.get(int(rnd), ()):
                    unexplained.append(f"r{int(rnd)} 城 {cid}：{was} → {now}")
        prev = cur
    return checked, unexplained


def reconcile_ships(q, ev):
    """**舰的完备性审计**：出现 = 造舰事件、消失 = 死因事件（船表只在活着时写行）。"""
    ships = q.table("ships")[["round", "ship_id"]]
    alive: dict[int, set] = {int(r): set(g["ship_id"]) for r, g in ships.groupby("round")}

    def by_type(ty):
        out: dict[int, set] = {}
        for r in ev[ev["type"] == ty].itertuples(index=False):
            if isinstance(r.target_id, str):
                out.setdefault(int(r.round), set()).add(r.target_id)
        return out

    dead, born = by_type("ship_destroyed"), by_type("ship_spawned")
    deaths = births = 0
    unexplained: list[str] = []
    for r in range(1, (max(alive) if alive else 0) + 1):
        prev, cur = alive.get(r - 1, set()), alive.get(r, set())
        for sid in prev - cur:
            deaths += 1
            if sid not in dead.get(r, set()):
                unexplained.append(f"r{r} 舰 {sid} 消失但没有死因事件")
        for sid in cur - prev:
            births += 1
            if sid not in born.get(r, set()):
                unexplained.append(f"r{r} 舰 {sid} 出现但没有造舰事件")
    return deaths, births, unexplained


def owner_flips(ev):
    """**同回合归属翻转不变量**：一个回合内同一座城不能易主两次（净效果为零的振荡）。"""
    live = ("city_defected", "city_overrun", "colony_founded")
    flip = ev[ev["type"].isin(live)]
    bad = [f"r{int(rnd)} 城 {cid} 同回合易主 {len(g)} 次"
           for (rnd, cid), g in flip.groupby(["round", "target_id"]) if len(g) > 1]
    return len(flip), bad


def headline_check(ev):
    """**标题必须点到名**：事件命名的每个实体都要逐字出现在 `headline` 里。"""
    bad, checked = [], 0
    for r in ev.itertuples(index=False):
        h = r.headline if isinstance(r.headline, str) else ""
        parts = [getattr(r, ic) for ic in ("actor_id", "target_id") if isinstance(getattr(r, ic), str)]
        parts += [p["id"] for p in (r.extra if isinstance(r.extra, list) else [])
                  if isinstance(p, dict) and isinstance(p.get("id"), str)]
        for p in parts:
            checked += 1
            if p not in h:
                bad.append(f"r{int(r.round)} {r.type}：标题里没有「{p}」——{h[:60]}")
    return checked, bad


def combat_report(q, tol: float = 1e-9) -> dict:
    """**战斗/损伤的数据级判据**（`src/tests/sim/combat.rs` 里能只看数据的那几条）。

    逐发明细住在 `attack` 事件的 `data.shots`（B4 用户裁决：战斗中间量进事件层）——每条 =
    一件武器的一发，带选择输入与结算分解。加上舰表的 `hull/hull_max/hull_regen`，
    下面这些就能在**全 3 seed × 400 回合的每一行**上成立（Rust 版是「造一个世界、打一炮」）：

    ① **护甲再生恒等式**：没被打、也没欠费生锈的舰，
       `hull(t+1) == min(hull_max, hull(t) + hull_regen·hull_max)`；
    ② **面板域**：`0 < hull ≤ hull_max`、`0 ≤ shield ≤ shield_max`、`attack/speed/组件完整度 ≥ 0`；
    ③ **射击域**：`hit ∈ [0,1]`、`damage ≥ 0`、`damage > 0 ⇒ 在射程内且没被跳过`、
       `没命中 ⇒ 没伤害`、`没有点防 ⇒ 拦截量为 0`、`这一发击杀 ⇒ 伤害 ≥ 目标剩余船体`；
    ④ **击杀归属**（两个方向）：每一发 `killed` 的射击都要有对应的 `ship_destroyed` 事件，
       每条 `ship_destroyed` 也要有那一发——**沉舰这件事只能由射击解释**。
    """
    ships = q.table("ships")
    ev = q.table("events")
    fp = q.table("faction_process")
    out: dict = {"shots": 0}

    shot_bad: dict[str, list[str]] = {}
    killed_shot: set = set()
    # ⚠ 扣船体的是 **`hull_pen`**（过护盾/护甲之后真正打进船体的量），不是 `damage`
    # （`damage` = 过点防后的伤害，标题里那个「N 伤害」就是它；`hull_mult` 可以 > 1，
    # 所以一发 `hull_pen` 完全可能**大于** `damage`——我第一版用 `damage` 对账就假红了）。
    hull_lost: dict = {}
    kill_checked = 0
    kill_short: list[str] = []
    for r in ev[ev["type"] == "attack"].itertuples(index=False):
        rnd, tgt = int(r.round), r.target_id
        for sh in (r.data or {}).get("shots") or []:
            out["shots"] += 1
            dmg = float(sh.get("damage") or 0.0)
            pen = float(sh.get("hull_pen") or 0.0)
            if pen:
                hull_lost[(rnd, tgt)] = hull_lost.get((rnd, tgt), 0.0) + pen
            if sh.get("killed"):
                killed_shot.add((rnd, tgt))
                kill_checked += 1
                # **那一发的 `hull_pen` 必须够打掉它自己记下的「开火前船体」**——引擎就是
                # `hull -= hull_pen; destroyed = hull <= 0`，所以这条逐发可对账。
                # （① 别用 `damage`：`hull_mult` 可 > 1，`hull_pen` 能比 `damage` 大；
                #   ② 别用「上一回合末的船体」：改装的船下一回合 `hull_max` 就变了——
                #   实测 r86 莱茵4 上一回合 24.0、本回合按 12.0 的船体被打掉。）
                if pen + tol < float(sh.get("target_hull_before") or 0.0):
                    if len(kill_short) < 3:
                        kill_short.append(
                            f"r{rnd} {tgt}: 这一发打进 {pen:.4f} < 开火前船体 {sh.get('target_hull_before')}")
            why = None
            if not (0.0 <= float(sh.get("hit") or 0.0) <= 1.0):
                why = "命中折减越界"
            elif dmg < -tol or pen < -tol:
                why = "伤害为负"
            elif dmg > tol and (sh.get("skipped") or not sh.get("in_range")):
                why = "射程外/被跳过却掉血"
            elif float(sh.get("hit") or 0.0) <= tol and dmg > tol:
                why = "没命中却有伤害"
            elif float(sh.get("pd") or 0.0) <= tol and float(sh.get("pd_absorbed") or 0.0) > tol:
                why = "没有点防却拦截了"
            if why and len(shot_bad.get(why, [])) < 2:
                shot_bad.setdefault(why, []).append(f"r{rnd} {r.actor_id}→{tgt}: {why}")
    out["shot_bad"] = [m for v in shot_bad.values() for m in v][:4]
    out["shot_bad_kinds"] = len(shot_bad)

    # **只有战死的才必须有那一发**：欠费生锈报废（`upkeep_shortfall`）与战斗无关。
    dead = ev[ev["type"] == "ship_destroyed"]
    dead_cause = {(int(r.round), r.target_id): ((r.data or {}).get("cause") or "?")
                  for r in dead.itertuples(index=False)}
    dead_pairs = set(dead_cause)
    combat_dead = {k for k, c in dead_cause.items() if c == "combat"}
    out["dead"], out["combat_dead"] = len(dead_pairs), len(combat_dead)
    out["dead_causes"] = sorted({c for c in dead_cause.values()})
    out["kill_unexplained"] = [f"r{rnd} {sid}：战沉却没有一发 killed 射击"
                               for (rnd, sid) in sorted(combat_dead - killed_shot)][:3]
    out["kill_phantom"] = [f"r{rnd} {sid}：有 killed 射击却没有沉舰事件"
                           for (rnd, sid) in sorted(killed_shot - dead_pairs)][:3]

    ship_bad: list[str] = []
    for r in ships.itertuples(index=False):
        if not (0.0 < float(r.hull) <= float(r.hull_max) + tol):
            ship_bad.append(f"r{int(r.round)} {r.ship_id}: hull={r.hull} / max={r.hull_max}")
        elif not (-tol <= float(r.shield) <= float(r.shield_max) + tol):
            ship_bad.append(f"r{int(r.round)} {r.ship_id}: shield={r.shield} / max={r.shield_max}")
        elif float(r.attack) < -tol or float(r.speed) < -tol:
            ship_bad.append(f"r{int(r.round)} {r.ship_id}: attack={r.attack} speed={r.speed}")
        elif any(float(h) < -tol for h in (r.component_hp or [])):
            ship_bad.append(f"r{int(r.round)} {r.ship_id}: 组件完整度出现负数")
    out["ship_bad"] = ship_bad[:3]
    out["ship_bad_n"] = len(ship_bad)

    rust_round = {(int(r.round), r.faction_id) for r in fp.itertuples(index=False)
                  if float(r.fleet_rust or 0.0) > 0.0}
    per_round: dict = {}
    for r in ships.itertuples(index=False):
        per_round.setdefault(int(r.round), {})[r.ship_id] = r
    checked = regen_bad = regen_short = 0
    regen_ex: list[str] = []
    for rnd in sorted(per_round):
        prev = per_round.get(rnd - 1)
        if not prev:
            continue
        for sid, cur in per_round[rnd].items():
            was = prev.get(sid)
            if was is None or hull_lost.get((rnd, sid)) or (rnd, cur.faction_id) in rust_round:
                continue
            checked += 1
            c_max, c_regen = float(cur.hull_max), float(cur.hull_regen)
            base = min(c_max, float(was.hull) + c_regen * c_max)
            got = float(cur.hull)
            # ⚠ 只断言**下界 + 上限**，不断言等式：引擎的再生是
            # `hull + hull_max × (hull_regen + home_regen_bonus)`——**本土防御半径内还有一份加成**
            # （`sim/military.rs`），而那份加成读面不暴露（要看位置/首都/MOND，属于引擎内部）。
            # 下界仍然是真判据：安静回合里**至少**要长回基础再生量，且绝不超过上限。
            if got > c_max + 1e-9 or got + 1e-6 < base:
                regen_bad += 1
                if len(regen_ex) < 3:
                    regen_ex.append(f"r{rnd} {sid}: hull {was.hull} → {got}，基础再生至少应到 {base:.4f}"
                                    f"（上限 {c_max}）")
            elif got > base + 1e-6:
                regen_short += 1        # 拿到本土加成的行（计入防空转说明，不算违规）
    out["regen_checked"], out["regen_bad"], out["regen_ex"] = checked, regen_bad, regen_ex
    out["regen_bonus_rows"] = regen_short

    # —— 击杀所需伤害：**逐发**对账（见上面那一段注释），不再按回合累计 ——
    out["kill_checked"], out["kill_short"] = kill_checked, kill_short
    return out


def _nan(x) -> bool:
    return x is None or (isinstance(x, float) and x != x)


def blueprint_report(q) -> dict:
    """**设计图（blueprint）的数据级判据**（`src/tests/sim/blueprints.rs` + `autocontrol/blueprints.rs`）。

    读面上三处 join 就能验「**出厂快照**」这条语义：

    ① **快照不随时间变**：一条舰的 `components` 在它一生里恒定（改图不碰已下水的舰）；
    ② **出厂那一刻取自图**：`spawned_round` 的 `components` == 那张图**本回合或上一回合**的
       `components`——⚠ 两个都要认：同一回合里可能**先下水、后改图**（实测 9/76 例），
       而图那一行是**回合末**的状态，舰身上留的是**下水那一刻**的（这正是快照语义本身）；
    ③ **事件归因与表一致**：`ship_spawned.data.blueprint` == 该舰行上的 `blueprint`；
    ④ **`ship_count` 是算出来的**：图表的计数 == 该回合指向它的舰数；
    ⑤ **图按 `(舰级, 选装)` 去重**：同一势力同一回合不允许两张签名相同的图（防「设计图爆炸」）。
    """
    ships = q.table("ships")
    bps = q.table("blueprints")
    ev = q.table("events")
    out: dict = {}

    life: dict = {}
    for r in ships.itertuples(index=False):
        life.setdefault(r.ship_id, set()).add(tuple(r.components or []))
    drift = {k: v for k, v in life.items() if len(v) > 1}
    out["ship_kinds"] = len(life)
    out["drift_n"] = len(drift)
    out["drift"] = [f"{k}: {list(v)[:2]}" for k, v in list(drift.items())[:3]]

    design = {(int(r.round), r.faction_id, r.blueprint_id): r for r in bps.itertuples(index=False)}
    same = prev = neither = 0
    snap_ex: list[str] = []
    spawned = ships[ships["round"] == ships["spawned_round"]]
    for r in spawned.itertuples(index=False):
        if not r.blueprint:
            continue
        rnd, fid = int(r.round), r.faction_id
        cur = design.get((rnd, fid, r.blueprint))
        if cur is None or not (cur.components or []):
            continue                      # 这一回合图没快照 / 空选装 = 交给生成器
        got = {c for c in (r.components or [])}
        if got == set(cur.components):
            same += 1
        elif (before := design.get((rnd - 1, fid, r.blueprint))) is not None \
                and got == set(before.components or []):
            prev += 1
        else:
            neither += 1
            if len(snap_ex) < 3:
                snap_ex.append(f"r{rnd} {r.ship_id}: 舰上 {sorted(got)}，图（本回合）{sorted(cur.components)}")
    out["snap_same"], out["snap_prev"], out["snap_bad"], out["snap_ex"] = same, prev, neither, snap_ex

    attr: dict = {}
    for e in ev[ev["type"] == "ship_spawned"].itertuples(index=False):
        attr[(int(e.round), e.target_id)] = (e.data or {}).get("blueprint")
    mis: list[str] = []
    for r in spawned.itertuples(index=False):
        key = (int(r.round), r.ship_id)
        if key not in attr:
            continue
        # ⚠ pandas 把缺失值给成 NaN，事件里是 `null`/None ⇒ 比之前要归一（否则「None vs nan」假红）。
        mine = None if _nan(r.blueprint) else r.blueprint
        theirs = None if _nan(attr[key]) else attr[key]
        if mine != theirs and len(mis) < 3:
            mis.append(f"r{int(r.round)} {r.ship_id}: 事件说 {theirs}，表说 {mine}")
    out["attr_bad"], out["attr_checked"] = mis, len(attr)

    cnt: dict = {}
    for r in ships.itertuples(index=False):
        if r.blueprint:
            k = (int(r.round), r.faction_id, r.blueprint)
            cnt[k] = cnt.get(k, 0) + 1
    sc_bad: list[str] = []
    for r in bps.itertuples(index=False):
        k = (int(r.round), r.faction_id, r.blueprint_id)
        if int(r.ship_count or 0) != cnt.get(k, 0) and len(sc_bad) < 3:
            sc_bad.append(f"r{int(r.round)} {r.faction_id}/{r.blueprint_id}: 表说 {r.ship_count}，实为 {cnt.get(k, 0)}")
    out["count_bad"], out["bp_rows"] = sc_bad, len(bps)

    seen: dict = {}
    dup: list[str] = []
    # ⚠ `class` 是 Python 关键字，`itertuples` 会把它改名 ⇒ 这里用 `iterrows`。
    for _, r in bps.iterrows():
        sig = (int(r["round"]), r["faction_id"], r["class"], tuple(r["components"] or []))
        if sig in seen and len(dup) < 3:
            dup.append(f"r{sig[0]} {sig[1]}: {sig[2]} {list(sig[3])} 有两张图（{seen[sig]} / {r['blueprint_id']}）")
        seen[sig] = r["blueprint_id"]
    out["dup_bad"], out["dup_sigs"] = dup, len(seen)
    return out


def extract(dirpath):
    """事件层 + 舰表 + 城表 + 编年史 → 一份小结（按投影缓存成 pickle）。

    返回 dict（不是每回合一行的表）：这一组的判据本来就只需要计数 + 违规样例 + 少量序列。
    """
    q = KIT.load(str(dirpath), only=("events", "ships", "cities", "faction_process", "blueprints"))
    ev = q.table("events")

    # ① 被拆平的城，**同回合内**不该被它自己的旧主复垦（一对净效果为零的事件）。
    #    `city_razed`：target = 城、`data.owner` = 丢城的一方；`colony_founded`：actor = 建城方。
    #    顺序按 `seq` 判（拆平在前、复垦在后才是要抓的那条路径）。
    losers: dict[tuple, tuple] = {}
    razings = 0
    for _, r in ev[ev["type"] == "city_razed"].iterrows():
        razings += 1
        data = r["data"] if isinstance(r["data"], dict) else {}
        losers[(int(r["round"]), r["target_id"])] = (int(r["seq"]), data.get("owner"))
    bad: list[str] = []
    for _, r in ev[ev["type"] == "colony_founded"].iterrows():
        hit = losers.get((int(r["round"]), r["target_id"]))
        if hit and hit[0] < int(r["seq"]) and hit[1] == r["actor_id"]:
            bad.append(f"r{int(r['round'])}：{r['target_id']} 被 {hit[1]} 丢掉后又被同一个势力复垦")

    # ② 「定制化」（装了组件的）**活舰**在整局里出现过——累计口径（末回合快照会随轨迹归零）。
    ships = q.table("ships")
    fitted = ships[ships["hull"] > 0]["components"]
    customized = int(fitted.map(lambda c: bool(c) and len(c) > 0).sum())

    # ③ 编年史（累计，住在主流最后一行的 `chronicle` 里）：节拍、顺序、唯一性、参与者。
    chronicle = q.facts["chronicle"].iloc[-1]

    # ④ 战争回合数：把 `war_started`/`war_ended` 按**无序势力对**配对，得每场战争的时长。
    #    （地板那条判据的「真实长局」那一半；`war_scar_floor` 的**形状**那一半要内部函数与
    #    合成世界，留在 `src/tests/sim/horizon_mid.rs`。）
    open_wars: dict[tuple, int] = {}
    durations: list[int] = []
    for _, r in ev[ev["type"].isin(["war_started", "war_ended"])].sort_values(["round", "seq"]).iterrows():
        pair = tuple(sorted((r["actor_id"], r["target_id"])))
        rnd = int(r["round"])
        if r["type"] == "war_started":
            open_wars.setdefault(pair, rnd)
        elif pair in open_wars:
            durations.append(rnd - open_wars.pop(pair))

    # ⑤ 完备性审计（Rust 侧 `src/tests/projection/mod.rs` 那两条的同源守卫）。
    city_checked, city_unexplained = reconcile_cities(q, ev)
    deaths, births, ship_unexplained = reconcile_ships(q, ev)
    flips, flip_bad = owner_flips(ev)
    hl_checked, hl_bad = headline_check(ev)

    # ⑥ 战斗/损伤（`src/tests/sim/combat.rs` 里能只看数据的那几条）。
    combat = combat_report(q)
    blueprints = blueprint_report(q)

    return {"razings": razings, "refound_bad": bad, "customized": customized,
            "ships": int(len(ships)), "foundings": int((ev["type"] == "colony_founded").sum()),
            "chronicle": chronicle, "war_durations": durations, "wars_open": len(open_wars),
            "city_checked": city_checked, "city_unexplained": city_unexplained,
            "ship_deaths": deaths, "ship_births": births, "ship_unexplained": ship_unexplained,
            "flips": flips, "flip_bad": flip_bad,
            "headline_checked": hl_checked, "headline_bad": hl_bad,
            "combat": combat, "blueprints": blueprints, "meta": q.meta}


def run(h, ck) -> None:
    out = h.digests([(s, ROUNDS) for s in SEEDS], extract)

    razings = sum(d["razings"] for d in out)
    refounds = [(s, msg) for s, d in zip(SEEDS, out) for msg in d["refound_bad"]]
    customized = sum(d["customized"] for d in out)
    foundings = sum(d["foundings"] for d in out)
    tag = f"{len(SEEDS)} seed × {ROUNDS} 回合"

    ck.check("被拆平的城不在同回合被旧主复垦", not refounds,
             "；".join(m for _, m in refounds[:3]) or f"{tag}：{razings} 次拆平，0 次自我复垦")
    ck.check("拆平守卫没有空转（真发生过拆平）", razings >= MIN_RAZINGS,
             f"{tag} 共 {razings} 次拆平（下限 {MIN_RAZINGS}）")
    ck.check("整局里出现过装了组件的活舰", customized > 0,
             f"{tag}：累计 {customized} 舰·回合（舰表 {sum(d['ships'] for d in out)} 行）")
    ck.check("建城事件确实在发（与复垦那条互为对照）", foundings > 0, f"{tag} 共 {foundings} 次建城")

    audit_checks(h, ck, out)
    combat_checks(h, ck, out)
    blueprint_checks(h, ck, out)
    story_checks(h, ck, out)
    war_floor_checks(h, ck, out)


def blueprint_checks(h, ck, out) -> None:
    """**设计图**（`sim/blueprints.rs` + `autocontrol/blueprints.rs` 里能只看数据的那几条）。"""
    tag = f"{len(SEEDS)} seed × {ROUNDS} 回合"
    bp = [d["blueprints"] for d in out]

    kinds = sum(b["ship_kinds"] for b in bp)
    dr = sum(b["drift_n"] for b in bp)
    sample = next((m for b in bp for m in b["drift"]), "")
    ck.check("出厂快照不随时间变（一条舰的选装一生恒定：改图不碰已下水的舰）", dr == 0,
             f"{sample}（共 {dr} 条）" if dr else f"{tag}：{kinds:,} 条舰的选装全程没变过")
    ck.check("快照守卫没有空转（真的看过舰）", kinds >= 100, f"{kinds:,} 条舰")

    same = sum(b["snap_same"] for b in bp)
    prev = sum(b["snap_prev"] for b in bp)
    nb = sum(b["snap_bad"] for b in bp)
    sample = next((m for b in bp for m in b["snap_ex"]), "")
    ck.check("出厂那一刻的选装取自那张图（本回合的图，或「同回合先下水后改图」前的上一版）", nb == 0,
             f"{sample}（共 {nb} 例）" if nb else
             f"{tag}：{same + prev} 条有图新舰全部对得上（其中 {prev} 条属「先下水后改图」）")
    ck.check("出厂快照守卫没有空转（真有带图下水的新舰）", same + prev >= 10, f"{same + prev} 条（下限 10）")

    ab = sum(len(b["attr_bad"]) for b in bp)
    ac = sum(b["attr_checked"] for b in bp)
    sample = next((m for b in bp for m in b["attr_bad"]), "")
    ck.check("造舰事件的图归因与舰表一致（有图才写，写了就要对）", ab == 0,
             sample or f"{tag}：{ac:,} 条造舰事件的归因全部与表一致")

    cb = sum(len(b["count_bad"]) for b in bp)
    rows = sum(b["bp_rows"] for b in bp)
    sample = next((m for b in bp for m in b["count_bad"]), "")
    ck.check("图表的 ship_count 是算出来的（== 该回合指向它的舰数）", cb == 0,
             sample or f"{tag}：{rows:,} 张图快照的计数全部自洽")

    db = sum(len(b["dup_bad"]) for b in bp)
    sigs = sum(b["dup_sigs"] for b in bp)
    sample = next((m for b in bp for m in b["dup_bad"]), "")
    ck.check("图按 (舰级, 选装) 去重（同一势力同一回合没有两张同签名的图）", db == 0,
             sample or f"{tag}：{sigs:,} 个 (回合,势力,舰级,选装) 签名各只有一张图")


def combat_checks(h, ck, out) -> None:
    """**战斗/损伤**（`src/tests/sim/combat.rs` 搬过来的那几条，样本放到每一行）。"""
    tag = f"{len(SEEDS)} seed × {ROUNDS} 回合"
    shots = sum(d["combat"]["shots"] for d in out)
    kinds = sum(d["combat"]["shot_bad_kinds"] for d in out)
    sample = next((m for d in out for m in d["combat"]["shot_bad"]), "")
    ck.check("每一发都自洽（命中折减∈[0,1]、射程外不掉血、没命中没伤害、没点防不拦截）",
             kinds == 0, "；".join(next((d["combat"]["shot_bad"] for d in out if d["combat"]["shot_bad"]), []))
             if kinds else f"{tag}：{shots:,} 发逐发成立")
    ck.check("射击守卫没有空转（真的开过火）", shots >= 500, f"{shots:,} 发（下限 500）")

    # 击杀所需的伤害：**按 (回合, 目标) 累计**对账——`target_hull_before` 是那一发那一刻的记录，
    # 同一回合可能多舰轮着打，所以「一发打不死满血目标」是正常的；能对账的是「这一回合挨的总伤害
    # ≥ 上一回合结束时它的船体」（上一回合还没有它 = 本回合新造的，跳过）。
    kshort = next((m for d in out for m in d["combat"]["kill_short"]), "")
    kc = sum(d["combat"]["kill_checked"] for d in out)
    ck.check("击杀那一发打进船体的量（hull_pen）≥ 它自己记下的开火前船体", not kshort,
             kshort or f"{tag}：{kc:,} 次击杀全部满足（伤害够把它打掉）")
    ck.check("击杀伤害守卫没有空转（有可对账的击杀发数）", kc >= 10, f"{kc:,} 次（下限 10）")

    dead = sum(d["combat"]["dead"] for d in out)
    cdead = sum(d["combat"]["combat_dead"] for d in out)
    causes = sorted({c for d in out for c in d["combat"]["dead_causes"]})
    unex = next((m for d in out for m in d["combat"]["kill_unexplained"]), "")
    phan = next((m for d in out for m in d["combat"]["kill_phantom"]), "")
    ck.check("战沉的舰都有一发 killed 射击（欠费生锈报废的不算）", not unex,
             unex or f"{tag}：{cdead:,} 次战沉全部有那一发（死因集合 {causes}）")
    ck.check("击杀守卫反过来也成立（killed 射击真的沉了船）", not phan,
             phan or f"{tag}：没有「打死了却没沉」的射击")
    ck.check("击杀守卫没有空转（真的打沉过船）", cdead >= 10, f"{cdead:,} 次战沉 / {dead:,} 次沉没（下限 10）")

    sbad = sum(d["combat"]["ship_bad_n"] for d in out)
    sample = next((m for d in out for m in d["combat"]["ship_bad"]), "")
    ck.check("舰面板在定义域里（0 < hull ≤ max、shield ∈ [0,max]、attack/speed/组件≥0）",
             sbad == 0, f"{sample}（共 {sbad} 处）" if sbad else
             f"{tag}：{sum(d['ships'] for d in out):,} 个舰·回合行全部在域里")

    rc = sum(d["combat"]["regen_checked"] for d in out)
    rb = sum(d["combat"]["regen_bad"] for d in out)
    bonus = sum(d["combat"]["regen_bonus_rows"] for d in out)
    sample = next((m for d in out for m in d["combat"]["regen_ex"]), "")
    ck.check("安静回合里护甲只增不减、且不超上限（基础再生量是下界；本土加成读面看不到，不断言等式）",
             rb == 0, f"{sample}（共 {rb} 处）" if rb else
             f"{tag}：{rc:,} 行成立，其中 {bonus:,} 行拿到了本土再生加成（> 基础量）")
    ck.check("再生守卫没有空转（真有安静回合可查）", rc >= 500, f"{rc:,} 行（下限 500）")


def audit_checks(h, ck, out) -> None:
    """**投影完备性审计**（`src/tests/projection/mod.rs` 的四条搬过来，样本更宽）。

    它们要证明的是「历史什么都没丢」：密集快照里每一次变化，引擎自己都给得出原因。
    Rust 版跑 seed 42 / 120 回合；这里跑 3 个种子 × 400 回合（样本大一个量级）。
    """
    tag = f"{len(SEEDS)} seed × {ROUNDS} 回合"

    checked = sum(d["city_checked"] for d in out)
    bad = [f"seed {s}: {m}" for s, d in zip(SEEDS, out) for m in d["city_unexplained"]]
    ck.check("城的每次归属/存亡变化都有事件命名它", not bad,
             "；".join(bad[:3]) or f"{tag}：{checked} 次变化全部有事件解释")
    ck.check("城审计没有空转（真检查到了变化）", checked >= 20,
             f"{checked} 次变化（下限 20；Rust 版只要求 5）")

    deaths = sum(d["ship_deaths"] for d in out)
    births = sum(d["ship_births"] for d in out)
    sbad = [f"seed {s}: {m}" for s, d in zip(SEEDS, out) for m in d["ship_unexplained"]]
    ck.check("舰的出现有造舰事件、消失有死因事件", not sbad,
             "；".join(sbad[:3]) or f"{tag}：{births} 次出生 / {deaths} 次死亡全部有事件解释")
    ck.check("舰审计没有空转（真的出生过也死过）", deaths >= 5 and births >= 5,
             f"{births} 次出生 / {deaths} 次死亡")

    flips = sum(d["flips"] for d in out)
    fbad = [f"seed {s}: {m}" for s, d in zip(SEEDS, out) for m in d["flip_bad"]]
    ck.check("一回合内同一座城不会易主两次（净效果为零的振荡）", not fbad,
             "；".join(fbad[:3]) or f"{tag}：{flips} 次活城易主，0 次同回合翻转两遍")
    ck.check("易主守卫没有空转（真的易主过）", flips >= 5, f"{flips} 次活城易主（下限 5）")

    hl = sum(d["headline_checked"] for d in out)
    hbad = [f"seed {s}: {m}" for s, d in zip(SEEDS, out) for m in d["headline_bad"]]
    ck.check("事件的 headline 逐字点到每个参与者（索引指向谁，句子就说谁）", not hbad,
             "；".join(hbad[:3]) or f"{tag}：{hl} 个实体全部出现在标题里")
    ck.check("标题守卫没有空转（真的检查了实体）", hl >= 100, f"{hl} 个实体（下限 100）")


def story_checks(h, ck, out) -> None:
    """剧情编年史（`sim/horizon_mid.rs` 的两条）：节拍按回合触发、顺序单调、id 唯一、
    **参与者是具体的**（事件型触发写的是本回合的实际对象，不是泛化空标签）。

    与 Rust 版的区别只有一处：节拍清单**从 `meta.json` 的 `story` 读**，不写死
    「prologue 在 1 回合 / planet_x_arrives 在 60 回合」——那正是最容易过时的地方
    （story 表加一条节拍，写死的断言就假红或空转）。
    """
    chron = [d["chronicle"] for d in out]
    ids_bad, order_bad, dup_bad = [], [], []
    for seed, rows in zip(SEEDS, chron):
        ids = [c["id"] for c in rows]
        rounds = [c["round"] for c in rows]
        if rounds != sorted(rounds):
            order_bad.append(f"seed {seed}: 编年史不是按触发回合单调的")
        dupes = {i for i in ids if ids.count(i) > 1}
        if dupes:
            dup_bad.append(f"seed {seed}: 这些节拍触发了不止一次 {sorted(dupes)[:3]}")

    story = (out[0]["meta"] or {}).get("story") or []
    beats = [b for b in story if (b.get("trigger") or {}).get("kind") == "round_at"]
    for seed, rows in zip(SEEDS, chron):
        fired = {c["id"] for c in rows}
        for b in beats:
            if (b["trigger"].get("round") or 0) <= ROUNDS and b["id"] not in fired:
                ids_bad.append(f"seed {seed}: RoundAt 节拍 {b['id']}（第 {b['trigger']['round']} 回合）没触发")
    ck.check("RoundAt 节拍都触发了（清单从 meta.story 读，不写死回合数）",
             bool(beats) and not ids_bad,
             "；".join(ids_bad[:3]) or f"{len(beats)} 条节拍 × {len(SEEDS)} 个种子全部触发")
    ck.check("编年史按触发回合单调、id 唯一（每个节拍只触发一次）",
             not order_bad and not dup_bad, "；".join((order_bad + dup_bad)[:3]) or "顺序单调、无重复")
    ck.check("编年史守卫没有空转（真触发了节拍）",
             all(len(rows) >= len(beats) for rows in chron),
             f"各种子触发 {[len(r) for r in chron]} 条（RoundAt 节拍 {len(beats)} 条）")

    # 参与者具体：事件型节拍写的是本回合的实际对象（谁与谁开战、哪座城被夷平、谁建立了殖民地）。
    CONCRETE = {"first_war": 2, "first_raze": 2, "first_colony": 2}
    part_bad = []
    seen: list[str] = []
    for seed, rows in zip(SEEDS, chron):
        by_id = {c["id"]: c for c in rows}
        for bid, least in CONCRETE.items():
            c = by_id.get(bid)
            if c is None:
                continue
            seen.append(bid)
            parts = c.get("participants") or []
            if len(parts) < least or any((not p) for p in parts):
                part_bad.append(f"seed {seed}: {bid} 的参与者是 {parts}（要 ≥{least} 个具体对象）")
        cn = by_id.get("cn_us_rivalry")
        if cn is not None:
            seen.append("cn_us_rivalry")
            if not {"中国", "美国"} <= set(cn.get("participants") or []):
                part_bad.append(f"seed {seed}: cn_us_rivalry 的参与者是 {cn.get('participants')}")
    ck.check("事件型节拍的参与者是具体的（开战双方 / 城与拆城者 / 殖民者与天体）",
             not part_bad, "；".join(part_bad[:3]) or f"整局里见到 {sorted(set(seen))}")

    # RoundAt 节拍保留**静态**参与者（配置里写的那一串，没有事件可富化）。
    # ⚠ `meta.json` 的 `story` 段**不发** beat 的静态 `participants`（只发 id/title/trigger/effects），
    #   所以判据不写「等于配置里那一串」——改判「**非空**」+「**跨种子逐字相同**」：
    #   静态的东西在任何世界里都是同一串；哪天有人给 RoundAt 接了事件富化，这一条就红。
    static_bad, seen_parts = [], {}
    for seed, rows in zip(SEEDS, chron):
        by_id = {c["id"]: c for c in rows}
        for b in beats:
            c = by_id.get(b["id"])
            if c is None:
                continue
            parts = c.get("participants") or []
            if not parts:
                static_bad.append(f"seed {seed}: RoundAt 节拍 {b['id']} 一个参与者都没有")
            seen_parts.setdefault(b["id"], {})[seed] = parts
    for bid, per_seed in seen_parts.items():
        vals = {tuple(v) for v in per_seed.values()}
        if len(vals) > 1:
            static_bad.append(f"节拍 {bid} 的参与者在不同种子之间不同（{per_seed}）——静态节拍被事件富化了？")
    ck.check("RoundAt 节拍的参与者非空、且跨种子逐字相同（静态 = 没有被事件富化）",
             not static_bad, "；".join(static_bad[:2]) or f"{len(seen_parts)} 条节拍 × {len(SEEDS)} 个种子都对得上")


def war_floor_checks(h, ck, out) -> None:
    """记恨地板（战争疤痕）的**真实长局那一半**：没有一场战争短于地板承诺的回合数。

    地板自身的**形状**（随年龄抬高、窗口内始终敌意、出了窗口彻底消失、只属于那一对）
    要内部函数 `war_scar_floor` 与一个手工摆出来的合成世界 ⇒ 那一半留在
    `src/tests/sim/horizon_mid.rs`（两栏对账见 `.agents/notes/test-decoupled-suite.md`）。
    """
    meta = out[0]["meta"]
    span = (meta.get("diplomacy") or {}).get("war_scar_rounds")
    base = (meta.get("diplomacy") or {}).get("war_scar_relation")
    thr = (meta.get("combat") or {}).get("war_threshold")
    if not span or base is None or thr is None:
        ck.check("战争疤痕：配置里读得到 war_scar_rounds / war_scar_relation / war_threshold",
                 False, f"span={span} base={base} thr={thr}")
        return
    min_age = next((a for a in range(0, int(span) + 1)
                    if base * (1.0 - a / float(span)) > thr), None)
    ck.check("战争疤痕：配置自洽（初值低于交战阈值、且窗口内会抬过阈值）",
             base < thr and min_age is not None,
             f"span={int(span)} base={base} thr={thr} ⇒ 最短战争 {min_age} 回合")

    durations = [d for o in out for d in o["war_durations"]]
    short = [d for d in durations if d < min_age]
    ck.check("没有任何一场战争短于地板承诺的回合数", not short,
             f"最短 {min(durations) if durations else '—'}" if not short else
             f"有 {len(short)} 场短于 {min_age}：{sorted(short)[:5]}")
    ck.check("战争守卫没有空转（整局里真的打完过仗）", len(durations) >= 5,
             f"{len(durations)} 场打完（未结 {sum(o['wars_open'] for o in out)} 场）")


if __name__ == "__main__":
    sys.exit(group_main("g2_mid", run))
