//! 网格算子的**类型化契约**（图脚本静态链接这一份，不是 dylib）。

use px_cook::{Cooked, Identity, Op};
use px_field_schema::field::Field;
use px_graph_schema::{Grid, fnv1a_sources};
use px_mesh_schema::{MeshData, params};
use px_volume_schema::VolumeData;

use crate::{cubesphere, proxy};

/// 立方球网格：一张场当位移。
pub struct CubeSphere;

impl Op for CubeSphere {
    const IDENTITY: Identity = Identity {
        id: params::CUBESPHERE,
        version: cubesphere::VERSION,
        source_hash: fnv1a_sources(&[
            "typed.rs",
            "cubesphere.rs",
            "../../px_mesh_schema/src/params.rs",
            "../../px_mesh_schema/src/payload.rs",
            "../../px_field_schema/src/field.rs",
        ]),
    };

    type Params = params::cubesphere::Params;
    type Inputs<'a> = &'a Cooked<Field>;
    type Payload = MeshData;

    fn cook(params: &Self::Params, inputs: &Self::Inputs<'_>, grid: Grid) -> MeshData {
        cubesphere::eval(params, &[inputs.field()], grid)
    }
}

/// 等值面代理：吃一份体积，吐一张闭合网格。
pub struct Proxy;

impl Op for Proxy {
    const IDENTITY: Identity = Identity {
        id: params::PROXY,
        version: proxy::VERSION,
        source_hash: fnv1a_sources(&[
            "typed.rs",
            "proxy.rs",
            "../../px_volume_schema/src/volume.rs",
            "../../px_volume_schema/src/params.rs",
            "../../px_volume_schema/src/payload.rs",
            "../../px_mesh_schema/src/params.rs",
            "../../px_mesh_schema/src/payload.rs",
        ]),
    };

    type Params = params::proxy::Params;
    type Inputs<'a> = &'a Cooked<VolumeData>;
    type Payload = MeshData;

    fn cook(params: &Self::Params, inputs: &Self::Inputs<'_>, _grid: Grid) -> MeshData {
        // ⚠ 与老路径同一个采样器：`VolumeGrid` 只在**同一个面内**插值，
        // 换成 `VolumeData` 直接当 sampler 会把面缝焊法换掉。
        let sampler = px_volume_schema::VolumeGrid::new(&inputs.value);
        proxy::surface(params, &sampler)
            .unwrap_or_else(|err| panic!("等值面失败：{err}"))
    }
}
