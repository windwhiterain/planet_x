# 体积云

云覆盖度立方图（`Domain::CubeMap`，六面沿行堆叠）+ 云壳内的体积 raymarch，以及云密度场的正确梯度组装。

## §39 云覆盖度图 + 体积云（阶段 1–2）

**形状**：云直接用一张 cubemap，高度上的分布和细节用 shader 提供 ✓。理由：64³ 体积铺满 `[-1.1,1.1]³` 时体素边长 0.0344，而云带只有 0.05 厚 ⇒ **竖着只有 1.45 个体素** ✗，PCG 表达不了云顶/云底。
高度交给 shader 之后，**烘出来那张图不需要垂直分辨率** ✓ ⇒ 既不用 `Volume3D`、也不用 `texture_cube_array`，连 §24.4 预判的"把算子泛化到 3D"都免了 ✓。
地面云影走**自写 surface material**（"自写，反正也要换 cubemap"）✓ —— 落在阶段 3/4。

### §39.1 现在能跑什么

```
cargo run -p px_graphs --bin clouds            # 烘云覆盖度图（256²×6，冷 ~1.5 s，缓存 ~0.3 s）
px_render --serve                              # 常驻渲染服务
px_render --planet <H.pxart> --mesh <M.pxart> --palette rocky --clouds <C.pxart> --out x.png
px_render ... --cloud <K>                      # 消光倍率，默认 1（= 900）
px_render ... --cam yaw,pitch,dist             # 局部放大判据（开发期主力）
cargo test --workspace                         # 全绿
```

### §39.2 `Domain::CubeMap`：六面沿行堆叠

- **布局**：`width = 面边长`、`height = 6 × 面边长`，行主序 ⇒ **与 wgpu 的 cube 层布局逐字节一致** ✓，上传时不用重排 ✓（`clouds::coverage_image` 只做 f32 → f16）。
- **面朝向：代价 0** ✓。仓里原有的 `cube_direction` / `cube_face_of` **本来就已经是** wgpu 的约定（面序 +X −X +Y −Y +Z −Z，面内朝向也一致）✓ —— §35 代价表里"面朝向：小但真实"那一格实际没花钱 ✓。
- **验证方式（值得照抄）**：把覆盖度**直接当颜色输出**（`return vec4(vec3(mask), 1.0);`），热重载看一眼：整球图案连续、六面缝合处无痕 ⇒ 约定端到端正确 ✓。总览图看不出这个，**这个判据是唯一能证明它的** ✓。
- `px_ops` 侧 `sample_direction` 走**跨面双线性** ✓（面的四条边是别人的纹素，通用 uv 路径会 clamp）。两条连续性测试让一张光滑解析场跨 +X/+Z 与 +X/+Y 棱走一遍：正确实现最大跳变 < 0.02，clamp 实现是 **0.43** ✗。

### §39.3 四条必须遵守的规则

1. **凡声明 `#define_import_path` 的 WGSL 模块都必须被当作资产加载** ✓。否则每个 import 它的管线都报 `Shader import not yet available`，而 Bevy 会一直重试（`pipeline_cache.rs:688` 把它重新入队）
   ⇒ 永久失败且**不报错到日志** ✗。现由 `ShaderLibraryPlugin` 扫 asset 目录、凡声明 import path 的一律加载
   （`atmosphere.rs` 里那个"写了没人读"的 `ShaderLibrary` 就是在干这件事，只服务 `common.wgsl` 一个库）。
2. **就绪判据不能把"失败"当"完成"** ✓：管线编译失败时必须判不就绪，并把**管线名 + 错误原文**打出来。否则会出一张**少了那个材质的"成功"图** ✗。**症状特征**：一整轮参数扫描的 PNG **字节数完全相同** ✗（极易被误读成"云太薄"）。
3. **`--serve` 必须有"空闲"这个状态** ✓：没活时把相机 `is_active` 关掉，有活了再打开、照样留 6 帧预热。
   否则离屏相机空闲也全速画，体积云把 GPU 持续占住 ⇒ 空闲的服务器在 **20–120 s 内必然掉设备** ✗（DX12 `DXGI_ERROR_DEVICE_REMOVED` = `0x887A0005`，日志里先看到的是下游症状：
   `Command allocator creation failed`、`clustering metadata staging buffer is invalid`）。
   A/B：同一台服务、同一颗行星、**只去掉云**，空闲 120 s **零掉设备** ✓；带云的每次必掉 ✗。
   修完实测：**空闲 240 s ✓ ＋ 连打 30 张图全成 ✓，零掉设备** ✓，且出图**逐字节不变** ✓。
   ⚠ "连续热重载之后每张图涨到 5–7 s"**不是热重载的锅** ✗ —— 那是同一个劣化过程的前兆（先变慢、后掉设备）；"改 shader ⇒ 不重建不重启"本身没错 ✓。
4. **怀疑"shader 被内联成一坨巨大字节码"⇒ 能量的就别猜** ✓：`tests/shaders.rs` 把每个入口 shader 过一遍 **naga 的 HLSL 后端并报体量**（涨一个数量级就失败）✓。实测 `clouds.wgsl` → **503 行 HLSL**（对照 `atmosphere.wgsl` 182 行）✓；naga 把 `gradient_noise_3` 输出成**真函数 + 真循环** ✓，两个 march 循环的边界是 uniform ⇒ **驱动也展不开** ✓。
   ⇒ WGSL 的内联确实是按调用点复制的 ✓，所以"循环体里塞几个噪声函数"值得量 ✗ —— 只是这次量出来没那么大 ✓。

### §39.4 价格（都在 RTX 3060 Laptop / DX12 上实测）

| | 每张图 |
|---|---|
| 无云 | **375 ms** |
| 云（56 主步 + 6 太阳步，3 阶 fbm） | **396 ms** |
| 云（塔状噪声，5 阶 fbm） | ~600 ms |
| §25.3 记的旧热路径 | ~1.0 s（含烘图） |

⇒ 云**没有**打破"每次 ~0.3 s"这条工作流 ✓。烘云图冷 1.5 s、命中缓存 0.3 s ✓。
⚠ 上面这些数**只能在"服务刚起来、且没在掉设备的路上"时测** ✗ —— 劣化之后同一个场景会变成 5–7 s ✗（§39.3 第 3 条）。

### §39.5 已测出的边界（用户已知情并选择保留）

- **云带 1.01–1.06 与地形打架** ✓（用户已知情并选择保留）：地形最高点 **1.036** ⇒ 陆地高于 1.01 的地方，云带**下半截被地表截掉**（`scene_distance` 把射线切在地形上 ✓，行为正确，但云底埋在山里）。画面上表现为"云在山尖上被削平" ✓，海洋/低地上没有这个问题 ✓。要改只需动 `CLOUD_BASE`。
- **相位与多散射**：单次散射 + 太阳在相机背后时云是灰的 ✗（背散射几何）。现在用**三阶透射率近似**（`exp(-τ)`、`exp(-τ/2)`、`exp(-τ/4)` 加权）✓，并把 HG 相位归一化到前向 = 1、与各向同性按 0.4/0.6 混合 ✓ ⇒ 云终于是白的 ✓。
- **云只在边缘浓**：这是**薄壳的正确光学** ✓（天底路径 ~0.05，掠射路径 ~0.64，差 13 倍），不是 bug。消光取 900 让天底也能压到不透 ✓。

### §39.6 下一步（阶段 3/4，待办）

- **阶段 3**：行星地表迁标准 cubemap + 自写 surface material（§35 的 B 档）。⚠ 判据要换：哈希**必然**变，"对已知好图无回归"这条不再适用，得改成并排 + 差异带。
- **阶段 4**：云影落进自写材质（只作用直接光）；把壳内介质 raymarch 提炼成通用库（`clouds.wgsl` 里的 `cloud_density` 是唯一一份密度实现，云影复用它 ✓）。

## §43 云密度场的**正确**梯度组装

**完整且经验证的形式**（用 shader 里的名字，`along = across·span()`，所以 `along/span = across`）：

```
P_t = I − d⊗dᵀ                                    // 切向投影算子

∇_p ρ = (ρ_a / span) · d                                              // ① 高度
      + ρ_n · [ (s/r)·P_t·g_q  +  (along/span)·(d·g_q)·d ]            // ② 噪声
      + (ρ_cover / r) · P_t · g_cover                                 // ③ 覆盖度贴图
```

被淘汰的形式 ✗ —— 只有在"∂ρ/∂r 是固定 d 的真偏导、且 ∇_d ρ 已经是切向"时才对：

```
∇ρ = (∂ρ/∂r)·r̂ + (1/r)·∇_d ρ
```

⚠ **②中方括号里第二项是最容易漏的，而且漏了是灾难性的** —— 因为 `q` 通过 `a` 依赖 `r`，**噪声有一个"超出 ∂ρ/∂a"的径向依赖**。实测（20000 个壳内随机点，对比三维中心差分）：

| 组装方式 | 中位数 | 最大 |
|---|---|---|
| **正确** | 1.3e-07 | 2.0e-05 ✓ |
| 漏掉 `P_t` 投影 | 3.0e-01 | 6.2 ✗ 百分比级错 |
| **漏掉 `(along/span)(d·g_q)d`** | **7.0e+00** | **9.7e+01 ✗✗ 灾难** |
| 往 `g_cover` 里加 3.7·d 的径向垃圾 | 与正确**逐位相同** ✓ | ← `P_t` 把它吃掉了 |

⇒ **③ 只需要 `g_cover` 的切向部分** ✓ —— 所以**那张烘焙图正好就是答案**：一次 `textureSampleLevel` 同时拿到 mask 和梯度，再 `P_t` 投影 ✓✓。用户那句"梯度不是烘焙了吗，只需要一次采样"**完全成立** ✓。

⚠ 但必须确认 `gba` 是**喂给 `coverage_of` 的那个标量**的梯度（`.r` 的 mask）。现在 R=`mixed`、GBA=`∇mixed` ✓ 一致；但 `coverage_of` 之上还套了 `smoothstep(0,0.45, clamp(...))`，所以 `∂cover/∂mask = 6t(1-t)/0.45 · 1/max(1-coverage,1e-4) · [0<t<1]` ✓。

### §43.1 分段点的处理约定（不是障碍，是约定）

`smoothstep` 的导数在两端**自己归零**（实测 `s'(0)=0`、`s'(1)=0`），所以 CAS 输出里那堆 Piecewise/Heaviside **会塌缩成一串 `select(...)`** ✓ —— 12 行就能转写完：

```wgsl
let t1 = clamp(a / base, 0.0, 1.0);
let ds1 = select(0.0, 6.0 * t1 * (1.0 - t1) / base, t1 > 0.0 && t1 < 1.0);
let dc_dn = select(0.0, top * detail_strength, m > base + 0.02);
```

⚠ 需要显式决定的：**解析梯度要不要在"值被强制为 0"的地方也返回 0** ✓ —— 现在两条路径都在 `altitude` 出壳或 `cover <= 0` 时直接返回 0 ✓，与 FD 版一致 ✓。⚠ **找面不靠梯度** ✓：找面靠的是对 `cloud_field` 的步进（`if cloud_field(point) > SURFACE_LEVEL { break }`），**梯度只用来算法线** ✓ ⇒ 换解析版不改变找面的行为；硬表面云**已经换掉了** ✓。

### §43.2 工具选型（已调研，不要再重新查）

| 工具 | 结论 |
|---|---|
| **`num-dual`**（MIT OR Apache，活跃） | ✓ **用它**。但**不做 `px_ops` 的 dev-dependency，也不是那个 crate** —— 见 §45：它住在一个**独立 crate `px_verify`** 里 ✓（用户否掉了塞进 `px_ops`：脚手架不该长在出图的 crate 上）。`default-features = false` 可以甩掉 `nalgebra`/`simba` ✓ |
| `symbolica` | ✗ **不要**。源码可见但**非开源**：受雇使用需付费 **EUR 3000/年/开发机**，CI 单独报价 |
| `egg` | ✗ 它是 e-graph 重写库，**你不写规则它就不会求导** —— 等于手推 |
| `rust-gpu` / `cubecl` | ✗ 前者只出 SPIR-V；后者是 compute DSL，等于把场**写第三遍** |
| `naga-rust` | 只能当"无 GPU 时执行"的备选，作者自己说"expect compilation failures" |

⚠ **`num-dual` 的分段语义是它的卖点**：其源码注释明确 *"Comparisons are only made based on the real part. This allows the code to follow the same execution path as real-valued code would."* ⇒ 分支与真值代码一致 ✓。但它**没有 `floor`/`min`/`max`/`clamp`/`abs`**（`Dual` 甚至不实现 `num_traits::Float`），要自己写 ~6 个泛型小工具 ✓ —— **这是好事**：分支语义变成显式可审的。
