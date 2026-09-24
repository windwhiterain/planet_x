<#
.SYNOPSIS
  渲染侧的测量仪器：**默认只测"改的那一档 + 一个参照档"**，一次扫描只起一个服务。

.DESCRIPTION
  内容全部从产物来：脚本只给图名与节点名，路径由 `target/pcg/<图>/manifest.json` 解析，
  渲染器只收 `--scene <路径>`。协议（`docs/render/instruments.md` §57）把活分成几路，
  **每一路都回一份结构化 JSON 报告**（`--report <路径>` 落盘，同一份也回给调用方）：

    · **性能主路径**（`--perf`，默认）：⚠ **已随锚退休（S8-a）** —— 见下面那条横幅。
    · **老性能路**（`-Phase scene`，回退）：⚠ **已随锚退休（S8-a）**。
    · **截图请求**（`-Phase shot`）：一串场景，每个出一张图就立刻切下一个；报告里每张图有
      产物键、路径、字节数、sha256、分辨率，以及可用性标记 `has_cloud`（防"丢云壳"）。
    · **A/B 正对照**（`-Phase ab`）：同一对场景、只差 shader 成员键；A→B→A→B 逐字节复核。

  ⚠⚠ **S8-a：Perf 与 Stable 两路退休了，别在这里把它们补回来。**
     它们要的是**计时用的帧循环**（逐帧采样 / 丢窗 / 等 K 帧 + 每条 pass 的编码器级
     GPU 时间戳），而今天唯一在的宿主（`px_render`，本脚本的 `$Exe` 已换成它）
     **按需渲染**：一条请求画一帧就回话，给不出逐帧序列、也没有那七段 span
     ⇒ 服务端收到这类请求**当场拒**，而那句拒词**指不到真正的原因**。
     真正的原因是：**能给出可比 `gpu_ms` / `pair` 的那支宿主（bevy 锚 exe）已经不在了、
     且不可重建**（冻结的构建产物，重建出来的不是同一个字节序列 —— `docs/anchors.md`）。
     ⚠ **§157 修正（2026-09-19）**：上面这句在写下时（S8-c）**不成立** —— 实测
     `target/debug/px_render.exe` 是一支**还能跑的 Bevy 宿主**（六份冻产物出图与
     那六张已删除的判据图 逐字节全中，仪器 `target/pre-rename/bevy-six.ps1`）；**裁决是不留**，
     改名又覆盖那个路径 ⇒ **从 §157 起这句成立**。全文与"拿回来的路"见 `docs/anchors.md`
     与 `docs/archive/render-wgpu.md` §157。
     ⇒ 这两路由 `Stop-RetiredPhase` **当场拒并说清**（不是等 180 s 超时、也不是等服务端
     回一句"这一路不在这一版"）。量法与全部读数留在 git 历史与 `docs/archive/render-wgpu.md` §147/§153。
     本宿主**有的**计时仪器是 `px_render --spans 预热,测量`（§153 的 J4 仪器，量**逐条
     pass** 的编码器级时间戳）—— ⚠ 它**不是** `--perf` 的替代品，名字与口径都不同。

  `-Sweep` 才是大扫（5 档 × `-Rounds` 轮）—— ⚠ 它只服务那两条退休的路，今天一起失效。

  仪器这一侧只做三件事：**起/停服务**、**解析产物**、**把报告折成表**。
  「窗口/帧是从哪来的、有没有被污染」由服务端在报告里交代，harness 不再去数日志行。

  ⚠ 起服务前有**单例硬闸**（`Assert-NoOtherRenderServer`）：本机还有 `px_render*` 在跑、
  或 `target/render-server.json` 还在，就报错退出并打印占用者 —— 两个并列的测量循环会让
  双方的数据都作废，这条闸把它变成会失败的门，而不是靠人记得。

  ⚠ 起服务前有 **shader 成员一致性断言**（`Assert-ShaderMembersAgree`）：整批场景必须钉同一份
  `shaders/clouds@<内容键>`。不一致就退出 —— 混版量出来的数没有意义（`-AllowMixedShaders`
  是给"故意交错两版"那个实验留的口子）。

  ⚠⚠ **事故记录（4）：「判据跑的是哪一份产物」（§122 / §131.2 / §144 那条形状第四次咬人）。**
  `Invoke-StablePhase` 里 `Resolve-Artifact`（按 `target/pcg/<图>/manifest.json` 解析路径）
  曾经跑在 `Invoke-Bake` **之前**，而另外三个相位早就写着"次序不能反"并已修好 ——
  **只有 stable 这一路漏了**。后果不是报错，而是**静默量错东西**：先取路径再重烘 ⇒ 这一轮量的
  是**上一份**产物（实测：清单里还是老形状的 `orbit-bare`（键 `28a9b516c132`，无帧图），
  于是探针拿它出了四档的数，而当时刚烘出来的是 `add550e772b2`）。
  ⚠ 那批读数**本身是有效的**（它们来自老形状的 `orbit-bare` —— 那正是锚宿主能读的那种文档，
  见下条），有问题的是**探针没说清它读的是哪一份**。⇒ 报告里那句"档"要能回答
  "我跑的是哪份产物"，否则数与产物对不上账。
  ⚠ 与之配套的另一条：`target/oracle/px_render-bevy.exe`（S-1 那支锚，git `4fc772d`）
  **读不了帧图形状的文档**（`PassSchema` 那些栏是它之后才加的：`missing field 'shader'`）。
  要给锚取数就走 `--bin scene <档> --no-frame-graph` 烘**老形状**产物 + 本探针 `-Bake:$false`
  （老形状可以当"提问的靶子"，不能当"交付的形状"）。
  ⚠⚠ **S8-c 标注：上面这条"要给锚取数"的路已经断了**（§154）——
  那支 exe 不在了、也不可重建（冻结的构建产物），所以**没有"给锚取数"这回事了**。
  ⚠ **§157 修正（2026-09-19）**："断了"这句在写下时**不成立** —— 当时 `target/debug/px_render.exe`
  是一支**还能跑的 Bevy 宿主**（六份冻产物出图与 那六张已删除的判据图 逐字节全中）。
  **裁决是不留**，改名又覆盖那个路径 ⇒ **从 §157 起成立**；要拿回来见 §157 的命令。
  它读不了帧图形状这件事**仍然成立**（那是记录），老形状今天只剩**逃生门判据**这一个正当用途
  （`docs/anchors.md`）。上面那两行原文留着：它是"当时为什么这么设计相位"的出处。

.EXAMPLE
  .\tools\frame-probe.ps1 -Scenes orbit-proxy
  .\tools\frame-probe.ps1 -Sweep -Rounds 3
  .\tools\frame-probe.ps1 -Phase scene -Rounds 3 -Windows 4
  .\tools\frame-probe.ps1 -Scenes orbit-soft-new,orbit-soft-old   # 改前 vs 改后：配对
  .\tools\frame-probe.ps1 -Phase ab -Scenes orbit-soft,softfix-analytic -Bake:$false
#>
param(
    [ValidateSet('stable', 'scene', 'shot', 'legacy', 'both', 'ab')][string]$Phase = 'stable',
    [string]$Graph = 'scene',
    # 日常只给"我改的那一档"；没给参照档就自动配一个（见 .DESCRIPTION）。
    [string[]]$Scenes = @('orbit-soft'),
    [string]$Reference = 'orbit-bare',
    [int]$Frames = 60,
    [int]$Width = 2240,
    [int]$Height = 1400,
    [int]$Rounds = 1,
    # 大扫（5 档 × Rounds 轮）—— 显式开关，不是默认。
    [switch]$Sweep,
    # 拿上一次报告里的产物键与现在的清单做 diff，报出"哪几档真的变了"。
    [switch]$Changed,
    # 每档每轮的**干净**窗口数（只有 -Phase scene 这条回退路用）。
    [int]$Windows = 4,
    # 老路先丢几个窗口：重建跨过的那几个。
    [int]$DropWindows = 1,
    [bool]$Bake = $true,
    [switch]$AllowMixedShaders,
    [string]$Exe = 'target\debug\px_render.exe',
    [string]$Tag = 'fp'
)

# 大扫的那 5 档。日常不给 `-Sweep` 时它们**不参与**。
$SweepScenes = @('orbit-bare', 'orbit', 'orbit-surface', 'orbit-proxy', 'orbit-soft')
if ($Sweep) {
    if ($PSBoundParameters.ContainsKey('Scenes')) {
        throw '-Sweep 用它自己那 5 档；要指定档位就别加 -Sweep，直接给 -Scenes'
    }
    $Scenes = $SweepScenes
}

. "$PSScriptRoot\harness.ps1"

function Get-Artifacts {
    $map = [ordered]@{}
    foreach ($name in $Scenes) { $map[$name] = (Resolve-Artifact -Graph $Graph -Node $name) }
    return $map
}

function Get-SceneArgs {
    param($Artifact)
    return @('--scene', $Artifact.Path, '--pcg-root', $HarnessCacheRoot)
}

function Get-Median {
    param([double[]]$Values)
    if ($Values.Count -eq 0) { return [double]::NaN }
    $sorted = @($Values | Sort-Object)
    $mid = [int][Math]::Floor($sorted.Count / 2)
    if ($sorted.Count % 2 -eq 1) { return $sorted[$mid] }
    return ($sorted[$mid - 1] + $sorted[$mid]) / 2.0
}

# 最小二乘斜率，X = 轮序号 1..N（判"某一档随轮次单调漂移而其它档不漂"）。
function Get-Slope {
    param([double[]]$Values)
    $n = $Values.Count
    if ($n -lt 2) { return [double]::NaN }
    $meanX = ($n + 1) / 2.0
    $meanY = ($Values | Measure-Object -Average).Average
    $num = 0.0
    $den = 0.0
    for ($i = 0; $i -lt $n; $i++) {
        $dx = ($i + 1) - $meanX
        $num += $dx * ($Values[$i] - $meanY)
        $den += $dx * $dx
    }
    return $num / $den
}

function Format-Series {
    param([double[]]$Values)
    if ($Values.Count -eq 0) { return '（空）' }
    return (@($Values | ForEach-Object { '{0:N2}' -f $_ }) -join ' ')
}

# 读报告：落盘的那份就是回给调用方的那份（服务端保证逐字节相同）。
function Read-Report {
    param([string]$Path)
    if (-not (Test-Path $Path)) { throw "报告不在：$Path" }
    return (Get-Content $Path -Raw | ConvertFrom-Json)
}

# 一次**性能请求**：一个场景，先出图再收窗口。返回报告对象。
function Invoke-PerfRequest {
    param($Artifact, [string]$Err, [string]$Out, [string]$Report)
    $arguments = (Get-SceneArgs $Artifact) + @(
        '--perf', '--windows', "$Windows", '--drop', "$DropWindows",
        '--width', "$Width", '--height', "$Height", '--out', $Out, '--report', $Report)
    Invoke-Client -Arguments $arguments -Err $Err -What "性能请求 $($Artifact.Node)"
    return (Read-Report -Path $Report)
}

# 一次**截图请求**：一整批场景，每个一张图就立刻切下一个。
function Invoke-ShotsRequest {
    param($Artifacts, [string]$Err, [string]$Report, [string]$Prefix)
    $arguments = @()
    foreach ($name in $Artifacts.Keys) {
        $arguments += @('--scene', $Artifacts[$name].Path, '--out', "$Prefix$name.png")
    }
    $arguments += @('--width', "$Width", '--height', "$Height", '--report', $Report, '--shots')
    Invoke-Client -Arguments $arguments -Err $Err -What '截图请求'
    return (Read-Report -Path $Report)
}

function Get-RoundOrder {
    param([int]$Round, [string[]]$Names)
    if (-not $Names) { $Names = $Scenes }
    if ($Round % 2 -eq 1) { return @($Names) }
    return @($Names[($Names.Count - 1)..0])
}

# 产物键：`--changed` 拿它 diff。
function Get-CurrentKeys {
    param($Artifacts)
    $keys = [ordered]@{}
    foreach ($name in $Artifacts.Keys) { $keys[$name] = $Artifacts[$name].Key }
    return $keys
}

# 上一次报告里的产物键 vs 现在的清单：哪几档真的变了。
# ⚠ 反直觉但必须说清：**改 shader 会让所有钉它的云的场景键全变** ⇒ 那时这里会指向
# 全部云档。那**不代表你要全测** —— 该由人指定"我改的是哪一档"，这个工具只负责算差值与给历史。
function Show-Changed {
    param($Artifacts, [string]$Path)
    $now = Get-CurrentKeys -Artifacts $Artifacts
    if (-not (Test-Path $Path)) {
        Write-Host "  （没有上一份键记录：$Path。这一次只写下基线，下一次才有得比）"
        ($now | ConvertTo-Json) | Set-Content -Path $Path -Encoding utf8
        return
    }
    $before = Get-Content $Path -Raw | ConvertFrom-Json
    $changed = @()
    foreach ($name in $now.Keys) {
        $old = $before.$name
        if ($null -eq $old) { $changed += "$name（新增）"; continue }
        if ($old -ne $now[$name]) {
            $changed += ('{0}（{1} → {2}）' -f $name, ([string]$old).Substring(0, 12), ([string]$now[$name]).Substring(0, 12))
        }
    }
    foreach ($property in @($before.PSObject.Properties)) {
        if (-not $now.Contains($property.Name)) { $changed += "$($property.Name)（这次不在清单里）" }
    }
    if ($changed.Count -eq 0) {
        Write-Host '  ⇒ 这几档的产物键**一个都没变**：现在量的是同一份内容（键 = 内容）。'
    } else {
        Write-Host "  ⇒ 键变了的档：$($changed -join ' / ')"
        Write-Host '  ⚠ 改 shader 会让**所有钉它的云的场景**键全变 —— 那不代表你要全测；'
        Write-Host '     该由人指定"我改的是哪一档"，这里只负责算差值与给历史。'
    }
    ($now | ConvertTo-Json) | Set-Content -Path $Path -Encoding utf8
}

function Invoke-Bake {
    param([string[]]$Names)
    if (-not $Names) { $Names = $Scenes }
    Write-Host '==== 统一重烘（保证整批场景钉同一份 WGSL）===='
    & cargo run -p px_graphs --bin shaders 2>&1 | ForEach-Object { Write-Host "  $_" }
    if ($LASTEXITCODE -ne 0) { throw "重烘 shaders 失败（退出码 $LASTEXITCODE）" }
    foreach ($name in $Names) {
        & cargo run -p px_graphs --bin scene $name 2>&1 | ForEach-Object { Write-Host "  $_" }
        if ($LASTEXITCODE -ne 0) { throw "重烘场景 $name 失败（退出码 $LASTEXITCODE）" }
    }
}

function Write-ShaderTable {
    param($Table)
    Write-Host '==== 这批场景钉的 shader（断言已过：同槽只有一个内容键）===='
    Format-ShaderMembers -Table $Table | ForEach-Object { Write-Host "  $_" }
}

# ---------------------------------------------------------------------------
# ⚠⚠ 两条**随锚退休**的路：Perf 与 Stable（S8-a）
# ---------------------------------------------------------------------------
#
# 为什么是"当场拒并说清"而不是"删掉这几段代码"：**这条工具的量法本身是判据的一部分**
# （§51.18 / §57 / §147.4 的配对差、漂移、第一窗口污染那几栏），删掉它等于把"我们量过什么"
# 从脚本里抹掉。留着的代价只有一个：**它不能再跑** —— 而这一点必须由这里的一句话说出来，
# 不许留给服务端去说：服务端只会说"这一路不在这一版"（`px_render/src/serve.rs`），
# 那句话**指不到真正的原因**（能给出可比 `gpu_ms` / `pair` 的那支宿主已经不在了）。
function Stop-RetiredPhase {
    param([string]$Phase, [string]$What)
    throw @"
-Phase $Phase 已**随锚退休**（S8-a）：这条请求这里不会发出去。
  它要的是 $What 那套**计时用的帧循环**（逐帧采样 / 丢窗 / 等"重建后已渲染 K 帧"
  + 每条 pass 的编码器级 GPU 时间戳），而今天唯一在的宿主（px_render）是
  **按需渲染**的：一条请求画一帧就回话，没有逐帧序列、也没有那七段 span。
  ⚠ 而能给出可比 `gpu_ms` / `pair` 的那支宿主（bevy 锚 exe）**已经不在了，且不可重建**
    —— 它是冻结的构建产物，重建出来的不是同一个字节序列（`docs/anchors.md`）。
  ⇒ 这两路**退休，不再修**。它们的量法与全部读数留在 git 历史与
     docs/archive/render-wgpu.md §147 / §153 里。
  本宿主**有的**那件计时仪器是 `px_render --spans 预热,测量`（§153 的 J4 仪器）：
    它量的是**逐条 pass** 的编码器级时间戳 —— ⚠ 它**不是** `--perf` 的替代品，
    名字与口径都不同（那个数不叫 gpu_ms）。
  还在的两条路：`-Phase shot`（判据图 + has_cloud）与 `-Phase ab`（A/B 正对照）。
"@
}

# ---------------------------------------------------------------------------
# 性能：新协议（一个服务，档间不重启）
# ---------------------------------------------------------------------------
function Invoke-ScenePhase {
    param([switch]$SkipBake)
    Stop-RetiredPhase -Phase 'scene' -What 'Perf（逐帧采样 + 丢窗）'
    # ⚠ 次序不能反：`Get-Artifacts` 取的是**清单里的键**，而清单要重烘之后才指向新内容。
    # 先取路径再重烘 ⇒ 这一轮量的是上一份产物（实测踩过：改了 surface shader、键也换了，
    # 出图却一个像素没变 —— 因为整批场景路径还是旧的）。
    if ($Bake -and -not $SkipBake) { Invoke-Bake }
    $artifacts = Get-Artifacts
    $table = Assert-ShaderMembersAgree -Artifacts $artifacts -AllowMixed:$AllowMixedShaders
    Write-Host ''
    Write-ShaderTable -Table $table
    Write-Host ''
    Write-Host '==== 起服务（整轮扫描只起这一次，档间不重启）===='
    $log = "target/$Tag-scene.log"
    $err = "$log.err"
    $server = Start-RenderServer -Extra @('--fps') -Log $log -Err $err -Width $Width -Height $Height
    $perRoundRecs = [ordered]@{}
    foreach ($name in $Scenes) { $perRoundRecs[$name] = @() }
    $scan = [System.Diagnostics.Stopwatch]::StartNew()
    try {
        Write-Host ("  pid {0}；收尾只停这一个" -f $server.Id)
        # 热身：启动时槽里是占位 WGSL，第一次推真场景要现编管线。
        # 用"带 shader 槽最多的那一档"热身，把这次编译付掉，不进样本。
        $warm = $Scenes[0]
        $most = -1
        foreach ($name in $Scenes) {
            $count = @($table[$name].Keys).Count
            if ($count -gt $most) { $most = $count; $warm = $name }
        }
        Write-Host ("---- 热身一次（{0}）----" -f $warm)
        $null = Invoke-PerfRequest -Artifact $artifacts[$warm] -Err $err -Out "target/$Tag-warm.png" -Report "target/$Tag-warm.json"
        $scan.Restart()
        for ($round = 1; $round -le $Rounds; $round++) {
            $order = @(Get-RoundOrder -Round $round)
            Write-Host ('---- 第 {0} 轮（{1}）----' -f $round, ($order -join ' → '))
            for ($position = 1; $position -le $order.Count; $position++) {
                $name = $order[$position - 1]
                $report = Invoke-PerfRequest -Artifact $artifacts[$name] -Err $err `
                    -Out "target/$Tag-r$round-$name.png" -Report "target/$Tag-r$round-$name.json"
                $perf = @($report.perf)[0]
                if ($null -eq $perf) { throw "$name 的报告里没有 perf 段" }
                $perRoundRecs[$name] += [pscustomobject]@{
                    Round = $round; Position = $position
                    Windows = @($perf.windows | ForEach-Object { [double]$_ })
                    Dropped = @($perf.dropped | ForEach-Object { [double]$_ })
                    Median = [double]$perf.median
                    ErrorBar = [double]$perf.error_bar.value_ms
                    Rule = $perf.error_bar.rule
                    Gpu = $perf.gpu
                    Shot = @($report.shots)[0]
                    Millis = [int]$report.millis
                }
                Write-Host ('  {0,-14} min {1,7:N2} / 中位 {2,7:N2} ms（n={3}）｜丢 {4}｜请求 {5:N1} s' -f `
                        $name, ($perf.windows | Measure-Object -Minimum).Minimum, $perf.median,
                    @($perf.windows).Count, (Format-Series @($perf.dropped | ForEach-Object { [double]$_ })), ($report.millis / 1000.0))
            }
        }
    }
    finally {
        $scan.Stop()
        Stop-RenderServer $server
    }

    Write-Host ''
    Write-Host '==== 每档每轮的原始窗口（全打；丢弃的那几窗也打出来）===='
    foreach ($name in $Scenes) {
        foreach ($row in $perRoundRecs[$name]) {
            Write-Host ('{0,-14} 第 {1} 轮（位 {2}/{3}）丢弃 {4,-16} 干净 {5}' -f `
                    $name, $row.Round, $row.Position, $Scenes.Count, (Format-Series $row.Dropped), (Format-Series $row.Windows))
            $util = @($row.Gpu | ForEach-Object { $_.util_pct } | Where-Object { $_ })
            $sm = @($row.Gpu | ForEach-Object { $_.sm_mhz } | Where-Object { $_ })
            $pw = @($row.Gpu | ForEach-Object { $_.power_w } | Where-Object { $_ })
            $vr = @($row.Gpu | ForEach-Object { $_.vram_mib_max } | Where-Object { $_ })
            $tp = @($row.Gpu | ForEach-Object { $_.temp_c_max } | Where-Object { $_ })
            Write-Host ('{0,-14}           窗口期 SM {1}–{2} MHz／功耗 {3}–{4} W／利用率 {5}–{6}%／显存峰 {7} MiB／温度峰 {8}°C' -f `
                    '', ($sm | Measure-Object -Minimum).Minimum, ($sm | Measure-Object -Maximum).Maximum,
                    ($pw | Measure-Object -Minimum).Minimum, ($pw | Measure-Object -Maximum).Maximum,
                    ($util | Measure-Object -Minimum).Minimum, ($util | Measure-Object -Maximum).Maximum,
                    ($vr | Measure-Object -Maximum).Maximum, ($tp | Measure-Object -Maximum).Maximum)
            Write-Host ('{0,-14}           可用性：{1}' -f '', $row.Shot.verdict)
        }
    }

    Write-Host ''
    Write-Host '==== 每档汇总 ===='
    Write-Host '  误差棒（跨轮）= 各轮中位数的半极差 (max−min)/2；单次请求内的半极差由报告另给。'
    Write-Host ('{0,-14} {1,9} {2,9} {3,7} {4,9} {5,9}  {6}' -f '档', '合并中位', '误差棒±', 'n', 'min', '请求内±', '每轮中位')
    $summary = [ordered]@{}
    foreach ($name in $Scenes) {
        $all = @($perRoundRecs[$name] | ForEach-Object { $_.Windows } | ForEach-Object { $_ })
        $perRound = @($perRoundRecs[$name] | ForEach-Object { $_.Median })
        $bar = (($perRound | Measure-Object -Maximum).Maximum - ($perRound | Measure-Object -Minimum).Minimum) / 2.0
        $inner = ($perRoundRecs[$name] | ForEach-Object { $_.ErrorBar } | Measure-Object -Maximum).Maximum
        $summary[$name] = [pscustomobject]@{ Median = (Get-Median $all); Bar = $bar; N = $all.Count; PerRound = $perRound }
        Write-Host ('{0,-14} {1,9:N2} {2,9:N2} {3,7} {4,9:N2} {5,9:N2}  {6}' -f `
                $name, $summary[$name].Median, $bar, $all.Count, ($all | Measure-Object -Minimum).Minimum, $inner, (Format-Series $perRound))
    }

    Write-Host ''
    Write-Host '==== (a) 热漂移：每档的「每轮中位」随轮次有没有单调漂移 ===='
    Write-Host '  判据：|漂移|（斜率 × (轮数−1)）与**轮内极差**比 —— 后者是本档的重复性噪声。'
    Write-Host ('{0,-14} {1,10} {2,12} {3,11} {4,11}  {5}' -f '档', '斜率ms/轮', '漂移(全扫描)', '轮间半极差', '轮内极差', '判')
    $drifted = @()
    foreach ($name in $Scenes) {
        $perRound = @($summary[$name].PerRound)
        $slope = Get-Slope -Values $perRound
        $drift = if ([double]::IsNaN($slope)) { [double]::NaN } else { $slope * ($perRound.Count - 1) }
        $within = 0.0
        foreach ($row in $perRoundRecs[$name]) {
            $spread = ($row.Windows | Measure-Object -Maximum).Maximum - ($row.Windows | Measure-Object -Minimum).Minimum
            if ($spread -gt $within) { $within = $spread }
        }
        $verdict = if ([double]::IsNaN($drift)) { '轮数不够，量不了' }
        elseif ([Math]::Abs($drift) -le $within) { '未见（|漂移| ≤ 轮内极差）' }
        else { '⚠ 漂移超过轮内极差' }
        if ($verdict -like '⚠*') { $drifted += $name }
        Write-Host ('{0,-14} {1,10:N4} {2,12:N2} {3,11:N2} {4,11:N2}  {5}' -f `
                $name, $slope, $drift, $summary[$name].Bar, $within, $verdict)
    }
    if ($drifted.Count -eq 0) {
        Write-Host '  ⇒ 没有哪一档的漂移超过它自己的轮内极差：**未见**"某一档单调漂移而其它档不漂"。'
    } else {
        Write-Host "  ⇒ ⚠ 这几档的漂移超过轮内极差，要单独量化：$($drifted -join ' / ')"
    }

    Write-Host ''
    Write-Host '==== (b) 第一窗口污染：含第一窗口 vs 丢弃第一窗口 ===='
    Write-Host ('{0,-14} {1,12} {2,12} {3,12} {4,10}  {5}' -f '档', '含第一窗中位', '丢第一窗中位', '差', '差%', '第一窗原始值')
    foreach ($name in $Scenes) {
        $withFirst = @($perRoundRecs[$name] | ForEach-Object { @($_.Dropped) + @($_.Windows) } | ForEach-Object { $_ })
        $without = @($perRoundRecs[$name] | ForEach-Object { $_.Windows } | ForEach-Object { $_ })
        $a = Get-Median $withFirst
        $b = Get-Median $without
        $crossed = @($perRoundRecs[$name] | ForEach-Object { $_.Dropped } | ForEach-Object { $_ })
        Write-Host ('{0,-14} {1,12:N2} {2,12:N2} {3,12:N2} {4,9:N2}%  {5}' -f `
                $name, $a, $b, ($a - $b), (100.0 * ($a - $b) / $b), (Format-Series $crossed))
    }

    Write-Host ''
    Write-Host ('==== 整轮墙钟（第 1 轮起算，不含起服务与热身）：{0:N1} s = {1:N2} min ====' -f `
            $scan.Elapsed.TotalSeconds, ($scan.Elapsed.TotalSeconds / 60))
}

# ---------------------------------------------------------------------------
# 性能主路径：等条件成立 + 逐帧采样 + 配对差
# ---------------------------------------------------------------------------

# 一次**新主路径**请求：每一步都等条件成立再逐帧采样。`--out` 不给：这一路不出图。
function Invoke-StableRequest {
    # ⚠ 这里**不能**写 `[string[]]`：传进来的是产物对象，会被字符串化成一个空串。
    param($Artifacts, [string]$Err, [string]$Report)
    $arguments = @('--perf', '--frames', "$Frames", '--width', "$Width", '--height', "$Height",
        '--report', $Report)
    foreach ($artifact in $Artifacts) {
        $arguments += @('--scene', $artifact.Path, '--pcg-root', $HarnessCacheRoot)
    }
    Invoke-Client -Arguments $arguments -Err $Err -What '性能请求（新主路径）'
    return (Read-Report -Path $Report)
}

function Format-Quantiles {
    param($Perf)
    $mean = if (@($Perf.frames).Count -gt 0) { (@($Perf.frames) | Measure-Object -Average).Average } else { [double]::NaN }
    return ('min {0,7:N2} / p50 {1,7:N2} / p90 {2,7:N2} / p99 {3,7:N2} / max {4,7:N2} ms（n={5}，均值 {6,6:N2}，长尾 p99−p50 {7,6:N2}）' -f `
            [double]$Perf.min, [double]$Perf.p50, [double]$Perf.p90, [double]$Perf.p99, [double]$Perf.max,
        [int]$Perf.n, $mean, ([double]$Perf.p99 - [double]$Perf.p50))
}

function Format-Waits {
    param($Perf)
    $w = $Perf.waits
    if ($null -eq $w) { return '（没有 waits 段）' }
    return ('等管线 {0,6:N0} ms ｜ 等资产 {1,5:N1} ms ｜ 等稳定 {2,5:N0} ms ｜ 采样 {3,6:N0} ms ｜ 合计 {4,6:N0} ms' -f `
            [double]$w.pipelines_ms, [double]$w.assets_ms, [double]$w.settle_ms, [double]$w.sample_ms, [double]$w.total_ms)
}

function Format-Gpu {
    param($Perf)
    $g = $Perf.gpu_ms
    if ($null -eq $g) { return '（设备没给时间戳：这一档只有 app 侧的数）' }
    $c = $Perf.compare
    return ('GPU min {0,6:N2} / p50 {1,6:N2} / p90 {2,6:N2} / p99 {3,6:N2} / max {4,6:N2} ms（n={5}，凑够样本多等 {6} 帧，取值 {7:N1} µs/帧）｜GPU/app：中位 {8:N3} 均值 {9:N3}' -f `
            [double]$g.min, [double]$g.p50, [double]$g.p90, [double]$g.p99, [double]$g.max, [int]$g.n,
        [int]$g.lag_frames, [double]$g.poll_us, `
        $(if ($null -eq $c) { [double]::NaN } else { [double]$c.ratio }), `
        $(if ($null -eq $c) { [double]::NaN } else { [double]$c.ratio_mean }))
}

# 一次测量里每个场景一份账：逐帧值存进 `windows` 字段的名字太容易误读，这里就叫 Frames。
function Get-FrameRec {
    param($Perf, [string]$Node, [string]$Order)
    return [pscustomobject]@{
        Node    = $Node
        Order   = $Order
        Frames  = @($Perf.frames | ForEach-Object { [double]$_ })
        Gpu     = @($Perf.gpu_ms.frames | ForEach-Object { [double]$_ })
        Min     = [double]$Perf.min
        P50     = [double]$Perf.p50
        P90     = [double]$Perf.p90
        P99     = [double]$Perf.p99
        Max     = [double]$Perf.max
        N       = [int]$Perf.n
        Key     = [string]$Perf.key
        Waits   = $Perf.waits
        GpuMs   = $Perf.gpu_ms
        Compare = $Perf.compare
        Millis  = [int]$Perf.n
    }
}

function Invoke-StablePhase {
    param([switch]$SkipBake)
    Stop-RetiredPhase -Phase 'stable' -What 'Stable（性能主路径）'
    # 配对规则（写在文档里，别让人猜）：
    #   · 给 1 档  ⇒ 它 + 参照档（默认 orbit-bare）= 一次配对测量；
    #   · 给 2 档  ⇒ 就是"改前 vs 改后"这一对，不再加参照；
    #   · 给 ≥3 档 ⇒ 每档各自与参照档配一次（大扫走这条）。
    $units = @()
    if ($Scenes.Count -eq 1) {
        if ($Scenes[0] -eq $Reference) { $units += , @($Scenes[0]) }
        else { $units += , @($Scenes[0], $Reference) }
    }
    elseif ($Scenes.Count -eq 2) {
        $units += , @($Scenes[0], $Scenes[1])
    }
    else {
        foreach ($name in $Scenes) {
            if ($name -eq $Reference) { $units += , @($name) }
            else { $units += , @($name, $Reference) }
        }
    }
    $needed = [ordered]@{}
    foreach ($unit in $units) { foreach ($name in $unit) { $needed[$name] = $true } }
    $bakeNames = @($needed.Keys)
    if ($Bake -and -not $SkipBake) { Invoke-Bake -Names $bakeNames }
    # ⚠ **次序不能反**（`Invoke-ScenePhase` / `Invoke-ShotPhase` / `Invoke-AbPhase` 顶上那句注释
    #    是同一件事）：`Resolve-Artifact` 取的是**清单里的键**，而清单要重烘之后才指向新内容。
    #    这一条在本相位**漏过一次**，代价见文件头「事故记录」第 4 条 —— 所以解析放在烘之后。
    $artifacts = [ordered]@{}
    foreach ($name in $needed.Keys) { $artifacts[$name] = (Resolve-Artifact -Graph $Graph -Node $name) }
    $table = Assert-ShaderMembersAgree -Artifacts $artifacts -AllowMixed:$AllowMixedShaders
    Write-Host ''
    Write-ShaderTable -Table $table
    if ($Changed) {
        Write-Host ''
        Write-Host '==== -Changed：上一次报告里的产物键 vs 现在的清单 ===='
        Show-Changed -Artifacts $artifacts -Path "target/$Tag-keys.json"
    }
    Write-Host ''
    Write-Host ("==== 配对单元：{0} ====" -f (($units | ForEach-Object { '(' + ($_ -join ' − ') + ')' }) -join ' '))
    Write-Host '  判据是**配对差**（被测档 − 参照档），不是任何一档的绝对值。'

    $log = "target/$Tag-stable.log"
    $err = "$log.err"
    $server = Start-RenderServer -Extra @('--fps') -Log $log -Err $err -Width $Width -Height $Height
    $records = @()
    $scan = [System.Diagnostics.Stopwatch]::StartNew()
    try {
        Write-Host ("  pid {0}；收尾只停这一个" -f $server.Id)
        # 热身一次（不进样本）：启动时槽里是占位 WGSL，第一次推真场景要现编管线。
        $first = @($units[0] | ForEach-Object { $artifacts[$_] })
        Write-Host ("---- 热身一次（{0}）----" -f ($units[0] -join ' + '))
        $null = Invoke-StableRequest -Artifacts $first -Err $err -Report "target/$Tag-stable-warm.json"
        $scan.Restart()
        for ($round = 1; $round -le $Rounds; $round++) {
            Write-Host ('---- 第 {0} 轮 ----' -f $round)
            for ($slot = 1; $slot -le $units.Count; $slot++) {
                $unit = $units[$slot - 1]
                $order = @(Get-RoundOrder -Round $round -Names $unit)
                $report = Invoke-StableRequest -Artifacts @($order | ForEach-Object { $artifacts[$_] }) -Err $err `
                    -Report ("target/$Tag-stable-r$round-$($unit[0]).json")
                $perf = @($report.perf)
                $label = ($order -join ' → ')
                $sign = 1.0
                if ($unit.Count -eq 2 -and $perf[0].scene -ne $artifacts[$unit[0]].Path) { $sign = -1.0 }
                $pair = $report.pair
                $appDelta = [double]::NaN
                $gpuDelta = [double]::NaN
                if ($null -ne $pair -and $unit.Count -eq 2) {
                    $appDelta = $sign * [double]$pair.app_delta_ms
                    if ($null -ne $pair.gpu_delta_ms) { $gpuDelta = $sign * [double]$pair.gpu_delta_ms }
                }
                Write-Host ('  [{0}] {1}（{2:N1} s）' -f $slot, $label, ([int]$report.millis / 1000.0))
                for ($i = 0; $i -lt $perf.Count; $i++) {
                    $name = $perf[$i].label
                    Write-Host ('      {0,-34} {1}' -f $name, (Format-Quantiles -Perf $perf[$i]))
                    Write-Host ('      {0,-34} {1}' -f '', (Format-Waits -Perf $perf[$i]))
                    Write-Host ('      {0,-34} {1}' -f '', (Format-Gpu -Perf $perf[$i]))
                    # 参照档那一份单独成行：把它并进"被测档"那一行会让人以为那一档就是这么慢。
                    $role = if ($i -eq 0 -or $unit.Count -eq 1) { $unit[0] } else { $unit[1] + '（参照）' }
                    $records += Get-FrameRec -Perf $perf[$i] -Node $role -Order $label
                }
                if ($unit.Count -eq 2) {
                    $suffix = if ($sign -lt 0) { '（这一轮顺序反了，已归一化到「' + $unit[0] + ' − ' + $unit[1] + '」）' } else { '' }
                    Write-Host ('      配对差 {0} − {1}：app 中位 {2:N2} ms（⚠ 双峰，别用）｜app 均值 {3:N2} ms｜GPU {4}{5}' -f `
                            $unit[0], $unit[1], $appDelta, ($sign * [double]$pair.app_mean_delta_ms), `
                            $(if ([double]::IsNaN($gpuDelta)) { '（没有时间戳）' } else { '{0:N2} ms' -f $gpuDelta }), $suffix)
                    $records += [pscustomobject]@{
                        Node = "$($unit[0]) − $($unit[1])"; Order = $label; Frames = @(); Gpu = @()
                        # 配对行借这两个槽：P50 放 app**均值**口径的差值（中位那个双峰，不能用），
                        # Min 放 GPU p50 口径的差值。
                        Min = $gpuDelta; P50 = ($sign * [double]$pair.app_mean_delta_ms); P90 = [double]::NaN; P99 = [double]::NaN
                        Max = [double]::NaN; N = 0; Key = ''; Waits = $null; GpuMs = $null; Compare = $null
                        Millis = 0
                    }
                }
            }
        }
    }
    finally {
        $scan.Stop()
        Stop-RenderServer $server
    }

    Write-Host ''
    Write-Host '==== 每档分位数（把各轮的逐帧值合起来算）===='
    Write-Host ('{0,-16} {1,8} {2,8} {3,8} {4,8} {5,8} {6,7} {7,9}  {8}' -f '档', 'min', 'p50', 'p90', 'p99', 'max', 'n', '长尾', '每轮 p50')
    $byNode = [ordered]@{}
    foreach ($row in $records) {
        if ($row.N -eq 0) { continue }
        if (-not $byNode.Contains($row.Node)) { $byNode[$row.Node] = @() }
        $byNode[$row.Node] += $row
    }
    foreach ($node in $byNode.Keys) {
        $all = @($byNode[$node] | ForEach-Object { $_.Frames } | ForEach-Object { $_ })
        $sorted = @($all | Sort-Object)
        $q = {
            param($s, $p)
            if ($s.Count -eq 0) { return [double]::NaN }
            if ($s.Count -eq 1) { return $s[0] }
            $pos = $p * ($s.Count - 1)
            $lo = [Math]::Floor($pos); $hi = [Math]::Ceiling($pos)
            if ($lo -eq $hi) { return $s[$lo] }
            return $s[$lo] * (1 - ($pos - $lo)) + $s[$hi] * ($pos - $lo)
        }
        $p50 = & $q $sorted 0.50
        Write-Host ('{0,-16} {1,8:N2} {2,8:N2} {3,8:N2} {4,8:N2} {5,8:N2} {6,7} {7,9:N2}  {8}' -f `
                $node, $sorted[0], $p50, (& $q $sorted 0.90), (& $q $sorted 0.99), $sorted[$sorted.Count - 1], `
                $all.Count, ((& $q $sorted 0.99) - $p50), (Format-Series @($byNode[$node] | ForEach-Object { $_.P50 })))
    }

    $pairRows = @($records | Where-Object { $_.Node -like '*−*' })
    $deltas = @($pairRows | ForEach-Object { $_.P50 })
    $gpuDeltas = @($pairRows | ForEach-Object { $_.Min } | Where-Object { -not [double]::IsNaN($_) })
    if ($deltas.Count -gt 0) {
        Write-Host ''
        Write-Host '==== 配对差（被测档 − 参照档）===='
        $sortedDelta = @($deltas | Sort-Object)
        $mid = [int][Math]::Floor($sortedDelta.Count / 2)
        $median = if ($sortedDelta.Count % 2 -eq 1) { $sortedDelta[$mid] } else { ($sortedDelta[$mid - 1] + $sortedDelta[$mid]) / 2.0 }
        $bar = if ($sortedDelta.Count -lt 2) { [double]::NaN } else { ($sortedDelta[-1] - $sortedDelta[0]) / 2.0 }
        Write-Host ('  逐轮（app 均值口径的差；± 是各轮差的半极差，不是标准差）：{0}' -f (Format-Series $deltas))
        Write-Host ('  app 均值口径：中位 {0:N2} ms｜跨轮误差棒± {1:N2} ms（轮数 {2}）' -f $median, $bar, $sortedDelta.Count)
        if ($gpuDeltas.Count -gt 0) {
            Write-Host ('  逐轮（GPU p50 口径的差）：{0}' -f (Format-Series $gpuDeltas))
            Write-Host ('  GPU 口径：中位 {0:N2} ms' -f (Get-Median $gpuDeltas))
        } else {
            Write-Host '  GPU 口径：没有时间戳，量不了。'
        }
        if ($sortedDelta.Count -lt 2) {
            Write-Host '  ⚠ 轮数 1 ⇒ 误差棒量不出来。要误差棒再加 -Rounds N；日常不必。'
        }
    }

    Write-Host ''
    Write-Host ('==== 整轮墙钟（第 1 轮起算，不含起服务与热身）：{0:N1} s = {1:N2} min ====' -f `
            $scan.Elapsed.TotalSeconds, ($scan.Elapsed.TotalSeconds / 60))
}

# ---------------------------------------------------------------------------
# 截图：新协议（一整批一个请求）
# ---------------------------------------------------------------------------
function Invoke-ShotPhase {
    param([switch]$SkipBake)
    # ⚠ 次序不能反：`Get-Artifacts` 取的是**清单里的键**，而清单要重烘之后才指向新内容。
    # 先取路径再重烘 ⇒ 这一轮量的是上一份产物（实测踩过：改了 surface shader、键也换了，
    # 出图却一个像素没变 —— 因为整批场景路径还是旧的）。
    if ($Bake -and -not $SkipBake) { Invoke-Bake }
    $artifacts = Get-Artifacts
    $table = Assert-ShaderMembersAgree -Artifacts $artifacts -AllowMixed:$AllowMixedShaders
    Write-Host ''
    Write-ShaderTable -Table $table
    $log = "target/$Tag-shot.log"
    $err = "$log.err"
    $server = Start-RenderServer -Extra @() -Log $log -Err $err -Width $Width -Height $Height
    $reports = @()
    try {
        # 热身一次（用带槽最多的那一档），把启动那次 install+重编付掉。
        # ⚠ 用**截图请求**热身，不用性能请求：这一路的服务没带 --fps，性能请求会被拒收。
        $warm = $Scenes[0]
        $most = -1
        foreach ($name in $Scenes) {
            $count = @($table[$name].Keys).Count
            if ($count -gt $most) { $most = $count; $warm = $name }
        }
        $warmOnly = [ordered]@{}
        $warmOnly[$warm] = $artifacts[$warm]
        $null = Invoke-ShotsRequest -Artifacts $warmOnly -Err $err -Report "target/$Tag-warm.json" -Prefix "target/$Tag-warm-"
        for ($round = 1; $round -le $Rounds; $round++) {
            $order = @(Get-RoundOrder -Round $round)
            Write-Host ('---- 判据图 第 {0} 轮（{1}）----' -f $round, ($order -join ' → '))
            $args = [ordered]@{}
            foreach ($name in $order) { $args[$name] = $artifacts[$name] }
            $report = Invoke-ShotsRequest -Artifacts $args -Err $err `
                -Report "target/$Tag-shot-r$round.json" -Prefix "target/$Tag-shot-r$round-"
            $reports += [pscustomobject]@{ Round = $round; Report = $report }
        }
    }
    finally {
        Stop-RenderServer $server
    }

    Write-Host ''
    Write-Host '==== 有云判据（服务端算的：与**这一批第一张**做 16×10 网格光通量差分）===='
    Write-Host ('{0,-6} {1,-14} {2,10} {3,14} {4,10} {5}' -f '轮', '档', '字节', '网格差分', 'has_cloud', '判')
    $noCloud = @()
    foreach ($entry in $reports) {
        foreach ($shot in @($entry.Report.shots)) {
            $name = [System.IO.Path]::GetFileNameWithoutExtension($shot.out) -replace "^$([regex]::Escape($Tag))-shot-r$($entry.Round)-", ''
            Write-Host ('{0,-6} {1,-14} {2,10} {3,14} {4,10} {5}' -f `
                    $entry.Round, $name, $shot.bytes, $shot.diff_vs_ref_grid, $shot.has_cloud, $shot.verdict)
            if ($shot.declared_clouds -and -not $shot.has_cloud) { $noCloud += "第$($entry.Round)轮/$name" }
        }
    }
    if ($noCloud.Count -eq 0) {
        Write-Host '⇒ 每一档每一轮的判据图都过：**没有出现过无云的图**。'
    } else {
        Write-Host "⇒ ⚠ 出现过无云的图：$($noCloud -join ' / ')"
    }
}

# ---------------------------------------------------------------------------
# 旧协议：单档冷启动 + 档间重启
# ---------------------------------------------------------------------------
function Invoke-LegacyPhase {
    param($Artifacts, $Table)
    Stop-RetiredPhase -Phase 'legacy（旧协议：单档冷启动 + 档间重启）' -What 'Perf（逐窗口采样）'
    $log = "target/$Tag-legacy.log"
    $err = "$log.err"
    $records = [ordered]@{}
    foreach ($name in $Scenes) { $records[$name] = @() }
    $scan = [System.Diagnostics.Stopwatch]::StartNew()
    for ($round = 1; $round -le $Rounds; $round++) {
        $order = @(Get-RoundOrder -Round $round)
        Write-Host ('---- 旧协议 第 {0} 轮（{1}）----' -f $round, ($order -join ' → '))
        foreach ($name in $order) {
            $server = Start-RenderServer -Extra @('--fps') -Log $log -Err $err -Width $Width -Height $Height
            try {
                # 冷启动 ⇒ 每档都要先热身一张（P14/P17：冷启动第一个请求可能丢云壳）。
                $null = Invoke-PerfRequest -Artifact $Artifacts[$name] -Err $err `
                    -Out "target/$Tag-legacy-warm-$name.png" -Report "target/$Tag-legacy-warm-$name.json"
                $report = Invoke-PerfRequest -Artifact $Artifacts[$name] -Err $err `
                    -Out "target/$Tag-legacy-r$round-$name.png" -Report "target/$Tag-legacy-r$round-$name.json"
            }
            finally {
                Stop-RenderServer $server
            }
            $perf = @($report.perf)[0]
            $records[$name] += [pscustomobject]@{
                Round = $round; Windows = @($perf.windows | ForEach-Object { [double]$_ })
                Dropped = @($perf.dropped | ForEach-Object { [double]$_ })
                Median = [double]$perf.median; Millis = [int]$report.millis
            }
            Write-Host ('  {0,-14} 中位 {1,7:N2} ms（n={2}）｜丢 {3}｜请求 {4:N1} s' -f `
                    $name, $perf.median, @($perf.windows).Count, (Format-Series @($perf.dropped | ForEach-Object { [double]$_ })), ($report.millis / 1000.0))
        }
    }
    $scan.Stop()
    Write-Host ''
    Write-Host '==== 旧协议汇总（单档冷启动 + 档间重启）===='
    Write-Host ('{0,-14} {1,9} {2,9} {3,7} {4,9}  {5}' -f '档', '合并中位', '误差棒±', 'n', 'min', '每轮中位')
    foreach ($name in $Scenes) {
        $all = @($records[$name] | ForEach-Object { $_.Windows } | ForEach-Object { $_ })
        $perRound = @($records[$name] | ForEach-Object { $_.Median })
        $bar = (($perRound | Measure-Object -Maximum).Maximum - ($perRound | Measure-Object -Minimum).Minimum) / 2.0
        Write-Host ('{0,-14} {1,9:N2} {2,9:N2} {3,7} {4,9:N2}  {5}' -f `
                $name, (Get-Median $all), $bar, $all.Count, ($all | Measure-Object -Minimum).Minimum, (Format-Series $perRound))
    }
    Write-Host ''
    Write-Host '==== 旧协议的第一窗口（每档冷启动后第一个请求丢的那几窗）===='
    foreach ($name in $Scenes) {
        foreach ($row in $records[$name]) {
            Write-Host ('{0,-14} 第 {1} 轮 丢弃 {2}｜干净 {3}' -f $name, $row.Round, (Format-Series $row.Dropped), (Format-Series $row.Windows))
        }
    }
    Write-Host ('==== 旧协议墙钟：{0:N1} s = {1:N2} min ====' -f $scan.Elapsed.TotalSeconds, ($scan.Elapsed.TotalSeconds / 60))
}

# ---------------------------------------------------------------------------
# A/B 正对照：同一对场景、只差 shader 成员键；A→B→A→B 逐字节复核
# ---------------------------------------------------------------------------
function Invoke-AbPhase {
    param([switch]$SkipBake)
    if ($Scenes.Count -ne 2) { throw "-Phase ab 要正好两档：Scenes[0] = A，Scenes[1] = B" }
    # ⚠ 次序不能反：`Get-Artifacts` 取的是**清单里的键**，而清单要重烘之后才指向新内容。
    # 先取路径再重烘 ⇒ 这一轮量的是上一份产物（实测踩过：改了 surface shader、键也换了，
    # 出图却一个像素没变 —— 因为整批场景路径还是旧的）。
    if ($Bake -and -not $SkipBake) { Invoke-Bake }
    $artifacts = Get-Artifacts
    $table = Assert-ShaderMembersAgree -Artifacts $artifacts -AllowMixed:$true
    Write-Host ''
    Write-ShaderTable -Table $table
    Write-Host '  （A/B 是"参数一模一样、只差 shader 成员键"的一对 —— 所以图的差别只能来自 shader 版本）'
    $log = "target/$Tag-ab.log"
    $err = "$log.err"
    $server = Start-RenderServer -Extra @() -Log $log -Err $err -Width $Width -Height $Height
    # ⚠ 这里必须是普通哈希表：`[ordered]` 的整数键会被当成**下标**（`$shots[1]` 越界），
    # 而这里的键就是 1/2/3/4。
    $shots = @{}
    $order = @($Scenes[0], $Scenes[1], $Scenes[0], $Scenes[1])
    try {
        # 热身用截图请求（这一路的服务没带 --fps，性能请求会被拒收）。
        $warmOnly = [ordered]@{}
        $warmOnly[$Scenes[0]] = $artifacts[$Scenes[0]]
        $null = Invoke-ShotsRequest -Artifacts $warmOnly -Err $err -Report "target/$Tag-ab-warm.json" -Prefix "target/$Tag-ab-warm-"
        for ($i = 1; $i -le $order.Count; $i++) {
            $name = $order[$i - 1]
            $single = [ordered]@{}
            $single[$name] = $artifacts[$name]
            $report = Invoke-ShotsRequest -Artifacts $single -Err $err `
                -Report "target/$Tag-ab$i.json" -Prefix "target/$Tag-ab$i-"
            $shot = @($report.shots)[0]
            $shots[$i] = [pscustomobject]@{ Index = $i; Name = $name; Bytes = $shot.bytes; Hash = $shot.sha256; Verdict = $shot.verdict }
            Write-Host ('  #{0} {1,-16} {2}  {3,9} B  {4}' -f $i, $name, $shot.sha256.Substring(0, 16), $shot.bytes, $shot.verdict)
        }
    }
    finally {
        Stop-RenderServer $server
    }
    Write-Host ''
    Write-Host '==== A/B 判据 ===='
    $ok = $true
    foreach ($pair in @(@(1, 3, $Scenes[0]), @(2, 4, $Scenes[1]))) {
        $a = $shots[$pair[0]]
        $b = $shots[$pair[1]]
        $same = $a.Hash -eq $b.Hash
        if (-not $same) { $ok = $false }
        Write-Host ('  {0,-16} #{1} vs #{2}：{3}' -f $pair[2], $pair[0], $pair[1], $(if ($same) { '逐字节相同 ✓' } else { '⚠ 不同' }))
    }
    Write-Host ('  {0} vs {1}（两版 shader 必须有差别）：{2}' -f $Scenes[0], $Scenes[1], $(if ($shots[1].Hash -ne $shots[2].Hash) { '不同 ✓' } else { '⚠ 逐字节相同 —— 版本切换没生效' }))
    if ($ok -and $shots[1].Hash -ne $shots[2].Hash) {
        Write-Host '  ⇒ A→B→A 复现成立：换回熟版画回同一张图，两版之间确有差别。'
    } else {
        Write-Host '  ⇒ ⚠ A/B 正对照没通过。'
    }
}

# 老几路（截图 / 老性能 / A/B）没有"配对"这回事，默认还是那 5 档；
# **只有新主路径**默认"改哪个场景就测哪个"。
if (-not $Sweep -and -not $PSBoundParameters.ContainsKey('Scenes') -and $Phase -ne 'stable') {
    $Scenes = $SweepScenes
}

# ★ `Get-Artifacts` **当场读清单**，所以它只许出现在"这条路的活已经重烘完"之后：
#   在顶上无条件取一次，就等于把**所有**相位的路径钉在重烘之前 —— 新加的场景这一轮
#   直接报"清单里没有这个节点"（实测踩过），改过内容的场景则安静地量上一份产物。
if ($Phase -eq 'stable') {
    Invoke-StablePhase
} elseif ($Phase -eq 'scene' -or $Phase -eq 'both') {
    if ($Phase -eq 'both' -and $Bake) { Invoke-Bake }
    $artifactsAll = Get-Artifacts
    $forScene = [ordered]@{}
    foreach ($name in $Scenes) { $forScene[$name] = $artifactsAll[$name] }
    if ($Phase -eq 'both') {
        $table = Assert-ShaderMembersAgree -Artifacts $forScene
        Write-Host ''
        Write-Host '==== 新旧协议同档对照：先旧协议（单档冷启动 + 档间重启），再新协议（一个服务不重启）===='
        Invoke-LegacyPhase -Artifacts $forScene -Table $table
        Write-Host ''
        Write-Host '==== 新协议（同一会话、紧接着旧协议）===='
        Invoke-ScenePhase -SkipBake
    } else {
        Invoke-ScenePhase
    }
} elseif ($Phase -eq 'legacy') {
    if ($Bake) { Invoke-Bake }
    $artifactsAll = Get-Artifacts
    $table = Assert-ShaderMembersAgree -Artifacts $artifactsAll
    Write-ShaderTable -Table $table
    Invoke-LegacyPhase -Artifacts $artifactsAll -Table $table
} elseif ($Phase -eq 'ab') {
    Invoke-AbPhase
} else {
    Invoke-ShotPhase
}
