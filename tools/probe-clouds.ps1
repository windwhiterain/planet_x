<#
.SYNOPSIS
  云的多视角出图探针：几份场景 × 几个相机，看图不看数。

.DESCRIPTION
  与 frame-probe.ps1 共用 tools/harness.ps1：场景按**图名/节点名**从
  `target/pcg/<图>/manifest.json` 解析，缺图缺节点就抛错；渲染器只收一个 `--scene`。

  **消融档是场景的变体，不是开关**：`ablate = "surface"` 写在 clouds part 的参数里，
  所以这里收到的是一串场景节点名（`orbit` / `orbit-bare` / `orbit-surface` …），
  而不是一串档名。要量哪一档就先照 `art/scene/orbit-surface.toml` 那样烘一份。

  服务就绪读日志的「渲染管线全部就绪」，不再 Sleep 15 硬等；
  客户端退出码与 Refused 都当硬失败。停服务按租约 pid，不做全局杀。

  帧时间归 frame-probe.ps1，这里只出图（判据是眼睛 + 并排比）。

.EXAMPLE
  .\tools\probe-clouds.ps1 -Scenes orbit,orbit-bare,orbit-surface
#>
param(
    [string]$Graph = 'scene',
    [string[]]$Scenes = @('orbit', 'orbit-bare', 'orbit-surface', 'orbit-proxy'),
    [int]$Size = 1100,
    [string]$Out = 'target/probe-clouds'
)

. "$PSScriptRoot\harness.ps1"
New-Item -ItemType Directory -Force -Path $Out | Out-Null

$cams = @(
    '0.90,0.25,2.10',
    '1.60,0.25,2.10',
    '2.40,0.25,2.10',
    '0.90,0.60,2.10'
)

foreach ($name in $Scenes) {
    $sceneArtifact = Resolve-Artifact -Graph $Graph -Node $name
    Write-Host "==== $name（全部来自产物）===="
    Format-Artifact $sceneArtifact
    $scene = @('--scene', $sceneArtifact.Path, '--pcg-root', $HarnessCacheRoot)

    $log = "$Out/$name.log"
    $err = "$log.err"
    Write-Host "---- 起服务（不吃任何内容旗标）----"
    $server = Start-RenderServer -Extra @() -Log $log -Err $err -Width $Size -Height $Size
    try {
        $index = 0
        foreach ($cam in $cams) {
            $shot = "$Out/$name-$index.png"
            Invoke-Client -Err $err -What "出图 $name-$index" -Arguments ($scene + @(
                    '--width', "$Size", '--height', "$Size", '--cam', $cam, '--out', $shot))
            if (-not (Test-Path $shot)) { throw "没出图：$shot" }
            Write-Host "  拍好 $name-$index（$cam）"
            $index++
        }
    }
    finally {
        Stop-RenderServer $server
    }
    Write-Host "$name：$($cams.Count) 张 -> $Out/$name-*.png"
}
Write-Host 'probe 完成'
