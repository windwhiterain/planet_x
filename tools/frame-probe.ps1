param(
    [string]$Field = "target/pcg/ab/ad/ad7a18ea349db3e259d429f9bdb72711bec1f714207827238a5456767f239517.pxart",
    [string]$Mesh = "target/pcg/ab/6d/6df28459c385cf8e0f05f00ae8be6ad9b436b42b619e6f34e610f0919825c532.pxart",
    [string]$Cover = "target/pcg/ab/81/81d3d21d406121f73be974a566778e3947365124fc30191674b9f899d6a315d5.pxart",
    [string]$Slope = "target/pcg/ab/af/afc8c41a477401a5fcba4ac81e4e632dcc951ebaf12981061050c3b79943167b.pxart,target/pcg/ab/a7/a7aa0d14169157f6cb6b4e97b98f528660db139a2703ef49ac42ffac51346476.pxart,target/pcg/ab/4e/4e9571d1ba536d1a41e3517dd303238f7eb6856ec594696feb64a7b92ba995d5.pxart",
    [int]$Width = 2240,
    [int]$Height = 1400,
    [int]$Samples = 4,
    [string[]]$Cases = @("nocloud", "volume", "surface", "normals")
)

$ErrorActionPreference = "Stop"
$exe = "target\debug\px_render.exe"
$env:WGPU_BACKEND = "dx12"

function Wait-For {
    param([string]$Path, [string]$Pattern, [int]$Least, [int]$TimeoutSeconds)
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        if (Test-Path $Path) {
            $hits = @(Select-String -Path $Path -Pattern $Pattern -ErrorAction SilentlyContinue)
            if ($hits.Count -ge $Least) { return $hits }
        }
        Start-Sleep -Milliseconds 250
    }
    return @()
}

foreach ($case in $Cases) {
    $log = "target/fp-$case.log"
    Remove-Item $log, "$log.err" -ErrorAction SilentlyContinue
    Remove-Item target\render-server.json -ErrorAction SilentlyContinue

    $serverArgs = @("--serve", "--fps", "--width", $Width, "--height", $Height)
    if ($case -eq "sun" -or $case -eq "noise" -or $case -eq "fetch" -or $case -eq "detail") {
        $serverArgs += @("--cloud-ablate", $case)
    } elseif ($case -eq "surface" -or $case -eq "normals") {
        $serverArgs += @("--cloud-ablate", $case)
    }
    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    $server = Start-Process -FilePath $exe -ArgumentList $serverArgs -RedirectStandardOutput $log -RedirectStandardError "$log.err" -PassThru

    $ready = Wait-For -Path $log -Pattern "渲染管线全部就绪" -Least 1 -TimeoutSeconds 60
    if ($ready.Count -eq 0) { Write-Output "$case : 管线没就绪"; Stop-Process -Id $server.Id -Force; continue }

    $cli = @("--planet", $Field, "--mesh", $Mesh, "--palette", "rocky", "--width", $Width, "--height", $Height, "--out", "target/fp-$case.png")
    if ($case -ne "nocloud") { $cli += @("--clouds", $Cover, "--cloud-slope", $Slope) }
    & $exe @cli | Out-Null

    $lines = Wait-For -Path $log -Pattern "帧时间" -Least $Samples -TimeoutSeconds 90
    $watch.Stop()
    $all = @($lines | ForEach-Object { [double](($_.Line -replace "^.*帧时间 ", "") -replace " ms.*$", "") })
    $best = if ($all.Count -gt 0) { ($all | Measure-Object -Minimum).Minimum } else { 0 }
    $spread = if ($all.Count -gt 0) { ($all | Measure-Object -Maximum).Maximum - $best } else { 0 }
    Write-Output ("{0,-8} 最小 {1,6:N2} ms   抖动 {2,5:N2} ms   [{3:N1}s]  各窗口: {4}" -f `
        $case, $best, $spread, $watch.Elapsed.TotalSeconds, (($all | ForEach-Object { "{0:N2}" -f $_ }) -join " | "))
    Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 400
}
