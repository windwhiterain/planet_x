<#
.SYNOPSIS
  Planet X 的 PCG / 渲染探针入口。

.DESCRIPTION
  探针不是测试：它们要 GPU、要几分钟，所以从 cargo test 里搬出来了
  （见 .agents/notes/art/09-instruments.md §47）。**退出码才是判据**。

  「优化程度」由 -Level 选：
    dev      默认。编译最快（bevy 保持 -O0），跑得最慢。
    opt      只把本地 crate 提到 O2 —— 不给 bevy 换 profile，不触发全量重编。
    release  全量 O3：跑得最快，第一次编译最贵。

.EXAMPLE
  .\tools\px.ps1 -Target field_dual -Level opt   # §46.3 的 arbiter
  .\tools\px.ps1 -Target gradient                # 探针冒烟 + 逐通道归因
  .\tools\px.ps1 -Target dual_field              # 云场梯度 vs 对偶数（纯 CPU）
  .\tools\px.ps1 -Target planet -Level opt       # 烘星球图（PCG）
  .\tools\px.ps1 -Target test                    # 快速测试链（默认 members，不碰 bevy）
  .\tools\px.ps1 -Target test-all                # 全量（含 px_render / px_probe，慢）
                                                 # ⚠ S8-c 标注：`px_render` 已删（§154）⇒ 今天
                                                 # 就是 `cargo test --workspace`（九个 crate）。
                                                 # 原文留着：它是这条命令当时的形状。
                                                 # ⚠ §157（2026-09-19）：wgpu 宿主改名叫 `px_render`
                                                 # ⇒ 上面"含 px_render / px_probe"**字面又成立**，
                                                 # 只是对象换了（旧义 = 已删的 Bevy 宿主）。
#>
param(
    [ValidateSet(
        'field_dual', 'gradient', 'device',
        'dual', 'dual_field', 'dual_noise',
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
    if ($Target -in 'field_dual', 'gradient', 'device', 'dual', 'dual_field', 'dual_noise') { $packages += 'px_probe' }

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

    # ⚠ `@(...)` 不是多余的：PowerShell 的 `switch` 只有**一个**匹配分支且它输出**一个**元素时，
    # 会把数组**拆成标量**（`@('test') + @()` ⇒ 字符串 `"test"`），而 `@cargoArgs` 对字符串
    # 是**按字符**摊开的 ⇒ cargo 实际收到 `test t e s t`，报「unexpected argument 's' found」。
    # 只有 `-Target test`（`$extra` 为空、结果恰好一个元素）踩得到，
    # 也就是 §106 与 J6 里写的那条命令 —— 加了 `@()` 才真的是"一条命令"。§108.3 记了这次。
    $cargoArgs = @(switch ($Target) {
        { $_ -in 'field_dual', 'gradient', 'device', 'dual', 'dual_field', 'dual_noise' } { @('run', '-p', 'px_probe', '--bin', $Target) + $extra }
        { $_ -in 'planet', 'desert', 'clouds' } { @('run', '-p', 'px_graphs', '--bin', $Target) + $extra }
        'test' { @('test') + $extra }
        'test-all' { @('test', '--workspace') + $extra }
        'check' { @('check', '-p', 'px_ops', '-p', 'px_verify', '-p', 'px_protocol') + $extra }
    })

    Write-Host ("cargo " + ($cargoArgs -join ' ')) -ForegroundColor DarkGray
    & cargo @cargoArgs @Rest
    exit $LASTEXITCODE
}
finally {
    Pop-Location
}
