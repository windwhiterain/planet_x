# 33 · 第 10 轮：第三层次由**图**组合（第二个 cube 槽）+ 细丝层

延续用户那条口径：**"大气不够丰富，层次单一"**。上一轮加的第二尺度是**同一张条带图缩放**
（shader 里的小把戏）—— 这一轮把第三层次做成**另一张独立的场**，走**第二个 cube 槽**进材质：
**层次由图组合**，材质只负责把几层叠起来。

---

## 一、系统：材质可以吃**第二张立方图**（`TEXTURE_SLOTS` 里空着的那一对 cube）

⚠ 先说清"缺的是什么"：`TEXTURE_SLOTS` 有 **12 格（8×2D + 4×cube）**，cube 是 5 / 7 / 21 / 23 ——
气态巨行星从前只用了 **7**，**21 号那一对一直空着**。所以这不是"加宽契约"，而是
**把已经留好的那一格接上**：图侧烘第二张 + 场景编译器把成员接过去。

| 改在哪 | 内容 |
|---|---|
| `px_graphs/src/bin/gasgiant.rs` | 新节点 `filaments_raw`（`field.fbm`，频率 9.5、`zonal` 4.5）＋ `filaments`（用**同一个** `swirl` 推歪它 ⇒ 细丝跟着条带走，而不是铺一层无关噪点） |
| `art/gasgiant/{filaments_raw,filaments}.toml` | 两个新参数文件。⚠ 踩了一个坑：先只写了 `filaments.toml` 且内容是 fbm 参数，而**节点名 `filaments` 是那个 warp** ⇒ 驱动报"`unknown field frequency, expected one of strength/lateral/probe`"——参数文件名必须与**节点名**对齐 |
| `px-scene/src/recipe.rs` | 本体那条路上多一支：可选的成员 **`detail`** → `field_cube` → `TextureRef::new(21, …)`。⚠ **可选**：不给成员就吃渲染器的兜底贴图 ⇒ 没有这一层的场景产物**逐字节不变** |
| `art/shaders/gasgiant.wgsl` | 声明 `@binding(21) filament_map` + `@binding(22) filament_sampler`，新栏 `filament_gain`；细丝与第二尺度**同一条分工**（乘在 `edge` 上 ⇒ 只长在带的过渡带上） |
| `art/scene/orbit-gasgiant.toml` | `members = { …, bands = "gasgiant::mixed", detail = "gasgiant::filaments" }`、`filament_gain = 0.34` |

**实测证据**（场景编译的审计行，第二张图确实接上了 21 号格）：

```text
[planet] … ｜贴图 bands@7=generated/gas_bands@304d2abbcfbb detail@21=generated/gas_filaments@14671d84f852
```

出图 `target/shot-gasgiant.png`（960×640，506833 B，sha256 前 16 `021454146E90B6C3`）：
带的内部有了细丝结构、边缘更碎 ⇒ 三个尺度叠在一起（粗带 / 同图缩放的第二尺度 / 独立细丝场）。

## 二、判据（只跑受影响的）

| 判据 | 读数 |
|---|---|
| 重烘 8 份 shader 逐份对键 | **只有 `gasgiant` 变**（`287837a2fc00` → `de7ab2d3dd37`） |
| `cargo test -p px-scene` | **43 passed**（编译器改了：多一支可选成员） |
| `art/anchor` §三 六格重烘 | **6/6 与第十次登记值相同** ⇒ 新成员路径对没有它的场景是**惰性的** |
| §一 J1 六张重出图 | **6/6 逐字节不变** |
| `gasgiant` 观感 | 同机位出图（见上） |

⇒ 这一轮**不需要重登记**：动的是一份只被气态行星用到的 shader（`gasgiant` 不在 §三/J1 里）+ 一个
新增的可选成员路径。

## 三、记下的两处

1. **参数文件名必须与节点名对齐**：`filaments` 那个节点是 warp，我却把 fbm 的参数写进了
   `filaments.toml` ⇒ 驱动报的是"字段不认识"，而不是"文件对不上节点"。⇒ 拆成
   `filaments_raw.toml`（fbm）与 `filaments.toml`（warp）。
2. 材质的**贴图格是固定超集**（12 格），所以"多一层"的天花板是 4 张 cube —— 今天用了 2 张
   （7 / 21），还剩 5 / 23。

## 四、待办与一个要用户拍板的点

* **V14（影偏糊）**：现在查清了 —— `point_shadow_textures.size = "1024x1024"` 写在
  **共享帧图** `art/frame/default.toml` 里，而它的注释明确写着"帧图不是内容，六个场景共用；
  抄进每个场景只会让共用在暗处漂移"。且 1024 是**锚 oracle 的策略**——§一 J1 六张判据图就是
  锚的行为。⇒ 要"更利落的影"就得**动共享帧图**（会改 §三 + J1 六张的像素，属有意变更要重登记）。
  这是**产品决策**，先问用户，不擅自动。
* 剩下：**V9**（可喂章：章的取样域口径要先问用户）、V3（极区）、V6（月球反照）、V7（天王星配色档）、
  V8（月球更细一档）、V10（独立椭圆涡）、S2c（结构张量仪器）、S2b（更多 part 种类）。
