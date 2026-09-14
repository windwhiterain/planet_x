<#
.SYNOPSIS
  Planet X 的 PCG / 渲染探针入口。

.DESCRIPTION
  探针不是测试：它们要 GPU、要几分钟，所以从 cargo test 里搬出来了
  （见 .agents/notes/art/08-instruments.md §47）。**退出码才是判据**。

  「优化程度」由 -Level 选：
    dev      默认。编译最快（bevy 保持 -O0），跑得最慢。
    opt      只把本地 crate 提到 O2 —— 不给 bevy 换 profile，不触发全量重编。
    release  全量 O3：跑得最快，第一次编译最贵。

.EXAMPLE
  .\tools\px.ps1 -Target field_dual -Level opt   # §46.3 的 arbiter
  .\tools\px.ps1 -Target gradient                # 探针冒烟 + 逐通道归因
  .\tools\px.ps1 -Target planet -Level opt       # 烘星球图（PCG）
  .\tools\px.ps1 -Target test                    # 快速测试链（默认 members，不碰 bevy）
  .\tools\px.ps1 -Target test-all                # 全量（含 px_render / px_probe，慢）
#>
param(
    [ValidateSet(
        'field_dual', 'gradient', 'device',
        'planet', 'desert', 'clouds',
        'test', 'test-all', 'check'
    )][string]$Target = 'field_dual',

    [ValidateSet('dev', 'opt', 'release')][string]$Level = 'dev',

    [Parameter(ValueFromRemainingArguments = $true)][string[]]$Rest = @()
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    $packages = @('px_ops', 'px_verify', 'px_graphs')
    if ($Target -in 'field_dual', 'gradient', 'device') { $packages += 'px_probe' }

    $extra = @()
    switch ($Level) {
        'dev' { }
        'opt' {
            # 逐包覆盖 profile.dev，而不是新建一个 profile：
            # 新 profile 会让 cargo 把 552 个依赖全部重编一遍（§47.4）。
            foreach ($package in $packages) {
                $extra += @('--config', "profile.dev.package.$package.opt-level=2")
            }
        }
        'release' { $extra += '--release' }
    }

    $cargoArgs = switch ($Target) {
        { $_ -in 'field_dual', 'gradient', 'device' } { @('run', '-p', 'px_probe', '--bin', $Target) + $extra }
        { $_ -in 'planet', 'desert', 'clouds' } { @('run', '-p', 'px_graphs', '--bin', $Target) + $extra }
        'test' { @('test') + $extra }
        'test-all' { @('test', '--workspace') + $extra }
        'check' { @('check', '-p', 'px_ops', '-p', 'px_verify', '-p', 'px_protocol') + $extra }
    }

    Write-Host ("cargo " + ($cargoArgs -join ' ')) -ForegroundColor DarkGray
    & cargo @cargoArgs @Rest
    exit $LASTEXITCODE
}
finally {
    Pop-Location
}
