//! 体积算子的**类型化契约**（图脚本静态链接这一份，不是 dylib）。
//!
//! ⚠ 参数文件的**位置**是图的知识：同一个 `volume.cloud.coarse` 在 `clouds` 里
//! 叫 `coarse`、也叫 `coarse_fine` ⇒ `cook_volume` 每次调用都点名。

use px_cook::{Cooked, Identity, Op};
use px_field_schema::field::Field;
use px_graph_schema::{Grid, fnv1a_sources};
use px_volume_schema::{VolumeData, params};

/// 烘一份体积（立方球参数空间）。
pub struct CloudCoarse;

impl Op for CloudCoarse {
    const IDENTITY: Identity = Identity {
        id: params::CLOUD_COARSE,
        version: crate::VERSION,
        source_hash: fnv1a_sources(&[
            "typed.rs",
            "lib.rs",
            "../../px_volume_schema/src/volume.rs",
            "../../px_volume_schema/src/params.rs",
            "../../px_volume_schema/src/payload.rs",
            "../../px_field_schema/src/field.rs",
            "../../px_verify/src/cloud_field.rs",
            "../../px_verify/src/noise.rs",
            "../../px_verify/src/dual.rs",
            "../../px_verify/src/proxy.rs",
        ]),
    };

    type Params = params::Params;
    type Inputs<'a> = &'a Cooked<Field>;
    type Payload = VolumeData;

    fn cook(params: &Self::Params, coverage: &Self::Inputs<'_>, _grid: Grid) -> VolumeData {
        crate::eval_sampled(params, coverage.field())
    }
}
