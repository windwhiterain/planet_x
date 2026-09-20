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

/// **体网格上的分形噪声**（`field.fbm3`）：采样点是"这一格的体素坐标"，不是球面方向。
///
/// ⚠ 它与 [`fbm`] 的差别不只是"三维"：`fbm` 的球面档按 `direction` 取噪声（**没有径向**），
///   于是它造出来的场是"贴在球面上的一层皮"；这一档按 `(s, t, altitude)` 取噪声
///   ⇒ 才有真正的**体内结构**（云里前中后三层各自不同）。体渲染要的正是后者。
///
/// ⚠ 频率的参照系是"**一整面 = 1.0**"（体素坐标是 `[0,1]³`）⇒ 与 `fbm` 的球面档
///   （`direction` 的模长是 1）**数值口径相近**，但**不是同一把尺子**：同一个频率下
///   这一档的格子数是 `frequency` 个/面。
#[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
#[serde(default, deny_unknown_fields)]
pub struct Fbm3Params {
    pub frequency: f32,
    pub octaves: u32,
    pub lacunarity: f32,
    pub gain: f32,
    pub seed: u32,
    /// **各向异性**：把体素坐标的第三维（径向）乘上它 ⇒ 结构沿**径向**拉长／压扁。
    ///
    /// ⚠ 星云的盘状/纤维状结构是**沿视线方向拉长**的，而各向同性的噪声给的是"一坨坨圆球"
    ///   ⇒ 这一栏是"云"与"絮"之间那个旋钮。`1.0` = 各向同性。
    pub zonal: f32,
}

impl Default for Fbm3Params {
    fn default() -> Self {
        Self {
            frequency: 3.0,
            octaves: 6,
            lacunarity: 2.0,
            gain: 0.5,
            seed: 7,
            zonal: 1.0,
        }
    }
}

/// **体网格上的脊状噪声**（`field.ridged3`）：星云的"丝"就是脊。
///
/// ⚠ `sharpness` 越大脊越细（`1.0` = 三角波，`2` 以上 = 一根根细丝）。
///   星云那些一丝一丝的纤维结构靠的是这一档 + 后续的域扭曲。
#[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
#[serde(default, deny_unknown_fields)]
pub struct Ridged3Params {
    pub frequency: f32,
    pub octaves: u32,
    pub lacunarity: f32,
    pub gain: f32,
    pub seed: u32,
    pub sharpness: f32,
    pub zonal: f32,
}

impl Default for Ridged3Params {
    fn default() -> Self {
        Self {
            frequency: 6.0,
            octaves: 5,
            lacunarity: 2.1,
            gain: 0.55,
            seed: 21,
            sharpness: 2.0,
            zonal: 1.0,
        }
    }
}

/// **体网格上的域扭曲**（`field.warp3`）：按偏移场挪动采样点。
///
/// ⚠ 它与 [`warp`]（球面那一档）的差别：那一档在**切平面**上挪（`lateral` 决定
///   沿视线挪多少），这一档在**体素空间**里挪三个轴 —— 没有"切平面"这回事，
///   三格偏移就是三维位移。
#[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
#[serde(default, deny_unknown_fields)]
pub struct Warp3Params {
    /// 位移总量（**体素坐标**的单位：`1.0` = 挪一整面）。⚠ 这里不是弧度。
    pub strength: f32,
    /// 轴向权重：`0` = 只沿径向挪，`1` = 三个轴等权。
    pub axial: f32,
}

impl Default for Warp3Params {
    fn default() -> Self {
        Self {
            strength: 0.35,
            axial: 1.0,
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
        /// **球面档的各向异性**（2026-09-20 加）：把噪声沿**经度**拉长几倍。
        ///
        /// 取样点在球面上是方向 `d`，这一栏把 `d.y` 乘上它再查噪声 ⇒ 噪声在**纬度方向**上
        /// 变化快 `zonal` 倍、在经度方向不变 ⇒ 出来的形状是"**沿经度拉长的长条**"。
        ///
        /// ⚠ 为什么要它：气态巨行星的湍流**在经度方向是连贯的**（流线、长条），不是一团团
        ///   各向同性的疙瘩。拿各向同性的噪声去推条带，条带会被搅成"絮状/团块"
        ///   （第 6 轮那版实测就是这样）。`1.0` = 各向同性。
        /// ⚠ 默认 `1.0` ⇒ **既有节点一位不改**（不是拿 `aspect` 来兼职：它默认 2.0，
        ///   球面档历史上忽略它，改成生效会把全仓每一张球面 fbm 都换掉）。
        pub zonal: f32,
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
                zonal: 1.0,
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

/// **盖章式打坑**（`field.stamps`）—— 与 `CratersParams` 同一个意图，换一套**摆法**。
///
/// ⚠ 与 `field.craters` 的差别（用户 2026-09-20 的口径："像印章一样打很多上去，按真实物理压盖"）：
///   `craters` 是**元胞距离**：每格一个坑、同层半径是**常数**、同层不重叠 ⇒ 坑的大小与间距偏整齐。
///   真实的撞击地貌是：**小坑极多、偶尔一个巨坑**（尺寸幂律）、**互相压盖**（年轻的坑挖掉老的坑缘，
///   于是老坑只剩半个），而且**不是哪儿都一样密**（月海少坑、高地密坑）。
///   这一支就是照这三件事来的 —— 三个机制各自对应下面三组参数：
///   1. **摆法**：格点还是那个加速结构（每格至多一个印章），但每个印章的**半径与年龄都是随机的**；
///   2. **压盖**：按**年龄序**从老到新依次"挖掘"—— 碗内把已有地形**清掉**再落碗底，
///      坑缘是往上堆的；年轻的坑因此会把老坑的坑缘切掉（真实规则，不是"谁新谁覆盖"那么简单）；
///   3. **分布**：半径按**幂律**抽（`power > 1` ⇒ 小坑多），密度受**上游场当遮罩**
///      （`mask_lo..mask_hi` 之间线性映射成盖章概率）。
#[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
#[serde(default, deny_unknown_fields)]
pub struct StampsParams {
    /// 格子密度：球面档是 `direction × frequency`，平面档是 `(u × aspect, v) × frequency`。
    /// ⚠ 它同时是**印章尺寸的尺子**：半径以格为单位（见 `max_radius`）。
    pub frequency: f32,
    /// 叠几"代"印章（每代换个格点尺度 ⇒ 巨坑与小坑并存；代与代之间也是"老的先打"）。
    pub octaves: u32,
    /// 每代格子密度的倍率（与 `fbm` 同口径）。
    pub lacunarity: f32,
    /// 每代的权重衰减（与 `fbm` 同口径）。
    pub gain: f32,
    /// 印章中心在它自己格子里的散布（`0` = 规则格点，`1` = 满格乱撒）。
    pub jitter: f32,
    pub seed: u32,
    /// 平面档的横比（与 `fbm` 同口径；球面档不用）。
    pub aspect: f32,
    /// 球面档（按 `direction` 取格点）；`false` = 平面档。
    pub spherical: bool,
    /// 半径上限（格为单位）：`base_radius × (1 + 该代往上加)` 之后的封顶，也是幂律抽样的上界。
    pub max_radius: f32,
    /// 半径下限（格为单位）：幂律抽样的下界（再小就比一个纹素还细 ⇒ 只会变成噪点）。
    pub min_radius: f32,
    /// 幂律指数：`r = min + (max − min)·(1 − u)^power`（`u ∈ [0,1)` 由哈希给）。
    /// `power = 1` = 均匀；`power > 1` = **小坑多、大坑少**（真实的撞击坑尺寸分布就是这个方向）。
    pub power: f32,
    /// 坑深 / 半径（真实简单坑的深径比约 1/5 ⇒ `0.2`）。
    pub depth: f32,
    /// 坑缘高 / 坑深（真实约 `0.2` 上下）。
    pub height: f32,
    /// 坑缘半宽 / 半径。
    pub rim: f32,
    /// 碗内的**清除强度**：`1` = 碗里旧地形全清（年轻坑挖到底）、`0` = 旧地形原样保留（只叠加）。
    /// ⚠ 这一栏就是"按真实物理压盖"的那个旋钮：真实撞击是**挖掉**再堆坑缘，所以默认给得高。
    pub excavate: f32,
    /// 老化：越老的印章，坑越浅、缘越平（`0` = 不老化；`1` = 最老的只剩一半）。
    pub degrade: f32,
    /// 遮罩：上游值 ≤ `mask_lo` 的地方**完全不盖章**，≥ `mask_hi` 的地方**满概率**。
    pub mask_lo: f32,
    pub mask_hi: f32,
}

impl Default for StampsParams {
    fn default() -> Self {
        Self {
            frequency: 6.0,
            octaves: 3,
            lacunarity: 2.4,
            gain: 0.7,
            jitter: 0.9,
            seed: 31,
            aspect: 2.0,
            spherical: true,
            max_radius: 0.55,
            min_radius: 0.12,
            power: 2.2,
            depth: 0.22,
            height: 0.22,
            rim: 0.35,
            excavate: 0.85,
            degrade: 0.45,
            mask_lo: 0.0,
            mask_hi: 1.0,
        }
    }
}
