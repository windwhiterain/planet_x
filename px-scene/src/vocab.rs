//! **场景词汇**：那些"编译器自己消化"的结构键、它们的缺省值，以及量纲上的新类型。
//!
//! 这里的东西有两个来源，别混：
//!
//! 1. **结构键**（[`PLANET_KEYS`] / [`CLOUDS_KEYS`] / [`ATMOSPHERE_KEYS`]）：编译器自己要用
//!    它们（半径、色板、灯、形状档……）⇒ 它们**不进**材质参数表。
//!    ⚠ 这三张表**不是白名单**（§80 第 2 步拆掉的那堵墙）：名字只要在这份 shader 的契约里
//!    就按名字透传；两边都不是才报错（见 [`crate::contract::merge_named`]）。
//! 2. **量纲**（[`Radius`] / [`Spin`] / [`SeaLevel`] / …）：新类型只为了一件事 ——
//!    `radius` 与 `spin` 与 `sea_level` 都是 `f32`，而把它们互相传错在旧形状里是**静默**的。
//!    ⚠ 新类型的**值**就是原来的 `f32`（`.0`）：不引入任何换算，所以逐字节判据不受影响。

use std::collections::BTreeMap;

use px_protocol::scene::Value;

/// 场景倾斜：局部系 → 世界系。**只在这里出现一次**（渲染器里没有这个常数了）。
pub const SYSTEM_TILT: f32 = 0.34;
/// 点光源的射程系数（`|position| × 2.5`）。原来是渲染器的 `SUN_RANGE_FACTOR`。
pub const SUN_RANGE_FACTOR: f32 = 2.5;
/// 天空盒亮度。原来是渲染器的 `SKYBOX_BRIGHTNESS`。
pub const SKYBOX_BRIGHTNESS: f32 = 900.0;
/// 云影的"有多不透明"（覆盖度 1 处压掉约 86% 直接光）与"指定高度"缺省。
pub const CLOUD_SHADOW_GAIN: f32 = 2.0;
pub const CLOUD_SHADOW_HEIGHT: f32 = 0.5;
/// 云壳的缺省内/外半径因子（× 行星半径）。原来是 `px_render::clouds` 里的两个常数。
pub const CLOUD_BASE: f32 = 1.01;
pub const CLOUD_TOP: f32 = 1.06;
/// 天空盒的面尺寸。原来是渲染器里写死的 512。
pub const STARS_FACE: u32 = 512;
/// 环的段数与环带贴图尺寸。原来是渲染器里写死的 384 / 1024×4。
pub const RING_SEGMENTS: u32 = 384;
pub const RING_BAND: (u32, u32) = (1024, 4);

/// 一条量纲新类型。⚠ 它**不做换算、不做 clamp** —— 值就是原来那个数。
macro_rules! quantity {
    ($($name:ident, $doc:literal);* $(;)?) => {
        $(
            #[doc = $doc]
            #[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default)]
            pub struct $name(pub f32);

            impl $name {
                pub const fn new(value: f32) -> Self {
                    Self(value)
                }

                pub const fn get(self) -> f32 {
                    self.0
                }
            }

            impl From<f32> for $name {
                fn from(value: f32) -> Self {
                    Self(value)
                }
            }

            impl From<$name> for f32 {
                fn from(value: $name) -> Self {
                    value.0
                }
            }
        )*
    };
}

quantity! {
    Radius,      "行星半径（世界单位）。";
    Spin,        "行星自转角（弧度），进 `SYSTEM_TILT × spin` 那个合成朝向。";
    SeaLevel,    "海平面（色板贴图分行星 / 海洋的那条线）。";
    Displace,    "位移量（只对色板那条路有意义，网格是 `planet` 图的产物）。";
    RingFactor,  "环的外半径因子（× 行星半径）；`0` = 没有环。";
    ShellFactor, "壳的半径因子（× 行星半径）：云壳的内 / 外半径就是它。";
    Extinction,  "云的光学深度（材质里那一格叫 `density`）。";
    Coverage,    "覆盖度（云形状档的那一档）。";
    Density,     "大气密度（还要乘行星的 `atmo`）。";
    Softness,    "大气软化量。";
}

/// 云壳的分段数：形状档里的整数档。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RaySteps(pub u32);

impl RaySteps {
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

/// **编译器自己消化的结构键**（行星）：半径 / 色板 / 灯 / 消融档这些是拿来**造场景**的，
/// 不是材质参数。
///
/// ⚠ `subdivisions`（2026-09-20 追加）只在 `primitive = "icosphere"` 那一支用：它是**内建球
/// 的细分数**，与材质无关 ⇒ 必须是结构键，否则会被当成"shader 没声明的参数"当场拒。
pub const PLANET_KEYS: [&str; 16] = [
    "palette",
    "displace",
    "sea_level",
    "radius",
    "spin",
    "rings",
    "shadows",
    "cloud_shadow",
    "shadow_height",
    "light_position",
    "light_color",
    "light_intensity",
    "light_range",
    "atmo",
    "ablate",
    "subdivisions",
];

/// **编译器自己消化的结构键**（卫星 part，2026-09-20 第 7 轮加）。
///
/// ⚠ 为什么要有 `kind = "moon"` 这个 part：场景编译器从前只认 planet / clouds / atmosphere，
///   "天上还有一颗小球"这件事**根本写不出来**（参考图上那颗凌日的卫星就是它）。
///   `radius` / `subdivisions` 进几何，`position` 进变换，`spin` 与行星同口径（`orientation()`）；
///   其余一律交给本 part 那份 shader 的契约去判（与 planet 同一条口径）。
/// **灯**（`kind = "light"`）的结构键：位置 / 颜色 / 强度 / 射程 / 要不要投影。
///
/// ⚠ 2026-09-20 加：在这之前编译器**只建那一盏太阳**（`lights: vec![sun]`）—— 于是
/// "行星把光反照到月球暗面"（地球反照）这种**第二光源**在场景里根本没法表达。
/// ⚠ 它**没有 shader**：灯不是物体（`PartFile.shader` 因此是可选字段）。
pub const LIGHT_KEYS: [&str; 5] = ["position", "color", "intensity", "range", "shadows"];

pub const MOON_KEYS: [&str; 4] = ["radius", "subdivisions", "position", "spin"];

/// **编译器自己消化的结构键**（云）：形状档、消融档、风 —— 这些要么进几何、要么与云影同口径。
pub const CLOUDS_KEYS: [&str; 24] = [
    "inner",
    "outer",
    "extinction",
    "coverage",
    "base",
    "top",
    "detail_scale",
    "detail_strength",
    "erode",
    "phase",
    "shadow",
    "steps",
    "bump",
    "seed",
    "slope_scale",
    "taper",
    "coverage_gain",
    "surface_level",
    "bound",
    "gradient",
    "wind",
    "wind_skin",
    "ablate",
    "tint",
];

/// **编译器自己消化的结构键**（大气）：内半径要跟行星半径对账、外半径是因子、密度要乘行星的
/// `atmo`、色要扩成四元数 —— 所以这四个由编译器算。
pub const ATMOSPHERE_KEYS: [&str; 5] = ["inner", "outer", "density", "softness", "tint"];

/// 云的形状档：原来是 `px_render::clouds::CloudShape`（渲染器侧）。缺省值逐项照抄。
#[derive(Clone, Copy)]
pub struct CloudShape {
    pub coverage: f32,
    pub base: f32,
    pub top: f32,
    pub detail_scale: f32,
    pub detail_strength: f32,
    pub erode: f32,
    pub phase: f32,
    pub shadow: f32,
    pub steps: u32,
    pub bump: f32,
    pub seed: u32,
    pub slope_scale: f32,
    pub taper: f32,
    pub coverage_gain: f32,
    pub surface_level: f32,
    pub bound: u32,
    pub gradient: u32,
    pub wind: f32,
    pub wind_skin: f32,
}

impl Default for CloudShape {
    fn default() -> Self {
        Self {
            coverage: 0.35,
            base: 0.06,
            top: 0.62,
            detail_scale: 16.0,
            detail_strength: 0.55,
            erode: 0.0,
            phase: 0.62,
            shadow: 1.0,
            steps: 56,
            bump: 0.85,
            seed: 7,
            slope_scale: 0.12,
            taper: 0.45,
            coverage_gain: 2.6,
            surface_level: 0.20,
            bound: 0,
            gradient: 1,
            wind: 0.0,
            wind_skin: 0.0,
        }
    }
}

/// 消融档：仪器档的名字 → 码（`clouds.wgsl` 里的 `ABLATE_*`）。
pub fn ablate_code(name: &str) -> Result<u32, String> {
    match name {
        "none" => Ok(0),
        "sun" => Ok(1),
        "noise" => Ok(2),
        "fetch" => Ok(3),
        "detail" => Ok(4),
        "surface" => Ok(5),
        "normals" => Ok(6),
        other => Err(format!(
            "消融档只认 none / sun / noise / fetch / detail / surface / normals，不认 '{other}'"
        )),
    }
}

/// 云的参数块：**逐项就是 `clouds.wgsl` 里 `CloudParams` 的那 25 格**（顺序无所谓，
/// 渲染器按名字反射打包）。这里放的是值，不是布局。
pub fn cloud_params(
    shape: CloudShape,
    ablate: u32,
    tint: [f32; 3],
    extinction: f32,
    inner: f32,
    outer: f32,
    shape_orientation: [f32; 4],
) -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("orientation".to_string(), Value::Quad(shape_orientation)),
        (
            "tint".to_string(),
            Value::Quad([tint[0], tint[1], tint[2], 1.0]),
        ),
        ("inner".to_string(), Value::Num(f64::from(inner))),
        ("outer".to_string(), Value::Num(f64::from(outer))),
        ("density".to_string(), Value::Num(f64::from(extinction))),
        (
            "coverage".to_string(),
            Value::Num(f64::from(shape.coverage)),
        ),
        ("base".to_string(), Value::Num(f64::from(shape.base))),
        ("top".to_string(), Value::Num(f64::from(shape.top))),
        (
            "detail_scale".to_string(),
            Value::Num(f64::from(shape.detail_scale)),
        ),
        (
            "detail_strength".to_string(),
            Value::Num(f64::from(shape.detail_strength)),
        ),
        ("erode".to_string(), Value::Num(f64::from(shape.erode))),
        ("phase".to_string(), Value::Num(f64::from(shape.phase))),
        ("shadow".to_string(), Value::Num(f64::from(shape.shadow))),
        ("steps".to_string(), Value::Num(f64::from(shape.steps))),
        ("bump".to_string(), Value::Num(f64::from(shape.bump))),
        ("seed".to_string(), Value::Num(f64::from(shape.seed))),
        ("ablate".to_string(), Value::Num(f64::from(ablate))),
        (
            "slope_scale".to_string(),
            Value::Num(f64::from(shape.slope_scale)),
        ),
        ("taper".to_string(), Value::Num(f64::from(shape.taper))),
        (
            "coverage_gain".to_string(),
            Value::Num(f64::from(shape.coverage_gain)),
        ),
        (
            "surface_level".to_string(),
            Value::Num(f64::from(shape.surface_level)),
        ),
        ("bound".to_string(), Value::Num(f64::from(shape.bound))),
        (
            "gradient".to_string(),
            Value::Num(f64::from(shape.gradient)),
        ),
        ("wind".to_string(), Value::Num(f64::from(shape.wind))),
        (
            "wind_skin".to_string(),
            Value::Num(f64::from(shape.wind_skin)),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 消融档的名字是**内容**，码是 shader 定的；两边对不上就等于切了个不存在的档。
    #[test]
    fn the_ablation_names_map_to_the_codes_the_shader_knows() {
        assert_eq!(ablate_code("none").unwrap(), 0);
        assert_eq!(ablate_code("surface").unwrap(), 5);
        assert_eq!(ablate_code("normals").unwrap(), 6);
        assert!(ablate_code("surfaces").is_err());
    }
}
