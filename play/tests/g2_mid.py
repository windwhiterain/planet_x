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


def extract(dirpath):
    """事件层 + 舰表 + 城表 + 编年史 → 一份小结（按投影缓存成 pickle）。

    返回 dict（不是每回合一行的表）：这一组的判据本来就只需要计数 + 违规样例 + 少量序列。
    """
    q = KIT.load(str(dirpath), only=("events", "ships", "cities"))
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

    return {"razings": razings, "refound_bad": bad, "customized": customized,
            "ships": int(len(ships)), "foundings": int((ev["type"] == "colony_founded").sum()),
            "chronicle": chronicle, "war_durations": durations, "wars_open": len(open_wars),
            "city_checked": city_checked, "city_unexplained": city_unexplained,
            "ship_deaths": deaths, "ship_births": births, "ship_unexplained": ship_unexplained,
            "flips": flips, "flip_bad": flip_bad,
            "headline_checked": hl_checked, "headline_bad": hl_bad,
            "meta": q.meta}


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
    story_checks(h, ck, out)
    war_floor_checks(h, ck, out)


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
