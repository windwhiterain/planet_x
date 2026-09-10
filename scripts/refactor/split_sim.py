"""把 src/sim.rs 按主题切成 src/sim/ 子模块。

安全网：把「头 + 所有块 + 尾部」按原顺序拼回去，必须与原文**逐字节相同** —— 否则
说明有行被吞掉或重复。任何边界错位都会在这里爆掉，而不是在游戏行为里悄悄体现。
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "src" / "sim.rs"

# (名字, 目标模块) —— 顺序必须与文件中的出现顺序一致。
ASSIGNMENT = [
    ("derived_from_state", "mod"),
    ("advance", "mod"),
    ("war_pairs", "relations"),
    ("dist", "geometry"),
    ("city_position", "geometry"),
    ("hostile", "relations"),
    ("faction_at_war", "relations"),
    ("war_scar_floor", "relations"),
    ("relation", "relations"),
    ("adjust_relation", "relations"),
    ("ideology_similarity", "ideology"),
    ("ev", "events"),
    ("kill_ship", "events"),
    ("sweep_dead_ships", "events"),
    ("ShipSpawn", "ships"),
    ("spawn_ship", "ships"),
    ("RazeCause", "cities"),
    ("raze_city", "cities"),
    ("wire_city_control", "cities"),
    ("reseed_city", "cities"),
    ("found_city", "cities"),
    ("building_health", "cities"),
    ("invest_weight", "production"),
    ("build_weight", "production"),
    ("city_loyalty_budget", "production"),
    ("deposit_area", "production"),
    ("labor_ratio", "production"),
    ("step_production", "production"),
    ("step_upkeep", "production"),
    ("military_need", "market"),
    ("relation_price_mult", "market"),
    ("trade_blocked", "market"),
    ("trade_block_cause", "market"),
    ("step_market", "market"),
    ("step_contracts", "market"),
    ("listed_value", "market"),
    ("pay_with_surplus", "market"),
    ("per_area_cost", "production"),
    ("budget_remaining", "production"),
    ("max_affordable_inc", "production"),
    ("commit_spend", "production"),
    ("step_construction", "construction"),
    ("build_city", "construction"),
    ("blueprint_launch_blocked", "construction"),
    ("blueprint_launch_waiting", "construction"),
    ("deterrence", "ships"),
    ("ship_power", "ships"),
    ("step_military", "military"),
    ("smoothstep", "geometry"),
    ("total_live_pop", "governance"),
    ("war_strength", "power"),
    ("faction_military_share", "governance"),
    ("faction_mond_ship_share", "governance"),
    ("faction_pop_share", "governance"),
    ("faction_colonizing", "governance"),
    ("ideology_loyalty_debuff", "governance"),
    ("faction_ideology_debuffs", "governance"),
    ("step_governance", "governance"),
    ("ideology_distance", "ideology"),
    ("most_ideologically_distant_faction", "ideology"),
    ("defect_city", "governance"),
    ("step_capital", "capital"),
    ("faction_capital_share", "capital"),
    ("highest_pop_city_body", "capital"),
    ("capital_anchor_cost", "capital"),
    ("move_toward", "geometry"),
    ("trade_anchor", "geometry"),
    ("route_depth", "mond"),
    ("is_mond_master", "mond"),
    ("mond_drift", "mond"),
    ("nav_roll", "mond"),
    ("fnv1a", "mond"),
    ("derived_roll", "mond"),
    ("mond_arrival_chance", "mond"),
    ("haul_split", "haul"),
    ("HaulStep", "haul"),
    ("HaulStep", "haul"),
    ("cargo_owner", "haul"),
    ("haul_load", "haul"),
    ("haul_unload", "haul"),
    ("haul_act", "haul"),
    ("haul_step", "haul"),
    ("hit_factor", "military"),
    ("fire", "military"),
    ("fire_concentrate", "military"),
    ("resolve_shot", "military"),
    ("cluster_pd_cover", "military"),
    ("home_defense_mult", "military"),
    ("home_regen_bonus", "military"),
    ("behavior_is_valid", "ships"),
    ("has_blank_site", "cities"),
    ("bombard_city", "military"),
    ("colonize", "military"),
    ("reset_order_keep_mode", "ships"),
    ("seed_colony_buildings", "cities"),
    ("behavior_dest", "ships"),
    ("step_diplomacy", "relations"),
    ("set_relation_sym", "relations"),
    ("faction_power", "power"),
    ("faction_power_share", "power"),
    ("coalition_of", "power"),
    ("dominant_hegemon", "power"),
    ("active_coalition_hegemon", "power"),
    ("sanctioned_hegemon", "power"),
    ("sanction_cost_mult", "power"),
    ("coalition_war_focus", "power"),
    ("balance_picture", "metrics"),
    ("round_metrics", "metrics"),
    ("offered_by_resource", "market"),
    ("step_balance_of_power", "power"),
    ("step_ideology", "ideology"),
    ("military_deltas", "events"),
    ("step_story", "story"),
    ("story_participants", "story"),
    ("grant_story_ship", "story"),
    ("story_trigger_fired", "story"),
]

DOC = {
    "capital": "首都：亡城强迁 + 周期性 AI 评估 + 迁都代价（贸易锚点/治理距离）。",
    "cities": "城市生命周期：立城 / 复垦 / 夷平 / 易主接线 + 殖民地建筑种子。",
    "construction": "建造：投资与建造权重 → 开工结算、造舰、设计图下水门控。",
    "events": "事件日志与舰船死亡：`ev` / `kill_ship` / 清尸，以及由事件历史推导的军事信号。",
    "geometry": "位置/距离/趋近等纯几何工具（不含导航掷骰，见 `mond`）。",
    "governance": "治理与忠诚：光速治理成本、人均面积、忠诚度、思潮扣分、城市叛变。",
    "haul": "集货运输：配额 → 抽签派单 → 装卸分录（货权守恒）。",
    "ideology": "思潮：相似度/距离、驱动量（战争得失 / MOND / 经济）与每回合演化。",
    "market": "市场与承包：价格、封锁、挂单/成交、雇佣运力市场、供需撮合。",
    "metrics": "回合派生汇总：`balance_picture` 与 `round_metrics`（单一权威观测）。",
    "military": "军事回合：舰队行为与移动、战斗结算、轰炸、殖民。",
    "mond": "MOND 异常导航：深处的偏移是**伪随机范围**，掷骰走 `(势力, 舰名, 回合, 用途)` 派生哈希。",
    "power": "权力/霸权/联盟/制裁与均势外交（反制联盟是概率化的，不是硬阈值）。",
    "production": "生产与维护：矿产出、货栈、劳动比、维护费、建造预算扣减。",
    "relations": "关系与外交：宣战/停战阈、好感调整、战痕地板、外交回合。",
    "ships": "舰船：下水与蓝图装载、行为合法性、行为目的地、单舰战力与威慑。",
    "story": "剧情编年史：数据驱动的叙事弧、触发条件与参与者。",
}

ITEMS = re.compile(r"^(?:(?:pub(?:\(crate\))?) )?(fn|struct|enum|impl|const|static|type) ([A-Za-z_][A-Za-z0-9_]*)")
DOC_LINE = re.compile(r"^\s*(///|//|#\[)")


def main() -> int:
    lines = SRC.read_text(encoding="utf-8").split("\n")
    # 代码区：到 `#[cfg(test)]` 之前（测试模块本轮先留在 mod.rs）。
    cfg_test = next(i for i, l in enumerate(lines) if l.startswith("#[cfg(test)]"))
    code_end = cfg_test  # exclusive

    found = []
    for i in range(code_end):
        m = ITEMS.match(lines[i])
        if m:
            found.append((i, m.group(1), m.group(2)))

    if [n for _, _, n in found] != [n for n, _ in ASSIGNMENT]:
        print("item 序列与预期不符：")
        got = [(k, n) for _, k, n in found]
        for i, (a, b) in enumerate(zip(got, ASSIGNMENT)):
            if a != b:
                print(f"  第一处分歧 @#{i}: 解析={a} 预期={b}")
                break
        print(f"  解析 {len(got)} 项 / 预期 {len(ASSIGNMENT)} 项")
        return 1

    # 每项的可动部分 = 它自己的 doc/属性注释行开始，到下一项的可动部分之前。
    def block_start(idx: int) -> int:
        i = found[idx][0]
        while i > 0 and DOC_LINE.match(lines[i - 1]):
            i -= 1
        return i

    starts = [block_start(i) for i in range(len(found))]
    ends = starts[1:] + [code_end]
    header = "\n".join(lines[: starts[0]])

    blocks: dict[str, list[str]] = {}
    order: list[str] = []
    for (name, module), s, e in zip(ASSIGNMENT, starts, ends):
        if module not in blocks:
            blocks[module] = []
            order.append(module)
        blocks[module].append("\n".join(lines[s:e]))

    # ---- 自校验：拼回去必须与原文完全相同 ----
    tail = "\n".join(lines[cfg_test:])
    rebuilt_code = "\n".join("\n".join(lines[s:e]) for s, e in zip(starts, ends))
    if rebuilt_code != "\n".join(lines[starts[0]:code_end]):
        print("!! 拼接结果与原文不一致 —— 边界错位，未写任何文件")
        return 1
    print("自校验通过：块拼接 == 原文代码区（逐字节）")

    # ---- 写文件 ----
    subs = [m for m in order if m != "mod"]
    for module in subs:
        body = "\n".join(blocks[module])
        (ROOT / "src" / "sim").mkdir(parents=True, exist_ok=True)
        (ROOT / "src" / "sim" / f"{module}.rs").write_text(
            f"//! {DOC[module]}\n\nuse super::*;\n\n{body}", encoding="utf-8"
        )

    decls = "\n".join(f"pub mod {m};" for m in sorted(subs))
    uses = "\n".join(f"pub use {m}::*;" for m in sorted(subs))
    mod_body = "\n\n".join(blocks["mod"])
    mod_rs = (
        header
        + "\n"
        + decls
        + "\n\n"
        + uses
        + "\n\n"
        + mod_body
        + "\n\n"
        + tail
    )
    (ROOT / "src" / "sim" / "mod.rs").write_text(mod_rs, encoding="utf-8")
    SRC.unlink()

    print(f"mod.rs = {len(mod_rs.split(chr(10)))} 行（含测试）")
    for m in sorted(subs):
        n = len((ROOT / "src" / "sim" / f"{m}.rs").read_text(encoding="utf-8").split("\n"))
        print(f"  {m}.rs = {n} 行")
    return 0


if __name__ == "__main__":
    sys.exit(main())
