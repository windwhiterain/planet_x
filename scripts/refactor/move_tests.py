"""把各源文件里的内联 `#[cfg(test)] mod tests { ... }` 搬到 src/tests/ 下。

做法（保住私有项可见性，不改任何可见性）：
    src/foo.rs 里原本的
        #[cfg(test)]
        mod tests { ...体... }
    换成
        #[cfg(test)]
        #[path = "tests/foo.rs"]
        mod tests;
    —— 测试模块在**模块树里仍然是 foo 的子模块**（只是物理文件搬到 src/tests/），
    所以 `use super::*;` 与私有项访问全都不变。
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# (源文件, 测试模块的「模块目录」相对 src/ 的路径, 目标文件相对 src/tests/)
# 模块目录 = 决定 #[path] 相对谁解析的那个目录。
TABLE = [
    ("src/sim/mod.rs", "src/sim", "src/tests/sim/mod.rs",
     "sim 的单元测试：不推进回合或 ≤48 回合（短档）。长档见同目录 horizon_mid.rs。"),
    ("src/control/mod.rs", "src/control", "src/tests/control.rs",
     "control 的单元测试（读写面 = 控制树，不推进回合）。"),
    ("src/projection.rs", "src", "src/tests/projection/mod.rs",
     "projection 的单元测试（≤48 回合的读面守卫）。"),
    ("src/config.rs", "src", "src/tests/config.rs", "config 的单元测试（配置装载 / checkpoint 往返）。"),
    ("src/agent.rs", "src", "src/tests/agent.rs", "agent 视图的单元测试。"),
    ("src/json.rs", "src", "src/tests/json.rs", "json 工具的单元测试。"),
    ("src/prng.rs", "src", "src/tests/prng.rs", "PRNG 的单元测试。"),
    ("src/model/state.rs", "src/model", "src/tests/model/state.rs", "State 的单元测试（checkpoint 迁移）。"),
    ("src/model/contract.rs", "src/model", "src/tests/model/contract.rs", "承包单模型的单元测试。"),
    ("src/model/market.rs", "src/model", "src/tests/model/market.rs", "市场模型的单元测试。"),
    ("src/autocontrol/freight.rs", "src/autocontrol", "src/tests/autocontrol/freight.rs", "集货派单的单元测试。"),
    ("src/autocontrol/contract.rs", "src/autocontrol", "src/tests/autocontrol/contract.rs", "雇佣承包市场的单元测试。"),
    ("src/autocontrol/shipbuilding.rs", "src/autocontrol", "src/tests/autocontrol/shipbuilding.rs", "造舰决策的单元测试。"),
    ("src/autocontrol/tactics.rs", "src/autocontrol", "src/tests/autocontrol/tactics.rs", "战术决策的单元测试。"),
    ("src/autocontrol/blueprints.rs", "src/autocontrol", "src/tests/autocontrol/blueprints.rs", "AI 建图的单元测试。"),
    ("src/autocontrol/style.rs", "src/autocontrol", "src/tests/autocontrol/style.rs", "风格三轴的单元测试。"),
    ("src/autocontrol/economy.rs", "src/autocontrol", "src/tests/autocontrol/economy.rs", "经济决策的单元测试。"),
    ("web/src/lib.rs", "web/src", "web/src/tests.rs", "WebUI 服务端的单元测试（HTTP 面 / 生命周期）。"),
]


def extract(text: str):
    """返回 (开括号行号, 闭括号行号) —— 0 基。"""
    lines = text.split("\n")
    start = None
    for i, l in enumerate(lines):
        if l.strip() == "#[cfg(test)]":
            start = i
            break
    if start is None:
        return None
    # mod tests { 行
    j = start
    while j < len(lines) and not re.match(r"^\s*mod tests\s*\{", lines[j]):
        j += 1
    assert j < len(lines), "找不到 mod tests {"
    depth = 0
    for k in range(j, len(lines)):
        depth += lines[k].count("{") - lines[k].count("}")
        if depth == 0:
            return start, j, k
    raise AssertionError("括号没配平")


def dedent(body: list[str]) -> list[str]:
    out = []
    for l in body:
        if l.startswith("    "):
            out.append(l[4:])
        elif l.strip() == "":
            out.append("")
        else:
            out.append(l)
    return out


def main() -> int:
    only = sys.argv[1:]
    for src_rel, mod_dir, dest_rel, doc in TABLE:
        if only and not any(o in src_rel for o in only):
            continue
        src = ROOT / src_rel
        if not src.exists():
            print(f"跳过（不存在）：{src_rel}")
            continue
        text = src.read_text(encoding="utf-8")
        got = extract(text)
        if got is None:
            print(f"跳过（没有内联测试）：{src_rel}")
            continue
        ctest_line, mod_line, close_line = got
        lines = text.split("\n")
        body = dedent(lines[mod_line + 1: close_line])

        dest = ROOT / dest_rel
        dest.parent.mkdir(parents=True, exist_ok=True)
        header = f"//! {doc}\n\n"
        dest.write_text(header + "\n".join(body).strip("\n") + "\n", encoding="utf-8")

        # 相对路径：从「模块目录」到目标文件
        rel = Path(dest_rel).relative_to(Path(dest_rel).parts[0])  # 去掉 src/ 或 web/
        rel_from_mod = Path(
            __import__("os").path.relpath(ROOT / dest_rel, ROOT / mod_dir)
        ).as_posix()
        include = (
            "#[cfg(test)]\n"
            f'#[path = "{rel_from_mod}"]\n'
            "mod tests;\n"
        )
        new_text = "\n".join(lines[:ctest_line]) + "\n" + include + "\n".join(
            lines[close_line + 1:]
        )
        src.write_text(new_text, encoding="utf-8")
        print(f"{src_rel}: 搬出 {close_line - mod_line - 1} 行 → {dest_rel}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
