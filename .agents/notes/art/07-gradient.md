# 云密度场的梯度

基准的三条腿各自覆盖什么、精确仲裁者 `px_verify` 的形状与数字、解析梯度怎么上的生产、以及差商这条腿为什么退场。

## §44 梯度验证的三条腿，各自覆盖什么

基准一共**四条腿**，它们**覆盖不同的失效模式，缺一条都不够**：

| 腿 | 回答的问题 | 覆盖 | 需要 GPU | 状态 |
|---|---|---|---|---|
| **① 对偶数语义**（`px_verify/tests/dual.rs`） | 仪器本身准吗？ | 手写的 `max/min/clamp/smoothstep` 对不对 | ✗ | ✓ |
| **② 噪声层对拍**（`px_verify/tests/dual_noise.rs`） | 噪声的对偶梯度 = 它自己值函数的导数吗？ | **代数风险**（噪声部分） | ✗ | ✓ |
| **③ WGSL 逐点对拍**（GPU 探针） | **发布的 WGSL 里**，解析梯度 = 值函数的导数吗？ | **代数风险（全式）＋ 它就在正确的边界一侧** | ✓ | ✓ |
| **④ 场级 f64 仲裁者**（`px_verify/src/cloud_field.rs`） | **基准本身**对不对？ | **基准的失效**（差商底、相消、折点） | ✗ | ✓ |

⚠ **①②都只在"Rust 函数"这一侧** —— 它们**回答不了"Rust 的函数 = WGSL 的函数吗"** ✗。那条边界才是真正的风险所在。

⚠⚠ **③ 也不够**：③ 拿差商当基准，而差商在生产配置上**自己就有 6.6e-4 ~ 1.1e-2 的底**（§45.2），故障量级 1.9e-1 只比它大 30 倍 ⇒ **③ 单独分不清"公式错"和"基准不够格"** ✗。**④ 因此是第一等的腿，不是兜底** ✓。
### §44.1 顺序：④ 不是"兜底仲裁者"，是第一等的腿

建腿的顺序是**先 ③（GPU 上的 WGSL 自对拍）**；但 Rust 场级 f64 参考**不是**"降级为只在 ③ 模棱两可（比如折点附近）时出手的仲裁者"✗ —— 那省下了一份必须维护的重复实现，却把**唯一能判定的仪器**降级掉了。理由见 §45.2：③ 的基准（差商）在**生产配置**上的底是 6.6e-4 → 1.1e-2，与故障量级 1.9e-1 只差 30 倍 ⇒ **不是"模棱两可"，是基准不够格** ✗。⇒ **④ 与 ③ 并列，都是第一等的腿** ✓。
### §44.2 判据：扫 h，不要拍容忍度

噪声对拍第一次跑出 7% 偏差，看着像公式错：

```
h=1e-2  0.3527
h=1e-3  0.1691
h=1e-4  0.00024    ← 缩了 700 倍 ⇒ 是截断误差，不是公式错
h=1e-5  0.00263    ← 又涨（f32 相消）
```

**教科书 U 形**：左截断、右相消、真值在中间。**对偶数一直是对的，是用截断误差 7% 的步长在量它。** ⇒ 测试**断言收敛本身**，而不是断言一个数：

```rust
for pair in worst_per_step.windows(2) {
    assert!(pair[1] < pair[0] * 0.5, "偏差没有随步长缩小 ⇒ 公式错，不是步长太大");
}
```

**公式错了扫描是平的；步长太大它会缩。** 一个魔法容忍度**区分不了这两者** ✗。

⚠ 这套扫 h 机械存在的唯一理由是**基准是差分**；基准精确（④）之后它整套消失 —— 不需要步长、不需要折点余量、不需要扫 h、不需要退到简化配置（§45.1）。
### §44.3 折点：没有对错，只有一个要声明的约定

`clamp` / `max` / `floor` 边界处导数**不存在**。对偶数返回**它实际走到那一支的单侧导数** ✓；中心差分**跨过折点**，给的是另一个东西 ✗。**两者在折点邻域不可比。**

⚠ 判据**不能按"平均值"过滤**折点故障：`fbm_3` 对**三个各自 `clamp01` 的八度取平均**，**内层八度饱和时平均值看着完全正常** ✓（诊断打印才看见某八度正好是 `1.000000000`）。折点上的偏差靠**逐通道归因**抓（§46.3 的 `the_residual_is_attributed_to_one_channel`）✓。
### §44.4 headless wgpu 的地基

`px_render` 原来是 **binary crate，测试 `use` 不到它的任何类型** ✗。现在是 lib（`px_render/src/lib.rs`），组装助手在 `px_render::shaders` —— 一份实现，离线门与 GPU 探针共用，不是"测试里镜像一份"。无窗口设备：`Instance::default()` + `request_adapter(compatible_surface: None)`，**4.24 秒**拿到 RTX 3060。

⚠ 这条路径走的是 **Vulkan 不是 DX12**（`Instance::default()` 自己挑后端）—— 对拍出现说不清的差异时要记得这个变量。探针现在默认钉 **DX12**，`WGPU_BACKEND=vulkan|gl` 仍可覆盖。
## §45 精确仲裁者：独立 crate `px_verify`（2026-09-14）
### §45.1 形状

```
px_verify/
  src/dual.rs         num-dual 的 Scalar 适配（分支按实部取 ⇒ 与 shader 的 f32 比较同构）
  src/noise.rs        照抄 noise.wgsl 的 gradient_noise_3 / fbm_3
  src/cloud_field.rs  云场参考实现，逐级暴露 footprint/lobed/floor_here/ceiling/under_top
  tests/              对偶数梯度 vs 自身差商（场级 + 覆盖度带）
```

`px_probe` 以普通 dependency 引用它 ✓；`px_ops` **不引** ✓（`Scalar` trait 留在 `px_ops` —— 它只是个 trait，不是脚手架）。`px_render` 的 `[dev-dependencies]` 只剩 `naga` —— `px_verify` / `wgpu` / `bytemuck` / `pollster` / `encase` 都随 GPU 探针搬去 `px_probe` ✓。

⚠ **对偶数给的是精确导数** ⇒ **不需要步长、不需要折点余量、不需要扫 h、不需要退到简化配置** ✓✓。
### §45.2 三处纠正（§43–44 里写错的地方）

1. **§43.1「`ABLATE_NORMALS` 靠 FD 的尖峰找表面」是错的** ✓。找面靠的是 `if cloud_field(point) > SURFACE_LEVEL { break }`，**梯度只喂法线** ✓。硬表面云**已经换成 `cloud_field_gradient_analytic`** ✓，FD 版 `cloud_field_gradient` 和只服务于它的 `SURFACE_EPSILON` 一起删掉 ✓（每像素少 6 次 `cloud_field` 调用）。
2. **§44.1 的顺序结论被实测推翻** ✓。原结论："③ 先建，Rust 场级参考**降级为仲裁者**，只在模棱两可时出手 ⇒ 少一份重复实现"。实测：③ 拿差商当基准，而差商在**生产配置**上的底是

   ```
   h=8e-5  6.6e-4      ← 最好的
   h=5e-6  1.1e-2      ← 越细越差（f32 相消占了上风）
   ```
   故障量级是 **1.9e-1** —— 只比底大 **30 倍**。这不是"模棱两可"，是**基准不够格** ✗。⇒ **仲裁者必须是第一等的腿** ✓；把它降级掉的代价是**唯一能判定的仪器没了** ✓。
3. **§43.2 的「做 dev-dependency」作废** ✓ —— 脚手架不该长在出图的 PCG crate（`px_ops`）上，于是有 `px_verify` ✓。
### §45.3 找到的东西

**(a) `px_ops` 的噪声和 shader 的噪声不是同一个函数** ✓✓ —— **生产问题，已裁决：不改代码，只改名**（§46.2①）。

```rust
px_ops::noise::gradient_noise_3   let tx = smooth_scalar(point[0] - x0);
                                  let dot = g[0]*(tx - off[0]) + ...   // ✗ 点乘的是平滑后的 local
```
```wgsl
noise.wgsl::gradient_noise_3      let local = point - base;
                                  let axis = local - offset;           // ✓ 点乘的是原始 local
```

固定点 `[1.3, 2.7, -0.4]`、seed 7：Rust **0.33985612** 对 shader **0.31646734** ✓。变量名叫 `tx`，所以这个替换在 Rust 里**看不见** —— `weight` 用对了，`dot` 用错了 ✓。⇒ **凡是拿 `px_ops` 当"shader 的参考"的验证都会被它带偏** ✓。`px_verify/src/noise.rs` 是**照抄 shader** 的独立实现 ✓ —— 两边**不应该**共享这份代码 ✓。

**(b) 覆盖度那条链的链式因子写错** ✓ —— `|修正后| / |原先| = 9.259259 = 1/0.108 = 1/baked.g`，到 7 位 ✓ —— `baked.g` 是 `∂mask/∂x`，是梯度的**分量**，不可能同时当**标量因子** ✓。

**(c) f16 覆盖度贴图当不了这个 oracle** ✓ —— mask 在 4h 上只走 **5.7e-5**，而 f16 在 0.5 附近的台阶是 **2.44e-4**（大 4.3 倍），过滤后成阶梯，**差商恒为零** ✓✓。**这才是盲区"为什么"存在** ✓。（⚠ 生产烘那张图用 `Rgba16Float` **本身够用** ✓ —— 它存的是**值**不是**差商** ✓；只有拿它当差分 oracle 才会坏 ✓。）
### §45.4 两侧对照不是形式主义

arbiter 一度报「**修复前** 3.1e-3 反而比**修复后** 2.0e-2 **更接近**精确值」✗ —— 即"修复把东西改坏了"。原因：把 `∇mask` 当成 `∇cover` 送进模型，**少乘了 chain** ✓。于是参考值落在 **1×**，修复后 **4.9×**，修复前 **0.54×** —— 1× 自然离 0.54× 更近 ✓。⇒ **单向断言（"偏差 < 阈值"）在这里会判反，而且正好放过真 bug** ✓✓。覆盖度那条现在是**两侧**断言的：修复后 **4.288e-6**，修复前 **2.348e-2**，**相差 5476 倍** ✓。
### §45.5 现在的数字（生产参数，RTX 3060 / Vulkan）

| 链 | 精确基准下的中位相对偏差 |
|---|---|
| 高度 ① + 噪声 ②（常量覆盖度，204 点） | **4.905e-6**（最坏 6.2e-4，落在噪声 clamp 边界） |
| 覆盖度 ③（变覆盖度，122 点） | **4.288e-6**（最坏 2.589e-4） |

⇒ **生产路径上手写的链式法则全对** ✓✓ —— 这是整条线第一次有精确证据。
### §45.6 规约

- **别拿差商当"公式对不对"的判官** —— 先量**基准自己的底** ✓。差商的底 ≈ `ε/(2h) × 场值量级 / 梯度量级`；壳只有 0.05 厚、|∇ρ| ~ 500 时，h=5e-6 的底就有 **1e-2** ✓。
- **两侧断言**：只测"新的对不对"会漏掉"参考实现自己写错了"，而且会**判反** ✓（§45.4）。
- **子 agent 会被打断，且可续跑** ✓：先 `list_agents` 看状态，`[ready]` 可用 `send_message` 续跑；被打断 ≠ 失败，**不要新派一个重做** ✓。
- **不要在同一个分支上跑两个 agent** ✓ —— 另开 worktree ✓。
### §45.7 三项待裁决 —— 已全部裁决，落法在 §46.2
## §46 三项裁决 + 解析梯度上生产（2026-09-14）
### §46.1 硬表面云换成解析梯度（生产改动）

`cloud_field_gradient`（6 点差商）删除，`ABLATE_NORMALS` / `ABLATE_SURFACE` 改调 `cloud_field_gradient_analytic` ✓。每像素少 6 次 `cloud_field` 调用（每次内含两组 fbm、共 5 个梯度噪声）⇒ 噪声工作量约降到 1/6。

⚠ **「`ABLATE_NORMALS` 靠 FD 的尖峰找表面」是错的** ✓：找面靠的是 `if cloud_field(point) > SURFACE_LEVEL { break }` 的步进，**梯度只喂法线** ✓。

**价格**（2240×1400，同一轮，min）：nocloud 15.10 / volume 15.50 / **surface 13.63** / normals 13.47。改之前是 surface **20.18** 对 volume 13.90，**贵 6.3 ms** ✓。

⚠ 同一轮里 `nocloud` 比 `surface` 还慢 —— 不可能 ⇒ 有 **1.5–2 ms 的时间漂移** ✓。**可信的是"surface ≈ volume、溢价消失"这个相对关系**；绝对值要交错 A/B 才算数 ✓。
### §46.2 三项裁决

| §45.7 | 裁决 | 落法 |
|---|---|---|
| ① `px_ops` 噪声要不要对齐 shader | **不改代码，只改名** | `gradient_noise_3` → **`faded_gradient_noise_3`**。它点乘的是 `smoothstep(local)` 而不是 `local`，所以它**不是** Perlin 梯度噪声，而是把淡入曲线也套进偏移向量的变体；名字里带 `faded` 就是这个意思。shader 那边不动 —— `gradient noise` 本来就是这一类噪声的正式名称。**没有任何生产路径需要两者一致**，而对齐会改动出图并作废全部 PCG 缓存 ⇒ 不值。全仓只有 3 处引用，都在 `px_ops/src/noise.rs` 自身。 |
| ② `gradient.rs` 里那两个失败的生产测试 | **A′：各自只保留还能兑现的部分** | `the_probe_harness…` 降级为烟测（探针能跑 / 能回读 / 能落进壳里），收敛性断言拿掉；`the_analytic_gradient_matches_central_differences` 先挪到 `inner 0.01 / outer 0.06` + 常量覆盖度，**仍然不收敛** ⇒ **删除**。 |
| ③ `ABLATE_ANALYTIC` 单八度调试分支 | **删掉** | 常量、`billows` / `billows_along` 的单八度分支、`Ablate::Analytic`、`--cloud-ablate analytic`、probe 内的两条同款分支、`simple_params` 里的设置，全删。它存在的理由是"简化到能手推"，而 arbiter **不需要简化** ⇒ 没有产品功能、只剩误导。 |
### §46.3 差商这条腿正式退场

小半径配置**没能**救回收敛性 ✓。小半径治的是 `(radius-inner)/span` 的 f32 相消，治不了第三条成因 —— **模板内的门穿越**（场在 4h 上走 `|∇|·4h`，而折点余量在量纲上就不够）。⇒ **差商在任何一个生产相关的配置下都不足以判定梯度对错** ✓✓。它剩下的价值是"**完全不知道解析公式长什么样**"这个独立视角 ⇒ 由 `the_analytic_gradient_keeps_the_kink_convention`（门约定）与 `the_residual_is_attributed_to_one_channel`（逐通道归因）承担，两条保留 ✓。**梯度对错从现在起只有一个判据**：`px_probe --bin field_dual` 的 arbiter（f64 精确、无步长、无折点余量、两侧对照）✓ —— 即 `.\tools\px.ps1 -Target field_dual -Level opt`。**宁可只有一个够格的判据，也不要一个不够格却看起来在判的** ✓。
### §46.4 `#import` 只内联点名的符号 —— 测试侧严格宽松于运行时

运行时报：
```
failed to process shader error: no definition in scope for identifier: `NoiseSample`
    ┌─ shaders/clouds.wgsl:211:102   →  fn sampled_noise_along(...) -> NoiseSample
```

Bevy 只内联 `#import` 里**点名的**符号；`clouds.wgsl` 列了 `fbm_3 / fbm_3_grad / rotate_vector`，没列 `NoiseSample` ✓。补上即好 ✓。

⚠ **它一直坏着，而且测试结构上看不见** ✓✓：`px_render::shaders::assemble`（`tests/common/mod.rs` 只再导出）递归展开**整个模块**（`render_source` 把 `#import` 的模块全文拼进来），而运行时只给点名的符号 ⇒ **测试侧比运行时宽松** ✗。`tests/shaders.rs` 的"每个 shader 都能解析校验"**给不了**这个保证 ✓。

⇒ **shader 的运行时正确性，唯一的仪器是 viewer 的 stderr** ✓。`failed to process shader` 计数必须为 0 —— 这该写成一个**门**，而不是靠人记得去看（待做）✓。
### §46.5 三个流程规约（都是"以为手里的坐标还有效"）

1. `git stash push` **暂存不了已提交的改动** ✓ ⇒ 要取旧版本用 `git show <rev>:<path>` 写进工作区 ✓。
2. `git stash pop` **弹的是栈顶，不一定是你的** ✓ ⇒ 弹之前用 `git stash show` 核对条目 ✓。
3. **`edit` 之后文件就变了，任何基于行号的操作必须重新取** ✓✓ ⇒ 行号一律现取（`grep -n` + 算术），不要记 ✓。
