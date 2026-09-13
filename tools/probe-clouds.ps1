param(
    [string]$Field = "target/pcg/ab/ad/ad7a18ea349db3e259d429f9bdb72711bec1f714207827238a5456767f239517.pxart",
    [string]$Mesh = "target/pcg/ab/6d/6df28459c385cf8e0f05f00ae8be6ad9b436b42b619e6f34e610f0919825c532.pxart",
    [string]$Cover = "target/pcg/ab/81/81d3d21d406121f73be974a566778e3947365124fc30191674b9f899d6a315d5.pxart",
    [string]$Slope = "target/pcg/ab/af/afc8c41a477401a5fcba4ac81e4e632dcc951ebaf12981061050c3b79943167b.pxart,target/pcg/ab/a7/a7aa0d14169157f6cb6b4e97b98f528660db139a2703ef49ac42ffac51346476.pxart,target/pcg/ab/4e/4e9571d1ba536d1a41e3517dd303238f7eb6856ec594696feb64a7b92ba995d5.pxart",
    [int]$Size = 1100,
    [string]$Out = "target/probe-clouds"
)

$ErrorActionPreference = "Stop"
$exe = "target\debug\px_render.exe"
$env:WGPU_BACKEND = "dx12"
New-Item -ItemType Directory -Force -Path $Out | Out-Null

$cams = @(
    "0.90,0.25,2.10",
    "1.60,0.25,2.10",
    "2.40,0.25,2.10",
    "0.90,0.60,2.10"
)
$modes = @(
    @{ Name = "volume";  Args = @() },
    @{ Name = "surface"; Args = @("--cloud-ablate", "surface") },
    @{ Name = "normals"; Args = @("--cloud-ablate", "normals") }
)

foreach ($mode in $modes) {
    Get-Process -Name px_render -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Milliseconds 700
    Remove-Item target\render-server.json -ErrorAction SilentlyContinue
    $server = Start-Process -FilePath $exe -ArgumentList (@("--serve", "--width", $Size, "--height", $Size) + $mode.Args) `
        -RedirectStandardOutput "$Out/$($mode.Name).log" -RedirectStandardError "$Out/$($mode.Name).err" -PassThru
    Start-Sleep -Seconds 15
    $index = 0
    foreach ($cam in $cams) {
        & $exe --planet $Field --mesh $Mesh --palette rocky --clouds $Cover --cloud-slope $Slope `
            --width $Size --height $Size --cam $cam --out "$Out/$($mode.Name)-$index.png" | Out-Null
        $index++
    }
    Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 700
    Write-Output "$($mode.Name): $($cams.Count) 张 -> $Out/$($mode.Name)-*.png"
}
Get-Process -Name px_render -ErrorAction SilentlyContinue | Stop-Process -Force
Write-Output "probe 完成"
