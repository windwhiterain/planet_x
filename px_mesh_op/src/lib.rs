//! **网格域**的算子：立方球位移网格（`mesh.cubesphere`）与等值面代理（`mesh.proxy`）。
//!
//! `mesh.proxy` 是**叶子**：它只认识体积的参数空间约定（`px_volume_schema`）与算法库
//! （`isosurface`）—— 不认识云、不认识驱动 ⇒ 换算法库不动图脚本。

use px_graph_schema::{Grid, OpCall, OpDescriptor, OpTable, ParamsCanonical};
use px_mesh_schema::{params, payload as mesh_payload};

pub mod typed;

pub mod cubesphere;
pub mod proxy;

static OPS: &[OpDescriptor] = &[cubesphere::DESCRIPTOR, proxy::DESCRIPTOR];

#[unsafe(no_mangle)]
pub extern "Rust" fn px_mesh_op_table() -> &'static OpTable {
    static TABLE: OpTable = OpTable {
        ops: OPS,
        canonical_params: params::canonical as ParamsCanonical,
        call: call as OpCall,
    };
    &TABLE
}

extern "Rust" fn call(
    op_id: &str,
    params_json: &str,
    grid: Grid,
    inputs: &[&[u8]],
) -> Result<Vec<u8>, String> {
    match op_id {
        params::CUBESPHERE => {
            let typed: params::cubesphere::Params = serde_json::from_str(params_json)
                .map_err(|err| format!("参数 JSON 解不开：{err}"))?;
            let height = px_field_schema::payload::decode(inputs[0], grid.projection)?;
            let mesh = cubesphere::eval(&typed, &[&height], grid);
            mesh_payload::encode(&mesh).placeholder()
        }
        params::PROXY => {
            let typed: params::proxy::Params = serde_json::from_str(params_json)
                .map_err(|err| format!("参数 JSON 解不开：{err}"))?;
            let volume = px_volume_schema::payload::decode(inputs[0])?;
            let sampler = px_volume_schema::VolumeGrid::new(&volume);
            let mesh = proxy::surface(&typed, &sampler)?;
            mesh_payload::encode(&mesh).placeholder()
        }
        other => Err(format!("px_mesh_op 不认识算子 {other}")),
    }
}
