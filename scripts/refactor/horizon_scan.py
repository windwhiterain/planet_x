"""给每个用例量出它模拟了多少个月：扫 test 体里的 advance 调用与循环上界。

只做分类用（决定它进哪一档），不修改任何文件。
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TEST_ATTR = re.compile(r"^(\s*)#\[(test|ignore)")
FN = re.compile(r"^\s*(?:pub )?fn ([A-Za-z_][A-Za-z0-9_]*)")
LOOP = re.compile(r"for\s+\w+\s+in\s+0\.\.=?(\w+)")
CONST = re.compile(r"^\s*const (\w+)\s*:\s*\w+\s*=\s*(\d+)")
ADVANCE = re.compile(r"\badvance\s*\(|\badvance_once\s*\(|\.advance\(")


def consts(text: str) -> dict[str, int]:
    return {m.group(1): int(m.group(2)) for m in (CONST.match(l) for l in text.split("\n")) if m}


def blocks(lines: list[str]):
    """产出 (attr起, 名字, 体文本)。"""
    idx = [i for i, l in enumerate(lines) if TEST_ATTR.match(l)]
    for k, i in enumerate(idx):
        end = idx[k + 1] if k + 1 < len(idx) else len(lines)
        # 名字：从 attr 往后第一个 fn
        name = None
        for j in range(i, min(end, i + 8)):
            m = FN.match(lines[j])
            if m:
                name = m.group(1)
                break
        if name:
            yield i, name, "\n".join(lines[i:end])


def main() -> int:
    targets = sys.argv[1:] or [
        "src/sim/mod.rs",
        "src/control/mod.rs",
        "src/projection.rs",
        "src/config.rs",
        "src/agent.rs",
        "src/json.rs",
        "src/prng.rs",
        "src/model/state.rs",
        "src/model/contract.rs",
        "src/model/market.rs",
        "src/autocontrol/freight.rs",
        "src/autocontrol/contract.rs",
        "src/autocontrol/shipbuilding.rs",
        "src/autocontrol/tactics.rs",
        "src/autocontrol/blueprints.rs",
        "src/autocontrol/style.rs",
        "src/autocontrol/economy.rs",
        "web/src/lib.rs",
        "tests/longhorizon.rs",
        "tests/trade_probe.rs",
        "tests/projection_derived.rs",
        "tests/control_read_face.rs",
    ]
    rows = []
    for t in targets:
        p = ROOT / t
        if not p.exists():
            continue
        text = p.read_text(encoding="utf-8")
        lines = text.split("\n")
        cs = consts(text)
        for i, name, body in blocks(lines):
            ignored = "#[ignore" in body
            n_adv = len(ADVANCE.findall(body))
            bounds = []
            for m in LOOP.finditer(body):
                v = m.group(1)
                bounds.append(cs.get(v, int(v) if v.isdigit() else 0))
            months = max(bounds) if bounds else (1 if n_adv else 0)
            # 循环体里有 advance 才算推进月份
            rows.append((months, t, name, n_adv, ignored, len(body.split("\n"))))
    rows.sort(key=lambda r: (-r[0], r[1], r[2]))
    print(f"{'months':>7} {'adv':>4} {'ign':>4} {'lines':>6}  file::test")
    for months, t, name, n_adv, ignored, ln in rows:
        print(f"{months:>7} {n_adv:>4} {'Y' if ignored else '':>4} {ln:>6}  {t}::{name}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
