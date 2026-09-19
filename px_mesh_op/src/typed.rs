//! 网格算子的**声明**：身份 + 超参数类型 + 图参数形状 + 输出域 + 怎么算。
//!
//! ⚠ 输入在**图**那一侧：立方球吃一张场、等值面吃一份体积。

use px_cook::{Cooked, Unary1, px_op};
use px_field_schema::field::Field;
use px_mesh_schema::{MeshData, params};
use px_volume_schema::VolumeData;

/// 一张场。
pub type FieldInput = Unary1<Cooked<Field>>;
/// 一份体积。
pub type VolumeInput = Unary1<Cooked<VolumeData>>;

/// 立方球网格：一张场当位移。
pub struct CubeSphere;

px_op! { CubeSphere = params::CUBESPHERE, params::cubesphere::Params, FieldInput, MeshData,
         |p, i, g| crate::cubesphere::eval(p, &[i.a.sample()], g) }

/// 等值面代理：吃一份体积，吐一张闭合网格。
pub struct Proxy;

px_op! { Proxy = params::PROXY, params::proxy::Params, VolumeInput, MeshData,
         |p, i, _g| {
             // ⚠ 与老路径同一个采样器：`VolumeGrid` 只在**同一个面内**插值，
             // 换成 `VolumeData` 直接当 sampler 会把面缝焊法换掉。
             let sampler = px_volume_schema::VolumeGrid::new(i.a.volume());
             crate::proxy::surface(p, &sampler)
                 .unwrap_or_else(|err| panic!("等值面失败：{err}"))
         } }
