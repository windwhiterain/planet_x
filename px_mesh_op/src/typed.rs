//! 网格算子的**声明**：身份 / 超参数 / 图参数形状 / 输出域 / 怎么算 / 源码清单。
//!
//! ⚠ 输入（哪张图接谁）在**图**那一侧：这里只声明"我吃一张场"或"我吃一份体积"。

use px_cook::{Cooked, Unary1, px_op};
use px_field_schema::field::Field;
use px_graph_schema::OpKind;
use px_mesh_schema::{MeshData, params};
use px_volume_schema::VolumeData;

/// 一张场（图侧用它当上游）。
pub type FieldInput = Unary1<Cooked<Field>>;
/// 一份体积。
pub type VolumeInput = Unary1<Cooked<VolumeData>>;

/// 立方球网格：一张场当位移。
pub struct CubeSphere;

px_op! { CubeSphere = params::CUBESPHERE, 1, params::cubesphere::Params, FieldInput, MeshData,
         OpKind::Mesh, &["height"],
         [include_str!("cubesphere.rs"),
          include_str!("../../px_mesh_schema/src/params.rs"),
          include_str!("../../px_mesh_schema/src/payload.rs"),
          include_str!("../../px_field_schema/src/field.rs")],
         |p, i, g| crate::cubesphere::eval(p, &[i.a.sample()], g) }

/// 等值面代理：吃一份体积，吐一张闭合网格。
pub struct Proxy;

px_op! { Proxy = params::PROXY, 1, params::proxy::Params, VolumeInput, MeshData,
         OpKind::Mesh, &["volume"],
         [include_str!("proxy.rs"),
          include_str!("../../px_volume_schema/src/volume.rs"),
          include_str!("../../px_volume_schema/src/params.rs"),
          include_str!("../../px_volume_schema/src/payload.rs"),
          include_str!("../../px_mesh_schema/src/params.rs"),
          include_str!("../../px_mesh_schema/src/payload.rs")],
         |p, i, _g| {
             // ⚠ 与老路径同一个采样器：`VolumeGrid` 只在**同一个面内**插值，
             // 换成 `VolumeData` 直接当 sampler 会把面缝焊法换掉。
             let sampler = px_volume_schema::VolumeGrid::new(i.a.volume());
             crate::proxy::surface(p, &sampler)
                 .unwrap_or_else(|err| panic!("等值面失败：{err}"))
         } }
