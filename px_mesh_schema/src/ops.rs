//! 网格算子的**声明**：身份 / 超参数 / 图参数形状 / 输出域。
//!
//! ⚠ **一行实现都没有**：算法在 `px_mesh_op` 里（dylib，运行时按身份装载）。
//!   这里只说「一个算子是什么、吃什么、吐什么」——于是图侧接错一个上游、少给一个字段，
//!   都是**编译错**，而且编译这一份不需要实现库在场。
//!
//! ⚠ 图参数的形状（`CubeSphereInput { height }` / `ProxyInput { volume }`）**是接口的一部分**，
//!   所以它住在声明旁边：它是"这个算子被接的那个 struct"，不是实现细节。

use px_field_schema::field::Field;
use px_graph_schema::{Cooked, px_op};
use px_volume_schema::VolumeData;

use crate::MeshData;
use crate::params;

/// 立方球网格要吃的东西：**一张高度场**。
#[derive(px_derive::PxInputs)]
pub struct CubeSphereInput {
    pub height: Cooked<Field>,
}

/// 等值面要吃的东西：**一份体积**。
#[derive(px_derive::PxInputs)]
pub struct ProxyInput {
    pub volume: Cooked<VolumeData>,
}

px_op! {
    /// 立方球网格：一张场当位移。
    CubeSphere, "mesh.cubesphere", "px_mesh_op", params::cubesphere::Params, CubeSphereInput, MeshData
}

px_op! {
    /// 等值面代理：吃一份体积，吐一张闭合网格。
    Proxy, "mesh.proxy", "px_mesh_op", params::proxy::Params, ProxyInput, MeshData
}
