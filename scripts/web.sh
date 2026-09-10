#!/usr/bin/env bash
# 行星X WebUI 在 git_bash 下的唯一入口：转调 scripts/web.ps1。
#
# 真正的逻辑（先杀本 worktree 的旧实例 → cargo run → 挂启动者租约）都在 .ps1 里，
# 这里只做「找到 PowerShell + 把路径转成 Windows 形式」这两件事，免得两处维护同一
# 套进程清理逻辑（那份逻辑一旦分叉，就又会有人起出孤儿进程）。
set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

ps="$(command -v pwsh || command -v powershell.exe || true)"
# 本机可能只有 Windows PowerShell 5.1（`pwsh` 7 常常没装）——那就直接点名它的位置，
# 别让「找不到 pwsh」变成一个假的启动失败。
if [ -z "$ps" ] && [ -x "/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe" ]; then
    ps="/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe"
fi
if [ -z "$ps" ]; then
    echo "scripts/web.sh: 找不到 pwsh / powershell.exe（本脚本只是 web.ps1 的壳）" >&2
    exit 1
fi

# PowerShell 只认 Windows 路径：git_bash 下要先 cygpath。
script="$here/web.ps1"
if command -v cygpath >/dev/null 2>&1; then
    script="$(cygpath -w "$script")"
fi

exec "$ps" -NoProfile -ExecutionPolicy Bypass -File "$script" "$@"
