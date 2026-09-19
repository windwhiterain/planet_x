//! 体积算子的**声明**：身份 / 超参数 / 图参数形状 / 输出域 / 怎么算 / 源码清单。
//!
//! ⚠ 输入（哪张图接哪张场）在**图**那一侧：这里只声明"我吃一张场"。

use px_cook::{Cooked, Unary1, px_op};
use px_field_schema::field::Field;
use px_graph_schema::OpKind;
use px_volume_schema::{VolumeData, params};

/// 一张场（上游由**图**接）。
pub type FieldInput = Unary1<Cooked<Field>>;

/// **产物**形状：`cook::<CloudCoarse>` 的返回就是它（图侧拿它当"这个节点算出来了"）。
pub type VolumeOut = Cooked<VolumeData>;

/// 烘一份体积（立方球参数空间）。
pub struct CloudCoarse;

px_op! { CloudCoarse = params::CLOUD_COARSE, 1, params::Params, FieldInput, VolumeData,
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
         |p, i, _g| crate::eval_sampled(p, i.a.sample()) }
