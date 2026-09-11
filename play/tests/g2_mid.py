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
* **贸易三账**（`src/tests/sim/trade.rs` 搬走的三条）：成交清单逐回合 ↔ `view.market_settled`、
  价格分解逐项重算、买方名次 = 引擎的购买力序。
* **货舱（M2）**：`ships.cargo_capacity` = 舰级舱容 × 战损折算 `船体/船体上限`，且舰级舱容
  是设计裁决（`src/tests/sim/haul.rs` 的 `cargo_capacity_...` 数据级那一半）。
* **产地货栈（M1）**：`depots` 表 = 非首都天体的在栈存量；首都天体上不许"凭空"出现货栈，
  且 `cities.depot_value` 必须等于同一本账按价值计的城视角合计
  （`src/tests/sim/haul.rs` 的 `off_capital_production_...` 数据级那一半）。
* **派单抽签**：`round_inputs.rolls` 里每条 `route` 记录的 `pool`（每条腿那一刻的权重）
  与 `value`/`pool_total`/`picked` 逐条复算——抽签就是按积压占比切成区间
  （`src/tests/autocontrol/freight.rs` 的 `route_lottery_...`）。
* **合成场景 · 拨控制叶**（施工图 §5.6 第 6 批）：悬空指针 / 玩家钉住的图 / 回收只碰自己造的 /
  预算的两个极端——`autocontrol/blueprints.rs` 与 `sim/spending.rs` 里剩下那几条。造世界用
  `planet_x`、捏世界用 Python 改档（`h.scenario`）或引擎自己的 `--apply`（`h.scenario_apply`），
  断言仍只看投影。
"""

from __future__ import annotations

import json
import math
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _harness import CACHE_ROOT, KIT, group_main  # noqa: E402

SEEDS = (1, 7, 11)       # 与 g3 共用同一批 400 回合世界（轨迹确定性嵌套 ⇒ 同一目录）
ROUNDS = 400
MIN_RAZINGS = 20          # 与 Rust 版同阈值：样本太小 ⇒ 守卫会空转，得报出来
MIN_TRADES = 50           # 成交对账防空转：3 seed × 400 回合实测 1366 笔
MIN_RANK_ROUNDS = 300     # 买方名次防空转：3 seed × 400 回合实测 1200 个「排过队」的回合
MIN_CARGO_ROWS = 1000     # 货舱防空转：3 seed × 400 回合的「舰·回合」行数下限
MIN_DEPOT_ROWS = 100      # 货栈防空转：3 seed × 400 回合的货栈行数下限
MIN_ROUTE_DRAWS = 50      # 派单抽签防空转：3 seed × 400 回合的 route 记录下限

# 合成场景：造世界（0 回合）→ 拨控制叶 → 推 3 回合。回合 1 就能看见 AI 那一趟（`design_fleets`
# 挂在 `step_construction` 末尾、每回合都跑），3 回合足够看出「它没有回头修」。
SCENARIO_SEED = 42
SCENARIO_ROUNDS = 3
FID = "中国"
# 首都那条场景要**盖住至少 3 个评估轮**（`governance.capital_review_every = 12`）：
# 49 = 4×12 + 1 ⇒ 第 12/24/36/48 回合都是评估轮。窗口短了 A/B 的另一半（`Auto` 留下评估行）
# 就会空转——判据里有一条专门盯着这个比例。
CAPITAL_ROUNDS = 49
# 拨到一个**离人口中心很远**但该势力确实有城的天体（水星熔炉基地在水星）。
CAPITAL_FAR = "水星"
# 停泊那条场景（`fleet.rs::dock_follows_body…`）：把一艘钉 `Dock`、另一艘钉 `Idle`。
# 名字是种子 42 开局的（`DOCK_SHIP` 在前、`IDLE_SHIP` 在后），场景里有一条防空转判据盯着它们还在不在。
DOCK_SHIP, IDLE_SHIP, DOCK_BODY = "长城", "赤霄", "海王星"
DOCK_ROUNDS = 12
# 「买不起就在等钱」的 A/B 窗口：要跑过「进度攒满」（corvette `build_points = 15`、# 实测 `rate = 10`/回合）+ 至少再两回合，`launch_waiting` 才亮得住。
AFFORD_ROUNDS = 6
# 清空/垫厚国库要写**国库里真有的键**（`factions.资源`）**加上选装要的货**（那份货中国开局没有，# 不写进去「垫厚」那一臂也还是买不起——第一版就是这么假绿的）。
AFFORD_RESOURCES = ("氦-3", "金", "硅", "碳", "铁")
# 陈旧的跟随（`fleet.rs::player_stale_follow…`）：钉一根**从第 0 回合就悬空**的 Follow。
STALE_SHIP = "长城"
STALE_ROUNDS = 4
# 货栈账在**哪几个回合**取：库存水平是真实世界自己长出来的（判据不摆库存）。
SITE_LEDGER_ROUNDS = (80, 400)
# 「下水那艘长什么样」两臂用的城（`--apply` 的图指向不同城的建造区，两臂互不干扰）。
LAUNCH_YARDS = {"Player": "长三角", "Auto": "水星熔炉基地"}
LAUNCH_ROUNDS = 8
# 图上写的**倾向**（供指令的那条链 2026-10 已删：图只给倾向，不给指令）。
LAUNCH_DOCTRINE = {"temper": 0.5, "lone_wolf": -0.5}
LAUNCH_KITING = 0.7
# 修船那条场景：造一个受损组件，两臂只差坐标（本土 / 外海）。
REPAIR_COMPONENT, REPAIR_START, REPAIR_ROUNDS = "railgun", 5.0, 4
# 玩家钉的常驻运输线：起点天体选法见 `commanded_haul_checks` 的说明（本地需求小、
# 能攒出可出口余量；实测 r41 才等到货 ⇒ 窗口 60 回合）。
HAUL_SHIP, HAUL_FROM, HAUL_TO, HAUL_ROUNDS = "北斗", "灶神星", "地球", 60
# 娱乐拉忠诚那条：势力取一个「城市分布很散」的（远城才有低距离目标）。
WELFARE_FID, WELFARE_START, WELFARE_ROUNDS = "星系矿业", 0.35, 5
# 殖民那条：起点回合与天体**从长局里扫出来**（开局 22 座城占满 22 个定居点，
# 得等到有城被拆平才有可复垦的空位）。
COLONIZE_ROUNDS = 40
# 驻泊深度那条：把一个势力的舰全搬到同一个日心距、钉 Idle。
KNOW_FID = "中国"
# 治理那两条：城搬到离首都最远的天体 / 国库清零，两臂只差 `MOND 掌握度`。
GOV_FID, GOV_ROUNDS = "中国", 5
# 护盾那条：造一仗（攻方装炮、守方装盾、两家关系压到 -35、摆在远离首都处）。
DUEL_ATK, DUEL_DEF, DUEL_GUN, DUEL_ROUNDS = "中国", "美国", "railgun", 3
# 舰队防空那条：两臂只差 PD 友舰的坐标。
PD_ATK, PD_DEF, PD_ROUNDS = "中国", "美国", 3
# 拦光那条：两臂只差守方装不装点防。
INTC_ATK, INTC_DEF, INTC_ROUNDS = "中国", "美国", 3
# 跟随那条：跟随者 + 友舰同在原点，一艘敌舰贴在射程内。
FOLLOW_FID, FOLLOW_ENEMY, FOLLOW_ROUNDS = "中国", "美国", 3
# 谁给城出钱：把城改成「还差一半没建」，只拨池子。
SITE_FID, SITE_STOCK = "中国", ("铁", "碳", "硅")
# 战争推思潮：把最偏和平端那家的舰摆成「一发即沉」的仗。
IDEO_ROUNDS = 1
# 建造瓶颈：三条臂（库存 × 预算）看 increment 与 rate 的关系。
BUILD_ROUNDS = 3
# 管不起那条：把舰改成重舰 ⇒ 覆盖率结构性掉到 0，两臂只差 MOND 掌握度。
GOV_PLATE_ROUNDS = 5
# 静息亲和：两臂只差思潮，靠多回合让确定性拉力压过外交噪声。
AFFIN_ROUNDS = 30
# 改旗易帜那条：把旧主推到极端、一个对照势力推到相反极、其余中立（倒下谁由
# `--call ideology_similarity` 算出来，不写死）。
DEFECT_FID, DEFECT_TARGET_HINT, DEFECT_LOYALTY, DEFECT_ROUNDS = "中国", "无国界科学组织", 0.05, 3
# `autocontrol::blueprints::DESIGN_PREFIX` 的镜像（引擎改名要跟着改；这类镜像表一律删掉、
# 问引擎要声明面是方向，但目前没有这个名字的声明面）。
DESIGN_PREFIX = "自动"


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
    cities = q.table("cities")[["round", "城名", "势力", "已焚毁"]]
    prev: dict[str, tuple] = {}
    checked, unexplained = 0, []
    for rnd, grp in cities.groupby("round", sort=True):
        cur = {r.城名: (r.势力 or "", bool(r.已焚毁)) for r in grp.itertuples(index=False)}
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
    ships = q.table("ships")[["round", "舰名"]]
    alive: dict[int, set] = {int(r): set(g["舰名"]) for r, g in ships.groupby("round")}

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
    # B4 事件层那两条（`sim/shots.rs` 的 `the_event_breakdown_adds_up_to_the_aggregate_damage`
    # + `a_salvo_aimed_only_at_a_corpse_does_not_emit_an_attack`，2026-10 第 7 批）：
    # 齐射一级的账——逐发之和必须等于聚合伤害，且**每条事件至少有一发真打出去**。
    salvo_bad: list[str] = []
    salvo_n = skipped_n = no_live = 0
    for r in ev[ev["type"] == "attack"].itertuples(index=False):
        shots = (r.data or {}).get("逐发") or []
        salvo_n += 1
        if not shots:
            if len(salvo_bad) < 3:
                salvo_bad.append(f"r{int(r.round)} {r.actor_id}→{r.target_id}: 齐射没带逐发明细")
            continue
        tot = sum(float(sh.get("伤害") or 0.0) for sh in shots)
        if abs(tot - float(r.magnitude or 0.0)) > 0.0101:  # `magnitude` 过 r2
            if len(salvo_bad) < 3:
                salvo_bad.append(f"r{int(r.round)} {r.actor_id}→{r.target_id}: 逐发之和 {tot:.4f}"
                                 f" ≠ 聚合 {r.magnitude}")
        live = [sh for sh in shots if not sh.get("未击发")]
        skipped_n += len(shots) - len(live)
        if not live:
            no_live += 1
            if len(salvo_bad) < 3:
                salvo_bad.append(f"r{int(r.round)} {r.actor_id}→{r.target_id}: 一发真打出去的都没有，"
                                 f"却发了事件")

    for r in ev[ev["type"] == "attack"].itertuples(index=False):
        rnd, tgt = int(r.round), r.target_id
        for sh in (r.data or {}).get("逐发") or []:
            out["shots"] += 1
            dmg = float(sh.get("伤害") or 0.0)
            pen = float(sh.get("实入船体") or 0.0)
            if pen:
                hull_lost[(rnd, tgt)] = hull_lost.get((rnd, tgt), 0.0) + pen
            if sh.get("补刀"):
                killed_shot.add((rnd, tgt))
                kill_checked += 1
                # **那一发的 `hull_pen` 必须够打掉它自己记下的「开火前船体」**——引擎就是
                # `hull -= hull_pen; destroyed = hull <= 0`，所以这条逐发可对账。
                # （① 别用 `damage`：`hull_mult` 可 > 1，`hull_pen` 能比 `damage` 大；
                #   ② 别用「上一回合末的船体」：改装的船下一回合 `hull_max` 就变了——
                #   实测 r86 莱茵4 上一回合 24.0、本回合按 12.0 的船体被打掉。）
                if pen + tol < float(sh.get("战前船体") or 0.0):
                    if len(kill_short) < 3:
                        kill_short.append(
                            f"r{rnd} {tgt}: 这一发打进 {pen:.4f} < 开火前船体 {sh.get('target_hull_before')}")
            why = None
            if not (0.0 <= float(sh.get("命中折减") or 0.0) <= 1.0):
                why = "命中折减越界"
            elif not sh.get("未击发") and not (0.2 <= float(sh.get("命中折减") or 0.0) <= 1.0):
                why = "命中折减低于下限 0.2"
            elif not sh.get("未击发") and not (0.0 <= float(sh.get("护盾吸收比") or 0.0) <= 1.0):
                why = "护盾吸收比例越界"
            elif not sh.get("未击发") and not (0.0 <= float(sh.get("护甲减伤") or 0.0) <= 0.85):
                why = "护甲减伤越界"
            elif not sh.get("未击发") and float(sh.get("本土防御") or 0.0) <= 0.0:
                why = "本土防御倍率非正"
            elif not sh.get("未击发") and float(sh.get("分配乘数") or 0.0) <= 0.0:
                why = "火力分配乘数非正"
            elif not sh.get("未击发") and float(sh.get("战前船体") or 0.0) <= 0.0:
                why = "打的时候目标已经死了"
            elif dmg < -tol or pen < -tol:
                why = "伤害为负"
            elif dmg > tol and (sh.get("未击发") or not sh.get("在射程内")):
                why = "射程外/被跳过却掉血"
            elif float(sh.get("命中折减") or 0.0) <= tol and dmg > tol:
                why = "没命中却有伤害"
            elif float(sh.get("点防拦截") or 0.0) <= tol and float(sh.get("点防吃掉") or 0.0) > tol:
                why = "没有点防却拦截了"
            if why and len(shot_bad.get(why, [])) < 2:
                shot_bad.setdefault(why, []).append(f"r{rnd} {r.actor_id}→{tgt}: {why}")
    # **组件损耗**（第 7 批，`sim/combat.rs::fire_degrades_components_under_damage`）：
    #   ① 没挨打 ⇒ 组件耐久**一点不掉**（长局逐行；这条是修复那条的孪生守卫）
    #   ② 真打进船体（`hull_pen > 0`）⇒ **有**组件掉了——原件也是 `any(|(a,b)| a < b)` 的存在性
    #      ⚠ 别写成「每发必掉」：实测 72 次真伤里有 3 次组件耐久一位没动（`组件耐久` 过 r2，
    #      浅伤折到组件上的量小到看不见）。
    pen: dict = {}
    for e in ev[ev["type"] == "attack"].itertuples(index=False):
        for sh in (e.data or {}).get("逐发") or []:
            if not sh.get("未击发"):
                key = (int(e.round), e.target_id)
                pen[key] = pen.get(key, 0.0) + float(sh.get("实入船体") or 0.0)
    shp = q.table("ships")
    series: dict = {}
    for r in shp.itertuples(index=False):
        series.setdefault(r.舰名, {})[int(r.round)] = (list(r.组件), list(r.组件耐久))
    no_hit = no_hit_bad = hit = hit_drop = 0
    for name, rs in series.items():
        for r0, r1 in zip(sorted(rs), sorted(rs)[1:]):
            (c0, h0), (c1, h1) = rs[r0], rs[r1]
            if c0 != c1:
                continue
            drop = any(y < x - 1e-9 for x, y in zip(h0, h1))
            if pen.get((r1, name), 0.0) > 1e-9:
                hit += 1
                hit_drop += 1 if drop else 0
            else:
                no_hit += 1
                no_hit_bad += 1 if drop else 0
    out["comp"] = {"no_hit": no_hit, "no_hit_bad": no_hit_bad, "hit": hit, "hit_drop": hit_drop}
    out["salvo_bad"] = salvo_bad[:4]
    out["salvo_n"], out["skipped_n"], out["no_live"] = salvo_n, skipped_n, no_live
    out["shot_bad"] = [m for v in shot_bad.values() for m in v][:4]
    out["shot_bad_kinds"] = len(shot_bad)

    # **只有战死的才必须有那一发**：欠费生锈报废（`upkeep_shortfall`）与战斗无关。
    dead = ev[ev["type"] == "ship_destroyed"]
    dead_cause = {(int(r.round), r.target_id): ((r.data or {}).get("击毁原因") or "?")
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
        if not (0.0 < float(r.船体) <= float(r.船体上限) + tol):
            ship_bad.append(f"r{int(r.round)} {r.舰名}: hull={r.船体} / max={r.船体上限}")
        elif not (-tol <= float(r.护盾) <= float(r.护盾上限) + tol):
            ship_bad.append(f"r{int(r.round)} {r.舰名}: shield={r.护盾} / max={r.护盾上限}")
        elif float(r.attack) < -tol or float(r.speed) < -tol:
            ship_bad.append(f"r{int(r.round)} {r.舰名}: attack={r.attack} speed={r.speed}")
        elif any(float(h) < -tol for h in (r.组件耐久 or [])):
            ship_bad.append(f"r{int(r.round)} {r.舰名}: 组件完整度出现负数")
    out["ship_bad"] = ship_bad[:3]
    out["ship_bad_n"] = len(ship_bad)

    rust_round = {(int(r.round), r.势力) for r in fp.itertuples(index=False)
                  if float(r.fleet_rust or 0.0) > 0.0}
    per_round: dict = {}
    for r in ships.itertuples(index=False):
        per_round.setdefault(int(r.round), {})[r.舰名] = r
    checked = regen_bad = regen_short = 0
    regen_ex: list[str] = []
    for rnd in sorted(per_round):
        prev = per_round.get(rnd - 1)
        if not prev:
            continue
        for sid, cur in per_round[rnd].items():
            was = prev.get(sid)
            if was is None or hull_lost.get((rnd, sid)) or (rnd, cur.势力) in rust_round:
                continue
            checked += 1
            c_max, c_regen = float(cur.船体上限), float(cur.hull_regen)
            base = min(c_max, float(was.船体) + c_regen * c_max)
            got = float(cur.船体)
            # ⚠ 只断言**下界 + 上限**，不断言等式：引擎的再生是
            # `hull + hull_max × (hull_regen + home_regen_bonus)`——**本土防御半径内还有一份加成**
            # （`sim/military.rs`），而那份加成读面不暴露（要看位置/首都/MOND，属于引擎内部）。
            # 下界仍然是真判据：安静回合里**至少**要长回基础再生量，且绝不超过上限。
            if got > c_max + 1e-9 or got + 1e-6 < base:
                regen_bad += 1
                if len(regen_ex) < 3:
                    regen_ex.append(f"r{rnd} {sid}: hull {was.船体} → {got}，基础再生至少应到 {base:.4f}"
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
        life.setdefault(r.舰名, set()).add(tuple(r.组件 or []))
    drift = {k: v for k, v in life.items() if len(v) > 1}
    out["ship_kinds"] = len(life)
    out["drift_n"] = len(drift)
    out["drift"] = [f"{k}: {list(v)[:2]}" for k, v in list(drift.items())[:3]]

    # 列名就是引擎发的中文名词（`舰级`/`选装`）；`iterrows` 给的是 Series ⇒ 用 `[]` 取值。
    design = {(int(r["round"]), r["势力"], r["图名"]):
              {"舰级": r["舰级"], "选装": r["选装"]} for _, r in bps.iterrows()}
    same = prev = neither = 0
    snap_ex: list[str] = []
    spawned = ships[ships["round"] == ships["下水回合"]]
    for r in spawned.itertuples(index=False):
        if not r.出厂图:
            continue
        rnd, fid = int(r.round), r.势力
        cur = design.get((rnd, fid, r.出厂图))
        if cur is None or not (cur["选装"] or []):
            continue                      # 这一回合图没快照 / 空选装 = 交给生成器
        got = {c for c in (r.组件 or [])}
        if got == set(cur["选装"]):
            same += 1
        elif (before := design.get((rnd - 1, fid, r.出厂图))) is not None \
                and got == set(before["选装"] or []):
            prev += 1
        else:
            neither += 1
            if len(snap_ex) < 3:
                snap_ex.append(f"r{rnd} {r.舰名}: 舰上 {sorted(got)}，图（本回合）{sorted(cur['选装'])}")
    out["snap_same"], out["snap_prev"], out["snap_bad"], out["snap_ex"] = same, prev, neither, snap_ex

    attr: dict = {}
    for e in ev[ev["type"] == "ship_spawned"].itertuples(index=False):
        attr[(int(e.round), e.target_id)] = (e.data or {}).get("出厂图")
    mis: list[str] = []
    for r in spawned.itertuples(index=False):
        key = (int(r.round), r.舰名)
        if key not in attr:
            continue
        # ⚠ pandas 把缺失值给成 NaN，事件里是 `null`/None ⇒ 比之前要归一（否则「None vs nan」假红）。
        mine = None if _nan(r.出厂图) else r.出厂图
        theirs = None if _nan(attr[key]) else attr[key]
        if mine != theirs and len(mis) < 3:
            mis.append(f"r{int(r.round)} {r.舰名}: 事件说 {theirs}，表说 {mine}")
    out["attr_bad"], out["attr_checked"] = mis, len(attr)

    cnt: dict = {}
    for r in ships.itertuples(index=False):
        if r.出厂图:
            k = (int(r.round), r.势力, r.出厂图)
            cnt[k] = cnt.get(k, 0) + 1
    sc_bad: list[str] = []
    for r in bps.itertuples(index=False):
        k = (int(r.round), r.势力, r.图名)
        if int(r.ship_count or 0) != cnt.get(k, 0) and len(sc_bad) < 3:
            sc_bad.append(f"r{int(r.round)} {r.势力}/{r.图名}: 表说 {r.ship_count}，实为 {cnt.get(k, 0)}")
    out["count_bad"], out["bp_rows"] = sc_bad, len(bps)

    seen: dict = {}
    dup: list[str] = []
    # ⚠ 用 `iterrows`：列名是中文名词（`舰级`/`选装`），`itertuples` 的属性名不好认。
    for _, r in bps.iterrows():
        sig = (int(r["round"]), r["势力"], r["舰级"], tuple(r["选装"] or []))
        if sig in seen and len(dup) < 3:
            dup.append(f"r{sig[0]} {sig[1]}: {sig[2]} {list(sig[3])} 有两张图（{seen[sig]} / {r['图名']}）")
        seen[sig] = r["图名"]
    out["dup_bad"], out["dup_sigs"] = dup, len(seen)

    # 建造区 ↔ 设计图（`autocontrol/blueprints.rs`）：① 有指针 ⇒ 图必须**存在**（不许悬空）；
    # ② 图与建造区的舰级不符 ⇒ **只允许滞后几回合**（retool 当回合改了舰级，AI 那一趟下一回合
    # 才把图对齐；实测 3 例全是这样），**窗口末尾不许还挂着不符的图**。
    cities = q.table("cities")
    yard_rows = 0
    dangling: list[str] = []
    streak: dict = {}
    bad_streak: dict = {}
    # ⚠ `last` 必须先取**全局**最大回合：写在循环里累加的话，每一行都会被当成「窗口末尾」。
    last = int(cities["round"].max())
    for r in cities.itertuples(index=False):
        rnd, fid = int(r.round), r.势力
        for b in r.建筑 or []:
            st = b.get("建造舰级")
            if not st:
                continue
            yard_rows += 1
            ptr = b.get("设计图")
            key = (r.城名, b["建筑编号"])
            row = design.get((rnd, fid, ptr)) if ptr else None
            if ptr and row is None:
                if len(dangling) < 3:
                    dangling.append(f"r{rnd} {r.城名} 建筑{b['id']}：指针「{ptr}」在图库里不存在")
            if row is not None and row["舰级"] != st:
                streak[key] = streak.get(key, 0) + 1
                bad_streak[key] = bad_streak.get(key, 0) + 1
                if rnd == last and len(dangling) < 3:
                    dangling.append(f"窗口末尾 {r.城名} 建筑{b['id']}：区说造 {st}，图说 {row['舰级']}")
            else:
                streak[key] = 0
    out["yard_rows"] = yard_rows
    out["yard_dangling"] = dangling
    out["yard_max_streak"] = max(bad_streak.values(), default=0)
    out["yard_mismatch"] = sum(bad_streak.values())
    return out


def id_report(q) -> dict:
    """**建筑 id 永不复用**（`State.next_building_id` 单调计数器）。

    `Building` 是唯一没有名字的实体 ⇒ 它的身份就是 `(城名, 序号)`（见笔记 §2），那个序号**必须**
    经得起当长期引用：跨回合的 UI 选中态、agent 笔记里的「建筑 21」、两份存档的对比。

    ⚠ 旧分配器是「每回合扫全场取 `max(id) + 1`」⇒ **最高 id 的建筑一被拆，下个新建筑就拿回那个
    号**。A/B 实测（seed 1 / 400 回合）：旧分配器 **3 个 id 被复用**（91/92/93 在 r104 出现、消失、
    又在 r128 前后落到另一座城），换单调计数器后 **0 例**。

    判据：一个 id 的出现回合**必须连成一段**（有洞 = 消失后又回来 = 复用了）。
    """
    cities = q.table("cities")
    life: dict = {}
    last = 0
    for r in cities.itertuples(index=False):
        rnd = int(r.round)
        last = max(last, rnd)
        for b in r.建筑 or []:
            life.setdefault(b["建筑编号"], set()).add(rnd)
    gaps = [i for i, rs in life.items() if max(rs) - min(rs) + 1 != len(rs)]
    gone = [i for i, rs in life.items() if max(rs) < last]      # 窗口内消失过（防空转用）
    sample = []
    for i in gaps[:3]:
        rs = sorted(life[i])
        sample.append(f"id {i}：出现于 {rs[0]}…{rs[-1]} 共 {len(rs)} 个回合（中间有洞）")
    return {"ids": len(life), "gaps": sample, "gap_n": len(gaps), "gone_n": len(gone)}


def trade_report(q) -> dict:
    """贸易三账（B3）：`src/tests/sim/trade.rs` 里三条只看数据的判据小结。

    数据：`market_trades`（每笔一对一行）+ 主流每回合的 `view.market_settled` + `meta.json`
    的 `market` 配置。判据住在 :func:`trade_checks` 里（只改阈值/措辞不重读投影）。

    * **成交对账**：逐回合把每笔的 `moved[资源] × (1 − loss)` 加起来，必须等于那一回合
      `view.market_settled` 里**收到**的量（逐资源、双向）。
    * **价格分解**：拿同一行的 `depth` / `dist_au` 与 `meta.market` 的常数重算
      `mond_extra` / `freight_rate`——读面给的分解式必须自洽。
    * **买方名次**：`factions[].market_rank` 是引擎排好的购买力序（降序、同额按名字升序），
      读者不该自己重排。
    """
    mt = q.table("market_trades")
    market = (q.meta or {}).get("market") or {}

    # ① 成交清单 ↔ 世界收到的量（逐回合；不能跨回合求和，`market_settled` 是每回合一份）。
    by_round = {int(r): g.to_dict("records") for r, g in mt.groupby("round")}
    recon_bad: list[str] = []
    trades = 0
    recon_rounds = 0
    for _, fact in q.facts.iterrows():
        rnd = int(fact["round"])
        rows = by_round.get(rnd) or []
        if not rows:
            continue
        settled = fact["view"].get("market_settled") or {}
        received: dict[str, float] = {}
        for row in rows:
            trades += 1
            moved = row.get("moved") or {}
            if not moved:
                recon_bad.append(f"r{rnd}：{row['buyer']}→{row['seller']} 没有货，不该占位")
            if row["buyer"] == row["seller"]:
                recon_bad.append(f"r{rnd}：{row['buyer']} 自己跟自己成交")
            if not (0.0 <= row["loss"] <= 1.0):
                recon_bad.append(f"r{rnd}：{row['buyer']}→{row['seller']} loss={row['loss']} 越界")
            for rt, take in moved.items():
                if take <= 0.0:
                    recon_bad.append(f"r{rnd}：{row['buyer']}→{row['seller']} 的 {rt} 报了非正的 {take}")
                received[rt] = received.get(rt, 0.0) + take * (1.0 - row["loss"])
        for rt, got in settled.items():
            if got <= 1e-9:
                continue
            back = received.get(rt, 0.0)
            if abs(back - got) > 1e-6:
                recon_bad.append(f"r{rnd} {rt}：成交清单加起来 {back}，世界账却是 {got}")
        for rt in received:
            if settled.get(rt, 0.0) <= 1e-9:
                recon_bad.append(f"r{rnd} {rt}：清单里有成交，世界账上却是 0")
        recon_rounds += 1

    # ② 价格分解逐项重算（同一条定义式；`depth = 0` ⇒ 不穿带 ⇒ 零运费、零丢货）。
    price_bad: list[str] = []
    seen_freight = seen_same_body = seen_mond = 0
    for row in mt.to_dict("records"):
        if row["dist_au"] < 0.0 or row["depth"] < 0.0:
            price_bad.append(f"{row['buyer']}→{row['seller']}：距离/深度为负 "
                             f"{row['dist_au']}/{row['depth']}")
        if row["rel_mult"] <= 0.0:
            price_bad.append(f"{row['buyer']}→{row['seller']}：关系倍率 {row['rel_mult']} ≤ 0")
        if not (0.0 <= row["mastery"] <= 1.0):
            price_bad.append(f"{row['buyer']}→{row['seller']}：掌握度 {row['mastery']} 越界")
        want_mond = (market["mond_freight_mult"] * (row["depth"] / (row["depth"] + 1.0))
                     if row["depth"] > 0.0 else 0.0)
        if abs(row["mond_extra"] - want_mond) >= 1e-9:
            price_bad.append(f"{row['buyer']}→{row['seller']}：穿带溢价 "
                             f"{row['mond_extra']} ≠ {want_mond}")
        want_freight = market["freight_per_au"] * row["dist_au"] * (1.0 + row["mond_extra"])
        if abs(row["freight_rate"] - want_freight) >= 1e-9:
            price_bad.append(f"{row['buyer']}→{row['seller']}：运费率 "
                             f"{row['freight_rate']} ≠ {want_freight}")
        if row["depth"] == 0.0 and row["loss"] != 0.0:
            price_bad.append(f"{row['buyer']}→{row['seller']}：没穿带却丢货 {row['loss']}")
        if row["freight_rate"] > 0.0:
            seen_freight += 1
        if row["dist_au"] == 0.0:
            seen_same_body += 1
        if row["mond_extra"] > 0.0:
            seen_mond += 1
            if row["depth"] <= 0.0:
                price_bad.append(f"{row['buyer']}→{row['seller']}：有穿带溢价却没有深度")
            # 掌握度到顶 ⇒ 一点货都不丢；否则穿了带必须丢（崇拜教开局掌握度就是 1.0）。
            if row["mastery"] < 1.0 - 1e-9:
                if not (row["loss"] > 0.0):
                    price_bad.append(f"{row['buyer']}→{row['seller']}：穿带且掌握度只有 "
                                     f"{row['mastery']:.3f}，却不丢货")
            elif row["loss"] != 0.0:
                price_bad.append(f"{row['buyer']}→{row['seller']}：掌握度到顶却丢货 {row['loss']}")

    # ③ 买方名次 = 引擎排好的购买力序（名次必须是 0..n-1 的排列）。
    rank_bad: list[str] = []
    rank_rounds = 0
    for _, fact in q.facts.iterrows():
        rnd = int(fact["round"])
        rows = [(name, r.get("market_rank"), r.get("purchasing_power") or 0.0)
                for name, r in fact["view"]["factions"].items()]
        if any(rk is None for _, rk, _ in rows):
            # 回合 0 是 `pre` 面：市场还没跑 ⇒ 中性缺省就是 `null`（见 `neutral.rs`）。
            if rnd == 0:
                continue
            rank_bad.append(f"r{rnd}：有势力没有名次"
                            f"（{sum(1 for _, rk, _ in rows if rk is None)} 个）")
            continue
        rank_rounds += 1
        for i, (name, rank, _) in enumerate(sorted(rows, key=lambda t: (-t[2], t[1]))):
            if rank != i:
                rank_bad.append(f"r{rnd} {name}：名次 {rank} ≠ 按购买力/名字应是 {i}")
                break

    return {"trades": trades, "recon_bad": recon_bad, "recon_rounds": recon_rounds,
            "price_bad": price_bad,
            "price_seen": {"freight": seen_freight, "same_body": seen_same_body, "mond": seen_mond},
            "rank_bad": rank_bad, "rank_rounds": rank_rounds}


def cargo_report(q) -> dict:
    """货舱（M2 / 施工图 §5 第 2 批）：有效舱容公式 + 舰级舱容的设计裁决。

    `ships` 表现已发 `载货`（按资源的在舱货物）与 `cargo_capacity`（有效舱容）。判据住在
    :func:`cargo_checks` 里。

    ⚠ `船体` / `船体上限` / `cargo_capacity` 都按 **r2** 发（读面约定：标量留两位压 token），
    所以公式对账走**舍入区间**（真值必须落在 `船体±0.005`、`船体上限±0.005` 推出的区间，
    再给 `cargo_capacity` 自身的 r2 留 ±0.005），不是逐位相等。
    """
    ships = q.table("ships")
    meta = q.meta or {}
    spec_cargo = {cls: s.get("cargo") for cls, s in (meta.get("ships") or {}).items()}

    formula_bad: list[str] = []
    checked = damaged = with_cargo = 0
    for r in ships.to_dict("records"):
        cls = r["舰级"]
        spec = spec_cargo.get(cls)
        if spec is None:
            formula_bad.append(f"r{r['round']} {r['舰名']}：舰级 {cls} 不在 meta.ships 里")
            continue
        checked += 1
        hmax, hull = r["船体上限"], r["船体"]
        if hmax <= 0.0:
            lo = hi = spec
        else:
            lo = spec * max(0.0, min(1.0, (hull - 0.005) / (hmax + 0.005)))
            hi = spec * max(0.0, min(1.0, (hull + 0.005) / max(hmax - 0.005, 1e-9)))
        got = r["cargo_capacity"]
        if not (lo - 0.005 - 1e-9 <= got <= hi + 0.005 + 1e-9):
            formula_bad.append(f"r{r['round']} {r['舰名']}（{cls}）：舱容 {got} 不在 "
                               f"[{lo:.4f}, {hi:.4f}]（船体 {hull}/{hmax} × 舰级 {spec}）")
        if hmax > 0.0 and hull < hmax - 1e-9:
            damaged += 1
        if r.get("载货"):
            with_cargo += 1

    # 舰级舱容是「设计裁决」：见 config/game.ron 的 ships 注释第 (3) 类。
    design = {"corvette": 2.0, "destroyer": 4.0, "cruiser": 6.0,
              "carrier": 20.0, "battleship": 6.0}
    design_bad = [f"{cls}：meta.ships 给 {spec_cargo.get(cls)} ≠ 设计裁决 {cap}"
                  for cls, cap in design.items() if spec_cargo.get(cls) != cap]
    bulk = sorted(cls for cls, cap in spec_cargo.items() if (cap or 0.0) >= 20.0)
    return {"checked": checked, "damaged": damaged, "with_cargo": with_cargo,
            "formula_bad": formula_bad, "design_bad": design_bad, "bulk": bulk,
            "classes": sorted(spec_cargo)}


def depot_report(q) -> dict:
    """产地货栈（M1 / 施工图 §5 第 3 批）：`depots` 表 ↔ `cities.depot_value`，以及「首都即集散地」。

    判据住在 :func:`depot_checks` 里。

    ⚠ **§12 同回合相位错位**：一条货栈行落在**当前**首都天体上时，只有两种说得清的可能——
    * `stale`：迁都**之前**它就在那儿（货栈冻结、不会自己搬走）；
    * `same_round`：首都本回合才搬过来——产出那一步看到的还是旧首都，所以往这里放了货。
    两者都要**逐处解释**（排除即断言）；其余一律算违规。
    """
    dep = q.table("depots")
    fac = q.table("factions")
    cities = q.table("cities")
    rv = ((q.meta or {}).get("market") or {}).get("resource_value") or {}
    capital = {(int(r["round"]), r["势力"]): r["capital_body"]
               for r in fac.to_dict("records")}

    rows = dep.to_dict("records")
    pos_bad: list[str] = []
    seen: set = set()
    for r in rows:
        key = (int(r["round"]), r["势力"], r["天体名"], r["resource"])
        if r["amount"] <= 0.0:
            pos_bad.append(f"r{key[0]} {key[1]}@{key[2]} {key[3]}={r['amount']} 非正")
        if key in seen:
            pos_bad.append(f"r{key[0]} {key[1]}@{key[2]} {key[3]} 重复行")
        seen.add(key)

    presence: dict = {}
    for r in rows:
        presence.setdefault((r["势力"], r["天体名"]), set()).add(int(r["round"]))
    stale = same_round = 0
    unexplained: list[str] = []
    for r in rows:
        rnd, fid, body = int(r["round"]), r["势力"], r["天体名"]
        if body != capital.get((rnd, fid)):
            continue
        prior = [rr for rr in presence[(fid, body)]
                 if rr < rnd and capital.get((rr, fid)) != body]
        prev_cap = capital.get((rnd - 1, fid))
        if prior:
            stale += 1
        elif prev_cap is not None and prev_cap != body:
            same_round += 1
        else:
            unexplained.append(f"r{rnd} {fid}@{body} {r['resource']}：首都天体上凭空出现货栈")

    agg: dict = {}
    for r in rows:
        k = (int(r["round"]), r["势力"], r["天体名"])
        agg[k] = agg.get(k, 0.0) + r["amount"] * rv.get(r["resource"], 1.0)
    agg_bad: list[str] = []
    nonzero = 0
    for c in cities.to_dict("records"):
        if c["已焚毁"]:
            continue
        want = agg.get((int(c["round"]), c["势力"], c["天体名"]), 0.0)
        got = c["depot_value"]
        if got > 0.0:
            nonzero += 1
        if abs(got - want) > 0.006:
            agg_bad.append(f"r{int(c['round'])} {c['城名']}: depot_value={got} ≠ Σ货栈={want}")

    return {"rows": len(rows), "pos_bad": pos_bad, "stale": stale, "same_round": same_round,
            "unexplained": unexplained, "agg_bad": agg_bad, "nonzero": nonzero,
            "bodies": len(presence)}


def dispatch_report(q) -> dict:
    """派单抽签（`autocontrol::freight::route_for`）：`round_inputs.rolls` 里的 `route` 记录。

    ⚠ **为什么不建 `haul_lanes` 表**（施工图 §5 第 4 批的建议）：`route_for` 在
    `step_military` 里**逐舰**调用，每艘舰看到的腿都是**那一刻**的（前面的舰已经把货搬走、
    池子改了，见 `haul_step` 在同一循环里）⇒ 一张「每回合每势力一条」的 `lanes()` 表既不
    忠实（§12 同回合相位错位）、也会和抽签记录打架。而抽签记录里的 `pool` 正是**那一刻、
    那艘舰**看到的候选腿（权重 = 货量），所以这一页用 `round_inputs` 就够了，**零新增序列化**。

    判据：逐条把 `value × pool_total` 按池子顺序切段，落点必须 == `picked`；且
    `pool_total == Σ权重`、权重全正、`picked` 在池子里。
    """
    ri = q.table("round_inputs")
    bad: list[str] = []
    draws = multi = 0
    for r in ri.to_dict("records"):
        rnd = int(r["round"])
        for roll in (r.get("rolls") or []):
            if roll.get("purpose") != "route":
                continue
            draws += 1
            who = roll.get("subject")
            pool = roll.get("pool") or []
            total = roll.get("pool_total")
            picked = roll.get("picked")
            val = roll.get("value")
            if not pool:
                bad.append(f"r{rnd} {who}：route 抽签没有候选池")
                continue
            if any((p.get("weight") or 0.0) <= 0.0 for p in pool):
                bad.append(f"r{rnd} {who}：候选池里有非正的权重")
            ssum = sum(float(p.get("weight") or 0.0) for p in pool)
            if total is None or abs(ssum - total) > 1e-9 * max(1.0, abs(total)):
                bad.append(f"r{rnd} {who}：pool_total={total} ≠ Σ权重={ssum}")
            names = [p.get("name") for p in pool]
            if len(set(names)) != len(names):
                bad.append(f"r{rnd} {who}：候选池里有重名腿")
            for n in names:
                parts = (n or "").split("→")
                if len(parts) != 2 or not parts[0] or not parts[1] or parts[0] == parts[1]:
                    bad.append(f"r{rnd} {who}：腿名 {n!r} 不是 A→B（或两端相同）")
            if picked not in names:
                bad.append(f"r{rnd} {who}：picked={picked!r} 不在候选池里")
                continue
            if len(pool) >= 2:
                multi += 1
            # 精确复算 `route_for` 的切段：x = value × total；落在哪段就选哪条。
            x = float(val) * float(total)
            got = pool[-1].get("name")
            for p in pool:
                if x < float(p["weight"]):
                    got = p["name"]
                    break
                x -= float(p["weight"])
            if got != picked:
                bad.append(f"r{rnd} {who}：picked={picked!r} ≠ value×total 落在的 {got!r}")
    return {"draws": draws, "multi": multi, "bad": bad}


def extract(dirpath):
    """事件层 + 舰表 + 城表 + 编年史 → 一份小结（按投影缓存成 pickle）。

    返回 dict（不是每回合一行的表）：这一组的判据本来就只需要计数 + 违规样例 + 少量序列。
    """
    q = KIT.load(str(dirpath), only=("events", "ships", "cities", "faction_process", "blueprints",
                                      "market_trades", "depots", "factions",
                                      "round_inputs", "body_positions", "haul_steps"))
    ev = q.table("events")
    fac_all = q.table("factions")

    # ① 被拆平的城，**同回合内**不该被它自己的旧主复垦（一对净效果为零的事件）。
    #    `city_razed`：target = 城、`data.owner` = 丢城的一方；`colony_founded`：actor = 建城方。
    #    顺序按 `seq` 判（拆平在前、复垦在后才是要抓的那条路径）。
    losers: dict[tuple, tuple] = {}
    razings = 0
    for _, r in ev[ev["type"] == "city_razed"].iterrows():
        razings += 1
        data = r["data"] if isinstance(r["data"], dict) else {}
        losers[(int(r["round"]), r["target_id"])] = (int(r["seq"]), data.get("失城方"))
    bad: list[str] = []
    for _, r in ev[ev["type"] == "colony_founded"].iterrows():
        hit = losers.get((int(r["round"]), r["target_id"]))
        if hit and hit[0] < int(r["seq"]) and hit[1] == r["actor_id"]:
            bad.append(f"r{int(r['round'])}：{r['target_id']} 被 {hit[1]} 丢掉后又被同一个势力复垦")

    # ② 「定制化」（装了组件的）**活舰**在整局里出现过——累计口径（末回合快照会随轨迹归零）。
    ships = q.table("ships")
    fitted = ships[ships["船体"] > 0]["组件"]
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
    ids = id_report(q)

    # ⑦ 剧情的机械后果（`src/tests/sim/story.rs::story_effects_apply` 那一条，2026-10 第 7 批）。
    #    节拍清单与**后果**都从 `meta.story` 读（不写死「prologue 在第 1 回合给谁降多少」），
    #    逐条对着 `factions.关系` 看方向对不对。⚠ 只判**方向**：关系每回合还会被外交漂移
    #    与噪声推着走，`after - before == delta` 不是不变量（Rust 原件也只判了方向）。
    rel_at = {(int(r["round"]), r["势力"]): (r["关系"] or {}) for _, r in q.table("factions").iterrows()}
    story_bad: list[str] = []
    rel_checked = 0
    for b in q.meta.get("story") or []:
        trig = b.get("trigger") or {}
        rnd = int(trig.get("round") or 0)
        if trig.get("kind") != "round_at" or not 1 <= rnd <= ROUNDS:
            continue
        if not any(c["id"] == b["id"] for c in chronicle):
            continue
        for e in b.get("effects") or []:
            if e.get("kind") != "relations":
                continue
            a, other, delta = e["a"], e["b"], float(e["delta"])
            before = (rel_at.get((rnd - 1, a)) or {}).get(other)
            after = (rel_at.get((rnd, a)) or {}).get(other)
            if before is None or after is None:
                story_bad.append(f"{b['id']}：读面上找不到 {a}↔{other} 的关系")
                continue
            rel_checked += 1
            if (delta > 0 and not after > before) or (delta < 0 and not after < before):
                story_bad.append(f"r{rnd} {b['id']}：{a}↔{other} 该{'升' if delta > 0 else '降'} "
                                 f"{delta:+g}，却 {before:g}→{after:g}")

    # ⑧ 剧情 `grant_ship` 的出厂位置（`story.rs::story_grant_ship_spawns_a_fleet_member`，
    #    2026-10 第 7 批）：读面补了 `body_positions` 之后，「出厂位置 = 天体这一刻的位置 +
    #    (0.05, 0.05)」终于判得了（以前 `bodies` 是静态母表，只能靠引擎内部算）。
    bpos = {(int(r["round"]), r["天体名"]): (r["x"], r["y"])
            for _, r in q.table("body_positions").iterrows()}
    by_round = {}
    for _, s in ships.iterrows():
        by_round.setdefault(int(s["round"]), {})[s["舰名"]] = s
    grant_bad: list[str] = []
    grant_n = 0
    for b in q.meta.get("story") or []:
        trig = b.get("trigger") or {}
        rnd = int(trig.get("round") or 0)
        if trig.get("kind") != "round_at" or not 1 <= rnd <= ROUNDS:
            continue
        if not any(c["id"] == b["id"] for c in chronicle):
            continue
        for e in b.get("effects") or []:
            if e.get("kind") != "grant_ship":
                continue
            at, before = by_round.get(rnd, {}), by_round.get(rnd - 1, {})
            fresh = [s for n, s in at.items()
                     if n not in before and s["势力"] == e["faction"] and s["舰级"] == e["class"]]
            if not fresh:
                grant_bad.append(f"r{rnd} {b['id']}：{e['faction']} 没有多出一艘 {e['class']}")
                continue
            grant_n += 1
            for s in fresh:
                bp = bpos.get((rnd, e["body"]))
                if bp is None:
                    grant_bad.append(f"r{rnd} {b['id']}：读面上没有 {e['body']} 这一回合的位置")
                    continue
                # ⚠ 读面的 x/y 都过 `r2`（两位小数）⇒ 偏移只能判到 ±0.01，判不到 1e-9。
                off = (round(s["x"] - bp[0], 4), round(s["y"] - bp[1], 4))
                if abs(off[0] - 0.05) > 0.0101 or abs(off[1] - 0.05) > 0.0101:
                    grant_bad.append(f"r{rnd} {s['舰名']}：出厂偏移 {off} ≠ (0.05, 0.05)")
                if s["order_effective"] != "Idle":
                    grant_bad.append(f"r{rnd} {s['舰名']}：出厂指令 {s['order_effective']} ≠ Idle")

    # ⑨ 贸易禁运名单（`sim/tests/trade.rs::trade_block_list_names_the_blocker_and_the_tier`，
    #    第 7 批）。以前只有 `--derived` 的 `metrics.factions[].trade_blocked_by` 读得到 ⇒
    #    投影侧给 `factions` 补了一列 `贸易禁运`（`{禁运方: 档位}`）。这里只判**结构**，
    #    「与引擎判据同源」那半在 `trade_block_checks` 里拿 `--call trade_block_cause` 复核。
    known = set(fac_all["势力"])
    rel_at = {(int(r["round"]), r["势力"]): (r["关系"] or {}) for _, r in fac_all.iterrows()}
    war_threshold = float((q.meta.get("combat") or {}).get("war_threshold") or 0.0)
    TIERS = {"war", "cold", "coalition"}
    tb_bad: list[str] = []
    tb_n = 0
    tb_causes: set = set()
    tb_at_round: dict = {}
    for _, r in fac_all.iterrows():
        rnd, fid = int(r["round"]), r["势力"]
        for blocker, cause in (r["贸易禁运"] or {}).items():
            tb_n += 1
            tb_causes.add(cause)
            where = f"r{rnd} {blocker}→{fid}"
            tb_at_round.setdefault(rnd, []).append((blocker, fid, cause))
            if blocker == fid:
                tb_bad.append(f"{where}：把自己列进禁运名单了")
            elif blocker not in known:
                tb_bad.append(f"{where}：{blocker} 不是这个世界的势力")
            elif cause not in TIERS:
                tb_bad.append(f"{where}：没见过这一档（{cause}）")
            elif cause == "war":
                # ⚠ `war` 档 = **敌对**（`hostile` = 关系 ≤ `combat.war_threshold`），
                # 不是「宣战过」——第一版拿 `war_started` 重建去对，整片假红。
                a = (rel_at.get((rnd, blocker)) or {}).get(fid)
                b = (rel_at.get((rnd, fid)) or {}).get(blocker)
                if a is None or b is None:
                    tb_bad.append(f"{where}：读面上读不到这一对的关系")
                elif min(a, b) > war_threshold + 1e-9:
                    tb_bad.append(f"{where}：报的是战争禁运，但两边关系 {a:g}/{b:g} "
                                  f"都在交战阈值 {war_threshold:g} 之上")

    # 战争疤痕那条（第 7 批）要：① 一对真开过战的势力 + 那一回合；② 一对**从没打过仗**的势力。
    starts = [(int(r["round"]), tuple(sorted((r["actor_id"], r["target_id"]))))
              for _, r in ev[ev["type"] == "war_started"].iterrows()]
    fought = {p for _, p in starts}
    all_f = sorted(set(fac_all["势力"]))
    never = next(((x, y) for i, x in enumerate(all_f) for y in all_f[i + 1:]
                  if (x, y) not in fought and (y, x) not in fought), None)

    return {"razings": razings, "refound_bad": bad, "customized": customized,
            "war_first": starts[0] if starts else None, "war_never": never,
            "ships": int(len(ships)), "foundings": int((ev["type"] == "colony_founded").sum()),
            "chronicle": chronicle, "war_durations": durations, "wars_open": len(open_wars),
            "city_checked": city_checked, "city_unexplained": city_unexplained,
            "ship_deaths": deaths, "ship_births": births, "ship_unexplained": ship_unexplained,
            "flips": flips, "flip_bad": flip_bad,
            "headline_checked": hl_checked, "headline_bad": hl_bad,
            "launch": _launch_report(ships),
            "haul_leg": haul_leg_report(q),
            "trade_block": {"n": tb_n, "bad": tb_bad[:4], "causes": sorted(tb_causes),
                            "rounds": sorted(tb_at_round), "by_round": tb_at_round},
            "story_bad": story_bad, "story_rel_checked": rel_checked,
            "grant_bad": grant_bad, "grant_n": grant_n,
            "combat": combat, "blueprints": blueprints, "ids": ids, "trade": trade_report(q), "cargo": cargo_report(q),
            "depot": depot_report(q), "dispatch": dispatch_report(q),
            "meta": q.meta}


def trade_checks(h, ck, out) -> None:
    """贸易三账（B3）：`src/tests/sim/trade.rs` 搬过来的三条判据。"""
    tag = f"{len(SEEDS)} seed × {ROUNDS} 回合"
    rep = [d["trade"] for d in out]
    trades = sum(r["trades"] for r in rep)

    recon_bad = [(s, m) for s, r in zip(SEEDS, rep) for m in r["recon_bad"]]
    ck.check("每笔成交的实收都归到世界账（成交清单 ↔ view.market_settled 逐回合对账）",
             not recon_bad,
             "；".join(m for _, m in recon_bad[:3]) or
             f"{tag}：{trades:,} 笔成交、{sum(r['recon_rounds'] for r in rep)} 个成交回合全部对账")
    ck.check("成交对账没有空转（真的成交过）", trades >= MIN_TRADES,
             f"{tag} 共 {trades:,} 笔成交（下限 {MIN_TRADES}）")

    price_bad = [(s, m) for s, r in zip(SEEDS, rep) for m in r["price_bad"]]
    seen = {k: sum(r["price_seen"][k] for r in rep) for k in ("freight", "same_body", "mond")}
    ck.check("价格分解逐项自洽（拿记录下来的 depth/dist 与 meta.market 重算）",
             not price_bad,
             "；".join(m for _, m in price_bad[:3]) or
             f"{tag}：{trades:,} 笔的 mond_extra / freight_rate 全部咬合")
    ck.check("价格分解没有空转（真出现过跨天体运费与同天体零运费两档）",
             seen["freight"] >= 1 and seen["same_body"] >= 1,
             f"跨天体运费 {seen['freight']} 笔、同天体零运费 {seen['same_body']} 笔"
             f"（穿带溢价 {seen['mond']} 笔，轨迹相关，不强制出现）")

    rank_bad = [(s, m) for s, r in zip(SEEDS, rep) for m in r["rank_bad"]]
    rank_rounds = sum(r["rank_rounds"] for r in rep)
    ck.check("买方名次就是引擎排好的购买力序（降序、同额按名字升序、名次是 0..n-1）",
             not rank_bad,
             "；".join(m for _, m in rank_bad[:3]) or
             f"{tag}：{rank_rounds} 个市场的排队全部是 0..n-1 的购买力序")
    ck.check("名次守卫没有空转（真有排过队的回合）", rank_rounds >= MIN_RANK_ROUNDS,
             f"{tag} 共 {rank_rounds} 个回合排过队（下限 {MIN_RANK_ROUNDS}）")


def cargo_checks(h, ck, out) -> None:
    """货舱（M2）：有效舱容公式 + 舰级舱容的设计裁决。"""
    tag = f"{len(SEEDS)} seed × {ROUNDS} 回合"
    rep = [d["cargo"] for d in out]
    checked = sum(r["checked"] for r in rep)
    damaged = sum(r["damaged"] for r in rep)
    with_cargo = sum(r["with_cargo"] for r in rep)

    formula_bad = [(s, m) for s, r in zip(SEEDS, rep) for m in r["formula_bad"]]
    ck.check("有效舱容 = 舰级舱容 × 战损折算 hull/hull_max（真实舰·回合逐行）", not formula_bad,
             "；".join(m for _, m in formula_bad[:3]) or
             f"{tag}：{checked:,} 个舰·回合的舱容都落在舍入区间里（其中 {damaged:,} 行受过伤）")
    ck.check("舱容守卫没有空转（真受过伤、真装过货）",
             checked >= MIN_CARGO_ROWS and damaged >= 10 and with_cargo >= 10,
             f"{checked:,} 个舰·回合（下限 {MIN_CARGO_ROWS}）、{damaged:,} 行 hull<hull_max、"
             f"{with_cargo:,} 行舱里有货")

    design_bad = [(s, m) for s, r in zip(SEEDS, rep) for m in r["design_bad"]]
    bulk = sorted({c for r in rep for c in r["bulk"]})
    ck.check("舰级舱容是设计裁决（护卫2 / 驱逐4 / 巡洋6 / 航母20 / 战列6，航母唯一散货船）",
             not design_bad and bulk == ["carrier"],
             "；".join(m for _, m in design_bad[:3]) or
             f"五个舰级全对；唯一散货船 = {bulk}（其余都 < 20）")


def depot_checks(h, ck, out) -> None:
    """产地货栈（M1）：首都即集散地 + `cities.depot_value` 的城视角合计。"""
    tag = f"{len(SEEDS)} seed × {ROUNDS} 回合"
    rep = [d["depot"] for d in out]
    rows = sum(r["rows"] for r in rep)
    nonzero = sum(r["nonzero"] for r in rep)
    stale = sum(r["stale"] for r in rep)
    same_round = sum(r["same_round"] for r in rep)

    pos_bad = [(s, m) for s, r in zip(SEEDS, rep) for m in r["pos_bad"]]
    ck.check("货栈行都是正的、不重复（稀疏：没积压的天体不占行）", not pos_bad,
             "；".join(m for _, m in pos_bad[:3]) or f"{tag}：{rows:,} 行全为正且唯一")

    unexplained = [(s, m) for s, r in zip(SEEDS, rep) for m in r["unexplained"]]
    ck.check("首都天体上没有凭空的货栈（首都即集散地；迁都的相位错位逐处解释）", not unexplained,
             "；".join(m for _, m in unexplained[:3]) or
             f"{tag}：{rows:,} 行里只有 {stale} 行 stale + {same_round} 行 same_round 落在首都天体上，全部有解释")
    ck.check("货栈守卫没有空转（真有货栈、也真解释过迁都）",
             rows >= MIN_DEPOT_ROWS and (stale + same_round) >= 1,
             f"{rows:,} 行货栈（下限 {MIN_DEPOT_ROWS}）、{stale} stale / {same_round} same_round")

    agg_bad = [(s, m) for s, r in zip(SEEDS, rep) for m in r["agg_bad"]]
    ck.check("cities.depot_value ≡ Σ depots × 资源价值（城视角合计）", not agg_bad,
             "；".join(m for _, m in agg_bad[:3]) or
             f"{tag}：{nonzero:,} 个「有积压的城·回合」的 depot_value 全部对得上")
    ck.check("合计守卫没有空转（真有积压的城）", nonzero >= 10,
             f"{nonzero:,} 个「有积压的城·回合」（下限 10）")


def dispatch_checks(h, ck, out) -> None:
    """派单抽签（`autocontrol::freight`）：按积压占比切成区间。"""
    tag = f"{len(SEEDS)} seed × {ROUNDS} 回合"
    rep = [d["dispatch"] for d in out]
    draws = sum(r["draws"] for r in rep)
    multi = sum(r["multi"] for r in rep)
    bad = [(s, m) for s, r in zip(SEEDS, rep) for m in r["bad"]]
    ck.check("派单抽签逐条自洽（pool_total=Σ权重、picked=value×total 落在的那一段）",
             not bad,
             "；".join(m for _, m in bad[:3]) or
             f"{tag}：{draws:,} 条 route 抽签全部按池子占比落段（其中 {multi:,} 条多候选）")
    ck.check("派单抽签没有空转（真有抽签、且真有多候选）",
             draws >= MIN_ROUTE_DRAWS and multi >= MIN_ROUTE_DRAWS,
             f"{draws:,} 条 route 抽签（下限 {MIN_ROUTE_DRAWS}），多候选 {multi:,}")


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
    trade_checks(h, ck, out)
    cargo_checks(h, ck, out)
    depot_checks(h, ck, out)
    dispatch_checks(h, ck, out)
    new_ship_checks(h, ck, out)
    trade_block_checks(h, ck, out)
    war_scar_scenario_checks(h, ck, out)
    haul_leg_checks(h, ck, out)
    knowledge_scenario_checks(h, ck)
    governance_scenario_checks(h, ck)
    duel_scenario_checks(h, ck)
    pd_cover_scenario_checks(h, ck)
    intercept_scenario_checks(h, ck)
    follow_scenario_checks(h, ck)
    site_build_scenario_checks(h, ck)
    ideology_war_scenario_checks(h, ck)
    build_line_scenario_checks(h, ck)
    gov_plate_scenario_checks(h, ck)
    affinity_scenario_checks(h, ck)
    blueprint_checks(h, ck, out)
    id_checks(h, ck, out)
    scenario_checks(h, ck)
    blueprint_scenario_checks(h, ck)
    capital_scenario_checks(h, ck)
    dock_scenario_checks(h, ck)
    stale_follow_scenario_checks(h, ck)
    site_ledger_checks(h, ck)
    blueprint_launch_checks(h, ck)
    combat_scenario_checks(h, ck)
    commanded_haul_checks(h, ck)
    welfare_scenario_checks(h, ck)
    colonize_scenario_checks(h, ck)
    defection_scenario_checks(h, ck)
    story_checks(h, ck, out)
    war_floor_checks(h, ck, out)


def scenario_checks(h, ck) -> None:
    """**合成场景**：造世界 → 用 Python 改档 → 推进 → 只看数据断言。

    这些用例以前只能在 Rust 里做（`fresh_world(42)` + `state.ship_mut(&name).hull = 6.0` +
    调内部 API 断言）。现在档能存成 JSON（`--save w.json`，见 `config::CheckpointFormat`）：
    「造」用 `planet_x`、「捏」用 Python 的 `json` 模块（`h.gen(patch=…)`），**断言仍然只看投影**。

    搬过来的两条（各删掉一条 Rust 原件）：

    ① `combat::damaged_ship_regenerates_hull_each_round`——把一艘舰的船体改成一半、扔到
       `[80, 80]`（远离本土），每回合应当**恰好**长回 `hull_max × (hull_regen + 本土加成)`：
       本土加成的有无是**两个离散值**（在不在自家 `home_radius` 内），所以「增量 ∈ {基础, 基础+
       加成}」是可以逐位判的，不是模糊的「大概长了点」。
    ② `spending::labor_and_housing_capacity_are_captured_from_this_steps_state`——把一座城的
       人口压到 1 ⇒ 用工系数掉到 `min_efficiency`（配置值，不是写死的 0.1）；而且**回合 0 那一行
       的中性值是 1.0**（「这一步还没跑」= 不缺人手，不是「全城没人上工」）。
    """
    # ① 护甲再生
    seed = 42
    w = h.gen(CACHE_ROOT / "scenario" / "_probe.json", seed)
    st = h.state_dump(w)
    ship = st["ships"][0]
    name, hmax = ship["舰名"], float(ship["船体上限"])
    proj = h.scenario("regen", seed, 3, patch={"ships": {name: {"船体": hmax / 2, "坐标": [80.0, 80.0]}}})
    q = KIT.load(str(proj), only=("ships", "factions"))
    rows = q.table("ships")
    mine = rows[rows["舰名"] == name].sort_values("round")
    steps, capped, bad = 0, 0, []
    hulls = list(mine["船体"])
    regen = float(mine["hull_regen"].iloc[0])
    bonus = float(q.table("factions").pipe(lambda t: t[t["势力"] == ship["势力"]])[
        "本土再生加成"].iloc[0])
    base, boosted = regen * hmax, (regen + bonus) * hmax
    for i in range(len(hulls) - 1):
        delta = hulls[i + 1] - hulls[i]
        if abs(delta) < 1e-9:
            capped += 1                       # 已经满了（不超上限）
        elif abs(delta - base) < 1e-6 or abs(delta - boosted) < 1e-6:
            steps += 1
        else:
            bad.append(f"r{i}→r{i + 1}: 增量 {delta:.6f} 既不是基础 {base:.6f} 也不是含加成 {boosted:.6f}")
    ck.check("合成场景：受伤的舰每回合按 hull_max × 再生率长回来（本土加成是另一个离散值）",
             not bad and steps >= 2,
             "；".join(bad[:2]) or
             f"{name}（{hmax:g} 船体）：船体 {hulls[0]:.2f} → {hulls[-1]:.2f}，{steps} 次按 {base:.4f}/回合 增长"
             f"（含加成档 {boosted:.4f}，另有 {capped} 次已满）")
    ck.check("合成场景：护甲再生守卫没有空转（真的有可长回来的回合）", steps >= 2, f"{steps} 次增量（下限 2）")

    # ② 用工系数
    city = max(st["cities"], key=lambda c: c["人口"])
    proj2 = h.scenario("labor", seed, 3, patch={"cities": {city["城名"]: {"人口": 1}}})
    q2 = KIT.load(str(proj2), only=("city_process",))
    cp = q2.table("city_process")
    mine2 = cp[cp["城名"] == city["城名"]].sort_values("round")
    floor = float(q2.meta["economy"]["min_efficiency"])
    labor = list(mine2["labor"])
    housing = float(mine2["housing_capacity"].iloc[-1])
    ck.check("合成场景：人口压到 1 ⇒ 用工系数掉到 min_efficiency（配置值，不是写死的）",
             len(labor) >= 2 and abs(labor[0] - 1.0) < 1e-12 and abs(labor[1] - floor) < 1e-9,
             f"{city['城名']}：回合 0 用工 {labor[0]:g}（没跑那一步 ⇒ 中性值 1.0）→ "
             f"回合 1 {labor[1]:.4f}（下限 {floor:g}），住房容量 {housing:g}")
    ck.check("合成场景：用工系数守卫没有空转（真有住房容量可算）", housing > 0, f"住房容量 {housing:g}")


# ── 合成场景 · 拨控制叶（施工图 §5.6 第 6 批）────────────────────────────────────
#
# 这一族用例的共性：要**先在世界上做一件事**（钉一张玩家的图 + 让建造区指过去 / 让某根图指针悬空 /
# 造几张没人指向的图 / 把预算拨到两个极端），再看引擎那一趟怎么反应。以前只能在 Rust 里
# `fresh_world(42)` + 直接改内部状态；现在**全部走引擎自己的写入口**：
#
# * **拨控制叶**（`h.scenario_apply(diffs=…)`）：图库 / 建造区 / 预算——走 `--apply`（补丁形状由
#   g4 逐条对账）⇒ Python 里不出现第二份控制面形状。
# * **悬空指针**也用 `--apply` 造，而且比改状态字段**更忠实**：先建图 + 把建造区指过去，再**删掉
#   那张图**——「删整张图 ⇒ 挂它的建造区随后是悬空指针 ⇒ 停产」（`src/control/blueprint.rs`），
#   而**删图本来就是玩家/agent 的动作**。反过来，直接往状态里塞一个不存在的图名是绕路：写面
#   **拒绝**写不存在的图（`no_such_blueprint`，那是写面契约，施工图 §4 明说不搬）。
# * 只剩一处**读状态字段**：从状态档里认出「这个势力有哪些建造区」。那处字段名是引擎的镜像，
#   见 `_yards_of`（身份键问引擎；其余名字写错**不会静默**——探针那条判据立刻红）。


def _yards_of(h, st: dict, faction: str) -> list[tuple[str, int, str]]:
    """状态档里某势力的**建造区**：`(城名, 建筑编号, 舰级)`。

    ⚠ 这里出现的是**引擎的状态字段名**（中文名词，见 [field-naming](../../.agents/notes/field-naming.md)）
    ——它们是镜像，所以按 `_harness` 的规矩分两半：

    * **身份键问引擎**（`h.identity_keys()`；唯一真值是 `model::IDENTITY`，随 `--nouns` 发出来）：
      城市的身份键直接取；「城市指向势力」那个字段取的是 `Faction` 的身份键——同一个名词。
    * 剩下三个（`建筑` / `建筑编号` / `建造舰级`）引擎**还没有声明面**，只能写在这儿。写错的代价
      是**响亮的**：一个建造区都找不到 ⇒ 下面「探针世界里真有可用的建造区」立刻红，而不是拿个
      旧名字去改档、再把人引向「补丁没落地」那个错方向。
    """
    keys = h.identity_keys()
    id_key, fac_key = keys["cities"], keys["factions"]
    out = []
    for c in st.get("cities") or []:
        if c.get(fac_key) != faction:
            continue
        for b in c.get("建筑") or []:
            if b.get("建造舰级"):
                out.append((c[id_key], b["建筑编号"], b["建造舰级"]))
    return out


def _yard_ptr_by_round(ci, city: str, bid: int) -> dict:
    """某个建造区的**设计图指针**逐回合（读面 `cities.建筑[].设计图`）。"""
    out = {}
    for _, r in ci[ci["城名"] == city].iterrows():
        for b in r["建筑"] or []:
            if b["建筑编号"] == bid:
                out[int(r["round"])] = b.get("设计图")
    return out


def _bp_rows(bps, fid: str, name: str) -> dict:
    """某张设计图逐回合的行：`{回合: 行}`（读面 `blueprints`）。"""
    mine = bps[(bps["势力"] == fid) & (bps["图名"] == name)]
    return {int(r["round"]): r for _, r in mine.iterrows()}


def _bp_decisions(dec, fid: str, name: str | None = None) -> list[dict]:
    """设计图的判定行（`kind=blueprint`）；`name` 给了就只看那一张图。"""
    rows = dec[(dec["kind"] == "blueprint") & (dec["势力"] == fid)]
    if name is not None:
        rows = rows[rows["actor"] == name]
    return rows.to_dict("records")


def _stock_patch(faction: str, amount: float, keys) -> dict:
    """`{factions: {势力: {资源: {…}}}}` 形状的**状态补丁**：把国库垫厚。

    为什么要垫：P1-5 之后 Player 写的 `construction_budget` 还要再乘一个维护 reserve 的
    `con_scale`（`autocontrol/budget.rs`）⇒ **库存不到 reserve 时写多大的预算都是 0**
    （详见施工图 §5.6 那个 ⚠）。这是**隔离变量**：不垫库存，「批满」根本没被批下去。

    ⚠ `资源` 与城那边的 `建筑`/`建造舰级` 一样是**引擎状态字段名的镜像**（引擎还没有声明面）。
    写错的代价是响亮的：`edit()` 找不到字段 ⇒ 进 `h.warnings`，`report()` 会印出来，而且
    「批满」那条判据立刻红。势力名与资源名都不用在这儿写死（前者是身份键，后者从 `--control` 来）。
    """
    return {"factions": {faction: {"资源": {k: amount for k in keys}}}}


def _build_lines(cp, city: str) -> list[tuple[int, str, dict]]:
    """某城逐回合的建造行：`(回合, 舰级, {rate, increment})`（稀疏：有建造区才有键）。"""
    out = []
    for _, r in cp[cp["城名"] == city].iterrows():
        for k, v in (r["build"] or {}).items():
            out.append((int(r["round"]), k, v))
    return out


def blueprint_scenario_checks(h, ck) -> None:
    """**合成场景 · 拨控制叶**：`autocontrol/blueprints.rs` + `sim/spending.rs` 剩下那几条。

    搬过来的：前三条**整条**（Rust 原件删了）；第四条搬**读面那一半**，另一半（`RoundSink`
    的支出账、单步语义）按 §4 留在 Rust。

    ① `a_dangling_pointer_is_left_dangling`——删图是玩家/agent 的动作，那个区停产**本身就是
       可见后果** ⇒ AI 不替他收拾（不建新图、不改指针）。
    ② `a_player_pinned_design_and_its_yard_are_left_alone`——玩家的图归玩家：不改图、不改指针、
       回收也不碰。
    ③ `only_unreferenced_selfmade_designs_are_reaped`——回收只碰**自己造的**：名字带 AI 前缀、
       没人指向、且**不是玩家钉的**（`mode: Inherit` 才是流水；`--apply` 只写值会把它钉成
       `Player`，那这条安全属性就整个反过来了）。
    ④ `build_lines_separate_the_money_bottleneck_from_the_capacity_ceiling` 的**读面那一半**
       ——同一座城只改预算一个变量：批 0 ⇒ 建造行**还在**、`rate > 0`、`increment = 0`（是缺钱
       不是没船坞）；批满 ⇒ 顶到产能上限；`rate` 与钱无关。⚠ 「批满」**要先垫国库**：P1-5 之后
       预算值还要乘维护 reserve 的 `con_scale`，库存不到 reserve 时写 1e6 也是 0（见下面那段注释
       与 Rust 原件 `sim/spending.rs` 的模块头）。
    """
    seed = SCENARIO_SEED
    st = h.state_dump(h.gen(CACHE_ROOT / "scenario" / "_bp_probe.json", seed))
    yards = _yards_of(h, st, FID)
    ck.check("合成场景（设计图）：探针世界里真有可用的建造区", len(yards) >= 2,
             f"{FID} 开局 {len(yards)} 个建造区：{yards}")
    if not yards:
        return
    city, bid, class_ = yards[0]
    meta = json.loads(h.capture(["--meta"]))
    slots = int(meta["ships"][class_]["slots"])
    comps = ["kinetic", "ion_drive"][:max(1, min(2, slots))]

    # ① 悬空指针 ---------------------------------------------------------------
    # 造法 = **两次 `--apply`**：先建一张普通的自建图、把建造区指过去，再**把图删掉** ⇒ 指针悬空。
    ghost = "待删的图"
    proj = h.scenario_apply("bp_dangling", seed, SCENARIO_ROUNDS, [
        {"control": [{"势力": FID,
                      "设计图库": [{"图名": ghost, "舰级": class_,
                                      "选装": ["kinetic"], "归属": "Inherit"}],
                      "建筑": [{"城": city, "建筑": bid, "设计图": ghost}]}]},
        {"control": [{"势力": FID, "设计图库": [{"图名": ghost, "删叶": True}]}]},
    ])
    q = KIT.load(str(proj), only=("cities", "blueprints", "decisions", "city_process", "ships"))
    ptrs = _yard_ptr_by_round(q.table("cities"), city, bid)
    ck.check("合成场景（设计图）：悬空指针原样保留（AI 不替你修）",
             bool(ptrs) and all(p == ghost for p in ptrs.values()),
             f"{city} 建筑{bid} 的指针逐回合：{ptrs}")
    # 防空转：那根指针**真的**是悬空的（不是「图还在、指针合法」蒙混过关）。
    lib = set(q.table("blueprints")["图名"])
    ck.check("合成场景（设计图）：悬空指针守卫没有空转（指针真的悬空）",
             ptrs.get(0) == ghost and ghost not in lib,
             f"「{ghost}」不在任何势力的图库里（全表 {len(lib)} 个图名）")
    # 防空转：这一趟 AI **真的**跑过（它给别人建了图）——否则「没修」可能只是「没跑」。
    acts = _bp_decisions(q.table("decisions"), FID)
    ck.check("合成场景（设计图）：AI 那一趟真的跑了（同一局里它给别的建造区建了图）",
             any(d["verdict"] in ("created", "reused") for d in acts),
             f"{FID} 的设计图判定 {len(acts)} 行：{sorted({d['verdict'] for d in acts})}")
    # `sim/blueprints.rs::a_dangling_blueprint_pointer_stops_the_yard`（第 7 批搬来）：
    # **停产 = `city_process.build` 里根本没有那一行**（不是 `increment = 0`——那是「缺钱」，
    # 见下面 ④；两种停法在读面上分得开）。防空转靠 ④：同一座城在批满时**有**建造行。
    dangling_rows = _build_lines(q.table("city_process"), city)
    ck.check("合成场景（设计图）：指针悬空 ⇒ 那个建造区连建造行都没有（进度一点不涨、也不下水）",
             not dangling_rows,
             f"{city} 在悬空窗口里的建造行：{dangling_rows}" if dangling_rows
             else f"{city} 在整个窗口里零建造行（{len(q.table('ships'))} 行舰表，没有新舰）")

    # ② 玩家的图一个字都不许动 -------------------------------------------------
    mine = "玩家的守卫图"
    diff = {"control": [{"势力": FID,
                         "设计图库": [{"图名": mine, "舰级": class_, "选装": comps}],
                         "建筑": [{"城": city, "建筑": bid, "设计图": mine}]}]}
    proj = h.scenario_apply("bp_pinned", seed, SCENARIO_ROUNDS, [diff])
    q = KIT.load(str(proj), only=("cities", "blueprints", "decisions"))
    rows = _bp_rows(q.table("blueprints"), FID, mine)
    ck.check("合成场景（设计图）：玩家钉住的图归玩家、选装一个字都没动",
             bool(rows) and all(str(r["mode"]) == "Player" and list(r["选装"] or []) == comps
                                for r in rows.values()),
             f"「{mine}」逐回合：{[(k, str(v['mode']), list(v['选装'] or []))
                                   for k, v in sorted(rows.items())]}")
    ptrs = _yard_ptr_by_round(q.table("cities"), city, bid)
    ck.check("合成场景（设计图）：玩家指过去的建造区不许被改派",
             bool(ptrs) and all(p == mine for p in ptrs.values()), f"指针逐回合：{ptrs}")
    touched = _bp_decisions(q.table("decisions"), FID, mine)
    ck.check("合成场景（设计图）：重估/回收都不许碰玩家的图（decisions 里零行）", not touched,
             f"「{mine}」的判定行：{touched}" if touched else "0 行")
    acts = _bp_decisions(q.table("decisions"), FID)
    ck.check("合成场景（设计图）：玩家图守卫没有空转（AI 那一趟真的跑了）",
             any(d["verdict"] in ("created", "reused") for d in acts),
             f"{FID} 的设计图判定 {len(acts)} 行：{sorted({d['verdict'] for d in acts})}")

    # ③ 回收只碰自己造的 -------------------------------------------------------
    aic, mine2, pinned = f"{DESIGN_PREFIX}强袭·陈图", "玩家自己的图", f"{DESIGN_PREFIX}堡垒·玩家钉的"
    diff = {"control": [{"势力": FID, "设计图库": [
        {"图名": aic, "舰级": class_, "选装": ["kinetic"], "归属": "Inherit"},
        {"图名": mine2, "舰级": class_, "选装": ["kinetic"]},
        {"图名": pinned, "舰级": class_, "选装": []},
    ]}]}
    proj = h.scenario_apply("bp_reap", seed, SCENARIO_ROUNDS, [diff])
    q = KIT.load(str(proj), only=("blueprints", "decisions"))
    bps = q.table("blueprints")
    a, b, c = (_bp_rows(bps, FID, n) for n in (aic, mine2, pinned))
    mode_at0 = {n: (str(r[0]["mode"]) if 0 in r else None)
                for n, r in ((aic, a), (mine2, b), (pinned, c))}
    # 防空转：三张图**真的按预期建出来了**（各有回合 0 的行与模式），否则「没了/还在」判不出。
    ck.check("合成场景（设计图）：三张图按预期建成（自建=流水、另两张=玩家）",
             mode_at0[aic] == "Inherit" and mode_at0[mine2] == "Player"
             and mode_at0[pinned] == "Player",
             f"回合 0 的模式：{mode_at0}")
    ck.check("合成场景（设计图）：没人指向的自建图被回收了（回合 0 还在、之后没了）",
             bool(a) and 0 in a and all(r not in a for r in range(1, SCENARIO_ROUNDS + 1)),
             f"「{aic}」出现的回合：{sorted(a)}")
    ck.check("合成场景（设计图）：不是 AI 命名的图不许碰",
             all(r in b for r in range(SCENARIO_ROUNDS + 1)),
             f"「{mine2}」出现的回合：{sorted(b)}")
    ck.check("合成场景（设计图）：玩家钉住的 AI 命名图不许回收",
             all(r in c for r in range(SCENARIO_ROUNDS + 1)),
             f"「{pinned}」出现的回合：{sorted(c)}")
    reaped = [d for d in _bp_decisions(q.table("decisions"), FID) if d["verdict"] == "reaped"]
    ck.check("合成场景（设计图）：回收在 decisions 里留了痕（verdict=reaped）",
             any(d["actor"] == aic for d in reaped),
             f"{len(reaped)} 条回收判定：{[d['actor'] for d in reaped][:4]}")

    # ④ 造舰慢是缺钱还是缺产能 -------------------------------------------------
    # 一份 diff 干三件事：**拆掉图指针 + 钉死舰级**（否则 `retool_shipyards` 中途改装，那一行就
    # 找不到了），再把两个预算拨到同一个极端值。资源清单**问引擎**（`--control` 的预算模板：
    # 每个势力的资源键与它逐一对上），不手抄状态字段。
    ctl = json.loads(h.capture(["--seed", str(seed), "--control"]))["control"]
    # `--control` 读面的字段名就是控制面的中文名（`建造预算`/`投资预算`）。
    res = sorted({e["资源"] for f in ctl if f["势力"] == FID
                  for k in ("建造预算", "投资预算") for e in f[k]})
    ck.check("合成场景（预算）：预算模板给出了这个势力的资源清单（防空转）", len(res) >= 1,
             f"{FID} 的预算资源：{res}")

    # ⚠ **国库要先垫厚**：P1-5 之后 `construction_budget` 的值会再乘一个维护 reserve 的
    # `con_scale`（`autocontrol/budget.rs`：`(库存价值 − 维护 reserve) / 建舰上限`，夹进 [0,1]）
    # ——**库存不到 reserve 时写多大的预算都是 0**。所以要测「钱 vs 产能」，得先把库存垫到
    # reserve 之上；这是**同一把尺子**，Rust 原件（`src/tests/sim/spending.rs`）现在也这么做。
    stock = _stock_patch(FID, 1e6, res)

    def leaves(v: float) -> dict:
        return {"control": [{"势力": FID,
                             "建筑": [{"城": city, "建筑": bid,
                                       "设计图": None, "建造舰级": class_}],
                             "建造预算": [{"资源": r, "值": v} for r in res],
                             "投资预算": [{"资源": r, "值": v} for r in res]}]}

    lines = {}
    for tag, v in (("rich", 1e6), ("poor", 0.0)):
        p = h.scenario_apply(f"bp_budget_{tag}", seed, SCENARIO_ROUNDS, [leaves(v)], patch=stock)
        lines[tag] = _build_lines(KIT.load(str(p), only=("city_process",)).table("city_process"), city)

    rich = [(r, k, l) for r, k, l in lines["rich"] if k == class_]
    capped = [(r, k, l) for r, k, l in rich
              if l["rate"] > 0 and abs(l["increment"] - l["rate"]) <= 1e-9 * max(1.0, abs(l["rate"]))]
    ck.check("合成场景（预算）：批满 ⇒ 顶到产能上限（钱管够，是船坞的产能封顶）",
             bool(rich) and len(capped) == len(rich),
             f"{city} 的 {class_}：{[(r, round(l['rate'], 4), round(l['increment'], 4)) for r, _, l in rich]}")
    poor = lines["poor"]
    zero = all(float(l["increment"]) == 0.0 for _, _, l in poor)
    pos = [(r, k, l) for r, k, l in poor if l["rate"] > 0]
    ck.check("合成场景（预算）：批 0 ⇒ 建造行还在、rate > 0，而 increment = 0（是缺钱，不是没船坞）",
             bool(poor) and zero and bool(pos),
             f"{city} 逐行：{[(r, k, round(l['rate'], 4), round(l['increment'], 4)) for r, k, l in poor][:4]}"
             f"（{len(poor)} 行，其中 {len(pos)} 行 rate > 0）")
    # `rate` 是产能、与钱无关：同一座城同一舰级，两种预算给**同一个**值。
    rates = {}
    for tag, rows in (("rich", rich), ("poor", [(r, k, l) for r, k, l in poor if k == class_])):
        rates[tag] = {(r, round(l["rate"], 6)) for r, _, l in rows if l["rate"] > 0}
    ck.check("合成场景（预算）：rate 是产能、与批了多少钱无关（同一舰级两种预算同一个 rate）",
             bool(rates["rich"]) and rates["rich"] == rates["poor"],
             f"{class_} 的 rate：批满 {sorted(rates['rich'])} / 批 0 {sorted(rates['poor'])}")
    # ① 的反向防空转：同一座城在**批满**时是有建造行的（所以「悬空 ⇒ 零行」不是「这城不造船」）。
    ck.check("合成场景（设计图）：悬空那半的防空转（同一座城批满时有建造行）",
             bool(rich), f"{city} 批满时 {class_} 的建造行 {len(rich)} 条（悬空那臂是 0 条）")

    # ⑤ 买不起 ⇒ 在等钱（`sim/blueprints.rs::a_player_blueprint_that_cannot_be_afforded_waits_for_money`）
    #    **A/B 两侧只差国库一个变量**：同一个图、同一根指针，一次把国库清零、一次垫厚。
    #    `blueprints.launch_waiting` 就是那个「进度满了却没下水」的可见标记。
    rich_name = "豪华护卫"
    # 中国**开局国库里真有的键**（读面 `factions.资源`）——用来防空转：那份选装要的货里有它
    # 没有的东西，「买不起」才是真的买不起（第一版把预算表的键当国库键，垫厚那臂其实还是穷的）。
    fr = KIT.load(str(h.projection(seed, SCENARIO_ROUNDS)), only=("factions",)).table("factions")
    stock_keys = sorted((list(fr[(fr["round"] == 0) & (fr["势力"] == FID)]["资源"])[0] or {}).keys())
    cost = None
    arms = {}
    for tag, amt in (("poor", 0.0), ("rich", 1e6)):
        proj = h.scenario_apply(f"bp_afford_{tag}", seed, AFFORD_ROUNDS, [
            {"control": [{"势力": FID,
                          "设计图库": [{"图名": rich_name, "舰级": class_,
                                          "选装": ["plasma", "ion_drive"]}],
                          "建筑": [{"城": city, "建筑": bid, "设计图": rich_name}]}]}],
            patch=_stock_patch(FID, amt, AFFORD_RESOURCES))
        q = KIT.load(str(proj), only=("blueprints", "ships", "city_process"))
        bp = _bp_rows(q.table("blueprints"), FID, rich_name)
        sh = q.table("ships")
        sh = sh[sh["出厂图"] == rich_name]
        cost = cost or next((r["component_cost"] for r in bp.values() if r["component_cost"]), None)
        arms[tag] = {
            "waiting": [(r, bool(bp[r]["launch_waiting"])) for r in sorted(bp)],
            "launched": sorted({int(x) for x in sh["round"]}),
            "inc": [v["increment"] for _, _, v in _build_lines(q.table("city_process"), city)],
        }
    ck.check("合成场景（设计图）：那份选装真的要国库里没有的货（否则「买不起」是废话）",
             bool(cost) and bool(set(cost) - set(stock_keys)),
             f"「{rich_name}」的组件成本 {cost}；中国开局国库只有 {stock_keys}")
    ck.check("合成场景（设计图）：买不起 ⇒ 一次都不下水，且 `launch_waiting` 亮起来（等待可见）",
             not arms["poor"]["launched"] and any(w for _, w in arms["poor"]["waiting"]),
             f"清空国库那一臂：下水 {arms['poor']['launched']}、"
             f"launch_waiting {arms['poor']['waiting']}")
    ck.check("合成场景（设计图）：进度继续攒（车坞没停，是钱没到）",
             bool(arms["poor"]["inc"]) and any(v > 0 for v in arms["poor"]["inc"]),
             f"清空国库那一臂的逐回合 increment：{[round(v, 3) for v in arms['poor']['inc']]}")
    ck.check("合成场景（设计图）：垫厚国库 ⇒ 同一个图就下水了（A/B 的另一半，进度没丢）",
             len(arms["rich"]["launched"]) >= 1,
             f"垫厚那一臂：下水回合 {arms['rich']['launched']}、"
             f"launch_waiting {arms['rich']['waiting']}")


def dock_scenario_checks(h, ck) -> None:
    """**合成场景 · 停泊与待命**：`sim/tests/fleet.rs::dock_follows_body_and_idle_holds_position`。

    这条以前搬不动——「Dock 跟不跟得上公转」要**逐回合的天体位置**，而 `bodies` 是静态母表
    （只有轨道根数）。第 7 批补了派生表 `body_positions`（一行 = 一个天体这一回合的位置，
    与 `ships.x/y` 同一把绝对坐标尺子）⇒ 现在读得出来了。

    ⚠ **必须钉成 `Player`**：AI 会在回合末刚派完 Dock、下一回合开头就改派（实测长局里
    `Dock` 的 797 个「两回合同天体」样本**全部原地没动**，就是这种没执行过的叶子）。
    钉住归属才是这条用例本来测的东西。

    **防空转的一半在 `Idle` 那边**：天体在走，所以「位置不动」不是「世界静止」的必然结果——
    那个舰到海王星的距离每回合都在变（30.733 → 30.737），变的只有天体。
    """
    seed = SCENARIO_SEED
    diff = {"control": [{"势力": FID, "指令": [
        {"舰": DOCK_SHIP, "行为": {"Dock": {"body": DOCK_BODY}}, "归属": "Player"},
        {"舰": IDLE_SHIP, "行为": "Idle", "归属": "Player"},
    ]}]}
    proj = h.scenario_apply("fleet_dock", seed, DOCK_ROUNDS, [diff])
    q = KIT.load(str(proj), only=("ships", "body_positions"))
    sh, bp = q.table("ships"), q.table("body_positions")

    body = {(int(r["round"]), r["天体名"]): (r["x"], r["y"]) for _, r in bp.iterrows()}
    ck.check("合成场景（停泊）：探针世界里那两艘舰与那个天体都在（防空转）",
             len(sh[sh["舰名"] == DOCK_SHIP]) >= 2 and len(sh[sh["舰名"] == IDLE_SHIP]) >= 2
             and len(body) >= DOCK_ROUNDS,
             f"{DOCK_SHIP}/{IDLE_SHIP} 各 {len(sh[sh['舰名'] == DOCK_SHIP])} 行、"
             f"天体位置 {len(body)} 行")

    rows = {n: sh[sh["舰名"] == n].sort_values("round") for n in (DOCK_SHIP, IDLE_SHIP)}
    dock_seq = [(int(r["round"]), r["order_effective"], (r["x"], r["y"])) for _, r in rows[DOCK_SHIP].iterrows()]
    idle_seq = [(int(r["round"]), r["order_effective"], (r["x"], r["y"])) for _, r in rows[IDLE_SHIP].iterrows()]

    # ① Dock 的**指令持久**（不退化成 Idle）。
    want_dock = {"Dock": {"body": DOCK_BODY}}
    drifted = [r for r, e, _ in dock_seq if e != want_dock]
    ck.check(f"合成场景（停泊）：`Dock` 指令逐回合持久（不退化成 Idle）", not drifted,
             f"{len(dock_seq)} 个回合全在，指令 = {dock_seq[0][1]}")

    # ② Dock 的舰**真的朝那个天体去**：逐回合与天体当前位置的距离单调缩短。
    dist = [(r, math.dist(p, body[(r, DOCK_BODY)])) for r, _, p in dock_seq if (r, DOCK_BODY) in body]
    grew = [(a, b) for (ra, a), (rb, b) in zip(dist, dist[1:]) if b > a + 1e-9]
    ck.check("合成场景（停泊）：`Dock` 的舰逐回合朝目标天体靠近（在走，不是冻着）",
             len(dist) >= 3 and not grew,
             f"{DOCK_SHIP} 到 {DOCK_BODY} 的距离：{[round(d, 3) for _, d in dist[:6]]}"
             + (f"…；逆增 {grew[:2]}" if grew else "（逐回合缩短）"))

    # ③ Idle 的舰**位置逐字不动**。
    moved = [(r, p) for (_, _, p0), (r, _, p) in zip(idle_seq, idle_seq[1:]) if p != p0]
    ck.check("合成场景（停泊）：`Idle` 的舰位置逐字不动（待命就是待命，不漂移）", not moved,
             "；".join(f"r{r} 跑到 {p}" for r, p in moved[:3]) or f"{len(idle_seq)} 个回合同一个坐标 {idle_seq[0][2]}")

    # ④ 防空转的另一半：窗口里天体**真的在动**（否则 ③ 是「世界静止」的废话）。
    bmove = [math.dist(body[(r, DOCK_BODY)], body[(r + 1, DOCK_BODY)])
             for r in range(DOCK_ROUNDS) if (r, DOCK_BODY) in body and (r + 1, DOCK_BODY) in body]
    ck.check("合成场景（停泊）：窗口里目标天体真的在公转（否则「位置不动」是废话）",
             bool(bmove) and max(bmove) > 1e-6, f"{DOCK_BODY} 每回合最大位移 {max(bmove):.4f} AU")


def capital_scenario_checks(h, ck) -> None:
    """**合成场景 · Player 钉的首都**：`sim/tests/capital.rs::player_capital_not_overridden_by_ai_review`。

    另外三条首都判据（亡城强迁 / 周期评估 / 判定稀疏）都在 g3 的长局上成立，读面
    （`decisions[kind=capital]` + `factions.capital_body`）本来就够。**这一条不行**：它要
    「把首都拨到一个较远的天体上、再让评估轮跑过」——那要写控制叶 ⇒ 只能造场景。

    ⚠ **必须做成 A/B，否则是空转**：默认配置 `admin_range = 6.0`，水星到地球才 0.61 AU
    ⇒ 评估**根本不会想迁**（候选成本 = 现成本）。所以「Player 没被覆盖」单看一边什么都证明不了。
    另一半是同位置、同长度窗口下的 `Auto`：它**必须**留下评估行。两边合起来才说明
    「Player 那几个字真的挂住了 AI 的手」，而不是「评估压根没跑」。
    """
    seed = SCENARIO_SEED
    rounds = CAPITAL_ROUNDS
    diff = lambda mode: {"control": [{"势力": FID, "首都": {"值": CAPITAL_FAR, "归属": mode}}]}  # noqa: E731

    runs = {}
    for mode in ("Player", "Auto"):
        proj = h.scenario_apply(f"capital_{mode.lower()}", seed, rounds, [diff(mode)])
        q = KIT.load(str(proj), only=("factions", "decisions"))
        fa = q.table("factions")
        fa = fa[fa["势力"] == FID].sort_values("round")
        dec = q.table("decisions")
        dec = dec[(dec["kind"] == "capital") & (dec["势力"] == FID)].sort_values("round")
        runs[mode] = {
            "body": dict(zip(fa["round"].astype(int), fa["capital_body"])),
            # ⚠ `target` 是 DataFrame 列 ⇒ JSON 的 `null` 到这儿是 NaN，归一化掉（打印时才不刺眼）
            "rows": [(int(r["round"]), r["verdict"],
                      r["target"] if isinstance(r["target"], str) else None) for _, r in dec.iterrows()],
        }

    # 评估周期（从 `meta` 读真值，不写死）：窗口要盖住至少 3 个评估轮，否则 A/B 的一半会空转。
    q = KIT.load(str(h.projection(seed, rounds)), only=("factions",))
    every = int(q.meta["governance"]["capital_review_every"])
    ck.check("合成场景（首都）：窗口盖住了足够多的评估轮（防空转）",
             rounds // max(every, 1) >= 3, f"{rounds} 回合 / 每 {every} 回合评估一次")

    moved = [r for r in runs["Auto"]["rows"] if r[1] in ("review", "relocate")]
    ck.check("合成场景（首都）：同一位置、同一窗口，`Auto` 真的被评估过（A/B 的另一半）",
             len(moved) > 0,
             f"`Auto` 的判定行：{runs['Auto']['rows']}")

    player_moved = [r for r in runs["Player"]["rows"] if r[1] in ("review", "relocate")]
    ck.check("合成场景（首都）：Player 钉的首都既不被评估、也不被周期迁移",
             not player_moved,
             f"`Player` 的判定行：{runs['Player']['rows']}（只允许 `forced`——亡城硬规则照旧）")

    # 有效首都逐回合 = 「钉的那个」直到最近一次迁都，此后是那个 `target`。
    # ⚠ 从 **0** 起：投影的第 0 行是「拨完叶、还没推进」的那一态（已经能看到钉的首都）。
    want, cur = {}, CAPITAL_FAR
    for r in range(0, rounds + 1):
        for pr, verdict, target in runs["Player"]["rows"]:
            if pr == r:
                cur = target
        want[r] = cur
    got = runs["Player"]["body"]
    stray = {r: (want.get(r), b) for r, b in got.items() if want.get(r) != b}
    ck.check("合成场景（首都）：逐回合的有效首都 = 钉的那个（亡城后 = 迁都的 target）",
             not stray,
             "；".join(f"r{r}: 期望 {w} 实为 {b}" for r, (w, b) in list(stray.items())[:3])
             or f"{len(got)} 个回合全部对上（钉 {CAPITAL_FAR}）")


def stale_follow_scenario_checks(h, ck) -> None:
    """**合成场景 · 陈旧的跟随**：`sim/tests/fleet.rs::player_stale_follow_degrades_to_idle_and_does_not_drift`。

    造法比 Rust 原件更直接：钉一根**从第 0 回合就悬空**的 `Follow`（目标舰名不存在）。
    ⚠ 写面**不校验指令目标**（与「悬空的图指针」正相反——那个会被 `no_such_blueprint` 拒掉）
    ⇒ 这条退化路径本来就该存在。读面三样都在：`order_effective`（退化后的值）、
    `ships.x/y`（有没有朝原点漂）、`events[stale_order]`（退化留没留痕）。

    防空转在**别的舰**身上：同一局里它们照常在动 ⇒「这艘位置不动」不是「世界静止」的废话。
    """
    seed = SCENARIO_SEED
    ghost = "已经不存在的舰"
    diff = {"control": [{"势力": FID, "指令": [
        {"舰": STALE_SHIP, "行为": {"Follow": {"ship": ghost}}, "归属": "Player"}]}]}
    proj = h.scenario_apply("fleet_stale_follow", seed, STALE_ROUNDS, [diff])
    q = KIT.load(str(proj), only=("ships", "events"))
    sh, ev = q.table("ships"), q.table("events")
    mine = sh[sh["舰名"] == STALE_SHIP].sort_values("round")
    seq = [(int(r["round"]), r["order_effective"], (r["x"], r["y"])) for _, r in mine.iterrows()]
    ck.check("合成场景（陈旧跟随）：写面真的接下了那根悬空的 Follow（防空转）",
             bool(seq) and seq[0][1] == {"Follow": {"ship": ghost}},
             f"{STALE_SHIP} 回合 0 的指令 = {seq[0][1] if seq else None}")

    idle = [r for r, e, _ in seq if r >= 1 and e == "Idle"]
    ck.check("合成场景（陈旧跟随）：目标不存在 ⇒ 指令退化成 Idle（不追一艘不存在的舰）",
             len(idle) == len(seq) - 1 and len(seq) >= 2,
             f"{STALE_SHIP} 逐回合指令：{[e for _, e, _ in seq]}")

    drift = [(r, p) for (_, _, p0), (r, _, p) in zip(seq, seq[1:]) if p != p0]
    ck.check("合成场景（陈旧跟随）：舰不许朝原点漂（位置逐字不动）", not drift,
             "；".join(f"r{r} 跑到 {p}" for r, p in drift[:3])
             or f"{len(seq)} 个回合都是 {seq[0][2]}")

    hits = [(int(r["round"]), r["target_id"]) for _, r in ev[ev["type"] == "stale_order"].iterrows()
            if r["target_id"] == STALE_SHIP]
    ck.check("合成场景（陈旧跟随）：退化**留痕**（`stale_order` 事件指着那艘舰）", bool(hits),
             f"事件层：{hits[:3]}" if hits else "一条 `stale_order` 都没有")

    others = sh[(sh["舰名"] != STALE_SHIP) & (sh["round"] == 1)]
    moved = 0
    for _, r in others.iterrows():
        prev = sh[(sh["舰名"] == r["舰名"]) & (sh["round"] == 0)]
        if len(prev) and (prev.iloc[0]["x"], prev.iloc[0]["y"]) != (r["x"], r["y"]):
            moved += 1
    ck.check("合成场景（陈旧跟随）守卫没有空转（同一局里别的舰真的在动）", moved > 0,
             f"回合 0→1 有 {moved} 艘别的舰挪了位置")


def site_ledger_checks(h, ck) -> None:
    """**货栈账**：出口净额与进口缺口**逐资源至少有一个是 0**
    （`sim/tests/site_supply.rs::a_site_never_exports_what_it_still_needs`，2026-10 第 7 批）。

    以前这条要在 Rust 里手工把库存摆成三档（空 / 一点 / 堆成山）再直调 `exportable_at`；
    现在**一次 `--call site_ledger` 拿每个站点一行的完整账**（现货 / 保留 / 可出口 / 缺口），
    在几个不同回合各拿一次——库存水平是**真实世界自己长出来的**，判据里没有一行是抄的公式。

    ⚠ 站点集合包含「有城但还没货栈」的：只按 `state.depots` 收的话回合 0 是空表（判据空转）。
    """
    rows = both = 0
    export_ok = 0
    bad: list[str] = []
    for rounds in SITE_LEDGER_ROUNDS:
        ckpt = h.gen(CACHE_ROOT / "scenario" / f"ledger_s{SCENARIO_SEED}_r{rounds}.json",
                     SCENARIO_SEED, rounds)
        ledger = json.loads(h.capture(["--start", str(ckpt), "--call", "site_ledger"]))["value"]
        for row in ledger:
            rows += 1
            where = f"r{rounds} {row['势力']}@{row['天体名']}"
            for rt in set(row["可出口"]) | set(row["缺口"]):
                out = float(row["可出口"].get(rt, 0.0))
                inc = float(row["缺口"].get(rt, 0.0))
                if out > 1e-9 and inc > 1e-9:
                    both += 1
                    if len(bad) < 3:
                        bad.append(f"{where} {rt}：出口 {out:.2f} 与缺口 {inc:.2f} 同时在")
            for rt, stock in (row["现货"] or {}).items():
                if float(stock) > float(row["保留"].get(rt, 0.0)) + 1e-9 \
                        and float(row["可出口"].get(rt, 0.0)) > 0.0:
                    export_ok += 1
    ck.check("货栈账：出口净额与进口缺口逐资源至少有一个是 0（货不会往返乒乓）", not both,
             "；".join(bad) or f"{rows} 个「站点·回合」逐个资源对过，一处都没有两边同时为正")
    ck.check("货栈账：现货超过保留量 ⇒ 一定有出口（否则首都收不到货）", export_ok > 0,
             f"{export_ok} 处「现货 > 保留」的货都算出了正出口")
    ck.check("货栈账守卫没有空转（真看了很多站点·回合）", rows >= 20,
             f"{rows} 个站点·回合（{len(SITE_LEDGER_ROUNDS)} 个回合的账）")


def blueprint_launch_checks(h, ck) -> None:
    """**合成场景 · 下水那艘舰长什么样**：`sim/blueprints.rs` 剩下那四条（第 7 批）。

    | Rust 原件 | 判据 |
    | --- | --- |
    | `spawn_uses_the_yard_blueprint` | 船坞挂着玩家图 ⇒ 下水那艘的 `组件` = 图上的 `选装` |
    | `the_yard_launches_from_any_designs_components` | 同一份 `选装`、图归 `Auto` ⇒ 一样按它装配（**与图的归属无关**：归属只管「谁能改这张图」） |
    | `blueprint_role_governs_new_ships` | 图上写了 `角色` ⇒ 新舰就是那个角色（比舰队默认更具体，链：叶 → 图 → 舰队默认） |
    | `fleet_default_still_covers_blueprintless_ships` | 图对那条轴沉默 ⇒ 仍由**舰队默认**作答（回归守卫：新层不许把旧行为吃掉） |

    两臂只差**图的归属**一个变量，而且都摆了「舰队默认 = Observe」——那是个**非缺省**值，
    所以「图赢」与「没图听舰队默认」两条都不是空转。国库按「国库真有的键 + 选装要的货」垫
    （P1-5 的 `con_scale`：库存不到维护 reserve 时写多大预算都是 0）。
    """
    seed = SCENARIO_SEED
    meta = json.loads(h.capture(["--meta"]))
    comps = ["kinetic", "ion_drive"]
    cost_keys = sorted({rt for c in comps for rt in (meta["components"][c].get("cost") or {})})
    fr = KIT.load(str(h.projection(seed, SCENARIO_ROUNDS)), only=("factions",)).table("factions")
    stock_keys = sorted((list(fr[(fr["round"] == 0) & (fr["势力"] == FID)]["资源"])[0] or {}).keys())
    stock = _stock_patch(FID, 1e6, sorted(set(stock_keys) | set(cost_keys) | {"铁", "碳", "硅"}))

    st = h.state_dump(h.gen(CACHE_ROOT / "scenario" / "_bp_launch.json", seed))
    yards = {c: (bid, cls) for c, bid, cls in _yards_of(h, st, FID)}
    arms = [(mode, LAUNCH_YARDS[mode]) for mode in ("Player", "Auto")]
    seen: dict[str, dict] = {}
    for mode, city in arms:
        bid, class_ = yards[city]
        name = f"下水图·{mode}"
        diff = {"control": [{"势力": FID,
                             "舰队默认角色": {"角色": "Observe"},
                             "设计图库": [{"图名": name, "舰级": class_,
                                             "选装": comps, "角色": "Freight",
                                             "风格": LAUNCH_DOCTRINE, "姿态": LAUNCH_KITING,
                                             "归属": mode}],
                             "建筑": [{"城": city, "建筑": bid, "设计图": name}]}]}
        proj = h.scenario_apply(f"bp_launch_{mode.lower()}", seed, LAUNCH_ROUNDS, [diff], patch=stock)
        q = KIT.load(str(proj), only=("ships",))
        sh = q.table("ships")
        from_bp = sh[sh["出厂图"] == name].sort_values("round")
        plain = sh[(sh["出厂图"].isna()) & (sh["round"] == LAUNCH_ROUNDS) & (sh["势力"] == FID)]
        seen[mode] = {
            "city": city, "class": class_, "n": len(from_bp),
            "comps": sorted({tuple(sorted(c or [])) for c in from_bp["组件"]}),
            "role": sorted({str(r) for r in from_bp["角色"]}),
            "role_mode": sorted({str(m) for m in from_bp["role_mode"]}),
            "plain_role": sorted({str(r) for r in plain["角色"]}),
            "plain_mode": sorted({str(m) for m in plain["role_mode"]}),
            "doctrine": sorted({json.dumps(d, sort_keys=True) for d in from_bp["风格"]}),
            "kiting": sorted({float(k) for k in from_bp["姿态"]}),
            "src": sorted({str(s) for s in from_bp["order_source"]}),
        }

    a = seen["Player"]
    ck.check("合成场景（下水）：玩家图的选装就是下水那艘的选装（防空转：真下了水）",
             a["n"] >= 1 and a["comps"] == [tuple(sorted(comps))],
             f"{a['city']} 的 {a['class']} 下水 {a['n']} 艘，组件 {a['comps']}")
    b = seen["Auto"]
    ck.check("合成场景（下水）：图归 `Auto` 也照它的选装装配（出厂规格与图的归属无关）",
             b["n"] >= 1 and b["comps"] == a["comps"],
             f"{b['city']} 的 {b['class']} 下水 {b['n']} 艘，组件 {b['comps']}（玩家图那臂 {a['comps']}）")
    ck.check("合成场景（下水）：图上的 `角色` 管住新舰（比舰队默认更具体 ⇒ 图赢）",
             a["role"] == ["Freight"] and a["role_mode"] == ["Player"],
             f"图上下水的舰：角色 {a['role']} / 归属 {a['role_mode']}（舰队默认那条臂摆的是 Observe）")
    ck.check("合成场景（下水）：图沉默的那条轴仍由舰队默认作答（不是缺省的 War，是摆的 Observe）",
             bool(a["plain_role"]) and a["plain_role"] == ["Observe"]
             and a["plain_mode"] == ["Player"],
             f"没图可言的舰：角色 {a['plain_role']} / 归属 {a['plain_mode']}")
    # `order_source_separates_a_missing_leaf_from_a_silent_one` 的那半：**图写得再满也不能
    # 指挥指令**——倾向（风格/姿态/角色）从图上下来，但「这一回合干什么」只由逐舰叶供值。
    ck.check("合成场景（下水）：图给的倾向（风格/姿态）落到新舰上",
             a["doctrine"] == [json.dumps(LAUNCH_DOCTRINE, sort_keys=True)]
             and a["kiting"] == [LAUNCH_KITING],
             f"图上那批舰：风格 {a['doctrine']} / 姿态 {a['kiting']}")
    ck.check("合成场景（下水）：图写得再满也不供指令（`order_source` 只会是 leaf）",
             a["src"] == ["leaf"], f"图上那批舰的 order_source：{a['src']}")


def combat_scenario_checks(h, ck) -> None:
    """**合成场景 · 受损组件与本土修复**（`sim/combat.rs::damaged_components_repair_in_friendly_territory`）。

    长局读面证不了这条：实测 seed 42 / 400 回合里「未挨打却修了」的组件·回合**一共只有 1 个**，
    而且它在**外地**——「本土修得更快」那半根本没有样本。所以照原件那样**造**一个受损组件：
    `h.scenario(patch=…)` 把一艘舰的组件换成一件 `railgun`、耐久打到 5.0、坐标搬到首都天体
    （另一臂搬到海王星）。`edit()` 要「带身份键的行表」，而**舰有身份键**（`舰名`）⇒ 这处能捏
    （`depots` 那种复合键的 map 就不行）。

    两臂**只差坐标一个变量**；`本土半径` 从读面 `factions.本土半径` 来，不用写死 AU。
    """
    seed = SCENARIO_SEED
    st = h.state_dump(h.gen(CACHE_ROOT / "scenario" / "_repair_probe.json", seed))
    pos = {b["天体名"]: b["位置"] for b in st["bodies"]}
    ship = next(s for s in st["ships"] if s["势力"] == FID)
    full = float(json.loads(h.capture(["--call", "component_integrity",
                                       "--args", json.dumps({"component": REPAIR_COMPONENT})]))["value"])

    # 首都从读面拿；「外地」取**离首都最远**的那个天体（不写死地名）。
    fac0 = KIT.load(str(h.projection(seed, 1)), only=("factions",)).table("factions")
    cap_body = str(list(fac0[(fac0["round"] == 1) & (fac0["势力"] == FID)]["capital_body"])[0])
    far_body = max((b for b in pos if b != cap_body),
                   key=lambda b: math.dist(pos[b], pos[cap_body]))

    arms = {}
    for tag, where in (("home", cap_body), ("away", far_body)):
        patch = {"ships": {ship["舰名"]: {"坐标": pos[where], "组件": [REPAIR_COMPONENT],
                                          "组件耐久": [REPAIR_START]}}}
        proj = h.scenario(f"combat_repair_{tag}", seed, REPAIR_ROUNDS, patch)
        q = KIT.load(str(proj), only=("ships", "factions", "body_positions"))
        rows = q.table("ships")
        rows = rows[rows["舰名"] == ship["舰名"]].sort_values("round")
        hp = [float(list(x)[0]) for x in rows["组件耐久"]]
        fac = q.table("factions")
        fac = fac[(fac["round"] == 1) & (fac["势力"] == FID)]
        bp = q.table("body_positions")
        b1 = bp[(bp["round"] == 1) & (bp["天体名"] == cap_body)]
        # ⚠ 量的是「到**首都**的距离」（本土规则认的是它），不是「到投放的那个天体」。
        d_cap = math.dist((float(rows.iloc[1]["x"]), float(rows.iloc[1]["y"])),
                          (float(list(b1["x"])[0]), float(list(b1["y"])[0])))
        arms[tag] = {"hp": hp, "d": d_cap, "where": where,
                     "radius": float(list(fac["本土半径"])[0]),
                     "gain": [round(b - a, 4) for a, b in zip(hp, hp[1:])]}

    ck.check(f"合成场景（修船）：起点真的受损（{REPAIR_START} < 满值 {full}，防空转）",
             full > REPAIR_START > 0.0, f"`{REPAIR_COMPONENT}` 的完整度满值 {full}")
    for tag, cn in (("home", "本土"), ("away", "外地")):
        g = arms[tag]["gain"]
        ck.check(f"合成场景（修船）：{cn}受损组件逐回合修复（每一回合都涨）",
                 bool(g) and all(x > 0 for x in g),
                 f"{cn}（放在 {arms[tag]['where']}，距首都 {arms[tag]['d']:.1f} AU，"
                 f"半径 {arms[tag]['radius']:.1f}）"
                 f"逐回合修复量 {g}，耐久 {[round(h, 2) for h in arms[tag]['hp']]}")
    ck.check("合成场景（修船）：**本土修得更快**（两臂只差坐标，外地的修复量逐回合都更小）",
             all(a > b for a, b in zip(arms["home"]["gain"], arms["away"]["gain"])),
             f"本土 {arms['home']['gain']} vs 外地 {arms['away']['gain']}"
             f"（{arms['home']['d']:.1f} / {arms['away']['d']:.1f} AU，半径 {arms['home']['radius']:.1f}）")
    ck.check("合成场景（修船）：两臂到**首都**的距离真的一个在内、一个在外（防空转）",
             arms["home"]["d"] <= arms["home"]["radius"] < arms["away"]["d"],
             f"放在 {arms['home']['where']}/{arms['away']['where']} ⇒ 距首都 "
             f"{arms['home']['d']:.1f} ≤ 半径 {arms['home']['radius']:.1f} < {arms['away']['d']:.1f} AU")


def _launch_report(ships) -> dict:
    """**新舰出厂时的指令归属**（`sim/tests/fleet.rs::newly_built_ships_have_no_order_of_their_own`）。

    取「下水那一回合正好等于本回合」的行（= 刚出厂的那一批），看三样：
    `order_leaf_mode`（叶片自己的表态）、`order_effective_mode`（有效归属）、`order_source`。

    ⚠ 原件还断言「叶片里的**值**是个占位 `Idle`」——那半**读面看不见**：AI 会在**同一个回合内**
    就给它派活（实测 345 艘里只有 30 艘的 `order_effective` 还是 `Idle`，其余已经是
    `DockCity`/`Colonize`/`Haul`）。`spawn_ship` 那一刻的状态没有读法——这条判据只看
    「叶片有没有主张」（`Inherit`）与「谁最终说了算」（`Auto`），那才是回归面。
    """
    launch = ships[ships["下水回合"] == ships["round"]]
    return {
        "n": int(len(launch)),
        "leaf": sorted({str(m) for m in launch["order_leaf_mode"]}),
        "mode": sorted({str(m) for m in launch["order_effective_mode"]}),
        "src": sorted({str(s) for s in launch["order_source"]}),
    }


def new_ship_checks(h, ck, out) -> None:
    """**新舰没有自己的指令主张**（`sim/tests/fleet.rs`，2026-10 第 7 批搬来）。"""
    reps = [d["launch"] for d in out]
    n = sum(r["n"] for r in reps)
    leaf = sorted({m for r in reps for m in r["leaf"]})
    mode = sorted({m for r in reps for m in r["mode"]})
    src = sorted({s for r in reps for s in r["src"]})
    ck.check("新舰出厂时叶片**没有说话**（`order_leaf_mode` 只会是 Inherit）",
             leaf == ["Inherit"], f"{n} 艘新舰的叶片模式：{leaf}")
    ck.check("新舰归**系统**（有效归属 Auto——没有任何更高的一层替它表态）",
             mode == ["Auto"], f"{n} 艘新舰的有效归属：{mode}（来源 {src}）")
    ck.check("新舰指令守卫没有空转（真的有一批刚出厂的舰）", n >= 50, f"{n} 艘（下限 50）")


def commanded_haul_checks(h, ck) -> None:
    """**合成场景 · 玩家钉的常驻运输线**（`sim/haul.rs::a_commanded_haul_route_delivers_depot_cargo_into_the_capital_pool`）。

    钉一片**玩家**的 `Haul{from, to}` 叶（自动控制不碰它）+ 把角色钉成运输舰，然后**不再下任何指令**：
    货要从**产地货栈**走进**首都池**，而且这条路线自己往复。

    ⚠ 判据用 `haul_steps`（`loaded` 之后 `delivered` 且 `into_pool: true`）与「指令逐回合不变」，
    **不**拿首都池的净增量当判据——池子同时在花钱（建设/造舰/维护），收进来的货是毛额。
    原件也写明了这一点。

    ⚠ 起点选**灶神星**是有理由的：本地城的需求小，产出能攒出**可出口余量**；实测水星/火星/木星
    都不行（本地建设把余量吃光了，船合规地一直 `waiting`——那正是 P0-1 的
    「出口腿不许装走本地保留量」）。实测 r41 才等到第一批货（1.7 件），所以窗口给足 60 回合，
    并把「真等到过货」写成防空转判据。
    """
    seed = SCENARIO_SEED
    diff = {"control": [{"势力": FID, "指令": [
        {"舰": HAUL_SHIP, "行为": {"Haul": {"from": HAUL_FROM, "to": HAUL_TO}},
         "归属": "Player", "角色": "Freight"}]}]}
    proj = h.scenario_apply("haul_commanded", seed, HAUL_ROUNDS, [diff])
    q = KIT.load(str(proj), only=("haul_steps", "ships", "depots", "factions"))
    hs = q.table("haul_steps")
    mine = hs[hs["舰名"] == HAUL_SHIP].sort_values("round")
    steps = [(int(r["round"]), str(r["step"]), float(r["units"] or 0.0), bool(r["into_pool"]))
             for _, r in mine.iterrows()]
    sh = q.table("ships")
    orders = [str(o) for o in sh[sh["舰名"] == HAUL_SHIP]["order_effective"]]

    ck.check("合成场景（玩家运输线）：探针世界里那艘舰与那条线都成立（防空转）",
             len(steps) >= 10, f"{HAUL_SHIP} 在 {HAUL_ROUNDS} 回合里有 {len(steps)} 条运输动作")
    ck.check("合成场景（玩家运输线）：玩家钉的路线**逐回合不变**（不需要重下指令）",
             len(set(orders)) == 1 and HAUL_FROM in orders[0] and HAUL_TO in orders[0],
             f"{len(orders)} 个回合的指令：{sorted(set(orders))}")

    loaded = [s for s in steps if s[1] == "loaded"]
    delivered = [s for s in steps if s[1] == "delivered" and s[3]]
    ck.check("合成场景（玩家运输线）：货真的从**产地货栈**装上了船",
             bool(loaded) and all(u > 0 for _, _, u, _ in loaded),
             f"装货 {[(r, round(u, 2)) for r, _, u, _ in loaded]}")
    ck.check("合成场景（玩家运输线）：卸下来的货**进了首都池**（`into_pool`）",
             bool(delivered), f"进池卸货 {[(r, round(u, 2)) for r, _, u, _ in delivered]}")
    # 「两个回合内装完并卸到池里」：每一次装货之后，下一回合就是一次进池卸货。
    pairs = []
    for r, _, _, _ in loaded[:6]:
        pairs.append(any(dr in (r, r + 1) for dr, _, _, _ in delivered))
    ck.check("合成场景（玩家运输线）：装完那一回合/下一回合就卸进池（常驻路线自己往复）",
             bool(pairs) and all(pairs), f"每个装货回合之后都跟着进池卸货：{pairs}")
    ck.check("合成场景（玩家运输线）：守卫没有空转（真等到过货、也真等待过）",
             len(loaded) >= 1 and any(s[1] == "waiting" for s in steps),
             f"{len(loaded)} 次装货、{sum(1 for s in steps if s[1] == 'waiting')} 回合等待")


def welfare_scenario_checks(h, ck) -> None:
    """**合成场景 · 重金娱乐拉住远城**（`sim/ideology.rs::entertainment_holds_a_distant_city`）。

    两臂**只差有没有那份福利预算**：同一座城（该势力 `gov_distance` 最大的那座）、同样的起点忠诚度、
    同样的满仓国库。造法全是现成入口：国库与城市忠诚度走 `h.scenario(patch=…)`（势力/城都有身份键），
    福利预算与城市权重走 `--apply` 的 `福利预算` / `城市福利预算` 叶——**与原件那份 diff 同形**。

    防空转在**对照臂**：同样的 0.35 起点，不投福利时忠诚度**真的往下走** ⇒ 「不降」不是「世界本来
    就这样」。⚠ 对照臂实测到 r4 会自己跳回 0.75（别的东西把它拉起来了），所以判据看的是
    「窗口里**下滑过**」而不是「末端更低」。
    """
    seed = SCENARIO_SEED
    q0 = KIT.load(str(h.projection(seed, 1)), only=("cities", "factions"))
    ci = q0.table("cities")
    mine = ci[(ci["round"] == 0) & (ci["势力"] == WELFARE_FID)]
    ck.check("合成场景（娱乐拉忠诚）：探针世界里那个势力有多座城（防空转）",
             len(mine) >= 2, f"{WELFARE_FID} 开局 {len(mine)} 座城")
    city = mine.loc[mine["gov_distance"].idxmax()]
    keys = sorted((list(q0.table("factions").pipe(
        lambda x: x[(x["round"] == 0) & (x["势力"] == WELFARE_FID)])["资源"])[0] or {}).keys())
    patch = {"factions": {WELFARE_FID: {"资源": {k: 100000.0 for k in keys}}},
             "cities": {city["城名"]: {"忠诚度": WELFARE_START}}}
    welfare = {"control": [{"势力": WELFARE_FID,
                            "福利预算": [{"资源": "铁", "值": 5000.0, "归属": "Player"}],
                            "城市福利预算": [{"城": city["城名"], "值": 500.0, "归属": "Player"}]}]}

    arms = {}
    for tag, diffs in (("none", []), ("rich", [welfare])):
        proj = h.scenario_apply(f"welfare_{tag}", seed, WELFARE_ROUNDS, diffs, patch=patch)
        q = KIT.load(str(proj), only=("cities",))
        rows = q.table("cities")
        rows = rows[rows["城名"] == city["城名"]].sort_values("round")
        arms[tag] = {"loy": [float(x) for x in rows["忠诚度"]],
                     "razed": [bool(x) for x in rows["已焚毁"]]}

    rich, none = arms["rich"], arms["none"]
    ck.check(f"合成场景（娱乐拉忠诚）：重金娱乐的远城忠诚**逐回合不降**（起点 {WELFARE_START}）",
             len(rich["loy"]) >= 3 and all(b >= a - 1e-9 for a, b in zip(rich["loy"], rich["loy"][1:])),
             f"忠诚度 {[round(x, 3) for x in rich['loy']]}（距首都 {city['gov_distance']:.1f} AU）")
    ck.check("合成场景（娱乐拉忠诚）：对照臂（同样起点、不投福利）忠诚**真的往下走**（防空转）",
             min(none["loy"]) < none["loy"][0] - 1e-9,
             f"对照臂忠诚度 {[round(x, 3) for x in none['loy']]}（最低 {min(none['loy']):.3f}）")
    ck.check("合成场景（娱乐拉忠诚）：两臂那座城都没被焚毁（有钱就不离心）",
             not any(rich["razed"]) and not any(none["razed"]),
             f"重金臂 {rich['razed']}｜对照臂 {none['razed']}")


def colonize_scenario_checks(h, ck) -> None:
    """**合成场景 · 殖民是一次性指令，但归属不是**（`sim/fleet.rs::colonize_keeps_player_ownership`）。

    两个臂：

    * **真殖民**：挑一个「有**空定居点**」的回合（开局 22 座城占满 22 个定居点，得等到有城被拆平），
      钉一片玩家 `Colonize{天体}` 叶，跑一段 ⇒ ① `colony_founded` 事件真的落在那个天体上、
      ② 一次性指令被**花掉**（`order_effective` 从 `Colonize` 变成 `Idle`）、
      ③ **归属没被清掉**（`order_leaf_mode` 全程 `Player`）。
    * **早退**（目标天体定居点已满）：同样钉一片玩家叶 ⇒ 早退**也不许把船交回系统**
      （`order_leaf_mode` 仍是 `Player`）——原件那半只断言这一条。

    ⚠ `colony_founded` 的 `target_id` 是**城名**，天体在 `data.body`（第一版按 `target_id == 天体`
    去找，一条都没找到）。
    """
    seed = SCENARIO_SEED
    q = KIT.load(str(h.projection(seed, ROUNDS)), only=("cities", "settlements"))
    alls = {(r["天体名"], r["定居点"]) for _, r in q.table("settlements").iterrows()}
    ci = q.table("cities")
    occ: dict = {}
    for _, r in ci.iterrows():
        if not r["已焚毁"]:
            occ.setdefault(int(r["round"]), set()).add((r["天体名"], r["定居点"]))
    start = next((r for r in sorted(occ) if alls - occ[r]), None)
    body = sorted(alls - occ[start])[0][0] if start is not None else None
    ck.check("合成场景（殖民）：长局里真的出现过**空定居点**（否则这条判据无从谈起）",
             start is not None, f"最早在第 {start} 回合有可复垦的定居点（天体 {body}）")
    if start is None:
        return

    st = h.state_dump(h.gen(CACHE_ROOT / "scenario" / "_colonize.json", seed, start))
    ship = next(s for s in st["ships"] if s["势力"] == FID)
    diff = {"control": [{"势力": FID, "指令": [
        {"舰": ship["舰名"], "行为": {"Colonize": {"body": body}}, "归属": "Player"}]}]}
    proj = h.scenario_apply("colonize_real", seed, COLONIZE_ROUNDS, [diff], start_round=start)
    q = KIT.load(str(proj), only=("events", "ships", "cities"))
    ev = q.table("events")
    hits = [(int(r["round"]), r["actor_id"], r["target_id"]) for _, r in ev[ev["type"] == "colony_founded"].iterrows()
            if (r["data"] or {}).get("天体") == body]
    sh = q.table("ships")
    mine = sh[sh["舰名"] == ship["舰名"]].sort_values("round")
    modes = [str(m) for m in mine["order_leaf_mode"]]
    seq = [str(o) for o in mine["order_effective"]]

    ck.check("合成场景（殖民）：玩家钉的 `Colonize` 真的建成了城（事件落在那个天体上）",
             any(a == FID for _, a, _ in hits),
             f"{body} 上的复垦事件：{hits[:3]}")
    ck.check("合成场景（殖民）：一次性指令被**花掉**（有效指令从 Colonize 变成 Idle）",
             any("Colonize" in o for o in seq) and seq[-1] == "Idle",
             f"指令序列 {seq[:2]} … {seq[-2:]}（{len(seq)} 个回合）")
    ck.check("合成场景（殖民）：**归属没被清掉**（叶片全程 Player，不被交回系统）",
             bool(modes) and set(modes) == {"Player"}, f"叶片模式：{sorted(set(modes))}")

    # 早退：开局 22 座城占满 22 个定居点 ⇒ 目标天体无处可殖民。
    full = {b for b, _ in alls if not (alls - occ[0])}
    ck.check("合成场景（殖民）：早退臂的前提成立（回合 0 每个天体都满了）",
             bool(full), f"回合 0 空定居点数 {len(alls - occ[0])}")
    if full:
        target = sorted(full)[0]
        st0 = h.state_dump(h.gen(CACHE_ROOT / "scenario" / "_colonize0.json", seed, 0))
        ship0 = next(s for s in st0["ships"] if s["势力"] == FID)
        d0 = {"control": [{"势力": FID, "指令": [
            {"舰": ship0["舰名"], "行为": {"Colonize": {"body": target}}, "归属": "Player"}]}]}
        p0 = h.scenario_apply("colonize_early", seed, COLONIZE_ROUNDS, [d0])
        s0 = KIT.load(str(p0), only=("ships",)).table("ships")
        m0 = [str(m) for m in s0[s0["舰名"] == ship0["舰名"]]["order_leaf_mode"]]
        ck.check("合成场景（殖民）：**早退也不许把船交回系统**（叶片仍是 Player）",
                 bool(m0) and set(m0) == {"Player"},
                 f"目标天体 {target}（回合 0 已满）⇒ 叶片模式 {sorted(set(m0))}")


def defection_scenario_checks(h, ck) -> None:
    """**合成场景 · 低忠诚改旗易帜**（`sim/ideology.rs::low_loyalty_city_defects_to_most_opposing_ideology_instead_of_razing`）。

    捏三样：**那个势力的思潮**推到极端、**对照势力**推到相反极、其余中立（全都有身份键 ⇒ 直接改档），
    再把一座城的忠诚压到叛变阈值之下。

    「倒向谁」**不写死**：拿 `--call ideology_similarity` 把「旧主 × 每个势力」的相似度都算一遍，
    判据要求倒戈目标就是**相似度最低**的那一个（引擎自己那份实现）。另加「没被夷平、人口与建筑都在」。
    """
    seed = SCENARIO_SEED
    q0 = KIT.load(str(h.projection(seed, 1)), only=("cities", "factions"))
    ci, fr = q0.table("cities"), q0.table("factions")
    mine = ci[(ci["round"] == 0) & (ci["势力"] == DEFECT_FID)]
    city = mine.loc[mine["gov_distance"].idxmin()]
    f0 = fr[fr["round"] == 0]
    axes = ["和平↔军国", "科学↔技术", "人民↔精英", "自然↔殖民"]
    others = [r["势力"] for _, r in f0.iterrows() if r["势力"] != DEFECT_FID]
    ck.check("合成场景（改旗易帜）：探针世界里那个势力有城、也有别人可倒（防空转）",
             len(mine) >= 1 and len(others) >= 2,
             f"{DEFECT_FID} 有 {len(mine)} 座城；候选新主 {len(others)} 个")

    patch = {"cities": {city["城名"]: {"忠诚度": DEFECT_LOYALTY}},
             "factions": {r["势力"]: {"思潮": {a: (1.0 if r["势力"] == DEFECT_FID
                                                 else -1.0 if r["势力"] == DEFECT_TARGET_HINT
                                                 else 0.0) for a in axes}}
                          for _, r in f0.iterrows()}}
    proj = h.scenario("defect_live", seed, DEFECT_ROUNDS, patch)
    q = KIT.load(str(proj), only=("cities", "events", "factions"))
    rows = q.table("cities")
    rows = rows[rows["城名"] == city["城名"]].sort_values("round")
    # ⚠ 思潮要从**这个场景自己的**投影读（补丁已落地）。第一版读的是对照局 ⇒ 相似度算的是
    # 默认思潮，判据只是**碰巧**还是那一家（又一处「绿了但没在测」）。
    sc = q.table("factions")
    ideo0 = {r["势力"]: dict(r["思潮"]) for _, r in sc[sc["round"] == 0].iterrows()}
    ev = q.table("events")
    hits = [(int(r["round"]), (r["data"] or {})) for _, r in ev[ev["type"] == "city_defected"].iterrows()
            if (r["data"] or {}).get("城") == city["城名"]]
    ck.check("合成场景（改旗易帜）：忠诚低于阈值 ⇒ 那座城**倒戈了**（不是被夷平）",
             bool(hits) and not bool(list(rows["已焚毁"])[-1]),
             f"{city['城名']} 的倒戈事件：{hits[:2]}；已焚毁 {list(rows['已焚毁'])[-1]}")

    # 「倒向谁」= 与旧主**思潮相似度最低**的那个（用引擎自己的相似度函数算）。
    sims = {f: call_ideology_similarity(h, ideo0[DEFECT_FID], ideo0[f]) for f in others}
    want = min(sims, key=lambda f: sims[f])
    got = hits[0][1].get("新主") if hits else None
    ck.check("合成场景（改旗易帜）：倒向的是**思潮最对立**的那一家（相似度最低，引擎自己算的）",
             got == want and got is not None,
             f"相似度 {sorted((round(v, 3), k) for k, v in sims.items())} ⇒ 应倒向 {want}，实为 {got}")

    first, last = rows.iloc[0], rows.iloc[-1]
    ck.check("合成场景（改旗易帜）：城连同人口与建筑一起易主（不是拆平重来）",
             int(last["人口"]) == int(first["人口"]) and len(last["建筑"]) == len(first["建筑"])
             and str(last["势力"]) == str(want),
             f"人口 {first['人口']} → {last['人口']}；建筑 {len(first['建筑'])} → {len(last['建筑'])}；"
             f"势力 {first['势力']} → {last['势力']}")


def call_ideology_similarity(h, a: dict, b: dict) -> float:
    """问引擎要两份思潮的相似度（`--call ideology_similarity`）。"""
    return float(json.loads(h.capture(["--call", "ideology_similarity",
                                       "--args", json.dumps({"a": a, "b": b},
                                                             ensure_ascii=False)]))["value"])


def trade_block_checks(h, ck, out) -> None:
    """**贸易禁运名单**（`sim/tests/trade.rs::trade_block_list_names_the_blocker_and_the_tier`，第 7 批）。

    投影 `factions` 新补了一列 `贸易禁运` = `{禁运方: 档位}`（三档 `war`/`cold`/`coalition`，
    由 `sim::trade_block_cause` 判定）。判据两半：

    * **结构**（全回合）：名单里没有自己、禁运方是真势力、档位在取值域里，
      而且 `war` 档**两边关系真的在 `combat.war_threshold` 之下**（`war` 档 = **敌对**，
      不是「宣战过」——第一版拿 `war_started` 事件去重建，整片假红）；
    * **同源复核**：挑一个回合，逐个条目问 `--call trade_block_cause(禁运方, 我)`，
      要求与名单**逐字相等**——这正是「别在读面另编一套」的检查。
    """
    rep = [d["trade_block"] for d in out]
    n = sum(r["n"] for r in rep)
    bad = [(s, m) for s, r in zip(SEEDS, rep) for m in r["bad"]]
    causes = sorted({c for r in rep for c in r["causes"]})
    ck.check("禁运名单：没有自己禁运自己、禁运方是真势力、档位在 war/cold/coalition 里、"
             "`war` 档两边关系真的在交战阈值之下",
             not bad, "；".join(m for _, m in bad[:3]) or
             f"{len(SEEDS)} seed 共 {n:,} 条名单，档位 {causes}")
    ck.check("禁运名单守卫没有空转（真的出现过禁运，且不止一档）",
             n >= 10 and len(causes) >= 2, f"{n:,} 条、{len(causes)} 档 {causes}")

    # 同源复核：一个回合、逐个条目问引擎。
    reps = [r for r in rep if r["rounds"]]
    if not reps:
        ck.check("禁运名单：同源复核（有可复核的回合）", False, "没有任何回合有禁运条目")
        return
    rnd = reps[0]["rounds"][len(reps[0]["rounds"]) // 2]
    seed = SEEDS[rep.index(reps[0])]
    ckpt = h.gen(CACHE_ROOT / "scenario" / f"_tb_s{seed}_r{rnd}.json", seed, rnd)
    mism = []
    checked = 0
    for blocker, fid, cause in reps[0]["by_round"][rnd]:
        want = json.loads(h.capture(["--start", str(ckpt), "--call", "trade_block_cause",
                                     "--args", json.dumps({"a": blocker, "b": fid},
                                                           ensure_ascii=False)]))["value"]
        checked += 1
        if want != cause:
            mism.append(f"{blocker}→{fid}：名单 {cause} vs 引擎 {want}")
    ck.check("禁运名单：与引擎判据**同源**（逐条目问 `--call trade_block_cause`，逐字相等）",
             bool(mism) is False and checked > 0,
             "；".join(mism[:3]) or f"第 {rnd} 回合 {checked} 条逐个复核相等")


def haul_leg_report(q) -> dict:
    """**运输腿别由货舱决定**（`sim/tests/haul.rs::a_haul_route_alternates_legs_because_of_the_cargo`，第 7 批）。

    读面两份就够，**不用新列**：`ships.order_effective` 里**声明的** `Haul{from, to}`
    （路线是引擎给的，不是我推的）+ `haul_steps`（这一步在哪、干什么）+ `ships.载货`。
    判据 = 那三步各自该在哪一端：

    * `en_route`：**舱里有货 ⇒ 目标是 `to`；空舱 ⇒ 目标是 `from`**（这就是「腿别由货舱决定」）；
    * `waiting`：只在 `from`，且舱必须是空的（「货栈空就原地等」）；
    * `loaded` 只在 `from`、`delivered` 只在 `to`。

    ⚠ 第一版我拿「观测到的装卸天体」去推 from/to，结果单路线舰里还有 196 处反例——**推出来的路线
    是错的**（承包投递的卸货端与货主不是一回事）。声明路线就在读面上，不用推。
    """
    hs, sh = q.table("haul_steps"), q.table("ships")
    route: dict = {}
    cargo: dict = {}
    for _, r in sh.iterrows():
        key = (int(r["round"]), r["舰名"])
        o = r["order_effective"]
        h = (o or {}).get("Haul") if isinstance(o, dict) else None
        route[key] = ((h or {}).get("from"), (h or {}).get("to")) if h else None
        cargo[key] = sum((r["载货"] or {}).values())
    bad: list[str] = []
    n = orphan = 0
    kinds: set = set()
    for _, r in hs.iterrows():
        key = (int(r["round"]), r["舰名"])
        rt = route.get(key)
        if not rt or not rt[0]:
            orphan += 1
            continue
        frm, to = rt
        n += 1
        kinds.add(r["step"])
        where = f"r{key[0]} {r['舰名']}（{frm}→{to}）"
        c = cargo.get(key, 0.0)
        step, body = r["step"], r["body"]
        if step == "en_route":
            if c > 1e-9 and body != to:
                bad.append(f"{where}：舱里有货却开向 {body}（该去卸货端 {to}）")
            elif c <= 1e-9 and body != frm:
                bad.append(f"{where}：空舱却开向 {body}（该回装货端 {frm}）")
        elif step == "waiting":
            if body != frm:
                bad.append(f"{where}：在卸货端干等")
            elif c > 1e-9:
                bad.append(f"{where}：舱里有货却在干等")
        elif step == "loaded" and body != frm:
            bad.append(f"{where}：在卸货端装货")
        elif step == "delivered" and body != to:
            bad.append(f"{where}：在装货端卸货")
    return {"n": n, "orphan": orphan, "bad": bad[:4], "kinds": sorted(kinds)}


def haul_leg_checks(h, ck, out) -> None:
    """**运输腿别**（`sim/tests/haul.rs`，第 7 批搬来）。"""
    reps = [d["haul_leg"] for d in out]
    n = sum(r["n"] for r in reps)
    orphan = sum(r["orphan"] for r in reps)
    kinds = sorted({k for r in reps for k in r["kinds"]})
    bad = [(s, m) for s, r in zip(SEEDS, reps) for m in r["bad"]]
    ck.check("运输腿别：`en_route` 的目标由**货舱**决定（有货去卸货端、空舱回装货端），"
             "`waiting` 只在装货端且舱是空的，装/卸各在自己那一端",
             not bad, "；".join(m for _, m in bad[:3]) or
             f"{len(SEEDS)} seed 共 {n:,} 步全部落在声明路线该在的那一端")
    ck.check("运输腿别守卫没有空转（四种步骤都出现过，且真等到过货）",
             set(kinds) == {"loaded", "delivered", "waiting", "en_route"} and n >= 500,
             f"{n:,} 步，步骤 {kinds}；无声明路线的 {orphan} 步（回合末刚被改派的那种）")


def knowledge_scenario_checks(h, ck) -> None:
    """**合成场景 · 驻泊深度决定知识**（`sim/tests/knowledge.rs` 那三条，第 7 批）。

    把某势力的**每一艘舰**用 `patch` 搬到同一个日心距、再用玩家 `Idle` 钉住（Idle 保持位置），
    于是「在场强度」是一个干净的乘式。三个臂：**带内**（日心距 1 AU，在 `mond.radius` 以内）、
    **浅**（`radius + 2`）、**深**（`radius + 40`）。

    | 原件 | 这里 |
    | --- | --- |
    | `presence_comes_only_from_ships_in_the_band` | 带内 ⇒ 强度 0；强度 == 舰数 × `(1 + 深度 × 权重)`；同样一批舰**停得越深强度越大** |
    | `control_climbs_toward_the_presence_target_and_stops_there` | 浅臂：目标严格在 `(0,1)`，掌握度**逐回合朝它爬、不超过它、差距在缩小**（「收敛」那半由 g3 的**逐回合定律**判，样本多三个数量级） |
    | `a_real_deep_presence_reaches_the_top` | 深臂：目标正好 `1.0`，掌握度在 `mastery_rounds` 之内**爬到 1.0**；`--call mond_target` 在门槛两侧是 `1.0` / `< 1` |

    ⚠ 浅臂窗口只给 30 回合：再长那三艘舰会战死，强度掉回 0（实测 150 回合后目标塌成 0）。
    """
    seed = SCENARIO_SEED
    meta = json.loads(h.capture(["--meta"]))
    radius = float(meta["mond"]["radius"])
    w = float(meta["mond"]["knowledge"]["depth_weight"])
    st = h.state_dump(h.gen(CACHE_ROOT / "scenario" / "_know_probe.json", seed))
    names = [s["舰名"] for s in st["ships"] if s["势力"] == KNOW_FID and s["船体"] > 0]
    ck.check("合成场景（知识）：探针世界里那个势力开局有舰（防空转）",
             len(names) >= 1, f"{KNOW_FID} 开局 {len(names)} 艘：{names}")

    arms: dict = {}
    for tag, rau, rnds in (("in_belt", 1.0, 8), ("shallow", radius + 2.0, 30), ("deep", radius + 40.0, 60)):
        patch = {"ships": {n: {"坐标": [rau, 0.0]} for n in names}}
        diff = {"control": [{"势力": KNOW_FID,
                             "指令": [{"舰": n, "行为": "Idle", "归属": "Player"} for n in names]}]}
        proj = h.scenario_apply(f"know_{tag}", seed, rnds, [diff], patch=patch)
        q = KIT.load(str(proj), only=("factions",))
        f = q.table("factions")
        f = f[f["势力"] == KNOW_FID].sort_values("round")
        arms[tag] = {"rau": rau, "depth": rau - radius,
                     "pres": [float(x) for x in f["mond_presence"]],
                     "target": [float(x) for x in f["mond_target"]],
                     "m": [float(x) for x in f["MOND 掌握度"]]}

    belt = arms["in_belt"]
    ck.check("合成场景（知识）：**带外才产生知识**——日心距在 `mond.radius` 以内的舰一点也不算",
             all(x == 0.0 for x in belt["pres"]) and all(x == 0.0 for x in belt["target"])
             and all(x == 0.0 for x in belt["m"]),
             f"停在 {belt['rau']} AU（半径 {radius}）⇒ 强度 {sorted(set(belt['pres']))}、"
             f"目标 {sorted(set(belt['target']))}、掌握度 {sorted(set(belt['m']))}")

    want = len(names) * (1.0 + 2.0 * w)
    sh = arms["shallow"]
    ck.check("合成场景（知识）：在场强度 = 舰数 × `(1 + 深度 × 权重)`（深处的**一艘**就顶浅处好几艘）",
             all(abs(x - want) < 0.011 for x in sh["pres"]),
             f"停在深 {sh['depth']:.0f} AU ⇒ 强度 {sorted(set(round(x, 3) for x in sh['pres']))}，"
             f"期望 {want:.3f}（{len(names)} 艘 × (1 + {sh['depth']:.0f} × {w})）")
    ck.check("合成场景（知识）：同样一批舰**停得越深强度越大**（外缘永远值得派人去）",
             max(arms["deep"]["pres"]) > max(sh["pres"]) > 0.0,
             f"浅 {max(sh['pres']):.3f} < 深 {max(arms['deep']['pres']):.3f}")

    ck.check("合成场景（知识）：**浅驻泊的目标严格在 (0,1)**（饱和而非断崖）",
             all(0.0 < x < 1.0 for x in sh["target"]),
             f"目标 {sorted(set(round(x, 3) for x in sh['target']))}")
    gap = [t - m for t, m in zip(sh["target"], sh["m"])]
    ck.check("合成场景（知识）：掌握度**朝目标爬、不超过目标、差距在缩小**",
             all(b >= a - 1e-9 for a, b in zip(sh["m"], sh["m"][1:]))
             and all(g >= -0.011 for g in gap)
             and gap[-1] < gap[0] - 1e-9,
             f"掌握度 {[round(x, 3) for x in sh['m'][:5]]}…；差 {gap[0]:.3f} → {gap[-1]:.3f}")

    dp = arms["deep"]
    rounds = int(meta["mond"]["knowledge"]["mastery_rounds"])
    ck.check("合成场景（知识）：**真·深空常驻 ⇒ 学满**（目标正好 1.0，掌握度在 `mastery_rounds` 内到顶）",
             all(x == 1.0 for x in dp["target"]) and dp["m"][-1] >= 1.0 - 1e-9
             and next((i for i, x in enumerate(dp["m"]) if x >= 1.0 - 1e-9), 10**9) <= rounds + 1,
             f"深 {dp['depth']:.0f} AU ⇒ 目标 {sorted(set(dp['target']))}，"
             f"掌握度第 {next((i for i, x in enumerate(dp['m']) if x >= 1.0 - 1e-9), -1)} 回合到顶"
             f"（`mastery_rounds` = {rounds}）")
    ref = float(meta["mond"]["knowledge"]["mastery_presence"])
    ck.check("合成场景（知识）：`--call mond_target` 在门槛两侧是 `<1` / `1.0`（差一点就是差的）",
             call_mond_target(h, ref - 0.1) < 1.0 and call_mond_target(h, ref) == 1.0,
             f"强度 {ref - 0.1} → {call_mond_target(h, ref - 0.1):.6f}；"
             f"强度 {ref} → {call_mond_target(h, ref)}")


def call_mond_target(h, presence: float) -> float:
    """问引擎要某个在场强度对应的目标掌握度（`--call mond_target`）。"""
    return float(json.loads(h.capture(["--call", "mond_target",
                                       "--args", json.dumps({"presence": presence})]))["value"])


def governance_scenario_checks(h, ck) -> None:
    """**合成场景 · 治理的两条**（`sim/tests/governance.rs`，第 7 批）：两臂**只差 `MOND 掌握度`**。

    | 原件 | 判据 |
    | --- | --- |
    | `a_mond_master_keeps_a_deep_city_loyal_where_a_mortal_loses_it` | 把一座城搬到**离首都最远**的天体、娱乐预算钉 0：凡人掉（0.50→0.37）、掌握者回升（0.50→0.62）、差 > 0.2、掌握者的城没丢 |
    ⚠ `mastery_does_not_pay_the_governance_bill` **没搬**：它要「覆盖率 0 ⇒ 欠费暴跌支路」，
    而**完整回合里够不到**——产出先到账，覆盖率恒 > 0（实测国库清零后忠诚仍然稳在 1.0）。
    那是 `step_governance` 的单元测（§4 内部契约类）。

    造法全是现成入口：势力 `MOND 掌握度` / 国库、城的 `所在天体` / `忠诚度` 走 `patch`
    （都有身份键），娱乐预算走 `--apply` 的 `城市福利预算`。
    """
    seed = SCENARIO_SEED
    q0 = KIT.load(str(h.projection(seed, 1)), only=("cities", "factions"))
    ci, fr = q0.table("cities"), q0.table("factions")
    mine = ci[(ci["round"] == 0) & (ci["势力"] == GOV_FID)]
    city = mine.loc[mine["gov_distance"].idxmin()]
    f0 = fr[(fr["round"] == 0) & (fr["势力"] == GOV_FID)]
    keys = sorted((list(f0["资源"])[0] or {}).keys())
    cap_body = str(list(f0["capital_body"])[0])
    deep = max((b for b in set(ci["天体名"]) if b != cap_body),
               key=lambda b: float(ci[(ci["天体名"] == b)]["gov_distance"].max()))
    ck.check("合成场景（治理）：探针世界里那座城与那个深空天体都在（防空转）",
             bool(city["城名"]) and deep != cap_body,
             f"{GOV_FID} 取城 {city['城名']}（在首都 {cap_body}），搬到 {deep}")
    zero_welfare = {"control": [{"势力": GOV_FID,
                                 "城市福利预算": [{"城": city["城名"], "值": 0.0, "归属": "Player"}]}]}

    # ① 深空 A/B：只拨掌握度。
    deep_arms = {}
    for tag, mc in (("mortal", 0.0), ("master", 1.0)):
        patch = {"factions": {GOV_FID: {"资源": {k: 5000.0 for k in keys}, "MOND 掌握度": mc}},
                 "cities": {city["城名"]: {"所在天体": deep, "忠诚度": 0.5}}}
        proj = h.scenario_apply(f"gov_deep_{tag}", seed, GOV_ROUNDS, [zero_welfare], patch=patch)
        rows = KIT.load(str(proj), only=("cities",)).table("cities")
        rows = rows[rows["城名"] == city["城名"]].sort_values("round")
        deep_arms[tag] = {"loy": [float(x) for x in rows["忠诚度"]], "owner": [str(x) for x in rows["势力"]]}
    dm, ds = deep_arms["mortal"], deep_arms["master"]
    ck.check("合成场景（治理）：**凡人守不住深空的城**（忠诚逐回合下滑，防空转）",
             dm["loy"][-1] < dm["loy"][0] - 1e-9,
             f"凡人（掌握 0）忠诚 {[round(x, 3) for x in dm['loy']]}")
    ck.check("合成场景（治理）：**掌握者守得住**（忠诚回升、比凡人高 0.2 以上、城没丢）",
             ds["loy"][-1] > ds["loy"][0] + 1e-9 and ds["loy"][-1] > dm["loy"][-1] + 0.2
             and ds["owner"][-1] == GOV_FID and ds["loy"][-1] > 0.6,
             f"掌握者忠诚 {[round(x, 3) for x in ds['loy']]}（凡人末端 {dm['loy'][-1]:.3f}）；"
             f"末端归属 {ds['owner'][-1]}")

def duel_scenario_checks(h, ck) -> None:
    """**合成场景 · 护盾先吸、船体再吃溢出**（`sim/combat.rs::combat_respects_shields_and_speed_evasion` 的**前半**，第 7 批）。

    这条**长局读面证不了**：seed 42 / 400 回合的 **249 发**里，`absorbed > 0` 的有 **0 发**
    ——没人装护盾组件。所以照原件那样**造**一仗：把一艘中国舰装 `railgun`、把一艘美国舰装
    `shield`（护盾打满、船体 24），两舰摆在远离任何首都的地方（本土防御倍率 = 1），
    再把两家的关系压到 `-35` ⇒ 下个回合就是一场真仗。全部字段（`坐标`/`组件`/`护盾`/`船体`/`关系`）
    都有身份键，直接 `patch`。

    后半（**回避**：目标越快命中折减越低）早在 g1 的 `--call hit_factor` 里压着。
    """
    seed = SCENARIO_SEED
    st = h.state_dump(h.gen(CACHE_ROOT / "scenario" / "_duel_probe.json", seed))
    atk = next(s for s in st["ships"] if s["势力"] == DUEL_ATK)
    dfd = next(s for s in st["ships"] if s["势力"] == DUEL_DEF)
    patch = {"ships": {
        atk["舰名"]: {"坐标": [80.0, 80.0], "组件": [DUEL_GUN], "组件耐久": [18.0]},
        dfd["舰名"]: {"坐标": [80.4, 80.0], "组件": ["shield"], "组件耐久": [18.0],
                      "船体": 24.0, "船体上限": 24.0,
                      "护盾": 12.0, "护盾上限": 12.0}},
        "factions": {DUEL_ATK: {"关系": {DUEL_DEF: -35.0}},
                     DUEL_DEF: {"关系": {DUEL_ATK: -35.0}}}}
    proj = h.scenario("duel_live", seed, DUEL_ROUNDS, patch)
    q = KIT.load(str(proj), only=("events", "ships"))
    ev = q.table("events")
    shots = [s for _, r in ev[ev["type"] == "attack"].iterrows()
             if r["target_id"] == dfd["舰名"]
             for s in (r["data"] or {}).get("逐发") or [] if not s.get("未击发")]
    ck.check("合成场景（护盾）：那一仗真的打起来了（有齐射指着守方，防空转）",
             bool(shots), f"{atk['舰名']}→{dfd['舰名']}：{len(shots)} 发"
             if shots else "一發都没有（构造成立？）")
    if not shots:
        return
    absorbing = [s for s in shots if float(s["护盾吸收"] or 0) > 1e-9]
    spill = [s for s in shots if float(s["实入船体"] or 0) > 1e-9]
    ck.check("合成场景（护盾）：**护盾池优先吸收**（`absorbed > 0`，且不超过这一发的伤害）",
             bool(absorbing) and all(float(s["护盾吸收"]) <= float(s["伤害"]) + 1e-9
                                     for s in absorbing),
             f"{len(absorbing)}/{len(shots)} 发被吸收；样本 "
             f"{[(round(float(s['伤害']), 3), round(float(s['护盾吸收']), 3)) for s in absorbing[:2]]}")
    ck.check("合成场景（护盾）：**船体也吃溢出**（护盾挡不完 ⇒ `hull_pen > 0`）",
             bool(spill),
             f"{len(spill)}/{len(shots)} 发打进船体；样本 "
             f"{[(round(float(s['护盾吸收']), 3), round(float(s['实入船体']), 3)) for s in spill[:2]]}")
    sh = q.table("ships")
    mine = sh[sh["舰名"] == dfd["舰名"]].sort_values("round")
    first, last = mine.iloc[0], mine.iloc[-1]
    ck.check("合成场景（护盾）：守方的护盾与船体都掉了，但**没被一炮打死**",
             float(last["护盾"]) < float(first["护盾"]) and float(last["船体"]) < float(first["船体"])
             and float(last["船体"]) > 0.0,
             f"护盾 {float(first['护盾']):.2f} → {float(last['护盾']):.2f}；"
             f"船体 {float(first['船体']):.2f} → {float(last['船体']):.2f}")


def pd_cover_scenario_checks(h, ck) -> None:
    """**合成场景 · 舰队防空屏护**（`sim/combat.rs::fleet_air_defense_covers_nearby_missile_targets`，第 7 批）。

    两臂**只差那艘 PD 友舰的坐标**：攻方装 `missile`、目标（无组件）在两艘舰当中，
    友舰装 `point_defense`——一次摆在目标旁边（41, 40），一次摆到 (100, 100)。

    判据直接用**逐发的 `pd` / `pd_absorbed`**（读面就有）：附近有 PD ⇒ 挡住一部分、
    目标的 `damage` 更低；PD 舰一远 ⇒ 拦截归零。这比原件直调 `cluster_pd_cover` 更贴
    「导弹有没有被拦下来」这件事。
    """
    seed = SCENARIO_SEED
    st = h.state_dump(h.gen(CACHE_ROOT / "scenario" / "_pd_probe.json", seed))
    atk = next(s for s in st["ships"] if s["势力"] == PD_ATK)
    dfd = next(s for s in st["ships"] if s["势力"] == PD_DEF)
    friend = next(s for s in st["ships"] if s["势力"] == PD_DEF and s["舰名"] != dfd["舰名"])
    arms = {}
    for tag, pdpos in (("near", [41.0, 40.0]), ("far", [100.0, 100.0])):
        patch = {"ships": {
            atk["舰名"]: {"坐标": [40.5, 40.0], "组件": ["missile"], "组件耐久": [18.0]},
            dfd["舰名"]: {"坐标": [40.0, 40.0], "组件": [], "组件耐久": []},
            friend["舰名"]: {"坐标": pdpos, "组件": ["point_defense"], "组件耐久": [18.0]}},
            "factions": {PD_ATK: {"关系": {PD_DEF: -35.0}}, PD_DEF: {"关系": {PD_ATK: -35.0}}}}
        proj = h.scenario(f"pd_{tag}", seed, PD_ROUNDS, patch)
        ev = KIT.load(str(proj), only=("events",)).table("events")
        shots = [s for _, r in ev[ev["type"] == "attack"].iterrows() if r["target_id"] == dfd["舰名"]
                 for s in (r["data"] or {}).get("逐发") or [] if not s.get("未击发")]
        arms[tag] = {"n": len(shots),
                     "pd": [float(s["点防拦截"] or 0.0) for s in shots],
                     "absorbed": [float(s["点防吃掉"] or 0.0) for s in shots],
                     "dmg": [float(s["伤害"] or 0.0) for s in shots]}
    ck.check("合成场景（防空）：两臂都真的打起来了（防空转）",
             arms["near"]["n"] >= 1 and arms["far"]["n"] >= 1,
             f"近处 PD {arms['near']['n']} 发、远处 PD {arms['far']['n']} 发")
    if not (arms["near"]["n"] and arms["far"]["n"]):
        return
    ck.check("合成场景（防空）：**附近有 PD 就替友舰拦导弹**（`pd > 0` 且真的吸掉了一部分）",
             all(x > 0.0 for x in arms["near"]["pd"])
             and all(x > 0.0 for x in arms["near"]["absorbed"]),
             f"近处拦截量 {arms['near']['pd']}、吸收 {arms['near']['absorbed']}")
    ck.check("合成场景（防空）：**PD 舰一远，屏护就没了**（拦截归零、目标实收伤害更高）",
             all(x == 0.0 for x in arms["far"]["pd"])
             and min(arms["far"]["dmg"]) > min(arms["near"]["dmg"]),
             f"远处拦截 {arms['far']['pd']}、伤害 {arms['far']['dmg']}；"
             f"近处拦截 {arms['near']['pd']}、伤害 {arms['near']['dmg']}")


def intercept_scenario_checks(h, ck) -> None:
    """**合成场景 · 被拦光的齐射也留一条事件**（`sim/shots.rs::a_fully_intercepted_salvo_still_leaves_an_event`，第 7 批）。

    两臂**只差守方装不装点防**：攻方一发 `missile`，守方空手 / 两层 `point_defense`。
    「被拦光」这件事长局里几乎不出现（seed 42 / 400 回合里那种齐射只有 **2 条**，多数种子 0 条）
    ⇒ 只能造。判据：

    * 拦光臂：`pd_absorbed > 0`、`pd` 盖过一发导弹、**`damage == 0` 且 `hull_pen == 0`**，
      而**那条 `attack` 事件照样在**（`magnitude == 0` 不等于不发事件——这正是这条要回答的那一格）；
    * 空手臂：`pd == 0` 且 `damage > 0`（防空转：证明「0 伤害」是拦截的功劳）；
    * 一件武器一发 = **一条逐发记录**。
    """
    seed = SCENARIO_SEED
    st = h.state_dump(h.gen(CACHE_ROOT / "scenario" / "_intc_probe.json", seed))
    atk = next(s for s in st["ships"] if s["势力"] == INTC_ATK)
    dfd = next(s for s in st["ships"] if s["势力"] == INTC_DEF)
    arms = {}
    for tag, comps in (("bare", []), ("two_pd", ["point_defense", "point_defense"])):
        patch = {"ships": {
            atk["舰名"]: {"坐标": [40.5, 40.0], "组件": ["missile"], "组件耐久": [18.0]},
            dfd["舰名"]: {"坐标": [40.0, 40.0], "组件": comps, "组件耐久": [18.0] * len(comps)}},
            "factions": {INTC_ATK: {"关系": {INTC_DEF: -35.0}},
                         INTC_DEF: {"关系": {INTC_ATK: -35.0}}}}
        proj = h.scenario(f"intc_{tag}", seed, INTC_ROUNDS, patch)
        ev = KIT.load(str(proj), only=("events",)).table("events")
        rows = [r for _, r in ev[ev["type"] == "attack"].iterrows() if r["target_id"] == dfd["舰名"]]
        arms[tag] = {"salvos": len(rows), "mag": [float(r["magnitude"] or 0.0) for r in rows],
                     "shots": [s for r in rows for s in (r["data"] or {}).get("逐发") or []]}
    bare, full = arms["bare"], arms["two_pd"]
    ck.check("合成场景（拦光齐射）：两臂都真的打起来了（防空转）",
             bare["shots"] and full["shots"],
             f"空手臂 {len(bare['shots'])} 发、两层点防臂 {len(full['shots'])} 发")
    if not (bare["shots"] and full["shots"]):
        return
    ck.check("合成场景（拦光齐射）：两层点防把这一发**吃光**（`damage == 0`、`hull_pen == 0`、吸收为正）",
             all(float(s["伤害"]) == 0.0 and float(s["实入船体"]) == 0.0
                 and float(s["点防吃掉"]) > 0.0 for s in full["shots"]),
             f"逐发 {[(s['点防拦截'], s['点防吃掉'], s['伤害']) for s in full['shots']]}")
    ck.check("合成场景（拦光齐射）：**被拦光也照样留一条 `attack` 事件**（只有伤害是 0）",
             full["salvos"] >= 1 and all(m == 0.0 for m in full["mag"])
             and all(not s.get("未击发") and s.get("在射程内") for s in full["shots"]),
             f"{full['salvos']} 条齐射、聚合伤害 {full['mag']}，逐发 skipped/in_range = "
             f"{[(s.get('未击发'), s.get('在射程内')) for s in full['shots']]}")
    ck.check("合成场景（拦光齐射）：**空手臂真的打得出伤害**（0 伤害不是世界本来就这样）",
             all(float(s["点防拦截"]) == 0.0 and float(s["伤害"]) > 0.0 for s in bare["shots"]),
             f"空手臂逐发 pd/damage = {[(s['点防拦截'], s['伤害']) for s in bare['shots']]}")
    ck.check("合成场景（拦光齐射）：一件武器一发 = **一条逐发记录**（齐射数与逐发数一一对应）",
             all(len(arms[t]["shots"]) == len(arms[t]["mag"]) for t in arms),
             f"齐射 {full['salvos']} 条 / 逐发 {len(full['shots'])} 条")


def follow_scenario_checks(h, ck) -> None:
    """**合成场景 · 跟随者只打敌人**（`sim/fleet.rs::follow_ship_auto_attacks_hostile_but_not_the_followed_friend`，第 7 批）。

    钉一片玩家 `Follow{友舰}` 叶，再把三方摆开：跟随者与友舰同在 (0,0)、一艘敌舰贴在 0.3 AU
    （在护卫舰射程内）、其余敌舰撵到 (50,50)，两家关系压到 `-35`。判据三条：

    * 跟随者**自动开火打敌人**（`attack` 事件 actor=跟随者、target=敌舰）；
    * 跟随者**绝不打自己跟着的友舰**（没有一条指向友舰的 `attack`）；
    * 那片 Follow 叶**没被降级**（`order_effective` 仍是 `Follow{友舰}`、叶片仍是 `Player`）。
    """
    seed = SCENARIO_SEED
    st = h.state_dump(h.gen(CACHE_ROOT / "scenario" / "_follow_probe.json", seed))
    cn = [s for s in st["ships"] if s["势力"] == FOLLOW_FID]
    us = [s for s in st["ships"] if s["势力"] == FOLLOW_ENEMY]
    fol, friend = cn[0], cn[1]
    enemy, rest = us[0], us[1:]
    patch = {"ships": {fol["舰名"]: {"坐标": [0.0, 0.0]}, friend["舰名"]: {"坐标": [0.0, 0.0]},
                       enemy["舰名"]: {"坐标": [0.3, 0.0]},
                       **{s["舰名"]: {"坐标": [50.0, 50.0]} for s in rest}},
             "factions": {FOLLOW_FID: {"关系": {FOLLOW_ENEMY: -35.0}},
                          FOLLOW_ENEMY: {"关系": {FOLLOW_FID: -35.0}}}}
    diff = {"control": [{"势力": FOLLOW_FID, "指令": [
        {"舰": fol["舰名"], "行为": {"Follow": {"ship": friend["舰名"]}}, "归属": "Player"}]}]}
    proj = h.scenario_apply("follow_live", seed, FOLLOW_ROUNDS, [diff], patch=patch)
    q = KIT.load(str(proj), only=("events", "ships"))
    ev = q.table("events")
    at = [(int(r["round"]), r["actor_id"], r["target_id"]) for _, r in ev[ev["type"] == "attack"].iterrows()]
    ck.check("合成场景（跟随）：跟随者**自动开火打敌人**（防空转：这仗真的打了）",
             any(a == fol["舰名"] and t == enemy["舰名"] for _, a, t in at),
             f"{fol['舰名']}→{enemy['舰名']}；本局攻击事件 {at[:4]}")
    ck.check("合成场景（跟随）：跟随者**绝不打自己跟着的友舰**",
             not any(t == friend["舰名"] for _, _, t in at),
             f"指向 {friend['舰名']} 的攻击事件 {[x for x in at if x[2] == friend['舰名']]}")
    sh = q.table("ships")
    mine = sh[sh["舰名"] == fol["舰名"]].sort_values("round")
    orders = [str(o) for o in mine["order_effective"]]
    modes = [str(m) for m in mine["order_leaf_mode"]]
    ck.check("合成场景（跟随）：那片 Follow 叶**没被降级**（仍是 Follow{友舰}、叶片仍是 Player）",
             bool(orders) and all("Follow" in o and friend["舰名"] in o for o in orders)
             and set(modes) == {"Player"},
             f"{len(orders)} 个回合的指令 {sorted(set(orders))}；叶片 {sorted(set(modes))}")


def site_build_scenario_checks(h, ck) -> None:
    """**合成场景 · 谁能给城出钱**（`sim/tests/site_supply.rs` 的前两条，第 7 批）。

    开局 `depots` 本来就是空的 ⇒ 原件的 `gut_all_stock`（清池 + 清货栈）在回合 0 是**空操作**，
    所以这两条能原样造：把城改成「还差一半没建」（`建筑[].已建成面积 = 面积 × 0.5`，
    `护甲` 跟着改），再只拨**池子**。

    | 原件 | 判据 |
    | --- | --- |
    | `the_capital_body_spends_the_faction_pool` | 首都城：池满 ⇒ **第 1 回合就长**；池空 ⇒ 第 1 回合不长 |
    | `only_the_local_depot_can_fund_an_offsite_city` | 非首都城：池满 ⇒ **第 1 回合不变**（池子到不了别人家门口）；自然跑 40 回合里**确实长过**（本地货栈就是它的钱包） |

    ⚠ 判据只敢看**第 1 回合**：城自己会产出，「池空」臂到 r3 也会长起来（实测 60→69.12），
    而「非首都·池满」臂到 r4 也会开始长 ⇒ 拿多回合当判据会变成假绿。
    """
    seed = SCENARIO_SEED
    q0 = KIT.load(str(h.projection(seed, 1)), only=("cities", "factions"))
    ci, fr = q0.table("cities"), q0.table("factions")
    f0 = fr[fr["round"] == 0]
    cap_body = str(list(f0[f0["势力"] == SITE_FID]["capital_body"])[0])
    mine = ci[(ci["round"] == 0) & (ci["势力"] == SITE_FID)]
    cap_city = mine[mine["天体名"] == cap_body].iloc[0]
    off_city = mine[mine["天体名"] != cap_body].iloc[0]
    rich = {r["势力"]: {"资源": ({k: 5000.0 for k in SITE_STOCK} if r["势力"] == SITE_FID else {})}
            for _, r in f0.iterrows()}
    broke = {r["势力"]: {"资源": {}} for _, r in f0.iterrows()}
    ck.check("合成场景（谁给城出钱）：首都城与非首都城都找到了（防空转）",
             cap_city["城名"] != off_city["城名"] and len(cap_city["建筑"]) >= 1,
             f"首都城 {cap_city['城名']}@{cap_body}（{len(cap_city['建筑'])} 栋）；"
             f"非首都城 {off_city['城名']}@{off_city['天体名']}（{len(off_city['建筑'])} 栋）")

    def half(city):
        return [dict(b, **{"已建成面积": float(b["面积"]) * 0.5,
                           "护甲": float(b["面积"]) * 0.5}) for b in city["建筑"]]

    def built(rows, name):
        out = []
        for _, r in rows[rows["城名"] == name].sort_values("round").iterrows():
            out.append(round(sum(float(b["已建成面积"]) for b in r["建筑"]), 4))
        return out

    arms = {}
    for tag, city, res, rnds in (("cap_rich", cap_city, rich, 3), ("cap_broke", cap_city, broke, 3),
                                 ("off_rich", off_city, rich, 3), ("off_natural", off_city, None, 40)):
        patch = {"cities": {city["城名"]: {"建筑": half(city)}}}
        if res:
            patch["factions"] = res
        proj = h.scenario(f"site_{tag}", seed, rnds, patch)
        rows = KIT.load(str(proj), only=("cities",)).table("cities")
        arms[tag] = built(rows, city["城名"])

    ck.check("合成场景（谁给城出钱）：**首都城第 1 回合就花池子长**（池满）",
             arms["cap_rich"][1] > arms["cap_rich"][0] + 1e-9,
             f"首都城已建面积 {arms['cap_rich']}")
    ck.check("合成场景（谁给城出钱）：**池空则第 1 回合不长**（防空转：长的是池子出的钱）",
             arms["cap_broke"][1] == arms["cap_broke"][0],
             f"池空臂已建面积 {arms['cap_broke']}")
    ck.check("合成场景（谁给城出钱）：**非首都城池满也长不动**（池子到不了别人家门口）",
             arms["off_rich"][1] == arms["off_rich"][0],
             f"非首都城（池满 5000）已建面积 {arms['off_rich']}")
    ck.check("合成场景（谁给城出钱）：**本地货栈才是它的钱包**——自然跑 40 回合里确实长过",
             arms["off_natural"][-1] > arms["off_natural"][0] + 1e-9,
             f"非首都城（不给池子）已建面积 {arms['off_natural'][0]} → {arms['off_natural'][-1]}")


def ideology_war_scenario_checks(h, ck) -> None:
    """**合成场景 · 战争得利把思潮推向军国**（`sim/ideology.rs::ideology_military_win_drives_toward_militarism`，第 7 批）。

    两臂**只差两家关系**：把「开局最偏和平端」那个势力的一艘舰装 `railgun`、敌舰船体压到 1
    （一发即沉），摆在远离首都处；打仗臂把两家关系压到 `-35` ⇒ 那一回合真的产生一次**我方击杀**。

    为什么能这么判：`和平↔军国` 的目标**只**由军事信号决定（`clamp(净战果 × military_scale)`）
    ⇒ 有击杀时它朝 `+0.5` 走、没击杀时朝 `0` 走，两者的位移**必然差一截**（实测 **0.06 vs 0.03**，
    正是 `0.05 × (0.5 − (−0.6))` 与 `0.05 × (0 − (−0.6))`）。判据只看**方向 + 谁走得多**，
    不去逐回合复算法条（列过 `r2`、每回合位移 ≤ 0.05 ⇒ 法条判据会退化成「怎么都过」）。
    """
    seed = SCENARIO_SEED
    q0 = KIT.load(str(h.projection(seed, 1)), only=("factions", "ships"))
    fac, sh = q0.table("factions"), q0.table("ships")
    f0 = fac[fac["round"] == 0].copy()
    f0["pm"] = [float(x["和平↔军国"]) for x in f0["思潮"]]
    row = f0.loc[f0["pm"].idxmin()]
    fid = str(row["势力"])
    armed = set(sh[sh["round"] == 0]["势力"])
    enemy = next((str(r["势力"]) for _, r in f0.iterrows()
                  if r["势力"] != fid and r["势力"] in armed), None)
    ck.check("合成场景（战争推思潮）：找得到「最偏和平端且有敌可打」的那一对（防空转）",
             enemy is not None and float(row["pm"]) < 0.0,
             f"{fid}（和平↔军国 = {row['pm']}）vs {enemy}")
    if enemy is None:
        return
    atk = sh[(sh["round"] == 0) & (sh["势力"] == fid)].iloc[0]
    tgt = sh[(sh["round"] == 0) & (sh["势力"] == enemy)].iloc[0]
    arms = {}
    for tag in ("fight", "peace"):
        patch = {"ships": {
            atk["舰名"]: {"坐标": [80.0, 80.0], "组件": ["railgun"], "组件耐久": [18.0]},
            tgt["舰名"]: {"坐标": [80.4, 80.0], "船体": 1.0, "船体上限": 1.0,
                          "护盾": 0.0, "护盾上限": 0.0}}}
        if tag == "fight":
            patch["factions"] = {fid: {"关系": {enemy: -35.0}}, enemy: {"关系": {fid: -35.0}}}
        proj = h.scenario(f"ideo_{tag}", seed, IDEO_ROUNDS, patch)
        q = KIT.load(str(proj), only=("factions", "events"))
        ff = q.table("factions")
        ff = ff[ff["势力"] == fid].sort_values("round")
        pm = [float(x["和平↔军国"]) for x in ff["思潮"]]
        ev = q.table("events")
        kills = [r for _, r in ev[ev["type"] == "ship_destroyed"].iterrows()
                 if ((r["data"] or {}).get("凶手") or {}).get("势力") == fid]
        arms[tag] = {"pm": pm, "kills": len(kills), "d": round(pm[-1] - pm[0], 4)}

    ck.check("合成场景（战争推思潮）：打仗那一臂**真的产生了我方击杀**（防空转）",
             arms["fight"]["kills"] >= 1 and arms["peace"]["kills"] == 0,
             f"打仗臂击杀 {arms['fight']['kills']}｜对照臂 {arms['peace']['kills']}")
    ck.check("合成场景（战争推思潮）：**战争得利把「和平↔军国」推向军国端**",
             arms["fight"]["d"] > 0.0 and arms["fight"]["pm"][-1] > arms["fight"]["pm"][0],
             f"打仗臂 {arms['fight']['pm']}（Δ={arms['fight']['d']}）")
    ck.check("合成场景（战争推思潮）：**有战果的比没战果的走得多**（对照臂只是朝 0 松弛）",
             arms["fight"]["d"] > arms["peace"]["d"],
             f"打仗臂 Δ={arms['fight']['d']} > 对照臂 Δ={arms['peace']['d']}；"
             f"对照臂 {arms['peace']['pm']}")


def war_scar_scenario_checks(h, ck, out) -> None:
    """**合成场景 · 战争疤痕的形状**（`sim/war_scar.rs::war_scar_floor_shape_decays_over_its_window`，第 7 批）。

    新挂 `--call war_scar_floor {a, b}`（吃 state：它查 `notables` 里那一对的开战记录与当前回合之差）。
    从长局里取**一对真开过战的势力**与那一回合，然后在 `age = 0 / 1 / span−1 / span` 各造一份档问它：

    * `age = 0` ⇒ 正好 `diplomacy.war_scar_relation`（刚开战是满额敌意）；
    * 年龄越大地板越**高**（单调）；窗口内（`span−1`）仍 `< 0`；出了窗口（`span`）⇒ `null`；
    * 交换两端逐值相等；**一对从没打过仗的势力** ⇒ `null`（疤痕只属于开战的那一对）。

    ⚠ 原件用**合成世界**（往 `notables` 塞一条 r10 的开战记录）⇒ 这里是**真长局**里的疤，
    取样回合随第一场战争而定（不写死 r10）。
    """
    seed = SEEDS[rep_index(out, "war_first")]
    first = next((d["war_first"] for d in out if d["war_first"]), None)
    never = next((d["war_never"] for d in out if d["war_never"]), None)
    span = int(meta_diplomacy(h)["war_scar_rounds"])
    base = float(meta_diplomacy(h)["war_scar_relation"])
    ck.check("合成场景（战争疤痕）：长局里真的打过仗，也存在从没打过仗的一对（防空转）",
             first is not None and never is not None,
             f"第一场 {first}｜从没打过的一对 {never}｜窗口 {span} 回合、满额敌意 {base}")
    if first is None or never is None:
        return
    rnd, (a, b) = first

    def floor_at(age: int, x: str, y: str):
        ckpt = h.gen(CACHE_ROOT / "scenario" / f"_ws_s{seed}_a{age}.json", seed, rnd + age)
        return json.loads(h.capture(["--start", str(ckpt), "--call", "war_scar_floor",
                                     "--args", json.dumps({"a": x, "b": y}, ensure_ascii=False)]))["value"]

    ages = [0, 1, 2, span - 1, span]
    vals = [floor_at(age, a, b) for age in ages]
    ck.check("合成场景（战争疤痕）：刚开战是**满额敌意**（`age = 0` 正好等于配置里的初值）",
             vals[0] is not None and abs(float(vals[0]) - base) < 1e-9,
             f"{a}×{b} 在 r{rnd} 的地板 = {vals[0]}（配置初值 {base}）")
    ck.check("合成场景（战争疤痕）：地板随年龄**单调抬高**，窗口内仍 `< 0`，出了窗口彻底消失",
             all(float(x) < float(y) for x, y in zip(vals[:3], vals[1:4]))
             and float(vals[3]) < 0.0 and vals[4] is None,
             f"年龄 {ages} ⇒ 地板 {vals}（窗口 {span}）")
    ck.check("合成场景（战争疤痕）：**与势力顺序无关**（交换两端逐值相等）",
             all(floor_at(age, b, a) == v for age, v in zip(ages[:3], vals[:3])),
             f"正向 {vals[:3]}｜反向 {[floor_at(age, b, a) for age in ages[:3]]}")
    ck.check("合成场景（战争疤痕）：**没打过仗的一对没有疤**（疤痕不牵连别人）",
             floor_at(0, never[0], never[1]) is None and floor_at(span - 1, never[0], never[1]) is None,
             f"{never[0]}×{never[1]} 在 age 0 / {span - 1} 都返回 null")


def rep_index(out, key: str) -> int:
    """哪一份摘要里有这个键（`h.gen` 要配对同一个 seed）。"""
    return next((i for i, d in enumerate(out) if d.get(key)), 0)


def meta_diplomacy(h) -> dict:
    """`meta.diplomacy`（战争疤痕的窗口与初值）。"""
    return json.loads(h.capture(["--meta"]))["diplomacy"]


def affinity_scenario_checks(h, ck) -> None:
    """**合成场景 · 思潮相似度决定静息亲和**（`sim/ideology.rs::ideology_similarity_shifts_diplomatic_affinity_directionally`，第 7 批）。

    两臂**只差两家思潮**：都给推到同一个极（相似度 = 1）或推到你死我活的两极（相似度 = 0），
    同时把 `阵营倾向`（alignment）压到 0、彼此的 `关系` 归零 ⇒ 关系的去向只能由思潮相似度解释。

    实测 `联合国 × 美国`：同极臂关系爬到 **+16.6**、对极臂掉到 **−53.9**（判据看方向 + 巨大间距，
    不比绝对值）。⚠ 原件把 `diplomacy.noise` 关掉（那是**配置**改动，读面没有开关）⇒ 这里靠
    **多回合**：每回合向静息值拉 `drift_rate`(0.02)，而噪声有界 ⇒ 30 回合后间距远大于噪声。
    """
    seed = SCENARIO_SEED
    q0 = KIT.load(str(h.projection(seed, 1)), only=("factions",))
    f0 = q0.table("factions")
    f0 = f0[f0["round"] == 0]
    a, b = str(f0.iloc[0]["势力"]), str(f0.iloc[1]["势力"])
    axes = ["和平↔军国", "科学↔技术", "人民↔精英", "自然↔殖民"]
    arms = {}
    for tag, bv in (("same", 1.0), ("opp", -1.0)):
        patch = {"factions": {
            a: {"思潮": {x: 1.0 for x in axes}, "阵营倾向": 0.0, "关系": {b: 0.0}},
            b: {"思潮": {x: bv for x in axes}, "阵营倾向": 0.0, "关系": {a: 0.0}}}}
        proj = h.scenario(f"affin_{tag}", seed, AFFIN_ROUNDS, patch)
        ff = KIT.load(str(proj), only=("factions",)).table("factions")
        ff = ff[ff["势力"] == a].sort_values("round")
        arms[tag] = [float((r["关系"] or {}).get(b, 0.0)) for _, r in ff.iterrows()]

    sim_same = call_ideology_similarity(h, {x: 1.0 for x in axes}, {x: 1.0 for x in axes})
    sim_opp = call_ideology_similarity(h, {x: 1.0 for x in axes}, {x: -1.0 for x in axes})
    ck.check("合成场景（静息亲和）：两臂的思潮相似度确实是 1 与 0（前提，防空转）",
             abs(sim_same - 1.0) < 1e-9 and sim_opp == 0.0,
             f"{a}×{b}：同极相似度 {sim_same}、对极 {sim_opp}")
    ck.check("合成场景（静息亲和）：两臂的关系都真的动了（外交那一步跑了）",
             abs(arms["same"][-1]) > 1.0 and abs(arms["opp"][-1]) > 1.0,
             f"同极末端 {arms['same'][-1]:.3f}｜对极末端 {arms['opp'][-1]:.3f}")
    ck.check("合成场景（静息亲和）：**思潮相似的一方静息关系更友好**（间距远大于噪声）",
             arms["same"][-1] > arms["opp"][-1] + 10.0,
             f"同极 {arms['same'][-1]:.3f} vs 对极 {arms['opp'][-1]:.3f}"
             f"（间距 {arms['same'][-1] - arms['opp'][-1]:.1f}）")


def build_line_scenario_checks(h, ck) -> None:
    """**合成场景 · 三条线分得开：钱、库存、产能**（`sim/spending.rs::build_lines_separate_the_money_bottleneck_from_the_capacity_ceiling`，第 7 批）。

    读面本来就有那两格：`city_process.build[舰级] = {rate, increment}`（schema 的原话：
    **"`increment < rate` ⇒ 钱是瓶颈；`increment ≈ rate` ⇒ 产能封顶"**）。造法也是现成入口：
    **建造预算**是控制面的 Player 叶（`建造预算: [{资源, 值, 归属}]`），**库存**是势力 `资源`。

    三条臂（同一座城、同一个舰级、同一个起点）：

    | 臂 | 库存 | 预算 | 实测 r1 |
    | --- | --- | --- | --- |
    | 两头都足 | 5000 | 5000 | `increment` **= rate = 11.92**（产能封顶） |
    | 钱批 0 | 5000 | 0 | `increment` **= 0**（钱是瓶颈） |
    | 库存 0 | 0 | 5000 | `increment` **≈ 0**（库存是瓶颈） |

    ⇒ 「钱不够」与「产能不够」是**两条不同的线**，而且第三条（库存）也看得见。
    """
    seed = SCENARIO_SEED
    q0 = KIT.load(str(h.projection(seed, 3)), only=("city_process", "factions"))
    cp, fa = q0.table("city_process"), q0.table("factions")
    rows = sorted(((float(v["rate"]), r["城名"], r["势力"], cls)
                   for _, r in cp[cp["round"] == 1].iterrows()
                   for cls, v in (r["build"] or {}).items() if float(v.get("rate", 0)) > 0),
                  reverse=True)
    ck.check("合成场景（建造瓶颈）：读面上真有 `rate > 0` 的建造线（防空转）",
             bool(rows), f"{len(rows)} 条；最大 {rows[0][1] if rows else '—'}")
    if not rows:
        return
    _, city, fid, cls = rows[0]
    res = sorted((list(fa[fa["势力"] == fid]["资源"])[0] or {}).keys())

    def run(tag, stock, budget):
        patch = {"factions": {fid: {"资源": {k: stock for k in res}}}}
        diff = {"control": [{"势力": fid,
                             "建造预算": [{"资源": k, "值": budget, "归属": "Player"} for k in res]}]}
        proj = h.scenario_apply(f"buildline_{tag}", seed, BUILD_ROUNDS, [diff], patch=patch)
        c = KIT.load(str(proj), only=("city_process",)).table("city_process")
        out = []
        for _, r in c[c["城名"] == city].sort_values("round").iterrows():
            b = (r["build"] or {}).get(cls)
            if b:
                out.append((float(b["rate"]), float(b["increment"])))
        return out

    rich = run("rich", 5000.0, 5000.0)
    poor = run("broke", 5000.0, 0.0)
    nostock = run("nostock", 0.0, 5000.0)
    ck.check("合成场景（建造瓶颈）：**两头都足 ⇒ 产能封顶**（`increment ≈ rate`）",
             rich and abs(rich[0][1] - rich[0][0]) < 1e-6 and rich[0][0] > 0,
             f"{city} 造 {cls}：rate {rich[0][0]:.3f}／increment {rich[0][1]:.3f}")
    ck.check("合成场景（建造瓶颈）：**钱批 0 ⇒ 一分进度都没有**（钱是瓶颈）",
             poor and all(i == 0.0 for _, i in poor) and poor[0][0] > 0,
             f"rate {poor[0][0]:.3f}／increment {[round(i, 4) for _, i in poor]}")
    ck.check("合成场景（建造瓶颈）：**库存 0 ⇒ 批满也拿不到进度**（第三条线：库存）",
             nostock and max(i for _, i in nostock) < 0.1 * max(r for r, _ in nostock),
             f"rate {nostock[0][0]:.3f}／increment {[round(i, 4) for _, i in nostock]}")
    ck.check("合成场景（建造瓶颈）：三条臂真的分得开（防空转）",
             rich[0][1] > max(i for _, i in nostock) and rich[0][1] > max(i for _, i in poor),
             f"两头足 {rich[0][1]:.3f}｜钱 0 {max(i for _, i in poor):.3f}｜"
             f"库存 0 {max(i for _, i in nostock):.3f}")


def gov_plate_scenario_checks(h, ck) -> None:
    """**合成场景 · 掌握度买的是「守得住」不是「管得起」**（`sim/governance.rs::mastery_does_not_pay_the_governance_bill`，第 7 批）。

    怎么把「覆盖率 0 ⇒ 欠费支路」**真的**造出来：把某势力的舰改成重舰（`battleship` + 三件重装）
    ⇒ 维护费**结构性**超过任何产出，国库当场见底 ⇒ `governance_coverage` 掉到 0 ✓。两臂**只差
    `MOND 掌握度`**（0 / 1），其余逐字相同。

    判据：**覆盖率真的到过 0**（防空转）+ 两臂的**忠诚度 / 覆盖率 / 欠费逐回合完全相同**
    ——付不出治理费时，掌握度一点忙也帮不上（它买的是深空城的忠诚，不是行政预算；
    后者由 g2 的**驻泊深度/治理**场景从正面压着）。
    """
    seed = SCENARIO_SEED
    q0 = KIT.load(str(h.projection(seed, 2)), only=("factions", "ships", "cities"))
    sh, ci = q0.table("ships"), q0.table("cities")
    f0 = q0.table("factions")
    f0 = f0[f0["round"] == 0]
    fid = str(f0.iloc[0]["势力"])
    mine = sh[(sh["round"] == 0) & (sh["势力"] == fid)]
    city = ci[(ci["round"] == 0) & (ci["势力"] == fid)].iloc[0]
    ck.check("合成场景（管不起）：找得到有舰的势力与它的城（防空转）",
             not mine.empty and bool(city["城名"]), f"{fid}：{len(mine)} 艘舰、城 {city['城名']}")
    if mine.empty:
        return
    arms = {}
    for tag, mc in (("mortal", 0.0), ("master", 1.0)):
        patch = {"ships": {r["舰名"]: {"舰级": "battleship", "组件": ["kinetic", "armor", "shield"],
                                       "组件耐久": [18.0] * 3} for _, r in mine.iterrows()},
                 "factions": {fid: {"MOND 掌握度": mc}}}
        proj = h.scenario(f"gov_plate_{tag}", seed, GOV_PLATE_ROUNDS, patch)
        q = KIT.load(str(proj), only=("cities", "faction_process"))
        c = q.table("cities")
        c = c[c["城名"] == city["城名"]].sort_values("round")
        pr = q.table("faction_process")
        pr = pr[pr["势力"] == fid].sort_values("round")
        arms[tag] = {"loy": [float(x) for x in c["忠诚度"]],
                     "cov": [float(x) for x in pr["governance_coverage"]],
                     "unpaid": [float(x) for x in pr["upkeep_unpaid"]]}
    m, s = arms["mortal"], arms["master"]
    ck.check("合成场景（管不起）：重舰把国库压垮 ⇒ **覆盖率真的到过 0**（欠费支路活着，防空转）",
             min(m["cov"]) == 0.0 and max(m["unpaid"]) > 0.0,
             f"覆盖率 {[round(x, 2) for x in m['cov']]}｜欠费 {[round(x, 2) for x in m['unpaid']]}")
    ck.check("合成场景（管不起）：**付不出治理费时，掌握度一点忙也帮不上**（两臂逐回合完全相同）",
             m["loy"] == s["loy"] and m["cov"] == s["cov"] and m["unpaid"] == s["unpaid"],
             f"凡人 忠诚 {[round(x, 2) for x in m['loy']]}｜掌握 "
             f"{[round(x, 2) for x in s['loy']]}（覆盖率与欠费也逐行相同）")


def id_checks(h, ck, out) -> None:
    """**建筑 id 永不复用**（State.next_building_id 单调计数器，见 id_report）。"""
    tag = f"{len(SEEDS)} seed x {ROUNDS} 回合"
    gaps = sum(d["ids"]["gap_n"] for d in out)
    total = sum(d["ids"]["ids"] for d in out)
    gone = sum(d["ids"]["gone_n"] for d in out)
    sample = next((m for d in out for m in d["ids"]["gaps"]), "")
    ck.check("建筑 id 一旦消失就不再回来（单调计数器 ⇒ 永不复用）", gaps == 0,
             f"{sample}（共 {gaps} 个）" if gaps else
             f"{tag}：{total:,} 个建筑 id 的出现回合都连成一段")
    ck.check("id 守卫没有空转（真的有 id 在窗口内消失过）", gone >= 5,
             f"{gone:,} 个 id 在本局内消失（下限 5）")


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

    yr = sum(b["yard_rows"] for b in bp)
    dg = sum(len(b["yard_dangling"]) for b in bp)
    sample = next((m for b in bp for m in b["yard_dangling"]), "")
    ck.check("建造区挂了图就必须挂到**存在**的图上（不许悬空，也不许窗口末尾还挂着不符的图）",
             dg == 0, f"{sample}（共 {dg} 处）" if dg else
             f"{tag}：{yr:,} 个「建造区·回合」的图指针全部落在图库里，且没有留下的舰级不符")
    ms = max((b["yard_max_streak"] for b in bp), default=0)
    mm = sum(b["yard_mismatch"] for b in bp)
    ck.check("守守卫没有空转（真检查过建造区）", yr >= 1000, f"{yr:,} 行（下限 1000）")
    ck.check("舰级不符只是**滞后**、会自己收敛（实测最长滞后）", ms <= 4,
             f"共 {mm} 行不符，最长连续 {ms} 回合（retool 当回合改了舰级、AI 下一趟把图对齐）")

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

    # 齐射一级的两条（`sim/shots.rs`，第 7 批）：逐发之和 = 聚合伤害；每条事件至少一发真打出去。
    sb = [(s, m) for s, d in zip(SEEDS, out) for m in d["combat"]["salvo_bad"]]
    salvos = sum(d["combat"]["salvo_n"] for d in out)
    skips = sum(d["combat"]["skipped_n"] for d in out)
    nolive = sum(d["combat"]["no_live"] for d in out)
    ck.check("齐射：逐发之和 = 聚合伤害，且每一发都在取值域里（命中折减∈[0.2,1]、护盾/护甲吸收、防御倍率、火力分配）",
             not sb, "；".join(f"seed {s}: {m}" for s, m in sb[:3])
             or f"{tag}：{salvos:,} 条齐射、{shots:,} 发逐发成立")
    ck.check("齐射：**瞄一艘已经沉了的舰不发事件**（每条 attack 至少有一发真打出去）",
             nolive == 0, f"{nolive} 条齐射全是跳过的发")
    ck.check("齐射守卫没有空转（真有过「跳过」的发，且真开过火）",
             skips > 0 and salvos > 0, f"{salvos:,} 条齐射里有 {skips} 发被跳过（射程外/目标已死）")

    # 组件损耗（第 7 批，`sim/combat.rs::fire_degrades_components_under_damage`）
    comp = [d["combat"]["comp"] for d in out]
    nh = sum(c["no_hit"] for c in comp)
    nh_bad = sum(c["no_hit_bad"] for c in comp)
    hit = sum(c["hit"] for c in comp)
    hit_drop = sum(c["hit_drop"] for c in comp)
    ck.check("组件损耗：**没挨打就不掉**（逐行；这是「修船」那条的孪生守卫）", nh_bad == 0,
             f"{tag}：{nh:,} 个「没挨打」的舰·回合零掉血")
    ck.check("组件损耗：**真打进船体就有组件掉了**（含被一炮打沉的）", hit > 0 and hit_drop > 0,
             f"{hit:,} 次真伤里有 {hit_drop:,} 次观察到组件耐久下降")
    ck.check("组件损耗守卫没有空转（既有挨打的、也有没挨打的）", nh > 0 and hit > 0,
             f"没挨打 {nh:,} / 挨打 {hit:,}")

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

    # 机械后果真的落到读面上（`story.rs::story_effects_apply` 那一条，2026-10 第 7 批）：
    # 声明了 `relations` 后果的每个节拍，触发那一回合那对势力的关系必须往**声明方向**动。
    eff_bad = [(s, m) for s, d in zip(SEEDS, out) for m in d["story_bad"]]
    eff_n = sum(d["story_rel_checked"] for d in out)
    ck.check("剧情的机械后果真的落到读面上（声明的每处关系增减都发生了）", not eff_bad,
             "；".join(f"seed {s}: {m}" for s, m in eff_bad[:3]) or f"{eff_n} 处关系增减逐处对上")
    ck.check("剧情后果守卫没有空转（真有声明了关系增减的节拍触发过）", eff_n > 0, f"{eff_n} 处")

    # `grant_ship` 的出厂位置与指令（`story_grant_ship_spawns_a_fleet_member`，第 7 批搬来）：
    # 位置 = 那个天体**这一刻**的位置 + (0.05, 0.05)，指令 Idle。判据由 `meta.story` 驱动。
    g_bad = [(s, m) for s, d in zip(SEEDS, out) for m in d["grant_bad"]]
    g_n = sum(d["grant_n"] for d in out)
    ck.check("剧情 `grant_ship` 真的多出一艘、且出厂位置 = 天体位置 + (0.05, 0.05)、指令 Idle",
             not g_bad, "；".join(f"seed {s}: {m}" for s, m in g_bad[:3]) or f"{g_n} 次出厂逐次对上")
    ck.check("剧情 `grant_ship` 守卫没有空转（真有节拍发过舰）", g_n > 0, f"{g_n} 次")

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
