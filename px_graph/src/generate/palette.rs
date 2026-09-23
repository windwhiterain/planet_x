//! 色板：`Palette` 本身（名字、大气三件套、默认参数），以及它下面那张色带
//! （`ramp` 线性插值 + GAS / WATER / LAND / FROZEN / SHEET / LAVA_ROCK / BASIN / DUNE）。
//!
//! 边界：这一档只管"颜色从哪来"。"t 怎么算、往哪儿混"在 `super::shade`。
//! 停靠点表的常量名是**地貌**的名字，不都是 palette 名（`GAS` 只给气态那份用，
//! `WATER`/`LAND` 归 Rocky、`FROZEN`/`SHEET` 归 Ice、`BASIN`/`DUNE` 归 Desert），
//! 别拿名字去配对；表和插值都逐字搬自 `px_render/src/planet.rs`。
//! `ramp` / `mix` / 这几张表只有 shade 那一档取，所以都是 `pub(super)`。

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Palette {
    Rocky,
    Gas,
    Ice,
    Lava,
    Desert,
    /// **月球灰**：风化层（regolith）由暗到亮的灰，加一层偏蓝的**月海**（低处）。
    ///
    /// ⚠ 2026-09-20 加的这一档：`moon` 图（三层陨坑）配 rocky 会出绿——那是 LAND 色带里
    ///   植被那一段（`[0.259, 0.435, 0.216]`）。无大气天体的地貌没有植被，所以它需要
    ///   自己那一档色带，而不是去改 `Rocky`（改它会**换掉全部既有场景的贴图字节**）。
    Moon,
}

impl Palette {
    pub const NAMES: [&'static str; 6] = ["rocky", "gas", "ice", "lava", "desert", "moon"];

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "rocky" => Some(Self::Rocky),
            "gas" => Some(Self::Gas),
            "ice" => Some(Self::Ice),
            "lava" => Some(Self::Lava),
            "desert" => Some(Self::Desert),
            "moon" => Some(Self::Moon),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Rocky => "rocky",
            Self::Gas => "gas",
            Self::Ice => "ice",
            Self::Lava => "lava",
            Self::Desert => "desert",
            Self::Moon => "moon",
        }
    }

    /// 大气的三件套：**色调（线性 RGB）+ 密度 + 软度**。
    ///
    /// 渲染器那份返回的是 bevy 的 `LinearRgba`；`px_graph` 不认识 bevy，所以色调那一项
    /// 落成 `[f32; 3]`。三个通道的**数值**与渲染器逐字相同，取用侧自己包成 `LinearRgba`
    /// （`LinearRgba::rgb(tint[0], tint[1], tint[2])`）。
    pub fn atmosphere(self) -> ([f32; 3], f32, f32) {
        match self {
            Self::Rocky => ([0.44, 0.64, 0.98], 0.300, 0.50),
            Self::Gas => ([1.00, 0.86, 0.62], 0.430, 0.40),
            Self::Ice => ([0.66, 0.87, 1.00], 0.270, 0.60),
            Self::Lava => ([1.00, 0.46, 0.20], 0.340, 0.45),
            Self::Desert => ([0.97, 0.79, 0.55], 0.340, 0.46),
            // 几乎没有大气：一层很淡的灰蓝（缺省是"薄到只剩贴着地平线的那一圈"）。
            Self::Moon => ([0.62, 0.66, 0.74], 0.060, 0.70),
        }
    }

    pub fn defaults(self) -> (f32, f32, f32) {
        match self {
            Self::Rocky => (0.075, 0.520, 0.0),
            Self::Gas => (0.010, 0.450, 2.35),
            Self::Ice => (0.055, 0.500, 0.0),
            Self::Lava => (0.095, 0.480, 0.0),
            Self::Desert => (0.085, 0.520, 0.0),
            // 月海线 0.38：比这低的算月海（暗）、比这高的算高地（亮）。
            Self::Moon => (0.045, 0.380, 0.0),
        }
    }
}

// ---------------------------------------------------------------------------
// 色带（逐字搬自 px_render/src/planet.rs）
// ---------------------------------------------------------------------------

pub(super) fn ramp(stops: &[(f32, [f32; 3])], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    let mut previous = stops[0];
    for stop in stops {
        if t <= stop.0 {
            let span = stop.0 - previous.0;
            let k = if span.abs() < f32::EPSILON {
                0.0
            } else {
                (t - previous.0) / span
            };
            return [
                previous.1[0] + (stop.1[0] - previous.1[0]) * k,
                previous.1[1] + (stop.1[1] - previous.1[1]) * k,
                previous.1[2] + (stop.1[2] - previous.1[2]) * k,
            ];
        }
        previous = *stop;
    }
    stops[stops.len() - 1].1
}

pub(super) fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

pub(super) const GAS: &[(f32, [f32; 3])] = &[
    (0.00, [0.286, 0.208, 0.145]),
    (0.30, [0.545, 0.427, 0.318]),
    (0.55, [0.804, 0.729, 0.616]),
    (0.78, [0.902, 0.812, 0.663]),
    (1.00, [0.616, 0.475, 0.353]),
];

pub(super) const WATER: &[(f32, [f32; 3])] = &[
    (0.00, [0.006, 0.020, 0.070]),
    (0.55, [0.031, 0.109, 0.259]),
    (1.00, [0.153, 0.353, 0.478]),
];

pub(super) const LAND: &[(f32, [f32; 3])] = &[
    (0.00, [0.706, 0.663, 0.502]),
    (0.05, [0.259, 0.435, 0.216]),
    (0.32, [0.169, 0.325, 0.153]),
    (0.60, [0.404, 0.376, 0.318]),
    (0.82, [0.612, 0.596, 0.573]),
    (1.00, [0.965, 0.973, 1.000]),
];

pub(super) const FROZEN: &[(f32, [f32; 3])] = &[
    (0.00, [0.086, 0.153, 0.227]),
    (0.55, [0.278, 0.443, 0.565]),
    (1.00, [0.686, 0.804, 0.867]),
];

pub(super) const SHEET: &[(f32, [f32; 3])] = &[
    (0.00, [0.549, 0.667, 0.741]),
    (0.45, [0.749, 0.847, 0.902]),
    (1.00, [0.965, 0.988, 1.000]),
];

pub(super) const LAVA_ROCK: &[(f32, [f32; 3])] = &[
    (0.00, [0.035, 0.027, 0.027]),
    (0.35, [0.086, 0.063, 0.055]),
    (0.60, [0.176, 0.125, 0.098]),
    (0.82, [0.290, 0.208, 0.157]),
    (1.00, [0.427, 0.353, 0.310]),
];

pub(super) const BASIN: &[(f32, [f32; 3])] =
    &[(0.00, [0.165, 0.122, 0.094]), (1.00, [0.376, 0.271, 0.184])];

pub(super) const DUNE: &[(f32, [f32; 3])] = &[
    (0.00, [0.310, 0.196, 0.122]),
    (0.18, [0.475, 0.302, 0.173]),
    (0.42, [0.706, 0.510, 0.290]),
    (0.70, [0.816, 0.663, 0.435]),
    (1.00, [0.882, 0.796, 0.651]),
];

/// **月海**（`Palette::Moon` 的低处）：暗、略偏蓝（玄武岩熔岩平原的观测色）。
pub(super) const MARE: &[(f32, [f32; 3])] = &[
    (0.00, [0.106, 0.110, 0.125]),
    (0.55, [0.157, 0.161, 0.176]),
    (1.00, [0.216, 0.220, 0.235]),
];

/// **风化层**（`Palette::Moon` 的高处）：中性灰，越亮越接近撞击溅射物的白。
pub(super) const REGOLITH: &[(f32, [f32; 3])] = &[
    (0.00, [0.259, 0.255, 0.247]),
    (0.35, [0.412, 0.404, 0.392]),
    (0.70, [0.596, 0.588, 0.573]),
    (1.00, [0.804, 0.800, 0.792]),
];
