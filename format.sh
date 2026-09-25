#!/usr/bin/env bash
# 格式化 / 机械修 lint。**这不是清理脚本** —— 它不删任何东西，产物回收是
# `px build --gc`（见 `docs/programs.md` 的驱动动词）。
set -euo pipefail

cargo fix --allow-dirty --all-targets
cargo fmt --all
