// 行星X WebUI — 天体「类型 → 视觉」的解析层。
//
// 视觉是**数据驱动**的：每个 state 天体带一个 `类型` key（config body_kinds 的键），本模块据
// `bodyKinds[body.类型]` 解析出颜色/尺寸/类别/星环/着色器分支，不内联猜测。
// config 缺项/后端没传时退回 DEFAULT_KIND（中性岩石外观）。

import { TUNING } from './tuning.js';

export const DEFAULT_KIND = {
  label: '天体', class: 'rock', color: '#8f9bb3', accent: '#5d6678',
  atmosphere: '#1a1a22', radius: 0.6, banded: false, emissive: 0.0,
  roughness: 1.0, metalness: 0.05,
};

export function specFor(visuals, body) {
  return (visuals && visuals[body.类型]) || DEFAULT_KIND;
}

// class 字符串 → 着色器里 uClass 的整数分支（见 planet.js::PLANET_FRAG 的 surfaceColor）。
// 顺序**不能**乱动：改这里等于改所有已生成截图的口径。
export const CLASS_INDEX = {
  rock: 0, terran: 1, venus: 2, martian: 3, lunar: 4,
  gas: 5, ice: 6, titan: 7, dwarf: 8,
};
export function classIndex(cls) {
  return CLASS_INDEX[cls] != null ? CLASS_INDEX[cls] : 0;
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
export function spinSpeed(spec) {
  switch (spec.class) {
    case 'gas': return 0.020;
    case 'ice': return 0.016;
    case 'terran': return 0.030;
    case 'venus': return -0.004;   // 金星逆行且极慢
    case 'titan': return 0.010;
    case 'martian': return 0.026;
    default: return 0.008;
  }
}
