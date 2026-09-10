"""给每个用例量出它模拟了多少个月（第二版）：解出循环上界的局部绑定，并打印证据行。

只做分类用（决定进哪一档），不修改任何文件。
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TEST_ATTR = re.compile(r"^(\s*)#\[(test|ignore)")
FN = re.compile(r"^\s*(?:pub )?fn ([A-Za-z_][A-Za-z0-9_]*)")
LOOP = re.compile(r"for\s+\w+\s+in\s+0\.\.=?([A-Za-z_][A-Za-z0-9_]*|\d+)")
LET = re.compile(r"let\s+(?:mut\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*(?::[^=]+)?=\s*([^;]+);")
CONST = re.compile(r"^\s*const (\w+)\s*:[^=]+=\s*(\d+)")
ADVANCE = re.compile(r"\badvance\s*\(|\badvance_once\s*\(")
RUNDS = re.compile(r"\b(\w*rounds?\w*|\w*horizon\w*|\w*months?\w*)\s*\(")


def blocks(lines: list[str]):
    idx = [i for i, l in enumerate(lines) if TEST_ATTR.match(l)]
    for k, i in enumerate(idx):
        end = idx[k + 1] if k + 1 < len(idx) else len(lines)
        name = None
        for j in range(i, min(end, i + 8)):
            m = FN.match(lines[j])
            if m:
                name = m.group(1)
                break
        if name:
            yield i, name, "\n".join(lines[i:end])


def resolve(expr: str, env: dict[str, str], depth: int = 0) -> str:
    expr = expr.strip()
    if depth > 4:
        return expr
    if expr.isdigit():
        return expr
    if expr in env:
        return f"{expr}={resolve(env[expr], env, depth + 1)}"
    m = re.search(r"(\d+)", expr)
    if m and ("PROBE" in expr or "unwrap_or" in expr or "var" in expr):
        return f"{expr}(默认{m.group(1)})"
    return expr


def main() -> int:
    targets = sys.argv[1:]
    rows = []
    for t in targets:
        p = ROOT / t
        if not p.exists():
            continue
        lines = p.read_text(encoding="utf-8").split("\n")
        cs = {m.group(1): m.group(2) for m in (CONST.match(l) for l in lines) if m}
        for _, name, body in blocks(lines):
            ignored = "#[ignore" in body
            n_adv = len(ADVANCE.findall(body))
            env = dict(cs)
            for m in LET.finditer(body):
                env.setdefault(m.group(1), m.group(2).strip())
            bounds = []
            for m in LOOP.finditer(body):
                v = m.group(1)
                bounds.append((v, resolve(v, env)))
            rows.append((n_adv, name, ignored, bounds, t))
    rows.sort(key=lambda r: (-r[0], r[4], r[1]))
    for n_adv, name, ignored, bounds, t in rows:
        if n_adv == 0 and not bounds:
            continue
        tag = "IGN" if ignored else "   "
        b = "; ".join(f"{v}->{r}" for v, r in bounds[:4])
        print(f"adv={n_adv:<2} {tag} {t}::{name}\n           loops: {b}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
