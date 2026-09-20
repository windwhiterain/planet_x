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

  ⚠ 面向 driver（bin `px`）的那四个 target **就是动词**：`list` / `build` / `gc` / `run`
  （`20-build-graph.md` §182 那两个词）。它们**不带** `-Level` 的逐包 opt-level 覆盖：
  实例库自成 workspace 根（`target/jit/<key>/`），那套 `--config` 是给主 workspace 的。

.EXAMPLE
  .\tools\px.ps1 -Task field_dual -Level opt   # §46.3 的 arbiter
  .\tools\px.ps1 -Task gradient                # 探针冒烟 + 逐通道归因
  .\tools\px.ps1 -Task dual_field              # 云场梯度 vs 对偶数（纯 CPU）
  .\tools\px.ps1 -Task planet -Level opt       # 烘星球图（PCG）
  .\tools\px.ps1 -Task list                    # stage 1 计划（px list）
  .\tools\px.ps1 -Task build                   # stage 1 执行：编缺的实例（px build）
  .\tools\px.ps1 -Task gc                      # 先编缺的，再回收非活实例库（px build --gc）
  .\tools\px.ps1 -Task gc --target             # …并把共享编译中间物 target/jit/target 一起清掉
  .\tools\px.ps1 -Task gc -Deep                # 另一种写法：连 `target/jit/<非活 key>/` 一起回收
  .\tools\px.ps1 -Task run -Graph planet          # 两个 stage 一条命令（px run planet）
  .\tools\px.ps1 -Task run -Graph planet --build  # stage 1 缺就编（默认不编：运行只读）
  .\tools\px.ps1 -Task test                    # 快速测试链（默认 members，不碰 bevy）
  .\tools\px.ps1 -Task test-all                # 全量（含 px_render / px_probe，慢）
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
        'list', 'build', 'gc', 'run',
        'test', 'test-all', 'check'
    )][string]$Task = 'field_dual',

    [ValidateSet('dev', 'opt', 'release')][string]$Level = 'dev',

    # ⚠ 只有 `-Task run` 用：图名（`px run <图>`）。为什么不写成位置参数：
    #   PowerShell 参数绑定会把**单独的 `-`** 当成一个参数名的开头（`-Level` 那条路），
    #   于是 `-Task run - planet` 报「参数"-"不属于 ValidateSet」；`--` 也有歧义。
    #   显式给一个具名参数最不意外，其余参数照旧走 `$Rest`。
    [string]$Graph = '',

    # ⚠ 只有 `-Task gc` 用：连 `target/jit/<非活 key>/` 一并回收（= `px build --gc --deep`）。
    #   其余开关（如 `--target`）照旧由 `$Rest` 原样转给 driver。
    #   ⚠ 参数名是 `-Task` 而不是 `-Target`（2026-09-20 改名）：PowerShell 的参数名按**前缀**
    #   匹配，叫 `-Target` 时 `--target` 会被绑到它身上、永远转不进 driver；`Task` 不是它的前缀。
    [switch]$Deep,

    [Parameter(ValueFromRemainingArguments = $true)][string[]]$Rest = @()
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    # ⚠ §159 之后算子在 dylib 里（`px_*_op`）：`-Level opt` 必须**同时**覆盖它们，
    # 否则`planet`/`clouds` 的热代码（噪声、等值面、体积烘培）还是 O0。
    $packages = @(
        'px_graph_schema', 'px_graph',
        'px_field_schema', 'px_field_op',
        'px_volume_schema', 'px_volume_op',
        'px_mesh_schema', 'px_mesh_op',
        'px_verify', 'px_graphs'
    )
    if ($Task -in 'field_dual', 'gradient', 'device', 'dual', 'dual_field', 'dual_noise') { $packages += 'px_probe' }

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

    # ⚠ 算子库不在图程序 exe 里（这正是「改一个算子不必重编图程序」的另一面）⇒
    #   跑图之前先把它们编出来，否则驱动找不到 dylib 会**当场拒**。
    #   `run` 也是"跑图之前"（stage 2 要装载它们）；而 `list` / `build` / `gc` 只碰 stage 1
    #   （跑 `px` 自己就要 `px_cook` + `px_graphs`，`cargo run` 会把它们编出来），所以不预编。
    if ($Task -in 'planet', 'desert', 'clouds', 'run') {
        & cargo build -p px_field_op -p px_volume_op -p px_mesh_op @extra
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }

    # ⚠ `@(...)` 不是多余的：PowerShell 的 `switch` 只有**一个**匹配分支且它输出**一个**元素时，
    # 会把数组**拆成标量**（`@('test') + @()` ⇒ 字符串 `"test"`），而 `@cargoArgs` 对字符串
    # 是**按字符**摊开的 ⇒ cargo 实际收到 `test t e s t`，报「unexpected argument 's' found」。
    # 只有 `-Task test`（`$extra` 为空、结果恰好一个元素）踩得到，
    # 也就是 §106 与 J6 里写的那条命令 —— 加了 `@()` 才真的是"一条命令"。§108.3 记了这次。
    $cargoArgs = @(switch ($Task) {
        { $_ -in 'field_dual', 'gradient', 'device', 'dual', 'dual_field', 'dual_noise' } { @('run', '-p', 'px_probe', '--bin', $Task) + $extra }
        { $_ -in 'planet', 'desert', 'clouds' } { @('run', '-p', 'px_graphs', '--bin', $Task) + $extra }
        # ── driver（bin `px`）：target 名就是动词，全部**不带** `$extra`（见 .DESCRIPTION）。
        'list' { @('run', '-q', '-p', 'px_graphs', '--bin', 'px', '--', 'list') }
        'build' { @('run', '-q', '-p', 'px_graphs', '--bin', 'px', '--', 'build') }
        'gc' {
            $flags = @('--gc')
            if ($Deep) { $flags += '--deep' }
            # `$Rest` 照旧原样跟上（`px build` 只认 `--gc` / `--deep` / `--target` 三个开关）。
            @('run', '-q', '-p', 'px_graphs', '--bin', 'px', '--', 'build') + $flags
        }
        # ⚠ 两个 stage 一条命令：`px run <图> [--build]`；图名走 `-Graph`（见参数那一栏的注释），
        #   图自己的参数走 `$Rest`（`px run` 会在第一个 `--` 之后原样转给图 exe）。
        'run' {
            if ([string]::IsNullOrEmpty($Graph)) {
                Write-Host '用法：.\tools\px.ps1 -Task run -Graph <图> [--build] [-- <图自己的参数…>]' -ForegroundColor Red
                exit 2
            }
            @('run', '-q', '-p', 'px_graphs', '--bin', 'px', '--', 'run', $Graph)
        }
        'test' { @('test') + $extra }
        'test-all' { @('test', '--workspace') + $extra }
        'check' { @('check', '-p', 'px_protocol', '-p', 'px_graph_schema', '-p', 'px_graph', '-p', 'px_field_schema', '-p', 'px_field_op', '-p', 'px_volume_schema', '-p', 'px_volume_op', '-p', 'px_mesh_schema', '-p', 'px_mesh_op', '-p', 'px_verify', '-p', 'px_graphs') + $extra }
    })

    # ⚠ 打印的是**真正要跑的那一条**（`$cargoArgs` + `$Rest`）：只打 `$cargoArgs` 会漏掉
    #   由 `$Rest` 转进去的开关（`--build` / `--target` 那些），读数就与实跑不符（实测踩过）。
    Write-Host ("cargo " + (@($cargoArgs) + @($Rest) -join ' ')) -ForegroundColor DarkGray
    & cargo @cargoArgs @Rest
    exit $LASTEXITCODE
}
finally {
    Pop-Location
}
