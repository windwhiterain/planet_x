//! 体积载荷的序列化。
//!
//! ⚠ 体积有个特别之处：`inner`/`outer` 住在**清单参数**里，不在 blob 里
//! （`px_protocol::art::VolumeData::from_blob` 只还原数组）。所以 `decode` 必须
//! 把清单一起看 —— 这就是为什么载荷要走 `PayloadBundle` 而不是裸 blob。

use std::collections::BTreeMap;

use px_graph_schema::PayloadBundle;
use px_protocol::art::{AssetKind, VolumeData};

pub fn encode(volume: &VolumeData) -> PayloadBundle {
    PayloadBundle::new(
        AssetKind::Volume,
        BTreeMap::from([
            ("res".to_string(), volume.res as f64),
            ("layers".to_string(), volume.layers as f64),
            ("inner".to_string(), volume.inner as f64),
            ("outer".to_string(), volume.outer as f64),
        ]),
        volume.blobs(),
    )
}

pub fn decode(bytes: &[u8]) -> Result<VolumeData, String> {
    let bundle = PayloadBundle::from_bytes(bytes)?;
    let mut volume = VolumeData::from_blob(bundle.one()?).map_err(|err| err.to_string())?;
    volume.inner = bundle.params.get("inner").copied().unwrap_or(0.0) as f32;
    volume.outer = bundle.params.get("outer").copied().unwrap_or(0.0) as f32;
    Ok(volume)
}
