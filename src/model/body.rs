use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::model::ResourceDeposit;

/// **绕焦点的二维轨道**：焦点的定义取决于天体层级——行星的焦点是**太阳**（在原点），
/// 卫星（月亮）的焦点是它的**母天体**。
///
/// **远日点方向**是从焦点指向轨道最远点的单位向量；近日点方向与它相反。
///
/// 卫星的 `近日点距离`/`远日点距离` 是**从母天体量的**、不是从太阳；
/// `Orbit::position` 给的是**相对焦点**的局部偏移，天体的世界（日心）坐标 =
/// 那个偏移 + 母天体的世界坐标（见 [`resolve_positions`]）。
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct Orbit {
    /// 近日点距离（AU）：卫星从**母天体**量、行星从**太阳**量。
    #[serde(rename = "近日点距离")]
    pub perihelion_distance: f32,
    /// 远日点距离（AU）：口径同近日点距离。
    #[serde(rename = "远日点距离")]
    pub aphelion_distance: f32,
    /// 远日点方向 (unit vector toward aphelion), in the shared world 2D axes.
    #[serde(rename = "远日点方向")]
    pub aphelion_direction: [f32; 2],
    /// 公转周期 in months.
    #[serde(rename = "公转周期")]
    pub period: f32,
    /// 本轨道环绕的**母天体名**（唯一名）；`None` = 环绕太阳（日心轨道）。卫星
    /// （月球/欧罗巴/泰坦/卡戎）的 parent 是其所绕行星；其世界位置 = 母天体世界位置
    /// + 本轨道局部位置。`#[serde(default)]` 容忍旧存档无此字段（一律视为环绕太阳）。
    #[serde(default)]
    #[serde(rename = "母天体")]
    pub parent: Option<String>,
}

impl Orbit {
    pub fn semi_major_axis(&self) -> f64 {
        (self.perihelion_distance as f64 + self.aphelion_distance as f64) / 2.0
    }

    pub fn eccentricity(&self) -> f64 {
        let a = self.semi_major_axis();
        let peri = self.perihelion_distance as f64;
        let aphe = self.aphelion_distance as f64;
        if peri <= 0.0 || a == 0.0 {
            return 0.0;
        }
        (aphe - peri) / (aphe + peri)
    }

    /// World position (in AU) of the body after `months` elapsed since the
    /// epoch. At `months == 0` the body sits at perihelion.
    pub fn position(&self, months: f32) -> [f64; 2] {
        let m = self.semi_major_axis();
        let e = self.eccentricity();
        let period = self.period as f64;
        let mean_motion = std::f64::consts::TAU / period;
        // Mean anomaly at the given time (radians), starting at perihelion.
        let mut mean_anomaly = (mean_motion * months as f64) % std::f64::consts::TAU;
        if mean_anomaly < 0.0 {
            mean_anomaly += std::f64::consts::TAU;
        }
        // Solve Kepler's equation  M = E - e * sin(E)  for the eccentric anomaly.
        let mut ecc_anomaly = mean_anomaly;
        for _ in 0..24 {
            let f = ecc_anomaly - e * ecc_anomaly.sin() - mean_anomaly;
            let fp = 1.0 - e * ecc_anomaly.cos();
            let delta = f / fp;
            ecc_anomaly -= delta;
            if delta.abs() < 1e-9 {
                break;
            }
        }
        // True anomaly from perihelion.
        let half = ecc_anomaly / 2.0;
        let true_anomaly =
            2.0 * ((1.0 + e).sqrt() * half.sin()).atan2((1.0 - e).sqrt() * half.cos());
        let r = m * (1.0 - e * ecc_anomaly.cos());

        // Perihelion direction is the opposite of the aphelion direction.
        let aphe = normalize2(self.aphelion_direction);
        let peri = [-aphe[0], -aphe[1]];
        let perp = [-peri[1], peri[0]];

        let cx = peri[0] * true_anomaly.cos() + perp[0] * true_anomaly.sin();
        let cy = peri[1] * true_anomaly.cos() + perp[1] * true_anomaly.sin();
        [cx * r, cy * r]
    }
}

fn normalize2(v: [f32; 2]) -> [f64; 2] {
    let len = ((v[0] as f64).powi(2) + (v[1] as f64).powi(2)).sqrt();
    if len <= 1e-12 {
        [1.0, 0.0]
    } else {
        [v[0] as f64 / len, v[1] as f64 / len]
    }
}

/// A habitable place (定居点) on a body. 定居点与城市一一对应：一个定居点至多
/// 容纳一座城市（见 [`City::settlement`]）。它的面积有限——坐落在其上的城市的
/// 建筑必须装得下；它的资源矿藏限定本定居点上采矿的上限。
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct Settlement {
    /// 定居点名（地球的五大城市群各占一个定居点；气态巨行星的定居点为轨道空间站）。
    #[serde(rename = "定居点")]
    pub name: String,
    /// 总面积 (total buildable area).
    #[serde(rename = "总面积")]
    pub total_area: f64,
    /// 生态容量 (population per unit area).
    #[serde(rename = "生态容量")]
    pub ecological_capacity: f64,
    /// 建设速度修正 (area built per unit time).
    #[serde(rename = "建设速度修正")]
    pub construction_speed_mod: f64,
    /// 建设资源修正 (resources consumed per unit area built).
    #[serde(rename = "建设资源修正")]
    pub construction_resource_mod: f64,
    /// 资源 deposits (resource key + area).
    #[serde(rename = "资源")]
    pub resources: Vec<ResourceDeposit>,
}

/// A celestial body hosting zero or more 定居点 (settlement sites), each of which
/// hosts **at most one** city (settlement ↔ city 1:1). A body's settlements are
/// keyed by their unique **name**; a city on this body points at its site via
/// [`City::settlement`] (the settlement's name).
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct Body {
    #[serde(rename = "天体名")]
    pub name: String,
    /// 天体类型 key（config `body_kinds` 表）。**state 只存一个类型 key**，类型/视觉元数据
    /// （颜色/尺寸/类别/星环等）由 `config/game.ron` 的 `body_kinds` 表提供——这就是
    /// 「视觉属性数据化、body 引用」：引擎不内联任何视觉数据，只记录引用。
    #[serde(default = "default_body_kind")]
    #[serde(rename = "类型")]
    pub kind: String,
    /// 本天体是否渲染星环（如土星的显著环、天王星/海王星的细环）。**intrinsic body 属性**：
    /// 是这颗天体本身的特征（而非类型属性——同为气态巨行星的木星无环），故落在 body 上。
    #[serde(default)]
    #[serde(rename = "星环")]
    pub ring: bool,
    #[serde(rename = "轨道")]
    pub orbit: Orbit,
    /// 当前位置 (current position in AU, recomputed each round from `orbit`).
    #[serde(rename = "位置")]
    pub position: [f64; 2],
    #[serde(rename = "定居点")]
    pub settlements: Vec<Settlement>,
}

fn default_body_kind() -> String {
    "rocky".to_string()
}

impl Body {
    /// Look up a 定居点 on this body by its unique **name** (settlement ↔ city 1:1).
    pub fn settlement(&self, name: &str) -> Option<&Settlement> {
        self.settlements.iter().find(|s| s.name == name)
    }
}

/// 把每颗天体的 `position` 解析为**世界（日心）坐标**：`Orbit::position(months)` 只给出
/// 相对本轨道焦点（太阳或母天体）的**局部**偏移，这里对每颗天体累加其母天体已经解析好的
/// 世界位置得到实际坐标；无母天体的天体其世界位置即局部位置。
///
/// 调用前提：`bodies` 里母天体排在卫星之前（world 生成的 parent-first 顺序；本函数按
/// 下标正序遍历，故能直接读到已解析的母天体位置）。对单层卫星系统（所有卫星的母天体都
/// 是日心行星）这已足够；若要支持嵌套卫星，需按拓扑序迭代或递归解析。
pub fn resolve_positions(bodies: &mut [Body], months: f32) {
    for i in 0..bodies.len() {
        let local = bodies[i].orbit.position(months);
        let parent_pos = match bodies[i].orbit.parent.as_ref() {
            Some(pname) => bodies
                .iter()
                .position(|b| &b.name == pname)
                .map(|pi| bodies[pi].position)
                .unwrap_or([0.0, 0.0]),
            None => [0.0, 0.0],
        };
        bodies[i].position = [parent_pos[0] + local[0], parent_pos[1] + local[1]];
    }
}

/// 一个天体/行星**类型**的视觉与类型元数据，定义在 `config/game.ron` 的 `body_kinds` 表。
///
/// 引擎只在每个 [`Body`] 上存 **类型 key**（见 [`Body::kind`]）；这张表携带前端（three.js）
/// 用它渲染该类型的展示属性。纯展示数据：模拟从不读它，只经 `/api/meta` 暴露给前端。
///
/// 着色器分支与「是否画色带」**由 [`BodyKindSpec::params`] 的变体推出**，不在这里各存一份：
/// 那两个字段与变体是同一事实的第二份表示，必然漂移（改了一处忘了另一处，就是「木星土星
/// 长得一模一样」那类问题的温床）。
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct BodyKindSpec {
    /// 中文类型名（如「气态巨行星」「矮行星」）。
    pub label: String,
    /// 基色（CSS hex）——行星表面主色。
    pub color: String,
    /// 补充色（用于条纹 / 渐变 / 云带 / 极冠）。
    pub accent: String,
    /// 大气辉光 / 边缘光颜色（hex）。
    pub atmosphere: String,
    /// 显示半径乘数（相对 1.0 的类地行星）。随类型给出「合理相对尺寸」。
    pub radius: f64,
    /// 自发光强度 0..1（城市灯光 / 热核地表 / 潮湿大气折射等）。
    pub emissive: f64,
    /// 材质粗糙度 0..1。
    pub roughness: f64,
    /// 材质金属度 0..1。
    pub metalness: f64,
    /// 该类型的**程序化表面参数**（见 [`SurfaceParams`]）。
    pub params: SurfaceParams,
}

/// 程序化表面的参数。**变体即着色器分支**，每个变体带**自己那套**字段。
///
/// 为什么是枚举而不是「一堆 Option 字段的扁平袋」：各类型的参数集交集很小 ——
/// 气巨要的是带纹频率 / 风暴数，类地要的是海平面 / 荒漠 / 极冠，冰封卫星要的是裂纹宽度 ——
/// 摊平会逼所有类型共用一套字段名（只能靠 `gas_` 前缀硬凑），而「这个字段属于哪个 class」
/// 会退化成隐式知识。
///
/// 为什么用**结构体变体**（`Gas { .. }`）而不是「新类型变体包一个 `GasParams` 结构」：
/// 后者在 RON 里必须写成 `Gas((band_freq: 12.0, ..))` —— **双括号**，那层多余的括号就是
/// 一层多余的间接，还容易写错（第一版就是这么栽的：`Expected struct GasParams but found
/// band_freq`）。结构体变体让 config 写成 `Gas(band_freq: 12.0, ..)`，一层就是一层。
///
/// ⚠ 字段名有意与前端 uniform 对齐（`band_freq` ↔ `uBandFreq`），这样 `planet.js` 里
/// 每个字面量都能一眼查到它现在从 config 的哪一项来。新增字段时两边一起改。
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub enum SurfaceParams {
    /// 岩石行星（着色器 `rockColor`）。
    Rock {
        /// 大尺度色斑强度 0..1。
        mottle: f64,
        /// 极冠范围。
        polar_cap: f64,
        /// 高度分层强度（高处亮、洼地暗）0..1。
        relief_shade: f64,
        /// 陨石坑密度 0..1。
        crater_density: f64,
    },
    /// 类地宜居（`terranColor`）。
    Terran {
        /// 干旱带强度 0..1（沙漠占比）。
        arid: f64,
        /// 极冠范围（越大极冠越往低纬长）。
        polar_cap: f64,
        /// 生物群系色差强度 0..1。
        biome: f64,
        /// 高频地表细节强度 0..1。
        detail: f64,
    },
    /// 浓厚大气（`venusColor`）。
    Venus {
        /// 纬向涡旋频率。
        swirl_freq: f64,
        /// 涡旋对色相的扰动幅度。
        swirl_amt: f64,
        /// 高速条纹强度 0..1（金星大气 4 天绕一圈）。
        streak: f64,
        /// 域扰动倍数（1.0 = 基准口径）。
        turbulence: f64,
    },
    /// 红色荒漠（`rockColor` 的火星分支）。
    Martian {
        /// 暗反照率区强度 0..1（Syrtis Major 那种）。
        dark_region: f64,
        /// 极冠范围。
        polar_cap: f64,
    },
    /// 灰岩卫星（`rockColor`）。
    Lunar {
        /// 大尺度色斑强度 0..1。
        mottle: f64,
        /// 极冠范围。
        polar_cap: f64,
        /// 高度分层强度 0..1（月海就是「洼地压暗」，与岩石共用同一字段）。
        relief_shade: f64,
        /// 陨石坑密度 0..1。
        crater_density: f64,
    },
    /// 气态巨行星（`gasColor`：强纬向色带 + 风暴）。默认档见 `default_gas()`。
    Gas {
        /// 纬向带基础频率（三条带 = `freq` / `freq*2.25` / `freq*4.33`）。
        band_freq: f64,
        /// 细带层权重：1.0 = 三层 0.50/0.32/0.18，0 = 只剩基频。
        band_detail: f64,
        /// 带纹对比：1.0 = 原窗口 (0.26, 0.80)，0 = 近乎均匀的一颗球。
        band_contrast: f64,
        /// 纬向剪切。**不能大**：大了带纹会被撕成斑块。
        shear: f64,
        /// 域扰动倍数（1.0 = 基准口径；走 `warpT`，自动折叠安全）。
        turbulence: f64,
        /// 极区压暗 0..1。
        polar: f64,
        /// 风暴数 0..=3（第 0 颗是「大红斑」那种最大的）。
        storm_count: i32,
        /// 风暴半径（第 i 颗再乘 `1 + 0.38*i`）。
        storm_size: f64,
        /// 风暴强度 0..1。
        storm_strength: f64,
        /// 整体雾霾 / 去饱和 0..1（土星比木星朦胧）。
        haze: f64,
    },
    /// 冰巨星（`icyColor` 的带状分支）。
    IceGiant {
        /// 纬向带频率。
        band_freq: f64,
        /// 带纹对比 0..1。天王星给到接近 0。
        band_contrast: f64,
        /// 带纹在混色里的权重（越大越显色）。
        band_weight: f64,
        /// 域扰动倍数（1.0 = 基准口径）。
        turbulence: f64,
        /// 暗斑（大暗斑那种）强度 0..1。
        spot: f64,
        /// 整体雾霾 / 去饱和 0..1。
        haze: f64,
    },
    /// 冰封卫星（`icyColor` 的无带分支：光滑冰面 + 裂纹 + 坑）。
    IceWorld {
        /// 大尺度色斑强度 0..1。
        mottle: f64,
        /// 裂纹频率。
        crack_freq: f64,
        /// 裂纹宽度（0.08 = 原口径）。
        crack_width: f64,
        /// 裂纹亮度 0..1。
        crack_amount: f64,
        /// 陨石坑密度 0..1。
        crater_density: f64,
    },
    /// 雾霾卫星（`titanColor`）。
    Titan {
        /// 甲烷湖强度 0..1。
        lake: f64,
        /// 极区湖的集中程度（越大越集中在极点）。
        lake_polar: f64,
        /// 整体雾霾 0..1。
        haze: f64,
    },
    /// 柯伊伯带矮行星（`rockColor`）。
    Dwarf {
        /// 大尺度色斑强度 0..1。
        mottle: f64,
        /// 陨石坑密度 0..1。
        crater_density: f64,
        /// 极冠范围。
        polar_cap: f64,
        /// 高度分层强度 0..1。
        relief_shade: f64,
    },
}

impl SurfaceParams {
    /// 着色器分支整数 —— 与前端 `kinds.js::VARIANT_CLASS` 同口径，
    /// 即 `planet.js::PLANET_FRAG` 里的 `uClass`。**顺序不能乱动**：改它等于改所有
    /// 已生成截图的口径。
    pub fn class_index(&self) -> i32 {
        match self {
            Self::Rock { .. } => 0,
            Self::Terran { .. } => 1,
            Self::Venus { .. } => 2,
            Self::Martian { .. } => 3,
            Self::Lunar { .. } => 4,
            Self::Gas { .. } => 5,
            Self::IceGiant { .. } | Self::IceWorld { .. } => 6,
            Self::Titan { .. } => 7,
            Self::Dwarf { .. } => 8,
        }
    }

    /// 是否画横向色带（前端 `uBanded`）。只有气巨与冰巨星有。
    pub fn banded(&self) -> bool {
        matches!(self, Self::Gas { .. } | Self::IceGiant { .. })
    }

    /// 自转速度分组名（见前端 `kinds.js::spinSpeed`）。比 `class_index` 细一档：
    /// 冰巨星与冰封卫星同属 `ice` 着色器分支，但转速不该一样。
    pub fn spin_group(&self) -> &'static str {
        match self {
            Self::Gas { .. } => "gas",
            Self::IceGiant { .. } => "ice_giant",
            Self::IceWorld { .. } => "ice_world",
            Self::Terran { .. } => "terran",
            Self::Venus { .. } => "venus",
            Self::Titan { .. } => "titan",
            Self::Martian { .. } => "martian",
            Self::Rock { .. } | Self::Lunar { .. } | Self::Dwarf { .. } => "rock",
        }
    }
}
