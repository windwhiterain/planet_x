# 39 · 第 16 轮：多光源（渲染器）+ 地球反照（V6）

用户选了 (a)：**"在渲染器里支持多光源"**。这一轮做的是渲染器的活，视觉上换来的是月球的地球反照。

## 一、为什么必须动渲染器

`art/shaders/lib/light.wgsl` 里 `sun_light` **写死读 `clustered_lights.data[0]`**（文件自己写着
"这个渲染器只摆一盏灯""多光源以后再说（§60.5、§64.9）"）⇒ 场景配方里的第二盏灯**进了文档、
进不了画面**（实测：加灯前后出图**逐字节相同**）。

## 二、改了什么（三处，缺一不可）

| 改在哪 | 内容 |
|---|---|
| `px_render/src/group0.rs` | `LightsUniform` 加一格 **`n_point_lights: vec4<u32>`**（`.x` = 盏数），装配处 `cluster.len()`；`FieldLayout` 判据表同步加这一条 |
| `px_shader/src/assemble.rs` | 桩 `LightsStub` 同名同序加 `n_point_lights: vec4<u32>`（布局判据逐字段比 naga 的反射） |
| `art/shaders/lib/light.wgsl` | 新 `light_count()`；把取光参数化成 **`point_light(index, point)`**（`shadow_id = index` —— 影子 cube 的层号就是灯序）；`sun_light` = 第 0 盏（老调用点一行不动） |
| `art/shaders/surface.wgsl` | 漫反射那一支改成**逐灯求和**；⚠ **云影只乘第 0 盏**（云挡的是太阳，不是从旁边那颗行星来的反照光） |
| `px-scene` | 新 part 种类 **`kind = "light"`**（结构键 `vocab::LIGHT_KEYS`）；`PartFile.shader` 改成**可选**（灯没有材质 —— 原先必填，所以"灯"这种 part 根本写不出来） |
| `art/scene/orbit-moon.toml` | 加一盏 `id = "earthshine"`：位置取"行星该在的地方"、偏冷白、强度 8e4（太阳的约一成）、**不投影** |

⚠ 两处踩坑都留在代码注释里：
1. **`Pod` 不许有隐式垫字节**，而 WGSL 侧补 `vec3<u32>` 垫字段会因 16 字节对齐把结构体撑到 48
   （实测报"绑定 32 而 shader 要 48"）⇒ 最后写成**一个 `vec4<u32>`**：两边都是 32 字节、零垫字节。
2. 改完 shader 没重烘就出图 ⇒ 被"include 闭包对不上"当场拦下（那道防呆是有用的）。

## 三、判据（都是"行为保持"这类，不是审美）

| 判据 | 读数 |
|---|---|
| **单光源场景逐字节不变**（这一轮最关键的一条） | `orbit-bare` = **`63184151909371A5`**（与本仓判据图的登记值逐字节相同）；`orbit-gasgiant` = `51CB74AE0045D231`（与本轮交付图逐字节相同）⇒ 逐灯求和在单灯时与旧式子**逐位相同** |
| `cargo test -p px_scene` | **77 passed**（配方改了：新 part 种类 + `shader` 可选） |
| `cargo test -p px_render` | **77 passed**（`LightsUniform` 的布局判据：成员个数/偏移/类型/整块大小逐项比 naga 的反射） |

## 四、看图

`target/shot-moon.png`：暗面（右侧）不再是死黑 —— 一层淡青灰的反照光，坑隐约可见。
夸张档 `target/probe-moon-earthshine-exaggerated.png`（强度 ×4）证明确实是**这盏灯**在起作用
（暗面大片亮起、坑看得清），终值 8e4 是"看得出来但不抢戏"的那一档。

## 五、下一轮

V8（月球更细的一档）或者继续气态行星那条线。⚠ 多光源现在只接进了 `surface.wgsl`
（月球/岩石用的是它）；`clouds` / `atmosphere` / `gasgiant` / `ring` 仍只吃第 0 盏 —— 那是**有意的
一步一验**，要接哪一份再说（它们的"太阳"语义更强，接的时候得逐支想清楚）。
