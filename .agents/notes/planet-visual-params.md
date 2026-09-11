# 程序化行星的参数化（config 驱动外观）

## 为什么

用户报「**木星土星长得一样，天王星海王星长得一样**」。根因不是参数调得不好，而是
**根本没有参数可填**：

- `src/world.rs::body_kind_for` 写死 `"木星" | "土星" => "gas_giant"`、`"天王星" | "海王星" =>
  "ice_giant"` ⇒ 两对星共用同一个 `BodyKindSpec`，连颜色都同源。
- `web/static/map3d/planet.js` 的 `gasColor`/`icyColor`/… 里那些决定外观的数字
  （带纹频率 `12.0/27.0/52.0`、写死的 `STORM[3]` 坐标、剪切系数）**全是硬编码字面量**，
  不从 config 来。

## 现在的形状

**变体即 class**：`model::SurfaceParams` 是**结构体变体枚举**，每个变体带自己那套字段，
`BodyKindSpec { label, color, accent, atmosphere, radius, emissive, roughness, metalness,
params }`。`class` 和 `banded` 两个字段**删掉了** —— 它们与变体是同一事实的第二份表示。

```ron
"gas_giant_jovian": (label: "气态巨行星", color: "#d8b18c", …, radius: 2.60,
    params: Gas(band_freq: 12.0, band_detail: 1.00, band_contrast: 1.00, shear: 1.7,
                turbulence: 1.20, polar: 0.70, storm_count: 3, storm_size: 0.15,
                storm_strength: 0.90, haze: 0.0)),
```

12 条 kind：`terran / venusian / martian / rocky / lunar / titan / ice_world / dwarf` +
拆出来的 `gas_giant_jovian / gas_giant_saturnian / ice_giant_uranian / ice_giant_neptunian`。

## 三个坑（都踩过）

1. **RON 里新类型变体要双括号**。第一版写成 `Gas(GasParams)`，config 就得写
   `Gas((band_freq: 12.0))` —— 报错 `Expected struct GasParams but found band_freq`。
   结构体变体让 config 少一层括号。**这就是「不要再加 indirect 层」在类型层面的体现。**
2. **`terrain/rockColor` 一条函数服务 4 个 class**（rock/martian/lunar/dwarf）。给每个变体
   自己一套字段后，某个 class 缺少 `relief_shade` 时该 uniform 是 **0**（不是"用默认值"），
   高度分层会静默消失。修法：让共用同一分支的几个变体**带上同一个字段名**，而不是在着色器
   里 `max()` 兜。
3. **参数填了不生效**是最难查的故障：`crater_density` 一度只被冰封卫星用，岩石/卫星/矮行星
   的 config 里填了却没有任何效果。**加了 param 就必须在着色器里真的读它**；前端
   `paramUniforms()` 从字段名自动派生 uniform 名（`band_freq` ⇒ `uBandFreq`）就是为了
   消灭「映射表忘了登记」这条路径。

## 联动点（改参数时四处一起）

| 位置 | 作用 |
|---|---|
| `src/model/body.rs::SurfaceParams` | 字段定义（**唯一的真相**） |
| `config/game.ron` body_kinds | 每种的值 |
| `web/static/map3d/kinds.js` | 变体→`uClass`/`banded`/转速 + `SURFACE_DEFAULTS` 兜底 |
| `web/static/map3d/planet.js` | 顶部 uniform 声明 + 各 `xxxColor()` 里真的读它 |

`VARIANT_CLASS` 与 Rust `class_index()` **必须逐项一致**：不一致的症状是「某类行星突然用了
别的表面公式」，**不报任何错**。

## 顺带修的

- `world.rs` 判「城市是不是轨道空间站」从写死 `matches!(kind, "gas_giant" | "ice_giant")` 改成
  走 `config.body_kind(kind).params.banded()` —— 否则拆 kind 时会静默漏判。
- 新增 `util.js::warpT()`：折叠安全的湍流倍数封装（amt 放大、warpK 反比压小，乘积恒定）。
  气巨的 `turbulence` 因此可以自由调大而不会重新引入折痕。
