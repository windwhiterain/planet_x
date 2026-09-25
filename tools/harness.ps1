<#
  渲染侧测试仪器的共用底座。dot-source 使用：

      . "$PSScriptRoot/harness.ps1"

  它只做两件事：

    1) **按图名 + 节点名**解析产物路径。渲染器吃的是内容，内容由产物决定
       （见 `docs/invariants.md`：产物格式是 ArtBundle，也是与渲染器之间唯一的接口）。
       命令行里出现手拼的哈希路径，就是把「同一张图的成员」这件事交给人手维护：
       哈希一改路径就静默过期，成员配错而宽高恰好相同则形状校验照样通过。
       这里改成从 `target/pcg/<图>/manifest.json` 按节点名取 `key`，再拼 CAS 路径。

    2) 起/停渲染服务，并把**被拒绝**与**管线编译失败**当硬失败。
       客户端退出码非 0、日志里出现 Refused / `后端断言失败`，立刻抛错。
       宿主同步建管线，所以坏管线**在收到请求那一刻**就回 `Refused` ⇒ 走的就是这条闸。

  3) **断言整批场景钉的是同一份 shader**（`Assert-ShaderMembersAgree`）。
      场景把 WGSL 钉在内容键上（见 `docs/renderer.md`）。只要有一档钉的是**旧** WGSL，
      换到那一档就会往槽里装新 shader ⇒ 那条管线打回重编
      ⇒ 出图/计时的头几个窗口里云壳根本没管线（图里没云、帧时间还偏高）。
      协议的前提是「整轮扫描只起一个服务、档间不重启」，所以这条必须在开跑前被断言，
      不一致就退出 —— 而不是先跑出一批被污染的读数、再靠"丢窗口"补救。

  「跳过」是判据的敌人：仪器拿不到数据时必须响，否则绿灯是假的。

  4) **后端只管子进程**：无条件写 `$env:WGPU_BACKEND` 会让任何 dot-source 过它的 shell
      里起的 viewer 都继承那个后端 —— 同一 exe/场景/shader 下 dx12 40.1 ms、
      vulkan 17.5 ms（2.3×），"云变慢"就是这么来的。现在起子进程时**现设现还原**。

  缺图、缺节点、产物不在、请求被拒、shader 不一致 —— 一律抛错，绝不静默跳过。

  起服务前还有一道**单例硬闸**（`Assert-NoOtherRenderServer`）：本机还有 `px_render*` 在跑、
  或 `target/render-server.json` 还在，就报错退出并打印占用者 —— 两个并列的测量循环会让
  双方的数据都作废，这条闸把它变成会失败的门，而不是靠人记得。

  计时仪器是宿主自己的 `px_render --spans <预热>,<测量>`（逐条 pass 的编码器级时间戳）。
  它**不是**帧循环采样：宿主按需渲染（一条请求画一帧就回话），要求"逐帧采样 / 丢窗 /
  等 K 帧"的请求会被服务端当场拒，拒词自己写着理由（`px_render/src/serve.rs`）。
#>
$ErrorActionPreference = 'Stop'
# 后端钉 vulkan，且**只作用于子进程**：写进当前 shell 会顺着 dot-source 污染整个会话。
$HarnessBackend = 'vulkan'

# 起子进程用的环境：`Start-Process -Environment` 是叠加式的，不动父进程。
function Get-HarnessChildEnv {
    return @{ WGPU_BACKEND = $HarnessBackend }
}

if (-not $Exe) { $Exe = 'target\debug\px_render.exe' }

$HarnessCacheRoot = 'target\pcg'
$HarnessLease = 'target\render-server.json'
# 「这一套仪器驱动的那支 exe」。⚠ 今天**没有调用方**（原来是给"改前那支"留的位），
# 但它是"这批读数是在哪支 exe 上取的"这句话的落点，所以照样对齐到新宿主，不许留死指针。
$HarnessExe = 'target\debug\px_render.exe'
# 硬失败模式。⚠ 逐条说清哪几个还活着：
#   Refused              活着 —— 新宿主对能力之外的请求回的就是 `Frame::Refused`（`serve.rs`）
#   后端断言失败          活着 —— `px_render/src/gpu.rs:14` 的 `BACKEND_ASSERT`
#   failed to process shader / 设备实例已经暂停 / 0x887A0005
#                        退休 —— 那两句属于旧的资产装载与 DXGI 那一档；今天锁死 Vulkan，也没有
#                        那一层。留着匹配不到，但**不许**把它们当成"门还在"的证据：真正拦住
#                        坏管线的是客户端退出码那一条。
$HarnessFailPattern = 'Refused|后端断言失败|管线编译失败'

function Get-GraphManifest {
    param([string]$Graph)
    $path = Join-Path $HarnessCacheRoot "$Graph\manifest.json"
    if (-not (Test-Path $path)) {
        throw "图 '$Graph' 的清单不在：$path`n  先烘：cargo run -p px_graphs --bin $Graph"
    }
    $entries = @(Get-Content $path -Raw | ConvertFrom-Json)
    if ($entries.Count -eq 0) { throw "图 '$Graph' 的清单是空的：$path" }
    return $entries
}

function Resolve-Artifact {
    param([string]$Graph, [string]$Node)
    $entries = Get-GraphManifest -Graph $Graph
    $entry = @($entries | Where-Object { $_.node -eq $Node })
    if ($entry.Count -eq 0) {
        $known = (@($entries | ForEach-Object { $_.node }) -join ' / ')
        throw "图 '$Graph' 里没有节点 '$Node'；这份清单有的节点：$known"
    }
    $key = $entry[0].key
    if ($key.Length -ne 64) { throw "图 '$Graph' 节点 '$Node' 的 key 不是 64 位十六进制：'$key'" }
    $path = Join-Path $HarnessCacheRoot ("ab\{0}\{1}.pxart" -f $key.Substring(0, 2), $key)
    if (-not (Test-Path $path)) {
        throw "图 '$Graph' 节点 '$Node' 的产物不在：$path`n  key $key（清单说它在）—— 重烘一次，或清单与 CAS 不同步"
    }
    return [pscustomobject]@{
        Graph   = $Graph
        Node    = $Node
        Op      = $entry[0].op
        Version = $entry[0].op_version
        Key     = $key
        Path    = (Resolve-Path $path).Path
        Bytes   = $entry[0].bytes
    }
}

function Format-Artifact {
    param($Artifact)
    return ('{0}/{1}  {2}@v{3}  {4}  {5:N1} KB' -f `
            $Artifact.Graph, $Artifact.Node, $Artifact.Op, $Artifact.Version, `
            $Artifact.Key.Substring(0, 12), ($Artifact.Bytes / 1024))
}

function Assert-NoRenderFailure {
    param([string]$Err)
    if (-not (Test-Path $Err)) { return }
    $bad = @(Select-String -Path $Err -Pattern $HarnessFailPattern -ErrorAction SilentlyContinue)
    if ($bad.Count -gt 0) { throw "渲染失败（fail-fast）：$($bad[0].Line.Trim())" }
}

# ---------------------------------------------------------------------------
# .pxart 里的场景帧：读出「这一档钉的是哪一份 shader」
# ---------------------------------------------------------------------------

# 产物是 `PXST` 头 + 一串 [u32 长度][payload]，payload 首字节 'J' = JSON 帧、'B' = 二进制帧。
# 只解 J 帧：场景产物里本来就没有 blob，但混装时不能因为一个 B 帧就把长度表读错位。
function Get-FramePayloads {
    param([string]$Path)
    $bytes = [System.IO.File]::ReadAllBytes($Path)
    if ($bytes.Length -lt 8) { throw "产物太短，不成流：$Path" }
    if ([System.Text.Encoding]::ASCII.GetString($bytes, 0, 4) -ne 'PXST') {
        throw "不是 PXST 流：$Path"
    }
    $version = [BitConverter]::ToUInt32($bytes, 4)
    if ($version -ne 1) { throw "流版本不认（$version），不知道长度表怎么读：$Path" }
    $frames = @()
    $offset = 8
    while ($offset -lt $bytes.Length) {
        if ($offset + 4 -gt $bytes.Length) { throw "长度表被截断：$Path" }
        $len = [BitConverter]::ToUInt32($bytes, $offset)
        $offset += 4
        if ($len -lt 1 -or $offset + $len -gt $bytes.Length) {
            throw "帧长度越界（$len），产物坏了：$Path"
        }
        $head = [char]$bytes[$offset]
        if ($head -eq 'J') {
            $text = [System.Text.Encoding]::UTF8.GetString($bytes, $offset + 1, $len - 1)
            $frames += , ($text | ConvertFrom-Json)
        }
        $offset += $len
    }
    return $frames
}

# 这份场景产物里每个「物体/角色」钉的 shader 成员（角色一般是 shader，键 = 那一份 WGSL 的内容键）。
#
# ⚠ 读的是**通用渲染文档**（`px_protocol::scene` v2，`docs/render/renderer.md` 65）：
# 每个物体的 `material.shader` 是它自己那一份 WGSL。v1 的 `parts[]` 已经没有这个形状了 ——
# 按老形状读会得到一张**空表**，而空表在这里等于"没有不一致"⇒ 闸门静默失效（比报错更坏）。
# 所以下面这种"什么都没读到"要当场抛错，不能返回空表。
function Get-SceneShaderMembers {
    param([string]$Path)
    $scene = @(Get-FramePayloads -Path $Path | Where-Object { $_.frame -eq 'scene' })
    if ($scene.Count -eq 0) { throw "产物里没有场景帧（Scene）：$Path" }
    $document = $scene[0]
    if (-not $document.PSObject.Properties.Name.Contains('objects')) {
        throw "场景帧不像通用渲染文档（缺 objects）：$Path  schema=$($document.schema) parts=$($document.PSObject.Properties.Name -contains 'parts')"
    }
    $members = [ordered]@{}
    foreach ($object in $document.objects) {
        $shader = $object.material.shader
        if ($shader.graph -eq 'shaders') {
            $members["$($object.id)/shader"] = [pscustomobject]@{
                Slot = "$($shader.graph)/$($shader.node)"
                Key  = $shader.key
            }
        }
    }
    if ($members.Count -eq 0) {
        throw "这份场景一个 shader 成员都没钉（objects=$($document.objects.Count)）：$Path"
    }
    return $members
}

# 同一份 WGSL 只能有一个内容键：**按槽分组**比对，任一组出现两个键就是硬失败。
# 注意不能要求"每份场景的成员集合一样" —— orbit-bare 本来就没有 clouds part。
#
# `-AllowMixed` 是给"故意交错两版 shader"那个实验留的口子：那时候不一致正是自变量。
# 开了它这里只**大声报到**、不拦；默认（不写）仍然是硬失败 ——
# 常规扫描要的是"一个服务 + 一份 WGSL"，混着量出来的数没有意义。
function Assert-ShaderMembersAgree {
    param($Artifacts, [switch]$AllowMixed)
    $table = [ordered]@{}
    foreach ($name in $Artifacts.Keys) {
        $table[$name] = Get-SceneShaderMembers -Path $Artifacts[$name].Path
    }
    $slots = [ordered]@{}
    foreach ($name in $table.Keys) {
        foreach ($role in $table[$name].Keys) {
            $entry = $table[$name][$role]
            if (-not $slots.Contains($entry.Slot)) { $slots[$entry.Slot] = @() }
            $slots[$entry.Slot] += [pscustomobject]@{
                Scene = $name
                Role  = $role
                Key   = $entry.Key
            }
        }
    }
    if ($slots.Count -eq 0) {
        throw '这批场景里一个 shader 成员都没有：产物不成形，别拿它当测量对象'
    }
    foreach ($slot in $slots.Keys) {
        $keys = @($slots[$slot] | ForEach-Object { $_.Key } | Select-Object -Unique)
        if ($keys.Count -le 1) { continue }
        $lines = @($slots[$slot] | ForEach-Object {
                '    {0,-14} {1,-14} {2}' -f $_.Scene, $_.Role, $_.Key
            })
        $report = @"
场景钉的 shader 不是同一份（槽 $slot 出现了 $($keys.Count) 个内容键）：
$($lines -join "`n")
  ⇒ 换到键不同的那一档会装新 shader、把管线打回重编，计时窗口与判据图都会被污染。
  ⇒ 先统一重烘：cargo run -p px_graphs --bin shaders；再逐个 cargo run -p px_graphs --bin scene <名>。
"@
        if ($AllowMixed) {
            Write-Host '⚠ 混版被**显式放行**（-AllowMixedShaders）：这批数只在「交错两版」那个实验里算数'
            Write-Host $report
        } else {
            throw $report
        }
    }
    return $table
}

function Format-ShaderMembers {
    param($Table)
    foreach ($name in $Table.Keys) {
        $parts = @($Table[$name].Keys | ForEach-Object { '{0}@{1}' -f $Table[$name][$_].Slot, $Table[$name][$_].Key.Substring(0, 12) })
        if ($parts.Count -eq 0) { $parts = @('（没有 shader 成员）') }
        '{0,-14} {1}' -f $name, ($parts -join '  ')
    }
}

# ---------------------------------------------------------------------------
# 单例闸：渲染服务是单例（第二个 --serve 覆写租约、把前一个逼死）
# ---------------------------------------------------------------------------

# 两个并列的测量循环会让双方的数据都作废，还可能把别人的服务打死。
# 所以起服务前先把"本机还有没有别的 px_render"变成一个**会失败的门**，而不是靠人记得。
function Assert-NoOtherRenderServer {
    # 按名字前缀匹配，不按绝对路径：比对的"改前那支 exe"（target\px_render-before-slots.exe）
    # 进程名就不是 px_render —— 只认精确名字会漏掉它，闸就形同虚设。
    $alive = @(Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Name -like 'px_render*' })
    if ($alive.Count -gt 0) {
        $lines = @($alive | ForEach-Object {
                $cmd = (Get-CimInstance Win32_Process -Filter "ProcessId=$($_.Id)" -ErrorAction SilentlyContinue).CommandLine
                '    pid {0}  启动 {1}  {2}' -f $_.Id, $_.StartTime.ToString('HH:mm:ss'), $cmd
            })
        throw "本机还有 $($alive.Count) 个 px_render 在跑，先让它们停（不要杀，等对方自己收）：`n$($lines -join "`n")"
    }
    if (Test-Path $HarnessLease) {
        $lease = Get-Content $HarnessLease -Raw | ConvertFrom-Json
        throw "租约还在：$HarnessLease（pid $($lease.pid)）—— 进程列表里没有它，但文件没清，说明上一个会话没走 Stop-RenderServer；确认后删掉这份租约再跑"
    }
}

function Start-RenderServer {
    param(
        [string[]]$Extra = @(),
        [string]$Log,
        [string]$Err,
        [int]$Width = 960,
        [int]$Height = 640,
        [int]$TimeoutSeconds = 180
    )
    # 起之前先过单例闸：有别人在跑就报错退出，**不替别人收尸**。
    Assert-NoOtherRenderServer
    Remove-Item $Log, $Err -ErrorAction SilentlyContinue
    $arguments = @('--serve', '--width', "$Width", '--height', "$Height") + $Extra
    # -NoNewWindow：否则每起一个服务都弹一个控制台窗口、抢焦点。
    # -Environment：后端只在**这个**子进程里是 vulkan，当前 shell 不留痕。
    $proc = Start-Process -FilePath $Exe -ArgumentList $arguments -NoNewWindow `
        -Environment (Get-HarnessChildEnv) `
        -RedirectStandardOutput $Log -RedirectStandardError $Err -PassThru
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        Assert-NoRenderFailure $Err
        if ((Test-Path $Log) -and
            (@(Select-String -Path $Log -Pattern '渲染管线全部就绪' -ErrorAction SilentlyContinue).Count -ge 1)) {
            return $proc
        }
        Start-Sleep -Milliseconds 250
    }
    Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
    throw "服务没在 $TimeoutSeconds s 内就绪：看 $Log 与 $Err"
}

# 只按**我拿到的那个 pid** 停：收尾不许按名字扫、不许碰别人的进程。
# 租约里的 pid 只有在它等于我起的那个时才动 —— 易主说明有别人接管了，那时候删租约就是替别人擦桌子。
function Stop-RenderServer {
    param($Proc)
    $killed = @()
    if ($Proc) {
        Stop-Process -Id $Proc.Id -Force -ErrorAction SilentlyContinue
        $killed += $Proc.Id
    }
    if (Test-Path $HarnessLease) {
        $leasePid = (Get-Content $HarnessLease -Raw | ConvertFrom-Json).pid
        if ($Proc -and $leasePid -eq $Proc.Id) {
            Remove-Item $HarnessLease -ErrorAction SilentlyContinue
        } elseif (-not $Proc) {
            Stop-Process -Id $leasePid -Force -ErrorAction SilentlyContinue
            $killed += $leasePid
            Remove-Item $HarnessLease -ErrorAction SilentlyContinue
        } else {
            Write-Host "⚠ 租约已易主（pid $leasePid ≠ 我起的 $($Proc.Id)），不动它"
        }
    }
    # 等它真的消失再返回：旧协议每轮都要重启，不等就会撞上自己的单例闸。
    for ($i = 0; $i -lt 40; $i++) {
        $alive = @($killed | Where-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue })
        if ($alive.Count -eq 0) { break }
        Start-Sleep -Milliseconds 250
    }
    Start-Sleep -Milliseconds 300
}

# 显式收尸：只在**人确认过**它是我自己上一轮留下的服务时用（`-Recover`）。
# 不再被 Start-RenderServer 自动调用 —— 自动收尸正是「我把别人的测量打死」那条路。
function Stop-StrayServer {
    param([switch]$Confirm)
    if (-not (Test-Path $HarnessLease)) {
        Write-Host '没有租约，没有要收的'
        return
    }
    $lease = Get-Content $HarnessLease -Raw | ConvertFrom-Json
    if (-not $Confirm) {
        throw "租约在：$HarnessLease（pid $($lease.pid)）。收尸要显式确认：Stop-StrayServer -Confirm"
    }
    Write-Host "按租约停 pid $($lease.pid)（$($lease.exe)）"
    Stop-Process -Id $lease.pid -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 800
    Remove-Item $HarnessLease -ErrorAction SilentlyContinue
}

function Invoke-Client {
    param([string[]]$Arguments, [string]$Err, [string]$What = '请求')
    # 后端只在这条子进程存活期间设，finally 里还原：当前 shell 干净，别的会话不受影响。
    $previous = $env:WGPU_BACKEND
    $env:WGPU_BACKEND = $HarnessBackend
    try {
        & $Exe @Arguments | Out-Null
        $code = $LASTEXITCODE
    } finally {
        $env:WGPU_BACKEND = $previous
    }
    if ($code -ne 0) {
        throw "$What 失败（客户端退出码 $code）：px_render $($Arguments -join ' ')"
    }
    Assert-NoRenderFailure $Err
}
