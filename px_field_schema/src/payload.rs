//! 场载荷的序列化：**算子之间流动的就是这一份字节**。
//!
//! blob 形状仍是拆分前 `px_ops::write_artifact` 那一档：`[height, width]` 的 f32 块；
//! 清单一栏 `width` / `height`，外加 `projection`（[`Domain::code`]）。
//!
//! ⚠ 投影为什么**进清单**：`Field::to_blob` 只存形状，而"这一格在世界里的哪"是**域**的
//!   事 —— 从前它由清单里的 `AssetKind` 反推，而资产种类已经不是图缓存载荷的一栏
//!   （那是**渲染**认"盘上这坨字节是什么"的概念）⇒ 载荷得自己说清楚，否则读盘上的
//!   一份场产物就不知道往哪采样（`store::load_field` 就是那个读者）。

use std::collections::BTreeMap;

use px_graph_schema::PayloadBundle;
use px_protocol::art::Domain;

use crate::field::Field;

/// 清单里那一格投影的键名。
pub const PROJECTION_KEY: &str = "projection";

pub fn encode(field: &Field) -> PayloadBundle {
    PayloadBundle::new(
        BTreeMap::from([
            ("width".to_string(), field.width as f64),
            ("height".to_string(), field.height as f64),
            (
                PROJECTION_KEY.to_string(),
                f64::from(field.projection.code()),
            ),
        ]),
        vec![field.to_blob()],
    )
}

/// ⚠ 投影优先取**载荷里那一格**（读盘上的产物时只有它说得清）；载荷里没有（旧产物）
///   就退回调用方给的那一个：算子给的是画布的投影，驱动给的是图规格的投影。
pub fn decode(bundle: &PayloadBundle, projection: Domain) -> Result<Field, String> {
    let mut field = Field::from_blob(bundle.one()?).map_err(|err| err.to_string())?;
    field.projection = stored_projection(bundle).unwrap_or(projection);
    Ok(field)
}

/// 清单里那一格投影（没有就算了 —— 旧产物没有这一栏）。
pub fn stored_projection(bundle: &PayloadBundle) -> Option<Domain> {
    bundle
        .params
        .get(PROJECTION_KEY)
        .copied()
        .and_then(|value| Domain::from_code(value as u8))
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
