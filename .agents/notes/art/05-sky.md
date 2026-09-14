# 天空与大气

星空统一到标准 cubemap、大气边缘光壳（自写 shader，现状是弦长积分）、以及 Bevy 内置散射大气为什么挂起、下次怎么验。

## §34 大气边缘光

### §34.1 结构

- `px_render/src/atmosphere.rs`：`AtmosphereMaterial`（`AsBindGroup` + 两个 uniform：参数、色调），
  `impl Material` 只给 `fragment_shader()` 与 `alpha_mode() = Add`；`AtmospherePlugin` 注册 `MaterialPlugin`。
  ⚠️ `AtmosphereParams.camera_x/y/z` 与 `sync_cameras` 是**死代码**：shader 早就改读 `view.world_position` 了，
  它却把最后一台相机的世界位置写进**全局** material uniform。收拾它要和 WGSL 的 uniform 布局一起改。
- `px_render/assets/shaders/atmosphere.wgsl`：**把法线用 `view.view_from_world` 转到视图空间**这一步保留；
  轮廓**不是**菲涅尔（`normal_view.z → 0`），而是**弦长积分、终点由深度决定**；菲涅尔写法已删除。
- 几何：比星球大 3.5% 的平滑球（`Sphere::new(r*1.035).mesh().ico(48)`），挂在同一个 system 实体下。
- 色调/强度/幂次**按调色板给**（`Palette::atmosphere()`）；`--atmo` 是渲染器侧倍率（0 关掉），与 `--ambient`/`--cam` 一样放在 `Request.view` 里（属于渲染器，不属于场景）。

### §34.2 必须遵守的规则

① **两个 App 都要注册 `AtmospherePlugin`。** 服务端是**另一个 App**，`Assets<AtmosphereMaterial>` 不存在 ⇒ `accept_jobs` 的系统参数校验失败、整个服务 panic。

② **`AssetPlugin.file_path` 相对可执行文件解析，不是相对工作目录** ⇒ 用 `asset_root()`：先试工作目录、再沿 exe 往上找，找到含 `shaders/atmosphere.wgsl` 的那个。

③ **换掉一个隐式默认值之前，先量出它到底是多少。** Bevy 的全局环境光默认是 **80**（不是 16）⇒ `DEFAULT_AMBIENT` 明显小于 80 就等于把场景环境光悄悄压下去（整颗星球发暗发蓝）。

④ **测量结论必须绑定测量时的系统状态** ✓ —— 系统半坏（管线还在输预热竞态）时测出的"引擎不可信"极易误导 ✓。
`view.world_position` 语义可信 ✓，shader 正大光明用它，**实测与传 uniform 版逐值相同** ✓。"整片糊在球面上"的真正原因是**几何**：壳只比行星大 3.5% ⇒ 可见壳面处处贴近自身轮廓 ⇒ 菲涅尔处处 ≈1 ✗。

## §35 用标准 cubemap 统一天空 + 统一到行星的代价表

两条已查实的事实：

- **`Skybox` 就是标准 cubemap，且按方向采样**：`skybox.wgsl` 里 `var skybox: texture_cube<f32>;`
  `textureSample(skybox, skybox_sampler, ray_direction * vec3(1.0, 1.0, -1.0))` ✓（那个 `vec3(1,1,-1)` 是它的面朝向约定）。`Skybox { image, brightness, rotation }` 定义在 `bevy_light`。
- **`StandardMaterial` 收不了 cubemap**：`pbr_bindings.wgsl` 是 `var base_color_texture: texture_2d<f32>;` ✗
  ⇒ 想让**行星**也走标准 cubemap，必须**自写 surface material**（大气那个 shader 是第一步）。

### A 档（现在的做法）：只统一天空

`star_cube(face)` 生成**真正的 cube `Image`**（6 层 + `TextureViewDimension::Cube` ✓），星点按**面内纹素**哈希 ✓；相机挂 `Skybox { image, brightness: 900 }` ✓；
**星空球实体与它的 UV 记账全部删掉** ✓✓。12 角度对照图（`target/probe-skycube.png`）星点是干净细点 ✓。
**代价（PCG 侧）：0** ✓ —— 天空从来不是 PCG 产物 ✓ ⇒ cubemap 统一的代价全在渲染器要不要自己拥有 surface shader，不在 PCG ✓。

### B 档（全面统一到行星）的代价表（待做时按此执行）

| 项 | 代价 |
|---|---|
| 布局 | 小：6 张等大正方形、**无 gutter**；线格式加层轴（`BlobHeader.shape` 本是 `Vec<u32>`，写 `[face, face, 6]` 即可） |
| 缓存 | 一次性全失效（键含画布尺寸）≈ 每图 1–2 秒 |
| 算子规则 | **中，唯一真规则**：需要邻居的算子必须走 `sample_direction`（模糊/腐蚀/距离场/流向），不能假定整图是平面；逐纹素算子不受影响（fbm/ridged/remap/mix ✓，warp 已合规 ✓） |
| 面朝向 | 小但真实：面序与**面内朝向**要对齐 wgpu 约定（面序已一致，面内朝向可用 `Skybox` 对拍一张验证） |
| 渲染器 | **大**：自写 surface material（cube 按方向采样 albedo/roughness/emissive + 光照），等价于把 feature 栈提前做掉 |

**规则**：平台已经为某件事定义了标准格式时，不要手搓一个等价物 —— 手工做（图集 + per-face UV 记账）翻车两次 ✗✗，让平台做（标准 cubemap + 按方向采样）一次通过 ✓✓。

## §36 试 Bevy 内置大气：挂起，以及已经测出来的边界

接了 Bevy 的散射大气（`Atmosphere` + `ScatteringMedium` + `AtmosphereSettings`），代码留在 `--scatter earth` 后面（默认关 ✓）。**结论：挂起，壳式后端继续用。**

### §36.1 接口（已查实）

- `Atmosphere` 是 `bevy_light::Atmosphere`（**世界坐标球**，实体的 `GlobalTransform` 即球心；文档明说**用缩放在世界空间重定尺度**，`inner_radius`/`outer_radius` 单位是米）。
  它在 `on_add` 时若 `GlobalTransform` 仍是默认值，会把球心挪到原点下方 `inner_radius` 处 ⇒ **轨道视角必须自己给变换**（"站在行星上"是它的默认姿态 ✗）。
- `bevy_light::atmosphere::ScatteringMedium`（`earth(256,256)` / `mars(...)` / `from_curve(...)`；`Default` = `earth(256,256)`）。
- `bevy_pbr::AtmosphereSettings` **挂在相机上**（LUT 尺寸与采样数），"最近的大气"参与渲染。
- `AtmospherePlugin` **不在** `PbrPlugin` 里，但 `DefaultPlugins` 已经带了它 ✓ ⇒ 显式再 add 会 panic："plugin was already added" ✗。

### §36.2 测出来的边界（四个数据点，都是哈希/截图）

| 配置 | 结果 |
|---|---|
| 默认（无 `AtmosphereSettings`，壳式大气） | **行星正常** ✓（与已知好图逐字节相同） |
| 只挂 `AtmosphereSettings`（没有 `Atmosphere` 实体） | **行星消失** ✗ 只剩星空 |
| `AtmosphereSettings` + `Atmosphere`（真实米制半径 × 缩放 1/6.36e6） | **整帧全黑** ✗ |
| 同上但半径改成场景单位（1.0 / 1.0125，缩放 1） | 与上一行**同一 hash** ✗ ⇒ **不是尺度问题** ✗ |

⇒ **只要让大气真正参与渲染，我们的离屏场景就被清空** ✗，且与半径标定无关 ✓。

### §36.3 下次的便宜实验（按顺序）—— 还没做

1. **在窗口相机上试**（预览窗口是真窗口 ✓，`--serve` 走的是离屏 `RenderTarget::Image` ✗）——这一步能把"离屏路径的问题"与"功能本身的问题"分开 ✓，是最便宜的一刀。
2. 检查与 `disable::<WinitPlugin>()` + `ScheduleRunnerPlugin` 驱动方式的关系（大气插件往我们没跑的调度里加系统的可能性 ✗）。
3. 若都不行 ⇒ 走我们自己可控的那条：**自写 surface material 采样 `aerial_view_lut`**（`render_sky.wgsl` 已经证明这条路成立 ✓，它用深度纹理做 in-scattering + transmittance 合成 ✓）。

**当前状态**：壳式大气是可用后端 ✓；散射后端挂起 ✓；默认路径经复核对已知好图**无回归** ✓。
