use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::model::ResourceDeposit;

/// 2D orbit around a focus. The focus is the **sun** (at the origin) for a
/// planet, or a **parent body** for a satellite (moon).
///
/// The aphelion direction is a unit vector pointing from the focus toward the
/// farthest point of the orbit; perihelion is the opposite direction.
///
/// A satellite's `perihelion_distance`/`aphelion_distance` are measured **from
/// its parent body**, not the sun; `Orbit::position` yields the **local** offset
/// from the focus, and the body's world (heliocentric) position is that offset
/// plus its parent's world position (see [`resolve_positions`]).
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct Orbit {
    /// 近日点距离 (perihelion distance), in AU. For a satellite, measured from
    /// its parent body; for a planet, from the sun.
    pub perihelion_distance: f32,
    /// 远日点距离 (aphelion distance), in AU. Same convention as perihelion.
    pub aphelion_distance: f32,
    /// 远日点方向 (unit vector toward aphelion), in the shared world 2D axes.
    pub aphelion_direction: [f32; 2],
    /// 公转周期 in months.
    pub period: f32,
    /// 本轨道环绕的**母天体名**（唯一名）；`None` = 环绕太阳（日心轨道）。卫星
    /// （月球/欧罗巴/泰坦/卡戎）的 parent 是其所绕行星；其世界位置 = 母天体世界位置
    /// + 本轨道局部位置。`#[serde(default)]` 容忍旧存档无此字段（一律视为环绕太阳）。
    #[serde(default)]
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
        let true_anomaly = 2.0 * ((1.0 + e).sqrt() * half.sin()).atan2((1.0 - e).sqrt() * half.cos());
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
    pub name: String,
    /// 总面积 (total buildable area).
    pub total_area: f64,
    /// 生态容量 (population per unit area).
    pub ecological_capacity: f64,
    /// 建设速度修正 (area built per unit time).
    pub construction_speed_mod: f64,
    /// 建设资源修正 (resources consumed per unit area built).
    pub construction_resource_mod: f64,
    /// 资源 deposits (resource key + area).
    pub resources: Vec<ResourceDeposit>,
}

/// A celestial body hosting zero or more 定居点 (settlement sites), each of which
/// hosts **at most one** city (settlement ↔ city 1:1). A body's settlements are
/// keyed by their unique **name**; a city on this body points at its site via
/// [`City::settlement`] (the settlement's name).
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct Body {
    pub name: String,
    pub orbit: Orbit,
    /// 当前位置 (current position in AU, recomputed each round from `orbit`).
    pub position: [f64; 2],
    /// 天体类型 key（config `body_kinds` 表）。**state 只存一个类型 key**，类型/视觉元数据
    /// （颜色/尺寸/类别/星环等）由 `config/game.ron` 的 `body_kinds` 表提供——这就是
    /// 「视觉属性数据化、body 引用」：引擎不内联任何视觉数据，只记录引用。
    #[serde(default = "default_body_kind")]
    pub kind: String,
    /// 本天体是否渲染星环（如土星的显著环、天王星/海王星的细环）。**intrinsic body 属性**：
    /// 是这颗天体本身的特征（而非类型属性——同为气态巨行星的木星无环），故落在 body 上。
    #[serde(default)]
    pub ring: bool,
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
/// 用它渲染该类型的展示属性。类型是一种「分类」——很多天体共享一个条目却仍能像真实星系
/// （terran / rocky / gas_giant / ice_giant / dwarf …）。纯展示数据：模拟从不读它，只经
/// `/api/meta` 暴露给前端。
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct BodyKindSpec {
    /// 中文类型名（如「气态巨行星」「矮行星」）。
    pub label: String,
    /// 渲染风格 / 着色器分支：`rock` | `terran` | `venus` | `martian` |
    /// `lunar` | `gas` | `ice` | `titan` | `dwarf` —— 前端据此选 shader。
    pub class: String,
    /// 基色（CSS hex）——行星表面主色。
    pub color: String,
    /// 补充色（用于条纹 / 渐变 / 云带 / 极冠）。
    pub accent: String,
    /// 大气辉光 / 边缘光颜色（hex）。
    pub atmosphere: String,
    /// 显示半径乘数（相对 1.0 的类地行星）。随类型给出「合理相对尺寸」。
    pub radius: f64,
    /// 气态巨行星 / 冰巨星的横向色带（由 `class == "gas"` / `"ice"` 的 shader 绘制）。
    pub banded: bool,
    /// 自发光强度 0..1（城市灯光 / 热核地表 / 潮湿大气折射等）。
    pub emissive: f64,
    /// 材质粗糙度 0..1。
    pub roughness: f64,
    /// 材质金属度 0..1。
    pub metalness: f64,
}
