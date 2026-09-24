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

/// ⚠ 投影取**载荷里那一格**（读盘上的产物时只有它说得清）；老产物没有这一栏就退回
///   `Equirect`（那时它是从图规格补的，而图规格已经没有投影了）。
pub fn decode(bundle: &PayloadBundle) -> Result<Field, String> {
    let mut field = Field::from_blob(bundle.one()?).map_err(|err| err.to_string())?;
    field.projection = stored_projection(bundle).unwrap_or(crate::field::Projection::Equirect);
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

    fn decode(bundle: &PayloadBundle, node: &str) -> Result<Self, String> {
        let _ = node;
        crate::payload::decode(bundle)
    }
}
