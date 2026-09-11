// 行星X WebUI — 天体「类型 → 视觉」的解析层。
//
// 视觉是**数据驱动**的：每个 state 天体带一个 `类型` key（config body_kinds 的键），本模块据
// `bodyKinds[body.类型]` 解析出颜色/尺寸/类别/星环/着色器分支，不内联猜测。
// config 缺项/后端没传时退回 DEFAULT_KIND（中性岩石外观）。
//
// **程序化参数住在 `spec.params`**：它是 Rust `model::SurfaceParams` 那个枚举，serde 按
// 「外部标记」序列化成 `{ Gas: { band_freq: 12.0, ... } }` —— 那唯一的键就是**变体名**，
// 而变体名同时决定了着色器分支和「有没有色带」。所以这里不再有 `spec.class` / `spec.banded`
// 这两个字段可读（它们是同一事实的第二份表示，必然漂移）。

import { TUNING } from './tuning.js';

export const DEFAULT_KIND = {
  label: '天体', color: '#8f9bb3', accent: '#5d6678',
  atmosphere: '#1a1a22', radius: 0.6, emissive: 0.0,
  roughness: 1.0, metalness: 0.05,
  params: { Rock: {} },
};

export function specFor(visuals, body) {
  return (visuals && visuals[body.类型]) || DEFAULT_KIND;
}

// 变体名 → 着色器里 uClass 的整数分支（见 planet.js::PLANET_FRAG 的 surfaceColor）。
// 顺序**不能**乱动：改这里等于改所有已生成截图的口径。
// ⚠ 必须与 Rust `model::SurfaceParams::class_index` **逐项一致**。两边不一致时症状是
// 「某一类行星突然用上了别的表面公式」，而且**不报任何错**。
export const VARIANT_CLASS = {
  Rock: 0, Terran: 1, Venus: 2, Martian: 3, Lunar: 4,
  Gas: 5, IceGiant: 6, IceWorld: 6, Titan: 7, Dwarf: 8,
};

// 有横向色带的变体（与 Rust `SurfaceParams::banded` 同口径）。
const BANDED = new Set(['Gas', 'IceGiant']);

/**
 * 取类型参数的**变体名**。`spec.params` 是外部标记枚举 ⇒ 只有唯一的键。
 * 缺项/形状不对时退回 `Rock`（中性外观），绝不抛 —— 渲染层不该因为一个字段崩掉。
 */
export function variantOf(spec) {
  const p = spec && spec.params;
  const k = p && typeof p === 'object' ? Object.keys(p)[0] : null;
  return VARIANT_CLASS[k] != null ? k : 'Rock';
}

// 各变体的参数默认值 —— **必须与 Rust 各 `impl Default` 保持一致**。
// 它们是兜底：config 里字段是全必填的（Rust 侧没有 `#[serde(default)]`），所以正常路径下
// 走不到这里；但后端/前端版本错配时，这里少一个字段会变成 uniform = NaN，整颗星球黑掉，
// 那种故障极难定位。宁可多这三十行。
const SURFACE_DEFAULTS = {
  Rock:      { mottle: 0.70, polar_cap: 0.87, relief_shade: 0.36, crater_density: 0.30 },
  Terran:    { arid: 0.72, polar_cap: 0.80, biome: 0.35, detail: 0.34 },
  Venus:     { swirl_freq: 11.0, swirl_amt: 9.0, streak: 0.28, turbulence: 1.0 },
  Martian:   { dark_region: 0.75, polar_cap: 0.84 },
  Lunar:     { mottle: 0.55, polar_cap: 0.90, relief_shade: 0.40, crater_density: 0.45 },
  Gas:       { band_freq: 12.0, band_detail: 1.0, band_contrast: 1.0, shear: 1.7,
               turbulence: 1.0, polar: 0.70, storm_count: 3, storm_size: 0.13,
               storm_strength: 0.85, haze: 0.0 },
  IceGiant:  { band_freq: 13.0, band_contrast: 1.0, band_weight: 0.75, turbulence: 1.0, spot: 0.0, haze: 0.0 },
  IceWorld:  { mottle: 0.32, crack_freq: 7.0, crack_width: 0.08, crack_amount: 0.55, crater_density: 0.35 },
  Titan:     { lake: 0.70, lake_polar: 0.55, haze: 0.18 },
  Dwarf:     { mottle: 0.70, crater_density: 0.55, polar_cap: 0.88, relief_shade: 0.30 },
};

/** 参数本体 = 默认值叠上 config 给的项。所有字段都有值，调用方不必再写 `?? x`。 */
export function surfaceOf(spec) {
  const v = variantOf(spec);
  const given = (spec && spec.params && spec.params[v]) || {};
  return Object.assign({}, SURFACE_DEFAULTS[v], given);
}

/** 着色器分支整数（`uClass`）。 */
export function classIndexFor(spec) {
  return VARIANT_CLASS[variantOf(spec)];
}

/** 是否画横向色带（`uBanded`）。 */
export function bandedFor(spec) {
  return BANDED.has(variantOf(spec));
}

// 天体显示半径：直接用 config 的 `radius`（相对类地行星）乘一个全局尺度。
// 刻意**不做** `2.4*radius + 0.9` 那种放大：那会让木星(7.1)比太阳(3.2)还大、卫星整个陷进
// 母星里。这里只做整体缩放，保住 config 里「气巨 > 冰巨 > 类地 > 卫星 > 矮行星」的次序。
export function bodyRadius(body, spec) {
  const r = TUNING.radiusScale * (spec.radius || 0.6);
  return Math.min(Math.max(r, TUNING.radiusMin), TUNING.radiusMax);
}

// 轴向倾角（弧度）：只影响自转轴/星环朝向的观感，**不影响**轨道。由天体名确定性派生。
export function axialTilt(name, rnd) {
  const u = rnd(name, 'tilt');
  if (u < 0.14) return (2 + rnd(name, 'tiltA') * 8) * Math.PI / 180;   // 少数几乎不倾
  if (u > 0.88) return (22 + rnd(name, 'tiltB') * 12) * Math.PI / 180; // 少数大倾角（天王星式）
  return (8 + rnd(name, 'tiltC') * 18) * Math.PI / 180;
}

// 自转速度（弧度/秒，渲染时间尺度）。气巨快、岩石慢；逆行按倾角>90°再翻。
// ⚠ 自转**当前是冻结的**（见 index.js：等 state 提供相位）。这个值暂时不生效，
// 留着是因为 `spinners` 仍按它建表。
export function spinSpeed(spec) {
  switch (variantOf(spec)) {
    case 'Gas': return 0.020;
    case 'IceGiant': return 0.016;
    case 'IceWorld': return 0.016;
    case 'Terran': return 0.030;
    case 'Venus': return -0.004;   // 金星逆行且极慢
    case 'Titan': return 0.010;
    case 'Martian': return 0.026;
    default: return 0.008;
  }
}
