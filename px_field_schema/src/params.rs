//! 七个场算子的**参数**：TOML 长什么样、默认值是多少、算子 id 叫什么。
//!
//! ⚠ 参数住在 schema 这一侧，是为了让**算键不必求值**（§17.1）：驱动拿描述符表里的
//! `canonical_params` 把 TOML 直接化成规范 JSON，`node_key` 用的就是这一份；算子求值时
//! 收到的也是同一份 JSON。两侧一份口径，不会出现「键里的参数」与「算出来的参数」不是同一个。

use serde::Serialize;
use serde::de::DeserializeOwned;

pub const CONSTANT: &str = "field.constant";
pub const FBM: &str = "field.fbm";
pub const GRADIENT: &str = "field.gradient";
pub const MIX: &str = "field.mix";
pub const REMAP: &str = "field.remap";
pub const RIDGED: &str = "field.ridged";
pub const WARP: &str = "field.warp";

pub mod constant {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(default, deny_unknown_fields)]
    pub struct Params {
        pub value: f32,
    }

    impl Default for Params {
        fn default() -> Self {
            Self { value: 0.5 }
        }
    }
}

pub mod fbm {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(default, deny_unknown_fields)]
    pub struct Params {
        pub frequency: f32,
        pub octaves: u32,
        pub lacunarity: f32,
        pub gain: f32,
        pub seed: u32,
        pub aspect: f32,
        pub spherical: bool,
    }

    impl Default for Params {
        fn default() -> Self {
            Self {
                frequency: 4.0,
                octaves: 6,
                lacunarity: 2.0,
                gain: 0.5,
                seed: 7,
                aspect: 2.0,
                spherical: true,
            }
        }
    }
}

pub mod gradient {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(default, deny_unknown_fields)]
    pub struct Params {
        pub component: u32,
        pub epsilon: f32,
    }

    impl Default for Params {
        fn default() -> Self {
            Self {
                component: 0,
                epsilon: 0.0,
            }
        }
    }
}

pub mod mix {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(default, deny_unknown_fields)]
    pub struct Params {
        pub bias: f32,
    }

    impl Default for Params {
        fn default() -> Self {
            Self { bias: 0.0 }
        }
    }
}

pub mod remap {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(default, deny_unknown_fields)]
    pub struct Params {
        pub in_min: f32,
        pub in_max: f32,
        pub out_min: f32,
        pub out_max: f32,
        pub smooth: bool,
    }

    impl Default for Params {
        fn default() -> Self {
            Self {
                in_min: 0.0,
                in_max: 1.0,
                out_min: 0.0,
                out_max: 1.0,
                smooth: true,
            }
        }
    }
}

pub mod ridged {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(default, deny_unknown_fields)]
    pub struct Params {
        pub frequency: f32,
        pub octaves: u32,
        pub lacunarity: f32,
        pub gain: f32,
        pub seed: u32,
        pub aspect: f32,
        pub sharpness: f32,
        pub spherical: bool,
    }

    impl Default for Params {
        fn default() -> Self {
            Self {
                frequency: 9.0,
                octaves: 5,
                lacunarity: 2.0,
                gain: 0.5,
                seed: 21,
                aspect: 2.0,
                sharpness: 1.6,
                spherical: true,
            }
        }
    }
}

pub mod warp {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(default, deny_unknown_fields)]
    pub struct Params {
        pub strength: f32,
        pub lateral: f32,
        pub probe: f32,
    }

    impl Default for Params {
        fn default() -> Self {
            Self {
                strength: 0.40,
                lateral: 0.50,
                probe: 0.07,
            }
        }
    }
}

/// TOML 原文 → 键用的规范 JSON。`None` = 参数文件不存在 ⇒ 算子的默认值（§17.2）。
pub fn canonical(op_id: &str, toml_text: Option<&str>) -> Result<String, String> {
    match op_id {
        CONSTANT => one::<constant::Params>(toml_text),
        FBM => one::<fbm::Params>(toml_text),
        GRADIENT => one::<gradient::Params>(toml_text),
        MIX => one::<mix::Params>(toml_text),
        REMAP => one::<remap::Params>(toml_text),
        RIDGED => one::<ridged::Params>(toml_text),
        WARP => one::<warp::Params>(toml_text),
        other => Err(format!("px_field_schema 不认识算子 {other}")),
    }
}

/// 某个算子的参数类型解析（图脚本要拿**类型化的**参数去跑仪器时用这条）。
pub fn parse<P: Serialize + DeserializeOwned + Default>(toml_text: Option<&str>) -> Result<P, String> {
    match toml_text {
        Some(text) => toml::from_str(text).map_err(|err| err.to_string()),
        None => Ok(P::default()),
    }
}

fn one<P: Serialize + DeserializeOwned + Default>(toml_text: Option<&str>) -> Result<String, String> {
    Ok(px_graph_schema::canonical_params(&parse::<P>(toml_text)?))
}
