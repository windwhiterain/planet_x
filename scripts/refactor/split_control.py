"""把 src/control.rs 按主题切成 src/control/ 子模块（与 split_sim.py 同一套安全网：
拆出来的块拼回去必须与原文逐字节相同，否则不写任何文件）。"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "src" / "control.rs"

ASSIGNMENT = [
    ("is_false", "wire"),
    ("double_option", "wire"),
    ("ShipOrderEntry", "wire"),
    ("ShipDoctrineEntry", "wire"),
    ("ShipKitingEntry", "wire"),
    ("ShipFreighterEntry", "wire"),
    ("BudgetEntry", "wire"),
    ("InvestWeightEntry", "wire"),
    ("BuildWeightEntry", "wire"),
    ("LoyaltyBudgetEntry", "wire"),
    ("BlueprintEntry", "wire"),
    ("FactionControlView", "wire"),
    ("ControlSurface", "wire"),
    ("DefaultShipOrder", "wire"),
    ("DefaultDoctrine", "wire"),
    ("DefaultKiting", "wire"),
    ("DefaultFreighter", "wire"),
    ("ShipOrderPatch", "wire"),
    ("ShipDoctrinePatch", "wire"),
    ("ShipKitingPatch", "wire"),
    ("ShipFreighterPatch", "wire"),
    ("BudgetPatch", "wire"),
    ("InvestWeightPatch", "wire"),
    ("BuildWeightPatch", "wire"),
    ("LoyaltyBudgetPatch", "wire"),
    ("BlueprintPatch", "wire"),
    ("referencing_yards", "blueprint"),
    ("yard_ship_type_intent", "blueprint"),
    ("apply_blueprint", "blueprint"),
    ("BuildingPatch", "wire"),
    ("CapitalPatch", "wire"),
    ("FactionControlPatch", "wire"),
    ("CommandReq", "wire"),
    ("SkippedLeaf", "wire"),
    ("ApplyReport", "wire"),
    ("ApplyReport", "wire"),
    ("control_view", "view"),
    ("scope_view", "view"),
    ("explicit", "view"),
    ("control_surface", "view"),
    ("control_schema_value", "view"),
    ("remove_conflicts", "apply"),
    ("leaf_removed", "apply"),
    ("apply_default_ship_order", "ship"),
    ("apply_default_doctrine", "ship"),
    ("apply_default_kiting", "ship"),
    ("apply_default_freighter", "ship"),
    ("apply_ship_order", "ship"),
    ("apply_ship_doctrine", "ship"),
    ("apply_ship_kiting", "ship"),
    ("apply_ship_freighter", "ship"),
    ("apply_budget", "budget"),
    ("BudgetKind", "budget"),
    ("BudgetKind", "budget"),
    ("apply_weight", "budget"),
    ("WeightKind", "budget"),
    ("WeightKind", "budget"),
    ("apply_loyalty_budget", "budget"),
    ("resolve_own_ship", "apply"),
    ("write_mode_leaf", "apply"),
    ("write_value_leaf", "apply"),
    ("check_city_building", "building"),
    ("apply_diff", "apply"),
    ("apply_building_patch", "building"),
    ("BEHAVIOR_TAGS", "normalize"),
    ("normalize_behavior", "normalize"),
    ("normalize_control_diffs", "normalize"),
    ("apply_patch", "apply"),
]

DOC = {
    "wire": "控制的**线上形状**：`--apply` / `POST /api/command` 的 patch 类型与读面视图"
            "（Entry / Patch / View / Report）——读面即写面，两侧共用同一批结构。",
    "view": "读面：`control_view` / `scope_view` / `control_surface` / `control_schema_value`。",
    "apply": "写面入口：`apply_diff` / `apply_patch`，以及叶子写入原语（模式叶 / 值叶）。",
    "ship": "逐舰叶片 + 舰队默认叶（order / doctrine / kiting / freighter）的写入。",
    "budget": "预算与权重叶（budget / invest / build / loyalty）的写入。",
    "building": "建筑叶片（含城市建筑校验）的写入。",
    "blueprint": "设计图叶片：引用它的船坞、意图轴、`apply_blueprint`。",
    "normalize": "行为叶的规范化（tagged 形式 → 枚举形式）与整份 diff 的预规范化。",
}

ITEMS = re.compile(
    r"^(?:(?:pub(?:\(crate\)|\(super\))?) )?"
    r"(fn|struct|enum|impl|const|static|type) ([A-Za-z_][A-Za-z0-9_]*)"
)
DOC_LINE = re.compile(r"^\s*(///|//|#\[)")


def main() -> int:
    lines = SRC.read_text(encoding="utf-8").split("\n")
    cfg_test = next(i for i, l in enumerate(lines) if l.startswith("#[cfg(test)]"))
    code_end = cfg_test

    found = []
    for i in range(code_end):
        m = ITEMS.match(lines[i])
        if m:
            found.append((i, m.group(1), m.group(2)))

    if [n for _, _, n in found] != [n for n, _ in ASSIGNMENT]:
        got = [(k, n) for _, k, n in found]
        for i, (a, b) in enumerate(zip(got, ASSIGNMENT)):
            if a != b:
                print(f"  第一处分歧 @#{i}: 解析={a} 预期={b}")
                break
        print(f" 解析 {len(got)} 项 / 预期 {len(ASSIGNMENT)} 项")
        return 1

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

    rebuilt = "\n".join("\n".join(lines[s:e]) for s, e in zip(starts, ends))
    if rebuilt != "\n".join(lines[starts[0]:code_end]):
        print("!! 拼接结果与原文不一致 —— 边界错位，未写任何文件")
        return 1
    print("自校验通过：块拼接 == 原文代码区（逐字节）")

    subs = [m for m in order if m != "mod"]
    (ROOT / "src" / "control").mkdir(parents=True, exist_ok=True)
    for module in subs:
        (ROOT / "src" / "control" / f"{module}.rs").write_text(
            f"//! {DOC[module]}\n\nuse super::*;\n\n" + "\n".join(blocks[module]),
            encoding="utf-8",
        )

    decls = "\n".join(f"pub mod {m};" for m in sorted(subs))
    uses = "\n".join(f"pub use {m}::*;" for m in sorted(subs))
    tail = "\n".join(lines[cfg_test:])
    mod_rs = f"{header}\n\n{decls}\n\n{uses}\n\n{tail}"
    (ROOT / "src" / "control" / "mod.rs").write_text(mod_rs, encoding="utf-8")
    SRC.unlink()

    print(f"mod.rs = {len(mod_rs.split(chr(10)))} 行（含测试）")
    for m in sorted(subs):
        n = len((ROOT / "src" / "control" / f"{m}.rs").read_text(encoding="utf-8").split("\n"))
        print(f"  {m}.rs = {n} 行")
    return 0


if __name__ == "__main__":
    sys.exit(main())
