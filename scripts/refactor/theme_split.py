"""把两个最大的测试文件按主题拆成子模块（`mod.rs` 保留夹具与零散用例）。

判据与 tier_split.py 相同：整块搬（文档注释/属性 → 函数体收尾的 `}`），列 0 缩进不变。
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FN = re.compile(r"^fn ([A-Za-z_][A-Za-z0-9_]*)\(")
ATTR_OR_COMMENT = re.compile(r"^\s*(///|//|#\[)")

SIM = {
    "blueprints.rs": (
        "设计图（`--apply` 的 blueprint 叶）与船坞下水：图压舰队默认、`order_source`、"
        "悬空指针停产、买不起就不下水。夹具 `attach_blueprint`/`spawn_at`/`stock` 在 `super`。",
        [
            "spawn_uses_the_yard_blueprint",
            "editing_a_blueprint_does_not_touch_existing_ships",
            "the_yard_launches_from_any_designs_components",
            "auto_blueprint_uses_choose_loadout_at_launch",
            "blueprint_default_order_governs_new_ships",
            "fleet_default_still_covers_blueprintless_ships",
            "order_source_separates_a_missing_leaf_from_a_silent_one",
            "a_dangling_blueprint_pointer_stops_the_yard",
            "a_player_blueprint_that_cannot_be_afforded_waits_for_money",
            "ship_spawned_event_carries_the_blueprint_only_when_there_is_one",
        ],
    ),
    "haul.rs": (
        "集货运输：产地货栈、货舱容量、`haul_split` 的 max-min 公平、货权守恒、"
        "承包分账、腿别交替、指令路线入首都池。",
        [
            "off_capital_production_lands_in_the_depot_not_the_pool",
            "cargo_capacity_is_class_capacity_times_hull_fraction",
            "haul_split_is_max_min_fair",
            "hauling_moves_cargo_without_creating_or_destroying_any",
            "a_hired_delivery_splits_the_cargo_between_carrier_and_shipper",
            "a_haul_route_alternates_legs_because_of_the_cargo",
            "a_commanded_haul_route_delivers_depot_cargo_into_the_capital_pool",
        ],
    ),
    "combat.rs": (
        "战斗与损伤：本土防御光环、面板随选装变化、点防按舰级缩放、"
        "逐组件损伤与友方领土修理、舰队防空、护盾与速度规避。",
        [
            "damaged_ship_regenerates_hull_each_round",
            "home_field_weakens_attackers_near_the_capital",
            "ship_panel_reflects_fitted_components",
            "ship_panel_scales_intercept_by_class_pd_mult",
            "fire_degrades_components_under_damage",
            "damaged_components_repair_in_friendly_territory",
            "fleet_air_defense_covers_nearby_missile_targets",
            "combat_respects_shields_and_speed_evasion",
        ],
    ),
    "mond.rs": (
        "MOND 异常导航：非 master 命不中深处目标（但**没有进不去的目标**）、"
        "`route_depth` 度量、以及那条只打印的探针。",
        [
            "mond_drift_misses_in_anomaly_but_masters_are_exact",
            "mond_depth_only_costs_attempts_never_makes_it_impossible",
            "probe_mond_attempts",
            "route_depth_measures_mond_immersion",
        ],
    ),
    "ideology.rs": (
        "思潮与忠诚：军事信号（只由事件推出）、战争/经济如何推思潮、"
        "相似度性质与外交亲和方向、低忠诚倒戈、娱乐设施拉住远城。",
        [
            "military_signal_uses_the_milestones_and_is_branch_agnostic",
            "entertainment_holds_a_distant_city",
            "low_loyalty_city_defects_to_most_opposing_ideology_instead_of_razing",
            "ideology_military_win_drives_toward_militarism",
            "ideology_economy_bad_drives_toward_populism_and_stays_bounded",
            "ideology_similarity_ranges_and_is_monotonic",
            "ideology_similarity_shifts_diplomatic_affinity_directionally",
        ],
    ),
    "capital.rs": (
        "首都：亡城强迁到人口最高活城、AI 周期性评估迁都、玩家钉的首都 AI 不许覆盖。",
        [
            "capital_destroyed_auto_relocates_to_highest_population_city",
            "ai_periodic_review_relocates_capital_to_population_center",
            "player_capital_not_overridden_by_ai_review",
        ],
    ),
    "story.rs": (
        "剧情：机械后果真的落到状态上、`grant_ship` 真的多出一艘舰队成员。",
        [
            "story_effects_apply",
            "story_grant_ship_spawns_a_fleet_member",
        ],
    ),
    "fleet.rs": (
        "舰队行为与默认：舰队默认管新舰、殖民归属、陈旧 follow 退化成 idle、"
        "跟随友舰时自动开火只打敌对者、回合事件日志、dock/idle 的位姿。",
        [
            "fleet_default_governs_newly_built_ships",
            "colonize_keeps_player_ownership",
            "player_stale_follow_degrades_to_idle_and_does_not_drift",
            "follow_ship_auto_attacks_hostile_but_not_the_followed_friend",
            "advance_populates_round_events",
            "dock_follows_body_and_idle_holds_position",
        ],
    ),
}

CTRL = {
    "normalize.rs": (
        "行为叶的规范化：tagged 形式被接受并改写、非法 tag 报出合法清单、"
        "默认形式原样保留、`apply_patch` 走同一条规范化。",
        [
            "normalize_behavior_accepts_tagged_form",
            "unknown_behavior_tag_is_rejected_with_the_legal_tags_and_a_hint",
            "normalize_behavior_keeps_default_form",
            "apply_patch_accepts_tagged_ship_order",
        ],
    ),
    "view.rs": (
        "读面：控制模板整面回传（含「不四舍五入」）、`ship_orders` 每舰一行且是不动点、"
        "`behavior` 为 null 的行不建叶。",
        [
            "the_control_template_round_trips_back_through_apply",
            "the_order_read_face_lists_every_ship_and_is_a_fixed_point",
            "a_null_behavior_row_never_invents_a_leaf",
            "the_control_template_never_rounds_a_leaf_value",
        ],
    ),
    "ship.rs": (
        "逐舰叶与舰队默认叶：doctrine 补丁、舰队默认覆盖无叶的舰、"
        "角色轴的删叶规则、两轴默认叶必须一起建、单轴叶用「在用的值」补另一轴。",
        [
            "apply_ship_doctrine_patch",
            "fleet_default_style_covers_ships_without_leaves",
            "fleet_default_order_covers_new_ships",
            "the_role_axis_obeys_the_same_delete_rules",
            "a_two_axis_fleet_default_must_be_created_with_both_axes",
            "a_single_axis_ship_leaf_seeds_the_other_axis_from_what_is_in_use",
        ],
    ),
    "apply.rs": (
        "写面与报告：写值即接管、scope 压过独立势力、旧拼写仍可载入、"
        "报告要 `is_clean` 且逐叶计数、消失的舰/别人的舰/拼错的势力或字段都要**响亮报错**"
        "而不是静默丢、删叶把值还给来源。",
        [
            "apply_loyalty_budget_patch",
            "writing_a_value_without_mode_takes_over",
            "scope_player_takes_over_independent_faction",
            "legacy_mode_spellings_still_load",
            "a_valid_diff_reports_clean_and_counts_every_leaf",
            "a_vanished_ship_is_reported_not_dropped_silently",
            "another_factions_ship_is_reported_with_its_own_code",
            "a_typo_faction_id_does_not_invent_a_phantom_faction",
            "a_building_index_from_another_city_is_reported_with_the_real_indices",
            "a_misspelled_control_field_is_rejected_rather_than_ignored",
            "removing_a_leaf_returns_the_value_to_its_source",
            "removing_works_for_fleet_defaults_stale_ships_and_budgets",
        ],
    ),
}


def find_block(lines, name):
    start = None
    for i, l in enumerate(lines):
        m = FN.match(l)
        if m and m.group(1) == name:
            start = i
            break
    if start is None:
        return None
    top = start
    while top > 0 and ATTR_OR_COMMENT.match(lines[top - 1]):
        top -= 1
    close = next((k for k in range(start, len(lines)) if lines[k] == "}"), None)
    return (top, close) if close is not None else None


def split(parent_rel: str, plan: dict, mod_path: str) -> int:
    parent = ROOT / parent_rel
    lines = parent.read_text(encoding="utf-8").split("\n")
    holes = []
    for module, (doc, names) in plan.items():
        blocks = []
        for n in names:
            got = find_block(lines, n)
            if got is None:
                print(f"  !! 找不到 {n}（{parent_rel}）")
                return 1
            top, close = got
            assert "#[test]" in "\n".join(lines[top: close + 1]), f"{n} 那块没有 #[test]"
            blocks.append((top, close, "\n".join(lines[top: close + 1])))
            holes.append((top, close))
        dest = ROOT / mod_path / module
        dest.parent.mkdir(parents=True, exist_ok=True)
        body = "\n\n".join(b for _, _, b in sorted(blocks))
        dest.write_text(f"//! {doc}\n\nuse super::*;\n\n{body}\n", encoding="utf-8")
        print(f"  {module}: {len(names)} 条 → {len(body.splitlines())} 行")

    for top, close in sorted(holes, reverse=True):
        end = close + 1
        while end < len(lines) and lines[end].strip() == "":
            end += 1
        del lines[top:end]

    last_use = max(i for i, l in enumerate(lines) if l.startswith("use "))
    decls = [f"mod {m[:-3]};" for m in sorted(plan)]
    lines[last_use + 1: last_use + 1] = [""] + decls
    parent.write_text("\n".join(lines), encoding="utf-8")
    print(f"  {parent_rel} 剩 {len(lines)} 行")
    return 0


def main() -> int:
    print("=== src/tests/sim/mod.rs")
    if split("src/tests/sim/mod.rs", SIM, "src/tests/sim"):
        return 1
    print("=== src/tests/control.rs（先改成目录模块）")
    src = ROOT / "src/tests/control.rs"
    dest_dir = ROOT / "src/tests/control"
    dest_dir.mkdir(parents=True, exist_ok=True)
    (dest_dir / "mod.rs").write_text(src.read_text(encoding="utf-8"), encoding="utf-8")
    src.unlink()
    # 改 #[path]
    cm = ROOT / "src/control/mod.rs"
    t = cm.read_text(encoding="utf-8").replace(
        '#[path = "../tests/control.rs"]', '#[path = "../tests/control/mod.rs"]'
    )
    cm.write_text(t, encoding="utf-8")
    if split("src/tests/control/mod.rs", CTRL, "src/tests/control"):
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
