use crate::payload;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointData {
    pub point: [f64; 3],
    pub tangent: [f64; 3],
    pub normal: [f64; 3],
    pub uv: [f64; 2],
    pub has_tangent: bool,
}

impl PointData {
    pub fn at(point: [f64; 3], u: f64, v: f64) -> Self {
        Self {
            point,
            tangent: [0.0; 3],
            normal: [0.0, 0.0, 1.0],
            uv: [u, v],
            has_tangent: false,
        }
    }

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
