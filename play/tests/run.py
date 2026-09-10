"""数据级测试的入口：**按组跑**（不用测试框架）。

    python play/tests/run.py                 # 快组（默认，内循环）
    python play/tests/run.py 1 3             # 只跑第 1、3 组
    python play/tests/run.py all -j 7        # 全组、7 路并行
    python play/tests/run.py all --refresh   # 无视缓存重跑投影（改过 config 之外的引擎行为时）
    python play/tests/run.py --list

每个组是一个独立脚本，可以单跑（`python play/tests/g3_long.py`）；`run.py` 只是把几组串起来
并汇总退出码（**任何一组红 ⇒ 退出码 1**）。

⚠ 用带 pandas 的解释器跑（kit 那个 venv 就是）：

    play/planet_xq/.venv/Scripts/python.exe play/tests/run.py all
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent

GROUPS: list[tuple[str, str, str]] = [
    ("1", "g1_contract.py", "读面契约（≤48 回合）：确定性、pre/post 同源、没有非有限的数"),
    ("2", "g2_mid.py", "中组（49–480 回合）：机制不变量（同回合复垦、选装、编年史）"),
    ("3", "g3_long.py", "长组（1000 回合 × 7 seed）：世界健康 + 政治机制"),
]
# 默认档 = 快组：内循环用这条（与 `cargo nextest run` 同一档语义）。
DEFAULT = ("1",)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="run.py", description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("groups", nargs="*", help="组号（1/2/3）或 all；缺省 = 快组")
    ap.add_argument("--list", action="store_true", help="列出组")
    ap.add_argument("--bin", default="release", help="release | debug | 二进制路径（透传给每个组）")
    ap.add_argument("--refresh", action="store_true", help="无视缓存重跑投影")
    ap.add_argument("-j", "--jobs", type=int, default=0, help="并行跑几个世界（透传）")
    args = ap.parse_args(argv)

    if args.list:
        for gid, script, desc in GROUPS:
            print(f"  {gid}  {script:16s} {desc}")
        return 0

    wanted = args.groups or list(DEFAULT)
    if "all" in wanted:
        wanted = [g for g, _, _ in GROUPS]
    known = {g: (script, desc) for g, script, desc in GROUPS}
    unknown = [g for g in wanted if g not in known]
    if unknown:
        print(f"没有这些组：{unknown}（--list 看有哪些）", file=sys.stderr)
        return 2

    passthrough: list[str] = ["--bin", args.bin]
    if args.refresh:
        passthrough.append("--refresh")
    if args.jobs:
        passthrough += ["-j", str(args.jobs)]

    results: list[tuple[str, int, float]] = []
    for gid in wanted:
        script = HERE / known[gid][0]
        print(f"\n=== 组 {gid}：{known[gid][1]} ===", flush=True)
        t0 = time.time()
        rc = subprocess.run([sys.executable, str(script), *passthrough],
                            cwd=str(HERE.parents[1])).returncode
        results.append((gid, rc, time.time() - t0))

    print("\n=== 汇总 ===")
    for gid, rc, dt in results:
        print(f"  组 {gid}  {'绿' if rc == 0 else '红'}  {dt:5.1f} s  {known[gid][0]}")
    bad = [g for g, rc, _ in results if rc != 0]
    if bad:
        print(f"  **{len(bad)} 组红**：{bad}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
