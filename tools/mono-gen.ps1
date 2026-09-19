# **生成 + 编译**一个单态化实例，并把它放进装载目录。
# 
# 为什么需要它：`bake<F: FieldFn>` 的实例必须与「场函数那个类型」在**同一个编译单元**
# 里生成，而图程序是 `bin` ⇒ 实例只能落在**图侧自建的 dylib** 里。这个工具就是那一级。
# 
# 它做的事（每一步都必要）：
# 1. **算 `mono_key`** = 场函数体 + 泛型体 + 模板接线的内容哈希；
# 2. **写进内容寻址的路径** `target/mono/<mono_key>/`。
# ⚠ 这一步是「缓存真的命中」的唯一开关：cargo 的指纹是**路径 + mtime**，
# 固定路径 ⇒ 每次都判"变了" ⇒ 永远重编，缓存等于没有；
# 3. `cargo build` 那个目录（它不在 workspace members 里，自己一个 target）；
# 4. 把 cdylib 复制成 `<file stem>_op.dll` 放进 `target/debug/`
# —— 入口名由库名派生（`px_graph_schema::op::table_symbol`），驱动一行不用改。
# 
# 用法：`pwsh tools/mono-gen.ps1`

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$template = Join-Path $PSScriptRoot 'mono-template'

# ── 1) 身份：哪些文件的内容决定这一份实例 ────────────────────────────────────
$ingredients = @(
    'tools/mono-template/src/fields.rs',
    'tools/mono-template/src/lib.rs',
    'tools/mono-template/Cargo.toml',
    'px_cook/src/field_fn.rs',
    'px_cook/src/lib.rs',
    'px_volume_schema/src/volume.rs',
    'px_volume_schema/src/params.rs',
    'px_volume_schema/src/payload.rs',
    'px_field_schema/src/field.rs',
    'px_verify/src/cloud_field.rs',
    'px_verify/src/proxy.rs',
    'px_volume_op/src/lib.rs'
)

$hash = [System.Security.Cryptography.SHA256]::Create()
$ingredientBytes = New-Object System.Collections.Generic.List[byte]
foreach ($relative in $ingredients) {
    $full = Join-Path $root $relative
    if (-not (Test-Path $full)) { throw "配料缺一份：$relative" }
    # 长度前缀挡住「拼起来一样」的两种切法（与 px_graph_schema::identity::fnv1a_sources 同一条口径）
    $bytes = [System.IO.File]::ReadAllBytes($full)
    $length = [BitConverter]::GetBytes([int64]$bytes.Length)
    $ingredientBytes.AddRange($length)
    $ingredientBytes.AddRange($bytes)
}
$monoKey = ([BitConverter]::ToString($hash.ComputeHash($ingredientBytes.ToArray())) -replace '-', '').Substring(0, 16).ToLower()
Write-Host "mono_key = $monoKey（来自 $($ingredients.Count) 份配料）"

# ── 2) 内容寻址的落点 ────────────────────────────────────────────────────────
$monoRoot = Join-Path $root "target/mono/$monoKey"
$crateDir = Join-Path (Join-Path $root 'target/mono') 'crate'
New-Item -ItemType Directory -Force -Path (Join-Path $crateDir 'src') | Out-Null
Copy-Item (Join-Path $template 'Cargo.toml') (Join-Path $crateDir 'Cargo.toml') -Force
Copy-Item (Join-Path $template 'src/lib.rs') (Join-Path $crateDir 'src/lib.rs') -Force
Copy-Item (Join-Path $template 'src/fields.rs') (Join-Path $crateDir 'src/fields.rs') -Force

# ── 3) 编（独立 target：不跟主 workspace 抢锁，也不污染它的增量缓存）───────────
# ⚠ 两条都试过，都错，记在这里：
#
# ① **共用主 workspace 的 `target/`** → 回归：生成 crate 的依赖图与主 workspace 不同
#    （它自己一份 `Cargo.lock`、自己的特征统一）⇒ cargo 把 `px_volume_op.dll` 等**再编一份**
#    进同一个 `target/debug/deps/` 覆盖主 workspace 那份 ⇒ `px_field_op.dll` 装载
#    `LoadLibraryExW failed`（实测复现）。
# ② **target 也按 mono_key 分** → 每次改一行场函数都在一个**新目录**里从零编依赖
#    （实测 20.8 s/次），比不带这一级还差。
#
# ⇒ 正确的分法是：**构建目录固定**（`target/mono/build`），让依赖编一次、之后复用；
#   而"这一份实例是谁"由**内容**决定（`SOURCE_HASH` 覆盖 `fields.rs`，键/描述符里都带着它）。
#   ⚠ 代价：源码目录也必须是固定的（`cargo` 的指纹是路径 + mtime）——
#   所以 `target/mono/crate` 这个位置**不代表身份**，它只是"当前那一份"的落点。
#   想同时要"内容寻址的源码路径"与"复用的依赖"，就得自己管依赖产物的存放 —— 见笔记 §166。
$targetDir = Join-Path (Join-Path $root 'target/mono') 'build'
Write-Host "cargo build → $crateDir（构建目录固定 $targetDir）"
& cargo build --manifest-path (Join-Path $crateDir 'Cargo.toml') --target-dir $targetDir
if ($LASTEXITCODE -ne 0) { throw "编单态化实例失败（exit $LASTEXITCODE）" }

# ── 4) 放进装载目录，让现有扫描接上 ──────────────────────────────────────────
$dll = Join-Path $targetDir 'debug/px_mono_clouds.dll'
if (-not (Test-Path $dll)) { throw "没找到 $dll" }
$deposit = Join-Path $root 'target/debug/px_mono_clouds_op.dll'
Copy-Item $dll $deposit -Force
Write-Host "装入 $deposit（$([math]::Round((Get-Item $deposit).Length / 1MB, 2)) MB）"

# ⚠ 这一份 dylib 在**运行时**还要它自己的上游 DLL（`px_volume_op` / `px_cook` …）——
# 那是 `dylib` 这个 crate-type 的 ABI 决定的（Rust 没有稳定 ABI，不能只带一份）。
# ⚠⚠ **不要把它们拷到 `target/debug/`**：那个目录里的同名 DLL 属于**主 workspace 的构建图**，
# 两个构建图（各自一份 Cargo.lock / 特征统一）的产物**不能互换** —— 实测一拷就
# `LoadLibraryExW failed`。正确做法是让**装载那一侧**把这份构建目录放进搜索路径：
#
#     $env:PATH = "$root	arget\monouild\debug;$env:PATH"
#
# （Windows 的 DLL 搜索顺序里 `PATH` 就在 exe 目录之后。）
Write-Host "⚠ 跑用它的图时，要把这一份构建目录放进 PATH："
Write-Host "    `$env:PATH = `"$targetDir\debug;`$env:PATH`""
