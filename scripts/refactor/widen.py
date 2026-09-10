import re
from pathlib import Path

root = Path("src/sim")
pat = re.compile(r"^(pub\(crate\) |pub\(super\) )?(fn|struct|enum|const|static|type) ")
n = 0
for f in sorted(root.glob("*.rs")):
    if f.name == "mod.rs":
        continue
    lines = f.read_text(encoding="utf-8").split("\n")
    for i, l in enumerate(lines):
        m = pat.match(l)
        if not m:
            continue
        # 「fn/struct/…」这个词本身的位置：有前缀就跳到前缀之后，没有就从头开始。
        kw_at = m.end(1) if m.group(1) else 0
        lines[i] = "pub " + l[kw_at:]
        n += 1
    f.write_text("\n".join(lines), encoding="utf-8")
print(f"widened {n} items to pub")
