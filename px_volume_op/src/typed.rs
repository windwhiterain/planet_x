//! 体积算子的**声明**：身份 / 超参数 / 图参数形状 / 输出域 / 怎么算 / 源码清单。
//!
//! ⚠ **图参数的形状是算子接口的一部分**（`CloudCoarseInput { coverage }`）——
//! 图侧写错字段、少给上游都是编译错。

use px_cook::{Cooked, Grid, PxInputs, px_op};
use px_field_schema::field::Field;
use px_graph_schema::OpKind;
use px_volume_schema::{VolumeData, params};

/// 烘一份体积要吃的东西：**一张覆盖度场**。
#[derive(Clone)]
pub struct CloudCoarseInput {
    pub coverage: Cooked<Field>,
}

impl PxInputs for CloudCoarseInput {
    fn collect(&self, hasher: &mut px_cook::blake3::Hasher) {
        hasher.update(&self.coverage.key);
    }
}

impl px_cook::FromPayloads for CloudCoarseInput {
    fn from_payloads(inputs: &[&[u8]], grid: Grid) -> Result<Self, String> {
        let [coverage] = inputs else {
            return Err(format!("吃 1 张覆盖度场，却收到 {} 个上游", inputs.len()));
        };
        Ok(Self {
            coverage: Cooked::from_bytes(coverage, grid)?,
        })
    }
}

/// 产物形状：`cook::<CloudCoarse>` 返回的就是它。
pub type VolumeOut = Cooked<VolumeData>;

/// 烘一份体积（立方球参数空间）。
pub struct CloudCoarse;

px_op! { CloudCoarse = params::CLOUD_COARSE, 1, params::Params, CloudCoarseInput, VolumeData,
         OpKind::Volume, &["coverage"],
         [include_str!("lib.rs"),
          include_str!("../../px_volume_schema/src/volume.rs"),
          include_str!("../../px_volume_schema/src/params.rs"),
          include_str!("../../px_volume_schema/src/payload.rs"),
          include_str!("../../px_field_schema/src/field.rs"),
          include_str!("../../px_verify/src/cloud_field.rs"),
          include_str!("../../px_verify/src/noise.rs"),
          include_str!("../../px_verify/src/dual.rs"),
          include_str!("../../px_verify/src/proxy.rs")],
         |p, i, _g| crate::eval_sampled(p, i.coverage.sample()) }
