#!/usr/bin/env bash
# 格式化 / 机械修 lint。**这不是清理脚本** —— 它不删任何东西，产物回收是
# `px build --gc`（见 `docs/system/build-graph.md` 的那三个动词）。
#
# 2026-09-28 改名（原名 `clean.sh`）：名字与行为不符是那种会让人敲错命令的缺陷。
set -euo pipefail

cargo fix --allow-dirty --all-targets
cargo fmt --all
