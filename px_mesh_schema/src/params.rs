//! 两个网格算子的参数：立方球网格（`mesh.cubesphere`）与等值面代理（`mesh.proxy`）。

use serde::Serialize;
use serde::de::DeserializeOwned;

pub const CUBESPHERE: &str = "mesh.cubesphere";
pub const PROXY: &str = "mesh.proxy";

pub mod cubesphere {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
    #[serde(default)]
    pub struct Params {
        pub subdivisions: u32,
        pub radius: f32,
        pub displace: f32,
        pub sea_level: f32,
        pub flat_sea: bool,
    }

    impl Default for Params {
        fn default() -> Self {
            Self {
                subdivisions: 128,
                radius: 1.0,
                displace: 0.075,
                sea_level: 0.52,
                flat_sea: false,
            }
        }
    }
}

pub mod proxy {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
    #[serde(default, deny_unknown_fields)]
    pub struct Params {
        /// 等值面高度。体积里存的是归一化后的场 `(粗场 - τ) / L`，所以默认 0.0。
        pub level: f32,
        /// 每个面的分辨率：每轴 `2^depth + 1` 个采样点。代理要粗，6~7 就够。
        pub depth: u32,
        /// 焊缝焊接容差（世界单位）。远小于一个格子，只吃掉两块之间 1 ULP 级的偏差。
        pub weld: f32,
        /// 几何外扩：每个顶点沿外法线往外推这么远（世界单位）。
        ///
        /// 为什么只能靠几何补：稠密 MC 的等值面比解析等值面**浅**（实测外边界余量 −0.0017），
        /// 云的轮廓上因此丢一圈；而体积里存的是 `(场−τ)/L`，`level` 往外偏的下限就是壁上那层
        /// `−τ/L`（再低提取器直接报"体积里没有这个等值面"）⇒ 偏 `level` 补不回来。
        ///
        /// 0 不进键（`skip_serializing_if`）：不写这个字段的老档输出逐位不变，键也不该变。
        #[serde(skip_serializing_if = "is_zero")]
        pub offset: f32,
    }

    /// `skip_serializing_if` 要的那条：0 等价于"没写这个字段"。
    fn is_zero(value: &f32) -> bool {
        *value == 0.0
    }

    impl Default for Params {
        fn default() -> Self {
            Self {
                level: 0.0,
                depth: 6,
                weld: 1e-4,
                offset: 0.0,
            }
        }
    }
}

/// TOML 原文 → 键用的规范 JSON（`None` = 文件不存在 ⇒ 默认值）。
pub fn canonical(op_id: &str, toml_text: Option<&str>) -> Result<String, String> {
    match op_id {
        CUBESPHERE => one::<cubesphere::Params>(toml_text),
        PROXY => one::<proxy::Params>(toml_text),
        other => Err(format!("px_mesh_schema 不认识算子 {other}")),
    }
}

pub fn parse<P: Serialize + DeserializeOwned + Default>(toml_text: Option<&str>) -> Result<P, String> {
    match toml_text {
        Some(text) => toml::from_str(text).map_err(|err| err.to_string()),
        None => Ok(P::default()),
    }
}

fn one<P: Serialize + DeserializeOwned + Default>(toml_text: Option<&str>) -> Result<String, String> {
    Ok(px_graph_schema::canonical_params(&parse::<P>(toml_text)?))
}
