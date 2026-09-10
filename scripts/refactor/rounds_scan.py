"""把各测试模块里「跑多少回合」的证据挖出来（write_index/--round/advance_for 等）。"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FILES = [
    "src/projection.rs", "src/agent.rs", "src/model/state.rs", "web/src/lib.rs",
    "tests/projection_derived.rs", "tests/control_read_face.rs", "tests/longhorizon.rs",
    "tests/trade_probe.rs", "src/autocontrol/freight.rs", "src/autocontrol/tactics.rs",
    "src/autocontrol/blueprints.rs", "src/autocontrol/style.rs", "src/autocontrol/contract.rs",
    "src/autocontrol/shipbuilding.rs", "src/model/contract.rs", "src/json.rs",
    "src/model/market.rs", "src/autocontrol/economy.rs", "src/prng.rs", "src/config.rs",
    "src/sim/mod.rs",
]
NUMCALL = re.compile(
    r"(\w+)\([^()]*?\b(\d{2,5})(?:u32|u64)?\b[^()]*?\)"
)
KEYS = ("write_index", "write_projections", "run", "advance_for", "advance_n", "for_rounds",
        "rounds", "run_rounds", "to_round", "simulate", "play", "write_out")
TEST_ATTR = re.compile(r"^\s*#\[test\]")
FN = re.compile(r"^\s*(?:pub )?fn ([A-Za-z_][A-Za-z0-9_]*)")


def main() -> int:
    for f in FILES:
        p = ROOT / f
        if not p.exists():
            continue
        lines = p.read_text(encoding="utf-8").split("\n")
        idx = [i for i, l in enumerate(lines) if TEST_ATTR.match(l)]
        if not idx:
            continue
        cur = None
        out = []
        for k, i in enumerate(idx):
            end = idx[k + 1] if k + 1 < len(idx) else len(lines)
            name = next((FN.match(lines[j]).group(1) for j in range(i, min(end, i + 10))
                         if FN.match(lines[j])), "?")
            hits = []
            for j in range(i, end):
                l = lines[j]
                for m in NUMCALL.finditer(l):
                    if m.group(1) in KEYS:
                        hits.append(f"{m.group(1)}({m.group(2)})")
                m2 = re.search(r'"--round"\s*,\s*"(\d+)"', l)
                if m2:
                    hits.append(f"--round {m2.group(1)}")
                m3 = re.match(r"\s*const \w+[^=]*=\s*(\d+)", l)
                if m3 and any(w in l.lower() for w in ("round", "horizon", "month")):
                    hits.append(f"const {m3.group(1)}")
            if hits:
                out.append((name, hits))
        if out:
            print(f"--- {f}")
            for name, hits in out:
                uniq = sorted(set(hits))
                print(f"    {name}: {', '.join(uniq)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
