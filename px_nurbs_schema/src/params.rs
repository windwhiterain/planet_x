//! NURBS 算子的参数。
//!
//! ⚠ **控制点 / 权 / 节点向量不在参数文件里**：它们是**载荷**（`NurbsCurve` / `NurbsSurface`
//!   的清单参数 + blob），由图上的上游节点造出来（`nurbs.circle` / `nurbs.bilinear_patch` …）
//!   —— 参数文件里写几百个控制点既不是"调参"，也不能被别的节点复用。
//!   参数只管**这一刀怎么切**：在哪求值、插哪个节点、细到多细。
//!
//! ⚠ 算子 id 与接口形状住在同目录的 `ops.rs` 里 —— 一处定义。

pub mod curve {
    use serde::{Deserialize, Serialize};

    /// 造一条**有理二次整圆**（精确，不是拟合）。
    ///
    /// ⚠ 圆是 NURBS 最经典的那条判据：`√2/2` 那个权一旦按 `f32` 存，圆就不再是圆
    ///   ⇒ 库里一律 `f64`（见 `curve.rs` 顶上那条）。
    #[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
    #[serde(default, deny_unknown_fields)]
    pub struct CircleParams {
        /// 半径。
        pub radius: f64,
        /// 放在哪张平面：`xy` / `xz` / `yz`。
        pub plane: String,
        /// 圆心。
        pub center: [f64; 3],
    }

    impl Default for CircleParams {
        fn default() -> Self {
            Self {
                radius: 1.0,
                plane: "xy".to_string(),
                center: [0.0; 3],
            }
        }
    }

    /// 造一张**有理二次球面**（赤道那圈用整圆的 9 点写法，两极靠半圆）。
    ///
    /// ⚠ 球与圆同一条理由进库：它是"有理二次**精确**表示二次曲面"这条性质的判据
    ///   —— 细分成网格之后每个顶点都应当落在半径上（误差只来自 f64 的舍入）。
    #[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
    #[serde(default, deny_unknown_fields)]
    pub struct SphereParams {
        pub radius: f64,
        pub center: [f64; 3],
        /// 纬向（v）分几段：每段 90° ⇒ 默认 2 就是上半球到下半球。
        pub rings: u32,
    }

    impl Default for SphereParams {
        fn default() -> Self {
            Self {
                radius: 1.0,
                center: [0.0; 3],
                rings: 2,
            }
        }
    }
}

pub mod eval {
    use serde::{Deserialize, Serialize};

    /// 在参数 `t` / `(u, v)` 上求值（以及一阶导）。
    #[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
    #[serde(default, deny_unknown_fields)]
    pub struct EvalParams {
        /// 参数值（曲线 `t`、曲面 `u`）。
        pub u: f64,
        /// 曲面那一档的第二个参数（曲线不看它）。
        pub v: f64,
        /// 一阶导要不要一起交出来（`nurbs.hodograph` 是它的常开档）。
        pub tangent: bool,
    }

    impl Default for EvalParams {
        fn default() -> Self {
            Self {
                u: 0.5,
                v: 0.5,
                tangent: false,
            }
        }
    }
}

pub mod insert {
    use serde::{Deserialize, Serialize};

    /// 插入节点（几何一个字不变，只多控制点）。
    #[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
    #[serde(default, deny_unknown_fields)]
    pub struct InsertParams {
        /// 插在哪（参数值）。
        pub t: f64,
        /// 插几重。
        pub times: u32,
        /// 曲面那一档的方向：`u` / `v`。
        pub along: String,
    }

    impl Default for InsertParams {
        fn default() -> Self {
            Self {
                t: 0.5,
                times: 1,
                along: "u".to_string(),
            }
        }
    }
}

pub mod elevate {
    use serde::{Deserialize, Serialize};

    /// 升阶（几何一个字不变，次数变高）。
    #[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
    #[serde(default, deny_unknown_fields)]
    pub struct ElevateParams {
        /// 目标次数（只许往上）。
        pub degree: u32,
    }

    impl Default for ElevateParams {
        fn default() -> Self {
            Self { degree: 3 }
        }
    }
}

pub mod tessellate {
    use serde::{Deserialize, Serialize};

    /// **细分**：按弦误差把一张曲面（或一条曲线）摊成三角网格。
    ///
    /// 误差是**世界单位**的弦高上界 —— 调它就是在"顶点数"与"看起来是圆的"之间选。
    #[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
    #[serde(default, deny_unknown_fields)]
    pub struct TessellateParams {
        /// 弦误差上界（世界单位）。越小顶点越多。
        pub tolerance: f64,
        /// 递归深度上限（保底：`2^depth` 段，防止在尖点处无限递归）。
        pub depth: u32,
        /// 参数域先切几段才开始细分。
        pub segments: u32,
    }

    impl Default for TessellateParams {
        fn default() -> Self {
            Self {
                tolerance: 1e-3,
                depth: 6,
                segments: 4,
            }
        }
    }
}
