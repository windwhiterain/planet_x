"""按「模拟时间」把高档用例挪进 horizon_mid / horizon_long 模块。

判据 = 该用例真正推进的回合数：
    T1 短档 ≤ 48 回合（4 年内）—— 默认跑
    T2 中档 49–480 回合          —— `-P mid`
    T3 长档 > 480 回合           —— `-P full`
移动只搬整块（从它的文档注释/属性行到函数体收尾的 `}`），并保持列 0 缩进。
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FN = re.compile(r"^fn ([A-Za-z_][A-Za-z0-9_]*)\(")
ATTR_OR_COMMENT = re.compile(r"^\s*(///|//|#\[)")


def find_block(lines: list[str], name: str):
    start = None
    for i, l in enumerate(lines):
        m = FN.match(l)
        if m and m.group(1) == name:
            start = i
            break
    if start is None:
        return None
    # 往上吃掉属性与文档注释
    top = start
    while top > 0 and ATTR_OR_COMMENT.match(lines[top - 1]):
        top -= 1
    # 往下找列 0 的闭括号
    close = None
    for k in range(start, len(lines)):
        if lines[k] == "}":
            close = k
            break
    if close is None:
        return None
    return top, close


def move(src_rel: str, names: list[str], dest_rel: str, doc: str, mod_decl: str) -> int:
    src = ROOT / src_rel
    lines = src.read_text(encoding="utf-8").split("\n")
    moved: list[str] = []
    holes: list[tuple[int, int]] = []
    for n in names:
        got = find_block(lines, n)
        if got is None:
            print(f"  !! 找不到 {n}")
            return 1
        top, close = got
        if not any("#[test]" in l for l in lines[top: close + 1]):
            print(f"  !! {n} 那块里没有 #[test]，怕是抓错了：{src_rel}:{top + 1}")
            return 1
        moved.append("\n".join(lines[top: close + 1]))
        holes.append((top, close))
        print(f"  {src_rel}:{top + 1}-{close + 1}  {n}")

    # 先摘掉（从后往前，免得行号错位），顺带吃掉紧随其后的空行
    for top, close in sorted(holes, reverse=True):
        end = close + 1
        while end < len(lines) and lines[end].strip() == "":
            end += 1
        del lines[top:end]

    # 插入 mod 声明（放在最后一条顶层 use 之后）
    last_use = max(i for i, l in enumerate(lines) if l.startswith("use "))
    lines.insert(last_use + 1, "")
    lines.insert(last_use + 2, mod_decl)
    src.write_text("\n".join(lines), encoding="utf-8")

    dest = ROOT / dest_rel
    dest.parent.mkdir(parents=True, exist_ok=True)
    body = "\n\n".join(moved)
    dest.write_text(f"//! {doc}\n\nuse super::*;\n\n{body}\n", encoding="utf-8")
    print(f"  → {dest_rel}（{len(body.splitlines())} 行）")
    return 0


def main() -> int:
    rc = move(
        "src/tests/sim/mod.rs",
        [
            "a_city_razed_this_round_is_not_refounded_by_its_own_loser_this_round",
            "long_run_produces_customized_ships",
            "story_chronicle_grows_deterministically",
            "story_participants_are_concrete",
            "war_scar_floor_makes_a_real_floor_on_war_duration",
        ],
        "src/tests/sim/horizon_mid.rs",
        "**中档（T2，49–480 回合）**的 sim 行为用例：400 回合的「同回合复垦」与「出厂风格」、\n"
        "60 回合的编年史/参与者/战痕地板。它们要的模拟时长是判据本身，所以不进快档（默认\n"
        "`cargo nextest run` 不选它们）。",
        "mod horizon_mid;",
    )
    return rc


if __name__ == "__main__":
    sys.exit(main())
