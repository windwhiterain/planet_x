"""中组（T2，49–480 回合）：`src/tests/sim/horizon_mid.rs` 里那两条**事件/快照可判**的机制不变量。

为什么是这两条（其余留在 Rust）：

* **同回合自我复垦**：判据完全在事件层（`city_razed` + `colony_founded` 的先后与归属），
  数据面就够——不需要碰 crate 内部；
* **定制化舰真的出现过**：判据在舰表（`hull > 0` + `components` 非空），也一样；
* 其余中档用例（战争疤痕地板的形状、剧情编年史、编年史参与者）要么读**控制面/关系序列**、
  要么与 `fresh_world` 的内部夹具耦合，等下一轮按「读面表」逐张搬（见笔记的待办）。

⚠ 口径与 Rust 版**逐字相同**：种子 `[1, 7, 42]`、400 回合、拆平数 ≥ 20（防空转）、
「整局里出现过」而不是「末回合还剩着」。
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _harness import KIT, group_main  # noqa: E402

SEEDS = (1, 7, 42)
ROUNDS = 400
MIN_RAZINGS = 20          # 与 Rust 版同阈值：样本太小 ⇒ 守卫会空转，得报出来


def extract(dirpath):
    """事件层 + 舰表 → 一份小结（按投影缓存成 pickle）。

    返回 dict（不是每回合一行的表）：这一组的判据本来就只需要两个计数 + 违规样例。
    """
    q = KIT.load(str(dirpath), only=("events", "ships"))
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
            bad.append(f"seed r{int(r['round'])}：{r['target_id']} 被 {hit[1]} 丢掉后又被同一个势力复垦")

    # ② 「定制化」（装了组件的）**活舰**在整局里出现过——累计口径（末回合快照会随轨迹归零）。
    ships = q.table("ships")
    fitted = ships[ships["hull"] > 0]["components"]
    customized = int(fitted.map(lambda c: bool(c) and len(c) > 0).sum())

    return {"razings": razings, "refound_bad": bad, "customized": customized,
            "ships": int(len(ships)), "foundings": int((ev["type"] == "colony_founded").sum())}


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


if __name__ == "__main__":
    sys.exit(group_main("g2_mid", run))
