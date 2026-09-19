param(
    [string]$Graph = "scene",
    [string]$Scene = "orbit",
    [string]$Out = "target/probe-sheet.png",
    [double]$Distance = 3.15,
    [int]$Width = 480,
    [int]$Height = 320,
    [string]$Exe = "target/debug/px_render.exe"
)

$ErrorActionPreference = "Stop"
. "$PSScriptRoot/harness.ps1"

# ⚠ S8-a：`$Exe` 的缺省值换成了 `px_render.exe`（bevy 宿主已删）。
#   这一路**不需要**改形状：它只做"客户端请求 + 拼一张对照图"，而新宿主的客户端语义
#   与 bevy 宿主**逐字同源**（§147.2：`request_once` 是照搬的）。
#   ⚠ 与 `frame-probe.ps1` 那两条**退休**的路不同：那两条要的是计时用的帧循环，
#   新宿主当场拒；这一路出的是图，出图那条路是成立的。

$sceneArtifact = Resolve-Artifact -Graph $Graph -Node $Scene
Write-Host "对照图的场景：$(Format-Artifact $sceneArtifact)"

$tilt = 0.34

function AimLocal([double[]]$local) {
    $y = $local[1] * [Math]::Cos($tilt) - $local[2] * [Math]::Sin($tilt)
    $z = $local[1] * [Math]::Sin($tilt) + $local[2] * [Math]::Cos($tilt)
    $x = $local[0]
    $length = [Math]::Sqrt($x * $x + $y * $y + $z * $z)
    @{
        Yaw = [Math]::Atan2($x, $z) * 180 / [Math]::PI
        Pitch = [Math]::Asin($y / $length) * 180 / [Math]::PI
    }
}

$cornerTop = AimLocal @(1, 1, 1)
$cornerBottom = AimLocal @(1, -1, 1)
$edgeMiddle = AimLocal @(1, 1, 0)
$faceCentre = AimLocal @(0, 0, 1)

$views = @(
    @{ Yaw = 0;   Pitch = 8;   Tag = "equator-000" },
    @{ Yaw = 90;  Pitch = 8;   Tag = "equator-090" },
    @{ Yaw = 180; Pitch = 8;   Tag = "equator-180" },
    @{ Yaw = 270; Pitch = 8;   Tag = "equator-270" },
    @{ Yaw = 0;   Pitch = 45;  Tag = "north-045" },
    @{ Yaw = 0;   Pitch = -45; Tag = "south-045" },
    @{ Yaw = 0;   Pitch = 82;  Tag = "north-pole" },
    @{ Yaw = 0;   Pitch = -82; Tag = "south-pole" },
    @{ Yaw = $cornerTop.Yaw;    Pitch = $cornerTop.Pitch;    Tag = "cube-corner-top";    Distance = 1.40 },
    @{ Yaw = $cornerBottom.Yaw; Pitch = $cornerBottom.Pitch; Tag = "cube-corner-bottom"; Distance = 1.40 },
    @{ Yaw = $edgeMiddle.Yaw;   Pitch = $edgeMiddle.Pitch;   Tag = "cube-edge-middle";   Distance = 1.40 },
    @{ Yaw = $faceCentre.Yaw;   Pitch = $faceCentre.Pitch;   Tag = "cube-face-centre";   Distance = 1.40 }
)

$shotDir = Join-Path "target" "probe"
New-Item -ItemType Directory -Force -Path $shotDir | Out-Null
Remove-Item "$shotDir\*.png" -ErrorAction SilentlyContinue

foreach ($view in $views) {
    $shot = Join-Path $shotDir "$($view.Tag).png"
    $arguments = @(
        "--scene", $sceneArtifact.Path,
        "--pcg-root", $HarnessCacheRoot,
        "--cam", "$($view.Yaw),$($view.Pitch),$(if ($view.Distance) { $view.Distance } else { $Distance })",
        "--width", "$Width",
        "--height", "$Height",
        "--out", $shot
    )
    # 走 Invoke-Client：后端只对这条子进程生效，且非 0 退出码当硬失败。
    Invoke-Client -Arguments $arguments -What "拍 $($view.Tag)"
    if (-not (Test-Path $shot)) {
        throw "角度 $($view.Tag) 没出图"
    }
    Write-Host "  拍好 $($view.Tag)"
}

Add-Type -AssemblyName System.Drawing
$columns = 4
$rows = [Math]::Ceiling($views.Count / $columns)
$sheet = New-Object System.Drawing.Bitmap ($columns * $Width), ($rows * $Height)
$canvas = [System.Drawing.Graphics]::FromImage($sheet)
$canvas.Clear([System.Drawing.Color]::Black)
$font = New-Object System.Drawing.Font "Consolas", 14

for ($index = 0; $index -lt $views.Count; $index++) {
    $view = $views[$index]
    $shot = Join-Path $shotDir "$($view.Tag).png"
    $image = [System.Drawing.Image]::FromFile((Resolve-Path $shot))
    $x = ($index % $columns) * $Width
    $y = [Math]::Floor($index / $columns) * $Height
    $canvas.DrawImage($image, $x, $y, $Width, $Height)
    $canvas.DrawString($view.Tag, $font, [System.Drawing.Brushes]::Yellow, $x + 8, $y + 6)
    $image.Dispose()
}

$canvas.Dispose()
$sheet.Save((Join-Path (Get-Location) $Out), [System.Drawing.Imaging.ImageFormat]::Png)
$sheet.Dispose()

Write-Host "对照图：$Out（$($views.Count) 个角度，场景 $(Format-Artifact $sceneArtifact)）"


