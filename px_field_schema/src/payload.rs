//! 场载荷的序列化：**算子之间流动的就是这一份字节**。
//!
//! 清单参数与 blob 形状逐字沿用**拆分前** `px_ops::write_artifact` 那一档：`{width, height}` + 一个
//! `[height, width]` 的 f32 块。场没有一个数住在清单里（不像体积的 `inner`/`outer`），
//! 所以 `encode`/`decode` 是无损的。

use std::collections::BTreeMap;

use px_graph_schema::PayloadBundle;
use px_protocol::art::Domain;

use crate::field::{Field, ProjectionKind};

pub fn encode(field: &Field) -> PayloadBundle {
    PayloadBundle::new(
        field.projection.asset_kind(),
        BTreeMap::from([
            ("width".to_string(), field.width as f64),
            ("height".to_string(), field.height as f64),
        ]),
        vec![field.to_blob()],
    )
}

/// ⚠ 投影**不在载荷里**（`Field::to_blob` 只存形状）⇒ 由调用方给：
/// 算子给的是画布的投影，驱动给的是图规格的投影。
pub fn decode(bundle: &PayloadBundle, projection: Domain) -> Result<Field, String> {
    let mut field = Field::from_blob(bundle.one()?).map_err(|err| err.to_string())?;
    field.projection = projection;
    Ok(field)
}

/// **场这一域**的编解码与读数 —— 契约在 `px_graph_schema::payload::Build`，话由域自己说。
impl px_graph_schema::Build for Field {
    /// 场的产物里带评审相机表（相机是"怎么看"，而场**能**被看）。
    const WITH_CAMERAS: bool = true;
    /// ⚠ 场的分辨率**就是画布** ⇒ 画布必须进键。
    const RESOLUTION_IS_CANVAS: bool = true;

    fn detail(payload: &Self) -> String {
        let stats = payload.stats();
        format!(
            "{}×{}｜值域 {:.4}..{:.4}｜均值 {:.4}",
            payload.width, payload.height, stats.min, stats.max, stats.mean
        )
    }

    fn encode(payload: &Self) -> Result<PayloadBundle, String> {
        Ok(crate::payload::encode(payload))
    }

    fn decode(bundle: &PayloadBundle, projection: Domain, node: &str) -> Result<Self, String> {
        let _ = node;
        crate::payload::decode(bundle, projection)
    }
}
