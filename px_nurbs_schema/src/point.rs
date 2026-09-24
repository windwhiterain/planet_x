//! **NURBS 求值出来的那一档载荷**：一个点（或一个切向量）、一个法线、一对参数。
//!
//! ⚠ 它为什么是一个**载荷**（而不是"顺手打印出来的三个数"）：求值要能进图 ——
//!   下游（比如"沿曲线取样点撒一批东西"）得拿到它，而进图的东西必须是**可缓存、
//!   可内容寻址**的。它同时充当"参数"那一档：`CurveAt` / `SurfaceAt` 收的就是它
//!   ⇒ 「在哪求值」可以由别的节点算出来，而不是只能在参数文件里写死。

use crate::payload;

/// 一个点 / 切向量 / 法线，外加它的参数坐标。
///
/// ⚠ 它同时是 `CurveAt` / `SurfaceAt` 的**上游**（它们只读 `uv` 那一栏）⇒
///   "在哪求值"可以由别的节点算出来，而不是只能在参数文件里写死。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointData {
    /// 位置。
    pub point: [f64; 3],
    /// 一阶导（切向量）。算没算由 `has_tangent` 说。
    pub tangent: [f64; 3],
    /// 单位法线（曲面那一档是曲面法线；曲线那一档是**曲线所在平面的法线**）。
    pub normal: [f64; 3],
    /// 参数坐标（曲线只填 `u`）。
    pub uv: [f64; 2],
    /// 这一份里有没有真算过切向量。
    pub has_tangent: bool,
}

impl PointData {
    /// 一个"只有点"的读数（切向量按 0 交出去）。
    pub fn at(point: [f64; 3], u: f64, v: f64) -> Self {
        Self {
            point,
            tangent: [0.0; 3],
            normal: [0.0, 0.0, 1.0],
            uv: [u, v],
            has_tangent: false,
        }
    }

    /// 逐个数（blob 顺序：点 / 切 / 法线 / uv / 标志 —— 3+3+3+2+1 = 12）。
    pub fn values(&self) -> Vec<f64> {
        let mut out = Vec::with_capacity(12);
        out.extend_from_slice(&self.point);
        out.extend_from_slice(&self.tangent);
        out.extend_from_slice(&self.normal);
        out.extend_from_slice(&self.uv);
        out.push(if self.has_tangent { 1.0 } else { 0.0 });
        out
    }

    pub fn from_values(values: &[f64]) -> Result<Self, String> {
        payload::expect_len(values, 12, "PointData")?;
        Ok(Self {
            point: [values[0], values[1], values[2]],
            tangent: [values[3], values[4], values[5]],
            normal: [values[6], values[7], values[8]],
            uv: [values[9], values[10]],
            has_tangent: values[11] != 0.0,
        })
    }

    /// 读数那一行。
    pub fn detail(&self) -> String {
        let mut text = format!(
            "点 ({:.6}, {:.6}, {:.6})｜参数 ({:.6}, {:.6})",
            self.point[0], self.point[1], self.point[2], self.uv[0], self.uv[1],
        );
        if self.has_tangent {
            text.push_str(&format!(
                "｜切 ({:.6}, {:.6}, {:.6})",
                self.tangent[0], self.tangent[1], self.tangent[2],
            ));
        }
        text
    }
}
