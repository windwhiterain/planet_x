"""数据级测试的入口：**先 build release，再按组跑**（不用测试框架）。

    python play/tests/run.py                 # 快组（默认，内循环）
    python play/tests/run.py 1 3             # 只跑第 1、3 组
    python play/tests/run.py all -j 7        # 全组、7 路并行
    python play/tests/run.py all --refresh   # 无视缓存重跑投影（改过 config 之外的引擎行为时）
    python play/tests/run.py --list

**流程就是「build release + python 测试」**（用户裁决）：入口默认先看一眼 release 二进制是不是
比源码旧（或不存在），是就先 `cargo build --release`——你改完 Rust 直接跑这条命令即可，
不用记得先编译。已经是最新就跳过（`--no-build` 可以强制跳过）。

每个组是一个独立脚本，可以单跑（`python play/tests/g3_long.py`）；`run.py` 只是把几组串起来
并汇总退出码（**任何一组红 ⇒ 退出码 1**）。

⚠ 用带 pandas 的解释器跑：
`uv run --project play/planet_xq python play/tests/run.py all`（`uv run` 会管好 venv）。
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
# 判「二进制比源码旧」要看的东西：引擎源码 + 两处配置 + 清单。
# （`config/game.ron` 与 `Cargo.toml` 都算：改它们等于改了引擎的行为。）
STALENESS_SOURCES = ("src", "Cargo.toml", "config")

GROUPS: list[tuple[str, str, str]] = [
    ("1", "g1_contract.py", "读面契约（≤48 回合）：确定性、pre/post 同源、没有非有限的数"),
    ("2", "g2_mid.py", "中组（49–480 回合）：机制不变量（同回合复垦、选装、编年史）"),
    ("3", "g3_long.py", "长组（1000 回合 × 7 seed）：世界健康 + 政治机制"),
    ("4", "g4_spec.py", "声明纪律（40 回合）：views.json 静态 + 写面/读面对账 + 认领完整性"),
]
# 默认档 = 快组：内循环用这条（与 `cargo nextest run` 同一档语义）。
DEFAULT = ("1", "4")


def newest_source_mtime() -> float:
    newest = 0.0
    for rel in STALENESS_SOURCES:
        p = REPO / rel
        if p.is_file():
            newest = max(newest, p.stat().st_mtime)
            continue
        for f in p.rglob("*"):
            if f.is_file() and f.suffix in (".rs", ".ron", ".toml"):
                newest = max(newest, f.stat().st_mtime)
    return newest


def ensure_binary(which: str, force_skip: bool) -> int:
    """把 `--bin` 指的那个二进制建出来（release / debug）；已经比源码新就跳过。

    返回 0（不用管）/ 1（建好了）/ 2（建失败）。
    """
    if force_skip:
        return 0
    profile = {"release": "release", "debug": "debug"}.get(which)
    if profile is None:                      # 显式给了路径 ⇒ 那是调用者自己负责的
        return 0
    exe = REPO / "target" / profile / "planet_x.exe"
    cfg = REPO / "Cargo.toml"
    if exe.exists() and exe.stat().st_mtime >= newest_source_mtime() and exe.stat().st_mtime >= cfg.stat().st_mtime:
        print(f"[build] {exe.relative_to(REPO)} 比源码新，跳过编译", flush=True)
        return 0
    why = "不存在" if not exe.exists() else "比源码旧"
    print(f"[build] {exe.relative_to(REPO)} {why} ⇒ cargo build --release"
          + ("" if profile == "release" else "（注：--bin debug 走 dev 档）"), flush=True)
    t0 = time.time()
    cmd = ["cargo", "build"] + (["--release"] if profile == "release" else [])
    rc = subprocess.run(cmd, cwd=str(REPO)).returncode
    if rc != 0:
        print(f"[build] 编译失败（退出码 {rc}）", file=sys.stderr)
        return 2
    print(f"[build] 完成，{time.time() - t0:.1f} s", flush=True)
    return 1


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="run.py", description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("groups", nargs="*", help="组号（1/2/3/4）或 all；缺省 = 快组")
    ap.add_argument("--list", action="store_true", help="列出组")
    ap.add_argument("--bin", default="release", help="release | debug | 二进制路径（透传给每个组）")
    ap.add_argument("--refresh", action="store_true", help="无视缓存重跑投影")
    ap.add_argument("--no-build", action="store_true", help="跳过前置编译（默认会按需 build）")
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

    if ensure_binary(args.bin, args.no_build) == 2:
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
