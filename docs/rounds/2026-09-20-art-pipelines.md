# 22 · 星球美术管线第二轮：陨坑算子、纬向条带、次表面散射（2026-09-20）

这一轮的主线是**用 graph 造更多程序化星球**，副线是**在这一路上把 graph 系统的问题挖出来修掉**。
两个新天体：`moon`（陨坑月球）与 `gasgiant`（气态巨行星）。四条系统问题（三条是这一轮踩出来的）。

分支：`feature/planet-art-graph-pipelines`，worktree `.worktrees/planet-art`，从 `v2` tip `f99b9ee` 起。
读数都在本副本 `target/` 下（内容寻址的 CAS + 各图清单）。

---

## 一、两个新天体

### 1.1 `moon`：基础地形 + 三层陨坑（`px run moon`）

新算子 **`field.craters`**（`px_field_schema::ops::Craters` / `px_field_op/src/ops/craters.rs`）：

* **吃上游**（`CratersInput { base }`），输出 = 输入 + 坑的剖面 —— 地形美术里"基底 + 叠一层细节"
  是**加法**，而值域里的加法在这个词汇表里只有这一条路（`field.mix` 是插值，插不出"坑缘高过基底"）；
* 每一层是**元胞距离**（到最近特征点的距离，格为单位；`px_field_op/src/noise.rs` 里新加的
  `worley_3` / `worley_2`）再套一个剖面：坑内是 `t²` 的碗（`t = 1 - d / radius`）、
  坑缘是 `radius .. radius + rim` 上的半正弦凸起。**两者在 `d = radius` 处都是 0** ⇒ 剖面连续、
  层与层可以直接相加而不会在坑边留下台阶；
* **球面档**（`spherical = true`）按 `direction` 取格点（3D 元胞）⇒ 没有接缝、两极也不挤；
  平面档按 `(u × aspect, v)` 取格点；
* 多层按 `gain` 加权、再**除以总权** ⇒ 叠多少层都不改变幅度量级；
* ⚠ **算子不钳制输出**：进来的地形本来就可能越界（上游是 `field.remap` 映宽的），
  "算出来必须落在 `[0,1]`"不是这一层替别人兜的底。收口是图的事（`moon` 图里接一个 `field.remap`）。

`moon` 图（`px_graphs/src/bin/moon.rs`，画布 780×520 `Cube`，6 个节点）：

```
terra(field.fbm) → basins(陨坑①低频大盆地) → craters(陨坑②主坑) → pits(陨坑③小坑)
                → height(field.remap 收口) → surface(mesh.cubesphere)
```

三层坑是**三个节点**而不是一个节点里的三个 octave：各自的 TOML 给频率/半径/深度 ⇒
大盆地、中坑、小坑分开调，而且调一层只重算它自己与下游。实测：6 个节点冷烘 5869 ms。

新色板 **`Palette::Moon`**（`px_graph/src/generate/palette.rs` + `shade.rs`）：
月海（暗、略偏蓝，`MARE`）×风化层（中性灰，`REGOLITH`），加一层溅射纹。
⚠ 为什么不复用 `rocky`：`LAND` 色带里有植被那一段（`[0.259, 0.435, 0.216]`），
无大气天体配上它就是一颗**绿球**（实测第一版就是）；而改 `Rocky` 会换掉全部既有场景的贴图字节
⇒ 只能新加一档（加一档是**追加**，既有色板的路径一个字节不动）。

出图：`target/shot-moon.png`（960×640，陨坑在晨昏线附近最清楚）。

### 1.2 `gasgiant`：条带场 + **次表面散射**材质（`px run gasgiant`）

⚠ **用户口径（两次打回）**：

1. 「不要用普通的云来处理气态行星啊，气态巨星应当用此表面散射」；
2. 参考图两张：土星（Cassini）与天王星（Voyager 2）—— 都是**光滑、低对比、晨昏线极软**的球。

第一版拿 `clouds` 那套软档（体积壳 + 等值面）画它，出来的是一圈圈**贴在球上的云带**
（带边像塑料环；那一版没留图，只有"带边是硬边"这条读数）。气态巨行星没有那个边界：
它是一整球气体，看见的每条带都是「光穿过这一柱气体再散射回来」的颜色。

⇒ 新材质 **`art/shaders/gasgiant.wgsl`**（自写、次表面散射，四步）：

| 步 | 算式 | 直观 |
|---|---|---|
| ① 穿透深度 | `depth = thickness · (1 − band_gain·(band−0.5)·2) · (1 + limb·grazing)` | 条带场给"这一柱多厚"，视线越斜穿得越长 |
| ② 每通道吸收 | `exp(−absorption·depth)`（RGB 各一个系数） | 蓝光吸得多 ⇒ 深带偏暖偏暗、亮带偏白（土星那条奶黄/棕的分界） |
| ③ 包裹漫反射 | `clamp((N·L + wrap)/(1 + wrap), 0, 1)` | 半透 ⇒ 照亮的一侧**越过几何晨昏线**，交界是一条宽软带 |
| ④ 边缘散射 | `gas · limb · grazing⁴ · lit` | 切向那一圈穿过的气体最厚 ⇒ 边缘自己发亮 |

条带**由图烘出来**：`gasgiant` 图（3 个节点 `turbulence(fbm) → bands(实例 LatBands) → mixed(remap)`）
→ `px_graph::generate::field_cube`（一张 `CubeMap` 场 → 6 层 RGBA16F 立方贴图）
→ 材质 `@binding(7)`。⇒ 图管线仍是条带来源，材质只管散射。

⚠ **两条踩出来的坑**（都在这一轮里修掉）：

* **本体不能用岩石那张位移网格**：第一版把 `planet` 图的 `mesh` 挂上去，结果是画面上出现几块
  "海岸线"——那其实是那张图的**陆地**：`world_normal` 带着地形起伏 ⇒ 条带被采样歪、轮廓也是锯齿。
  ⇒ ① 材质改用**几何球面法线** `normalize(world_position)`（光球层就是一个球）；
  ② 场景配方多了一条 `primitive = "icosphere"`（本体几何可以是内建球，不必是图烘的网格）。
* **参数要真的生效**：见 §二.1（这一条是整轮里最值钱的发现）。

出图：`target/shot-gasgiant.png`（960×640）。

---

## 二、四条系统问题（三条这一轮踩出来，一条顺手收口）

### 2.1 ⚠ 图侧实例拿不到节点参数 ⇒ **参数进键、不进计算**

**症状**：改 `art/<图>/<节点名>.toml` 里那三栏（`RemapParams` 的 `gain` / `bias` / `bands`）
会**换节点键、会重算**，而写出来的产物**逐字节相同** —— 美术改参数，看到的是"重算了一遍，
图没变"。

**量法**（`field_remap` 图，`waves` 那条实例）：不写参数文件 ⇒ 产物内容 `bc8ab272f232`；
写 `art/field_remap/bands.toml`（`bands = 3.0, gain = 0.0`）⇒ 节点键从 `079268f9a209` 变到
另一个值、CAS 里多一份文件，而**内容是同一份** `bc8ab272f232`。
⚠ 第一次量还搞混了"CAS 文件名（键）"与"文件字节的 sha256"这两件事（`6ecce441…` 是前者、
`bc8ab272…` 是后者）—— 记录在案：**量"内容变没变"必须比较字节，不是比较路径**。

**根因**：`px_field_alg::remap_with` 里那一句 `let _ = params;`（参数只用来对齐签名），
而 `FieldFn::value` 的签名里根本没有参数这一栏 ⇒ 图侧函数只能自己读 `RemapParams::default()`
（编译期常量）。体积域那边从第一天起就是把形状参数递给场函数的（`CoverCloud` 由
`proxy::from_volume(params)` 建、`cover` 收它）—— **场域这一档漏了**。

**修**：`Cell::value` / `FieldFn::value` 的第一栏就是 `&RemapParams`，`map_grid` 原样递给
`cell`（`remap_with` 不再丢它）。判据：`px_field_alg` 里新加一条
`the_node_params_reach_the_field_function`（场函数回什么，输出就得是什么；换个值必须换内容）。
端到端实测（`gasgiant` 图）：`bands = 7` ⇒ 内容 `2e480f6a52e41443`、均值 0.5238；
`bands = 3` ⇒ 内容 `b14726a8e7fed764`、均值 0.5258，**改回 7 又命中原来那份**。

### 2.2 ⚠ 图侧实例拿不到**球面方向** ⇒ 纬向/极冠这类函数写不进实例库

`uv` 是**图像坐标**：`CubeMap` 投影下 `v` 跨的是"六张面叠起来的那一条"（`height = 6 × face`），
不是纬度 —— 拿 `uv[1]` 当纬度会在面与面之间跳变。而"纬向条带 / 极冠 / 陨坑"这类函数**只能用
球面方向**。体积域的 `FieldFn::cover(&self, cloud, direction)` 从第一天起就收方向；场域此前只给 `uv`。

**修**：`Upstream` 加 `direction(x, y)`，`map_grid` 算一次递下去（同一族的两栏一起补）。

### 2.3 `px_decls` 两份会漂开的清单（顺手收口）

`decl()` 的 match 臂 + `NAMES` 数组是**两份**手维护清单，而门只能数**条数**
（`px_op!` 处数 == 表里条数）⇒ 配错行（`"Fbm" => facts::<Ridged>()`）**每一道门都过**，
直到某条实例拿它去编译才炸，且炸在离现场很远的地方。

**修**：合并成**一张** `TABLE: &[(&str, fn() -> DeclFacts)]`（`decl` / `names` / `entries` 全从它派生），
加一条门 `the_table_names_are_unique`（`decl()` 找的是第一个同名的行 ⇒ 重名会静默吞掉后面那行）。

### 2.4 干净 checkout 上 `px-scene` 帧图测试必红

帧图的片元成员按**名字**去 `shaders` 图的清单里查键，而那份清单只可能由**某个人先手动跑一次
`--bin shaders`** 才有 ⇒ `target/` 被 gitignore ⇒ 新 checkout 上 `frame::tests` 三条全红
（"图 'shaders' 的清单读不到"）。判据要的是"靶子在仓库里、产物可重跑"，所以"可重跑"必须是一个
**能被调用的函数**，而不是只活在某个 bin 的 `main` 里的代码。

**修**：`px_graph::bake_shader_graph()`（`driver.rs`）承载烘那一套，`--bin shaders` 变成薄壳，
测试用 `ensure_shader_graph()` 缺了就当场烘（**不跳过**）。判据：删掉 `target/pcg/shaders/manifest.json`
再跑那三条 ⇒ 通过，并且清单被重新写出来。

另：`px_cook` 缺实例的提示命令从 `cargo run -p px_graphs --bin px -- build` 改成
**直接跑 driver exe**（`<与图 exe 同目录>/px.exe build`；`driver_command()`）——
`cargo run` 会按另一套特性合并把图程序重链一遍，而本仓的口径一直是"读 key 直接跑 `target/debug/px.exe`"。
实测（藏起 `latbands` 的实例库再跑 `px run gasgiant`）：提示打的就是
`C:\…\target\debug\px.exe build` 与 `… px.exe run gasgiant --build`。

### 2.5 顺带：`px_shader` 那条"数文件个数"的测试

它写死 `wgsl_files(...).len() == 10`（"库 3 + 入口 7"）⇒ **加一份 shader** 就让一条与数量无关的
测试红（这一轮加 `gasgiant.wgsl` 就撞上了）。改成判**约定**：一个根只扫一层（每个文件只被收一次）、
**库带 `#define_import_path`、入口不带**（`--bin shaders` 判"是不是入口"用的就是这一条）。

---

## 三、新实例 `field.remap/latbands`（`art/inst/latbands.rs`）

表里加一行、加一个源文件，就多一条实例（`px list` 从两条变三条）：

```
cloud.coarse/band    51efd6241f82  px_volume_alg  art/inst/band.rs
field.remap/waves    a0050d001426  px_field_alg   art/inst/waves.rs
field.remap/latbands a0050d001426  px_field_alg   art/inst/latbands.rs
```

⚠ 两条场域实例**共用同一条声明**（`FieldRemap`）与同一个 `op_id` —— 区别只在 `source` 那一栏
（`type_name` 也不同：`Waves` / `LatBands`）。⇒ "同一张声明、不同的图侧函数"这件事不需要新声明。

`LatBands` 拿 `direction[1]` 取纬度、按 `bands` 圈撒正弦，上游（湍流场）**挪相位**
（`× 3.5`：相位摆幅要大于一圈条带的宽度，边界才会真的起伏成波浪，否则看着还是一圈死正弦）。
它就是气态巨行星的条带来源（`gasgiant` 图里那个 `bands` 节点）。

---

## 四、判据（这一轮跑了什么、没跑什么）

| 判据 | 覆盖的改动 | 读数 |
|---|---|---|
| `target/baseline/pre.json` 内容哈希对账 | **所有**算子/算法改动（键会换、内容不许变） | 旧四张图 **30/30 节点内容逐字节不变**（planet 6 / desert 8 / clouds 14 / field_remap 2） |
| J1 六张判据图（`art/anchor/hashes.txt` §一） | 色板 / 材质 / 场景编译 / 算子身份 | **6/6 逐字节相同**（哈希与字节数都对上） |
| `px_field_alg` 单测 | `FieldFn` 两栏 + `map_grid` | 过（含新加的那条"参数到得了场函数"） |
| `px_field_op` 单测 | `field.craters` 不破坏既有算子 | 过 |
| `px_decls --test inst_gate` | 单表 + 重名门 | 过 |
| `px_graphs --test inst_gate` | 三条实例的生成物、key 稳定性、真 cook | 过 |
| `px-scene --lib frame::` | 帧图测试自烘 + 材料/贴图条件化 | 过（7 条） |
| `px_shader --lib` | 数量断言 → 约定断言 | 过（21 条） |
| `px run moon` / `px run gasgiant` / `scene` / `px_render` | 两条新管线的端到端 | 出图 `target/shot-moon.png`、`target/shot-gasgiant.png` |

**没跑**：整仓 `cargo test --workspace`（80 个测试二进制，里面 `clouds` 的端到端 cook 就要 40 s）。
上面这张表里的判据是**按触及的分支挑的**：算子身份一变，覆盖它的是"30/30 内容逐字节"+ J1 六张，
而不是"把所有测试再跑一遍"。

⚠ 这一轮把 `art/anchor/hashes.txt` §三 **重登记**了一次（第六次）：根因 A 是本轮的身份变化，
根因 B 是**登记值本来就与这一份 worktree 对不上**（实测：把改动前的旧键写回清单再用今天的代码重烘，
得到 `DF84B7EF…`，不是登记的 `EBCD0389…`；主工作副本的暖清单里躺着更早一代 `2795F948…`）——
即登记值来自**第三份状态**，§六 那条"行尾约定 ⇒ 键变 ⇒ §三 全红"就是这一格的机制。
新值按本副本量、同一冻结态连跑两遍同值。
