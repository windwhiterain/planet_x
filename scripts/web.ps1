<#
.SYNOPSIS
  行星X WebUI 的唯一启动入口：先清掉**本 worktree**里还在跑的旧实例，再编译运行。

.DESCRIPTION
  为什么要先杀（两件事，一件都不能省）：

    * 旧实例锁着 `target\debug\planet_x_web.exe`。`cargo run` 是「先 build 再 run」，
      build 换不掉被占用的顶层 exe（只更新了 `deps\` 里的副本），于是「我明明重新
      编译了」跑的还是老货——本仓真发生过：进程起于 10:43，11:14 的重建没能替换顶层
      exe。所以杀必须在 cargo **之前**。
    * 起完把本脚本的 pid 当**租约**交给服务（`PLANET_X_WEB_OWNER_PID`）：本脚本一退出
      （Ctrl+C、关终端、后台 job 被收掉），服务立刻自退。Windows 不会替你收孙进程，
      这条是「忘了关 → 孤儿一直在监听」的正解。

  端口仍是服务自己**自动扫**（`3000` 被占就 `3001`…），多 worktree 同时跑互不干扰：
  这里只动 `target` 落在本目录下的实例，别的 worktree 的实例一个都不碰。

.EXAMPLE
  scripts/web.ps1              # 清本目录旧实例 → cargo run（debug）
  scripts/web.ps1 -Release     # 同上，release 构建
  scripts/web.ps1 -List        # 列出所有 worktree 的 planet_x_web 实例（pid/目录/启动时间）
  scripts/web.ps1 -Stop        # 只停本 worktree 的实例
#>
[CmdletBinding()]
param(
    [switch]$Stop,
    [switch]$List,
    [switch]$Release
)

$ErrorActionPreference = 'Stop'

# cargo 的正常进度（Compiling…）是往 stderr 写的：别让 ErrorActionPreference='Stop'
# 把它当成致命错误。老版本 PowerShell 没这个变量，用 Get-Variable 探一下再设。
if (Get-Variable -Name PSNativeCommandUseErrorActionPreference -ErrorAction SilentlyContinue) {
    Set-Variable -Name PSNativeCommandUseErrorActionPreference -Value $false
}

$root = Split-Path -Parent $PSScriptRoot
$targetRoot = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root 'target' }
$targetRoot = [System.IO.Path]::GetFullPath($targetRoot).TrimEnd('\') + '\'

function Get-WebInstance {
    @(Get-Process -Name planet_x_web -ErrorAction SilentlyContinue | ForEach-Object {
            $path = $null
            try { $path = $_.Path } catch { }
            $started = $null
            try { $started = $_.StartTime } catch { }
            [pscustomobject]@{
                Id        = $_.Id
                Path      = $path
                Started   = $started
                InThisDir = [bool]($path -and $path.StartsWith($targetRoot, [System.StringComparison]::OrdinalIgnoreCase))
            }
        })
}

function Format-Instance($i) {
    $when = if ($i.Started) { $i.Started.ToString('MM-dd HH:mm:ss') } else { '?' }
    $where = if ($i.InThisDir) { '本目录  ' } else { '别的目录' }
    "  pid {0,-7} {1}  {2}  {3}" -f $i.Id, $when, $where, $i.Path
}

# 注意 `@(...)`：PowerShell 5.1 里「只有一个元素」的函数返回会被拆成一个 PSCustomObject，
# 而那个东西**没有 `.Count`**（`.Count` 求值成 $null）——于是 `-not $instances.Count` 永远
# 为真，脚本会一边说「没有实例」一边把真实例喂进 kill 分支。包一层数组是必须的。
$instances = @(Get-WebInstance)

if ($List) {
    if (-not $instances.Count) {
        Write-Host '（没有正在跑的 planet_x_web）'
    }
    else {
        Write-Host "planet_x_web 实例（共 $($instances.Count) 个）："
        $instances | ForEach-Object { Write-Host (Format-Instance $_) }
    }
    # 只是看看就到此为止：`-List` 不该顺手把一个服务起起来（除非同时给了 `-Stop`）。
    if (-not $Stop) { return }
}

$mine = @($instances | Where-Object { $_.InThisDir })
if ($mine.Count) {
    Write-Host "停掉本目录（$root）的旧实例："
    $mine | ForEach-Object { Write-Host (Format-Instance $_) }
    foreach ($p in $mine) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
    # 等它真的走干净：紧接着 cargo 就要替换被它锁住的 exe。
    foreach ($p in $mine) { Wait-Process -Id $p.Id -Timeout 10 -ErrorAction SilentlyContinue }
    Write-Host '（已清干净：旧实例锁住的 exe 现在可以被新构建替换了）'
}

if ($Stop) { return }

# 租约：本脚本 pid 一没，服务立刻自退（这就是「再也不留孤儿」的那把锁）。
$env:PLANET_X_WEB_OWNER_PID = "$PID"
$cargoArgs = @('run', '-p', 'planet_x_web')
if ($Release) { $cargoArgs += '--release' }

Write-Host "启动中（cargo $($cargoArgs -join ' ')；看护 pid $PID，本脚本一退服务就退）…"
& cargo @cargoArgs
exit $LASTEXITCODE
