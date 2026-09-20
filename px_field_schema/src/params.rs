//! 场算子的**参数**：TOML 长什么样、默认值是多少。
//! （算子 id 与接口形状住在同目录的 `ops.rs` 里 —— 一处定义。）
//!
//! ⚠ 参数住 schema 这一侧，是因为**两处都要它**：算子声明（`ops.rs`）与判据仪器 ——
//! 谁也不许自己再抄一份字段。`cook` 拿 `O::Params` 解 TOML（缺文件走 `Default`），
//! 解出来的**规范 JSON** 进键，算子的 `render` 收到的是同一个值
//! ⇒ 不会出现「键里的参数」与「算出来的参数」不是同一个。

use serde::{Deserialize, Serialize};

/// **泛型实例那一档的参数**（`FieldRemap` 的 `Params`）。
///
/// ⚠ 它与 [`remap::Params`] **不是**同一个类型，也**不该**合并：`remap` 是"把 `[in_min, in_max]`
///   线性映到 `[out_min, out_max]`"那一档（七个预置算子之一）；这一档是"**上游 + 图侧函数**"
///   （`px_inst!` 复用的声明）——`in_*` / `out_*` / `smooth` 是**共享路径**自己那一把尺子
///   （活在 `px_field_alg::remap::normalize_value`，按 `[0,1] → [0,1]` 恒等档跑），
///   而这里的三栏是**给图侧函数调的**（对比度 / 偏移 / 条带数）。
///   两者共用的只有"归一化 + 钳制 + 映到输出值域"那一段，那一段住在 `px_field_alg` 里（**一份**）。
///
/// ⚠ 三栏都**进键**（`PxParams` 按字段名 + 字段值写哈希）：只改对比度必换键、必重算 ——
///   而图程序 exe **一个字节不动**（泛型参数与参数都由命令行走）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, px_derive::PxParams)]
#[serde(default, deny_unknown_fields)]
pub struct RemapParams {
    /// 对比度：图侧函数把落点从 `0.5` 往两边推开多少（`0` = 不推）。
    pub gain: f32,
    /// 偏移：整张场加多少再钳回 `[0,1]`。
    pub bias: f32,
    /// 条带数：图侧函数里那条正弦的**周期数**（换成"圈"就是 `bands` 圈）。
    pub bands: f32,
}

impl Default for RemapParams {
    fn default() -> Self {
        Self {
            gain: 0.65,
            bias: 0.0,
            bands: 8.0,
        }
    }
}

pub mod constant {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
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

    #[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
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

    #[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
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

    #[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
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

    #[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
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

    #[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
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

    #[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
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

/// **陨坑**（`field.craters`）：在一张地形场上按格点撒坑 —— 坑里凹、坑缘凸。
///
/// 它是"无大气天体"那张图的主角（`moon` 图）：先有一张基础地形，再用两层坑
/// （大盆地 + 小坑）叠上去。**球面档**按 `direction` 取格点（3D 元胞噪声）⇒ 没有接缝、
/// 两极也不会挤；平面档按 `(u × aspect, v)` 取格点。
///
/// ⚠ 每一层是**元胞距离**（到最近特征点的距离，格为单位）再套一个剖面：
///   坑内是 `t²` 的碗（`t = 1 - d / radius`），坑缘是 `radius .. radius + rim` 上的半正弦凸起
///   —— 两者在 `d = radius` 处**都是 0**，所以剖面连续、不会在坑边留下一条硬台阶。
/// ⚠ `depth` / `height` 是**值域单位**（不是格）：一层最多把值往下压 `depth / 2`、往上抬
///   `height / 2`，多层按 `gain` 加权后**用总权归一** ⇒ 叠多少层都不会把值顶出多少。
///   算子**不钳制**输出（"算出来的值域"是图自己的事：要钳就在下游接一个 `field.remap`）。
#[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
#[serde(default, deny_unknown_fields)]
pub struct CratersParams {
    /// 格子密度：球面档是 `direction × frequency`，平面档是 `(u × aspect, v) × frequency`。
    pub frequency: f32,
    /// 叠几层坑（每层格点尺度不同 ⇒ 大盆地与小坑并存）。
    pub octaves: u32,
    /// 每层格子密度的倍率（与 `fbm` 同口径）。
    pub lacunarity: f32,
    /// 每层的权重衰减（与 `fbm` 同口径）。
    pub gain: f32,
    /// 特征点在它自己格子里的散布（`0` = 规则格点，`1` = 满格乱撒）。⚠ 大于 `1` 时
    /// 27 邻域搜索不再保证找到最近点（会出现"本格的坑被邻居抢走"），算子把它钳在 `[0,1]`。
    pub jitter: f32,
    pub seed: u32,
    /// 平面档的横比（与 `fbm` 同口径；球面档不用）。
    pub aspect: f32,
    /// 球面档（按 `direction` 取格点）；`false` = 平面档。
    pub spherical: bool,
    /// 坑半径（格为单位）。
    pub radius: f32,
    /// 坑缘半宽（格为单位）。
    pub rim: f32,
    /// 坑深（往下压多少，值域单位）。
    pub depth: f32,
    /// 坑缘高（往上抬多少，值域单位）。
    pub height: f32,
}

impl Default for CratersParams {
    fn default() -> Self {
        Self {
            frequency: 6.0,
            octaves: 3,
            lacunarity: 2.15,
            gain: 0.55,
            jitter: 0.85,
            seed: 31,
            aspect: 2.0,
            spherical: true,
            radius: 0.62,
            rim: 0.18,
            depth: 0.35,
            height: 0.16,
        }
    }
}

